//! Tipos neutros del sistema de archivos: el lugar que muestra un panel, los elementos de una
//! carpeta y los mensajes de su lectura (que corre en otro hilo y avisa a la ventana con
//! `WM_APP_LOAD`), y la búsqueda en subcarpetas. Lo que depende del sistema (leer una carpeta,
//! íconos, unidades, vigilancia) está detrás de `sys::FileSystem`.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::WM_APP;
use crate::sys::{self, FileSystem, Handle};

/// Hay mensajes nuevos en el canal de carga. `wparam` = panel.
pub const WM_APP_LOAD: u32 = WM_APP + 1;
/// Cambió algo en la carpeta vigilada. `wparam` = panel, `lparam` = generación.
pub const WM_APP_CHANGED: u32 = WM_APP + 2;

/// Lugar que muestra un panel.
#[derive(Clone, PartialEq, Debug)]
pub enum Location {
    /// "Este equipo": carpetas del usuario, unidades, dispositivos, Red y Papelera.
    Computer,
    Path(PathBuf),
    /// Carpeta virtual del Shell sin ruta de disco: Papelera, Red, un teléfono (MTP)...
    Shell {
        /// Nombre de análisis del Shell ("::{645FF040-...}" para la Papelera).
        parsing: String,
        name: String,
        /// De dónde se llegó, para "subir"; sin él, se sube a "Este equipo".
        parent: Option<Box<Location>>,
    },
}

impl Location {
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Computer => None,
            Location::Path(p) => Some(match p.parent() {
                Some(parent) => Location::Path(parent.to_path_buf()),
                None => Location::Computer,
            }),
            Location::Shell { parent, .. } => Some(parent.as_deref().cloned().unwrap_or(Location::Computer)),
        }
    }

    /// Título de la pestaña (con el sistema de archivos instalado).
    pub fn title(&self) -> String {
        self.title_in(sys::file_system())
    }

    /// Título de la pestaña con el sistema de archivos `env`: el último tramo de la ruta; una raíz
    /// se titula como la nombra el sistema ("Disco local (C:)", "Macintosh HD").
    pub fn title_in(&self, env: &dyn FileSystem) -> String {
        match self {
            Location::Computer => "Este equipo".into(),
            Location::Path(p) => match p.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => env.root_title(p).unwrap_or_else(|| p.to_string_lossy().trim_end_matches('\\').to_string()),
            },
            Location::Shell { name, .. } => name.clone(),
        }
    }

    /// Segmentos de la barra de dirección: (texto, destino), con el sistema de archivos instalado.
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        self.crumbs_in(sys::file_system())
    }

    /// Segmentos de la barra de dirección con el sistema de archivos `env`.
    pub fn crumbs_in(&self, env: &dyn FileSystem) -> Vec<(String, Location)> {
        let mut out = vec![("Este equipo".to_string(), Location::Computer)];
        match self {
            Location::Computer => {}
            Location::Path(p) => {
                let mut acc = PathBuf::new();
                for comp in p.components() {
                    acc.push(comp.as_os_str());
                    let label = comp.as_os_str().to_string_lossy().into_owned();
                    match comp {
                        // La raíz sola ("/" o "\"): si el sistema la nombra es un tramo propio (el
                        // disco de arranque de macOS); si no ("C:" y "\" de Windows llegan como
                        // dos componentes), se pega al tramo anterior.
                        Component::RootDir => match env.root_title(Path::new(comp.as_os_str())) {
                            Some(name) => out.push((name, Location::Path(acc.clone()))),
                            None => out.last_mut().unwrap().1 = Location::Path(acc.clone()),
                        },
                        // Una unidad se muestra como en el Explorador: "Disco local (C:)".
                        Component::Prefix(_) => out.push((env.root_title(Path::new(&label)).unwrap_or(label), Location::Path(acc.clone()))),
                        _ => out.push((label, Location::Path(acc.clone()))),
                    }
                }
            }
            Location::Shell { name, parent, .. } => {
                if let Some(parent) = parent {
                    out = parent.crumbs_in(env);
                }
                out.push((name.clone(), self.clone()));
            }
        }
        out
    }

    /// ¿Esta ubicación es la carpeta `p`? Compara con `same_path` del sistema (sin distinguir
    /// mayúsculas ni separador final).
    pub fn is_path(&self, p: &Path) -> bool {
        matches!(self, Location::Path(own) if sys::file_system().same_path(own, p))
    }

    /// Ruta o nombre de análisis que entiende `IFileOperation` como destino (pegar, soltar).
    pub fn target(&self) -> Option<PathBuf> {
        match self {
            Location::Computer => None,
            Location::Path(p) => Some(p.clone()),
            Location::Shell { parsing, .. } => Some(PathBuf::from(parsing)),
        }
    }
}

/// Un elemento de la lista de una carpeta.
pub struct Entry {
    /// Nombre real en disco (se usa para abrir y navegar).
    pub name: OsString,
    /// Ruta completa o nombre de análisis del Shell, cuando no sale de "carpeta + nombre"
    /// (carpetas virtuales, accesos de "Este equipo").
    pub full: Option<OsString>,
    /// Grupo de orden: va antes que cualquier columna (en "Este equipo": carpetas,
    /// unidades, dispositivos, lugares del Shell).
    pub group: u8,
    /// PIDL absoluto de los elementos del Shell (ver `shell::Item`).
    pub pidl: Option<Arc<[u8]>>,
    /// Nombre para mostrar (en unidades, "Disco local (C:)").
    pub display: String,
    /// `display` en minúsculas, para ordenar y buscar sin recalcular.
    pub sort_key: String,
    pub is_dir: bool,
    pub size: u64,
    /// FILETIME en UTC; 0 si no aplica.
    pub modified: u64,
    /// Fecha de creación (FILETIME en UTC); 0 si no aplica.
    pub created: u64,
    pub attrs: u32,
    pub type_name: Arc<str>,
    /// Índice en la lista de imágenes del sistema.
    pub icon: i32,
    /// Superposición del ícono (0 = ninguna).
    pub overlay: u8,
    /// Ya se consultó la superposición (se hace solo para las filas que se llegan a ver).
    pub overlay_checked: bool,
}

impl Entry {
    pub fn is_hidden(&self) -> bool {
        const HIDDEN: u32 = 0x2;
        self.attrs & HIDDEN != 0
    }
}

pub enum LoadMsg {
    /// Un lote de elementos (en orden de llegada).
    Entries(Vec<Entry>),
    /// Ya se listó todo. Error si no se pudo abrir la carpeta.
    Listed(Result<(), String>),
    /// Íconos individuales resueltos después: (posición en orden de llegada, ícono).
    Icons(Vec<(u32, i32)>),
    /// Superposiciones: (posición, índice de superposición).
    Overlays(Vec<(u32, u8)>),
    /// Avance de una búsqueda: elementos revisados y fracción estimada del recorrido (0 a 1).
    Progress { scanned: u64, done: f32 },
}

pub struct LoadEvent {
    pub panel: usize,
    pub generation: u64,
    pub msg: LoadMsg,
}

/// Una carpeta del Acceso rápido de Windows.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickItem {
    pub name: String,
    pub path: PathBuf,
    /// Anclada (`System.Home.IsPinned`); si no, es una carpeta frecuente.
    pub pinned: bool,
}

/// Pedido de superposiciones de íconos (flecha de acceso directo, estado de OneDrive, Git...)
/// para elementos de una pestaña. El resultado vuelve como `LoadMsg::Overlays` por el canal de
/// carga (ver `sys::Overlays`).
pub struct OverlayRequest {
    pub hwnd: Handle,
    pub tx: Sender<LoadEvent>,
    pub panel: usize,
    pub generation: u64,
    /// (posición del elemento, ruta).
    pub items: Vec<(u32, PathBuf)>,
}

/// Quita los `.` y resuelve los `..` de una ruta escrita a mano, sin tocar el disco
/// ("C:\a\b\.." → "C:\a"). Un `..` en la raíz se ignora; en una ruta relativa sin nada que
/// quitar, se conserva.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::Prefix(_) | Component::RootDir) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// ¿Tiene extensión .zip?
pub fn is_zip(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// ¿Es una ruta de red (`\\servidor\recurso\...`)? Los prefijos `\\?\` y `\\.\` no cuentan.
pub fn is_unc(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.starts_with(r"\\") && !s.starts_with(r"\\?\") && !s.starts_with(r"\\.\")
}

// ---------- búsqueda en subcarpetas ----------

/// Busca en `roots` y todas sus subcarpetas los elementos cuyo nombre pasa `matches` (recibe el
/// nombre en minúsculas). Corre en un hilo y manda lo encontrado igual que una carga
/// (`LoadMsg::Entries`, con la ruta completa en `full`), más `LoadMsg::Progress` cada tanto, y
/// termina con `LoadMsg::Listed`. Se abandona si `current` deja de valer `generation`.
///
/// Recorre a lo ancho (primero lo más cercano). El avance se estima repartiendo el peso de cada
/// carpeta entre ella y sus subcarpetas: no hace falta contar todo antes de empezar. Lo propio de
/// cada sistema (ocultos, qué carpetas se recorren, tipo e ícono) lo responde `sys::file_system()`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_search(
    hwnd: Handle,
    tx: Sender<LoadEvent>,
    panel: usize,
    generation: u64,
    current: Arc<AtomicU64>,
    roots: Vec<PathBuf>,
    show_hidden: bool,
    matches: impl Fn(&str) -> bool + Send + 'static,
) {
    std::thread::spawn(move || {
        let fs = sys::file_system();
        fs.with_search_thread(Box::new(move || {
            let cancelled = || current.load(Ordering::Relaxed) != generation;
            let send = |msg: LoadMsg| {
                if tx.send(LoadEvent { panel, generation, msg }).is_ok() {
                    hwnd.post(WM_APP_LOAD, panel, 0);
                }
            };
            let weight = 1.0 / roots.len().max(1) as f64;
            let mut queue: VecDeque<(PathBuf, f64)> = roots.into_iter().map(|r| (r, weight)).collect();
            let (mut scanned, mut done) = (0u64, 0.0f64);
            let mut found: Vec<Entry> = Vec::new();
            let mut last = Instant::now();
            while let Some((dir, w)) = queue.pop_front() {
                if cancelled() {
                    return;
                }
                let Ok(reader) = std::fs::read_dir(&dir) else {
                    done += w;
                    continue;
                };
                let mut subdirs = Vec::new();
                for item in reader.flatten() {
                    if cancelled() {
                        return;
                    }
                    scanned += 1;
                    let name = item.file_name();
                    let path = item.path();
                    let Some(meta) = fs.search_metadata(&item, &path) else { continue };
                    if !show_hidden && fs.is_hidden(&meta, &name) {
                        continue;
                    }
                    // Los enlaces a carpetas no se recorren (pueden formar ciclos).
                    let real_dir = item.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink());
                    // Tampoco entra en otros discos montados adentro (en macOS, /System/Volumes/Data).
                    if real_dir && fs.searchable_dir(&name) && !fs.is_mount_point(&path) {
                        subdirs.push(path.clone());
                    }
                    if matches(&name.to_string_lossy().to_lowercase()) {
                        found.push(fs.disk_entry(&path, name, &meta));
                    }
                    if last.elapsed() >= Duration::from_millis(80) {
                        if !found.is_empty() {
                            send(LoadMsg::Entries(std::mem::take(&mut found)));
                        }
                        send(LoadMsg::Progress { scanned, done: done as f32 });
                        last = Instant::now();
                    }
                }
                // La carpeta se queda con una parte de su peso; el resto, en partes iguales, para
                // cada subcarpeta (que lo vuelve a repartir al leerse).
                let share = w / (subdirs.len() as f64 + 1.0);
                done += share;
                queue.extend(subdirs.into_iter().map(|d| (d, share)));
            }
            if !found.is_empty() {
                send(LoadMsg::Entries(found));
            }
            send(LoadMsg::Progress { scanned, done: 1.0 });
            send(LoadMsg::Listed(Ok(())));
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::FakeFileSystem;

    fn path(p: &str) -> Location {
        Location::Path(PathBuf::from(p))
    }

    /// Con el sistema de archivos falso las raíces no tienen nombre propio: el título es el texto de la ruta
    /// y "/" se pega a «Este equipo» (como "\" sin unidad en Windows).
    #[test]
    fn titles_and_crumbs_with_the_fake_env() {
        assert_eq!(Location::Computer.title_in(&FakeFileSystem), "Este equipo");
        assert_eq!(path("/a/b").title_in(&FakeFileSystem), "b");
        assert_eq!(path("/").title_in(&FakeFileSystem), "/");
        assert_eq!(
            path("/a/b").crumbs_in(&FakeFileSystem),
            [("Este equipo".to_string(), path("/")), ("a".to_string(), path("/a")), ("b".to_string(), path("/a/b"))]
        );
        let bin = Location::Shell { parsing: "::{bin}".into(), name: "Papelera".into(), parent: Some(Box::new(Location::Computer)) };
        assert_eq!(bin.title_in(&FakeFileSystem), "Papelera");
        assert_eq!(bin.crumbs_in(&FakeFileSystem), [("Este equipo".to_string(), Location::Computer), ("Papelera".to_string(), bin.clone())]);
    }

    #[test]
    fn parent_and_target() {
        assert_eq!(path("/a/b").parent(), Some(path("/a")));
        assert_eq!(path("/").parent(), Some(Location::Computer));
        assert_eq!(Location::Computer.parent(), None);
        let bin = Location::Shell { parsing: "::{bin}".into(), name: "Papelera".into(), parent: None };
        assert_eq!(bin.parent(), Some(Location::Computer));
        assert_eq!(bin.target(), Some(PathBuf::from("::{bin}")));
        assert_eq!(Location::Computer.target(), None);
    }

    // ---------- búsqueda en subcarpetas (con el `FileSystem` falso) ----------

    /// Corre una búsqueda y junta todo lo que manda hasta que termina.
    fn run_search(root: &Path, matches: impl Fn(&str) -> bool + Send + 'static, show_hidden: bool) -> (Vec<String>, f32) {
        crate::sys::testing::install_fakes();
        let (tx, rx) = std::sync::mpsc::channel();
        let current = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(7));
        spawn_search(crate::sys::Handle(0), tx, 1, 7, current, vec![root.to_path_buf()], show_hidden, matches);
        let (mut names, mut progress) = (Vec::new(), 0.0);
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            match ev.msg {
                LoadMsg::Entries(batch) => {
                    for e in batch {
                        let full = PathBuf::from(e.full.expect("con ruta completa"));
                        names.push(full.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                    }
                }
                LoadMsg::Progress { done, .. } => progress = done,
                LoadMsg::Listed(_) => break,
                _ => {}
            }
        }
        names.sort();
        (names, progress)
    }

    /// La búsqueda vive en el núcleo y le pide al `FileSystem` instalado los datos de cada
    /// elemento: con el falso, lo que empieza con punto está oculto en todos los sistemas.
    #[test]
    fn search_runs_in_core_with_fake_fs() {
        let root = std::env::temp_dir().join(format!("xdir-core-busqueda-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["a/b/c", "informes/2024", ".oculta"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["informe.txt", "a/b/c/Informe final.pdf", "a/nada.txt", "informes/2024/foto.jpg", ".oculta/informe.txt"] {
            std::fs::write(root.join(file), "x").unwrap();
        }

        let (names, progress) = run_search(&root, |n| n.contains("informe"), false);
        // La carpeta "informes" también coincide; lo de la carpeta oculta no se ve.
        assert_eq!(names, ["a/b/c/Informe final.pdf", "informe.txt", "informes"]);
        assert_eq!(progress, 1.0);

        // Mostrando ocultos.
        let (names, _) = run_search(&root, |n| n.ends_with(".txt"), true);
        assert_eq!(names, [".oculta/informe.txt", "a/nada.txt", "informe.txt"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `is_path` compara con el `same_path` del sistema instalado (el falso: sin distinguir
    /// mayúsculas ni separador final).
    #[test]
    fn is_path_uses_the_file_system() {
        crate::sys::testing::install_fakes();
        assert!(path("/a/B/").is_path(Path::new("/A/b")));
        assert!(!path("/a/b").is_path(Path::new("/a/c")));
        assert!(!Location::Computer.is_path(Path::new("/")));
    }
}

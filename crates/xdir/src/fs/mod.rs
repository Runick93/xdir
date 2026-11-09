//! Lectura de carpetas en segundo plano y vigilancia de cambios.
//!
//! Cada lectura corre en su propio hilo y manda los resultados por un canal, avisando a la
//! ventana con `WM_APP_LOAD`. La interfaz nunca espera al disco.
//!
//! Acá están los tipos comunes; la lectura en sí es de cada sistema (`win32.rs`, `macos.rs`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::platform::WM_APP;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

// Nombre del temporal del ayudante elevado de xtree: lo usa Windows; las pruebas, en todos.
#[cfg(any(windows, test))]
mod xtree_temp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Hay mensajes nuevos en el canal de carga. `wparam` = panel.
pub const WM_APP_LOAD: u32 = WM_APP + 1;
/// Cambió algo en la carpeta vigilada. `wparam` = panel, `lparam` = generación.
pub const WM_APP_CHANGED: u32 = WM_APP + 2;

pub use xdir_core::fs::Location;

/// `Location::is_path`: compara con `same_path` del sistema (sin distinguir mayúsculas ni
/// separador final), que todavía vive en `fs`.
pub trait LocationExt {
    /// ¿Esta ubicación es la carpeta `p`?
    fn is_path(&self, p: &Path) -> bool;
}

impl LocationExt for Location {
    fn is_path(&self, p: &Path) -> bool {
        matches!(self, Location::Path(own) if same_path(own, p))
    }
}

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

#[cfg_attr(not(windows), allow(dead_code))]
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

/// Quita los `.` y resuelve los `..` de una ruta escrita a mano, sin tocar el disco
/// ("C:\a\b\.." → "C:\a"). Un `..` en la raíz se ignora; en una ruta relativa sin nada que
/// quitar, se conserva.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component;
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
/// carpeta entre ella y sus subcarpetas: no hace falta contar todo antes de empezar.
#[allow(clippy::too_many_arguments)]
pub fn spawn_search(
    hwnd: crate::platform::Handle,
    tx: std::sync::mpsc::Sender<LoadEvent>,
    panel: usize,
    generation: u64,
    current: Arc<std::sync::atomic::AtomicU64>,
    roots: Vec<PathBuf>,
    show_hidden: bool,
    matches: impl Fn(&str) -> bool + Send + 'static,
) {
    use std::collections::VecDeque;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    std::thread::spawn(move || {
        with_search_thread(|| {
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
                    let Some(meta) = search_metadata(&item, &path) else { continue };
                    if !show_hidden && is_hidden(&meta, &name) {
                        continue;
                    }
                    // Los enlaces a carpetas no se recorren (pueden formar ciclos).
                    let real_dir = item.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink());
                    // Tampoco entra en otros discos montados adentro (en macOS, /System/Volumes/Data).
                    if real_dir && searchable_dir(&name) && !is_mount_point(&path) {
                        subdirs.push(path.clone());
                    }
                    if matches(&name.to_string_lossy().to_lowercase()) {
                        found.push(disk_entry(&path, name, &meta));
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
        });
    });
}


#[cfg(test)]
mod search_tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc::channel;

    /// Corre una búsqueda y junta todo lo que manda hasta que termina.
    fn run(root: &Path, query: &str, show_hidden: bool) -> (Vec<String>, f32) {
        // La búsqueda avisa a la ventana con `Handle::post`: necesita un `Waker` (el falso).
        xdir_core::sys::testing::install_fakes();
        let (tx, rx) = channel();
        let current = Arc::new(AtomicU64::new(7));
        spawn_search(crate::platform::Handle(0), tx, 1, 7, current, vec![root.to_path_buf()], show_hidden, crate::list::name_matcher(query));
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

    #[test]
    fn finds_matches_in_every_level() {
        let root = std::env::temp_dir().join(format!("xdir-busqueda-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["a/b/c", "informes/2024", ".oculta"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["informe.txt", "a/b/c/Informe final.pdf", "a/nada.txt", "informes/2024/foto.jpg", ".oculta/informe.txt"] {
            std::fs::write(root.join(file), "x").unwrap();
        }
        // En macOS lo que empieza con punto ya está oculto; en Windows se oculta con el atributo.
        #[cfg(windows)]
        assert!(std::process::Command::new("attrib").arg("+h").arg(root.join(".oculta")).status().unwrap().success());

        let (names, progress) = run(&root, "informe", false);
        // La carpeta "informes" también coincide; lo de la carpeta oculta no se ve.
        assert_eq!(names, ["a/b/c/Informe final.pdf", "informe.txt", "informes"]);
        assert_eq!(progress, 1.0);

        // Con patrones y mostrando ocultos.
        let (names, _) = run(&root, "*.txt", true);
        assert_eq!(names, [".oculta/informe.txt", "a/nada.txt", "informe.txt"]);

        let _ = std::fs::remove_dir_all(&root);
    }
}

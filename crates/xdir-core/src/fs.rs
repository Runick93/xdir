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

/// Título de la pestaña Configuración.
pub const SETTINGS_TITLE: &str = "Configuración";

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
    /// La pestaña Configuración: no se carga ni se vigila; dibuja `settings::draw` en vez de la
    /// lista (su página y su estado van en `Tab::settings`).
    Settings,
    /// Dentro de un archivo comprimido (solo lectura): `file` es el archivo del disco, `nested`
    /// la ruta interior (con `/`) de cada archivo comprimido metido dentro del anterior y `inner`
    /// la carpeta interior que se ve (con `/`, sin barra final; vacía en la raíz).
    Archive { file: PathBuf, nested: Vec<String>, inner: String },
}

/// Une una carpeta interior con un nombre (`""` + `a` → `a`, `a` + `b` → `a/b`).
pub fn inner_join(inner: &str, name: &str) -> String {
    if inner.is_empty() { name.to_string() } else { format!("{inner}/{name}") }
}

/// Último tramo de una ruta interior (`a/b/c.zip` → `c.zip`).
fn inner_last(inner: &str) -> &str {
    inner.rsplit('/').next().unwrap_or(inner)
}

/// La carpeta de una ruta interior (`a/b/c.zip` → `a/b`; `c.zip` → ``).
fn inner_parent(inner: &str) -> &str {
    inner.rfind('/').map_or("", |i| &inner[..i])
}

impl Location {
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Computer | Location::Settings => None,
            Location::Path(p) => Some(match p.parent() {
                Some(parent) => Location::Path(parent.to_path_buf()),
                None => Location::Computer,
            }),
            Location::Shell { parent, .. } => Some(parent.as_deref().cloned().unwrap_or(Location::Computer)),
            Location::Archive { file, nested, inner } => Some(if !inner.is_empty() {
                Location::Archive { file: file.clone(), nested: nested.clone(), inner: inner_parent(inner).to_string() }
            } else if let Some((last, rest)) = nested.split_last() {
                Location::Archive { file: file.clone(), nested: rest.to_vec(), inner: inner_parent(last).to_string() }
            } else {
                match file.parent() {
                    Some(p) => Location::Path(p.to_path_buf()),
                    None => Location::Computer,
                }
            }),
        }
    }

    /// Lo que hay adentro de esta ubicación con nombre `name`, si es una carpeta de un archivo
    /// comprimido: la subcarpeta.
    pub fn archive_child(&self, name: &str) -> Option<Location> {
        let Location::Archive { file, nested, inner } = self else { return None };
        Some(Location::Archive { file: file.clone(), nested: nested.clone(), inner: inner_join(inner, name) })
    }

    /// Entrar en el archivo comprimido `name` que está dentro de esta carpeta de un archivo
    /// comprimido.
    pub fn archive_nested(&self, name: &str) -> Option<Location> {
        let Location::Archive { file, nested, inner } = self else { return None };
        let mut nested = nested.clone();
        nested.push(inner_join(inner, name));
        Some(Location::Archive { file: file.clone(), nested, inner: String::new() })
    }

    /// ¿Está dentro de un archivo comprimido? (solo lectura)
    pub fn is_archive(&self) -> bool {
        matches!(self, Location::Archive { .. })
    }

    /// Texto de la barra de dirección: la ruta, o para un archivo comprimido la del archivo con
    /// lo de adentro («/x/fotos.zip/2024/viaje»).
    pub fn address_text(&self) -> String {
        match self {
            Location::Computer => "Este equipo".to_string(),
            Location::Settings => SETTINGS_TITLE.into(),
            Location::Path(p) => p.to_string_lossy().into_owned(),
            Location::Shell { parsing, .. } => parsing.clone(),
            Location::Archive { file, nested, inner } => {
                let mut s = file.to_string_lossy().into_owned();
                for part in nested.iter().map(String::as_str).chain([inner.as_str()]).filter(|p| !p.is_empty()) {
                    s.push('/');
                    s.push_str(part);
                }
                s
            }
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
            Location::Settings => SETTINGS_TITLE.into(),
            Location::Path(p) => match p.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => env.root_title(p).unwrap_or_else(|| p.to_string_lossy().trim_end_matches('\\').to_string()),
            },
            Location::Shell { name, .. } => name.clone(),
            Location::Archive { file, nested, inner } => match (inner.is_empty(), nested.last()) {
                (false, _) => inner_last(inner).to_string(),
                (true, Some(last)) => inner_last(last).to_string(),
                (true, None) => file.file_name().map_or_else(|| file.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned()),
            },
        }
    }

    /// Segmentos de la barra de dirección: (texto, destino), con el sistema de archivos instalado.
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        self.crumbs_in(sys::file_system())
    }

    /// Segmentos de la barra de dirección con el sistema de archivos `env`.
    pub fn crumbs_in(&self, env: &dyn FileSystem) -> Vec<(String, Location)> {
        if *self == Location::Settings {
            return vec![(SETTINGS_TITLE.to_string(), Location::Settings)];
        }
        let mut out = vec![("Este equipo".to_string(), Location::Computer)];
        match self {
            Location::Computer | Location::Settings => {}
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
            Location::Archive { file, nested, inner } => {
                // Las del archivo, después un tramo por carpeta y por archivo anidado de adentro.
                if let Some(parent) = file.parent() {
                    out = Location::Path(parent.to_path_buf()).crumbs_in(env);
                }
                let name = file.file_name().map_or_else(|| file.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
                out.push((name, Location::Archive { file: file.clone(), nested: Vec::new(), inner: String::new() }));
                let mut chain: Vec<String> = Vec::new();
                let walk = |out: &mut Vec<(String, Location)>, chain: &[String], path: &str, last_enters: bool| {
                    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
                    let mut acc = String::new();
                    for (i, part) in parts.iter().enumerate() {
                        acc = inner_join(&acc, part);
                        let loc = if last_enters && i + 1 == parts.len() {
                            let mut n = chain.to_vec();
                            n.push(acc.clone());
                            Location::Archive { file: file.clone(), nested: n, inner: String::new() }
                        } else {
                            Location::Archive { file: file.clone(), nested: chain.to_vec(), inner: acc.clone() }
                        };
                        out.push((part.to_string(), loc));
                    }
                };
                for n in nested {
                    walk(&mut out, &chain, n, true);
                    chain.push(n.clone());
                }
                walk(&mut out, &chain, inner, false);
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
            Location::Computer | Location::Settings => None,
            Location::Path(p) => Some(p.clone()),
            Location::Shell { parsing, .. } => Some(PathBuf::from(parsing)),
            // Solo lectura: no es destino de nada.
            Location::Archive { .. } => None,
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
    /// Tamaño comprimido de un elemento de adentro de un archivo comprimido (columna «Tamaño
    /// comprimido»); 0 si no aplica o el formato no lo dice.
    pub packed: u64,
}

/// Atributo de enlace (`FILE_ATTRIBUTE_REPARSE_POINT` en Windows): las cargas lo ponen en los
/// enlaces simbólicos y, en macOS, en los alias del Finder.
pub const ATTR_LINK: u32 = 0x400;

/// Atributo propio de un elemento de un archivo comprimido con nombre no permitido (sale de la
/// carpeta, es absoluto, es un nombre reservado…): se muestra pero no se abre ni se copia.
pub const ATTR_ARCHIVE_BAD: u32 = 0x4000_0000;

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
    /// Metadatos leídos (dimensiones, duración, autor…): (índice en la lista, datos). Llegan en
    /// lotes después de un `sys::Metadata::request`.
    Metadata(Vec<(usize, Meta)>),
    /// Avance de una búsqueda: elementos revisados y fracción estimada del recorrido (0 a 1).
    Progress { scanned: u64, done: f32 },
    /// Un aviso de la carga para la barra de estado («Se muestran los primeros 100.000
    /// elementos»).
    Notice(String),
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

/// Metadatos de un elemento que no salen de leer la carpeta: los de las columnas Dimensiones,
/// Duración, Autor, Etiquetas, Artista y Álbum. Vacío (`Default`) si el sistema no tiene datos.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// Ancho y alto en píxeles (imágenes y videos).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Duración en milisegundos (audio y video).
    pub duration_ms: Option<u64>,
    pub authors: Vec<String>,
    /// Etiquetas (palabras clave en Windows, etiquetas del Finder en macOS).
    pub tags: Vec<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Qué metadatos se piden (uno por columna nueva).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum MetaField {
    Dimensions,
    Duration,
    Authors,
    Tags,
    Artist,
    Album,
}

impl MetaField {
    pub const ALL: [MetaField; 6] = [MetaField::Dimensions, MetaField::Duration, MetaField::Authors, MetaField::Tags, MetaField::Artist, MetaField::Album];
}

/// Cuántos elementos va como mucho en un pedido de metadatos (y en cada lote de respuesta).
pub const META_BATCH: usize = 200;

/// Las respuestas de un servicio real salen en lotes de hasta `META_REPLY_BATCH` elementos, o con
/// lo que haya cada `META_REPLY_EVERY` (así una carpeta lenta igual se va llenando).
pub const META_REPLY_BATCH: usize = 50;
pub const META_REPLY_EVERY: Duration = Duration::from_millis(100);

/// Pedido de metadatos para elementos de una pestaña (ver `sys::Metadata`). La respuesta vuelve
/// como `LoadMsg::Metadata` por el canal de carga, en lotes, con la misma `generation`: si la
/// pestaña ya cargó otra cosa, la descarta.
pub struct MetadataRequest {
    pub hwnd: Handle,
    pub tx: Sender<LoadEvent>,
    pub panel: usize,
    pub generation: u64,
    /// La generación actual de la pestaña: si ya no es `generation`, el pedido quedó viejo y se
    /// deja de leer (ver `is_stale`).
    pub current: Arc<AtomicU64>,
    /// (índice del elemento en la lista, ruta).
    pub items: Vec<(usize, PathBuf)>,
    pub fields: Vec<MetaField>,
}

impl MetadataRequest {
    /// Manda un lote de respuestas y despierta al hilo de interfaz. `false` si la pestaña ya no
    /// escucha (se puede dejar de leer).
    pub fn send(&self, batch: Vec<(usize, Meta)>) -> bool {
        let ok = self.tx.send(LoadEvent { panel: self.panel, generation: self.generation, msg: LoadMsg::Metadata(batch) }).is_ok();
        if ok {
            self.hwnd.post(WM_APP_LOAD, self.panel, 0);
        }
        ok
    }

    /// ¿La pestaña ya cargó otra cosa? Entonces la respuesta se descartaría: no vale leer más.
    pub fn is_stale(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.generation
    }

    /// Responde leyendo cada elemento con `read` (en el hilo que llama, que no debe ser el de
    /// interfaz): manda lotes de `META_REPLY_BATCH` o lo juntado cada `META_REPLY_EVERY`. Deja de
    /// leer si el pedido quedó viejo o si la pestaña ya no escucha. Si `read` entra en pánico con
    /// un elemento, ese va sin datos y se sigue: el hilo del servicio no se muere.
    pub fn answer_with(self, mut read: impl FnMut(&Path) -> Meta) {
        let mut batch = Vec::new();
        let mut last = Instant::now();
        for (i, path) in &self.items {
            if self.is_stale() {
                return;
            }
            let meta = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(path))).unwrap_or_default();
            batch.push((*i, meta));
            if batch.len() >= META_REPLY_BATCH || last.elapsed() >= META_REPLY_EVERY {
                if !self.send(std::mem::take(&mut batch)) {
                    return;
                }
                last = Instant::now();
            }
        }
        if !batch.is_empty() && !self.is_stale() {
            self.send(batch);
        }
    }

    /// Responde sin datos para todos los elementos (sistemas sin de dónde leerlos): las columnas
    /// quedan vacías y los grupos en «Sin valor» en vez de «Leyendo…».
    pub fn answer_empty(self) {
        for chunk in self.items.chunks(META_BATCH) {
            if !self.send(chunk.iter().map(|(i, _)| (*i, Meta::default())).collect()) {
                return;
            }
        }
    }
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

/// ¿Es una carpeta? `None` si la ruta es de red (`Network::is_remote`, sin E/S): en el hilo de
/// interfaz no se toca; quien llama navega igual y deja que la carga (en su hilo) decida.
pub fn probe_dir(p: &Path) -> Option<bool> {
    (!crate::sys::network().is_remote(p)).then(|| p.is_dir())
}

/// Como `probe_dir`, para un archivo.
pub fn probe_file(p: &Path) -> Option<bool> {
    (!crate::sys::network().is_remote(p)).then(|| p.is_file())
}

/// Como `probe_dir`, para saber si existe.
pub fn probe_exists(p: &Path) -> Option<bool> {
    (!crate::sys::network().is_remote(p)).then(|| p.exists())
}

/// La forma larga de una ruta (`long_path`), salvo las de red: en Windows `GetLongPathNameW` va
/// al servidor y las rutas de red no tienen nombres 8.3 que valga la pena expandir.
pub fn long_path_local(p: &Path) -> PathBuf {
    if crate::sys::network().is_remote(p) { p.to_path_buf() } else { crate::sys::file_system().long_path(p) }
}

/// Las mayúsculas reales del disco (`true_case`), salvo en rutas de red (quedan como se
/// escribieron).
pub fn true_case_local(p: &Path) -> PathBuf {
    if crate::sys::network().is_remote(p) { p.to_path_buf() } else { crate::sys::file_system().true_case(p) }
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
    search_with(sys::file_system(), hwnd, tx, panel, generation, current, roots, show_hidden, matches);
}

/// `spawn_search` con un sistema de archivos dado (el instalado, o el real de una plataforma en
/// sus pruebas).
#[allow(clippy::too_many_arguments)]
pub fn search_with(
    fs: &'static dyn FileSystem,
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

    /// Dentro de un archivo comprimido: título, migas (las del archivo, más cada carpeta y cada
    /// archivo anidado) y subir hasta la carpeta del archivo.
    #[test]
    fn archive_parent_title_crumbs() {
        let dir = PathBuf::from("/a");
        let file = dir.join("fotos.zip");
        let at = |nested: &[&str], inner: &str| Location::Archive { file: file.clone(), nested: nested.iter().map(|s| s.to_string()).collect(), inner: inner.into() };
        let deep = at(&["2024/viejo.zip"], "x/y");
        assert_eq!(deep.title_in(&FakeFileSystem), "y");
        assert_eq!(at(&["2024/viejo.zip"], "").title_in(&FakeFileSystem), "viejo.zip");
        assert_eq!(at(&[], "").title_in(&FakeFileSystem), "fotos.zip");
        let crumbs = deep.crumbs_in(&FakeFileSystem);
        let names: Vec<&str> = crumbs.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, ["Este equipo", "a", "fotos.zip", "2024", "viejo.zip", "x", "y"]);
        assert_eq!(crumbs[3].1, at(&[], "2024"));
        assert_eq!(crumbs[4].1, at(&["2024/viejo.zip"], ""));
        assert_eq!(crumbs[6].1, deep);
        // Subir: x/y → x → raíz del anidado → su carpeta en el de afuera → raíz → carpeta del disco.
        let ups: Vec<Location> = std::iter::successors(Some(deep.clone()), |l| l.parent()).take(6).collect();
        assert_eq!(ups[1], at(&["2024/viejo.zip"], "x"));
        assert_eq!(ups[2], at(&["2024/viejo.zip"], ""));
        assert_eq!(ups[3], at(&[], "2024"));
        assert_eq!(ups[4], at(&[], ""));
        assert_eq!(ups[5], Location::Path(dir.clone()));
        assert_eq!(deep.target(), None);
        assert_eq!(at(&[], "").archive_child("b"), Some(at(&[], "b")));
        assert_eq!(at(&[], "b").archive_nested("c.zip"), Some(at(&["b/c.zip"], "")));
        assert_eq!(deep.address_text(), format!("{}/2024/viejo.zip/x/y", file.display()));
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

    /// Pedido de metadatos de `n` elementos con su canal, para probar `answer_with`.
    fn meta_request(n: usize) -> (MetadataRequest, std::sync::mpsc::Receiver<LoadEvent>, Arc<AtomicU64>) {
        crate::sys::testing::install_fakes();
        let (tx, rx) = std::sync::mpsc::channel();
        let current = Arc::new(AtomicU64::new(7));
        let items = (0..n).map(|i| (i, PathBuf::from(format!("/m/{i}")))).collect();
        let req = MetadataRequest { hwnd: Handle(0), tx, panel: 3, generation: 7, current: current.clone(), items, fields: MetaField::ALL.to_vec() };
        (req, rx, current)
    }

    fn batch_sizes(rx: &std::sync::mpsc::Receiver<LoadEvent>) -> Vec<usize> {
        rx.try_iter()
            .map(|e| {
                assert_eq!((e.panel, e.generation), (3, 7));
                match e.msg {
                    LoadMsg::Metadata(b) => b.len(),
                    _ => panic!("se esperaba un lote de metadatos"),
                }
            })
            .collect()
    }

    /// Lecturas rápidas: la respuesta va en lotes de 50, en orden, con lo que leyó cada ruta.
    #[test]
    fn metadata_answer_in_batches_of_50() {
        let (req, rx, _) = meta_request(120);
        req.answer_with(|p| Meta { width: p.file_name().and_then(|n| n.to_str()?.parse().ok()), ..Meta::default() });
        let all: Vec<LoadEvent> = rx.try_iter().collect();
        let sizes: Vec<usize> = all.iter().map(|e| if let LoadMsg::Metadata(b) = &e.msg { b.len() } else { 0 }).collect();
        assert_eq!(sizes, [50, 50, 20]);
        let got: Vec<(usize, Option<u32>)> =
            all.into_iter().flat_map(|e| if let LoadMsg::Metadata(b) = e.msg { b } else { Vec::new() }).map(|(i, m)| (i, m.width)).collect();
        assert_eq!(got, (0..120).map(|i| (i, Some(i as u32))).collect::<Vec<_>>());
    }

    /// Lecturas lentas: no se espera a juntar 50, se manda lo que haya cada 100 ms.
    #[test]
    fn metadata_answer_every_100ms() {
        let (req, rx, _) = meta_request(3);
        req.answer_with(|_| {
            std::thread::sleep(Duration::from_millis(110));
            Meta::default()
        });
        assert_eq!(batch_sizes(&rx), [1, 1, 1]);
    }

    /// Si la pestaña ya cargó otra cosa (otra generación), se deja de leer.
    #[test]
    fn metadata_answer_stops_when_stale() {
        let (req, rx, current) = meta_request(120);
        let mut reads = 0;
        req.answer_with(|_| {
            reads += 1;
            if reads == 10 {
                current.store(8, Ordering::Relaxed);
            }
            Meta::default()
        });
        assert_eq!(reads, 10);
        assert!(batch_sizes(&rx).iter().sum::<usize>() <= 10);
    }

    /// Si leer un elemento entra en pánico, ese elemento va sin datos y se siguen leyendo los
    /// demás: el hilo de metadatos no se muere.
    #[test]
    fn metadata_answer_survives_a_panicking_read() {
        let (req, rx, _) = meta_request(3);
        req.answer_with(|p| {
            if p.ends_with("1") {
                panic!("lectura rota");
            }
            Meta { width: Some(5), ..Meta::default() }
        });
        let got: Vec<(usize, Option<u32>)> =
            rx.try_iter().flat_map(|e| if let LoadMsg::Metadata(b) = e.msg { b } else { Vec::new() }).map(|(i, m)| (i, m.width)).collect();
        assert_eq!(got, [(0, Some(5)), (1, None), (2, Some(5))]);
    }

    /// Si la pestaña ya no escucha (canal cerrado), se deja de leer.
    #[test]
    fn metadata_answer_stops_when_nobody_listens() {
        let (req, rx, _) = meta_request(200);
        drop(rx);
        let mut reads = 0;
        req.answer_with(|_| {
            reads += 1;
            Meta::default()
        });
        assert_eq!(reads, 50);
    }
}

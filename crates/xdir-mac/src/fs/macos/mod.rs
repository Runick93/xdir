//! macOS: lectura de carpetas con `std::fs` y vigilancia de cambios con FSEvents (crate `notify`).
//!
//! "Este equipo" muestra las carpetas del usuario, los discos (el de arranque y los de
//! `/Volumes`) y la Papelera. No hay carpetas virtuales del Shell: `Location::Shell` no se usa.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::darwin::fs::MetadataExt as DarwinMetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use objc2_foundation::{NSFileManager, NSString};

use super::*;
use crate::icons::{self, IconKey};
use crate::platform::Handle;

/// Atributos como los de Windows (los usa la lista para ocultos y carpetas).
const ATTR_HIDDEN: u32 = 0x2;
const ATTR_DIRECTORY: u32 = 0x10;
/// `UF_HIDDEN` de `chflags`: oculto en el Finder aunque el nombre no empiece con punto.
const UF_HIDDEN: u32 = 0x8000;

/// Lanza la lectura de `location` en un hilo. Se abandona si `current` deja de valer `generation`.
pub fn spawn_load(hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
    std::thread::spawn(move || {
        let mut out = Batcher { hwnd, tx, panel, generation, current, pending: Vec::new(), last: Instant::now() };
        match &location {
            Location::Computer => list_computer(&mut out),
            Location::Path(path) => list_dir(&mut out, path),
            Location::Shell { .. } => out.send(LoadMsg::Listed(Err("Esta ubicación no existe en macOS".into()))),
        }
    });
}

struct Batcher {
    hwnd: Handle,
    tx: Sender<LoadEvent>,
    panel: usize,
    generation: u64,
    current: Arc<AtomicU64>,
    pending: Vec<Entry>,
    last: Instant,
}

impl Batcher {
    fn cancelled(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.generation
    }

    fn push(&mut self, e: Entry) {
        self.pending.push(e);
        // Primer lote enseguida para que la lista aparezca rápido; después cada ~40 ms.
        if self.pending.len() >= 4096 || self.last.elapsed() >= Duration::from_millis(40) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            let batch = std::mem::take(&mut self.pending);
            self.send(LoadMsg::Entries(batch));
        }
        self.last = Instant::now();
    }

    fn send(&self, msg: LoadMsg) {
        let event = LoadEvent { panel: self.panel, generation: self.generation, msg };
        if self.tx.send(event).is_ok() {
            self.hwnd.post(WM_APP_LOAD, self.panel, 0);
        }
    }
}

/// Tiempo del sistema → FILETIME (centésimas de microsegundo desde 1601), como en Windows.
fn filetime(t: std::io::Result<SystemTime>) -> u64 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    match t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) => (d.as_secs() + EPOCH_DIFF) * 10_000_000 + d.subsec_nanos() as u64 / 100,
        None => 0,
    }
}

fn extension(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| name[dot + 1..].to_lowercase())
}

/// Carpetas que el Finder muestra como un solo archivo (apps, bibliotecas de Fotos...).
fn is_package(ext: Option<&str>) -> bool {
    matches!(
        ext,
        Some(
            "app" | "bundle" | "framework" | "plugin" | "kext" | "pkg" | "mpkg" | "rtfd" | "photoslibrary" | "musiclibrary"
                | "imovielibrary" | "fcpbundle" | "logicx" | "band" | "xcodeproj" | "xcworkspace" | "playground"
                | "pages" | "numbers" | "key" | "prefpane" | "appex" | "saver" | "qlgenerator" | "mdimporter" | "xpc"
        )
    )
}

/// ¿Está oculto? Nombre con punto o marcado oculto en el Finder.
pub fn is_hidden(meta: &std::fs::Metadata, name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.') || meta.st_flags() & UF_HIDDEN != 0
}

/// Nombre del tipo de archivo como lo muestra el Finder ("Documento PDF"), cacheado por extensión.
fn type_name(ext: Option<&str>, is_dir: bool, package: bool) -> Arc<str> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<str>>>> = OnceLock::new();
    let key = match (is_dir && !package, ext) {
        (true, _) => "\\dir".to_string(),
        (false, Some(e)) => e.to_string(),
        (false, None) => String::new(),
    };
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let name: Arc<str> = if key == "\\dir" {
        Arc::from("Carpeta")
    } else {
        let described = ext.and_then(|e| {
            let t = objc2_uniform_type_identifiers::UTType::typeWithFilenameExtension(&NSString::from_str(e))?;
            t.localizedDescription().map(|d| capitalize_first(&d.to_string()))
        });
        Arc::from(described.unwrap_or_else(|| match ext {
            Some(e) => format!("Archivo {}", e.to_uppercase()),
            None => "Documento".into(),
        }))
    };
    cache.lock().unwrap().insert(key, name.clone());
    name
}

/// Primera letra en mayúscula: el sistema describe algunos tipos en minúscula («texto») y otros
/// no («Documento PDF»); en la columna Tipo van todos igual.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Entrada de la lista para un elemento de disco.
fn entry_for(path: &Path, name: OsString, meta: &std::fs::Metadata, group: u8, display: Option<String>) -> Entry {
    let display = display.unwrap_or_else(|| name.to_string_lossy().into_owned());
    let ext = extension(&name.to_string_lossy());
    let package = meta.is_dir() && is_package(ext.as_deref());
    let is_dir = meta.is_dir() && !package;
    let mut attrs = if is_dir { ATTR_DIRECTORY } else { 0 };
    if is_hidden(meta, &name) {
        attrs |= ATTR_HIDDEN;
    }
    let icon = if package || group > 0 {
        icons::index_of(IconKey::Path(path.to_path_buf()))
    } else if is_dir {
        icons::index_of(IconKey::Folder)
    } else {
        match &ext {
            Some(e) => icons::index_of(IconKey::Ext(e.clone())),
            None => icons::index_of(IconKey::File),
        }
    };
    Entry {
        sort_key: display.to_lowercase(),
        display,
        name,
        full: None,
        group,
        pidl: None,
        is_dir,
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified: filetime(meta.modified()),
        created: filetime(meta.created()),
        attrs,
        type_name: type_name(ext.as_deref(), meta.is_dir(), package),
        icon,
        overlay: 0,
        overlay_checked: true,
    }
}

/// Prepara el hilo de una búsqueda y corre `f` (en macOS no hace falta nada).
pub fn with_search_thread(f: impl FnOnce()) {
    f();
}

/// Datos de un elemento encontrado por la búsqueda: los enlaces simbólicos, de lo que apuntan.
pub fn search_metadata(item: &std::fs::DirEntry, path: &Path) -> Option<std::fs::Metadata> {
    std::fs::metadata(path).or_else(|_| item.metadata()).ok()
}

/// ¿Es la raíz de otro disco montado adentro (la búsqueda no cruza)? En APFS los volúmenes del
/// sistema y de datos comparten el identificador de dispositivo, así que se pregunta el punto de
/// montaje.
pub fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return false;
    }
    let mount = unsafe { std::ffi::CStr::from_ptr(st.f_mntonname.as_ptr()) };
    Path::new(std::ffi::OsStr::from_bytes(mount.to_bytes())) == path
}

/// ¿La búsqueda entra en esta carpeta? No en las que el Finder muestra como archivo (apps...).
pub fn searchable_dir(name: &OsStr) -> bool {
    !is_package(extension(&name.to_string_lossy()).as_deref())
}

/// Entrada de la lista para un elemento de disco fuera de la carpeta del panel (resultado de
/// búsqueda): con su ruta completa.
pub fn disk_entry(path: &Path, name: OsString, meta: &std::fs::Metadata) -> Entry {
    let mut e = entry_for(path, name, meta, 0, None);
    e.full = Some(path.as_os_str().to_os_string());
    e
}

fn list_dir(out: &mut Batcher, path: &Path) {
    let reader = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(e) => {
            out.send(LoadMsg::Listed(Err(e.to_string())));
            return;
        }
    };
    for item in reader {
        if out.cancelled() {
            return;
        }
        let Ok(item) = item else { continue };
        let full = item.path();
        // Los enlaces simbólicos se muestran como lo que apuntan (si existe).
        let Ok(meta) = std::fs::metadata(&full).or_else(|_| item.metadata()) else { continue };
        out.push(entry_for(&full, item.file_name(), &meta, 0, None));
    }
    out.flush();
    out.send(LoadMsg::Listed(Ok(())));
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Carpetas del usuario (mismo orden que en Windows: Escritorio, Descargas, Documentos,
/// Imágenes, Música, Vídeos).
const USER_FOLDERS: [&str; 6] = ["Desktop", "Downloads", "Documents", "Pictures", "Music", "Movies"];

/// Ruta de la carpeta del usuario `i`, si existe.
pub fn user_folder(i: usize) -> Option<PathBuf> {
    let p = home()?.join(USER_FOLDERS.get(i)?);
    p.is_dir().then_some(p)
}

/// Nombre que muestra el Finder para un elemento ("Descargas", "Macintosh HD").
fn display_name(path: &Path) -> String {
    let fm = NSFileManager::defaultManager();
    fm.displayNameAtPath(&NSString::from_str(&path.to_string_lossy())).to_string()
}

/// "Este equipo": carpeta personal y del usuario, discos y Papelera.
fn list_computer(out: &mut Batcher) {
    let mut folders: Vec<PathBuf> = home().into_iter().collect();
    folders.extend((0..USER_FOLDERS.len()).filter_map(user_folder));
    for path in folders {
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let mut e = entry_for(&path, path.clone().into_os_string(), &meta, 0, Some(display_name(&path)));
        e.full = Some(path.clone().into_os_string());
        e.icon = icons::index_of(IconKey::Path(path.clone()));
        out.push(e);
    }
    out.flush();
    for root in drives() {
        let Ok(meta) = std::fs::metadata(&root) else { continue };
        let mut e = entry_for(&root, root.clone().into_os_string(), &meta, 1, Some(drive_label(&drive_name(&root))));
        e.full = Some(root.clone().into_os_string());
        e.size = volume_size(&root);
        e.type_name = Arc::from("Disco");
        out.push(e);
        out.flush();
    }
    if let Some(trash) = home().map(|h| h.join(".Trash")).filter(|p| p.is_dir()) {
        if let Ok(meta) = std::fs::metadata(&trash) {
            let mut e = entry_for(&trash, trash.clone().into_os_string(), &meta, 3, Some("Papelera".into()));
            e.full = Some(trash.into_os_string());
            e.attrs &= !ATTR_HIDDEN;
            out.push(e);
        }
    }
    out.flush();
    out.send(LoadMsg::Listed(Ok(())));
}

/// Capacidad total de un disco, en bytes.
fn volume_size(root: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(root.as_os_str().as_bytes()) else { return 0 };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    st.f_blocks * st.f_bsize as u64
}

/// Discos montados: el de arranque ("/") y los de `/Volumes` (sin el enlace al de arranque).
pub fn drives() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/")];
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        let mut vols: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| !std::fs::read_link(p).is_ok_and(|t| t == Path::new("/")))
            .filter(|p| p.is_dir())
            .collect();
        vols.sort();
        out.extend(vols);
    }
    out
}

/// Identificador de un disco para `drive_label`: su ruta de montaje.
pub fn drive_name(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

static DRIVE_LABELS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Nombre de un disco ("/" → "Macintosh HD"), cacheado.
pub fn drive_label(drive: &str) -> String {
    let cache = DRIVE_LABELS.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(drive) {
        return hit.clone();
    }
    let label = display_name(Path::new(drive));
    let label = if label.is_empty() { drive.to_string() } else { label };
    cache.lock().unwrap().insert(drive.to_string(), label.clone());
    label
}

/// Olvida los nombres de disco guardados (cambió el conjunto de discos).
pub fn forget_drive_labels() {
    if let Some(cache) = DRIVE_LABELS.get() {
        cache.lock().unwrap().clear();
    }
}

/// ¿Es una ruta de un disco externo que ya no está montado?
pub fn drive_missing(path: &Path) -> bool {
    let mut comps = path.components();
    let (Some(std::path::Component::RootDir), Some(first), Some(vol)) = (comps.next(), comps.next(), comps.next()) else { return false };
    first.as_os_str() == "Volumes" && !Path::new("/Volumes").join(vol.as_os_str()).exists()
}

/// Subcarpetas de un lugar, para el menú de las flechas de la barra de dirección.
pub fn subfolders(loc: &Location) -> Vec<(String, Location)> {
    const MAX: usize = 500;
    match loc {
        Location::Computer => drives().into_iter().map(|d| (drive_label(&drive_name(&d)), Location::Path(d))).collect(),
        Location::Path(dir) => {
            let mut out: Vec<(String, Location)> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|name| (name.clone(), Location::Path(dir.join(name))))
                .collect();
            out.sort_by_key(|(n, _)| n.to_lowercase());
            out.truncate(MAX);
            out
        }
        Location::Shell { .. } => Vec::new(),
    }
}

/// Ícono genérico de carpeta.
pub fn folder_icon() -> Option<i32> {
    Some(icons::index_of(IconKey::Folder))
}

/// Lo escrito en la barra de dirección como lugar especial: en macOS no hay.
pub fn shell_location(_text: &str) -> Option<Location> {
    None
}

/// Carpetas virtuales de Windows: no existen en macOS.
pub const LINUX_PLACE: &str = "";
pub const NETWORK_PLACE: &str = "";

pub fn shell_place_exists(_parsing: &str) -> bool {
    false
}

pub fn shell_subfolders(_parsing: &str) -> Vec<(String, PathBuf)> {
    Vec::new()
}

/// Acceso rápido: es de Windows (la barra lateral del Finder no tiene una API pública).
pub fn quick_access() -> Vec<QuickItem> {
    Vec::new()
}

pub fn is_quick_pinned(_path: &Path) -> bool {
    false
}

pub fn quick_access_watch_dir() -> Option<PathBuf> {
    None
}

/// Forma larga de una ruta: en macOS no hay nombres cortos.
pub fn long_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Ruta escrita a mano con las mayúsculas reales del disco (APFS no distingue mayúsculas por
/// defecto): cada tramo se busca en su carpeta. Si algo falla, queda como estaba.
pub fn true_case(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(name) => {
                let found = std::fs::read_dir(if out.as_os_str().is_empty() { Path::new(".") } else { &out })
                    .ok()
                    .and_then(|rd| {
                        rd.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().to_lowercase() == name.to_string_lossy().to_lowercase())
                    });
                out.push(found.unwrap_or_else(|| name.to_os_string()));
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Conectar un recurso de red con credenciales: en macOS se hace desde el Finder.
pub fn connect_share(_owner: Handle, _path: &Path) -> bool {
    false
}

/// Misma ruta: sin distinguir mayúsculas (APFS por defecto) ni barra final.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy();
        let t = s.trim_end_matches('/');
        if t.is_empty() { "/".to_string() } else { t.to_lowercase() }
    };
    norm(a) == norm(b)
}

/// Vigila una carpeta y avisa a la ventana con un mensaje (`FileSystem::watch`). Se detiene al soltarlo.
pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
}

impl xdir_core::sys::Watch for Watcher {}

impl Watcher {
    /// Vigila una carpeta y avisa a la ventana con el mensaje `msg` (y estos `wparam`/`lparam`).
    pub fn with_message(hwnd: Handle, msg: u32, panel: usize, generation: isize, path: &Path) -> Option<Self> {
        use notify::Watcher as _;
        let (tx, rx) = channel::<()>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .ok()?;
        watcher.watch(path, notify::RecursiveMode::NonRecursive).ok()?;
        // Junta ráfagas de cambios (copias de muchos archivos) en un solo aviso. El hilo termina
        // cuando se suelta el vigilante (se cierra el canal).
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(250));
                while rx.try_recv().is_ok() {}
                hwnd.post(msg, panel, generation);
            }
        });
        Some(Self { _watcher: watcher })
    }
}

// ---------- xtree (escáner de uso de disco) ----------

mod xtree;
pub use self::xtree::*;

#[cfg(test)]
mod type_name_tests {
    use super::capitalize_first;

    #[test]
    fn type_names_start_with_capital() {
        assert_eq!(capitalize_first("texto"), "Texto");
        assert_eq!(capitalize_first("Documento PDF"), "Documento PDF");
        assert_eq!(capitalize_first("ícono"), "Ícono");
        assert_eq!(capitalize_first(""), "");
    }
}

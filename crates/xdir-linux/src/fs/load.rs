//! Lectura de carpetas en un hilo, con el mismo protocolo que macOS: lotes de `LoadMsg::Entries`
//! (el primero enseguida, después cada ~40 ms o 4096 elementos) y al final `LoadMsg::Listed`.
//! Cada mensaje despierta a la ventana con `WM_APP_LOAD`; si `current` deja de valer
//! `generation`, la carga se abandona sin avisar.

use std::ffi::{OsStr, OsString};
use std::fs::Metadata;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use xdir_core::fs::{Entry, LoadEvent, LoadMsg, Location, WM_APP_LOAD};
use xdir_core::sys::Handle;

use super::places::{drive_label, drive_name, drives, home, trash_dir, user_folder, volume_size, USER_FOLDER_COUNT};

/// Atributos como los de Windows (los usa la lista para ocultos y carpetas).
pub const ATTR_HIDDEN: u32 = 0x2;
pub const ATTR_DIRECTORY: u32 = 0x10;

/// Lanza la lectura de `location` en un hilo.
pub fn spawn_load(hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
    std::thread::spawn(move || {
        let mut out = Batcher { hwnd, tx, panel, generation, current, pending: Vec::new(), last: Instant::now() };
        match &location {
            Location::Computer => list_computer(&mut out),
            Location::Path(path) => list_dir(&mut out, path),
            Location::Shell { .. } | Location::Settings | Location::Archive { .. } => out.send(LoadMsg::Listed(Err("Esta ubicación no existe en Linux".into()))),
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
        // Se manda un lote cada ~40 ms (el primero también: `last` arranca al empezar la carga) o
        // al juntar 4096 entradas, lo que pase antes.
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

/// Tiempo del sistema → FILETIME (centésimas de microsegundo desde 1601), la unidad del núcleo.
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

/// ¿Está oculto? En Linux, solo por el nombre: empieza con punto.
pub fn is_hidden(name: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().first() == Some(&b'.')
}

/// Ícono (ninguno todavía) y tipo de un elemento de adentro de un archivo comprimido.
pub fn archive_look(name: &str, is_dir: bool) -> (i32, Arc<str>) {
    let ext = if is_dir { None } else { extension(name) };
    (0, type_name(ext.as_deref(), is_dir))
}

/// Nombre del tipo de archivo: "Carpeta", "Archivo PDF" o "Documento" (sin extensión).
fn type_name(ext: Option<&str>, is_dir: bool) -> Arc<str> {
    Arc::from(match (is_dir, ext) {
        (true, _) => "Carpeta".to_string(),
        (false, Some(e)) => format!("Archivo {}", e.to_uppercase()),
        (false, None) => "Documento".to_string(),
    })
}

/// Entrada de la lista para un elemento de disco. Sin íconos del sistema: índice 0.
pub fn entry_for(name: OsString, meta: &Metadata, group: u8, display: Option<String>) -> Entry {
    let display = display.unwrap_or_else(|| name.to_string_lossy().into_owned());
    let ext = extension(&name.to_string_lossy());
    let is_dir = meta.is_dir();
    let mut attrs = if is_dir { ATTR_DIRECTORY } else { 0 };
    if is_hidden(&name) {
        attrs |= ATTR_HIDDEN;
    }
    Entry {
        sort_key: display.to_lowercase(),
        display,
        name,
        full: None,
        group,
        pidl: None,
        is_dir,
        size: if is_dir { 0 } else { meta.len() },
        modified: filetime(meta.modified()),
        created: filetime(meta.created()),
        attrs,
        type_name: type_name(ext.as_deref(), is_dir),
        icon: 0,
        overlay: 0,
        overlay_checked: true,
        packed: 0,
    }
}

/// Datos de un elemento: los enlaces simbólicos, de lo que apuntan; si el enlace está roto, del
/// enlace mismo (`symlink_metadata`), para que igual aparezca en la lista.
pub fn metadata(item: &std::fs::DirEntry, path: &Path) -> Option<Metadata> {
    std::fs::metadata(path).or_else(|_| item.metadata()).ok()
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
        let Some(meta) = metadata(&item, &full) else { continue };
        let mut e = entry_for(item.file_name(), &meta, 0, None);
        // Enlaces simbólicos, para «Mostrar el original».
        if item.file_type().is_ok_and(|t| t.is_symlink()) {
            e.attrs |= xdir_core::fs::ATTR_LINK;
        }
        out.push(e);
    }
    out.flush();
    out.send(LoadMsg::Listed(Ok(())));
}

/// Nombre para mostrar de una carpeta de "Este equipo": su último tramo.
fn last_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// "Este equipo": carpeta personal y del usuario, discos y Papelera (como en macOS).
fn list_computer(out: &mut Batcher) {
    let mut folders: Vec<_> = home().into_iter().collect();
    folders.extend((0..USER_FOLDER_COUNT).filter_map(user_folder));
    for path in folders {
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let mut e = entry_for(path.clone().into_os_string(), &meta, 0, Some(last_name(&path)));
        e.full = Some(path.into_os_string());
        out.push(e);
    }
    out.flush();
    for root in drives() {
        if out.cancelled() {
            return;
        }
        let Ok(meta) = std::fs::metadata(&root) else { continue };
        let mut e = entry_for(root.clone().into_os_string(), &meta, 1, Some(drive_label(&drive_name(&root))));
        e.size = volume_size(&root).map_or(0, |(total, _)| total);
        e.type_name = Arc::from("Disco");
        e.full = Some(root.into_os_string());
        out.push(e);
        out.flush();
    }
    if let Some(trash) = trash_dir()
        && let Ok(meta) = std::fs::metadata(&trash)
    {
        let mut e = entry_for(trash.clone().into_os_string(), &meta, 3, Some("Papelera".into()));
        e.full = Some(trash.into_os_string());
        e.attrs &= !ATTR_HIDDEN;
        out.push(e);
    }
    out.flush();
    out.send(LoadMsg::Listed(Ok(())));
}

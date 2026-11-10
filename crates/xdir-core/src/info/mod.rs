//! Panel de información (Alt+Enter / ⌘I): lo que se sabe de los elementos seleccionados del panel
//! activo (tipo, tamaños, fechas, casillas, etiquetas, comentario, «Abrir con», permisos, disco).
//!
//! Todo se lee en otro hilo (`sys::Info::request`) con el patrón de generación de la vista previa:
//! cada pedido lleva una generación y la respuesta de una selección vieja no se manda. Nada de lo
//! que se lee abre el contenido: un elemento solo en la nube se informa con `cloud_only` y sus
//! datos salen de los metadatos del sistema de archivos. Los cambios (`InfoChange`) también se
//! escriben en otro hilo (`sys::Info::apply`) y el error vuelve como texto para la sección.
//!
//! Este módulo es neutro: los tipos, la lectura mínima con `std::fs` (`read_minimal`, la que usa
//! cada plataforma para lo que no tiene de dónde leer), la mezcla de varios elementos (`merge`) y
//! los permisos como texto. El tamaño de una carpeta se suma en `size`, y el dibujo está en `draw`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::sys::Handle;

pub mod draw;
pub mod size;

#[cfg(test)]
mod tests;

/// Llegó información del servicio (`wparam` 0) o pasó la pausa después de un cambio de sujeto
/// (`wparam` 1).
pub const WM_APP_INFO: u32 = crate::WM_APP + 29;
/// Terminó la edición de una caja del panel (nombre, comentario, etiqueta nueva). `wparam` = 1 si
/// hay que aplicarla.
pub const WM_APP_INFO_EDIT_DONE: u32 = crate::WM_APP + 30;

/// Una casilla con varios elementos: apagada en todos, prendida en todos o mezclada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tri {
    Off,
    On,
    Mixed,
}

/// Casillas de General. Cada sistema informa las suyas (ver la Decisión 8 del spec).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flag {
    /// macOS: `uchg` (`NSURLIsUserImmutableKey`).
    Locked,
    /// Windows: atributo; macOS: `UF_HIDDEN`; Linux: nombre con punto (solo para ver).
    Hidden,
    /// Windows: atributo; Linux: sin la escritura del dueño.
    ReadOnly,
}

impl Flag {
    pub fn label(self) -> &'static str {
        match self {
            Flag::Locked => "Bloqueado",
            Flag::Hidden => "Oculto",
            Flag::ReadOnly => "Solo lectura",
        }
    }
}

/// Una etiqueta (del Finder, palabra clave de Windows o `user.xdg.tags`) y su color 0xRRGGBB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub color: Option<u32>,
}

/// Una app que puede abrir el elemento. `id` lo entiende solo la plataforma que la listó.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRef {
    pub id: String,
    pub name: String,
    pub icon: i32,
}

/// «Abrir con»: la app actual, las que pueden abrirlo y si elegir una queda para el archivo
/// (macOS) o lo abre una vez (Windows y Linux). `system_dialog`: «Cambiar todos…» abre el diálogo
/// del sistema, que elige la app (Windows); si no, se elige en el panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWith {
    pub current: Option<AppRef>,
    pub apps: Vec<AppRef>,
    pub per_file: bool,
    pub system_dialog: bool,
}

/// Dueño, grupo y modo (bits de Unix: `0o640`…). `editable`: se puede cambiar (`chmod`); si no,
/// solo se ve (Windows: el dueño y si el usuario actual puede leer y escribir).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Perms {
    pub owner: String,
    pub group: String,
    pub mode: u32,
    pub editable: bool,
    pub can_read: bool,
    pub can_write: bool,
}

/// Datos de un disco entero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeData {
    pub total: u64,
    pub free: u64,
    pub format: String,
}

/// Lo que se sabe de un elemento. Las fechas son FILETIME (0: no hay). `tags`/`comment` en
/// `None`: el formato o el sistema no los admite. `flags`: (casilla, prendida, editable).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ItemInfo {
    pub path: PathBuf,
    pub kind_name: String,
    pub size: u64,
    pub on_disk: u64,
    pub created: u64,
    pub modified: u64,
    pub last_opened: u64,
    /// La última apertura es el último acceso del sistema (Windows y Linux), que puede no
    /// actualizarse.
    pub last_opened_is_access: bool,
    pub is_dir: bool,
    pub cloud_only: bool,
    pub tags: Option<Vec<Tag>>,
    pub comment: Option<String>,
    /// «Ocultar la extensión»: solo macOS.
    pub hidden_ext: Option<bool>,
    pub flags: Vec<(Flag, bool, bool)>,
    pub open_with: Option<OpenWith>,
    pub perms: Option<Perms>,
    /// Más información propia del sistema (rótulo, valor).
    pub more: Vec<(&'static str, String)>,
    pub volume: Option<VolumeData>,
}

/// Las secciones del panel, en el orden en que se dibujan. Cada una es un bit de
/// `informacion_secciones` en la sesión (abierta = 1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    Tags,
    General,
    More,
    Name,
    Comments,
    OpenWith,
    Perms,
}

impl Section {
    pub const ALL: [Section; 7] = [Section::Tags, Section::General, Section::More, Section::Name, Section::Comments, Section::OpenWith, Section::Perms];

    pub fn bit(self) -> u32 {
        1 << (self as u32)
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Tags => "Etiquetas",
            Section::General => "General",
            Section::More => "Más información",
            Section::Name => "Nombre y extensión",
            Section::Comments => "Comentarios",
            Section::OpenWith => "Abrir con",
            Section::Perms => "Compartir y permisos",
        }
    }
}

/// Secciones abiertas la primera vez (como en el diseño: Nombre y Comentarios plegadas).
pub const DEFAULT_OPEN: u32 = (1 << Section::Tags as u32)
    | (1 << Section::General as u32)
    | (1 << Section::More as u32)
    | (1 << Section::OpenWith as u32)
    | (1 << Section::Perms as u32);

/// Un cambio pedido desde el panel (se escribe en otro hilo con `sys::Info::apply`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoChange {
    Tags { paths: Vec<PathBuf>, add: Vec<String>, remove: Vec<String> },
    Comment { path: PathBuf, text: String },
    Flag { paths: Vec<PathBuf>, flag: Flag, on: bool },
    HideExt { path: PathBuf, on: bool },
    /// Elegir una app: queda para el archivo (macOS) o lo abre una vez con ella.
    OpenWith { paths: Vec<PathBuf>, app: String },
    /// «Cambiar todos…»: la app por defecto del tipo (en Windows, el diálogo del sistema).
    ChangeAll { path: PathBuf, app: String },
    Perms { paths: Vec<PathBuf>, mode: u32, recursive: bool },
}

impl InfoChange {
    /// La sección donde se muestra el error de este cambio.
    pub fn section(&self) -> Section {
        match self {
            InfoChange::Tags { .. } => Section::Tags,
            InfoChange::Comment { .. } => Section::Comments,
            InfoChange::Flag { .. } => Section::General,
            InfoChange::HideExt { .. } => Section::Name,
            InfoChange::OpenWith { .. } | InfoChange::ChangeAll { .. } => Section::OpenWith,
            InfoChange::Perms { .. } => Section::Perms,
        }
    }
}

/// Respuestas del servicio y del hilo que suma el tamaño de una carpeta.
#[derive(Debug)]
pub enum InfoReply {
    Items { generation: u64, items: Vec<ItemInfo> },
    /// Avance (o final, con `done`) de la suma de una carpeta; `skipped`: carpetas que no se
    /// pudieron leer (no están en la cuenta). La generación es la de la suma, no la del pedido.
    FolderSize { generation: u64, path: PathBuf, bytes: u64, files: u64, folders: u64, skipped: u64, done: bool },
    /// Terminó un cambio: `error` es el texto para la sección.
    Applied { generation: u64, section: Section, error: Option<String> },
}

/// Pedido de información (ver `sys::Info::request`).
pub struct InfoRequest {
    pub hwnd: Handle,
    pub tx: Sender<InfoReply>,
    pub generation: u64,
    /// La generación vigente del panel: si ya no es `generation`, el pedido quedó viejo.
    pub current: Arc<AtomicU64>,
    pub paths: Vec<PathBuf>,
}

impl InfoRequest {
    /// ¿Ya se pidió otra cosa? Entonces no vale leer ni responder.
    pub fn is_stale(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.generation
    }

    /// Manda la respuesta y despierta al hilo de interfaz con `WM_APP_INFO`; si el pedido quedó
    /// viejo, no manda nada.
    pub fn send(self, items: Vec<ItemInfo>) {
        if self.is_stale() {
            return;
        }
        if self.tx.send(InfoReply::Items { generation: self.generation, items }).is_ok() {
            self.hwnd.post(WM_APP_INFO, 0, 0);
        }
    }
}

/// Manda el resultado de un cambio (desde el hilo que lo aplicó).
pub fn send_applied(owner: Handle, tx: &Sender<InfoReply>, generation: u64, section: Section, error: Option<String>) {
    if tx.send(InfoReply::Applied { generation, section, error }).is_ok() {
        owner.post(WM_APP_INFO, 0, 0);
    }
}

/// Responde el pedido en el hilo que llama (que no debe ser el de interfaz) leyendo cada ruta con
/// `read`. Si se pide otra cosa a mitad de camino, deja de leer. Si `read` entra en pánico, el
/// elemento queda con lo mínimo.
pub fn answer(req: InfoRequest, read: &dyn Fn(&Path) -> ItemInfo) {
    let mut items = Vec::with_capacity(req.paths.len());
    for p in &req.paths {
        if req.is_stale() {
            return;
        }
        let item = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(p))).unwrap_or_else(|_| ItemInfo { path: p.clone(), ..ItemInfo::default() });
        items.push(item);
    }
    req.send(items);
}

/// Lo mínimo con `std::fs`, sin abrir nada: tamaño, fechas y si es carpeta. Lo que está solo en la
/// nube se lee sin seguir el enlace (`symlink_metadata`). El tipo queda vacío (la `App` pone el de
/// la lista) y no hay etiquetas, comentario, casillas ni «Abrir con».
pub fn read_minimal(path: &Path, cloud_only: &dyn Fn(&Path) -> bool) -> ItemInfo {
    let cloud = cloud_only(path);
    let meta = if cloud { std::fs::symlink_metadata(path) } else { std::fs::metadata(path).or_else(|_| std::fs::symlink_metadata(path)) };
    let mut info = ItemInfo { path: path.to_path_buf(), cloud_only: cloud, ..ItemInfo::default() };
    if let Ok(m) = meta {
        info.is_dir = m.is_dir();
        if !info.is_dir {
            info.size = m.len();
            info.on_disk = m.len();
        }
        info.created = filetime(m.created());
        info.modified = filetime(m.modified());
        info.last_opened = filetime(m.accessed());
        info.last_opened_is_access = true;
    }
    info
}

/// Tiempo del sistema → FILETIME (centésimas de microsegundo desde 1601); 0 si no hay.
pub fn filetime(t: std::io::Result<SystemTime>) -> u64 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    match t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) => (d.as_secs() + EPOCH_DIFF) * 10_000_000 + d.subsec_nanos() as u64 / 100,
        None => 0,
    }
}

/// Segundos de Unix → FILETIME (0 si es 0).
pub fn filetime_from_unix(secs: i64) -> u64 {
    const EPOCH_DIFF: i64 = 11_644_473_600;
    if secs == 0 { 0 } else { ((secs + EPOCH_DIFF).max(0) as u64) * 10_000_000 }
}

/// Varios elementos juntos: cantidad, tamaños sumados, tipo si es el mismo, cada casilla (las que
/// tienen todos) con su estado mezclado y las etiquetas que tienen todos.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub size: u64,
    pub on_disk: u64,
    pub kind: Option<String>,
    pub flags: Vec<(Flag, Tri, bool)>,
    pub tags: Vec<Tag>,
}

/// Mezcla de `items` (puro).
pub fn merge(items: &[ItemInfo]) -> Summary {
    let size = items.iter().map(|i| i.size).sum();
    let on_disk = items.iter().map(|i| i.on_disk).sum();
    let kind = items.first().map(|i| i.kind_name.clone()).filter(|k| !k.is_empty() && items.iter().all(|i| i.kind_name == *k));
    let mut flags = Vec::new();
    if let Some(first) = items.first() {
        for &(flag, _, _) in &first.flags {
            let states: Option<Vec<(bool, bool)>> = items.iter().map(|i| i.flags.iter().find(|f| f.0 == flag).map(|f| (f.1, f.2))).collect();
            let Some(states) = states else { continue };
            let on = states.iter().filter(|s| s.0).count();
            let tri = if on == 0 {
                Tri::Off
            } else if on == states.len() {
                Tri::On
            } else {
                Tri::Mixed
            };
            flags.push((flag, tri, states.iter().all(|s| s.1)));
        }
    }
    let tags = match items.first().and_then(|i| i.tags.as_ref()) {
        Some(first) => first.iter().filter(|t| items.iter().all(|i| i.tags.as_ref().is_some_and(|ts| ts.iter().any(|x| x.name == t.name)))).cloned().collect(),
        None => Vec::new(),
    };
    Summary { count: items.len(), size, on_disk, kind, flags, tags }
}

/// Acceso de una clase (dueño, grupo, todos) en el texto del panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    ReadWrite,
    ReadOnly,
    WriteOnly,
    None,
}

impl Access {
    /// Las que se ofrecen en el menú del panel.
    pub const CHOICES: [Access; 3] = [Access::ReadWrite, Access::ReadOnly, Access::None];

    pub fn label(self) -> &'static str {
        match self {
            Access::ReadWrite => "Lectura y escritura",
            Access::ReadOnly => "Solo lectura",
            Access::WriteOnly => "Solo escritura",
            Access::None => "Sin acceso",
        }
    }

    fn bits(self) -> u32 {
        match self {
            Access::ReadWrite => 0o6,
            Access::ReadOnly => 0o4,
            Access::WriteOnly => 0o2,
            Access::None => 0,
        }
    }
}

/// Acceso de la clase `who` (0 dueño, 1 grupo, 2 todos) en `mode`.
pub fn access_of(mode: u32, who: usize) -> Access {
    let shift = 6 - 3 * who as u32;
    match (mode >> shift) & 0o6 {
        0o6 => Access::ReadWrite,
        0o4 => Access::ReadOnly,
        0o2 => Access::WriteOnly,
        _ => Access::None,
    }
}

/// `mode` con el acceso de `who` cambiado (la ejecución y los bits especiales quedan igual).
pub fn with_access(mode: u32, who: usize, access: Access) -> u32 {
    let shift = 6 - 3 * who as u32;
    (mode & !(0o6 << shift)) | (access.bits() << shift)
}

/// Texto para leer y escribir del usuario actual (Windows, permisos solo para ver).
pub fn rw_text(can_read: bool, can_write: bool) -> &'static str {
    match (can_read, can_write) {
        (true, true) => Access::ReadWrite.label(),
        (true, false) => Access::ReadOnly.label(),
        (false, true) => Access::WriteOnly.label(),
        (false, false) => Access::None.label(),
    }
}

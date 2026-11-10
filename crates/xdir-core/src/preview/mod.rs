//! Vista previa: lo que la columna de vista previa necesita además de la imagen grande (texto,
//! cantidad de elementos de una carpeta, espacio de un disco, último acceso, «solo en la nube»,
//! sin permiso). Se lee en otro hilo (`sys::Preview`) con `answer`, que es neutro (`std::fs`):
//! cada plataforma solo aporta su chequeo de nube. La respuesta lleva la `generation` del pedido
//! y la de una selección vieja no se manda.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::sys::{self, Handle};

pub mod text;

#[cfg(test)]
pub(crate) mod tests;

/// Llegó una vista previa: lo manda el hilo del servicio y la `App` la toma del canal.
pub const WM_APP_PREVIEW: u32 = crate::WM_APP + 24;

/// Cuánto texto se lee como mucho (64 KB) y cuántas líneas se guardan.
pub const TEXT_MAX: usize = 65_536;
pub const TEXT_LINES: usize = 200;

/// Lo que se mira para decidir si un archivo sin extensión de texto es texto.
const SNIFF: usize = 4096;

/// Qué se previsualiza.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewTarget {
    File(PathBuf),
    Folder(PathBuf),
    /// Un disco entero («Este equipo»): su espacio.
    Drive(PathBuf),
}

impl PreviewTarget {
    pub fn path(&self) -> &Path {
        match self {
            PreviewTarget::File(p) | PreviewTarget::Folder(p) | PreviewTarget::Drive(p) => p,
        }
    }
}

/// Lo que se muestra arriba en la columna.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewBody {
    /// La imagen grande del servicio de miniaturas (`Painter::big_image`).
    Image,
    /// Las primeras líneas (hasta `TEXT_LINES`, de los primeros `TEXT_MAX` bytes).
    Text { lines: Vec<String> },
    Folder { items: u64 },
    Drive { total: u64, free: u64 },
    /// Solo en la nube: no se lee (se dibuja el ícono y «Disponible solo en línea»).
    CloudOnly,
    /// Sin permiso, bloqueado o que ya no existe.
    Unreadable,
    /// Sin vista previa: el ícono del tipo.
    Nothing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewInfo {
    pub body: PreviewBody,
    /// Último acceso como FILETIME (0 si no hay).
    pub accessed: u64,
}

/// Respuesta de un pedido, con su generación (la `App` descarta las de una selección vieja).
#[derive(Debug)]
pub struct PreviewReply {
    pub generation: u64,
    pub info: PreviewInfo,
}

/// Pedido de vista previa (ver `sys::Preview`).
pub struct PreviewRequest {
    pub hwnd: Handle,
    pub tx: Sender<PreviewReply>,
    pub generation: u64,
    /// La generación vigente de la columna: si ya no es `generation`, el pedido quedó viejo.
    pub current: Arc<AtomicU64>,
    pub target: PreviewTarget,
}

impl PreviewRequest {
    /// ¿Ya se pidió otra cosa? Entonces no vale leer ni responder.
    pub fn is_stale(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.generation
    }

    /// Manda la respuesta y despierta al hilo de interfaz con `WM_APP_PREVIEW`; si el pedido
    /// quedó viejo, no manda nada.
    pub fn send(self, info: PreviewInfo) {
        if self.is_stale() {
            return;
        }
        if self.tx.send(PreviewReply { generation: self.generation, info }).is_ok() {
            self.hwnd.post(WM_APP_PREVIEW, 0, 0);
        }
    }
}

/// Responde el pedido leyendo con `std::fs` en el hilo que llama (que no debe ser el de
/// interfaz). `cloud_only` dice si un elemento está solo en la nube: entonces no se abre. Si algo
/// entra en pánico, responde `Unreadable` y el hilo del servicio sigue.
pub fn answer(req: PreviewRequest, cloud_only: &dyn Fn(&Path) -> bool) {
    if req.is_stale() {
        return;
    }
    let info = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(&req.target, cloud_only)))
        .unwrap_or(PreviewInfo { body: PreviewBody::Unreadable, accessed: 0 });
    req.send(info);
}

/// Lee lo que muestra la columna para `target`.
pub fn read(target: &PreviewTarget, cloud_only: &dyn Fn(&Path) -> bool) -> PreviewInfo {
    let path = target.path();
    let cloud = !matches!(target, PreviewTarget::Drive(_)) && cloud_only(path);
    // De lo que está en la nube, los datos del elemento sin seguir el enlace (en Windows, sin
    // abrir el marcador de posición, que lo haría bajar).
    let meta = if cloud { std::fs::symlink_metadata(path) } else { std::fs::metadata(path).or_else(|_| std::fs::symlink_metadata(path)) };
    let accessed = meta.as_ref().map_or(0, |m| filetime(m.accessed()));
    let body = match target {
        _ if cloud => PreviewBody::CloudOnly,
        _ if meta.is_err() => PreviewBody::Unreadable,
        PreviewTarget::File(p) => file_body(p),
        PreviewTarget::Folder(p) => match std::fs::read_dir(p) {
            Ok(items) => PreviewBody::Folder { items: items.count() as u64 },
            Err(_) => PreviewBody::Unreadable,
        },
        PreviewTarget::Drive(p) => match sys::disk_scan().volume_space(p) {
            Some((total, free)) => PreviewBody::Drive { total, free },
            None => PreviewBody::Nothing,
        },
    };
    PreviewInfo { body, accessed }
}

/// Texto (por extensión o porque los primeros 4 KB lo son), `Image` (lo demás: lo dibuja el
/// servicio de miniaturas) o `Nothing` si está vacío.
fn file_body(path: &Path) -> PreviewBody {
    let Ok(mut file) = std::fs::File::open(path) else { return PreviewBody::Unreadable };
    let text_ext = path.extension().and_then(|e| e.to_str()).is_some_and(text::is_text_ext)
        // `.gitignore` no tiene extensión: es el nombre entero.
        || path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix('.')).is_some_and(text::is_text_ext);
    // Un byte más que el límite dice si el archivo sigue (y entonces el final puede ser un corte).
    let mut bytes = Vec::new();
    let first = if text_ext { TEXT_MAX } else { SNIFF };
    if (&mut file).take(first as u64 + 1).read_to_end(&mut bytes).is_err() {
        return PreviewBody::Unreadable;
    }
    if bytes.is_empty() {
        return PreviewBody::Nothing;
    }
    if !text_ext {
        let head = &bytes[..bytes.len().min(SNIFF)];
        if text::decode_text(head, bytes.len() > SNIFF).is_none() {
            return PreviewBody::Image;
        }
        // Parece texto: se sigue leyendo hasta el límite.
        if bytes.len() > SNIFF && (&mut file).take((TEXT_MAX - SNIFF) as u64).read_to_end(&mut bytes).is_err() {
            return PreviewBody::Unreadable;
        }
    }
    let cut = bytes.len() > TEXT_MAX;
    bytes.truncate(TEXT_MAX);
    text_body(&bytes, cut)
}

/// Las primeras líneas, o `Image` si no es texto (un `.txt` en otra codificación: el sistema
/// quizás tenga cómo mostrarlo).
fn text_body(bytes: &[u8], cut: bool) -> PreviewBody {
    match text::decode_text(bytes, cut) {
        Some(mut lines) => {
            lines.truncate(TEXT_LINES);
            PreviewBody::Text { lines }
        }
        None => PreviewBody::Image,
    }
}

/// Tiempo del sistema → FILETIME (centésimas de microsegundo desde 1601); 0 si no hay.
fn filetime(t: std::io::Result<SystemTime>) -> u64 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    match t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) => (d.as_secs() + EPOCH_DIFF) * 10_000_000 + d.subsec_nanos() as u64 / 100,
        None => 0,
    }
}

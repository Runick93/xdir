//! Integración con el sistema: portapapeles de archivos, abrir con el programa asociado,
//! propiedades, menú contextual y objetos de datos para arrastrar.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Un elemento para el Shell: su ruta (o nombre de análisis) y, si es virtual, su PIDL.
///
/// El PIDL hace falta cuando la ruta no alcanza para identificarlo en su contexto: un
/// elemento de la Papelera tiene ruta física, pero su menú ("Restaurar") es el de la Papelera.
#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub pidl: Option<Arc<[u8]>>,
}

impl From<PathBuf> for Item {
    fn from(path: PathBuf) -> Self {
        Self { path, pidl: None }
    }
}

/// Entrada propia del menú del fondo (Ver, Ordenar por, Actualizar...): el Shell solo aporta las
/// de las extensiones ("Nuevo", Git...), el resto lo pone la vista de carpeta del Explorador.
pub enum Entry {
    Item { id: u32, label: String, enabled: bool, checked: bool },
    Sub(String, Vec<Entry>),
    Sep,
}

/// Lo que devuelve `context_menu` si se eligió "Desanclar de Acceso rápido" (el Shell solo ofrece
/// ese verbo en el propio Acceso rápido: ahí la aplicación lo resuelve con `unpin_quick_access`).
pub const UNPIN_QUICK: &str = "unpin-quick";

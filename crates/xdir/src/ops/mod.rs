//! Operaciones de archivos (copiar, mover, borrar, renombrar, carpeta nueva y deshacer). Cada
//! operación corre en su propio hilo para que la ventana nunca se congele; al terminar avisa con
//! `WM_APP_OP_DONE`. La implementación es de cada sistema.

use std::path::PathBuf;

use crate::platform::WM_APP;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Terminó una operación; el resultado está en el canal.
pub const WM_APP_OP_DONE: u32 = WM_APP + 4;

pub enum Op {
    Copy { sources: Vec<PathBuf>, dest: PathBuf },
    Move { sources: Vec<PathBuf>, dest: PathBuf },
    Delete { paths: Vec<PathBuf>, recycle: bool },
    Rename { path: PathBuf, new_name: String },
    NewFolder { parent: PathBuf },
    Undo(Undo),
}

/// Cómo deshacer una operación ya hecha.
#[derive(Clone, Debug)]
pub enum Undo {
    /// Volver a llamar `path` como `old_name`.
    Rename { path: PathBuf, old_name: String },
    /// Copias o carpetas nuevas: se mandan a la papelera.
    Created(Vec<PathBuf>),
    /// (dónde quedó, dónde estaba).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// (nombre del elemento dentro de la papelera, dónde estaba).
    Recycled(Vec<(String, PathBuf)>),
}

pub struct OpResult {
    pub undo: Option<Undo>,
    /// Elemento a enfocar al terminar (renombrado o carpeta nueva).
    pub focus: Option<PathBuf>,
    /// Carpeta nueva: se abre el renombrado en línea sobre ella.
    pub rename_after: bool,
    pub error: Option<String>,
}

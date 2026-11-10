//! Operaciones de archivos (copiar, mover, borrar, renombrar, carpeta nueva, archivo de texto nuevo
//! y deshacer). Los tipos son del núcleo; la implementación de macOS (un hilo por operación) se
//! pide con `sys::file_ops()`.

pub use xdir_core::ops::{Op, OpResult, Undo, WM_APP_OP_DONE};

mod macos;
mod prims;
pub(crate) use self::macos::spawn as spawn_native;
pub(crate) use self::prims::MacCopyPrims;

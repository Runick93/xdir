//! Operaciones de archivos (copiar, mover, borrar, renombrar, carpeta nueva y deshacer). Los tipos
//! son del núcleo; la implementación de Windows (un hilo por operación) y se pide con
//! `sys::file_ops()`.

pub use xdir_core::ops::{Op, OpResult, Undo, WM_APP_OP_DONE};

mod prims;
mod win32;
pub(crate) use self::prims::WinCopyPrims;
pub use self::win32::item;
pub(crate) use self::win32::spawn as spawn_native;

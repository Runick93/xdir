//! Operaciones de archivos (copiar, mover, borrar, renombrar, carpeta nueva y deshacer). Los tipos
//! son del núcleo; la implementación (un hilo por operación) es de cada sistema y se pide con
//! `sys::file_ops()`.

use std::sync::mpsc::Sender;

use crate::platform::Handle;
pub use xdir_core::ops::{Op, OpResult, Undo, WM_APP_OP_DONE};

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::item;
#[cfg(windows)]
pub(crate) use self::win32::spawn as spawn_native;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use self::macos::spawn as spawn_native;

/// Corre `op` en su propio hilo con la implementación del sistema instalada.
pub fn spawn(owner: Handle, tx: Sender<OpResult>, op: Op) {
    xdir_core::sys::file_ops().spawn(owner, tx, op);
}

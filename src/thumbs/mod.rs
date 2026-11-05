//! Miniaturas e íconos grandes para las vistas de cuadrícula, en un hilo propio. La interfaz pide,
//! dibuja sin la imagen y el hilo avisa con `WM_APP_ICONS` cuando está lista.

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

/// De qué elemento: una ruta de disco o un PIDL absoluto (elementos virtuales del Shell).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Path(PathBuf),
    Pidl(Arc<[u8]>),
}

/// (elemento, lado en píxeles).
pub type Key = (Source, u32);

//! Miniaturas e íconos grandes para las vistas de cuadrícula, en un hilo propio. La interfaz pide,
//! dibuja sin la imagen y el hilo avisa con `WM_APP_ICONS` cuando está lista.

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

pub use xdir_core::thumbs::{Key, Source};

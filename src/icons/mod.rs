//! Íconos de archivos del sistema. La interfaz nunca espera por un ícono: lo pide, dibuja sin
//! él, y cuando está listo llega `WM_APP_ICONS` para repintar.

use crate::platform::WM_APP;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Hay íconos nuevos listos para dibujar.
pub const WM_APP_ICONS: u32 = WM_APP + 3;

/// Píxeles BGRA premultiplicados.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

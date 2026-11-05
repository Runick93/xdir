//! Dibujo de la ventana. Todas las coordenadas están en DIPs (1 DIP = 1 px a 96 DPI en Windows,
//! 1 punto en macOS). Cada sistema implementa `Renderer` y `Painter` con la misma interfaz.

pub use crate::platform::{Color, Rect, contains, rect};

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

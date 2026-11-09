//! Dibujo de la ventana. Todas las coordenadas están en DIPs (1 DIP = 1 px a 96 DPI en Windows,
//! 1 punto en macOS). Cada sistema implementa `Renderer` con la misma interfaz y su pintor
//! implementa el trait `Painter` del núcleo (`xdir_core::paint`), que es lo que recibe la interfaz.

pub use crate::platform::{Color, Rect, contains, rect};
pub use xdir_core::paint::{FontId, Painter};

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

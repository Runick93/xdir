//! Dibujo de la ventana. Todas las coordenadas están en DIPs (1 DIP = 1 px a 96 DPI). El
//! `Renderer` de Windows y su pintor implementan el trait `Painter` del núcleo (`xdir_core::paint`), que es lo que recibe la interfaz.

pub use crate::platform::{Color, Rect, rect};

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

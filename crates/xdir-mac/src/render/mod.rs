//! Dibujo de la ventana. Todas las coordenadas están en DIPs (1 DIP = 1 punto en macOS). El
//! `Renderer` de macOS y su pintor implementan el trait `Painter` del núcleo
//! (`xdir_core::paint`), que es lo que recibe la interfaz.

pub use crate::platform::{Color, Rect, rect};

mod macos;
pub use self::macos::*;

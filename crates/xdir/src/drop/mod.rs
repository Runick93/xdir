//! Arrastrar y soltar archivos sobre la ventana. Los tipos son del núcleo (`xdir_core::drop`);
//! acá queda el destino de arrastre de Windows.

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg_attr(not(windows), allow(unused_imports))]
pub use xdir_core::drop::{DragEvent, DragKind, Effect, WM_APP_DRAG};

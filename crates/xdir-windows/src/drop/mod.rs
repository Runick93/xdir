//! Arrastrar y soltar archivos sobre la ventana. Los tipos son del núcleo (`xdir_core::drop`);
//! acá queda el destino de arrastre de Windows.

mod win32;
pub use self::win32::*;

pub use xdir_core::drop::{DragEvent, DragKind, Effect, WM_APP_DRAG};

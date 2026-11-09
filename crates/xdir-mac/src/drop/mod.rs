//! Arrastrar y soltar archivos sobre la ventana. Los tipos son del núcleo (`xdir_core::drop`); el
//! destino de arrastre de macOS está en la vista (`app/mac.rs`).

pub use xdir_core::drop::{DragEvent, DragKind, Effect};

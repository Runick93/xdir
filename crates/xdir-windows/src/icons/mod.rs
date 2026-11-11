//! Íconos de archivos del sistema. La interfaz nunca espera por un ícono: lo pide, dibuja sin
//! él, y cuando está listo llega `WM_APP_ICONS` para repintar.

mod win32;
pub use self::win32::*;

/// Hay íconos nuevos listos para dibujar (vive en el núcleo, junto al `Painter`).
pub use xdir_core::paint::WM_APP_ICONS;

/// Píxeles BGRA premultiplicados (viven en el núcleo, junto al `Painter`).
pub use xdir_core::paint::Pixels;

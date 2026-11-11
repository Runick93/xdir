//! Tipos y servicios que no dependen del sistema operativo (color, rectángulo, teclas y el
//! "buzón" de la ventana) y lo propio de macOS (`macos`), que se reexporta acá; el resto del
//! programa solo usa lo de este módulo.

pub mod macos;
pub use self::macos::*;

pub use xdir_core::WM_APP;
pub use xdir_core::geom::{Color, Rect, rect};
pub use xdir_core::keys::{Key, keys};

/// La ventana principal; vive en el núcleo (`xdir_core::sys`) y `post` usa el `Waker` instalado.
pub use xdir_core::sys::Handle;

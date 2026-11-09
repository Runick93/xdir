//! Tipos y servicios que no dependen del sistema operativo: color, rectángulo, teclas y el
//! "buzón" de la ventana (mensajes que los hilos de trabajo le mandan al hilo de interfaz).
//!
//! Cada sistema implementa lo suyo en un submódulo (`win32`, `macos`) con la misma interfaz;
//! el resto del programa solo usa lo que se reexporta acá.

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

pub use xdir_core::WM_APP;
pub use xdir_core::geom::{Color, Rect, contains, rect};
pub use xdir_core::keys::{Key, keys};

/// La ventana principal; vive en el núcleo (`xdir_core::sys`) y `post` usa el `Waker` instalado.
pub use xdir_core::sys::Handle;

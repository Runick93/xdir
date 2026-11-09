//! Tipos y servicios que no dependen del sistema operativo: color, rectángulo, teclas y el
//! "buzón" de la ventana (mensajes que los hilos de trabajo le mandan al hilo de interfaz).
//!
//! Lo propio de Windows está en `win32` y se reexporta acá; el resto del programa solo usa lo de
//! este módulo.

mod win32;
pub use self::win32::*;

pub use xdir_core::WM_APP;
pub use xdir_core::geom::{Color, Rect, rect};
pub use xdir_core::keys::{Key, keys};

/// La ventana principal; vive en el núcleo (`xdir_core::sys`) y `post` usa el `Waker` instalado.
pub use xdir_core::sys::Handle;

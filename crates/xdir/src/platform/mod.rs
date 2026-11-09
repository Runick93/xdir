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

/// La ventana principal, vista desde cualquier hilo: sirve para mandarle mensajes de la
/// aplicación (`WM_APP + n`) que se procesan en el hilo de interfaz.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Handle(pub isize);

impl Handle {
    /// Encola un mensaje para el hilo de interfaz y vuelve enseguida.
    pub fn post(self, msg: u32, wparam: usize, lparam: isize) {
        post_message(self, msg, wparam, lparam);
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

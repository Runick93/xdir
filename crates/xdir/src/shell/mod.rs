//! Integración con el sistema: portapapeles de archivos, abrir con el programa asociado,
//! propiedades, menú contextual y objetos de datos para arrastrar.
//!
//! El servicio es `xdir_core::sys::shell()`: Windows lo implementa en `crate::sys::win32` con las
//! funciones de `win32.rs`. Lo que solo existe acá (objetos de datos para arrastrar, mensajes del
//! menú abierto) se usa directo desde `shell::win32`.

pub use xdir_core::shell::{Entry, Item};
#[cfg(windows)]
pub use xdir_core::shell::UNPIN_QUICK;

#[cfg(windows)]
pub mod win32;

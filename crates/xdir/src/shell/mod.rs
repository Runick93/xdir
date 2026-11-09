//! Integración con el sistema: portapapeles de archivos, abrir con el programa asociado,
//! propiedades, menú contextual y objetos de datos para arrastrar.
//!
//! El servicio es `xdir_core::sys::shell()`: cada sistema lo implementa en
//! `crate::sys::{win32,macos}` con las funciones de `win32.rs` / `macos.rs`. Lo que solo existe en
//! un sistema (objetos de datos para arrastrar, mensajes del menú abierto, `reveal`) se usa
//! directo desde su módulo de plataforma.

pub use xdir_core::shell::{Entry, Item};
#[cfg(windows)]
pub use xdir_core::shell::UNPIN_QUICK;

#[cfg(windows)]
pub mod win32;

#[cfg(target_os = "macos")]
pub mod macos;

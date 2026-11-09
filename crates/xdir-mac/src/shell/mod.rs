//! Integración con el sistema: portapapeles de archivos, abrir con el programa asociado,
//! propiedades, menú contextual y objetos de datos para arrastrar.
//!
//! El servicio es `xdir_core::sys::shell()`: macOS lo implementa en `crate::sys::macos` con las
//! funciones de `macos.rs`. Lo que solo existe acá (objetos para arrastrar, `reveal`) se usa
//! directo desde `shell::macos`.

pub use xdir_core::shell::{Entry, Item};

pub mod macos;

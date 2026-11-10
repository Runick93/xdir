//! Lectura de carpetas en segundo plano y vigilancia de cambios.
//!
//! Cada lectura corre en su propio hilo y manda los resultados por un canal, avisando a la
//! ventana con `WM_APP_LOAD`. La interfaz nunca espera al disco.
//!
//! Los tipos y la búsqueda están en el núcleo (`xdir_core::fs`) y lo propio de macOS detrás de
//! `sys::file_system()`; la implementación está en `macos/` (la instala `crate::sys`).

pub use xdir_core::fs::{Entry, LoadEvent, LoadMsg, Location, QuickItem, WM_APP_LOAD};

pub(crate) mod macos;
pub(crate) use macos::is_alias;

//! Lectura de carpetas en segundo plano y vigilancia de cambios.
//!
//! Cada lectura corre en su propio hilo y manda los resultados por un canal, avisando a la
//! ventana con `WM_APP_LOAD`. La interfaz nunca espera al disco.
//!
//! Los tipos y la búsqueda están en el núcleo (`xdir_core::fs`) y lo propio de Windows detrás
//! de `sys::file_system()`; la implementación de Windows está en `win32/` (la instala
//! `crate::sys`). Lo que solo usan otros módulos de la misma plataforma (`wide_path`, el elemento
//! del Acceso rápido, el ayudante elevado de xtree) se reexporta acá tal cual.

pub use xdir_core::fs::{Entry, LoadEvent, LoadMsg, Location, QuickItem, WM_APP_LOAD};

pub(crate) mod win32;
pub use self::win32::{XTREE_HELPER_ARG, quick_access_item, wide_path, xtree_helper_main};

// Nombre del temporal del ayudante elevado de xtree (fuera de Windows sus pruebas las incluye
// `main.rs`).
mod xtree_temp;

//! Lectura de carpetas en segundo plano y vigilancia de cambios.
//!
//! Cada lectura corre en su propio hilo y manda los resultados por un canal, avisando a la
//! ventana con `WM_APP_LOAD`. La interfaz nunca espera al disco.
//!
//! Los tipos y la búsqueda están en el núcleo (`xdir_core::fs`) y lo propio de cada sistema detrás
//! de `sys::file_system()`; las implementaciones están en `win32/` y `macos/` (las instala
//! `crate::sys`). Lo que solo usan otros módulos de la misma plataforma (`wide_path`, el elemento
//! del Acceso rápido, el ayudante elevado de xtree) se reexporta acá tal cual.

use std::path::Path;

use xdir_core::sys;

pub use xdir_core::fs::{Entry, LoadEvent, LoadMsg, Location, QuickItem, WM_APP_LOAD};

#[cfg(windows)]
pub(crate) mod win32;
#[cfg(windows)]
pub use self::win32::{XTREE_HELPER_ARG, quick_access_item, wide_path, xtree_helper_main};

// Nombre del temporal del ayudante elevado de xtree: lo usa Windows; las pruebas, en todos.
#[cfg(any(windows, test))]
mod xtree_temp;

#[cfg(target_os = "macos")]
pub(crate) mod macos;

/// ¿Es la misma ruta? (sin distinguir mayúsculas ni separador final). La usan las operaciones de
/// macOS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn same_path(a: &Path, b: &Path) -> bool {
    sys::file_system().same_path(a, b)
}

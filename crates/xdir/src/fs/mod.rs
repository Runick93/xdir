//! Lectura de carpetas en segundo plano y vigilancia de cambios.
//!
//! Cada lectura corre en su propio hilo y manda los resultados por un canal, avisando a la
//! ventana con `WM_APP_LOAD`. La interfaz nunca espera al disco.
//!
//! Fachada: los tipos y la búsqueda están en el núcleo (`xdir_core::fs`) y lo propio de cada
//! sistema detrás de `sys::file_system()`; las implementaciones están en `win32/` y `macos/` (las
//! instala `crate::sys`). Lo que solo usan otros módulos de la misma plataforma (`wide_path`, el
//! elemento del Acceso rápido, el ayudante elevado de xtree) se reexporta acá tal cual. Lo que
//! solo usa el árbol lateral queda sin usar sin la feature `arbol` (de ahí los `allow`).

use std::path::{Path, PathBuf};

use crate::platform::Handle;
use xdir_core::sys::{self, FileSystem};

pub use xdir_core::fs::{
    Entry, LoadEvent, LoadMsg, Location, QuickItem, WM_APP_CHANGED, WM_APP_LOAD, is_unc, is_zip, normalize_lexical,
};
pub use xdir_core::sys::Watch;

#[cfg(windows)]
pub(crate) mod win32;
#[cfg(windows)]
pub use self::win32::{XTREE_HELPER_ARG, quick_access_item, wide_path, xtree_helper_main};

// Nombre del temporal del ayudante elevado de xtree: lo usa Windows; las pruebas, en todos.
#[cfg(any(windows, test))]
mod xtree_temp;

#[cfg(target_os = "macos")]
pub(crate) mod macos;

fn fsys() -> &'static dyn FileSystem {
    sys::file_system()
}

/// Olvida los nombres de unidad guardados (cambió el conjunto de unidades).
pub fn forget_drive_labels() {
    fsys().forget_drive_labels()
}

/// ¿Es una ruta de una unidad que ya no está?
pub fn drive_missing(path: &Path) -> bool {
    fsys().drive_missing(path)
}

/// Subcarpetas de un lugar, para el menú de las flechas de la barra de dirección.
pub fn subfolders(loc: &Location) -> Vec<(String, Location)> {
    fsys().subfolders(loc)
}

/// Lo escrito en la barra de dirección como lugar especial del Shell.
pub fn shell_location(text: &str) -> Option<Location> {
    fsys().shell_location(text)
}

/// ¿Está anclada ahora mismo al Acceso rápido?
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn is_quick_pinned(path: &Path) -> bool {
    fsys().is_quick_pinned(path)
}

/// Carpeta cuyos cambios indican que el Acceso rápido se modificó.
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn quick_access_watch_dir() -> Option<PathBuf> {
    fsys().quick_access_watch_dir()
}

/// Ruta escrita a mano con las mayúsculas reales del disco.
pub fn true_case(path: &Path) -> PathBuf {
    fsys().true_case(path)
}

/// Conecta con un recurso de red pidiendo credenciales si hace falta.
pub fn connect_share(owner: Handle, path: &Path) -> bool {
    fsys().connect_share(owner, path)
}

/// ¿Es la misma ruta? (sin distinguir mayúsculas ni separador final).
// Fuera del núcleo solo la usan las operaciones de macOS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn same_path(a: &Path, b: &Path) -> bool {
    fsys().same_path(a, b)
}

/// Vigila la carpeta `path` y avisa a `hwnd` con `msg` (`wparam` = `panel`, `lparam`). Se
/// detiene al soltar el guardián. Sin la feature `arbol` no lo usa nadie (las pestañas van
/// directo a `sys::file_system()`).
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn watch(hwnd: Handle, msg: u32, panel: usize, lparam: isize, path: &Path) -> Option<Box<dyn Watch>> {
    fsys().watch(hwnd, msg, panel, lparam, path)
}

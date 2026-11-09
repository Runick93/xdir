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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;

use crate::platform::Handle;
use xdir_core::sys::{self, FileSystem};

pub use xdir_core::fs::{
    Entry, LoadEvent, LoadMsg, Location, QuickItem, WM_APP_CHANGED, WM_APP_LOAD, is_unc, is_zip, normalize_lexical, spawn_search,
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

/// Lanza la lectura de `location` en un hilo. Se abandona si `current` deja de valer `generation`.
pub fn spawn_load(hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
    fsys().spawn_load(hwnd, tx, panel, generation, current, location)
}

/// ¿Está oculto? (atributo de Windows; punto o marca del Finder en macOS).
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn is_hidden(meta: &std::fs::Metadata, name: &OsStr) -> bool {
    fsys().is_hidden(meta, name)
}

/// Ruta de la carpeta del usuario `i` (Escritorio, Descargas, Documentos, Imágenes, Música,
/// Vídeos), si existe.
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn user_folder(i: usize) -> Option<PathBuf> {
    fsys().user_folder(i)
}

/// Raíces de las unidades o discos montados.
pub fn drives() -> Vec<PathBuf> {
    fsys().drives()
}

/// Identificador de una unidad para `drive_label`.
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn drive_name(root: &Path) -> String {
    fsys().drive_name(root)
}

/// Nombre de una unidad ("Disco local (C:)", "Macintosh HD"), cacheado.
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn drive_label(drive: &str) -> String {
    fsys().drive_label(drive)
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

/// Ícono genérico de carpeta.
pub fn folder_icon() -> Option<i32> {
    fsys().folder_icon()
}

/// Lo escrito en la barra de dirección como lugar especial del Shell.
pub fn shell_location(text: &str) -> Option<Location> {
    fsys().shell_location(text)
}

/// Red del Explorador (vacío donde no existe).
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn network_place() -> &'static str {
    fsys().network_place()
}

/// Carpeta de Linux (WSL) del Explorador (vacío donde no existe).
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn linux_place() -> &'static str {
    fsys().linux_place()
}

/// ¿Existe esa carpeta virtual en este equipo?
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn shell_place_exists(parsing: &str) -> bool {
    fsys().shell_place_exists(parsing)
}

/// Subcarpetas con ruta de disco de una carpeta virtual, para el árbol lateral.
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn shell_subfolders(parsing: &str) -> Vec<(String, PathBuf)> {
    fsys().shell_subfolders(parsing)
}

/// Carpetas del Acceso rápido de Windows.
#[cfg_attr(not(feature = "arbol"), allow(dead_code))]
pub fn quick_access() -> Vec<QuickItem> {
    fsys().quick_access()
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

/// Forma larga de una ruta (sin nombres cortos 8.3).
pub fn long_path(path: &Path) -> PathBuf {
    fsys().long_path(path)
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
pub fn same_path(a: &Path, b: &Path) -> bool {
    fsys().same_path(a, b)
}

/// Vigila la carpeta `path` y avisa a `hwnd` con `msg` (`wparam` = `panel`, `lparam`). Se
/// detiene al soltar el guardián.
pub fn watch(hwnd: Handle, msg: u32, panel: usize, lparam: isize, path: &Path) -> Option<Box<dyn Watch>> {
    fsys().watch(hwnd, msg, panel, lparam, path)
}

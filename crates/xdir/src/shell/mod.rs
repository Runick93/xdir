//! Integración con el sistema: portapapeles de archivos, abrir con el programa asociado,
//! propiedades, menú contextual y objetos de datos para arrastrar.
//!
//! Fachada de `xdir_core::sys::shell()`: cada sistema implementa el servicio en
//! `crate::sys::{win32,macos}` con las funciones de `win32.rs` / `macos.rs`. Lo que solo existe en
//! un sistema (objetos de datos para arrastrar, mensajes del menú abierto, `reveal`) se usa
//! directo desde su módulo de plataforma.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub use xdir_core::shell::{Entry, Item, UNPIN_QUICK};
use xdir_core::sys;

use crate::fs::Location;
use crate::platform::Handle;

#[cfg(windows)]
pub mod win32;

#[cfg(target_os = "macos")]
pub mod macos;

/// Copia (o corta) los elementos al portapapeles de archivos.
pub fn copy_files(items: &[Item], cut: bool) -> Result<(), String> {
    sys::shell().copy_files(items, cut)
}

/// Archivos del portapapeles y si se cortaron.
pub fn clipboard_files() -> Option<(Vec<PathBuf>, bool)> {
    sys::shell().clipboard_files()
}

/// Pega lo que no tiene rutas de disco (elementos virtuales). `Ok(true)` si pegó algo.
pub fn paste_virtual(owner: Handle, dest: &Location) -> Result<bool, String> {
    sys::shell().paste_virtual(owner, dest)
}

pub fn clear_clipboard() {
    sys::shell().clear_clipboard()
}

pub fn copy_text(owner: Handle, text: &str) -> Result<(), String> {
    sys::shell().copy_text(owner, text)
}

pub fn open_file(owner: Handle, file: &Path, dir: Option<&Path>) {
    sys::shell().open_file(owner, file, dir)
}

pub fn properties(owner: Handle, items: &[Item]) {
    sys::shell().properties(owner, items)
}

/// Menú contextual del sistema (ver `xdir_core::sys::Shell::context_menu`).
#[allow(clippy::too_many_arguments)]
pub fn context_menu(
    owner: Handle,
    folder: &Location,
    items: &[Item],
    at: (f32, f32),
    extended: bool,
    intercept: bool,
    own: &[Entry],
    pinned: bool,
) -> Option<String> {
    sys::shell().context_menu(owner, folder, items, at, extended, intercept, own, pinned)
}

pub fn invoke_verb(owner: Handle, folder: &Location, items: &[Item], verb: &str) -> Result<(), String> {
    sys::shell().invoke_verb(owner, folder, items, verb)
}

pub fn pin_quick_access(owner: Handle, path: &Path) -> Result<(), String> {
    sys::shell().pin_quick_access(owner, path)
}

pub fn unpin_quick_access(owner: Handle, path: &Path) -> Result<(), String> {
    sys::shell().unpin_quick_access(owner, path)
}

pub fn set_menu_mode(dark: Option<bool>) {
    sys::shell().set_menu_mode(dark)
}

pub fn volume_of(p: &Path) -> Option<OsString> {
    sys::shell().volume_of(p)
}

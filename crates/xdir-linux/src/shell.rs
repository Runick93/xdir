//! Integración con el escritorio de Linux (`xdir_core::sys::Shell`): abrir con `xdg-open` y el
//! volumen de una ruta. Lo que necesita una interfaz (portapapeles, menú contextual, diálogos,
//! propiedades, Acceso rápido) todavía no está: devuelve `None` / `false` / "No disponible en
//! Linux", sin entrar en pánico.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use xdir_core::fs::Location;
use xdir_core::shell::{Entry, Item};
use xdir_core::sys::{Handle, Shell};

use crate::fs::places;

/// Lo que Linux todavía no puede hacer.
pub const UNAVAILABLE: &str = "No disponible en Linux";

/// Abre `path` con el programa asociado (`xdg-open`), en un proceso aparte. Un hilo lo espera para
/// que no quede zombi al terminar. Si no se puede lanzar no pasa nada (como `open` en macOS).
fn xdg_open(path: &Path, dir: Option<&Path>) {
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(path);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    if let Ok(mut c) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

/// Punto de montaje de `drives` que contiene a `path` (el más largo).
pub fn volume_in(path: &Path, drives: &[PathBuf]) -> Option<OsString> {
    drives
        .iter()
        .filter(|d| path.starts_with(d))
        .max_by_key(|d| d.components().count())
        .map(|d| d.as_os_str().to_os_string())
}

/// Integración con el escritorio de Linux (lo que instala `sys::install`).
pub struct LinuxShell;

/// A dónde apunta un enlace simbólico (relativo a su carpeta si no es absoluto).
pub fn link_target(path: &Path) -> Option<PathBuf> {
    let to = std::fs::read_link(path).ok()?;
    Some(if to.is_absolute() { to } else { path.parent()?.join(to) })
}

impl Shell for LinuxShell {
    fn link_style(&self) -> xdir_core::names::LinkStyle {
        xdir_core::names::LinkStyle::Symlink
    }

    fn link_target(&self, path: &Path) -> Option<PathBuf> {
        link_target(path)
    }

    fn copy_files(&self, _items: &[Item], _cut: bool) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn clipboard_files(&self) -> Option<(Vec<PathBuf>, bool)> {
        None
    }

    fn paste_virtual(&self, _owner: Handle, _dest: &Location) -> Result<bool, String> {
        Err(UNAVAILABLE.into())
    }

    fn clear_clipboard(&self) {}

    fn copy_text(&self, _owner: Handle, _text: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn pick_folder(&self, _owner: Handle, _title: &str) -> Option<PathBuf> {
        None
    }

    fn open_item(&self, _owner: Handle, item: &Item) {
        xdg_open(&item.path, None);
    }

    fn open_file(&self, _owner: Handle, file: &Path, dir: Option<&Path>) {
        xdg_open(file, dir);
    }

    fn properties(&self, _owner: Handle, _items: &[Item]) {}

    fn context_menu(
        &self,
        _owner: Handle,
        _folder: &Location,
        _items: &[Item],
        _at: (f32, f32),
        _extended: bool,
        _intercept: bool,
        _own: &[Entry],
        _pinned: bool,
    ) -> Option<String> {
        None
    }

    fn invoke_verb(&self, _owner: Handle, _folder: &Location, _items: &[Item], _verb: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn pin_quick_access(&self, _owner: Handle, _path: &Path) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn unpin_quick_access(&self, _owner: Handle, _path: &Path) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    /// Sin menús nativos no hay modo que fijar.
    fn set_menu_mode(&self, _dark: Option<bool>) {}

    fn volume_of(&self, path: &Path) -> Option<OsString> {
        volume_in(path, &places::drives())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_unsupported_returns_error() {
        let sh = LinuxShell;
        let items = [Item::from(PathBuf::from("/tmp/a"))];
        assert_eq!(sh.copy_files(&items, false), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.copy_files(&items, true), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.clipboard_files(), None);
        assert_eq!(sh.pick_folder(Handle(0), "x"), None);
        assert_eq!(sh.paste_virtual(Handle(0), &Location::Computer), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.invoke_verb(Handle(0), &Location::Computer, &items, "delete"), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.pin_quick_access(Handle(0), Path::new("/")), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.context_menu(Handle(0), &Location::Computer, &items, (0.0, 0.0), false, true, &[], false), None);
    }

    #[test]
    fn volume_is_longest_mount_point() {
        let drives = [PathBuf::from("/"), PathBuf::from("/media/usb"), PathBuf::from("/media/usb2")];
        assert_eq!(volume_in(Path::new("/media/usb/fotos/a.jpg"), &drives), Some("/media/usb".into()));
        assert_eq!(volume_in(Path::new("/media/usb2"), &drives), Some("/media/usb2".into()));
        assert_eq!(volume_in(Path::new("/home/a"), &drives), Some("/".into()));
        assert_eq!(volume_in(Path::new("relativa"), &drives), None);
    }
}

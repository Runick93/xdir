//! Sesión: carpeta de configuración, estado de un panel y texto de las ubicaciones. El resto de
//! la sesión (columnas, vistas, acento) vive todavía en `xdir`.

use std::path::{Path, PathBuf};

use crate::fs::Location;

const COMPUTER: &str = "Este equipo";
/// Carpetas virtuales: "shell:<nombre de análisis>|<nombre visible>".
const SHELL_PREFIX: &str = "shell:";

/// Pestañas de un panel y cuál está activa.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneState {
    pub tabs: Vec<Location>,
    pub active: usize,
}


/// Carpeta de configuración: junto al ejecutable en modo portable (si ahí existe
/// `xdir.portable`), si no la del usuario que da la plataforma.
pub fn config_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    resolve_config_dir(exe_dir.as_deref(), || crate::sys::env().config_dir())
}

/// La regla de `config_dir`, sin efectos: `exe_dir` es la carpeta del ejecutable y `platform`
/// la carpeta del usuario (solo se pide si no hay modo portable).
pub fn resolve_config_dir(exe_dir: Option<&Path>, platform: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    match exe_dir {
        Some(dir) if dir.join("xdir.portable").exists() => Some(dir.to_path_buf()),
        _ => platform(),
    }
}

pub fn location_str(l: &Location) -> String {
    match l {
        Location::Computer => COMPUTER.to_string(),
        Location::Path(p) => p.to_string_lossy().into_owned(),
        Location::Shell { parsing, name, .. } => format!("{SHELL_PREFIX}{parsing}|{name}"),
    }
}

pub fn location_from(s: &str) -> Location {
    if s == COMPUTER {
        return Location::Computer;
    }
    if let Some((parsing, name)) = s.strip_prefix(SHELL_PREFIX).and_then(|r| r.rsplit_once('|')) {
        return Location::Shell { parsing: parsing.to_string(), name: name.to_string(), parent: None };
    }
    Location::Path(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_mode_wins() {
        let exe_dir = std::env::temp_dir().join(format!("xdir-portable-{}", std::process::id()));
        std::fs::create_dir_all(&exe_dir).unwrap();
        // Sin la marca, manda la plataforma.
        let platform = PathBuf::from("/plataforma/xdir");
        assert_eq!(resolve_config_dir(Some(&exe_dir), || Some(platform.clone())), Some(platform.clone()));
        // Con `xdir.portable` junto al ejecutable, esa carpeta, sin preguntarle a la plataforma.
        std::fs::write(exe_dir.join("xdir.portable"), "").unwrap();
        let got = resolve_config_dir(Some(&exe_dir), || panic!("no debía llamar a la plataforma"));
        assert_eq!(got.as_deref(), Some(exe_dir.as_path()));
        // Sin ejecutable conocido, la plataforma.
        assert_eq!(resolve_config_dir(None::<&Path>, || Some(platform.clone())), Some(platform));
        let _ = std::fs::remove_dir_all(&exe_dir);
    }
}

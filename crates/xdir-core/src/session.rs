//! Sesión (por ahora solo la carpeta de configuración; el resto vive todavía en `xdir`).

use std::path::{Path, PathBuf};

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

//! Linux: la carpeta temporal privada para lo que se extrae de un archivo comprimido. Nunca en
//! `/tmp` (compartida entre usuarios): en `$XDG_RUNTIME_DIR` (del usuario, 0700) o, sin ella, en
//! su caché.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// La carpeta de las temporales, verificada (`private_dir`).
pub fn archive_temp_dir() -> Option<PathBuf> {
    private_dir(&archive_temp_path(std::env::var_os("XDG_RUNTIME_DIR"), std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))?)
}

/// `$XDG_RUNTIME_DIR/xdir-comprimidos`; si no hay (o es relativa), `$XDG_CACHE_HOME/xdir/comprimidos`
/// o `~/.cache/xdir/comprimidos`. Recibe los valores en vez de leerlos.
pub fn archive_temp_path(runtime: Option<OsString>, cache: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    if let Some(r) = runtime.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(r.join("xdir-comprimidos"));
    }
    let cache = cache.map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home.filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".cache")))?;
    Some(cache.join("xdir").join("comprimidos"))
}

/// Crea `path` con modo 0700 si no está y comprueba que sea una carpeta de verdad (no un enlace)
/// del usuario actual; si es suya pero la pueden ver otros, la deja 0700. `None` si no se puede
/// confiar en ella.
pub fn private_dir(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let m = std::fs::symlink_metadata(path).ok()?;
    // SAFETY: `getuid` no tiene precondiciones.
    if !m.file_type().is_dir() || m.uid() != unsafe { libc::getuid() } {
        return None;
    }
    if m.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn temp_path_prefers_runtime_dir_then_cache() {
        assert_eq!(archive_temp_path(os("/run/user/1000"), None, os("/home/a")), Some(PathBuf::from("/run/user/1000/xdir-comprimidos")));
        assert_eq!(archive_temp_path(os("rel"), os("/c"), os("/home/a")), Some(PathBuf::from("/c/xdir/comprimidos")));
        assert_eq!(archive_temp_path(None, None, os("/home/a")), Some(PathBuf::from("/home/a/.cache/xdir/comprimidos")));
        assert_eq!(archive_temp_path(None, None, None), None);
    }

    #[test]
    fn private_dir_checks_owner_mode_and_symlink() {
        let d = std::env::temp_dir().join(format!("xdir-origen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let fresh = d.join("nueva");
        assert_eq!(private_dir(&fresh), Some(fresh.clone()));
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o700);
        let open = d.join("abierta");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(private_dir(&open), Some(open.clone()));
        assert_eq!(std::fs::metadata(&open).unwrap().permissions().mode() & 0o777, 0o700);
        let link = d.join("enlace");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert_eq!(private_dir(&link), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}

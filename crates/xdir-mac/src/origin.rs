//! macOS: la carpeta temporal privada para lo que se extrae de un archivo comprimido.

use std::path::{Path, PathBuf};

/// `<TMPDIR del usuario>/xdir-comprimidos` (en macOS `TMPDIR` ya es del usuario, bajo
/// `/var/folders`), creada 0700 y verificada (`private_dir`).
pub fn archive_temp_dir() -> Option<PathBuf> {
    private_dir(&std::env::temp_dir().join("xdir-comprimidos"))
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

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xdir-origen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Nueva: 0700. Una que ya estaba abierta a otros: pasa a 0700. Un enlace (aunque lleve a una
    /// carpeta propia) o un archivo: no.
    #[test]
    fn private_dir_checks_owner_mode_and_symlink() {
        let d = tmp("privada");
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
        let file = d.join("archivo");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(private_dir(&file), None);
        // Una que no es del usuario (la raíz de todo, de root): no.
        assert_eq!(private_dir(Path::new("/private/var")), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}

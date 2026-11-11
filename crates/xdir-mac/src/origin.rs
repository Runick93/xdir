//! macOS: la carpeta temporal privada para lo que se extrae de un archivo comprimido, y la
//! cuarentena (`com.apple.quarantine`) que lo extraído hereda del archivo, como hacen la Utilidad
//! de Archivo y el Finder: lo bajado de internet sigue pidiendo permiso a Gatekeeper.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const QUARANTINE: &std::ffi::CStr = c"com.apple.quarantine";

fn c_path(p: &Path) -> Option<CString> {
    CString::new(p.as_os_str().as_bytes()).ok()
}

/// La cuarentena de `path` (sin seguir enlaces), si tiene.
pub fn quarantine_of(path: &Path) -> Option<Vec<u8>> {
    let c = c_path(path)?;
    let mut buf = vec![0u8; 1024];
    // SAFETY: `c` es una cadena C válida y `buf` tiene el largo que se pasa.
    let n = unsafe { libc::getxattr(c.as_ptr(), QUARANTINE.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, libc::XATTR_NOFOLLOW) };
    (n > 0).then(|| {
        buf.truncate(n as usize);
        buf
    })
}

/// Si `archive` está en cuarentena, `dst` (archivo o carpeta extraídos de él) también. Un enlace
/// no se toca.
pub fn copy_origin(archive: &Path, dst: &Path) {
    let Some(value) = quarantine_of(archive) else { return };
    if std::fs::symlink_metadata(dst).is_ok_and(|m| m.file_type().is_symlink()) {
        return;
    }
    let Some(c) = c_path(dst) else { return };
    // SAFETY: `c` es una cadena C válida y `value` tiene el largo que se pasa.
    unsafe { libc::setxattr(c.as_ptr(), QUARANTINE.as_ptr(), value.as_ptr().cast(), value.len(), 0, libc::XATTR_NOFOLLOW) };
}

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

    /// Lo extraído (archivo y carpeta) de un archivo en cuarentena queda en cuarentena; de uno sin
    /// ella, no; un enlace no se toca.
    #[test]
    fn copy_origin_copies_quarantine() {
        let d = tmp("cuarentena");
        let zip = d.join("bajado.zip");
        std::fs::write(&zip, "z").unwrap();
        let mark = b"0081;00000000;xdir;";
        let c = c_path(&zip).unwrap();
        assert_eq!(unsafe { libc::setxattr(c.as_ptr(), QUARANTINE.as_ptr(), mark.as_ptr().cast(), mark.len(), 0, 0) }, 0);
        let (file, dir) = (d.join("malo.command"), d.join("carpeta"));
        std::fs::write(&file, "x").unwrap();
        std::fs::create_dir(&dir).unwrap();
        copy_origin(&zip, &file);
        copy_origin(&zip, &dir);
        assert_eq!(quarantine_of(&file).as_deref(), Some(&mark[..]));
        assert_eq!(quarantine_of(&dir).as_deref(), Some(&mark[..]));
        let link = d.join("enlace");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        copy_origin(&zip, &link);
        let clean = d.join("limpio.zip");
        std::fs::write(&clean, "z").unwrap();
        let other = d.join("otro.txt");
        std::fs::write(&other, "x").unwrap();
        copy_origin(&clean, &other);
        assert_eq!(quarantine_of(&other), None);
        let _ = std::fs::remove_dir_all(&d);
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

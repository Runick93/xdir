//! Windows: la carpeta temporal privada para lo que se extrae de un archivo comprimido, y la
//! marca de la web (`Zone.Identifier`) que lo extraído hereda del archivo, como hace el Explorador:
//! lo bajado de internet sigue pasando por SmartScreen y la Vista protegida.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows::Win32::Security::{EqualSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_USER, WinBuiltinAdministratorsSid};
use windows::core::HSTRING;

/// `FILE_ATTRIBUTE_REPARSE_POINT`: enlace simbólico o unión.
const REPARSE: u32 = 0x400;

/// `%TEMP%\xdir-comprimidos` (la temporal del usuario, en su perfil), verificada (`private_dir`).
pub fn archive_temp_dir() -> Option<PathBuf> {
    private_dir(&std::env::temp_dir().join("xdir-comprimidos"))
}

/// Crea `path` si no está y comprueba que sea una carpeta de verdad (no un enlace ni una unión)
/// cuyo dueño sea el usuario actual (o el grupo Administradores, el dueño de lo que crea un
/// proceso elevado). `None` si no se puede confiar en ella.
pub fn private_dir(path: &Path) -> Option<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    std::fs::create_dir_all(path).ok()?;
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.is_dir() || m.file_attributes() & REPARSE != 0 || !owned_by_user(path) {
        return None;
    }
    Some(path.to_path_buf())
}

/// El flujo alternativo `Zone.Identifier` de `path`.
fn zone_stream(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(":Zone.Identifier");
    PathBuf::from(s)
}

/// Si `archive` tiene marca de la web, el archivo `dst` (extraído de él) también (las carpetas
/// no la llevan). Nunca pisa una marca que ya tenga.
pub fn copy_origin(archive: &Path, dst: &Path) {
    let Ok(mark) = std::fs::read(zone_stream(archive)) else { return };
    if !std::fs::symlink_metadata(dst).is_ok_and(|m| m.is_file()) {
        return;
    }
    let _ = std::fs::OpenOptions::new().write(true).create_new(true).open(zone_stream(dst)).and_then(|mut f| std::io::Write::write_all(&mut f, &mark));
}

/// ¿El dueño de `path` es el usuario del proceso (o Administradores)?
fn owned_by_user(path: &Path) -> bool {
    let Some(buf) = crate::info::token_user() else { return false };
    unsafe {
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut owner = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let wide = HSTRING::from(path.as_os_str());
        if GetNamedSecurityInfoW(&wide, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, Some(&mut owner), None, None, None, &mut sd) != ERROR_SUCCESS {
            return false;
        }
        let ok = EqualSid(owner, user.User.Sid).is_ok() || IsWellKnownSid(owner, WinBuiltinAdministratorsSid).as_bool();
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo extraído de un archivo con marca de la web la hereda; de uno sin marca, no.
    #[test]
    fn copy_origin_copies_zone_identifier() {
        let d = std::env::temp_dir().join(format!("xdir-zona-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let zip = d.join("bajado.zip");
        std::fs::write(&zip, "z").unwrap();
        let mark = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        std::fs::write(zone_stream(&zip), mark).unwrap();
        let file = d.join("malo.js");
        std::fs::write(&file, "x").unwrap();
        copy_origin(&zip, &file);
        assert_eq!(std::fs::read(zone_stream(&file)).unwrap(), mark);
        let other = d.join("otro.js");
        std::fs::write(&other, "x").unwrap();
        copy_origin(&d.join("sin-marca.zip"), &other);
        assert!(std::fs::read(zone_stream(&other)).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Nueva: sí. Un archivo: no. Una unión (enlace de carpeta): no.
    #[test]
    fn private_dir_checks_owner_and_reparse() {
        let d = std::env::temp_dir().join(format!("xdir-origen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let fresh = d.join("nueva");
        assert_eq!(private_dir(&fresh), Some(fresh.clone()));
        let file = d.join("archivo");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(private_dir(&file), None);
        let link = d.join("enlace");
        if std::os::windows::fs::symlink_dir(&fresh, &link).is_ok() {
            assert_eq!(private_dir(&link), None);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

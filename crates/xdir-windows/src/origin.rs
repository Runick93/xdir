//! Windows: la carpeta temporal privada para lo que se extrae de un archivo comprimido.

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

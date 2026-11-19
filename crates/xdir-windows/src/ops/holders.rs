//! Quién tiene abierto un archivo, con el Restart Manager de Windows. Se pregunta solo después de
//! un fallo definitivo y nunca desde el hilo de la ventana (lo garantiza quien llama).

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::RestartManager::{CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources, RmStartSession};
use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::core::{Owned, PCWSTR, PWSTR};
use xdir_core::sys::Holder;

/// Sesión del Restart Manager: se cierra siempre al soltarla (hay un máximo de 64 abiertas a la
/// vez en todo el sistema, así que una que se escapa rompe esto al rato).
struct Session(u32);

impl Drop for Session {
    fn drop(&mut self) {
        let _ = unsafe { RmEndSession(self.0) };
    }
}

/// Los procesos que tienen abierto `path`. Si no se puede saber (no existe, falla el Restart
/// Manager), la lista vacía.
pub(crate) fn holders_of(path: &Path) -> Vec<Holder> {
    let wide = plain_path(path);
    let mut handle = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    if unsafe { RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr())) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let session = Session(handle);
    if unsafe { RmRegisterResources(session.0, Some(&[PCWSTR(wide.as_ptr())]), None, None) } != ERROR_SUCCESS {
        return Vec::new();
    }
    processes(&session).iter().map(holder).collect()
}

/// La lista de `RmGetList`: primero se pregunta el tamaño y después se pide; si entre una pasada
/// y otra apareció otro proceso se vuelve a probar (pocas veces). Con error, nada.
fn processes(session: &Session) -> Vec<RM_PROCESS_INFO> {
    let mut list: Vec<RM_PROCESS_INFO> = Vec::new();
    for _ in 0..3 {
        let (mut needed, mut count, mut reasons) = (0u32, list.len() as u32, 0u32);
        let buf = if list.is_empty() { None } else { Some(list.as_mut_ptr()) };
        match unsafe { RmGetList(session.0, &mut needed, &mut count, buf, &mut reasons) } {
            ERROR_SUCCESS => {
                list.truncate(count as usize);
                return list;
            }
            ERROR_MORE_DATA => list = vec![RM_PROCESS_INFO::default(); needed as usize],
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

fn holder(info: &RM_PROCESS_INFO) -> Holder {
    let pid = info.Process.dwProcessId;
    let exe = exe_of(pid);
    let app = until_nul(&info.strAppName);
    let name = if app.is_empty() { exe.clone().unwrap_or_default() } else { app };
    Holder { name, exe, pid, user: user_of(pid) }
}

/// La ruta en UTF-16 terminada en 0, en su forma normal: el Restart Manager no entiende `\\?\`.
fn plain_path(p: &Path) -> Vec<u16> {
    let wide: Vec<u16> = p.as_os_str().encode_wide().collect();
    let unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let mut plain = if wide.starts_with(&unc) {
        // \\?\UNC\server\share\x → \\server\share\x
        [&[u16::from(b'\\'); 2][..], &wide[unc.len()..]].concat()
    } else if wide.starts_with(&verbatim) {
        wide[verbatim.len()..].to_vec()
    } else {
        wide
    };
    plain.push(0);
    plain
}

fn until_nul(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}

fn open_process(pid: u32) -> Option<Owned<HANDLE>> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    // SAFETY: `OpenProcess` acaba de abrirlo y es nuestro; `Owned` lo cierra al soltarlo.
    Some(unsafe { Owned::new(h) })
}

/// El nombre del ejecutable del proceso (solo el archivo, sin la carpeta).
fn exe_of(pid: u32) -> Option<String> {
    let process = open_process(pid)?;
    let mut buf = vec![0u16; 32 * 1024];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(*process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    let full = OsString::from_wide(&buf[..len as usize]);
    Path::new(&full).file_name().map(|n| n.to_string_lossy().into_owned())
}

/// El usuario dueño del proceso, «DOMINIO\usuario». Si no se puede mirar (el proceso inactivo,
/// los de sistema o de otros usuarios sin ser administrador), `None`.
fn user_of(pid: u32) -> Option<String> {
    let process = open_process(pid)?;
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(*process, TOKEN_QUERY, &mut token) }.ok()?;
    // SAFETY: `OpenProcessToken` acaba de abrirlo y es nuestro; `Owned` lo cierra al soltarlo.
    let token = unsafe { Owned::new(token) };
    let mut len = 0u32;
    let _ = unsafe { GetTokenInformation(*token, TokenUser, None, 0, &mut len) };
    if len == 0 {
        return None;
    }
    // De `u64`: `TOKEN_USER` lleva un puntero y tiene que quedar alineado.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    unsafe { GetTokenInformation(*token, TokenUser, Some(buf.as_mut_ptr().cast()), (buf.len() * 8) as u32, &mut len) }.ok()?;
    // SAFETY: `GetTokenInformation` llenó `buf` con un `TOKEN_USER` cuyo SID vive dentro de `buf`.
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let name = crate::info::account_name(sid);
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::PathBuf;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-holders-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("archivo.txt");
        File::create(&file).unwrap();
        file
    }

    #[test]
    fn file_open_without_sharing_names_this_process() {
        let path = temp_file("abierto");
        let _open = OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let me = std::env::var("USERNAME").unwrap().to_lowercase();
        let holders = holders_of(&path);
        let mine = holders.iter().find(|h| h.pid == std::process::id()).expect("este proceso tiene el archivo");
        assert!(!mine.name.is_empty());
        assert!(mine.exe.as_deref().is_some_and(|e| e.to_lowercase().ends_with(".exe")), "{mine:?}");
        let user = mine.user.as_deref().map(str::to_lowercase);
        assert!(user.as_deref().is_some_and(|u| u == me || u.ends_with(&format!("\\{me}"))), "{mine:?}");
    }

    #[test]
    fn free_file_has_no_holders() {
        let path = temp_file("libre");
        assert_eq!(holders_of(&path), Vec::new());
    }

    #[test]
    fn missing_file_has_no_holders() {
        let path = std::env::temp_dir().join(format!("xdir-holders-no-existe-{}", std::process::id())).join("nada.txt");
        assert_eq!(holders_of(&path), Vec::new());
    }

    #[test]
    fn verbatim_prefix_is_stripped() {
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        assert_eq!(plain_path(Path::new(r"\\?\C:\a\b.txt")), wide(r"C:\a\b.txt"));
        assert_eq!(plain_path(Path::new(r"\\?\UNC\srv\share\b.txt")), wide(r"\\srv\share\b.txt"));
        assert_eq!(plain_path(Path::new(r"C:\a\b.txt")), wide(r"C:\a\b.txt"));
    }

    #[test]
    fn unreadable_user_is_none() {
        // El proceso inactivo (PID 0) no se deja abrir nunca, ni siendo administrador.
        assert_eq!(user_of(0), None);
    }
}

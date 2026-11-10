//! Red en Windows (`WinNetwork`): conectar recursos y unidades con WNet (el diálogo de
//! credenciales es el de Windows y su «Recordar mis credenciales» las deja en el Administrador de
//! credenciales), lo conectado y lo recordado, desconectar, letras libres y los recursos de una
//! máquina. Lo bloqueante corre en hilos de trabajo; `is_remote` mira una lista guardada.

use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::NetManagement::{MAX_PREFERRED_LENGTH, NetApiBufferFree};
use windows::Win32::NetworkManagement::WNet::{
    CONNECT_INTERACTIVE, CONNECT_PROMPT, CONNECT_UPDATE_PROFILE, NET_CONNECT_FLAGS, NETRESOURCEW, RESOURCE_CONNECTED, RESOURCE_REMEMBERED,
    RESOURCETYPE_DISK, WNET_OPEN_ENUM_USAGE, WNetAddConnection3W, WNetCancelConnection2W, WNetCloseEnum, WNetEnumResourceW, WNetOpenEnumW,
};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use xdir_core::net::{Address, ConnectOptions, NetConnection, NetError};
use xdir_core::sys::{Handle, HostEvent, Network};

use crate::platform::HandleExt;

/// Código de WNet → `NetError`.
pub fn win_error(code: u32, host: &str, share: &str) -> NetError {
    match code {
        53 => NetError::NotFound { host: host.into() },
        67 => NetError::NoShare { host: host.into(), share: share.into() },
        5 | 1326 => NetError::BadCredentials,
        1219 => NetError::CredentialConflict { host: host.into() },
        1223 => NetError::Cancelled,
        2401 | 2404 => NetError::OpenFiles { by: None },
        85 => NetError::Other("La letra ya está en uso".into()),
        other => NetError::Other(std::io::Error::from_raw_os_error(other as i32).to_string()),
    }
}

/// Letras libres para una unidad de red, de Z hacia D: las que no están en `mask` (bit 0 = A) ni
/// recordadas.
pub fn free_letters(mask: u32, remembered: &[char]) -> Vec<char> {
    ('D'..='Z').rev().filter(|c| mask & (1 << (*c as u32 - 'A' as u32)) == 0 && !remembered.contains(c)).collect()
}

/// ¿Es una ruta UNC o de una de estas letras de red?
pub fn remote_by_form(path: &Path, letters: &[char]) -> bool {
    if xdir_core::fs::is_unc(path) {
        return true;
    }
    let s = path.to_string_lossy();
    let mut c = s.chars();
    match (c.next(), c.next()) {
        (Some(l), Some(':')) => letters.contains(&l.to_ascii_uppercase()),
        _ => false,
    }
}

fn pwstr(p: PWSTR) -> String {
    if p.is_null() { String::new() } else { unsafe { p.to_string().unwrap_or_default() } }
}

/// Recursos de disco de un alcance de WNet (`RESOURCE_CONNECTED` o `RESOURCE_REMEMBERED`):
/// (letra o vacío, nombre remoto).
fn enumerate(scope: windows::Win32::NetworkManagement::WNet::NET_RESOURCE_SCOPE) -> Result<Vec<(String, String)>, u32> {
    let mut h = HANDLE::default();
    let err = unsafe { WNetOpenEnumW(scope, RESOURCETYPE_DISK, WNET_OPEN_ENUM_USAGE(0), None, &mut h) };
    if err.0 != 0 {
        return Err(err.0);
    }
    let mut out = Vec::new();
    // 16 KB alcanzan para muchas entradas por llamada; se repite hasta ERROR_NO_MORE_ITEMS.
    let mut buf = vec![0u64; 2048];
    loop {
        let mut count = u32::MAX;
        let mut size = (buf.len() * 8) as u32;
        let err = unsafe { WNetEnumResourceW(h, &mut count, buf.as_mut_ptr().cast(), &mut size) };
        if err.0 != 0 {
            break;
        }
        let items = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const NETRESOURCEW, count as usize) };
        for r in items {
            out.push((pwstr(r.lpLocalName), pwstr(r.lpRemoteName)));
        }
    }
    unsafe {
        let _ = WNetCloseEnum(h);
    }
    Ok(out)
}

/// Letras de red de ahora (sin E/S de red: `GetDriveTypeW` mira el tipo de la unidad).
fn remote_letters() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    ('A'..='Z')
        .filter(|c| mask & (1 << (*c as u32 - 'A' as u32)) != 0)
        .filter(|c| unsafe { GetDriveTypeW(&HSTRING::from(format!("{c}:\\"))) } == 4 /* DRIVE_REMOTE */)
        .collect()
}

fn letter_of(local: &str) -> Option<char> {
    let mut c = local.chars();
    match (c.next(), c.next()) {
        (Some(l), Some(':')) => Some(l.to_ascii_uppercase()),
        _ => None,
    }
}

pub struct WinNetwork {
    letters: RwLock<Vec<char>>,
}

impl WinNetwork {
    pub fn new() -> WinNetwork {
        WinNetwork { letters: RwLock::new(remote_letters()) }
    }

    fn add(owner: Handle, remote: &str, letter: Option<char>, flags: NET_CONNECT_FLAGS) -> u32 {
        let mut remote_w: Vec<u16> = remote.encode_utf16().chain(Some(0)).collect();
        let mut local_w: Vec<u16> = letter.map(|l| format!("{l}:")).unwrap_or_default().encode_utf16().chain(Some(0)).collect();
        let res = NETRESOURCEW {
            dwType: RESOURCETYPE_DISK,
            lpRemoteName: PWSTR(remote_w.as_mut_ptr()),
            lpLocalName: if letter.is_some() { PWSTR(local_w.as_mut_ptr()) } else { PWSTR::null() },
            ..Default::default()
        };
        let hwnd = if owner.is_null() { None } else { Some(owner.hwnd()) };
        unsafe { WNetAddConnection3W(hwnd, &res, PCWSTR::null(), PCWSTR::null(), flags) }.0
    }
}

impl Network for WinNetwork {
    fn supports(&self, scheme: &str) -> bool {
        scheme == "smb"
    }

    fn connect(&self, owner: Handle, addr: &Address, opts: ConnectOptions, _cancel: &AtomicBool) -> Result<PathBuf, NetError> {
        if addr.scheme != "smb" {
            return Err(NetError::Unsupported { scheme: addr.scheme.clone() });
        }
        let share = addr.share.clone().ok_or_else(|| NetError::Other("Falta el recurso: \\\\servidor\\recurso".into()))?;
        let remote = format!(r"\\{}\{}", addr.host, share);
        let mut flags = CONNECT_INTERACTIVE;
        if opts.other_credentials {
            flags |= CONNECT_PROMPT;
        }
        if opts.reconnect {
            flags |= CONNECT_UPDATE_PROFILE;
        }
        let mut code = Self::add(owner, &remote, opts.drive_letter, flags);
        // Sin acceso con las credenciales guardadas: una vez más, pidiéndolas.
        if matches!(code, 5 | 1326) && !opts.other_credentials {
            code = Self::add(owner, &remote, opts.drive_letter, flags | CONNECT_PROMPT);
        }
        // 1202 (ERROR_DEVICE_ALREADY_REMEMBERED) y 85 sin letra no impiden abrir el recurso.
        if code != 0 && !(code == 1202 && opts.drive_letter.is_none()) {
            return Err(win_error(code, &addr.host, &share));
        }
        self.refresh_remote();
        let base = match opts.drive_letter {
            Some(l) => PathBuf::from(format!("{l}:\\")),
            None => PathBuf::from(remote),
        };
        Ok(addr.rest.iter().fold(base, |p, r| p.join(r)))
    }

    fn connections(&self) -> Result<Vec<NetConnection>, NetError> {
        let connected = enumerate(RESOURCE_CONNECTED).map_err(|c| win_error(c, "", ""))?;
        let remembered = enumerate(RESOURCE_REMEMBERED).unwrap_or_default();
        let to_conn = |local: &str, remote: &str, connected: bool| {
            let letter = letter_of(local);
            let address = Address::parse(remote).map(|a| a.to_url()).unwrap_or_else(|_| remote.to_string());
            let local = match letter {
                Some(l) => PathBuf::from(format!("{l}:\\")),
                None => PathBuf::from(remote),
            };
            NetConnection { address, local, letter, reconnect: false, connected }
        };
        let mut out: Vec<NetConnection> = connected.iter().map(|(l, r)| to_conn(l, r, true)).collect();
        for c in &mut out {
            c.reconnect = remembered.iter().any(|(l, _)| c.letter.is_some() && letter_of(l) == c.letter);
        }
        for (l, r) in &remembered {
            let letter = letter_of(l);
            if letter.is_some() && !out.iter().any(|c| c.letter == letter) {
                let mut c = to_conn(l, r, false);
                c.reconnect = true;
                out.push(c);
            }
        }
        Ok(out)
    }

    fn disconnect(&self, local: &Path, force: bool) -> Result<(), NetError> {
        let s = local.to_string_lossy();
        // Una unidad se nombra «Z:»; un recurso sin letra, por su UNC.
        let name = match letter_of(&s) {
            Some(l) => format!("{l}:"),
            None => s.trim_end_matches('\\').to_string(),
        };
        let code = unsafe { WNetCancelConnection2W(&HSTRING::from(name.as_str()), CONNECT_UPDATE_PROFILE, force) }.0;
        self.refresh_remote();
        if code == 0 { Ok(()) } else { Err(win_error(code, "", "")) }
    }

    fn shares(&self, host: &str) -> Result<Vec<String>, NetError> {
        let mut buf: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        let server = HSTRING::from(format!(r"\\{host}"));
        let code = unsafe { NetShareEnum(&server, 1, &mut buf, MAX_PREFERRED_LENGTH, &mut read, &mut total, None) };
        if code != 0 {
            return Err(win_error(code, host, ""));
        }
        let mut out = Vec::new();
        if !buf.is_null() {
            let items = unsafe { std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize) };
            for i in items {
                // Solo carpetas (sin impresoras ni los ocultos especiales como IPC$).
                if i.shi1_type.0 & 0xFF == STYPE_DISKTREE.0 {
                    out.push(pwstr(i.shi1_netname));
                }
            }
            unsafe {
                NetApiBufferFree(Some(buf as *const _));
            }
        }
        out.sort_by_key(|s| s.to_lowercase());
        Ok(out)
    }

    fn browse(&self, _hwnd: Handle, _tx: Sender<HostEvent>, _cancel: Arc<AtomicBool>) {
        // Windows muestra la red local con la Red del Shell en el árbol.
    }

    fn is_remote(&self, path: &Path) -> bool {
        let letters = self.letters.read().map(|l| l.clone()).unwrap_or_default();
        remote_by_form(path, &letters)
    }

    fn refresh_remote(&self) {
        let l = remote_letters();
        if let Ok(mut w) = self.letters.write() {
            *w = l;
        }
    }

    fn free_drive_letters(&self) -> Vec<char> {
        let mask = unsafe { GetLogicalDrives() };
        let remembered: Vec<char> = enumerate(RESOURCE_REMEMBERED).unwrap_or_default().iter().filter_map(|(l, _)| letter_of(l)).collect();
        free_letters(mask, &remembered)
    }

    fn can_force(&self) -> bool {
        true
    }

    fn open_credentials_place(&self, _owner: Handle) -> bool {
        std::process::Command::new("control").args(["/name", "Microsoft.CredentialManager"]).spawn().is_ok()
    }

    fn has_credentials_place(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_error_codes() {
        assert_eq!(win_error(53, "nas", "f"), NetError::NotFound { host: "nas".into() });
        assert_eq!(win_error(67, "nas", "f"), NetError::NoShare { host: "nas".into(), share: "f".into() });
        assert_eq!(win_error(5, "nas", "f"), NetError::BadCredentials);
        assert_eq!(win_error(1326, "nas", "f"), NetError::BadCredentials);
        assert_eq!(win_error(1219, "nas", "f"), NetError::CredentialConflict { host: "nas".into() });
        assert_eq!(win_error(1223, "nas", "f"), NetError::Cancelled);
        assert_eq!(win_error(2401, "nas", "f"), NetError::OpenFiles { by: None });
        assert_eq!(win_error(2404, "nas", "f"), NetError::OpenFiles { by: None });
        assert_eq!(win_error(85, "nas", "f"), NetError::Other("La letra ya está en uso".into()));
        assert!(matches!(win_error(1234, "nas", "f"), NetError::Other(_)));
    }

    #[test]
    fn free_letters_from_mask() {
        // A, C y Z ocupadas; Y recordada.
        let mask = 1 | (1 << 2) | (1 << 25);
        let f = free_letters(mask, &['Y']);
        assert_eq!(f.first(), Some(&'X'));
        assert!(!f.contains(&'C') && !f.contains(&'Z') && !f.contains(&'A') && !f.contains(&'B'));
        assert_eq!(free_letters(0, &[])[0], 'Z');
        assert_eq!(*free_letters(0, &[]).last().unwrap(), 'D');
    }

    #[test]
    fn win_is_remote_unc() {
        assert!(remote_by_form(Path::new(r"\\srv\rec\a"), &[]));
        assert!(remote_by_form(Path::new(r"Z:\fotos"), &['Z']));
        assert!(remote_by_form(Path::new(r"z:\fotos"), &['Z']));
        assert!(!remote_by_form(Path::new(r"C:\Users"), &['Z']));
        assert!(!remote_by_form(Path::new(r"\\?\C:\x"), &[]));
    }

    #[test]
    fn win_connections_enum_runs() {
        // En una máquina sin conexiones devuelve una lista (vacía), sin error.
        assert!(WinNetwork::new().connections().is_ok());
    }
}

//! Errores de Windows como los ve el usuario: el `HRESULT` (o el código Win32) pasa a un
//! `ErrKind` del núcleo, y una escritura frenada por el **acceso controlado a carpetas** de
//! Defender (protección contra ransomware) pasa a `ErrKind::Blocked`, que dice cómo permitir xdir.
//!
//! El acceso controlado a carpetas no avisa a la app: la escritura falla con «acceso denegado», «no
//! se encuentra» o un error de E/S (issue #72: crear una carpeta en Documentos redirigido decía «no
//! se encuentra»). Se sospecha de él solo si está encendido en modo bloquear, la ruta está en una
//! carpeta protegida y xdir no está en la lista de apps permitidas. El registro se lee una sola
//! vez, y recién la primera vez que falla algo.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ, RegCloseKey, RegEnumValueW, RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Favorites, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath,
};
use windows::core::{GUID, HSTRING, PWSTR};

use xdir_core::sys::{CopyError, ErrKind, starts_with_folded, too_long, unexpected};

use super::prims::err_kind;

/// `ERROR_IO_DEVICE`: «error de E/S» (lo da también el acceso controlado a carpetas).
const ERROR_IO_DEVICE: u32 = 1117;

/// `HRESULT` → `ErrKind`. Los de Win32 (`0x8007xxxx`) van por `err_kind`; los del motor de copia
/// del Shell (`COPYENGINE_E_*`, de `IFileOperation`) y de los property store, por su sentido; lo
/// demás, con el texto y el código de Windows.
pub(crate) fn hr_kind(hr: u32, text: &str) -> ErrKind {
    if hr >> 16 == 0x8007 {
        return err_kind(hr & 0xFFFF);
    }
    match hr {
        // STG_E_ACCESSDENIED; COPYENGINE_E_ACCESS_DENIED_SRC / _DEST / _ACCESSDENIED_READONLY.
        0x8003_0005 | 0x8027_0021 | 0x8027_0022 | 0x8027_003F => ErrKind::Denied,
        // STG_E_SHAREVIOLATION, STG_E_LOCKVIOLATION; COPYENGINE_E_SHARING_VIOLATION_SRC / _DEST.
        0x8003_0020 | 0x8003_0021 | 0x8027_0027 | 0x8027_0028 => ErrKind::InUse,
        // STG_E_FILENOTFOUND, STG_E_PATHNOTFOUND; COPYENGINE_E_PATH_NOT_FOUND_SRC / _DEST.
        0x8003_0002 | 0x8003_0003 | 0x8027_0023 | 0x8027_0024 => ErrKind::NotFound,
        // COPYENGINE_E_DISK_FULL, _DISK_FULL_CLEAN, _REMOVABLE_FULL; STG_E_MEDIUMFULL.
        0x8027_0032 | 0x8027_0033 | 0x8027_000E | 0x8003_0070 => ErrKind::DiskFull,
        // COPYENGINE_E_FILE_TOO_LARGE.
        0x8027_000D => ErrKind::TooBig,
        // COPYENGINE_E_ALREADY_EXISTS_*.
        0x8027_0029..=0x8027_002C => ErrKind::Exists,
        // E_ABORT; COPYENGINE_E_USER_CANCELLED, _CANCELLED.
        0x8000_4004 | 0x8027_0000 | 0x8027_0001 => ErrKind::Cancelled,
        // COPYENGINE_E_NET_DISCONNECT_SRC / _DEST.
        0x8027_0025 | 0x8027_0026 => ErrKind::Offline,
        // COPYENGINE_E_RECYCLE_UNKNOWN_ERROR (el mismo número que _CANT_REACH_SOURCE: casi siempre
        // llega al mandar a la Papelera).
        0x8027_0035 => ErrKind::Other("No se pudo mandar a la Papelera".into()),
        // COPYENGINE_E_DEST_IS_*CD / *DVD (de solo lectura o que no se escriben así).
        0x8027_000F..=0x8027_0014 => ErrKind::ReadOnly,
        // COPYENGINE_E_NEWFILE_NAME_TOO_LONG, _NEWFOLDER_NAME_TOO_LONG.
        0x8027_003B | 0x8027_003C => ErrKind::NameTooLong,
        // COPYENGINE_E_PATH_TOO_DEEP_SRC / _DEST.
        0x8027_001D | 0x8027_001E => ErrKind::PathTooLong,
        // COPYENGINE_E_INVALID_FILES_SRC / _DEST.
        0x8027_001B | 0x8027_001C => ErrKind::InvalidName,
        // COPYENGINE_E_REQUIRES_ELEVATION.
        0x8027_0002 => ErrKind::Other("Hace falta permiso de administrador: pedíselo a quien administra la PC".into()),
        // COPYENGINE_E_BLOCKED_BY_EDP_POLICY, _BLOCKED_BY_EDP_FOR_REMOVABLE_DRIVE,
        // _RMS_BLOCKED_BY_EDP_FOR_REMOVABLE_DRIVE, _BLOCKED_BY_DLP_POLICY, _SILENT_FAIL_BY_DLP_POLICY.
        0x8027_0048 | 0x8027_004A | 0x8027_004C | 0x8027_004E | 0x8027_004F => {
            ErrKind::Other("Lo bloquea una política de protección de datos de la empresa: consultá con sistemas".into())
        }
        _ => unexpected("Windows", text, &format!("0x{hr:08X}")),
    }
}

/// Un error de la API de Windows con la ruta que lo causó. El nombre demasiado largo
/// (`ERROR_FILENAME_EXCED_RANGE`) se separa del de la ruta entera mirando la ruta.
pub(crate) fn from_hr(hr: u32, text: &str, p: &Path) -> CopyError {
    let kind = match hr_kind(hr, text) {
        ErrKind::NameTooLong => too_long(p),
        k => k,
    };
    CopyError::new(kind, p)
}

/// Lo que se sabe del acceso controlado a carpetas.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Cfa {
    /// `EnableControlledFolderAccess`: 1 bloquea, 2 solo audita (no bloquea), 3 y 4 son solo
    /// para los sectores del disco. `None`: no se pudo leer (no se afirma nada).
    pub mode: Option<u32>,
    /// xdir está en la lista de apps permitidas.
    pub allowed: bool,
    /// Se pudieron leer todas las listas de apps permitidas (si no, xdir podría estar y no se
    /// sabe: el aviso dice «puede que»).
    pub allowed_known: bool,
    /// Carpetas protegidas: las conocidas (Documentos, Imágenes…, ya resueltas aunque estén
    /// redirigidas) y las agregadas en `ProtectedFolders`.
    pub protected: Vec<PathBuf>,
}

impl Cfa {
    /// ¿Puede haber sido el acceso controlado a carpetas el que frenó una escritura en `path`
    /// con `kind`? `io_device`: el código era `ERROR_IO_DEVICE`. `present`: lo que se tocaba
    /// estaba (la carpeta donde se creaba algo, o lo que se renombraba o borraba); si no estaba,
    /// «no se encuentra» es de verdad. `Some(seguro)`: si se pudo leer que xdir no está permitido.
    pub(crate) fn blocks(&self, kind: &ErrKind, io_device: bool, path: &Path, present: bool) -> Option<bool> {
        let suspect = match kind {
            ErrKind::Denied => true,
            ErrKind::NotFound => present,
            _ => io_device,
        };
        let blocked = suspect && self.mode == Some(1) && !self.allowed && self.protected.iter().any(|f| starts_with_folded(path, f));
        blocked.then_some(self.allowed_known)
    }
}

/// El modo que vale: la directiva de grupo manda, después lo que pone Intune (MDM) y al final lo
/// elegido en la PC.
pub(crate) fn mode_from(policy: Option<u32>, mdm: Option<u32>, setting: Option<u32>) -> Option<u32> {
    policy.or(mdm).or(setting)
}

/// Una lista de Intune (MDM) en un solo texto: rutas separadas por `|` o por el carácter U+F000
/// (lo que usa el CSP de Defender), sin vacíos.
pub(crate) fn parse_mdm_list(s: &str) -> Vec<String> {
    s.split(['|', '\u{F000}', '\0', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// ¿Está `exe` en la lista de apps permitidas? (sin distinguir mayúsculas; la lista puede tener
/// variables como `%ProgramFiles%` ya expandidas por quien la escribió, o no: se expanden).
pub(crate) fn allowed_in(exe: &Path, list: &[String], expand: &dyn Fn(&str) -> String) -> bool {
    list.iter().any(|a| xdir_core::sys::same_path_folded(Path::new(&expand(a)), exe))
}

/// El error de una escritura en `path`: el `ErrKind` del `HRESULT`, o `Blocked` si fue el acceso
/// controlado a carpetas (ver `Cfa::blocks`; `present` se pregunta solo si hace falta).
pub(crate) fn write_kind(hr: u32, text: &str, path: &Path, present: &dyn Fn() -> bool, cfa: &dyn Fn() -> Cfa) -> CopyError {
    let e = from_hr(hr, text, path);
    let io_device = hr >> 16 == 0x8007 && hr & 0xFFFF == ERROR_IO_DEVICE;
    let worth = matches!(e.kind, ErrKind::Denied | ErrKind::NotFound) || io_device;
    if worth && let Some(sure) = cfa().blocks(&e.kind, io_device, path, e.kind != ErrKind::NotFound || present()) {
        return CopyError::new(ErrKind::Blocked { sure }, path);
    }
    e
}

/// Como `write_kind`, con lo que dice el registro de esta PC (ver `cfa`).
pub(crate) fn write_error(hr: u32, text: &str, path: &Path, present: &dyn Fn() -> bool) -> CopyError {
    write_kind(hr, text, path, present, &cfa)
}

/// ¿Está? (sin seguir enlaces).
pub(crate) fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

const CFA_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access";
const CFA_SETTING: &str = r"SOFTWARE\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access";
/// Lo que pone Intune (MDM, CSP de Defender): las listas van en un solo texto (`parse_mdm_list`).
const CFA_MDM: &str = r"SOFTWARE\Microsoft\PolicyManager\current\device\Defender";

/// Cuánto vale lo leído del registro: si se cambia el permiso con xdir abierto, a los 30 s ya
/// cuenta.
const CFA_FRESH: std::time::Duration = std::time::Duration::from_secs(30);

/// Lo que dice el registro de esta PC: se lee la primera vez que falla una escritura y se vuelve
/// a leer si pasaron más de `CFA_FRESH` (siempre en el hilo de la operación, no en el de interfaz).
fn cfa() -> Cfa {
    static CFA: Mutex<Option<(Instant, Cfa)>> = Mutex::new(None);
    let mut cached = CFA.lock().unwrap_or_else(|e| e.into_inner());
    match cached.as_ref() {
        Some((at, c)) if at.elapsed() < CFA_FRESH => c.clone(),
        _ => {
            let c = read_cfa();
            *cached = Some((Instant::now(), c.clone()));
            c
        }
    }
}

fn read_cfa() -> Cfa {
    let mode = mode_from(
        read_dword(CFA_POLICY, "EnableControlledFolderAccess"),
        read_dword(CFA_MDM, "EnableControlledFolderAccess"),
        read_dword(CFA_SETTING, "EnableControlledFolderAccess"),
    );
    if mode != Some(1) {
        return Cfa { mode, ..Cfa::default() };
    }
    let mut protected: Vec<PathBuf> =
        [FOLDERID_Documents, FOLDERID_Pictures, FOLDERID_Videos, FOLDERID_Music, FOLDERID_Desktop, FOLDERID_Favorites].iter().filter_map(known_folder).collect();
    let (mut apps, mut allowed_known) = (Vec::new(), true);
    for key in [CFA_POLICY, CFA_SETTING] {
        protected.extend(value_names(&format!(r"{key}\ProtectedFolders")).unwrap_or_default().iter().map(|f| PathBuf::from(expand(f))));
        match value_names(&format!(r"{key}\AllowedApplications")) {
            Some(list) => apps.extend(list),
            None => allowed_known = false,
        }
    }
    if let Ok(list) = read_string(CFA_MDM, "ControlledFolderAccessProtectedFolders") {
        protected.extend(parse_mdm_list(&list.unwrap_or_default()).iter().map(|f| PathBuf::from(expand(f))));
    }
    match read_string(CFA_MDM, "ControlledFolderAccessAllowedApplications") {
        Ok(list) => apps.extend(parse_mdm_list(&list.unwrap_or_default())),
        Err(()) => allowed_known = false,
    }
    let allowed = std::env::current_exe().is_ok_and(|exe| allowed_in(&exe, &apps, &expand));
    Cfa { mode, allowed, allowed_known, protected }
}

/// Un texto de `HKLM\key\value`: `Ok(None)` si no está, `Err` si no se pudo leer.
fn read_string(key: &str, value: &str) -> Result<Option<String>, ()> {
    let (k, v) = (HSTRING::from(key), HSTRING::from(value));
    let mut size = 0u32;
    let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &k, &v, RRF_RT_REG_SZ | RRF_RT_REG_MULTI_SZ, None, None, Some(&mut size)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(());
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &k, &v, RRF_RT_REG_SZ | RRF_RT_REG_MULTI_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) };
    if status != ERROR_SUCCESS {
        return Err(());
    }
    let len = (size as usize / 2).min(buf.len());
    // REG_MULTI_SZ: los textos van separados por ceros (`parse_mdm_list` los separa).
    Ok(Some(String::from_utf16_lossy(&buf[..len])))
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn read_dword(key: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// Los nombres de los valores de una clave de `HKLM` (las listas de Defender guardan cada
/// carpeta o app como nombre de un valor). Vacía si la clave no está; `None` si no se puede leer
/// (sin permiso).
fn value_names(key: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut h = HKEY::default();
    match unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), None, KEY_READ, &mut h) } {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Some(out),
        _ => return None,
    }
    let mut buf = vec![0u16; 32_768];
    for i in 0.. {
        let mut len = buf.len() as u32;
        let status = unsafe { RegEnumValueW(h, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
        if status != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    unsafe {
        let _ = RegCloseKey(h);
    }
    Some(out)
}

/// `%VARIABLE%` → su valor (si no está definida, queda como estaba).
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else { break };
        let name = &rest[start + 1..start + 1 + len];
        out.push_str(&rest[..start]);
        match std::env::var(name) {
            Ok(v) if !name.is_empty() => out.push_str(&v),
            _ => out.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// El aviso de una operación del Shell que falló con `failed` (`present`: lo que se tocaba
/// está) y su explicación entera, si la tiene (el cartel). «No se encuentra» con la carpeta ahí
/// no se explica (issue #72, cuando no se pudo leer si el acceso controlado a carpetas está
/// encendido): se dice eso y se nombra la sospecha, sin afirmarla.
pub(crate) fn op_message(op: &xdir_core::ops::Op, failed: &CopyError, present: bool) -> (String, Option<String>) {
    use xdir_core::ops::{Op, doing, failure, failure_detail};
    let odd = failed.kind == ErrKind::NotFound && present && matches!(op, Op::NewFolder { .. } | Op::NewFile { .. } | Op::Rename { .. });
    if odd {
        let name = xdir_core::sys::shown_name(&failed.path);
        let what = doing(op, &failed.path);
        let notice = format!("No se pudo {what}: Windows no encuentra «{name}», pero está. Puede ser el acceso controlado a carpetas.");
        let detail = format!(
            "Windows dice que no encuentra «{name}», pero está: no se pudo {what}. Puede que una protección del sistema \
             (como el acceso controlado a carpetas de Seguridad de Windows) no deje a xdir escribir ahí. Si es así, permití xdir \
             en Seguridad de Windows ▸ Protección contra virus y amenazas ▸ Protección contra ransomware ▸ Permitir una aplicación \
             a través del acceso controlado a carpetas (en una PC de empresa, lo hace sistemas)."
        );
        return (notice, Some(detail));
    }
    (failure(op, failed), failure_detail(op, failed))
}

/// El texto de un error de Windows como lo ve el usuario (sin ruta: el motivo corto).
pub(crate) fn win_message(e: &windows::core::Error) -> String {
    let failed = from_hr(e.code().0 as u32, &e.message(), Path::new(""));
    match failed.kind {
        // Sin ruta no se sabe cuál de los dos.
        ErrKind::NameTooLong | ErrKind::PathTooLong => "El nombre o la ruta es demasiado largo".into(),
        _ => failed.message(),
    }
}

/// Por qué no se pudo leer la carpeta `p`, para la barra de estado: el motivo corto y siempre el
/// código de Windows (para diagnosticar, y para reconocer las fallas de credenciales de una
/// carpeta de red: `Tab::retry_with_credentials`).
pub(crate) fn load_message(e: &std::io::Error, p: &Path) -> String {
    let Some(n) = e.raw_os_error() else { return e.to_string() };
    let hr = 0x8007_0000 | (n as u32 & 0xFFFF);
    let text = e.to_string();
    let message = from_hr(hr, text.split(" (os error").next().unwrap_or(&text), p).message();
    if message.contains("Windows: 0x") { message } else { format!("{message} (Windows: 0x{hr:08X})") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(protected: &[&str]) -> Cfa {
        Cfa { mode: Some(1), allowed: false, allowed_known: true, protected: protected.iter().map(PathBuf::from).collect() }
    }

    /// Issue #72: Documentos redirigido a `C:\Documentos\d27523`; crear «Github» ahí falla con «no
    /// se encuentra» aunque la carpeta está: es el acceso controlado a carpetas.
    #[test]
    fn cfa_blocks_writes_in_protected_folders() {
        let cfa = on(&[r"C:\Documentos\d27523"]);
        let docs = Path::new(r"C:\Documentos\d27523\Github");
        assert_eq!(Some(true), cfa.blocks(&ErrKind::NotFound, false, docs, true));
        assert_eq!(Some(true), cfa.blocks(&ErrKind::Denied, false, docs, true));
        assert_eq!(Some(true), cfa.blocks(&ErrKind::Other("E/S".into()), true, docs, true));
        // Sin distinguir mayúsculas ni la barra.
        assert_eq!(Some(true), cfa.blocks(&ErrKind::Denied, false, Path::new(r"c:/documentos/D27523/x.txt"), true));
        // «No se encuentra» de verdad: la carpeta donde se creaba no está.
        assert_eq!(None, cfa.blocks(&ErrKind::NotFound, false, docs, false));
        // Otros motivos no son del acceso controlado.
        assert_eq!(None, cfa.blocks(&ErrKind::InUse, false, docs, true));
        assert_eq!(None, cfa.blocks(&ErrKind::Other("raro".into()), false, docs, true));
        // Fuera de las carpetas protegidas, tampoco.
        assert_eq!(None, cfa.blocks(&ErrKind::Denied, false, Path::new(r"C:\Proyectos\Github"), true));
        assert_eq!(None, cfa.blocks(&ErrKind::Denied, false, Path::new(r"C:\Documentos\d27523-otra"), true));
    }

    /// Solo con el modo bloquear y xdir sin permitir; sin poder leer el modo, nada.
    #[test]
    fn cfa_needs_block_mode_and_xdir_not_allowed() {
        let p = Path::new(r"C:\Users\ana\Documents\Github");
        let mut cfa = on(&[r"C:\Users\ana\Documents"]);
        assert_eq!(Some(true), cfa.blocks(&ErrKind::Denied, false, p, true));
        for mode in [None, Some(0), Some(2), Some(3), Some(4)] {
            cfa.mode = mode;
            assert_eq!(None, cfa.blocks(&ErrKind::Denied, false, p, true), "{mode:?}");
        }
        cfa.mode = Some(1);
        cfa.allowed = true;
        assert_eq!(None, cfa.blocks(&ErrKind::Denied, false, p, true));
        // Sin poder leer la lista de apps permitidas: se dice con «puede que».
        cfa.allowed = false;
        cfa.allowed_known = false;
        assert_eq!(Some(false), cfa.blocks(&ErrKind::Denied, false, p, true));
    }

    #[test]
    fn cfa_policy_wins_over_setting() {
        assert_eq!(mode_from(Some(2), Some(1), Some(1)), Some(2));
        assert_eq!(mode_from(None, Some(1), Some(0)), Some(1));
        assert_eq!(mode_from(None, None, Some(1)), Some(1));
        assert_eq!(mode_from(None, None, None), None);
    }

    /// Las listas de Intune vienen en un solo texto.
    #[test]
    fn cfa_mdm_lists() {
        assert_eq!(
            parse_mdm_list("C:\\Program Files\\xdir\\xdir.exe|%USERPROFILE%\\Proyectos\u{F000}D:\\Datos\0\0"),
            vec![r"C:\Program Files\xdir\xdir.exe".to_string(), r"%USERPROFILE%\Proyectos".to_string(), r"D:\Datos".to_string()]
        );
        assert!(parse_mdm_list("").is_empty());
        assert!(parse_mdm_list(" | ").is_empty());
    }

    #[test]
    fn cfa_allowed_apps_compare_paths() {
        let exe = Path::new(r"C:\Program Files\xdir\xdir.exe");
        let list = [r"%ProgramFiles%\xdir\XDIR.exe".to_string()];
        let expand = |s: &str| s.replace("%ProgramFiles%", r"C:\Program Files");
        assert!(allowed_in(exe, &list, &expand));
        assert!(!allowed_in(Path::new(r"C:\otro\xdir.exe"), &list, &expand));
    }

    /// Crear una carpeta (`IFileOperation`, error como HRESULT) o escribir con el motor de copia:
    /// con el acceso controlado encendido y la ruta protegida, `Blocked`; si no, el motivo común.
    #[test]
    fn write_kind_turns_cfa_failures_into_blocked() {
        let parent = Path::new(r"C:\Documentos\d27523");
        let cfa = || on(&[r"C:\Documentos\d27523"]);
        let off = || Cfa { mode: Some(0), ..on(&[r"C:\Documentos\d27523"]) };
        let yes = || true;
        for hr in [0x8007_0002, 0x8007_0003, 0x8007_0005, 0x8007_045D, 0x8027_0022, 0x8027_0024] {
            assert_eq!(write_kind(hr, "x", parent, &yes, &cfa).kind, ErrKind::Blocked { sure: true }, "{hr:08X}");
        }
        assert_eq!(write_kind(0x8007_0005, "x", parent, &yes, &off).kind, ErrKind::Denied);
        assert_eq!(write_kind(0x8007_0002, "x", parent, &|| false, &cfa).kind, ErrKind::NotFound);
        assert_eq!(write_kind(0x8007_0020, "x", parent, &yes, &cfa).kind, ErrKind::InUse);
        let unsure = || Cfa { allowed_known: false, ..on(&[r"C:\Documentos\d27523"]) };
        assert_eq!(write_kind(0x8007_0005, "x", parent, &yes, &unsure).kind, ErrKind::Blocked { sure: false });
        let e = write_kind(0x8007_0002, "x", parent, &yes, &cfa);
        let notice = e.notice("crear la carpeta nueva en «d27523»");
        assert!(notice.contains("«d27523»") && notice.contains("acceso controlado a carpetas"), "{notice}");
    }

    /// Lo que no se explica (no se pudo leer el registro) se dice sin afirmar.
    #[test]
    fn op_message_when_not_found_but_there() {
        use xdir_core::ops::Op;
        let parent = PathBuf::from(r"C:\Documentos\d27523");
        let op = Op::NewFolder { parent: parent.clone() };
        let gone = CopyError::new(ErrKind::NotFound, &parent);
        let (odd, detail) = op_message(&op, &gone, true);
        assert_eq!(odd, "No se pudo crear la carpeta nueva en «d27523»: Windows no encuentra «d27523», pero está. Puede ser el acceso controlado a carpetas.");
        assert!(detail.is_some_and(|d| d.contains("Permitir una aplicación")));
        assert_eq!(op_message(&op, &gone, false), ("No se pudo crear la carpeta nueva en «d27523»: ya no existe.".to_string(), None));
        let blocked = CopyError::new(ErrKind::Blocked { sure: true }, &parent);
        let (notice, detail) = op_message(&op, &blocked, true);
        assert!(notice.starts_with("Windows bloqueó a xdir en «d27523»"), "{notice}");
        assert!(detail.is_some_and(|d| d.contains("Permitir una aplicación")));
    }

    #[test]
    fn hresults_map_to_kinds() {
        assert_eq!(hr_kind(0x8007_0005, ""), ErrKind::Denied);
        assert_eq!(hr_kind(0x8007_0020, ""), ErrKind::InUse);
        assert_eq!(hr_kind(0x8003_0020, ""), ErrKind::InUse);
        assert_eq!(hr_kind(0x8027_0024, ""), ErrKind::NotFound);
        assert_eq!(hr_kind(0x8027_0032, ""), ErrKind::DiskFull);
        assert_eq!(hr_kind(0x8027_0026, ""), ErrKind::Offline);
        // Al mandar a la Papelera: no es «la red no responde».
        assert_eq!(hr_kind(0x8027_0035, ""), ErrKind::Other("No se pudo mandar a la Papelera".into()));
        assert_eq!(hr_kind(0x8027_003C, ""), ErrKind::NameTooLong);
        assert_eq!(hr_kind(0x8027_001E, ""), ErrKind::PathTooLong);
        assert_eq!(hr_kind(0x8027_0000, ""), ErrKind::Cancelled);
        assert_eq!(
            hr_kind(0x8004_0154, "Clase no registrada\r\n"),
            ErrKind::Other("Error inesperado del sistema (Clase no registrada; Windows: 0x80040154)".into())
        );
    }

    /// El código de Windows va una sola vez, y sin ruta no se elige entre nombre y ruta.
    #[test]
    fn load_and_bare_messages() {
        let e = std::io::Error::from_raw_os_error(1117);
        let m = load_message(&e, Path::new(r"C:\d"));
        assert_eq!(m.matches("Windows: 0x8007045D").count(), 1, "{m}");
        let m = load_message(&std::io::Error::from_raw_os_error(5), Path::new(r"C:\d"));
        assert_eq!(m, "No tenés permiso (Windows: 0x80070005)");
        let long = windows::core::Error::from_hresult(windows::core::HRESULT(0x8007_00CE_u32 as i32));
        assert_eq!(win_message(&long), "El nombre o la ruta es demasiado largo");
    }

    #[test]
    fn name_or_path_too_long() {
        let long_name = format!(r"C:\d\{}", "a".repeat(256));
        assert_eq!(from_hr(0x8007_00CE, "", Path::new(&long_name)).kind, ErrKind::NameTooLong);
        let deep = r"C:".to_string() + &r"\carpeta".repeat(40);
        assert_eq!(from_hr(0x8007_00CE, "", Path::new(&deep)).kind, ErrKind::PathTooLong);
    }
}

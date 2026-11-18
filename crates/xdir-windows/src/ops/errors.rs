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
use std::sync::OnceLock;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD, RegCloseKey, RegEnumValueW, RegGetValueW, RegOpenKeyExW,
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
        // COPYENGINE_E_NET_DISCONNECT_SRC / _DEST, _CANT_REACH_SOURCE.
        0x8027_0025 | 0x8027_0026 | 0x8027_0035 => ErrKind::Offline,
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
    /// Carpetas protegidas: las conocidas (Documentos, Imágenes…, ya resueltas aunque estén
    /// redirigidas) y las agregadas en `ProtectedFolders`.
    pub protected: Vec<PathBuf>,
}

impl Cfa {
    /// ¿Puede haber sido el acceso controlado a carpetas el que frenó una escritura en `path`
    /// con `kind`? `io_device`: el código era `ERROR_IO_DEVICE`. `present`: lo que se tocaba
    /// estaba (la carpeta donde se creaba algo, o lo que se renombraba o borraba); si no estaba,
    /// «no se encuentra» es de verdad.
    pub(crate) fn blocks(&self, kind: &ErrKind, io_device: bool, path: &Path, present: bool) -> bool {
        let suspect = match kind {
            ErrKind::Denied => true,
            ErrKind::NotFound => present,
            _ => io_device,
        };
        suspect && self.mode == Some(1) && !self.allowed && self.protected.iter().any(|f| starts_with_folded(path, f))
    }
}

/// El modo que vale: la política (lo que pone la empresa) manda sobre lo elegido en la PC.
pub(crate) fn mode_from(policy: Option<u32>, setting: Option<u32>) -> Option<u32> {
    policy.or(setting)
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
    if worth && cfa().blocks(&e.kind, io_device, path, e.kind != ErrKind::NotFound || present()) {
        return CopyError::new(ErrKind::Blocked, path);
    }
    e
}

/// Como `write_kind`, con lo que dice el registro de esta PC (leído una vez).
pub(crate) fn write_error(hr: u32, text: &str, path: &Path, present: &dyn Fn() -> bool) -> CopyError {
    write_kind(hr, text, path, present, &|| cfa().clone())
}

/// ¿Está? (sin seguir enlaces).
pub(crate) fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

const CFA_POLICY: &str = r"SOFTWARE\Policies\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access";
const CFA_SETTING: &str = r"SOFTWARE\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access";

/// Lo que dice el registro de esta PC (una vez por ejecución).
fn cfa() -> &'static Cfa {
    static CFA: OnceLock<Cfa> = OnceLock::new();
    CFA.get_or_init(read_cfa)
}

fn read_cfa() -> Cfa {
    let mode = mode_from(read_dword(CFA_POLICY, "EnableControlledFolderAccess"), read_dword(CFA_SETTING, "EnableControlledFolderAccess"));
    if mode != Some(1) {
        return Cfa { mode, ..Cfa::default() };
    }
    let mut protected: Vec<PathBuf> =
        [FOLDERID_Documents, FOLDERID_Pictures, FOLDERID_Videos, FOLDERID_Music, FOLDERID_Desktop, FOLDERID_Favorites].iter().filter_map(known_folder).collect();
    let mut apps = Vec::new();
    for key in [CFA_POLICY, CFA_SETTING] {
        protected.extend(value_names(&format!(r"{key}\ProtectedFolders")).iter().map(|f| PathBuf::from(expand(f))));
        apps.extend(value_names(&format!(r"{key}\AllowedApplications")));
    }
    let allowed = std::env::current_exe().is_ok_and(|exe| allowed_in(&exe, &apps, &expand));
    Cfa { mode, allowed, protected }
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
/// carpeta o app como nombre de un valor). Vacía si no se puede leer.
fn value_names(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut h = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), None, KEY_READ, &mut h) } != ERROR_SUCCESS {
        return out;
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
    out
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
/// está). «No se encuentra» con la carpeta ahí no se explica (issue #72, cuando no se pudo leer si
/// el acceso controlado a carpetas está encendido): se dice eso y se nombra la sospecha, sin
/// afirmarla.
pub(crate) fn op_message(op: &xdir_core::ops::Op, failed: &CopyError, present: bool) -> String {
    use xdir_core::ops::{Op, doing, failure};
    let odd = failed.kind == ErrKind::NotFound && present && matches!(op, Op::NewFolder { .. } | Op::NewFile { .. } | Op::Rename { .. });
    if odd {
        let name = xdir_core::sys::shown_name(&failed.path);
        return format!(
            "No se pudo {}: Windows dice que no encuentra «{name}», pero está. Puede que una protección del sistema \
             (como el acceso controlado a carpetas de Seguridad de Windows) no deje escribir ahí: probá permitir xdir \
             o consultá con sistemas.",
            doing(op, &failed.path)
        );
    }
    failure(op, failed)
}

/// El texto de un error de Windows como lo ve el usuario (sin ruta: el motivo y qué hacer).
pub(crate) fn win_message(e: &windows::core::Error) -> String {
    from_hr(e.code().0 as u32, &e.message(), Path::new("")).message()
}

/// Por qué no se pudo leer la carpeta `p`, para la barra de estado: el motivo y qué hacer, y
/// siempre el código de Windows (para diagnosticar, y para reconocer las fallas de credenciales
/// de una carpeta de red: `Tab::retry_with_credentials`).
pub(crate) fn load_message(e: &std::io::Error, p: &Path) -> String {
    let Some(n) = e.raw_os_error() else { return e.to_string() };
    let hr = 0x8007_0000 | (n as u32 & 0xFFFF);
    let text = e.to_string();
    let message = from_hr(hr, text.split(" (os error").next().unwrap_or(&text), p).message();
    if message.contains("(Windows: ") { message } else { format!("{message} (Windows: 0x{hr:08X})") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(protected: &[&str]) -> Cfa {
        Cfa { mode: Some(1), allowed: false, protected: protected.iter().map(PathBuf::from).collect() }
    }

    /// Issue #72: Documentos redirigido a `C:\Documentos\d27523`; crear «Github» ahí falla con «no
    /// se encuentra» aunque la carpeta está: es el acceso controlado a carpetas.
    #[test]
    fn cfa_blocks_writes_in_protected_folders() {
        let cfa = on(&[r"C:\Documentos\d27523"]);
        let docs = Path::new(r"C:\Documentos\d27523\Github");
        assert!(cfa.blocks(&ErrKind::NotFound, false, docs, true));
        assert!(cfa.blocks(&ErrKind::Denied, false, docs, true));
        assert!(cfa.blocks(&ErrKind::Other("E/S".into()), true, docs, true));
        // Sin distinguir mayúsculas ni la barra.
        assert!(cfa.blocks(&ErrKind::Denied, false, Path::new(r"c:/documentos/D27523/x.txt"), true));
        // «No se encuentra» de verdad: la carpeta donde se creaba no está.
        assert!(!cfa.blocks(&ErrKind::NotFound, false, docs, false));
        // Otros motivos no son del acceso controlado.
        assert!(!cfa.blocks(&ErrKind::InUse, false, docs, true));
        assert!(!cfa.blocks(&ErrKind::Other("raro".into()), false, docs, true));
        // Fuera de las carpetas protegidas, tampoco.
        assert!(!cfa.blocks(&ErrKind::Denied, false, Path::new(r"C:\Proyectos\Github"), true));
        assert!(!cfa.blocks(&ErrKind::Denied, false, Path::new(r"C:\Documentos\d27523-otra"), true));
    }

    /// Solo con el modo bloquear y xdir sin permitir; sin poder leer el modo, nada.
    #[test]
    fn cfa_needs_block_mode_and_xdir_not_allowed() {
        let p = Path::new(r"C:\Users\ana\Documents\Github");
        let mut cfa = on(&[r"C:\Users\ana\Documents"]);
        assert!(cfa.blocks(&ErrKind::Denied, false, p, true));
        for mode in [None, Some(0), Some(2), Some(3), Some(4)] {
            cfa.mode = mode;
            assert!(!cfa.blocks(&ErrKind::Denied, false, p, true), "{mode:?}");
        }
        cfa.mode = Some(1);
        cfa.allowed = true;
        assert!(!cfa.blocks(&ErrKind::Denied, false, p, true));
    }

    #[test]
    fn cfa_policy_wins_over_setting() {
        assert_eq!(mode_from(Some(2), Some(1)), Some(2));
        assert_eq!(mode_from(None, Some(1)), Some(1));
        assert_eq!(mode_from(None, None), None);
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
            assert_eq!(write_kind(hr, "x", parent, &yes, &cfa).kind, ErrKind::Blocked, "{hr:08X}");
        }
        assert_eq!(write_kind(0x8007_0005, "x", parent, &yes, &off).kind, ErrKind::Denied);
        assert_eq!(write_kind(0x8007_0002, "x", parent, &|| false, &cfa).kind, ErrKind::NotFound);
        assert_eq!(write_kind(0x8007_0020, "x", parent, &yes, &cfa).kind, ErrKind::InUse);
        let e = write_kind(0x8007_0002, "x", parent, &yes, &cfa);
        assert!(e.message().contains("«d27523»") && e.message().contains("acceso controlado a carpetas"), "{}", e.message());
    }

    /// Lo que no se explica (no se pudo leer el registro) se dice sin afirmar.
    #[test]
    fn op_message_when_not_found_but_there() {
        use xdir_core::ops::Op;
        let parent = PathBuf::from(r"C:\Documentos\d27523");
        let op = Op::NewFolder { parent: parent.clone() };
        let gone = CopyError::new(ErrKind::NotFound, &parent);
        let odd = op_message(&op, &gone, true);
        assert!(odd.starts_with("No se pudo crear la carpeta nueva en «d27523»: Windows dice que no encuentra «d27523», pero está."), "{odd}");
        assert!(odd.contains("acceso controlado a carpetas"), "{odd}");
        assert_eq!(op_message(&op, &gone, false), "No se pudo crear la carpeta nueva en «d27523»: ya no existe.");
        let blocked = CopyError::new(ErrKind::Blocked, &parent);
        assert!(op_message(&op, &blocked, true).starts_with("Windows bloqueó a xdir"));
    }

    #[test]
    fn hresults_map_to_kinds() {
        assert_eq!(hr_kind(0x8007_0005, ""), ErrKind::Denied);
        assert_eq!(hr_kind(0x8007_0020, ""), ErrKind::InUse);
        assert_eq!(hr_kind(0x8003_0020, ""), ErrKind::InUse);
        assert_eq!(hr_kind(0x8027_0024, ""), ErrKind::NotFound);
        assert_eq!(hr_kind(0x8027_0032, ""), ErrKind::DiskFull);
        assert_eq!(hr_kind(0x8027_0026, ""), ErrKind::Offline);
        assert_eq!(hr_kind(0x8027_003C, ""), ErrKind::NameTooLong);
        assert_eq!(hr_kind(0x8027_001E, ""), ErrKind::PathTooLong);
        assert_eq!(hr_kind(0x8027_0000, ""), ErrKind::Cancelled);
        assert_eq!(
            hr_kind(0x8004_0154, "Clase no registrada\r\n"),
            ErrKind::Other("Error inesperado del sistema (Clase no registrada; Windows: 0x80040154)".into())
        );
    }

    #[test]
    fn name_or_path_too_long() {
        let long_name = format!(r"C:\d\{}", "a".repeat(256));
        assert_eq!(from_hr(0x8007_00CE, "", Path::new(&long_name)).kind, ErrKind::NameTooLong);
        let deep = r"C:".to_string() + &r"\carpeta".repeat(40);
        assert_eq!(from_hr(0x8007_00CE, "", Path::new(&deep)).kind, ErrKind::PathTooLong);
    }
}

//! Versiones nuevas: al abrir, en un hilo aparte, se consulta la última release de GitHub y, si es
//! más nueva que esta, se avisa a la ventana con `WM_APP_UPDATE`. Sin conexión (o con cualquier
//! error) no pasa nada.
//!
//! Solo si el usuario lo pide, y solo en una instalación con instalador, se baja el instalador de
//! esa release a la carpeta temporal y se lo ejecuta en silencio (`WM_APP_UPDATE_READY` avisa que
//! terminó la descarga). La versión portable y las compilaciones propias solo abren la página.
//!
//! Las consultas y la descarga las hace `curl.exe`, que viene con Windows 10 y 11, para no sumar
//! una dependencia.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL, WM_APP};
use windows::core::{PCWSTR, w};

/// Hay una versión más nueva. `wparam` = la versión empaquetada (ver `pack`).
pub const WM_APP_UPDATE: u32 = WM_APP + 21;

/// Terminó la descarga del instalador. `wparam` = 1 si salió bien, 0 si no.
pub const WM_APP_UPDATE_READY: u32 = WM_APP + 22;

/// Página de la última release, donde se baja el instalador.
pub const RELEASES_URL: &str = "https://github.com/Runick93/xdir/releases/latest";
const API_URL: &str = "https://api.github.com/repos/Runick93/xdir/releases/latest";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

type Version = (u32, u32, u32);

/// "v0.1.10" o "0.1.10" → (0, 1, 10).
fn parse_version(s: &str) -> Option<Version> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let v = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(v)
}

/// El valor de "tag_name" en la respuesta JSON de la API.
fn tag_name(json: &str) -> Option<&str> {
    let rest = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(&rest[..rest.find('"')?])
}

/// Cabe en un `WPARAM`: 12 bits por componente.
fn pack((a, b, c): Version) -> usize {
    ((a as usize & 0xFFF) << 24) | ((b as usize & 0xFFF) << 12) | (c as usize & 0xFFF)
}

/// Versión empaquetada → "0.1.10".
pub fn unpack(v: usize) -> String {
    format!("{}.{}.{}", (v >> 24) & 0xFFF, (v >> 12) & 0xFFF, v & 0xFFF)
}

/// Busca en segundo plano; avisa a `hwnd` solo si hay algo más nuevo que esta versión.
pub fn check(hwnd: HWND) {
    let hwnd = hwnd.0 as usize;
    std::thread::spawn(move || {
        let Ok(out) = Command::new("curl.exe")
            .args(["-s", "-m", "10", "-H", "User-Agent: xdir", "-H", "Accept: application/vnd.github+json", API_URL])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        else {
            return;
        };
        let body = String::from_utf8_lossy(&out.stdout);
        let (Some(latest), Some(current)) = (tag_name(&body).and_then(parse_version), parse_version(crate::about::VERSION)) else {
            return;
        };
        if latest > current {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_UPDATE, WPARAM(pack(latest)), LPARAM(0));
            }
        }
    });
}

/// ¿Se puede actualizar sola? Solo si esta copia la dejó el instalador (hay un desinstalador al
/// lado del exe): una copia portable o una compilación propia no deben pisar la instalación.
pub fn can_self_update() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("unins000.exe").exists())).unwrap_or(false)
}

fn setup_path(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("xdir-{version}-setup.exe"))
}

/// Baja el instalador de `version` a la carpeta temporal y avisa a `hwnd` con `WM_APP_UPDATE_READY`.
pub fn download(hwnd: HWND, version: String) {
    let hwnd = hwnd.0 as usize;
    std::thread::spawn(move || {
        let path = setup_path(&version);
        let url = format!("https://github.com/Runick93/xdir/releases/download/v{version}/xdir-{version}-setup.exe");
        let _ = std::fs::remove_file(&path);
        let ok = Command::new("curl.exe")
            .args(["-s", "-f", "-L", "-m", "300", "-H", "User-Agent: xdir", "-o"])
            .arg(&path)
            .arg(&url)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|s| s.success())
            && looks_like_installer(&path);
        if !ok {
            let _ = std::fs::remove_file(&path);
        }
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_UPDATE_READY, WPARAM(usize::from(ok)), LPARAM(0));
        }
    });
}

/// Un ejecutable de Windows de tamaño razonable (no una página de error guardada como archivo).
fn looks_like_installer(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 2];
    f.read_exact(&mut head).is_ok() && &head == b"MZ" && f.metadata().is_ok_and(|m| m.len() > 500_000)
}

/// Ejecuta el instalador bajado, en silencio y con permiso de administrador (el instalador lo
/// pide; Windows muestra su aviso). `/RELAUNCH` hace que el instalador vuelva a abrir xdir al
/// terminar. Devuelve `false` si no se pudo lanzar o si el usuario no dio el permiso: entonces xdir
/// sigue abierto. Si devuelve `true`, quien llama tiene que cerrar la ventana enseguida para que
/// el instalador pueda reemplazar el exe.
pub fn launch_installer(hwnd: HWND, version: &str) -> bool {
    let setup = setup_path(version);
    if !setup.exists() {
        return false;
    }
    let file: Vec<u16> = setup.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            Some(hwnd),
            w!("runas"),
            PCWSTR(file.as_ptr()),
            w!("/SILENT /CLOSEAPPLICATIONS /NORESTART /RELAUNCH"),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Por encima de 32 es que se lanzó.
    result.0 as isize > 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.1.10"), Some((0, 1, 10)));
        assert!(parse_version("v0.1.10") > parse_version("0.1.9"));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn finds_the_tag_in_the_api_response() {
        let json = r#"{"url":"x","id":1,"tag_name": "v0.1.10","name":"xdir 0.1.10"}"#;
        assert_eq!(tag_name(json), Some("v0.1.10"));
        assert_eq!(tag_name(r#"{"message":"Not Found"}"#), None);
    }

    #[test]
    fn packing_round_trips() {
        assert_eq!(unpack(pack((0, 1, 10))), "0.1.10");
        assert_eq!(unpack(pack((12, 345, 6))), "12.345.6");
    }
}

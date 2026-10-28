//! Versiones nuevas: al abrir, en un hilo aparte, se consulta la última release de GitHub y, si es
//! más nueva que esta, se avisa a la ventana con `WM_APP_UPDATE`. Sin conexión (o con cualquier
//! error) no pasa nada.
//!
//! Si el usuario lo pide, se baja la release y se reemplaza esta copia (`WM_APP_UPDATE_READY` avisa
//! que terminó la descarga): en una instalación con instalador, se ejecuta el instalador en silencio;
//! en una copia portable, se baja el zip y se cambia el exe por el nuevo cuando esta se cierra.
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

/// Terminó una consulta. `wparam` = la última versión empaquetada (ver `pack`), o 0 si no se pudo
/// consultar. Quien recibe compara con `is_newer`.
pub const WM_APP_UPDATE: u32 = WM_APP + 21;

/// Qué pasó con la última consulta, para el diálogo "Acerca de".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Failed,
}

/// Terminó la descarga del instalador. `wparam` = 1 si salió bien, 0 si no.
pub const WM_APP_UPDATE_READY: u32 = WM_APP + 22;

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

fn unpack_version(v: usize) -> Version {
    (((v >> 24) & 0xFFF) as u32, ((v >> 12) & 0xFFF) as u32, (v & 0xFFF) as u32)
}

/// Versión empaquetada → "0.1.10".
pub fn unpack(v: usize) -> String {
    let (a, b, c) = unpack_version(v);
    format!("{a}.{b}.{c}")
}

/// ¿La versión empaquetada es más nueva que esta?
pub fn is_newer(v: usize) -> bool {
    parse_version(crate::about::VERSION).is_some_and(|current| unpack_version(v) > current)
}

/// Busca en segundo plano y avisa a `hwnd` con lo que encontró (0 si no pudo).
pub fn check(hwnd: HWND) {
    let hwnd = hwnd.0 as usize;
    std::thread::spawn(move || {
        let latest = Command::new("curl.exe")
            .args(["-s", "-m", "10", "-H", "User-Agent: xdir", "-H", "Accept: application/vnd.github+json", API_URL])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()
            .and_then(|out| tag_name(&String::from_utf8_lossy(&out.stdout)).and_then(parse_version));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_UPDATE, WPARAM(latest.map_or(0, pack)), LPARAM(0));
        }
    });
}

/// ¿Es una copia instalada con el instalador? (hay un desinstalador al lado del exe). Si no, es
/// portable o una compilación propia, y se actualiza reemplazando el exe.
fn installed() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("unins000.exe").exists())).unwrap_or(false)
}

fn setup_path(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("xdir-{version}-setup.exe"))
}

/// Carpeta donde se descomprime el zip portable de `version`.
fn portable_dir(version: &str) -> PathBuf {
    std::env::temp_dir().join(format!("xdir-{version}-portable"))
}

/// Baja `url` a `path` con curl.
fn fetch(url: &str, path: &std::path::Path) -> bool {
    Command::new("curl.exe")
        .args(["-s", "-f", "-L", "-m", "300", "-H", "User-Agent: xdir", "-o"])
        .arg(path)
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}

/// Baja la versión `version` a la carpeta temporal (el instalador, o el zip portable ya
/// descomprimido) y avisa a `hwnd` con `WM_APP_UPDATE_READY`.
pub fn download(hwnd: HWND, version: String) {
    let hwnd = hwnd.0 as usize;
    let installed = installed();
    std::thread::spawn(move || {
        let base = format!("https://github.com/Runick93/xdir/releases/download/v{version}");
        let ok = if installed {
            let path = setup_path(&version);
            let _ = std::fs::remove_file(&path);
            let ok = fetch(&format!("{base}/xdir-{version}-setup.exe"), &path) && looks_like_installer(&path);
            if !ok {
                let _ = std::fs::remove_file(&path);
            }
            ok
        } else {
            let zip = std::env::temp_dir().join(format!("xdir-{version}-portable.zip"));
            let dir = portable_dir(&version);
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_file(&zip);
            let ok = std::fs::create_dir_all(&dir).is_ok()
                && fetch(&format!("{base}/xdir-{version}-portable.zip"), &zip)
                && Command::new("tar.exe")
                    .arg("-xf")
                    .arg(&zip)
                    .arg("-C")
                    .arg(&dir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .status()
                    .is_ok_and(|s| s.success())
                && looks_like_installer(&dir.join("xdir.exe"));
            let _ = std::fs::remove_file(&zip);
            if !ok {
                let _ = std::fs::remove_dir_all(&dir);
            }
            ok
        };
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

/// Aplica lo bajado. Devuelve `false` si no se pudo (o si el usuario no dio el permiso): entonces
/// xdir sigue abierto. Si devuelve `true`, quien llama tiene que cerrar la ventana enseguida.
///
/// - Instalada: ejecuta el instalador en silencio y con permiso de administrador (Windows muestra
///   su aviso); `/RELAUNCH` hace que vuelva a abrir xdir al terminar.
/// - Portable o compilación propia: un PowerShell oculto espera a que este proceso termine, reemplaza
///   el exe por el nuevo y lo vuelve a abrir.
pub fn launch_installer(hwnd: HWND, version: &str) -> bool {
    if installed() {
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
        return result.0 as isize > 32;
    }
    let new = portable_dir(version).join("xdir.exe");
    let (Ok(exe), true) = (std::env::current_exe(), new.exists()) else { return false };
    let quote = |p: &std::path::Path| p.to_string_lossy().replace('\'', "\'\'");
    let script = format!(
        "Wait-Process -Id {} -ErrorAction SilentlyContinue; \
         for ($i = 0; $i -lt 30; $i++) {{ try {{ Copy-Item -LiteralPath '{}' -Destination '{}' -Force -ErrorAction Stop; break }} catch {{ Start-Sleep -Milliseconds 500 }} }}; \
         Start-Process -FilePath '{}'",
        std::process::id(),
        quote(&new),
        quote(&exe),
        quote(&exe),
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .is_ok()
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

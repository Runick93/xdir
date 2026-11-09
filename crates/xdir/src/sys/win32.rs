//! Servicios del sistema en Windows.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use windows::Win32::Foundation::{ERROR_SUCCESS, FILETIME, LPARAM, SYSTEMTIME, WPARAM};
use windows::Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::Shell::{SFBS_FLAGS_TRUNCATE_UNDISPLAYED_DECIMAL_DIGITS, ShellExecuteW, StrFormatByteSizeEx, StrFormatKBSizeW};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SPI_GETCLIENTAREAANIMATION, SW_SHOWNORMAL, SystemParametersInfoW};
use windows::core::{HSTRING, PCWSTR, w};
use xdir_core::keys::Key;
use std::sync::mpsc::Sender;
use xdir_core::ops::{Op, OpResult};
use xdir_core::sys::{ConsoleInfo, DiskScan, FileOps, Handle, Shell, Services, SystemEnv, Waker};
use xdir_core::xtree::scan::{DriveEntry, FastScan, Lister, Progress};
use xdir_core::xtree::tree::Node;
use xdir_core::fs::Location;
use xdir_core::shell::{Entry, Item};
use xdir_core::typed::MAX as TYPED_MAX;

use crate::fs::wide_path;
use crate::platform::HandleExt;
use crate::shell::win32 as sh;

/// Despierta a la ventana con `PostMessageW` (la cola de mensajes de Windows).
struct Win32Waker;

impl Waker for Win32Waker {
    fn post(&self, window: Handle, msg: u32, wparam: usize, lparam: isize) {
        unsafe {
            let _ = PostMessageW(Some(window.hwnd()), msg, WPARAM(wparam), LPARAM(lparam));
        }
    }
}

/// Historial de rutas escritas: el mismo del Explorador (`url1`, `url2`, ... de la más reciente a
/// la más antigua, hasta 25), así una ruta escrita en uno aparece en el otro.
const TYPED_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\TypedPaths");

/// Consolas: PowerShell (7 si está instalado) y CMD.
const POWERSHELL: u8 = 0;
const CMD: u8 = 1;

/// Entorno de Windows: registro, configuración regional del usuario y el Shell.
struct Win32Env;

impl SystemEnv for Win32Env {
    /// ¿Windows está en modo oscuro para las apps?
    fn system_dark(&self) -> bool {
        read_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme"))
            .map(|v| v == 0)
            .unwrap_or(false)
    }

    /// Color de acento de Windows (Configuración > Personalización > Colores), 0xRRGGBB.
    fn system_accent(&self) -> Option<u32> {
        // AccentColor se guarda como 0xAABBGGRR.
        read_dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor"))
            .map(|abgr| (abgr & 0xFF) << 16 | (abgr & 0xFF00) | (abgr >> 16) & 0xFF)
    }

    /// ¿Windows permite animaciones? (por defecto sí si la consulta falla).
    fn animations_allowed(&self) -> bool {
        let mut on = windows::core::BOOL(1);
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as *mut _), Default::default()) };
        ok.is_err() || on.as_bool()
    }

    fn key_down(&self, k: Key) -> bool {
        unsafe { GetKeyState(k.0 as i32) < 0 }
    }

    /// %APPDATA%\xdir
    fn config_dir(&self) -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("xdir"))
    }

    /// Fecha y hora local corta del usuario, p. ej. "24/10/2026 14:05".
    fn format_date(&self, ft: u64) -> String {
        if ft == 0 {
            return String::new();
        }
        let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        let mut date = [0u16; 64];
        let mut time = [0u16; 64];
        unsafe {
            if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
                return String::new();
            }
            // PCWSTR nulo = configuración regional del usuario.
            let d = GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&local), PCWSTR::null(), Some(&mut date), PCWSTR::null());
            let t = GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&local), PCWSTR::null(), Some(&mut time));
            format!("{} {}", wide_str(&date, d), wide_str(&time, t))
        }
    }

    /// Como lo muestra el Explorador: "1.234 KB".
    fn format_size_kb(&self, bytes: u64) -> String {
        let mut buf = [0u16; 64];
        unsafe {
            StrFormatKBSizeW(bytes as i64, &mut buf);
        }
        wide_str(&buf, -1)
    }

    /// "476 GB", "1,2 MB".
    fn format_size_short(&self, bytes: u64) -> String {
        let mut buf = [0u16; 64];
        unsafe {
            if StrFormatByteSizeEx(bytes, SFBS_FLAGS_TRUNCATE_UNDISPLAYED_DECIMAL_DIGITS, &mut buf).is_err() {
                return format!("{bytes} bytes");
            }
        }
        wide_str(&buf, -1)
    }

    fn typed_paths_load(&self) -> Vec<String> {
        (1..=TYPED_MAX)
            .map_while(|i| {
                let name: Vec<u16> = format!("url{i}\0").encode_utf16().collect();
                read_string(PCWSTR(name.as_ptr()))
            })
            .collect()
    }

    fn typed_paths_save(&self, list: &[String]) {
        unsafe {
            let mut key = HKEY::default();
            if RegCreateKeyExW(HKEY_CURRENT_USER, TYPED_KEY, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_READ | KEY_WRITE, None, &mut key, None)
                != ERROR_SUCCESS
            {
                return;
            }
            for i in 1..=TYPED_MAX {
                let name: Vec<u16> = format!("url{i}\0").encode_utf16().collect();
                match list.get(i - 1) {
                    Some(p) => {
                        let data: Vec<u8> = p.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
                        let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&data));
                    }
                    None => {
                        let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
                    }
                }
            }
            let _ = RegCloseKey(key);
        }
    }

    fn consoles(&self) -> Vec<ConsoleInfo> {
        vec![ConsoleInfo { id: POWERSHELL, label: "PowerShell" }, ConsoleInfo { id: CMD, label: "CMD" }]
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[("runas", "Ejecutar como administrador"), ("runasuser", "Ejecutar como otro usuario")]
    }

    /// Abre la consola con `dir` como carpeta de trabajo, con un verbo del Shell: `open`, `runas`
    /// (administrador, pide confirmación de UAC) o `runasuser` (pide usuario y contraseña de otra
    /// cuenta). Si la terminal predeterminada de Windows es Windows Terminal, se abre ahí.
    fn open_console(&self, owner: Handle, id: u8, dir: &Path, verb: &str) -> bool {
        let hwnd = owner.hwnd();
        let program = HSTRING::from(console_program(id).as_os_str());
        let verb = HSTRING::from(verb);
        let dir = wide_path(dir);
        let r = unsafe { ShellExecuteW(Some(hwnd), &verb, &program, PCWSTR::null(), PCWSTR(dir.as_ptr()), SW_SHOWNORMAL) };
        // ShellExecute devuelve un valor mayor que 32 si salió bien.
        r.0 as usize > 32
    }

    /// Una unidad ("C:", "C:\\") se titula como en el Explorador: "Disco local (C:)". Las demás
    /// raíces (`\\servidor\recurso`, "\\") muestran su texto.
    fn root_title(&self, path: &Path) -> Option<String> {
        let root = path.to_string_lossy();
        let root = root.trim_end_matches('\\');
        (root.len() == 2 && root.ends_with(':')).then(|| crate::fs::drive_label(root))
    }
}

/// Ejecutable de la consola: PowerShell 7 si está instalado, si no el de Windows.
fn console_program(id: u8) -> PathBuf {
    match id {
        CMD => PathBuf::from("cmd.exe"),
        _ => {
            let pwsh = std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join(r"PowerShell\7\pwsh.exe")).filter(|p| p.exists());
            pwsh.unwrap_or_else(|| PathBuf::from("powershell.exe"))
        }
    }
}

fn read_dword(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut _ as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn read_string(name: PCWSTR) -> Option<String> {
    let mut size = 0u32;
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, TYPED_KEY, name, RRF_RT_REG_SZ, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let status = RegGetValueW(HKEY_CURRENT_USER, TYPED_KEY, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size));
        (status == ERROR_SUCCESS).then(|| {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..len])
        })
    }
}

/// `len` incluye el nulo final (como devuelven las APIs); -1 = buscar el nulo.
fn wide_str(buf: &[u16], len: i32) -> String {
    let end = if len > 0 { (len as usize - 1).min(buf.len()) } else { buf.iter().position(|&c| c == 0).unwrap_or(buf.len()) };
    String::from_utf16_lossy(&buf[..end])
}

/// El entorno real de Windows (lo que instala `install`).
pub fn platform_env() -> Box<dyn SystemEnv> {
    Box::new(Win32Env)
}

struct Win32FileOps;
impl FileOps for Win32FileOps {
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op) {
        crate::ops::spawn_native(owner, tx, op);
    }
}

/// xtree: delega en las funciones de `fs` de Windows (MFT, ayudante elevado y unidades).
struct Win32DiskScan;

impl DiskScan for Win32DiskScan {
    fn lister(&self, root: &Path) -> Box<dyn Lister> {
        Box::new(crate::fs::xtree_lister(root))
    }

    fn fast_scan(&self, root: &Path, p: &Progress, cancel: &AtomicBool) -> FastScan {
        crate::fs::xtree_fast_scan(root, p, cancel)
    }

    fn elevate(&self, owner: Handle, root: &Path, p: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>> {
        crate::fs::xtree_elevate(owner, root, p, cancel)
    }

    fn drives(&self) -> Vec<DriveEntry> {
        crate::fs::xtree_drives()
    }

    fn volume_space(&self, path: &Path) -> Option<(u64, u64)> {
        crate::fs::volume_space(path)
    }

    fn whole_volume(&self, path: &Path) -> bool {
        crate::fs::whole_volume(path)
    }

    fn default_root(&self) -> PathBuf {
        crate::fs::xtree_default_root()
    }

    fn permission_issue(&self, p: &Progress) -> bool {
        crate::fs::xtree_permission_issue(p)
    }

    fn permission_hint(&self) -> &'static str {
        crate::fs::XTREE_PERMISSION_HINT
    }

    fn elevating_text(&self) -> &'static str {
        crate::fs::XTREE_ELEVATING_TEXT
    }
}

/// Shell: delega en `crate::shell::win32`; los errores pasan a texto con el mismo mensaje que
/// se mostraba antes (`message()` del HRESULT).
struct Win32Shell;

impl Shell for Win32Shell {
    fn copy_files(&self, items: &[Item], cut: bool) -> Result<(), String> {
        sh::copy_files(items, cut).map_err(|e| e.message())
    }

    fn clipboard_files(&self) -> Option<(Vec<PathBuf>, bool)> {
        sh::clipboard_files()
    }

    fn paste_virtual(&self, owner: Handle, dest: &Location) -> Result<bool, String> {
        sh::paste_virtual(owner, dest).map_err(|e| e.message())
    }

    fn clear_clipboard(&self) {
        sh::clear_clipboard()
    }

    fn copy_text(&self, owner: Handle, text: &str) -> Result<(), String> {
        sh::copy_text(owner, text).map_err(|e| e.message())
    }

    fn pick_folder(&self, owner: Handle, title: &str) -> Option<PathBuf> {
        sh::pick_folder(owner, title)
    }

    fn open_item(&self, owner: Handle, item: &Item) {
        sh::open_item(owner, item)
    }

    fn open_file(&self, owner: Handle, file: &Path, dir: Option<&Path>) {
        sh::open_file(owner, file, dir)
    }

    fn properties(&self, owner: Handle, items: &[Item]) {
        sh::properties(owner, items)
    }

    fn context_menu(
        &self,
        owner: Handle,
        folder: &Location,
        items: &[Item],
        at: (f32, f32),
        extended: bool,
        intercept: bool,
        own: &[Entry],
        pinned: bool,
    ) -> Option<String> {
        sh::context_menu(owner, folder, items, at, extended, intercept, own, pinned)
    }

    fn invoke_verb(&self, owner: Handle, folder: &Location, items: &[Item], verb: &str) -> Result<(), String> {
        sh::invoke_verb(owner, folder, items, verb).map_err(|e| e.message())
    }

    fn pin_quick_access(&self, owner: Handle, path: &Path) -> Result<(), String> {
        sh::pin_quick_access(owner, path).map_err(|e| e.message())
    }

    fn unpin_quick_access(&self, owner: Handle, path: &Path) -> Result<(), String> {
        sh::unpin_quick_access(owner, path).map_err(|e| e.message())
    }

    fn set_menu_mode(&self, dark: Option<bool>) {
        sh::set_menu_mode(dark)
    }

    fn volume_of(&self, path: &Path) -> Option<OsString> {
        sh::volume_of(path)
    }
}

/// Instala los servicios de Windows en el registro del núcleo. El ayudante elevado de xtree
/// (`--xtree-mft`) sale antes de llegar acá: no usa servicios.
pub fn install() {
    xdir_core::sys::install(Services {
        waker: Box::new(Win32Waker),
        env: platform_env(),
        file_ops: Box::new(Win32FileOps),
        disk_scan: Box::new(Win32DiskScan),
        shell: Box::new(Win32Shell),
    });
}

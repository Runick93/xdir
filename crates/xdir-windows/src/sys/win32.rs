//! Servicios del sistema en Windows.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64};

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
use xdir_core::sys::{ConsoleInfo, DiskScan, FileOps, FileSystem, Handle, LocalNow, Metadata, Overlays, Preview, Services, Shell, SystemEnv, Watch, Waker};
use xdir_core::xtree::scan::{DriveEntry, FastScan, Lister, Progress};
use xdir_core::xtree::tree::Node;
use xdir_core::fs::{Entry as FsEntry, LoadEvent, Location, MetadataRequest, OverlayRequest, QuickItem};
use xdir_core::preview::PreviewRequest;
use xdir_core::shell::{Entry, Item};
use xdir_core::typed::MAX as TYPED_MAX;

use crate::fs::win32 as fsp;
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

/// Consolas: PowerShell (7 si está instalado), CMD y Windows Terminal (si está instalada).
const POWERSHELL: u8 = 0;
const CMD: u8 = 1;
const WINDOWS_TERMINAL: u8 = 2;

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

    /// Hoy en hora local: la hora UTC llevada a la zona horaria del sistema
    /// (`SystemTimeToTzSpecificLocalTime`); la diferencia entre las dos es la de la zona.
    fn local_now(&self) -> LocalNow {
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let ft = ((unix + 11_644_473_600) as u64) * 10_000_000;
        let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        let ok = unsafe { FileTimeToSystemTime(&ft, &mut utc).is_ok() && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok() };
        let offset = if ok { system_time_secs(&local) - system_time_secs(&utc) } else { 0 };
        LocalNow::from_unix(unix, offset)
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
        detected_consoles().0.clone()
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[("runas", "Ejecutar como administrador"), ("runasuser", "Ejecutar como otro usuario")]
    }

    /// Abre la consola con `dir` como carpeta de trabajo, con un verbo del Shell: `open`, `runas`
    /// (administrador, pide confirmación de UAC) o `runasuser` (pide usuario y contraseña de otra
    /// cuenta). Si la terminal predeterminada de Windows es Windows Terminal, PowerShell y CMD se
    /// abren ahí. Windows Terminal se abre con `wt.exe -d "<carpeta>"` y los mismos verbos (con
    /// `runas`, la carpeta va en `-d` porque un proceso elevado no hereda la de trabajo).
    fn open_console(&self, owner: Handle, id: u8, dir: &Path, verb: &str) -> bool {
        let hwnd = owner.hwnd();
        let (program, params) = if id == WINDOWS_TERMINAL {
            let wt = detected_consoles().1.clone().unwrap_or_else(|| PathBuf::from("wt.exe"));
            (wt, Some(HSTRING::from(wt_params(dir))))
        } else {
            (console_program(id), None)
        };
        let program = HSTRING::from(program.as_os_str());
        let params = params.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
        let verb = HSTRING::from(verb);
        let dir = wide_path(dir);
        let r = unsafe { ShellExecuteW(Some(hwnd), &verb, &program, params, PCWSTR(dir.as_ptr()), SW_SHOWNORMAL) };
        // ShellExecute devuelve un valor mayor que 32 si salió bien.
        r.0 as usize > 32
    }
}

/// Las consolas del sistema y la ruta de `wt.exe`, buscadas una sola vez por proceso: el menú de
/// consolas las pide en cada pintado y la búsqueda recorre el disco.
fn detected_consoles() -> &'static (Vec<ConsoleInfo>, Option<PathBuf>) {
    static DETECTED: OnceLock<(Vec<ConsoleInfo>, Option<PathBuf>)> = OnceLock::new();
    DETECTED.get_or_init(|| {
        let (path, local) = (std::env::var_os("PATH"), std::env::var_os("LOCALAPPDATA").map(PathBuf::from));
        let wt = find_wt(path.as_deref(), local.as_deref(), &|p| p.exists());
        (windows_consoles(path.as_deref(), local.as_deref(), &|p| wt.as_deref() == Some(p)), wt)
    })
}

/// Consolas en el orden del menú: PowerShell, CMD y, si se encuentra `wt.exe`, Windows Terminal.
/// Recibe el PATH, `%LOCALAPPDATA%` y `exists` (¿existe el archivo?) en vez de mirar el sistema,
/// así se prueba sin depender de lo instalado.
pub(super) fn windows_consoles(path: Option<&OsStr>, local_app_data: Option<&Path>, exists: &dyn Fn(&Path) -> bool) -> Vec<ConsoleInfo> {
    let mut list = vec![ConsoleInfo { id: POWERSHELL, label: "PowerShell" }, ConsoleInfo { id: CMD, label: "CMD" }];
    if find_wt(path, local_app_data, exists).is_some() {
        list.push(ConsoleInfo { id: WINDOWS_TERMINAL, label: "Windows Terminal" });
    }
    list
}

/// `wt.exe`: el primero de las carpetas del PATH, si no el alias de la Tienda en
/// `%LOCALAPPDATA%\Microsoft\WindowsApps`.
pub(super) fn find_wt(path: Option<&OsStr>, local_app_data: Option<&Path>, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let in_path = path.into_iter().flat_map(std::env::split_paths).filter(|d| !d.as_os_str().is_empty()).map(|d| d.join("wt.exe"));
    let alias = local_app_data.map(|l| l.join(r"Microsoft\WindowsApps\wt.exe"));
    in_path.chain(alias).find(|p| exists(p))
}

/// Parámetros de `wt.exe` para abrir en `dir`: `-d "<dir>"`. Las barras al final se duplican
/// (`C:\` → `"C:\\"`) para que no escapen la comilla de cierre al separar los argumentos, y
/// los `;` se escriben `\;` porque para wt separan comandos.
pub(super) fn wt_params(dir: &Path) -> String {
    let d = dir.to_string_lossy().replace(';', "\\;");
    let trailing = d.len() - d.trim_end_matches('\\').len();
    format!("-d \"{d}{}\"", "\\".repeat(trailing))
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
        Box::new(fsp::xtree_lister(root))
    }

    fn fast_scan(&self, root: &Path, p: &Progress, cancel: &AtomicBool) -> FastScan {
        fsp::xtree_fast_scan(root, p, cancel)
    }

    fn elevate(&self, owner: Handle, root: &Path, p: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>> {
        fsp::xtree_elevate(owner, root, p, cancel)
    }

    fn drives(&self) -> Vec<DriveEntry> {
        fsp::xtree_drives()
    }

    fn volume_space(&self, path: &Path) -> Option<(u64, u64)> {
        fsp::volume_space(path)
    }

    fn whole_volume(&self, path: &Path) -> bool {
        fsp::whole_volume(path)
    }

    fn default_root(&self) -> PathBuf {
        fsp::xtree_default_root()
    }

    fn permission_issue(&self, p: &Progress) -> bool {
        fsp::xtree_permission_issue(p)
    }

    fn permission_hint(&self) -> &'static str {
        fsp::XTREE_PERMISSION_HINT
    }

    fn elevating_text(&self) -> &'static str {
        fsp::XTREE_ELEVATING_TEXT
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

/// Sistema de archivos: delega en `crate::fs::win32`. No guarda nada: COM se inicializa en el
/// hilo de cada lectura o búsqueda, y las cachés (nombres de unidad, tipos) son estáticas.
struct Win32FileSystem;

impl FileSystem for Win32FileSystem {
    fn spawn_load(&self, hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
        fsp::spawn_load(hwnd, tx, panel, generation, current, location)
    }

    fn with_search_thread(&self, f: Box<dyn FnOnce() + Send>) {
        fsp::with_search_thread(f)
    }

    fn search_metadata(&self, item: &std::fs::DirEntry, path: &Path) -> Option<std::fs::Metadata> {
        fsp::search_metadata(item, path)
    }

    fn is_hidden(&self, meta: &std::fs::Metadata, name: &OsStr) -> bool {
        fsp::is_hidden(meta, name)
    }

    fn searchable_dir(&self, name: &OsStr) -> bool {
        fsp::searchable_dir(name)
    }

    fn is_mount_point(&self, path: &Path) -> bool {
        fsp::is_mount_point(path)
    }

    fn disk_entry(&self, path: &Path, name: OsString, meta: &std::fs::Metadata) -> FsEntry {
        fsp::disk_entry(path, name, meta)
    }

    fn user_folder(&self, i: usize) -> Option<PathBuf> {
        fsp::user_folder(i)
    }

    fn drives(&self) -> Vec<PathBuf> {
        fsp::drives()
    }

    fn drive_name(&self, root: &Path) -> String {
        fsp::drive_name(root)
    }

    fn drive_label(&self, drive: &str) -> String {
        fsp::drive_label(drive)
    }

    fn forget_drive_labels(&self) {
        fsp::forget_drive_labels()
    }

    fn drive_missing(&self, path: &Path) -> bool {
        fsp::drive_missing(path)
    }

    /// Una unidad ("C:", "C:\\") se titula como en el Explorador: "Disco local (C:)". Las demás
    /// raíces (`\\servidor\recurso`, "\\") muestran su texto.
    fn root_title(&self, path: &Path) -> Option<String> {
        let root = path.to_string_lossy();
        let root = root.trim_end_matches('\\');
        (root.len() == 2 && root.ends_with(':')).then(|| fsp::drive_label(root))
    }

    fn subfolders(&self, loc: &Location) -> Vec<(String, Location)> {
        fsp::subfolders(loc)
    }

    fn folder_icon(&self) -> Option<i32> {
        fsp::folder_icon()
    }

    fn folder_type(&self) -> Arc<str> {
        fsp::folder_type()
    }

    fn shell_location(&self, text: &str) -> Option<Location> {
        fsp::shell_location(text)
    }

    fn network_place(&self) -> &'static str {
        fsp::NETWORK_PLACE
    }

    fn linux_place(&self) -> &'static str {
        fsp::LINUX_PLACE
    }

    fn shell_place_exists(&self, parsing: &str) -> bool {
        fsp::shell_place_exists(parsing)
    }

    fn shell_subfolders(&self, parsing: &str) -> Vec<(String, PathBuf)> {
        fsp::shell_subfolders(parsing)
    }

    fn quick_access(&self) -> Vec<QuickItem> {
        fsp::quick_access()
    }

    fn is_quick_pinned(&self, path: &Path) -> bool {
        fsp::is_quick_pinned(path)
    }

    fn quick_access_watch_dir(&self) -> Option<PathBuf> {
        fsp::quick_access_watch_dir()
    }

    fn long_path(&self, path: &Path) -> PathBuf {
        fsp::long_path(path)
    }

    fn true_case(&self, path: &Path) -> PathBuf {
        fsp::true_case(path)
    }

    fn connect_share(&self, owner: Handle, path: &Path) -> bool {
        fsp::connect_share(owner, path)
    }

    fn same_path(&self, a: &Path, b: &Path) -> bool {
        fsp::same_path(a, b)
    }

    fn watch(&self, hwnd: Handle, msg: u32, panel: usize, lparam: isize, path: &Path) -> Option<Box<dyn Watch>> {
        fsp::Watcher::with_message(hwnd, msg, panel, lparam, path).map(|w| Box::new(w) as Box<dyn Watch>)
    }

    /// Atributos sin conexión o «recuperar al abrir/leer» (ver `crate::metadata::cloud_only`).
    fn cloud_only(&self, path: &Path) -> bool {
        crate::metadata::cloud_only(&HSTRING::from(path))
    }

    fn name_rules(&self) -> xdir_core::rename_many::NameRules {
        xdir_core::rename_many::NameRules::WINDOWS
    }

    /// NTFS, FAT y exFAT no distinguen mayúsculas (salvo carpetas marcadas para WSL: se marcan
    /// choques de más, nunca de menos).
    fn case_insensitive(&self, _dir: &Path) -> bool {
        true
    }

    /// `MoveFileExW` sin `MOVEFILE_REPLACE_EXISTING`: falla si el destino existe.
    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), String> {
        use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
        let (f, t) = (wide_path(from), wide_path(to));
        match unsafe { MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVE_FILE_FLAGS(0)) } {
            Ok(()) => Ok(()),
            Err(e) => {
                let io = std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF);
                let at = if io.kind() == std::io::ErrorKind::AlreadyExists { to } else { from };
                Err(xdir_core::rename_many::io_message(&io, at))
            }
        }
    }
}

/// El sistema de archivos real de Windows (lo que instala `install`).
#[cfg(test)]
pub fn platform_file_system() -> Box<dyn FileSystem> {
    Box::new(Win32FileSystem)
}

/// Superposiciones de íconos: las resuelve el hilo propio de `crate::icons` (con COM).
struct Win32Overlays;

impl Overlays for Win32Overlays {
    fn request(&self, req: OverlayRequest) {
        crate::icons::queue_overlays(req)
    }
}

/// Metadatos de archivos: los lee el hilo propio de `crate::metadata` (con COM), de
/// `IPropertyStore`.
struct Win32Metadata;

impl Metadata for Win32Metadata {
    fn request(&self, req: MetadataRequest) {
        crate::metadata::queue(req)
    }
}

/// Vista previa de la columna: un hilo por pedido (una lectura lenta, de la red, no traba a la
/// siguiente) con la lectura neutra del núcleo y el chequeo de nube (atributos `OFFLINE` y
/// `RECALL_*`).
struct Win32Preview;

impl Preview for Win32Preview {
    fn request(&self, req: PreviewRequest) {
        let spawned = std::thread::Builder::new()
            .name("xdir-vista-previa".into())
            .spawn(move || xdir_core::preview::answer(req, &|p| crate::metadata::cloud_only(&HSTRING::from(p))));
        debug_assert!(spawned.is_ok(), "hilo de vista previa");
    }
}

/// La vista previa real de Windows (lo que instala `install`).
#[cfg(test)]
pub fn platform_preview() -> Box<dyn Preview> {
    Box::new(Win32Preview)
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
        file_system: Box::new(Win32FileSystem),
        overlays: Box::new(Win32Overlays),
        metadata: Box::new(Win32Metadata),
        preview: Box::new(Win32Preview),
        copy_prims: Box::new(crate::ops::WinCopyPrims),
        info: Box::new(crate::info::Win32Info),
    });
}

/// Segundos desde 1970 de un SYSTEMTIME (sin zona: el que se le pasa). Días por el algoritmo de
/// Howard Hinnant.
fn system_time_secs(t: &SYSTEMTIME) -> i64 {
    let (y, m, d) = (i64::from(t.wYear), i64::from(t.wMonth), i64::from(t.wDay));
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + d - 1;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + i64::from(t.wHour) * 3600 + i64::from(t.wMinute) * 60 + i64::from(t.wSecond)
}

//! Servicios del sistema en macOS.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSColorSpace, NSEvent, NSEventModifierFlags, NSWorkspace};
use objc2_foundation::{
    NSByteCountFormatter, NSByteCountFormatterCountStyle, NSDate, NSDateFormatter, NSDateFormatterStyle, NSString, NSUserDefaults,
};
use xdir_core::commands::Command;
use xdir_core::keys::Key;
use std::sync::mpsc::Sender;
use xdir_core::ops::{Op, OpResult};
use xdir_core::sys::{ConsoleInfo, DiskScan, FileOps, FileSystem, Handle, KeyStyle, Overlays, Services, Shell, SystemEnv, Watch, Waker};
use xdir_core::xtree::scan::{DriveEntry, FastScan, Lister, Progress};
use xdir_core::xtree::tree::Node;
use xdir_core::fs::{Entry as FsEntry, LoadEvent, Location, OverlayRequest, QuickItem};
use xdir_core::shell::{Entry, Item};

use crate::fs::macos as fsp;
use crate::shell::macos as sh;

/// Quién atiende los mensajes de la aplicación en el hilo principal (lo fija la ventana al
/// arrancar).
static HANDLER: OnceLock<fn(u32, usize, isize)> = OnceLock::new();

/// Registra la función que recibe en el hilo principal los mensajes de `Handle::post`.
pub fn set_message_handler(f: fn(u32, usize, isize)) {
    let _ = HANDLER.set(f);
}

/// Despierta al hilo principal encolando el mensaje en la cola principal de GCD. Hay una sola
/// ventana, así que el `Handle` no se usa.
struct MacWaker;

impl Waker for MacWaker {
    fn post(&self, _window: Handle, msg: u32, wparam: usize, lparam: isize) {
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(f) = HANDLER.get() {
                f(msg, wparam, lparam);
            }
        });
    }
}

/// Segundos entre 1601 (época de FILETIME, que usan las entradas) y 1970.
const EPOCH_DIFF: f64 = 11_644_473_600.0;

thread_local! {
    static DATES: Retained<NSDateFormatter> = {
        let f = NSDateFormatter::new();
        f.setDateStyle(NSDateFormatterStyle::ShortStyle);
        f.setTimeStyle(NSDateFormatterStyle::ShortStyle);
        f
    };
    static SIZES: Retained<NSByteCountFormatter> = {
        let f = NSByteCountFormatter::new();
        f.setCountStyle(NSByteCountFormatterCountStyle::File);
        f
    };
}

/// El historial de rutas escritas va en `rutas.txt` de la carpeta de configuración, una por línea.
fn typed_file() -> Option<PathBuf> {
    xdir_core::session::config_dir().map(|d| d.join("rutas.txt"))
}

/// macOS: atajos de fábrica distintos a los de Windows, con las convenciones del Finder (Ctrl es
/// ⌘ y Alt es ⌥). ⌘H oculta la app, así que los ocultos van con ⌘⇧. como en el Finder.
const KEYS: &[(Command, &[&str])] = &[
    (Command::Back, &["Ctrl+[", "Ctrl+Izquierda", "NavegadorAtras"]),
    (Command::Forward, &["Ctrl+]", "Ctrl+Derecha", "NavegadorAdelante"]),
    (Command::Up, &["Ctrl+Arriba", "Retroceso"]),
    (Command::Refresh, &["F5", "Ctrl+R"]),
    (Command::EditAddress, &["Ctrl+L", "Ctrl+Shift+G", "F4"]),
    (Command::ToggleHidden, &["Ctrl+Shift+."]),
    (Command::InvertSelection, &["Ctrl+Alt+I"]),
    // ⌘+ / ⌘- (también los del teclado numérico).
    (Command::SelectPattern, &["Ctrl+Más", "Ctrl+MásNum"]),
    (Command::DeselectPattern, &["Ctrl+Menos", "Ctrl+MenosNum"]),
    (Command::Properties, &["Ctrl+I"]),
    (Command::Recycle, &["Ctrl+Retroceso", "Supr"]),
    (Command::Delete, &["Ctrl+Alt+Retroceso", "Shift+Supr"]),
    (Command::CopyPaths, &["Ctrl+Alt+C"]),
    // ⌘⌥N: archivo de texto nuevo.
    (Command::NewTextFile, &["Ctrl+Alt+N"]),
];

/// macOS: nombres distintos a los de Windows.
const NAMES: &[(Command, &str)] = &[
    (Command::PowerShell, "Abrir Terminal en la carpeta"),
    (Command::Recycle, "Trasladar a la Papelera"),
    (Command::Properties, "Obtener información"),
];

/// Entorno de macOS: apariencia del sistema, formatos del usuario y Terminal.
struct MacEnv;

impl SystemEnv for MacEnv {
    /// ¿El sistema está en modo oscuro?
    fn system_dark(&self) -> bool {
        let defaults = NSUserDefaults::standardUserDefaults();
        defaults.stringForKey(&NSString::from_str("AppleInterfaceStyle")).is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark"))
    }

    /// Color de acento del sistema (Configuración del Sistema > Apariencia), 0xRRGGBB.
    fn system_accent(&self) -> Option<u32> {
        let color = NSColor::controlAccentColor();
        let rgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        Some(ch(rgb.redComponent()) << 16 | ch(rgb.greenComponent()) << 8 | ch(rgb.blueComponent()))
    }

    /// ¿El sistema permite animaciones? ("Reducir movimiento" en Accesibilidad las apaga.)
    fn animations_allowed(&self) -> bool {
        !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    /// En macOS, Ctrl de los atajos es ⌘ (como Ctrl+C → ⌘C) y Alt es ⌥.
    fn key_down(&self, k: Key) -> bool {
        let flags = NSEvent::modifierFlags_class();
        match k.0 {
            0x10 => flags.contains(NSEventModifierFlags::Shift),
            0x11 => flags.contains(NSEventModifierFlags::Command),
            0x12 => flags.contains(NSEventModifierFlags::Option),
            _ => false,
        }
    }

    /// ~/Library/Application Support/xdir
    fn config_dir(&self) -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/xdir"))
    }

    /// Fecha y hora con el formato corto del usuario, p. ej. "24/10/26 14:05".
    fn format_date(&self, ft: u64) -> String {
        if ft == 0 {
            return String::new();
        }
        let secs = ft as f64 / 10_000_000.0 - EPOCH_DIFF;
        let d = NSDate::dateWithTimeIntervalSince1970(secs);
        DATES.with(|f| f.stringFromDate(&d).to_string())
    }

    /// Como lo muestra el Finder: "1,2 MB".
    fn format_size_kb(&self, bytes: u64) -> String {
        self.format_size_short(bytes)
    }

    fn format_size_short(&self, bytes: u64) -> String {
        SIZES.with(|f| f.stringFromByteCount(bytes as i64).to_string())
    }

    fn typed_paths_load(&self) -> Vec<String> {
        let Some(text) = typed_file().and_then(|f| std::fs::read_to_string(f).ok()) else { return Vec::new() };
        text.lines().map(str::trim).filter(|l| !l.is_empty()).take(xdir_core::typed::MAX).map(str::to_string).collect()
    }

    fn typed_paths_save(&self, list: &[String]) {
        let Some(file) = typed_file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, list.join("\n") + "\n");
    }

    /// Terminal, y iTerm si está instalada.
    fn consoles(&self) -> Vec<ConsoleInfo> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        mac_consoles(home.as_deref(), &|p| p.exists())
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    fn key_style(&self) -> KeyStyle {
        KeyStyle::Mac
    }

    fn command_keys(&self, c: Command) -> Option<&'static [&'static str]> {
        KEYS.iter().find(|(k, _)| *k == c).map(|(_, keys)| *keys)
    }

    fn command_title(&self, c: Command) -> Option<&'static str> {
        NAMES.iter().find(|(k, _)| *k == c).map(|(_, name)| *name)
    }

    /// En macOS no hay CMD (el comando abre la consola principal, Terminal) y no existe el Acceso
    /// rápido de Windows.
    fn command_available(&self, c: Command) -> bool {
        !matches!(c, Command::Cmd | Command::ToggleQuickPin)
    }

    /// Abre Terminal o iTerm en la carpeta con `open -a` (el verbo no aplica).
    fn open_console(&self, _owner: Handle, id: u8, dir: &Path, _verb: &str) -> bool {
        let app = if id == ITERM { "iTerm" } else { "Terminal" };
        std::process::Command::new("open").args(["-a", app]).arg(dir).status().is_ok_and(|s| s.success())
    }
}

/// Consolas de macOS: Terminal siempre; iTerm si existe.
const TERMINAL: u8 = 0;
const ITERM: u8 = 1;

/// Consolas en el orden del menú: Terminal, y después iTerm si está en `/Applications` o en
/// `~/Applications`. `exists` dice si una ruta existe (el sistema de archivos real, o uno
/// simulado en las pruebas).
pub(super) fn mac_consoles(home: Option<&Path>, exists: &dyn Fn(&Path) -> bool) -> Vec<ConsoleInfo> {
    let mut list = vec![ConsoleInfo { id: TERMINAL, label: "Terminal" }];
    let system = PathBuf::from("/Applications/iTerm.app");
    let user = home.map(|h| h.join("Applications/iTerm.app"));
    if exists(&system) || user.is_some_and(|u| exists(&u)) {
        list.push(ConsoleInfo { id: ITERM, label: "iTerm" });
    }
    list
}

/// El entorno real de macOS (lo que instala `install`).
pub fn platform_env() -> Box<dyn SystemEnv> {
    Box::new(MacEnv)
}

struct MacFileOps;
impl FileOps for MacFileOps {
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op) {
        crate::ops::spawn_native(owner, tx, op);
    }
}

/// xtree: delega en las funciones de `fs` de macOS.
struct MacDiskScan;

impl DiskScan for MacDiskScan {
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

/// Shell: delega en `crate::shell::macos`; los errores pasan a texto con el mismo mensaje que
/// se mostraba antes (`ShellError::message`).
struct MacShell;

impl Shell for MacShell {
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

/// Sistema de archivos: delega en `crate::fs::macos`. No guarda nada: las cachés de AppKit son
/// por hilo y se crean en el hilo que las usa (el de cada lectura o búsqueda).
struct MacFileSystem;

impl FileSystem for MacFileSystem {
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

    /// Cualquier raíz es un disco: su nombre ("/" → "Macintosh HD").
    fn root_title(&self, path: &Path) -> Option<String> {
        Some(fsp::drive_label(&path.to_string_lossy()))
    }

    fn subfolders(&self, loc: &Location) -> Vec<(String, Location)> {
        fsp::subfolders(loc)
    }

    fn folder_icon(&self) -> Option<i32> {
        fsp::folder_icon()
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
}

/// El sistema de archivos real de macOS (lo que instala `install`).
#[cfg(test)]
pub fn platform_file_system() -> Box<dyn FileSystem> {
    Box::new(MacFileSystem)
}

/// Superposiciones de íconos: en macOS no hay (las etiquetas de color del Finder van aparte),
/// así que el pedido no se responde.
struct MacOverlays;

impl Overlays for MacOverlays {
    fn request(&self, _req: OverlayRequest) {}
}

/// Instala los servicios de macOS en el registro del núcleo.
pub fn install() {
    xdir_core::sys::install(Services {
        waker: Box::new(MacWaker),
        env: platform_env(),
        file_ops: Box::new(MacFileOps),
        disk_scan: Box::new(MacDiskScan),
        shell: Box::new(MacShell),
        file_system: Box::new(MacFileSystem),
        overlays: Box::new(MacOverlays),
    });
}

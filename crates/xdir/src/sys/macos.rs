//! Servicios del sistema en macOS.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSColorSpace, NSEvent, NSEventModifierFlags, NSWorkspace};
use objc2_foundation::{
    NSByteCountFormatter, NSByteCountFormatterCountStyle, NSDate, NSDateFormatter, NSDateFormatterStyle, NSString, NSUserDefaults,
};
use xdir_core::keys::Key;
use xdir_core::sys::{ConsoleInfo, FileOps, Handle, Services, SystemEnv, Waker};

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

    /// Una sola: Terminal.
    fn consoles(&self) -> Vec<ConsoleInfo> {
        vec![ConsoleInfo { id: 0, label: "Terminal" }]
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// Abre Terminal en la carpeta (el verbo no aplica).
    fn open_console(&self, _owner: Handle, _id: u8, dir: &Path, _verb: &str) -> bool {
        std::process::Command::new("open").args(["-a", "Terminal"]).arg(dir).status().is_ok_and(|s| s.success())
    }
}

/// El entorno real de macOS (lo que instala `install`).
pub fn platform_env() -> Box<dyn SystemEnv> {
    Box::new(MacEnv)
}

struct MacFileOps;
impl FileOps for MacFileOps {}

/// Instala los servicios de macOS en el registro del núcleo.
pub fn install() {
    xdir_core::sys::install(Services { waker: Box::new(MacWaker), env: platform_env(), file_ops: Box::new(MacFileOps) });
}

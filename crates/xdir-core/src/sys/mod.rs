//! Registro de servicios del sistema: lo que el núcleo necesita del sistema operativo, detrás de
//! traits. Cada plataforma instala sus implementaciones una sola vez al arrancar (`install`), antes
//! de crear la `App`, y el resto del código las pide con `waker()`, `env()` y `file_ops()`. Los
//! tests instalan implementaciones falsas con `testing::install_fakes()`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use crate::commands::Command;
use crate::keys::Key;
use crate::ops::{Op, OpResult};

pub mod testing;

#[cfg(test)]
mod tests;

/// La ventana principal, vista desde cualquier hilo: sirve para mandarle mensajes de la
/// aplicación (`WM_APP + n`) que se procesan en el hilo de interfaz.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Handle(pub isize);

impl Handle {
    /// Encola un mensaje para el hilo de interfaz y vuelve enseguida.
    pub fn post(self, msg: u32, wparam: usize, lparam: isize) {
        waker().post(self, msg, wparam, lparam);
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Despierta al hilo de interfaz: encola un mensaje para la ventana y vuelve enseguida (no
/// bloquea). Se llama desde cualquier hilo.
pub trait Waker: Send + Sync {
    fn post(&self, window: Handle, msg: u32, wparam: usize, lparam: isize);
}

/// Una consola que ofrece el sistema (PowerShell, CMD, Terminal…). `id` lo entiende solo la
/// plataforma que la listó.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConsoleInfo {
    pub id: u8,
    pub label: &'static str,
}

/// Cómo se escriben los atajos en pantalla.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyStyle {
    /// "Ctrl+Shift+T".
    Windows,
    /// Símbolos de Apple: "⇧⌘T".
    Mac,
}

/// Datos y preferencias del entorno del sistema: tema, animaciones, teclado, carpeta de
/// configuración, formatos regionales, historial de rutas escritas y consolas.
pub trait SystemEnv: Send + Sync {
    /// ¿El sistema está en modo oscuro para las apps?
    fn system_dark(&self) -> bool;
    /// Color de acento del sistema, 0xRRGGBB.
    fn system_accent(&self) -> Option<u32>;
    /// ¿El sistema permite animaciones?
    fn animations_allowed(&self) -> bool;
    /// ¿La tecla está apretada ahora? (Ctrl, Shift, Alt…)
    fn key_down(&self, k: Key) -> bool;
    /// Carpeta de configuración del usuario (`%APPDATA%\xdir`, `~/Library/Application
    /// Support/xdir`). El modo portable lo resuelve el núcleo antes (`session::config_dir`).
    fn config_dir(&self) -> Option<PathBuf>;
    /// Fecha y hora local corta del usuario a partir de un FILETIME. Vacío si es 0.
    fn format_date(&self, filetime: u64) -> String;
    /// Tamaño de archivo como lo muestra el explorador del sistema.
    fn format_size_kb(&self, bytes: u64) -> String;
    /// Tamaño abreviado: "476 GB", "1,2 MB".
    fn format_size_short(&self, bytes: u64) -> String;
    /// Rutas escritas en la barra de dirección, de la más reciente a la más antigua.
    fn typed_paths_load(&self) -> Vec<String>;
    fn typed_paths_save(&self, list: &[String]);
    /// Consolas que se pueden abrir, en el orden del menú.
    fn consoles(&self) -> Vec<ConsoleInfo>;
    /// Otras formas de abrir una consola (clic derecho en el menú): (verbo, texto del menú).
    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)];
    /// Abre la consola `id` con `dir` como carpeta de trabajo. `verb`: "open" o uno de
    /// `console_alt_verbs`.
    fn open_console(&self, owner: Handle, id: u8, dir: &Path, verb: &str) -> bool;
    /// Cómo se muestran los atajos. Por defecto, al estilo de Windows.
    fn key_style(&self) -> KeyStyle {
        KeyStyle::Windows
    }
    /// Atajos de fábrica propios del sistema para un comando (reemplazan a los comunes).
    fn command_keys(&self, _c: Command) -> Option<&'static [&'static str]> {
        None
    }
    /// Nombre propio del sistema para un comando en la paleta (reemplaza al común).
    fn command_title(&self, _c: Command) -> Option<&'static str> {
        None
    }
    /// ¿El comando existe en este sistema?
    fn command_available(&self, _c: Command) -> bool {
        true
    }
}

/// Operaciones de archivos del sistema.
pub trait FileOps: Send + Sync {
    /// Corre `op` en su propio hilo; al terminar manda el resultado por `tx` y avisa a `owner`
    /// con `WM_APP_OP_DONE`. Vuelve enseguida.
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op);
}

/// Las implementaciones de una plataforma, instaladas juntas con `install`.
pub struct Services {
    pub waker: Box<dyn Waker>,
    pub env: Box<dyn SystemEnv>,
    pub file_ops: Box<dyn FileOps>,
}

static SERVICES: OnceLock<Services> = OnceLock::new();

/// Instala los servicios del proceso. Va una sola vez, al arrancar; una segunda llamada se
/// ignora (y en debug avisa con `debug_assert!`).
pub fn install(s: Services) {
    let fresh = SERVICES.set(s).is_ok();
    debug_assert!(fresh, "servicios del sistema instalados dos veces");
}

/// Como `install`, pero sin quejarse si ya había servicios (para los falsos de los tests).
fn install_once(make: impl FnOnce() -> Services) {
    SERVICES.get_or_init(make);
}

fn get_or_panic(cell: &OnceLock<Services>) -> &Services {
    cell.get().unwrap_or_else(|| panic!("servicios del sistema no instalados"))
}

fn services() -> &'static Services {
    get_or_panic(&SERVICES)
}

pub fn waker() -> &'static dyn Waker {
    &*services().waker
}

pub fn env() -> &'static dyn SystemEnv {
    &*services().env
}

pub fn file_ops() -> &'static dyn FileOps {
    &*services().file_ops
}

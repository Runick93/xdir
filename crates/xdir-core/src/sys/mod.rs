//! Registro de servicios del sistema: lo que el núcleo necesita del sistema operativo, detrás de
//! traits. Cada plataforma instala sus implementaciones una sola vez al arrancar (`install`), antes
//! de crear la `App`, y el resto del código las pide con `waker()`, `env()`, `file_ops()`,
//! `disk_scan()`, `shell()`, `file_system()`, `overlays()`, `metadata()` y `preview()`. Los tests instalan implementaciones falsas con `testing::install_fakes()`.

use std::ffi::{OsStr, OsString};
use std::fs::{DirEntry, Metadata as FsMetadata};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use crate::commands::Command;
use crate::fs::{Entry as FsEntry, LoadEvent, Location, MetadataRequest, OverlayRequest, QuickItem};
use crate::keys::Key;
use crate::ops::{Op, OpResult};
use crate::preview::PreviewRequest;
use crate::shell::{Entry, Item};
use crate::xtree::scan::{DriveEntry, FastScan, Lister, Progress};
use crate::xtree::tree::Node;

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

/// Fecha de hoy en hora local, para agrupar por fecha (`list::groups::group_key`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalNow {
    /// Días desde 1970-01-01 en hora local.
    pub day: i64,
    /// Día de la semana: 0 = lunes … 6 = domingo.
    pub weekday: u8,
    /// Mes (1–12).
    pub month: u8,
    pub year: i32,
    /// Diferencia de la hora local con UTC, en segundos (-10800 es UTC-3): con ella se lleva la
    /// fecha de cada archivo (en UTC) al día local.
    pub offset: i64,
}

impl LocalNow {
    /// La fecha local del instante `unix` (segundos desde 1970, UTC) con la diferencia `offset`
    /// (segundos) de la zona horaria. Cálculo puro.
    pub fn from_unix(unix: i64, offset: i64) -> LocalNow {
        let day = (unix + offset).div_euclid(86_400);
        let (year, month, _) = crate::format::civil_from_days(day);
        // 1970-01-01 fue jueves (3 contando desde el lunes).
        LocalNow { day, weekday: (day + 3).rem_euclid(7) as u8, month: month as u8, year: year as i32, offset }
    }

    /// Ahora, con la diferencia `offset` (segundos) de la zona horaria.
    pub fn now_with_offset(offset: i64) -> LocalNow {
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        LocalNow::from_unix(unix, offset)
    }
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
    /// Fecha de hoy en hora local. Por defecto, en UTC; cada sistema la da con su zona horaria.
    fn local_now(&self) -> LocalNow {
        LocalNow::now_with_offset(0)
    }
}

/// Operaciones de archivos del sistema.
pub trait FileOps: Send + Sync {
    /// Corre `op` en su propio hilo; al terminar manda el resultado por `tx` y avisa a `owner`
    /// con `WM_APP_OP_DONE`. Vuelve enseguida.
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op);
}

/// Lo que el escáner y la vista de xtree necesitan del sistema: listar carpetas, el escaneo
/// rápido de un volumen entero, el escaneo con permisos (tecla A), los discos y su espacio.
pub trait DiskScan: Send + Sync {
    /// Listador de carpetas para un escaneo que empieza en `root` (cada sistema decide qué no
    /// recorrer: otros discos, enlaces...).
    fn lister(&self, root: &Path) -> Box<dyn Lister>;
    /// Escaneo rápido del volumen entero (la MFT en Windows), si aplica a `root`.
    fn fast_scan(&self, root: &Path, p: &Progress, cancel: &AtomicBool) -> FastScan;
    /// Tecla A: vuelve a analizar con permisos (bloqueante, corre en un hilo propio). `None` si no
    /// hay árbol (rechazado, cancelado o, en macOS, solo se abre el ajuste de permisos).
    fn elevate(&self, owner: Handle, root: &Path, p: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>>;
    /// Discos que se ofrecen para analizar (se vuelve a leer al abrir el menú).
    fn drives(&self) -> Vec<DriveEntry>;
    /// (total, libre) en bytes del volumen que contiene `path`.
    fn volume_space(&self, path: &Path) -> Option<(u64, u64)>;
    /// ¿`path` es un disco entero (`C:\`, `/`, `/Volumes/X`)?
    fn whole_volume(&self, path: &Path) -> bool;
    /// Carpeta que se analiza al abrir xtree sin elegir nada (el disco del sistema).
    fn default_root(&self) -> PathBuf;
    /// ¿Faltaron permisos en el escaneo? (aviso de la barra de datos y tecla A).
    fn permission_issue(&self, p: &Progress) -> bool;
    /// Aviso de la barra de datos cuando faltan permisos; la vista cambia `{n}` por la cantidad
    /// de carpetas sin permiso.
    fn permission_hint(&self) -> &'static str;
    /// Texto de la barra de datos y de estado mientras corre `elevate`.
    fn elevating_text(&self) -> &'static str;
}

/// Integración con el shell del sistema: portapapeles de archivos, abrir con el programa
/// asociado, propiedades y menú contextual. Los errores son el texto que ve el usuario.
///
/// Los menús y diálogos corren en el hilo de interfaz; lo que necesiten guardar mientras están
/// abiertos (el menú del Shell de Windows) es estado de ese hilo dentro de la plataforma, no del
/// servicio.
pub trait Shell: Send + Sync {
    /// Copia (o corta, con `cut`) los elementos al portapapeles de archivos.
    fn copy_files(&self, items: &[Item], cut: bool) -> Result<(), String>;
    /// Archivos del portapapeles y si se cortaron.
    fn clipboard_files(&self) -> Option<(Vec<PathBuf>, bool)>;
    /// Pega en `dest` lo que no tiene rutas de disco (elementos virtuales). `Ok(true)` si pegó algo.
    fn paste_virtual(&self, owner: Handle, dest: &Location) -> Result<bool, String>;
    /// Vacía el portapapeles (lo cortado se pega una sola vez).
    fn clear_clipboard(&self);
    fn copy_text(&self, owner: Handle, text: &str) -> Result<(), String>;
    /// Diálogo del sistema para elegir una carpeta.
    fn pick_folder(&self, owner: Handle, title: &str) -> Option<PathBuf>;
    /// Abre un elemento con su programa (las aplicaciones se ejecutan).
    fn open_item(&self, owner: Handle, item: &Item);
    /// Abre un archivo con su programa, con `dir` como carpeta de trabajo.
    fn open_file(&self, owner: Handle, file: &Path, dir: Option<&Path>);
    /// Ventana de propiedades ("Obtener información") de los elementos.
    fn properties(&self, owner: Handle, items: &[Item]);
    /// Menú contextual de `items` en `at` (DIPs del área de la ventana); sin `items`, el del fondo
    /// de `folder`. Con `intercept` devuelve los verbos que resuelve la aplicación ("rename",
    /// "delete", "cut", "copy", "paste"); `own` va arriba y si se elige una devuelve `own:<id>`;
    /// `"shell"` si lo resolvió el sistema; `UNPIN_QUICK` si se desancló la carpeta `pinned`.
    #[allow(clippy::too_many_arguments)]
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
    ) -> Option<String>;
    /// Ejecuta un verbo del sistema ("delete"...) sobre los elementos.
    fn invoke_verb(&self, owner: Handle, folder: &Location, items: &[Item], verb: &str) -> Result<(), String>;
    /// Ancla una carpeta al Acceso rápido de Windows.
    fn pin_quick_access(&self, owner: Handle, path: &Path) -> Result<(), String>;
    fn unpin_quick_access(&self, owner: Handle, path: &Path) -> Result<(), String>;
    /// Modo de los menús nativos: `None` sigue al sistema, `Some(oscuro)` lo fuerza.
    fn set_menu_mode(&self, dark: Option<bool>);
    /// Volumen de una ruta, para decidir si arrastrar mueve o copia.
    fn volume_of(&self, path: &Path) -> Option<OsString>;
}

/// El sistema de archivos: leer carpetas (en otro hilo), lo propio de cada sistema que necesita
/// la búsqueda, unidades y lugares especiales, rutas y vigilancia de cambios.
///
/// La lectura y la búsqueda corren en hilos de trabajo: lo que una implementación necesite por
/// hilo (COM en Windows, cachés de AppKit en macOS) se crea en ese hilo, no se guarda acá.
pub trait FileSystem: Send + Sync {
    /// Lanza la lectura de `location` en un hilo: manda `LoadEvent`s por `tx` y avisa a `hwnd`
    /// con `WM_APP_LOAD`. Se abandona si `current` deja de valer `generation`.
    fn spawn_load(&self, hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location);

    // Ganchos de la búsqueda (`fs::spawn_search`), que corre en su propio hilo.

    /// Prepara el hilo de una búsqueda (COM e íconos en Windows) y corre `f` en él.
    fn with_search_thread(&self, f: Box<dyn FnOnce() + Send>);
    /// Datos de un elemento encontrado (los enlaces simbólicos, de lo que apuntan en macOS).
    fn search_metadata(&self, item: &DirEntry, path: &Path) -> Option<FsMetadata>;
    /// ¿Está oculto? (atributo en Windows; punto o marca del Finder en macOS).
    fn is_hidden(&self, meta: &FsMetadata, name: &OsStr) -> bool;
    /// ¿La búsqueda entra en esta carpeta? (en macOS, no en paquetes como las apps).
    fn searchable_dir(&self, name: &OsStr) -> bool;
    /// ¿Es la raíz de otro disco montado adentro? (la búsqueda no cruza).
    fn is_mount_point(&self, path: &Path) -> bool;
    /// Elemento de la lista para un resultado de búsqueda, con su ruta completa en `full`.
    fn disk_entry(&self, path: &Path, name: OsString, meta: &FsMetadata) -> FsEntry;

    /// Carpeta del usuario `i` (0..6: Escritorio, Descargas, Documentos, Imágenes, Música,
    /// Vídeos), si existe.
    fn user_folder(&self, i: usize) -> Option<PathBuf>;
    /// Raíces de las unidades o discos montados.
    fn drives(&self) -> Vec<PathBuf>;
    /// Identificador de una unidad para `drive_label` ("C:\" → "C:"; en macOS, la ruta).
    fn drive_name(&self, root: &Path) -> String;
    /// Nombre de una unidad ("Disco local (C:)", "Macintosh HD"), cacheado.
    fn drive_label(&self, drive: &str) -> String;
    /// Olvida los nombres de unidad guardados (cambió el conjunto de unidades).
    fn forget_drive_labels(&self);
    /// ¿Es una ruta de una unidad que ya no está (se quitó el pendrive)?
    fn drive_missing(&self, path: &Path) -> bool;
    /// Nombre que el sistema le da a una raíz (una ruta sin último tramo: `C:`, `C:\`, `/`), para
    /// títulos y migas: "Disco local (C:)" en Windows, el nombre del disco en macOS. `None` = se
    /// muestra el texto de la ruta (y, en las migas, la raíz sola se pega al tramo anterior).
    fn root_title(&self, path: &Path) -> Option<String>;
    /// Subcarpetas de un lugar, para el menú de las flechas de la barra de dirección.
    fn subfolders(&self, loc: &Location) -> Vec<(String, Location)>;
    /// Ícono genérico de carpeta.
    fn folder_icon(&self) -> Option<i32>;
    /// Tipo que el sistema le da a una carpeta (el mismo que muestra la lista).
    fn folder_type(&self) -> Arc<str> {
        Arc::from("Carpeta")
    }
    /// Lo escrito en la barra de dirección como lugar especial del Shell ("shell:RecycleBinFolder").
    fn shell_location(&self, text: &str) -> Option<Location>;
    /// Nombre de análisis de la Red del Explorador (vacío si no existe en el sistema).
    fn network_place(&self) -> &'static str;
    /// Nombre de análisis de la carpeta de Linux (WSL) del Explorador (vacío si no existe).
    fn linux_place(&self) -> &'static str;
    /// ¿Existe esa carpeta virtual en este equipo?
    fn shell_place_exists(&self, parsing: &str) -> bool;
    /// Subcarpetas con ruta de disco de una carpeta virtual: (nombre, ruta), por nombre.
    fn shell_subfolders(&self, parsing: &str) -> Vec<(String, PathBuf)>;
    /// Carpetas del Acceso rápido de Windows (vacío donde no existe).
    fn quick_access(&self) -> Vec<QuickItem>;
    /// ¿Está anclada ahora mismo al Acceso rápido?
    fn is_quick_pinned(&self, path: &Path) -> bool;
    /// Carpeta cuyos cambios indican que el Acceso rápido se modificó.
    fn quick_access_watch_dir(&self) -> Option<PathBuf>;
    /// Forma larga de una ruta ("C:\Users\LEANDR~1" → "C:\Users\Leandro-Dev").
    fn long_path(&self, path: &Path) -> PathBuf;
    /// Ruta escrita a mano con las mayúsculas reales del disco.
    fn true_case(&self, path: &Path) -> PathBuf;
    /// Conecta con el recurso de red de `path` pidiendo credenciales si hace falta.
    fn connect_share(&self, owner: Handle, path: &Path) -> bool;
    /// ¿Es la misma ruta? (sin distinguir mayúsculas ni separador final).
    fn same_path(&self, a: &Path, b: &Path) -> bool;
    /// Vigila la carpeta `path` y avisa a `hwnd` con `msg` (`wparam` = `panel`, `lparam`) cuando
    /// cambia algo. Deja de vigilar al soltar el guardián. `None` si no se puede vigilar.
    fn watch(&self, hwnd: Handle, msg: u32, panel: usize, lparam: isize, path: &Path) -> Option<Box<dyn Watch>>;
    /// ¿El elemento está solo en la nube (iCloud, OneDrive, Dropbox)? Entonces no se lee su
    /// contenido ni se pide su imagen en macOS: abrirlo lo bajaría entero. No toca el contenido.
    fn cloud_only(&self, _path: &Path) -> bool {
        false
    }
    /// Caracteres que no puede tener un nombre (`rename_many::INVALID_WINDOWS` o `INVALID_UNIX`).
    fn invalid_name_chars(&self) -> &'static [char] {
        crate::rename_many::INVALID_UNIX
    }
    /// ¿El disco de `dir` no distingue mayúsculas? Bloqueante (corre en un hilo de trabajo). Por
    /// defecto sí: así se marcan más choques, nunca menos.
    fn case_insensitive(&self, _dir: &Path) -> bool {
        true
    }
    /// Renombra `from` a `to` sin pisar nunca lo que ya está (falla si `to` existe). Bloqueante:
    /// corre en un hilo de trabajo. El error es el texto para el usuario.
    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), String>;
}

/// Un guardián de carpeta (ver `FileSystem::watch`): vigila mientras viva y al soltarlo
/// (`Drop`) termina su hilo.
pub trait Watch: Send {}

/// Superposiciones de íconos (flecha de acceso directo, OneDrive, Git...): se resuelven en un
/// hilo aparte y vuelven como `LoadMsg::Overlays` por el canal del pedido.
pub trait Overlays: Send + Sync {
    fn request(&self, req: OverlayRequest);
}

/// Metadatos de archivos para las columnas Dimensiones, Duración, Autor, Etiquetas, Artista y
/// Álbum: se leen en un hilo aparte y vuelven como `LoadMsg::Metadata` por el canal del pedido,
/// en lotes (ver `MetadataRequest::send`). Donde no hay de dónde leerlos se responde vacío
/// (`MetadataRequest::answer_empty`).
pub trait Metadata: Send + Sync {
    fn request(&self, req: MetadataRequest);
}

/// Vista previa de la columna (texto, carpeta, disco, último acceso, nube, sin permiso): se lee
/// en otro hilo con `preview::answer` y vuelve por el canal del pedido (`PreviewRequest::send`).
pub trait Preview: Send + Sync {
    fn request(&self, req: PreviewRequest);
}

/// Las implementaciones de una plataforma, instaladas juntas con `install`.
pub struct Services {
    pub waker: Box<dyn Waker>,
    pub env: Box<dyn SystemEnv>,
    pub file_ops: Box<dyn FileOps>,
    pub disk_scan: Box<dyn DiskScan>,
    pub shell: Box<dyn Shell>,
    pub file_system: Box<dyn FileSystem>,
    pub overlays: Box<dyn Overlays>,
    pub metadata: Box<dyn Metadata>,
    pub preview: Box<dyn Preview>,
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

pub fn disk_scan() -> &'static dyn DiskScan {
    &*services().disk_scan
}

pub fn shell() -> &'static dyn Shell {
    &*services().shell
}

pub fn file_system() -> &'static dyn FileSystem {
    &*services().file_system
}

pub fn overlays() -> &'static dyn Overlays {
    &*services().overlays
}

pub fn metadata() -> &'static dyn Metadata {
    &*services().metadata
}

pub fn preview() -> &'static dyn Preview {
    &*services().preview
}

//! Implementaciones falsas de los servicios del sistema, para los tests (de este crate y de
//! `xdir`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use super::{ConsoleInfo, DiskScan, FileOps, Handle, Services, Shell, SystemEnv, Waker};
use crate::fs::Location;
use crate::format::thousands;
use crate::keys::Key;
use crate::ops::{Op, OpResult, WM_APP_OP_DONE};
use crate::shell::{Entry, Item};
use crate::xtree::scan::{DirListing, DriveEntry, FastScan, FileEntry, ListError, Lister, Progress};
use crate::xtree::tree::Node;

/// Instala los servicios falsos. Se puede llamar desde todos los tests: solo la primera vez
/// instala, y si ya había servicios instalados no hace nada.
pub fn install_fakes() {
    super::install_once(|| Services {
        waker: Box::new(FakeWaker),
        env: Box::new(FakeEnv),
        file_ops: Box::new(FakeFileOps),
        disk_scan: Box::new(FakeDiskScan),
        shell: Box::new(FakeShell),
    });
}

/// Mensajes recibidos por el `FakeWaker`, por número de mensaje (de todo el proceso).
static POSTED: Mutex<BTreeMap<u32, usize>> = Mutex::new(BTreeMap::new());

/// `Waker` falso: no despierta a nadie, solo cuenta cuántos mensajes de cada número le llegaron.
pub struct FakeWaker;

impl FakeWaker {
    /// Cuántos mensajes `msg` se mandaron hasta ahora (en todo el proceso de tests: conviene
    /// usar un número de mensaje propio de cada test, o comparar contra el valor de antes).
    pub fn count(msg: u32) -> usize {
        POSTED.lock().unwrap_or_else(|e| e.into_inner()).get(&msg).copied().unwrap_or(0)
    }
}

impl Waker for FakeWaker {
    fn post(&self, _window: Handle, msg: u32, _wparam: usize, _lparam: isize) {
        *POSTED.lock().unwrap_or_else(|e| e.into_inner()).entry(msg).or_insert(0) += 1;
    }
}

thread_local! {
    /// Historial de rutas escritas del falso: en memoria y por hilo, para que los tests no se
    /// pisen.
    static TYPED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `SystemEnv` falso: tema claro, sin animaciones ni teclas apretadas, una carpeta de
/// configuración temporal por hilo, formatos deterministas y una sola consola.
pub struct FakeEnv;

impl SystemEnv for FakeEnv {
    fn system_dark(&self) -> bool {
        false
    }

    fn system_accent(&self) -> Option<u32> {
        None
    }

    fn animations_allowed(&self) -> bool {
        false
    }

    fn key_down(&self, _k: Key) -> bool {
        false
    }

    /// En pruebas la configuración nunca es la real: cada hilo de prueba tiene su propia carpeta
    /// temporal (`temp/xdir-tests/<pid>-<id del hilo>`).
    fn config_dir(&self) -> Option<PathBuf> {
        let name = format!("{}-{:?}", std::process::id(), std::thread::current().id());
        Some(std::env::temp_dir().join("xdir-tests").join(name.replace(['(', ')'], "")))
    }

    /// "AAAA-MM-DD" (UTC) del FILETIME; vacío si es 0.
    fn format_date(&self, filetime: u64) -> String {
        if filetime == 0 {
            return String::new();
        }
        let (y, m, d) = civil_from_days((filetime / 864_000_000_000) as i64 - DAYS_1601_TO_1970);
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// Como el Explorador de Windows: "1.234 KB" (redondea hacia arriba).
    fn format_size_kb(&self, bytes: u64) -> String {
        format!("{} KB", thousands(bytes.div_ceil(1024)))
    }

    /// Como Windows: "123 bytes", "1,23 KB", "12,3 MB", "476 GB" (tres cifras, truncadas).
    fn format_size_short(&self, bytes: u64) -> String {
        if bytes < 1024 {
            return format!("{bytes} bytes");
        }
        let units = ["KB", "MB", "GB", "TB", "PB", "EB"];
        let mut unit = 0;
        let mut scale = 1024u128;
        while (bytes as u128) >= scale * 1024 && unit + 1 < units.len() {
            scale *= 1024;
            unit += 1;
        }
        // Centésimas de la unidad, truncadas.
        let hundredths = bytes as u128 * 100 / scale;
        let text = match hundredths {
            0..1000 => format!("{},{:02}", hundredths / 100, hundredths % 100),
            1000..10000 => format!("{},{}", hundredths / 100, hundredths % 100 / 10),
            _ => format!("{}", hundredths / 100),
        };
        format!("{text} {}", units[unit])
    }

    fn typed_paths_load(&self) -> Vec<String> {
        TYPED.with(|t| t.borrow().clone())
    }

    fn typed_paths_save(&self, list: &[String]) {
        TYPED.with(|t| *t.borrow_mut() = list.to_vec());
    }

    fn consoles(&self) -> Vec<ConsoleInfo> {
        vec![ConsoleInfo { id: 0, label: "Terminal" }]
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    fn open_console(&self, _owner: Handle, _id: u8, _dir: &Path, _verb: &str) -> bool {
        false
    }

    /// Ninguna raíz tiene nombre propio: se muestra el texto de la ruta.
    fn root_title(&self, _path: &Path) -> Option<String> {
        None
    }
}

/// Días entre 1601-01-01 (época de FILETIME) y 1970-01-01.
const DAYS_1601_TO_1970: i64 = 134_774;

/// Fecha civil (año, mes, día) de un número de días desde 1970-01-01 (algoritmo de Howard
/// Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

thread_local! {
    /// Operaciones recibidas por el falso y respuesta configurada: por hilo, para que los tests
    /// (que corren en paralelo, cada uno en su hilo) no se vean entre sí.
    static OPS: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
    static RESPONSE: RefCell<OpResult> = RefCell::new(OpResult::default());
}

/// `FileOps` falso: no toca el disco; anota cada `Op` recibida y responde por el canal, en el
/// acto, con el `OpResult` configurado (por defecto, éxito sin deshacer). El registro es del hilo
/// que llama a `spawn`, o sea el del test.
pub struct FakeFileOps;

impl FakeFileOps {
    /// Las operaciones pedidas desde este hilo hasta ahora (y vacía el registro).
    pub fn take_ops() -> Vec<Op> {
        OPS.with(|o| std::mem::take(&mut *o.borrow_mut()))
    }

    /// Respuesta de las próximas operaciones de este hilo.
    pub fn respond_with(result: OpResult) {
        RESPONSE.with(|r| *r.borrow_mut() = result);
    }
}

impl FileOps for FakeFileOps {
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op) {
        OPS.with(|o| o.borrow_mut().push(op));
        let _ = tx.send(RESPONSE.with(|r| r.borrow().clone()));
        owner.post(WM_APP_OP_DONE, 0, 0);
    }
}

/// Una carpeta servida por el `FakeDiskScan`: subcarpetas y archivos (nombre, tamaño).
type FakeDir = (Vec<String>, Vec<(String, u64)>);

/// Carpetas del disco falso y carpetas que se listaron, por ruta completa. Son de todo el proceso
/// (el escáner lista desde sus propios hilos, no desde el del test): cada test usa rutas propias.
static FAKE_DIRS: Mutex<BTreeMap<PathBuf, FakeDir>> = Mutex::new(BTreeMap::new());
static LISTED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// `DiskScan` falso: nunca toca el disco. El listador sirve las carpetas cargadas con `serve` (las
/// demás, vacías); sin escaneo rápido ni elevado, sin discos ni datos de volumen, y la raíz por
/// defecto es una carpeta temporal. Faltan permisos solo si el `Progress` lo dice (como en
/// Windows y macOS juntos: `need_admin` o carpetas denegadas).
pub struct FakeDiskScan;

impl FakeDiskScan {
    /// Carga en el disco falso la carpeta `path` con sus subcarpetas y archivos (nombre, tamaño).
    pub fn serve(path: &Path, dirs: &[&str], files: &[(&str, u64)]) {
        let dir = (dirs.iter().map(|d| d.to_string()).collect(), files.iter().map(|(n, s)| (n.to_string(), *s)).collect());
        FAKE_DIRS.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), dir);
    }

    /// ¿El listador falso listó `path`?
    pub fn was_listed(path: &Path) -> bool {
        LISTED.lock().unwrap_or_else(|e| e.into_inner()).contains(path)
    }

    /// La raíz por defecto del falso (una carpeta temporal; no hace falta que exista).
    pub fn root() -> PathBuf {
        std::env::temp_dir().join("xdir-tests").join("xtree-raiz")
    }
}

struct FakeLister;

impl Lister for FakeLister {
    fn list(&self, path: &Path) -> Result<DirListing, ListError> {
        LISTED.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf());
        let dirs = FAKE_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        let Some((sub, files)) = dirs.get(path) else { return Ok(DirListing::default()) };
        Ok(DirListing {
            dirs: sub.iter().map(OsString::from).collect(),
            files: files.iter().map(|(n, s)| FileEntry { name: n.as_str().into(), size: *s, id: None }).collect(),
        })
    }
}

impl DiskScan for FakeDiskScan {
    fn lister(&self, _root: &Path) -> Box<dyn Lister> {
        Box::new(FakeLister)
    }

    fn fast_scan(&self, _root: &Path, _p: &Progress, _cancel: &AtomicBool) -> FastScan {
        FastScan::NotApplicable
    }

    fn elevate(&self, _owner: Handle, _root: &Path, _p: &Progress, _cancel: &AtomicBool) -> Option<Vec<Node>> {
        None
    }

    fn drives(&self) -> Vec<DriveEntry> {
        Vec::new()
    }

    fn volume_space(&self, _path: &Path) -> Option<(u64, u64)> {
        None
    }

    fn whole_volume(&self, _path: &Path) -> bool {
        false
    }

    fn default_root(&self) -> PathBuf {
        FakeDiskScan::root()
    }

    fn permission_issue(&self, p: &Progress) -> bool {
        p.need_admin.load(Ordering::Relaxed) || p.denied.load(Ordering::Relaxed) > 0
    }

    fn permission_hint(&self) -> &'static str {
        "{n} carpetas sin permiso (tecla A)"
    }

    fn elevating_text(&self) -> &'static str {
        "Analizando con permisos\u{2026}"
    }
}

/// Una llamada recibida por el `FakeShell` (rutas en lugar de `Item`, salvo en el menú).
#[derive(Clone, Debug, PartialEq)]
pub enum ShellCall {
    CopyFiles { paths: Vec<PathBuf>, cut: bool },
    PasteVirtual(Location),
    ClearClipboard,
    CopyText(String),
    PickFolder(String),
    OpenItem(PathBuf),
    OpenFile(PathBuf, Option<PathBuf>),
    Properties(Vec<PathBuf>),
    ContextMenu { folder: Location, items: Vec<Item>, at: (f32, f32), extended: bool, intercept: bool, own: Vec<Entry>, pinned: bool },
    InvokeVerb { folder: Location, items: Vec<PathBuf>, verb: String },
    PinQuickAccess(PathBuf),
    UnpinQuickAccess(PathBuf),
    SetMenuMode(Option<bool>),
}

/// Respuestas configurables del `FakeShell`.
#[derive(Default)]
struct ShellState {
    calls: Vec<ShellCall>,
    /// Portapapeles de archivos: lo llena `copy_files` (o `FakeShell::set_clipboard`).
    clipboard: Option<(Vec<PathBuf>, bool)>,
    /// Lo que devuelve el próximo `context_menu`.
    menu_choice: Option<String>,
    /// Si hay, las operaciones que pueden fallar fallan con este texto.
    error: Option<String>,
    /// Carpeta que devuelve `pick_folder`.
    picked: Option<PathBuf>,
}

thread_local! {
    /// Estado del falso por hilo: los tests corren en paralelo, cada uno en su hilo, y el Shell se
    /// usa desde el hilo de interfaz (el del test).
    static SHELL: RefCell<ShellState> = RefCell::new(ShellState::default());
}

fn shell_state<T>(f: impl FnOnce(&mut ShellState) -> T) -> T {
    SHELL.with(|s| f(&mut s.borrow_mut()))
}

fn record(call: ShellCall) {
    shell_state(|s| s.calls.push(call));
}

fn paths(items: &[Item]) -> Vec<PathBuf> {
    items.iter().map(|i| i.path.clone()).collect()
}

/// `Shell` falso: no toca el sistema. Anota cada llamada (por hilo), guarda los archivos copiados
/// en un portapapeles en memoria y responde lo configurado: el menú no elige nada, no hay errores
/// ni carpeta elegida. El volumen de una ruta es su primer tramo en minúsculas (como Windows).
pub struct FakeShell;

impl FakeShell {
    /// Las llamadas hechas desde este hilo hasta ahora (y vacía el registro).
    pub fn take_calls() -> Vec<ShellCall> {
        shell_state(|s| std::mem::take(&mut s.calls))
    }

    /// Contenido del portapapeles de archivos de este hilo.
    pub fn set_clipboard(files: Option<(Vec<PathBuf>, bool)>) {
        shell_state(|s| s.clipboard = files);
    }

    /// Lo que devuelven los próximos menús contextuales de este hilo.
    pub fn choose(choice: Option<&str>) {
        shell_state(|s| s.menu_choice = choice.map(String::from));
    }

    /// Hace fallar (o no) las operaciones de este hilo que devuelven `Result`.
    pub fn fail_with(error: Option<&str>) {
        shell_state(|s| s.error = error.map(String::from));
    }

    /// Carpeta que devuelve `pick_folder` en este hilo.
    pub fn pick(folder: Option<PathBuf>) {
        shell_state(|s| s.picked = folder);
    }
}

fn outcome() -> Result<(), String> {
    shell_state(|s| s.error.clone()).map_or(Ok(()), Err)
}

impl Shell for FakeShell {
    fn copy_files(&self, items: &[Item], cut: bool) -> Result<(), String> {
        record(ShellCall::CopyFiles { paths: paths(items), cut });
        outcome()?;
        shell_state(|s| s.clipboard = Some((paths(items), cut)));
        Ok(())
    }

    fn clipboard_files(&self) -> Option<(Vec<PathBuf>, bool)> {
        shell_state(|s| s.clipboard.clone())
    }

    fn paste_virtual(&self, _owner: Handle, dest: &Location) -> Result<bool, String> {
        record(ShellCall::PasteVirtual(dest.clone()));
        outcome().map(|()| false)
    }

    fn clear_clipboard(&self) {
        record(ShellCall::ClearClipboard);
        shell_state(|s| s.clipboard = None);
    }

    fn copy_text(&self, _owner: Handle, text: &str) -> Result<(), String> {
        record(ShellCall::CopyText(text.to_string()));
        outcome()
    }

    fn pick_folder(&self, _owner: Handle, title: &str) -> Option<PathBuf> {
        record(ShellCall::PickFolder(title.to_string()));
        shell_state(|s| s.picked.clone())
    }

    fn open_item(&self, _owner: Handle, item: &Item) {
        record(ShellCall::OpenItem(item.path.clone()));
    }

    fn open_file(&self, _owner: Handle, file: &Path, dir: Option<&Path>) {
        record(ShellCall::OpenFile(file.to_path_buf(), dir.map(Path::to_path_buf)));
    }

    fn properties(&self, _owner: Handle, items: &[Item]) {
        record(ShellCall::Properties(paths(items)));
    }

    fn context_menu(
        &self,
        _owner: Handle,
        folder: &Location,
        items: &[Item],
        at: (f32, f32),
        extended: bool,
        intercept: bool,
        own: &[Entry],
        pinned: bool,
    ) -> Option<String> {
        record(ShellCall::ContextMenu { folder: folder.clone(), items: items.to_vec(), at, extended, intercept, own: own.to_vec(), pinned });
        shell_state(|s| s.menu_choice.clone())
    }

    fn invoke_verb(&self, _owner: Handle, folder: &Location, items: &[Item], verb: &str) -> Result<(), String> {
        record(ShellCall::InvokeVerb { folder: folder.clone(), items: paths(items), verb: verb.to_string() });
        outcome()
    }

    fn pin_quick_access(&self, _owner: Handle, path: &Path) -> Result<(), String> {
        record(ShellCall::PinQuickAccess(path.to_path_buf()));
        outcome()
    }

    fn unpin_quick_access(&self, _owner: Handle, path: &Path) -> Result<(), String> {
        record(ShellCall::UnpinQuickAccess(path.to_path_buf()));
        outcome()
    }

    fn set_menu_mode(&self, dark: Option<bool>) {
        record(ShellCall::SetMenuMode(dark));
    }

    fn volume_of(&self, path: &Path) -> Option<OsString> {
        path.components().next().map(|c| c.as_os_str().to_ascii_lowercase())
    }
}

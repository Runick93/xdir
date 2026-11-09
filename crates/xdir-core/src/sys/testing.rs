//! Implementaciones falsas de los servicios del sistema, para los tests (de este crate y de
//! `xdir`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;

use super::{ConsoleInfo, DiskScan, FileOps, FileSystem, Handle, Overlays, Services, Shell, SystemEnv, Watch, Waker};
use crate::fs::{Entry as FsEntry, LoadEvent, LoadMsg, Location, OverlayRequest, QuickItem, WM_APP_LOAD};
use crate::config::ColorRule;
use crate::format::{DAYS_1601_TO_1970, civil_from_days, size_kb_text, size_short_text};
use crate::geom::{Color, Rect};
use crate::host::{MenuPos, TextBox, TextKind, WindowHost};
use crate::keys::Key;
use crate::ops::{Op, OpResult, WM_APP_OP_DONE};
use crate::paint::{FontId, Painter};
use crate::shell::{Entry, Item};
use crate::theme::Theme;
use crate::thumbs;
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
        file_system: Box::new(FakeFileSystem),
        overlays: Box::new(FakeOverlays),
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
        size_kb_text(bytes)
    }

    /// Como Windows: "123 bytes", "1,23 KB", "12,3 MB", "476 GB" (tres cifras, truncadas).
    fn format_size_short(&self, bytes: u64) -> String {
        size_short_text(bytes)
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

/// Lecturas pedidas al `FileSystem` falso y guardianes creados y soltados, por ruta. Son de todo
/// el proceso (las lecturas corren en sus propios hilos): cada test usa rutas propias.
static LOADS: Mutex<BTreeMap<PathBuf, usize>> = Mutex::new(BTreeMap::new());
static WATCHES: Mutex<BTreeMap<PathBuf, (usize, usize)>> = Mutex::new(BTreeMap::new());

thread_local! {
    /// Unidades del falso en este hilo (ruta y nombre); vacío salvo `FakeFileSystem::set_drives`.
    static DRIVES: RefCell<Vec<(PathBuf, String)>> = const { RefCell::new(Vec::new()) };
}

/// `FileSystem` falso sobre `std::fs`: lee carpetas reales (las temporales de cada test) sin
/// íconos ni tipos del sistema, sin unidades (salvo las de `set_drives`, por hilo), sin carpetas
/// del usuario ni lugares del Shell. Lo que empieza con punto está oculto; las rutas se comparan
/// sin distinguir mayúsculas ni separador final, y quedan como se escribieron (`long_path` y
/// `true_case` no las cambian).
pub struct FakeFileSystem;

impl FakeFileSystem {
    /// Unidades (ruta y nombre) que ve este hilo: `drives` da las rutas y `drive_label`, los
    /// nombres. Con una lista vacía vuelve a no haber unidades.
    pub fn set_drives(drives: &[(&Path, &str)]) {
        DRIVES.with(|d| *d.borrow_mut() = drives.iter().map(|(p, n)| (p.to_path_buf(), n.to_string())).collect());
    }

    /// Cuántas lecturas de `path` se pidieron (en todo el proceso).
    pub fn loads(path: &Path) -> usize {
        LOADS.lock().unwrap_or_else(|e| e.into_inner()).get(path).copied().unwrap_or(0)
    }

    /// (creados, soltados) de los guardianes de `path` (en todo el proceso).
    pub fn watches(path: &Path) -> (usize, usize) {
        WATCHES.lock().unwrap_or_else(|e| e.into_inner()).get(path).copied().unwrap_or((0, 0))
    }

    /// Elemento de la lista para un elemento de disco: carpeta o archivo, sin ícono.
    fn entry(name: OsString, meta: &std::fs::Metadata) -> FsEntry {
        const ATTR_HIDDEN: u32 = 0x2;
        const ATTR_DIRECTORY: u32 = 0x10;
        let display = name.to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        let mut attrs = if is_dir { ATTR_DIRECTORY } else { 0 };
        if display.starts_with('.') {
            attrs |= ATTR_HIDDEN;
        }
        FsEntry {
            sort_key: display.to_lowercase(),
            display,
            name,
            full: None,
            group: 0,
            pidl: None,
            is_dir,
            size: if is_dir { 0 } else { meta.len() },
            modified: 0,
            created: 0,
            attrs,
            type_name: Arc::from(if is_dir { "Carpeta" } else { "Archivo" }),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }
}

/// Lee `path` y manda todo en un lote, y después `Listed` (o el error al abrirla).
fn fake_list(path: &Path, cancelled: impl Fn() -> bool, send: impl Fn(LoadMsg)) {
    let reader = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(e) => return send(LoadMsg::Listed(Err(e.to_string()))),
    };
    let mut batch = Vec::new();
    for item in reader.flatten() {
        if cancelled() {
            return;
        }
        let Ok(meta) = std::fs::metadata(item.path()).or_else(|_| item.metadata()) else { continue };
        batch.push(FakeFileSystem::entry(item.file_name(), &meta));
    }
    if !batch.is_empty() {
        send(LoadMsg::Entries(batch));
    }
    send(LoadMsg::Listed(Ok(())));
}

impl FileSystem for FakeFileSystem {
    fn spawn_load(&self, hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
        if let Location::Path(p) = &location {
            *LOADS.lock().unwrap_or_else(|e| e.into_inner()).entry(p.clone()).or_insert(0) += 1;
        }
        std::thread::spawn(move || {
            let send = |msg: LoadMsg| {
                if tx.send(LoadEvent { panel, generation, msg }).is_ok() {
                    hwnd.post(WM_APP_LOAD, panel, 0);
                }
            };
            match &location {
                Location::Path(p) => fake_list(p, || current.load(Ordering::Relaxed) != generation, send),
                // "Este equipo" del falso: vacío (sin unidades ni carpetas del usuario).
                Location::Computer => send(LoadMsg::Listed(Ok(()))),
                Location::Shell { .. } => send(LoadMsg::Listed(Err("Esta ubicación no existe".into()))),
            }
        });
    }

    fn with_search_thread(&self, f: Box<dyn FnOnce() + Send>) {
        f();
    }

    fn search_metadata(&self, item: &std::fs::DirEntry, path: &Path) -> Option<std::fs::Metadata> {
        std::fs::metadata(path).or_else(|_| item.metadata()).ok()
    }

    fn is_hidden(&self, _meta: &std::fs::Metadata, name: &OsStr) -> bool {
        name.to_string_lossy().starts_with('.')
    }

    fn searchable_dir(&self, _name: &OsStr) -> bool {
        true
    }

    fn is_mount_point(&self, _path: &Path) -> bool {
        false
    }

    fn disk_entry(&self, path: &Path, name: OsString, meta: &std::fs::Metadata) -> FsEntry {
        let mut e = FakeFileSystem::entry(name, meta);
        e.full = Some(path.as_os_str().to_os_string());
        e
    }

    fn user_folder(&self, _i: usize) -> Option<PathBuf> {
        None
    }

    fn drives(&self) -> Vec<PathBuf> {
        DRIVES.with(|d| d.borrow().iter().map(|(p, _)| p.clone()).collect())
    }

    fn drive_name(&self, root: &Path) -> String {
        root.to_string_lossy().into_owned()
    }

    /// El nombre de `set_drives` (si la unidad es de este hilo); si no, el mismo texto.
    fn drive_label(&self, drive: &str) -> String {
        DRIVES.with(|d| d.borrow().iter().find(|(p, _)| p.to_string_lossy() == drive).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| drive.to_string())
    }

    fn forget_drive_labels(&self) {}

    fn drive_missing(&self, _path: &Path) -> bool {
        false
    }

    /// Ninguna raíz tiene nombre propio: se muestra el texto de la ruta.
    fn root_title(&self, _path: &Path) -> Option<String> {
        None
    }

    fn subfolders(&self, loc: &Location) -> Vec<(String, Location)> {
        let Location::Path(dir) = loc else { return Vec::new() };
        let mut out: Vec<(String, Location)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .map(|name| (name.clone(), Location::Path(dir.join(name))))
            .collect();
        out.sort_by_key(|(n, _)| n.to_lowercase());
        out
    }

    fn folder_icon(&self) -> Option<i32> {
        None
    }

    fn shell_location(&self, _text: &str) -> Option<Location> {
        None
    }

    fn network_place(&self) -> &'static str {
        ""
    }

    fn linux_place(&self) -> &'static str {
        ""
    }

    fn shell_place_exists(&self, _parsing: &str) -> bool {
        false
    }

    fn shell_subfolders(&self, _parsing: &str) -> Vec<(String, PathBuf)> {
        Vec::new()
    }

    fn quick_access(&self) -> Vec<QuickItem> {
        Vec::new()
    }

    fn is_quick_pinned(&self, _path: &Path) -> bool {
        false
    }

    fn quick_access_watch_dir(&self) -> Option<PathBuf> {
        None
    }

    fn long_path(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn true_case(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn connect_share(&self, _owner: Handle, _path: &Path) -> bool {
        false
    }

    fn same_path(&self, a: &Path, b: &Path) -> bool {
        let norm = |p: &Path| {
            let s = p.to_string_lossy();
            let t = s.trim_end_matches(['/', '\\']);
            if t.is_empty() { s.chars().take(1).collect() } else { t.to_lowercase() }
        };
        norm(a) == norm(b)
    }

    /// Un guardián que no vigila nada: solo cuenta altas y bajas por ruta. `None` si la carpeta
    /// no existe (como los reales).
    fn watch(&self, _hwnd: Handle, _msg: u32, _panel: usize, _lparam: isize, path: &Path) -> Option<Box<dyn Watch>> {
        if !path.is_dir() {
            return None;
        }
        WATCHES.lock().unwrap_or_else(|e| e.into_inner()).entry(path.to_path_buf()).or_insert((0, 0)).0 += 1;
        Some(Box::new(FakeWatch(path.to_path_buf())))
    }
}

/// Guardián del `FileSystem` falso: al soltarlo, cuenta una baja para su carpeta.
pub struct FakeWatch(PathBuf);

impl Watch for FakeWatch {}

impl Drop for FakeWatch {
    fn drop(&mut self) {
        WATCHES.lock().unwrap_or_else(|e| e.into_inner()).entry(self.0.clone()).or_insert((0, 0)).1 += 1;
    }
}

/// `Overlays` falso: no hay superposiciones (no responde nada).
pub struct FakeOverlays;

impl Overlays for FakeOverlays {
    fn request(&self, _req: OverlayRequest) {}
}

/// `Painter` que no dibuja: registra cada `text` (fuente, texto y rectángulo) para que los tests
/// vean qué se escribió y con qué formato. `measure` da 7 DIPs por carácter; `clip` solo llama a
/// la clausura; íconos y miniaturas no hacen nada (`thumbnail` da `false`).
pub struct RecordingPainter {
    theme: Theme,
    texts: RefCell<Vec<(FontId, String, Rect)>>,
}

impl RecordingPainter {
    pub fn new(theme: Theme) -> Self {
        Self { theme, texts: RefCell::new(Vec::new()) }
    }

    /// Los textos dibujados, en orden.
    pub fn texts(&self) -> Vec<(FontId, String, Rect)> {
        self.texts.borrow().clone()
    }
}

impl Painter for RecordingPainter {
    fn fill(&self, _r: Rect, _c: Color) {}
    fn round_fill(&self, _r: Rect, _radius: f32, _c: Color) {}
    fn rounded_path(&self, _pts: &[(f32, f32, f32)], _closed: bool, _fill: Option<Color>, _stroke: Option<(Color, f32)>) {}
    fn circle(&self, _cx: f32, _cy: f32, _radius: f32, _fill: Color) {}
    fn circle_stroke(&self, _cx: f32, _cy: f32, _radius: f32, _c: Color, _width: f32) {}
    fn round_stroke_w(&self, _r: Rect, _radius: f32, _c: Color, _width: f32) {}
    fn round_stroke(&self, _r: Rect, _radius: f32, _c: Color) {}

    fn text(&self, font: FontId, s: &str, r: Rect, _c: Color) {
        self.texts.borrow_mut().push((font, s.to_string(), r));
    }

    fn measure(&self, _font: FontId, s: &str) -> f32 {
        7.0 * s.chars().count() as f32
    }

    fn shell_icon(&self, _index: i32, _overlay: u8, _x: f32, _y: f32, _size: f32) {}

    fn thumbnail(&self, _key: &thumbs::Key, _r: Rect) -> bool {
        false
    }

    fn logo(&self, _r: Rect) {}

    fn thumb_px(&self, dips: f32) -> u32 {
        dips.round() as u32
    }

    fn clip(&self, _r: Rect, f: &mut dyn FnMut()) {
        f();
    }

    fn theme(&self) -> &Theme {
        &self.theme
    }

    fn file_colors(&self) -> &[ColorRule] {
        &[]
    }
}

/// Ventana falsa para las pruebas de `App`: no crea nada. El área mide 1280×800 DIPs (se puede
/// cambiar con `client`), registra títulos, pedidos de repintar y pitidos, expande variables del
/// entorno y responde lo que el test le fije a las preguntas, al selector de color y al arrastre.
/// Dibuja con un `RecordingPainter` (guarda los textos del último cuadro) y mide 7 DIPs por
/// carácter. Las cajas de texto son entradas de un mapa (tipo, texto y si tienen el foco); los
/// menús se registran y devuelven lo que fije `set_menu_choice`.
pub struct FakeHost {
    /// Tamaño del área de la ventana en DIPs.
    pub client: Cell<(f32, f32)>,
    titles: RefCell<Vec<String>>,
    invalidations: Cell<usize>,
    beeps: Cell<usize>,
    color_choice: Cell<Option<u32>>,
    answer: Cell<Option<bool>>,
    drag_result: RefCell<Result<(), String>>,
    drawn: RefCell<Vec<(FontId, String, Rect)>>,
    /// Cajas de texto abiertas: tipo, texto y si tienen el foco.
    text_boxes: RefCell<HashMap<u32, (TextKind, String, bool)>>,
    next_text_box: Cell<u32>,
    menu_choice: Cell<Option<u32>>,
    menus: RefCell<Vec<Vec<Entry>>>,
    /// La ventana que se le pasó a `attach` (la última).
    attached: Cell<Option<Handle>>,
}

impl FakeHost {
    pub fn new() -> Rc<FakeHost> {
        Rc::new(FakeHost {
            client: Cell::new((1280.0, 800.0)),
            titles: RefCell::new(Vec::new()),
            invalidations: Cell::new(0),
            beeps: Cell::new(0),
            color_choice: Cell::new(None),
            answer: Cell::new(None),
            drag_result: RefCell::new(Ok(())),
            drawn: RefCell::new(Vec::new()),
            text_boxes: RefCell::new(HashMap::new()),
            next_text_box: Cell::new(1),
            menu_choice: Cell::new(None),
            menus: RefCell::new(Vec::new()),
            attached: Cell::new(None),
        })
    }

    /// El texto de la caja abierta de ese tipo (`None` si no hay ninguna).
    pub fn text_of(&self, kind: TextKind) -> Option<String> {
        self.text_boxes.borrow().values().find(|(k, _, _)| *k == kind).map(|(_, t, _)| t.clone())
    }

    /// Lo que devuelven los menús (`None`: se cerraron sin elegir).
    pub fn set_menu_choice(&self, id: Option<u32>) {
        self.menu_choice.set(id);
    }

    /// Los menús que se mostraron, en orden.
    pub fn menus(&self) -> Vec<Vec<Entry>> {
        self.menus.borrow().clone()
    }

    /// Los textos del último cuadro dibujado, en orden.
    pub fn drawn_texts(&self) -> Vec<(FontId, String, Rect)> {
        self.drawn.borrow().clone()
    }

    /// Los títulos que se pusieron, en orden.
    pub fn titles(&self) -> Vec<String> {
        self.titles.borrow().clone()
    }

    /// Cuántas veces se pidió repintar.
    pub fn invalidations(&self) -> usize {
        self.invalidations.get()
    }

    /// Lo que devuelve el selector de color (`None`: se canceló).
    pub fn set_color_choice(&self, c: Option<u32>) {
        self.color_choice.set(c);
    }

    /// La respuesta a `ask_yes_no_cancel` (`None`: cancelar).
    pub fn set_answer(&self, a: Option<bool>) {
        self.answer.set(a);
    }

    /// Cuántas veces sonó el pitido de "no se puede".
    pub fn beeps(&self) -> usize {
        self.beeps.get()
    }

    /// La ventana que recibió `attach` (`None` si todavía no se llamó).
    pub fn attached(&self) -> Option<Handle> {
        self.attached.get()
    }

    /// Lo que devuelve `start_drag` (por defecto, `Ok`).
    pub fn set_drag_result(&self, r: Result<(), String>) {
        *self.drag_result.borrow_mut() = r;
    }
}

impl WindowHost for FakeHost {
    fn attach(&self, window: Handle) {
        self.attached.set(Some(window));
    }

    fn invalidate(&self) {
        self.invalidations.set(self.invalidations.get() + 1);
    }

    fn client_size(&self) -> (f32, f32) {
        self.client.get()
    }

    fn cursor_dips(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn set_title(&self, title: &str) {
        self.titles.borrow_mut().push(title.to_string());
    }

    fn request_close(&self) {}
    fn focus_main(&self) {}
    fn focus_main_if_lost(&self) {}
    fn capture_mouse(&self) {}
    fn release_mouse(&self) {}
    fn track_mouse_leave(&self) {}

    fn beep(&self) {
        self.beeps.set(self.beeps.get() + 1);
    }

    fn ask_yes_no_cancel(&self, _title: &str, _text: &str) -> Option<bool> {
        self.answer.get()
    }

    fn open_url(&self, _url: &str) {}
    fn open_in_editor(&self, _path: &Path) {}

    fn wheel_lines(&self) -> f32 {
        3.0
    }

    /// Variables del entorno del proceso con la sintaxis de los dos sistemas (`%TEMP%`, `$TMPDIR`,
    /// `${HOME}`); las que no existen quedan como estaban.
    fn expand_env(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find(['%', '$']) {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            let (name, len) = match (&rest[i..i + 1], after.strip_prefix('{')) {
                ("%", _) => after.find('%').map_or(("", 0), |end| (&after[..end], end + 1)),
                (_, Some(inner)) => inner.find('}').map_or(("", 0), |end| (&inner[..end], end + 2)),
                _ => {
                    let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
                    (&after[..end], end)
                }
            };
            match std::env::var(name).ok().filter(|_| !name.is_empty()) {
                Some(v) => out.push_str(&v),
                None => out.push_str(&rest[i..i + 1 + len]),
            }
            rest = &after[len..];
        }
        out.push_str(rest);
        out
    }

    fn apply_theme(&self, _theme: &Theme, _follow_system: bool) -> bool {
        false
    }

    fn choose_color(&self, _current: u32) -> Option<u32> {
        self.color_choice.get()
    }

    fn start_drag(&self, _items: &[Item]) -> Result<(), String> {
        self.drag_result.borrow().clone()
    }

    fn placement(&self) -> Option<([i32; 4], bool)> {
        None
    }

    fn show(&self, _saved: Option<([i32; 4], bool)>) {}

    /// Dibuja en un `RecordingPainter` del tamaño de `client` y guarda sus textos.
    fn draw(&self, theme: &Theme, _file_colors: &[ColorRule], paint: &mut dyn FnMut(&dyn Painter, f32, f32)) -> Result<(), String> {
        let p = RecordingPainter::new(*theme);
        let (w, h) = self.client.get();
        paint(&p, w, h);
        *self.drawn.borrow_mut() = p.texts();
        Ok(())
    }

    fn text_width(&self, s: &str, _bold: bool) -> f32 {
        7.0 * s.chars().count() as f32
    }

    fn text_create(&self, kind: TextKind, _r: Rect, text: &str, _theme: &Theme) -> Option<TextBox> {
        let id = self.next_text_box.replace(self.next_text_box.get() + 1);
        self.text_boxes.borrow_mut().insert(id, (kind, text.to_string(), false));
        Some(TextBox(id))
    }

    fn text_get(&self, b: TextBox) -> String {
        self.text_boxes.borrow().get(&b.0).map(|(_, t, _)| t.clone()).unwrap_or_default()
    }

    fn text_set(&self, b: TextBox, text: &str) {
        if let Some((_, t, _)) = self.text_boxes.borrow_mut().get_mut(&b.0) {
            *t = text.to_string();
        }
    }

    fn text_select(&self, _b: TextBox, _from: usize, _to: isize) {}

    /// El foco pasa a esta caja (las demás lo pierden).
    fn text_focus(&self, b: TextBox) {
        for (id, (_, _, focused)) in self.text_boxes.borrow_mut().iter_mut() {
            *focused = *id == b.0;
        }
    }

    fn text_has_focus(&self, b: TextBox) -> bool {
        self.text_boxes.borrow().get(&b.0).is_some_and(|(_, _, f)| *f)
    }

    fn text_destroy(&self, b: TextBox) {
        self.text_boxes.borrow_mut().remove(&b.0);
    }

    fn text_restyle(&self, _theme: &Theme) {}

    fn popup_menu(&self, entries: &[Entry], _pos: MenuPos, _dot: Option<(u32, Color)>, _on_context: Option<crate::host::OnContext<'_>>) -> Option<u32> {
        self.menus.borrow_mut().push(entries.to_vec());
        self.menu_choice.get()
    }
}

// ---------- accesibilidad ----------

/// Un elemento de la lista de `FakeA11y`.
pub struct FakeA11yItem {
    pub name: String,
    pub is_dir: bool,
    pub selected: bool,
}

/// Un panel de `FakeA11y`: elementos, filtro rápido y foco.
pub struct FakeA11yPane {
    pub title: String,
    pub items: Vec<FakeA11yItem>,
    /// Filtro rápido: solo se ven los elementos cuyo nombre lo contiene.
    pub filter: String,
    /// Posición con foco, entre los que se ven.
    pub focus: Option<usize>,
}

/// Lado de cada panel (todos uno al lado del otro) y alto de las filas de la lista.
const A11Y_PANE_W: f32 = 200.0;
const A11Y_ROW_H: f32 = 24.0;
const A11Y_LIST_TOP: f32 = 30.0;

/// Una vista de accesibilidad en memoria: `n` paneles, todos a la vista, uno al lado del otro,
/// cada uno con los mismos elementos (en orden).
pub struct FakeA11y {
    pub panes: Vec<FakeA11yPane>,
    pub shown: Vec<usize>,
    pub active: usize,
    pub status: String,
    pub size: (f32, f32),
}

impl FakeA11y {
    pub fn new(panes: usize, items: &[&str]) -> FakeA11y {
        let panes = (0..panes)
            .map(|p| FakeA11yPane {
                title: format!("Carpeta {}", p + 1),
                items: items.iter().map(|n| FakeA11yItem { name: n.to_string(), is_dir: false, selected: false }).collect(),
                filter: String::new(),
                focus: None,
            })
            .collect::<Vec<_>>();
        let n = panes.len();
        FakeA11y { panes, shown: (0..n).collect(), active: 0, status: "Listo".into(), size: (A11Y_PANE_W * n.max(1) as f32, 600.0) }
    }

    /// Aplica el filtro rápido al panel (como al tipear en la lista).
    pub fn filter(&mut self, pane: usize, text: &str) {
        self.panes[pane].filter = text.to_string();
    }

    /// Índices (en `items`) de los elementos que se ven, en orden.
    fn visible(&self, pane: usize) -> Vec<usize> {
        let p = &self.panes[pane];
        (0..p.items.len()).filter(|&i| p.items[i].name.contains(&p.filter)).collect()
    }
}

impl crate::a11y::A11yView for FakeA11y {
    fn shown_panes(&self) -> Vec<usize> {
        self.shown.clone()
    }
    fn active(&self) -> usize {
        self.active
    }
    fn item_count(&self, pane: usize) -> usize {
        self.visible(pane).len()
    }
    fn focus_pos(&self, pane: usize) -> Option<usize> {
        self.panes[pane].focus
    }
    fn item_info(&self, pane: usize, pos: usize) -> Option<crate::a11y::ItemInfo> {
        let it = &self.panes[pane].items[*self.visible(pane).get(pos)?];
        let (_, _, list) = self.pane_rects(pane);
        let top = list.top + pos as f32 * A11Y_ROW_H;
        Some(crate::a11y::ItemInfo {
            name: it.name.clone(),
            kind: if it.is_dir { "Carpeta de archivos".into() } else { "Documento".into() },
            size: if it.is_dir { String::new() } else { "1 KB".into() },
            date: "01/01/2026".into(),
            is_dir: it.is_dir,
            selected: it.selected,
            rect: Rect { left: list.left, top, right: list.right, bottom: top + A11Y_ROW_H },
            offscreen: false,
        })
    }
    fn pane_title(&self, pane: usize) -> String {
        self.panes[pane].title.clone()
    }
    fn tab_pos(&self, _pane: usize) -> (usize, usize) {
        (0, 1)
    }
    fn address_text(&self, pane: usize) -> String {
        format!("/{}", self.panes[pane].title)
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn pane_rects(&self, pane: usize) -> (Rect, Rect, Rect) {
        let (left, right) = (pane as f32 * A11Y_PANE_W, (pane + 1) as f32 * A11Y_PANE_W);
        let bottom = self.size.1 - crate::ui::STATUSBAR_H;
        (
            Rect { left, top: 0.0, right, bottom },
            Rect { left, top: 0.0, right, bottom: A11Y_LIST_TOP },
            Rect { left, top: A11Y_LIST_TOP, right, bottom },
        )
    }
    fn client(&self) -> (f32, f32) {
        self.size
    }
    fn selected_positions(&self, pane: usize) -> Vec<usize> {
        let items = &self.panes[pane].items;
        self.visible(pane).iter().enumerate().filter(|(_, i)| items[**i].selected).map(|(pos, _)| pos).collect()
    }
    fn item_at(&self, pane: usize, _x: f32, y: f32) -> Option<usize> {
        let pos = ((y - A11Y_LIST_TOP) / A11Y_ROW_H).floor();
        (pos >= 0.0 && (pos as usize) < self.item_count(pane)).then_some(pos as usize)
    }
}

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

use super::{ConsoleInfo, DiskScan, FileOps, FileSystem, Handle, Info, LocalNow, Metadata, Overlays, Preview, Services, Shell, SystemEnv, Watch, Waker};
use crate::info::{InfoChange, InfoReply, InfoRequest, ItemInfo};
use crate::fs::{Entry as FsEntry, LoadEvent, LoadMsg, Location, Meta, MetaField, MetadataRequest, OverlayRequest, QuickItem, WM_APP_LOAD};
use crate::config::ColorRule;
use crate::format::{DAYS_1601_TO_1970, civil_from_days, size_kb_text, size_short_text};
use crate::geom::{Color, Rect};
use crate::host::{AuxWindow, MenuPos, TextBox, TextKind, WindowHost};
use crate::keys::Key;
use crate::ops::{Op, OpResult, WM_APP_OP_DONE};
use crate::paint::{FontId, Painter};
use crate::preview::{PreviewInfo, PreviewRequest, PreviewTarget};
use crate::shell::{Entry, Item};
use crate::theme::Theme;
use crate::thumbs;
use crate::xtree::scan::{DirListing, DriveEntry, FastScan, FileEntry, ListError, Lister, Progress};
use crate::xtree::tree::Node;

mod archive_fake;
mod copy_fake;
pub use archive_fake::FakeArchives;
pub use copy_fake::FakeCopyPrims;
mod net_fake;
pub use net_fake::FakeNetwork;

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
        metadata: Box::new(FakeMetadata),
        preview: Box::new(FakePreview),
        copy_prims: Box::new(FakeCopyPrims::shared()),
        archives: Box::new(FakeArchives::shared()),
        info: Box::new(FakeInfo),
        network: Box::new(FakeNetwork),
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
    /// Teclas que el falso informa como apretadas (por hilo).
    static KEYS_DOWN: RefCell<Vec<Key>> = const { RefCell::new(Vec::new()) };
    /// Fecha local que informa el falso (por hilo; ver `FakeEnv::set_local_now`).
    static LOCAL_NOW: Cell<LocalNow> = const { Cell::new(FakeEnv::DEFAULT_NOW) };
    /// Estilo de los atajos que informa el falso (por hilo; ver `FakeEnv::set_key_style`).
    static KEY_STYLE: Cell<super::KeyStyle> = const { Cell::new(super::KeyStyle::Windows) };
    static PORTABLE_SUPPORTED: Cell<bool> = const { Cell::new(true) };
}

/// `SystemEnv` falso: tema claro, sin animaciones, sin teclas apretadas salvo las de `hold_keys`, una carpeta de
/// configuración temporal por hilo, formatos deterministas y una sola consola.
pub struct FakeEnv;

impl FakeEnv {
    /// Desde ahora (en este hilo) `key_down` informa como apretadas solo estas teclas; `&[]` las suelta.
    pub fn hold_keys(keys: &[Key]) {
        KEYS_DOWN.with(|k| *k.borrow_mut() = keys.to_vec());
    }

    /// Fecha fija de `local_now` si no se configura otra: miércoles 11/11/2026, en UTC.
    pub const DEFAULT_NOW: LocalNow = LocalNow { day: 20_768, weekday: 2, month: 11, year: 2026, offset: 0 };

    /// Desde ahora (en este hilo) `key_style` devuelve este estilo (por defecto, Windows).
    pub fn set_key_style(style: super::KeyStyle) {
        KEY_STYLE.with(|k| k.set(style));
    }

    /// Desde ahora (en este hilo) `portable_supported` devuelve esto (por defecto, sí).
    pub fn set_portable_supported(on: bool) {
        PORTABLE_SUPPORTED.with(|p| p.set(on));
    }

    /// Desde ahora (en este hilo) `local_now` devuelve esta fecha.
    pub fn set_local_now(now: LocalNow) {
        LOCAL_NOW.with(|n| n.set(now));
    }
}

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

    /// Una por proceso de pruebas, bajo la temporal del sistema.
    fn archive_temp_dir(&self) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("xdir-comprimidos-pruebas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    fn key_down(&self, k: Key) -> bool {
        KEYS_DOWN.with(|d| d.borrow().contains(&k))
    }

    fn key_style(&self) -> super::KeyStyle {
        KEY_STYLE.with(Cell::get)
    }

    fn portable_supported(&self) -> bool {
        PORTABLE_SUPPORTED.with(Cell::get)
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

    /// Fija y configurable (`FakeEnv::set_local_now`), para que los grupos por fecha no dependan
    /// del día en que corren los tests.
    fn local_now(&self) -> LocalNow {
        LOCAL_NOW.with(|n| n.get())
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
/// Contenido de las carpetas virtuales falsas, por nombre de análisis (ver `set_shell_items`).
static SHELL_ITEMS: Mutex<BTreeMap<String, Vec<String>>> = Mutex::new(BTreeMap::new());

thread_local! {
    /// Volumen del falso en este hilo: espacio (total, libre) y si la raíz es un volumen entero;
    /// sin datos salvo `FakeDiskScan::set_volume`.
    static VOLUME: Cell<(Option<(u64, u64)>, bool)> = const { Cell::new((None, false)) };
}

/// `DiskScan` falso: nunca toca el disco. El listador sirve las carpetas cargadas con `serve` (las
/// demás, vacías); sin escaneo rápido ni elevado, sin discos ni datos de volumen (salvo los de
/// `set_volume`, por hilo), y la raíz por defecto es una carpeta temporal. Faltan permisos solo si
/// el `Progress` lo dice (como en Windows y macOS juntos: `need_admin` o carpetas denegadas).
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

    /// Datos del volumen que ve este hilo para cualquier ruta: espacio (total, libre) y si la ruta
    /// es un volumen entero. `set_volume(None, false)` vuelve a no haber datos.
    pub fn set_volume(space: Option<(u64, u64)>, whole: bool) {
        VOLUME.with(|v| v.set((space, whole)));
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

    fn volume_space(&self, path: &Path) -> Option<(u64, u64)> {
        FakeNetwork::note_io("volume_space", path);
        VOLUME.with(|v| v.get().0)
    }

    fn whole_volume(&self, _path: &Path) -> bool {
        VOLUME.with(|v| v.get().1)
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
    /// Lo que responde `menu_has_new` (de fábrica, no: como la Mac y Linux).
    menu_has_new: bool,
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

/// Originales de los accesos directos del falso (de todo el proceso: `link_target` corre en un
/// hilo de trabajo); cada test usa rutas propias.
static LINK_TARGETS: Mutex<BTreeMap<PathBuf, Option<PathBuf>>> = Mutex::new(BTreeMap::new());
/// Discos expulsables del falso, resultado de expulsar cada uno y los que se expulsaron (de todo
/// el proceso: se consultan desde hilos de trabajo).
static EJECTABLE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
type EjectResult = Result<(), crate::shell::EjectError>;
static EJECT_RESULTS: Mutex<BTreeMap<PathBuf, EjectResult>> = Mutex::new(BTreeMap::new());
static EJECTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Carpetas solo en la nube para todos los hilos (`FakeFileSystem::set_cloud_only_everywhere`).
static CLOUD_ALL: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
/// Disco de cada ruta para `volume_id` (todos los hilos; sin dato, `None`).
static VOLUME_IDS: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());
/// Enlaces a carpetas del falso (`set_dir_link`).
static DIR_LINKS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
/// Puntos de montaje del falso (`set_mount_point`).
static MOUNTS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

impl FakeShell {
    /// Estos discos se pueden expulsar (se suman a los de otros tests).
    pub fn set_ejectable(roots: &[PathBuf]) {
        EJECTABLE.lock().unwrap_or_else(|e| e.into_inner()).extend(roots.iter().cloned());
    }

    /// Lo que devuelve `eject(root)` (por defecto, éxito).
    pub fn set_eject_result(root: &Path, result: Result<(), crate::shell::EjectError>) {
        EJECT_RESULTS.lock().unwrap_or_else(|e| e.into_inner()).insert(root.to_path_buf(), result);
    }

    /// Todos los discos que se pidió expulsar (de todo el proceso).
    pub fn ejects() -> Vec<PathBuf> {
        EJECTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Desde ahora `link_target(link)` devuelve `target`.
    pub fn set_link_target(link: &Path, target: Option<PathBuf>) {
        LINK_TARGETS.lock().unwrap_or_else(|e| e.into_inner()).insert(link.to_path_buf(), target);
    }

    /// Las llamadas hechas desde este hilo hasta ahora (y vacía el registro).
    pub fn take_calls() -> Vec<ShellCall> {
        shell_state(|s| std::mem::take(&mut s.calls))
    }

    /// El menú del fondo del sistema trae «Nuevo» (como Windows) o no.
    pub fn set_menu_has_new(on: bool) {
        shell_state(|s| s.menu_has_new = on);
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
    /// Como Windows (el entorno falso también usa el estilo de Windows).
    fn link_style(&self) -> crate::names::LinkStyle {
        crate::names::LinkStyle::Lnk
    }

    fn link_target(&self, path: &Path) -> Option<PathBuf> {
        LINK_TARGETS.lock().unwrap_or_else(|e| e.into_inner()).get(path).cloned().flatten()
    }

    fn ejectable(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        let set = EJECTABLE.lock().unwrap_or_else(|e| e.into_inner());
        roots.iter().filter(|r| set.contains(*r)).cloned().collect()
    }

    fn eject(&self, root: &Path) -> Result<(), crate::shell::EjectError> {
        EJECTS.lock().unwrap_or_else(|e| e.into_inner()).push(root.to_path_buf());
        EJECT_RESULTS.lock().unwrap_or_else(|e| e.into_inner()).get(root).cloned().unwrap_or(Ok(()))
    }

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

    fn menu_has_new(&self) -> bool {
        shell_state(|s| s.menu_has_new)
    }

    fn volume_of(&self, path: &Path) -> Option<OsString> {
        FakeNetwork::note_io("volume_of", path);
        path.components().next().map(|c| c.as_os_str().to_ascii_lowercase())
    }
}

/// Lecturas pedidas al `FileSystem` falso y guardianes creados y soltados, por ruta. Son de todo
/// el proceso (las lecturas corren en sus propios hilos): cada test usa rutas propias.
static LOADS: Mutex<BTreeMap<PathBuf, usize>> = Mutex::new(BTreeMap::new());
static WATCHES: Mutex<BTreeMap<PathBuf, (usize, usize)>> = Mutex::new(BTreeMap::new());
/// Subcarpetas de cada carpeta virtual (y cuántas veces se pidieron), de todo el proceso.
static SHELL_SUBFOLDERS: Mutex<BTreeMap<String, (usize, Vec<(String, PathBuf)>)>> = Mutex::new(BTreeMap::new());
/// El nombre de análisis de la Red del falso cuando está prendida (`set_network_place`).
pub const FAKE_NETWORK_PLACE: &str = "::{RED-FALSA}";
thread_local! {
    static NET_PLACE: Cell<bool> = const { Cell::new(false) };
}
/// Cargas que fallan con ese texto (ver `FakeFileSystem::set_load_error`).
static LOAD_ERRORS: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());

thread_local! {
    /// Unidades del falso en este hilo (ruta y nombre); vacío salvo `FakeFileSystem::set_drives`.
    static DRIVES: RefCell<Vec<(PathBuf, String)>> = const { RefCell::new(Vec::new()) };
    /// Rutas que el falso informa como «solo en la nube» (por hilo; ver `set_cloud_only`).
    static CLOUD: RefCell<BTreeSet<PathBuf>> = const { RefCell::new(BTreeSet::new()) };
    /// Cuántas veces se preguntó `cloud_only` en este hilo.
    static CLOUD_CHECKS: Cell<usize> = const { Cell::new(0) };
    /// Carpeta que el falso trata como la Papelera (por hilo; ver `set_trash`).
    static TRASH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
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

    /// Desde ahora (en este hilo) `is_trash` dice que sí para esa carpeta (o para ninguna).
    pub fn set_trash(path: Option<&Path>) {
        TRASH.with(|t| *t.borrow_mut() = path.map(Path::to_path_buf));
    }

    /// Cuántas veces se preguntó `cloud_only` en este hilo (las vistas rápidas lo preguntan por
    /// `sys::preview()`, nunca al dibujar).
    pub fn cloud_checks() -> usize {
        CLOUD_CHECKS.with(Cell::get)
    }

    /// Desde ahora (en este hilo) `cloud_only(path)` dice `cloud`.
    pub fn set_cloud_only(path: &Path, cloud: bool) {
        CLOUD.with(|c| {
            let mut c = c.borrow_mut();
            if cloud {
                c.insert(path.to_path_buf());
            } else {
                c.remove(path);
            }
        });
    }

    /// Desde ahora `is_dir_link(path)` dice que sí (en todos los hilos).
    pub fn set_dir_link(path: &Path) {
        DIR_LINKS.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf());
    }

    /// Desde ahora `is_mount_point(path)` dice que sí (en todos los hilos).
    pub fn set_mount_point(path: &Path) {
        MOUNTS.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf());
    }

    /// Desde ahora `volume_id(path)` es `id` (en todos los hilos).
    pub fn set_volume_id(path: &Path, id: u64) {
        VOLUME_IDS.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), id);
    }

    /// Desde ahora `cloud_only(path)` dice que sí en todos los hilos (los hilos de trabajo también
    /// lo ven; cada test usa rutas propias).
    pub fn set_cloud_only_everywhere(path: &Path) {
        CLOUD_ALL.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf());
    }

    /// Desde ahora (en este hilo) hay Red del Shell (`network_place` no vacío), como en Windows.
    pub fn set_network_place(on: bool) {
        NET_PLACE.with(|c| c.set(on));
    }

    /// Las subcarpetas de la carpeta virtual `parsing` y cuántas veces se pidieron.
    pub fn set_shell_subfolders(parsing: &str, items: Vec<(String, PathBuf)>) {
        SHELL_SUBFOLDERS.lock().unwrap_or_else(|e| e.into_inner()).insert(parsing.to_string(), (0, items));
    }

    pub fn shell_subfolder_calls(parsing: &str) -> usize {
        SHELL_SUBFOLDERS.lock().unwrap_or_else(|e| e.into_inner()).get(parsing).map_or(0, |e| e.0)
    }

    /// Desde ahora (en todo el proceso) la carga de `path` falla con `error` (o vuelve a leer el
    /// disco con `None`).
    pub fn set_load_error(path: &Path, error: Option<&str>) {
        let mut m = LOAD_ERRORS.lock().unwrap_or_else(|e| e.into_inner());
        match error {
            Some(e) => m.insert(path.to_path_buf(), e.to_string()),
            None => m.remove(path),
        };
    }

    /// Cuántas lecturas de `path` se pidieron (en todo el proceso).
    pub fn loads(path: &Path) -> usize {
        LOADS.lock().unwrap_or_else(|e| e.into_inner()).get(path).copied().unwrap_or(0)
    }

    /// Lo que lista la carpeta virtual `parsing` (archivos con esos nombres para mostrar, como
    /// los de la Papelera: el nombre de análisis no es el que se ve). Sin esto, abrir una
    /// carpeta virtual da error.
    pub fn set_shell_items(parsing: &str, names: &[&str]) {
        SHELL_ITEMS.lock().unwrap_or_else(|e| e.into_inner()).insert(parsing.to_string(), names.iter().map(|n| n.to_string()).collect());
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
            // La fecha real del disco (en unidades de 100 ns): al cambiar lo de adentro de una
            // carpeta, cambia la suya.
            modified: meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| (d.as_nanos() / 100) as u64 + 116_444_736_000_000_000),
            created: 0,
            attrs,
            type_name: Arc::from(if is_dir { "Carpeta" } else { "Archivo" }),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
            packed: 0,
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
        // La pestaña Configuración y la de Novedades nunca se cargan: si llegan, quedan anotadas como
        // "xdir:configuracion" y "xdir:novedades".
        let counted = match &location {
            Location::Path(p) => Some(p.clone()),
            Location::Settings => Some(PathBuf::from("xdir:configuracion")),
            Location::Changelog => Some(PathBuf::from("xdir:novedades")),
            _ => None,
        };
        if let Some(p) = counted {
            *LOADS.lock().unwrap_or_else(|e| e.into_inner()).entry(p).or_insert(0) += 1;
        }
        std::thread::spawn(move || {
            let send = |msg: LoadMsg| {
                if tx.send(LoadEvent { panel, generation, msg }).is_ok() {
                    hwnd.post(WM_APP_LOAD, panel, 0);
                }
            };
            match &location {
                Location::Path(p) => {
                    // Una carpeta de red colgada (`FakeNetwork::hang`) no responde por un rato.
                    if let Some(d) = FakeNetwork::hang_for(&p.to_string_lossy()) {
                        std::thread::sleep(d);
                    }
                    let error = LOAD_ERRORS.lock().unwrap_or_else(|e| e.into_inner()).get(p).cloned();
                    match error {
                        Some(e) => send(LoadMsg::Listed(Err(e))),
                        None => fake_list(p, || current.load(Ordering::Relaxed) != generation, send),
                    }
                }
                // "Este equipo" del falso: vacío (sin unidades ni carpetas del usuario).
                Location::Computer | Location::Settings | Location::Changelog => send(LoadMsg::Listed(Ok(()))),
                // Lo lista el núcleo (`archive_nav`), nunca el sistema.
                Location::Archive { .. } => send(LoadMsg::Listed(Err("Esta ubicación no existe".into()))),
                Location::Shell { parsing, .. } => {
                    let items = SHELL_ITEMS.lock().unwrap_or_else(|e| e.into_inner()).get(parsing).cloned();
                    let Some(names) = items else { return send(LoadMsg::Listed(Err("Esta ubicación no existe".into()))) };
                    let batch: Vec<FsEntry> = names
                        .iter()
                        .enumerate()
                        .map(|(i, display)| FsEntry {
                            name: format!("$R{i:04}").into(),
                            full: Some(format!("{parsing}\\$R{i:04}").into()),
                            group: 0,
                            pidl: None,
                            sort_key: display.to_lowercase(),
                            display: display.clone(),
                            is_dir: false,
                            size: 0,
                            modified: 0,
                            created: 0,
                            attrs: 0,
                            type_name: Arc::from("Archivo"),
                            icon: 0,
                            overlay: 0,
                            overlay_checked: true,
                            packed: 0,
                        })
                        .collect();
                    if !batch.is_empty() {
                        send(LoadMsg::Entries(batch));
                    }
                    send(LoadMsg::Listed(Ok(())));
                }
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

    fn is_mount_point(&self, path: &Path) -> bool {
        MOUNTS.lock().unwrap_or_else(|e| e.into_inner()).contains(path)
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
        FakeNetwork::note_io("drive_label", Path::new(drive));
        DRIVES.with(|d| d.borrow().iter().find(|(p, _)| p.to_string_lossy() == drive).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| drive.to_string())
    }

    fn forget_drive_labels(&self) {}

    fn drive_missing(&self, path: &Path) -> bool {
        FakeNetwork::note_io("drive_missing", path);
        false
    }

    /// Ninguna raíz tiene nombre propio: se muestra el texto de la ruta.
    fn root_title(&self, _path: &Path) -> Option<String> {
        None
    }

    fn subfolders(&self, loc: &Location) -> Vec<(String, Location)> {
        let Location::Path(dir) = loc else { return Vec::new() };
        FakeNetwork::note_io("subfolders", dir);
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

    fn is_trash(&self, loc: &Location) -> bool {
        TRASH.with(|t| t.borrow().as_deref().is_some_and(|p| loc.is_path(p)))
    }

    fn network_place(&self) -> &'static str {
        if NET_PLACE.with(Cell::get) { FAKE_NETWORK_PLACE } else { "" }
    }

    fn linux_place(&self) -> &'static str {
        ""
    }

    fn shell_place_exists(&self, _parsing: &str) -> bool {
        false
    }

    fn shell_subfolders(&self, parsing: &str) -> Vec<(String, PathBuf)> {
        let mut m = SHELL_SUBFOLDERS.lock().unwrap_or_else(|e| e.into_inner());
        let e = m.entry(parsing.to_string()).or_insert((0, Vec::new()));
        e.0 += 1;
        e.1.clone()
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
        FakeNetwork::note_io("long_path", path);
        path.to_path_buf()
    }

    fn true_case(&self, path: &Path) -> PathBuf {
        FakeNetwork::note_io("true_case", path);
        path.to_path_buf()
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
        FakeNetwork::note_io("watch", path);
        // Una carpeta «de red» del falso (`FakeNetwork::set_remote`) se vigila aunque no exista.
        if !path.is_dir() && !crate::sys::network().is_remote(path) {
            return None;
        }
        WATCHES.lock().unwrap_or_else(|e| e.into_inner()).entry(path.to_path_buf()).or_insert((0, 0)).0 += 1;
        Some(Box::new(FakeWatch(path.to_path_buf())))
    }

    /// Lo puesto con `set_dir_link`.
    fn is_dir_link(&self, path: &Path) -> bool {
        DIR_LINKS.lock().unwrap_or_else(|e| e.into_inner()).contains(path)
    }

    /// Lo puesto con `set_volume_id`.
    fn volume_id(&self, path: &Path) -> Option<u64> {
        VOLUME_IDS.lock().unwrap_or_else(|e| e.into_inner()).get(path).copied()
    }

    /// Lo marcado con `set_cloud_only` en este hilo o con `set_cloud_only_everywhere`.
    fn cloud_only(&self, path: &Path) -> bool {
        FakeNetwork::note_io("cloud_only", path);
        CLOUD_CHECKS.with(|n| n.set(n.get() + 1));
        CLOUD.with(|c| c.borrow().contains(path)) || CLOUD_ALL.lock().unwrap_or_else(|e| e.into_inner()).contains(path)
    }

    /// Sobre el disco real (las carpetas temporales de cada test), sin pisar: falla si `to` existe
    /// o si está en `fail_rename_to`.
    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), String> {
        if RENAME_FAILS.lock().unwrap_or_else(|e| e.into_inner()).contains(to) {
            return Err(format!("No se pudo renombrar «{}»", from.display()));
        }
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(format!("Ya existe «{}»", to.display()));
        }
        std::fs::rename(from, to).map_err(|e| crate::rename_many::io_message(&e, from))
    }
}

/// Destinos con los que `FakeFileSystem::rename_noreplace` falla (de todo el proceso: renombra
/// desde hilos de trabajo; cada test usa rutas propias).
static RENAME_FAILS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

impl FakeFileSystem {
    /// Desde ahora, renombrar algo a `to` falla.
    pub fn fail_rename_to(to: &Path) {
        RENAME_FAILS.lock().unwrap_or_else(|e| e.into_inner()).insert(to.to_path_buf());
    }
}

/// Guardián del `FileSystem` falso: al soltarlo, cuenta una baja para su carpeta.
pub struct FakeWatch(PathBuf);

impl Watch for FakeWatch {}

impl Drop for FakeWatch {
    fn drop(&mut self) {
        // Soltar el guardián de una carpeta de red colgada espera al servidor (como Windows).
        if let Some(d) = FakeNetwork::hang_for(&self.0.to_string_lossy()) {
            std::thread::sleep(d);
        }
        WATCHES.lock().unwrap_or_else(|e| e.into_inner()).entry(self.0.clone()).or_insert((0, 0)).1 += 1;
    }
}

/// Un pedido de metadatos visto por el falso: (índice, ruta) de cada elemento y campos pedidos.
pub type MetaRequestRecord = (Vec<(usize, PathBuf)>, Vec<MetaField>);

thread_local! {
    /// Metadatos que devuelve el falso, por ruta (por hilo de prueba).
    static META_DATA: RefCell<HashMap<PathBuf, Meta>> = RefCell::new(HashMap::new());
    /// Pedidos recibidos: (índice, ruta) de cada elemento y campos pedidos.
    static META_REQUESTS: RefCell<Vec<MetaRequestRecord>> = const { RefCell::new(Vec::new()) };
    /// Demora de la respuesta: `None` responde en el mismo hilo, al pedir.
    static META_DELAY: Cell<Option<std::time::Duration>> = const { Cell::new(None) };
}

/// `Metadata` falso: anota cada pedido y responde con lo cargado con `serve` para cada ruta (o
/// vacío), en un solo lote, en el mismo hilo o, con `set_delay`, desde otro hilo después de la
/// demora. Todo es por hilo de prueba.
pub struct FakeMetadata;

impl FakeMetadata {
    /// Desde ahora (en este hilo) la ruta tiene estos metadatos.
    pub fn serve(path: &Path, meta: Meta) {
        META_DATA.with(|d| d.borrow_mut().insert(path.to_path_buf(), meta));
    }

    /// Pedidos recibidos hasta ahora (y los olvida): elementos y campos de cada uno.
    pub fn take_requests() -> Vec<MetaRequestRecord> {
        META_REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    /// Demora de las respuestas (`None`: en el mismo hilo, al pedir).
    pub fn set_delay(delay: Option<std::time::Duration>) {
        META_DELAY.with(|d| d.set(delay));
    }
}

impl Metadata for FakeMetadata {
    fn request(&self, req: MetadataRequest) {
        META_REQUESTS.with(|r| r.borrow_mut().push((req.items.clone(), req.fields.clone())));
        let batch: Vec<(usize, Meta)> =
            META_DATA.with(|d| req.items.iter().map(|(i, p)| (*i, d.borrow().get(p).cloned().unwrap_or_default())).collect());
        match META_DELAY.with(Cell::get) {
            None => {
                req.send(batch);
            }
            Some(delay) => {
                std::thread::spawn(move || {
                    std::thread::sleep(delay);
                    req.send(batch);
                });
            }
        }
    }
}

thread_local! {
    /// Vistas previas que devuelve el falso, por ruta (por hilo de prueba).
    static PREVIEW_DATA: RefCell<HashMap<PathBuf, PreviewInfo>> = RefCell::new(HashMap::new());
    /// Pedidos recibidos.
    static PREVIEW_REQUESTS: RefCell<Vec<PreviewTarget>> = const { RefCell::new(Vec::new()) };
    /// Demora de la respuesta: `None` responde en el mismo hilo, al pedir.
    static PREVIEW_DELAY: Cell<Option<std::time::Duration>> = const { Cell::new(None) };
}

/// `Preview` falso: anota cada pedido y responde con lo cargado con `serve` para su ruta o, si no
/// hay nada cargado, con la lectura real (`preview::read`, con la nube de `FakeFileSystem`). La
/// lectura se hace al pedir, en el hilo del test; la respuesta sale en ese momento o, con
/// `set_delay`, desde otro hilo después de la demora (y si el pedido quedó viejo, no sale). Todo
/// es por hilo de prueba.
pub struct FakePreview;

impl FakePreview {
    /// Desde ahora (en este hilo) la ruta tiene esta vista previa.
    pub fn serve(path: &Path, info: PreviewInfo) {
        PREVIEW_DATA.with(|d| d.borrow_mut().insert(path.to_path_buf(), info));
    }

    /// Pedidos recibidos hasta ahora (y los olvida).
    pub fn take_requests() -> Vec<PreviewTarget> {
        PREVIEW_REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    /// Demora de las respuestas (`None`: en el mismo hilo, al pedir).
    pub fn set_delay(delay: Option<std::time::Duration>) {
        PREVIEW_DELAY.with(|d| d.set(delay));
    }
}

impl Preview for FakePreview {
    fn request(&self, req: PreviewRequest) {
        PREVIEW_REQUESTS.with(|r| r.borrow_mut().push(req.target.clone()));
        let served = PREVIEW_DATA.with(|d| d.borrow().get(req.target.path()).cloned());
        let info = served.unwrap_or_else(|| {
            let fs = FakeFileSystem;
            crate::preview::read(&req.target, &|p| fs.cloud_only(p))
        });
        match PREVIEW_DELAY.with(Cell::get) {
            None => req.send(info),
            Some(delay) => {
                std::thread::spawn(move || {
                    std::thread::sleep(delay);
                    req.send(info);
                });
            }
        }
    }
}

thread_local! {
    /// Información que devuelve el falso, por ruta (por hilo de prueba).
    static INFO_DATA: RefCell<HashMap<PathBuf, ItemInfo>> = RefCell::new(HashMap::new());
    /// Pedidos recibidos (las rutas de cada uno).
    static INFO_REQUESTS: RefCell<Vec<Vec<PathBuf>>> = const { RefCell::new(Vec::new()) };
    /// Cambios aplicados.
    static INFO_APPLIED: RefCell<Vec<InfoChange>> = const { RefCell::new(Vec::new()) };
    /// Error con que responden los cambios (`None`: bien).
    static INFO_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    /// ¿Hay diálogo del sistema? Y cuántas veces se abrió.
    static INFO_MORE: Cell<bool> = const { Cell::new(true) };
    static INFO_MORE_CALLS: RefCell<Vec<Vec<PathBuf>>> = const { RefCell::new(Vec::new()) };
}

/// `Info` falso: anota cada pedido y responde al pedir, en el hilo del test, con lo cargado con
/// `serve` para cada ruta o, si no hay nada, con la lectura mínima del núcleo (con la nube de
/// `FakeFileSystem`, que no abre nada). Los cambios se anotan y responden con `fail_with`.
pub struct FakeInfo;

impl FakeInfo {
    pub fn serve(path: &Path, info: ItemInfo) {
        INFO_DATA.with(|d| d.borrow_mut().insert(path.to_path_buf(), info));
    }

    /// Pedidos recibidos hasta ahora (y los olvida).
    pub fn take_requests() -> Vec<Vec<PathBuf>> {
        INFO_REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    /// Cambios aplicados hasta ahora (y los olvida).
    pub fn take_applied() -> Vec<InfoChange> {
        INFO_APPLIED.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    /// Desde ahora los cambios fallan con este texto (`None`: andan).
    pub fn fail_with(error: Option<&str>) {
        INFO_ERROR.with(|e| *e.borrow_mut() = error.map(str::to_string));
    }

    /// ¿El sistema tiene diálogo propio? (por defecto, sí).
    pub fn set_more_in_system(on: bool) {
        INFO_MORE.with(|m| m.set(on));
    }

    /// Veces que se abrió el diálogo del sistema (y las olvida).
    pub fn take_more_calls() -> Vec<Vec<PathBuf>> {
        INFO_MORE_CALLS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }
}

impl Info for FakeInfo {
    fn request(&self, req: InfoRequest) {
        INFO_REQUESTS.with(|r| r.borrow_mut().push(req.paths.clone()));
        let fs = FakeFileSystem;
        let items = req
            .paths
            .iter()
            .map(|p| INFO_DATA.with(|d| d.borrow().get(p).cloned()).unwrap_or_else(|| crate::info::read_minimal(p, &|x| fs.cloud_only(x))))
            .collect();
        req.send(items);
    }

    fn apply(&self, owner: Handle, tx: Sender<InfoReply>, generation: u64, change: InfoChange) {
        let section = change.section();
        INFO_APPLIED.with(|r| r.borrow_mut().push(change));
        let error = INFO_ERROR.with(|e| e.borrow().clone());
        crate::info::send_applied(owner, &tx, generation, section, error);
    }

    fn more_in_system(&self, _owner: Handle, items: &[Item]) -> bool {
        INFO_MORE_CALLS.with(|r| r.borrow_mut().push(paths(items)));
        INFO_MORE.with(Cell::get)
    }

    fn has_more_in_system(&self) -> bool {
        INFO_MORE.with(Cell::get)
    }
}

/// `Overlays` falso: no hay superposiciones (no responde nada).
pub struct FakeOverlays;

impl Overlays for FakeOverlays {
    fn request(&self, _req: OverlayRequest) {}
}

/// `Painter` que no dibuja: registra cada `text` (fuente, texto y rectángulo) para que los tests
/// vean qué se escribió y con qué formato. `measure` da 7 DIPs por carácter; `clip` solo llama a
/// la clausura; los íconos se anotan (`icons`) y las miniaturas no hacen nada (`thumbnail` da `false`). `big_image` anota la
/// clave (`big_requests`) y devuelve lo fijado con `set_big_ready` (de entrada, `false`).
pub struct RecordingPainter {
    theme: Theme,
    texts: RefCell<Vec<(FontId, String, Rect)>>,
    paths: RefCell<Vec<RecordedPath>>,
    strokes: RefCell<Vec<(Rect, Color)>>,
    big: RefCell<Vec<(thumbs::Key, Rect)>>,
    big_ready: Cell<bool>,
    forgotten: RefCell<Vec<thumbs::Source>>,
    icons: RefCell<Vec<(f32, f32, f32)>>,
    fills: RefCell<Vec<(Rect, Color)>>,
    round_fills: RefCell<Vec<(Rect, Color)>>,
}

/// Trazo dibujado con `rounded_path`: puntos (sin radios), si es cerrado y su relleno.
pub type RecordedPath = (Vec<(f32, f32)>, bool, Option<Color>);

impl RecordingPainter {
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            texts: RefCell::new(Vec::new()),
            paths: RefCell::new(Vec::new()),
            strokes: RefCell::new(Vec::new()),
            big: RefCell::new(Vec::new()),
            big_ready: Cell::new(false),
            forgotten: RefCell::new(Vec::new()),
            icons: RefCell::new(Vec::new()),
            fills: RefCell::new(Vec::new()),
            round_fills: RefCell::new(Vec::new()),
        }
    }

    /// Los rectángulos rellenados con `round_fill` (rectángulo y color), en orden.
    pub fn round_fills(&self) -> Vec<(Rect, Color)> {
        self.round_fills.borrow().clone()
    }

    /// Los rectángulos rellenados con `fill` (rectángulo y color), en orden.
    pub fn fills(&self) -> Vec<(Rect, Color)> {
        self.fills.borrow().clone()
    }

    /// Claves pedidas a `big_image`, en orden.
    pub fn big_requests(&self) -> Vec<thumbs::Key> {
        self.big.borrow().iter().map(|(k, _)| k.clone()).collect()
    }

    /// Claves y rectángulos pedidos a `big_image`, en orden.
    pub fn big_draws(&self) -> Vec<(thumbs::Key, Rect)> {
        self.big.borrow().clone()
    }

    /// Fuentes cuyas imágenes grandes se olvidaron (`forget_big`), en orden.
    pub fn forgotten(&self) -> Vec<thumbs::Source> {
        self.forgotten.borrow().clone()
    }

    /// Lo que devuelve `big_image` desde ahora (si la imagen "está").
    pub fn set_big_ready(&self, ready: bool) {
        self.big_ready.set(ready);
    }

    /// Los trazos dibujados con `rounded_path`, en orden.
    pub fn paths(&self) -> Vec<RecordedPath> {
        self.paths.borrow().clone()
    }

    /// Los bordes dibujados con `round_stroke_w` (rectángulo y color), en orden.
    pub fn strokes(&self) -> Vec<(Rect, Color)> {
        self.strokes.borrow().clone()
    }

    /// Íconos dibujados con `shell_icon` (x, y, lado), en orden.
    pub fn icons(&self) -> Vec<(f32, f32, f32)> {
        self.icons.borrow().clone()
    }

    /// Los textos dibujados, en orden.
    pub fn texts(&self) -> Vec<(FontId, String, Rect)> {
        self.texts.borrow().clone()
    }
}

impl Painter for RecordingPainter {
    fn fill(&self, r: Rect, c: Color) {
        self.fills.borrow_mut().push((r, c));
    }
    fn round_fill(&self, r: Rect, _radius: f32, c: Color) {
        self.round_fills.borrow_mut().push((r, c));
    }
    fn rounded_path(&self, pts: &[(f32, f32, f32)], closed: bool, fill: Option<Color>, _stroke: Option<(Color, f32)>) {
        self.paths.borrow_mut().push((pts.iter().map(|&(x, y, _)| (x, y)).collect(), closed, fill));
    }
    fn circle(&self, _cx: f32, _cy: f32, _radius: f32, _fill: Color) {}
    fn circle_stroke(&self, _cx: f32, _cy: f32, _radius: f32, _c: Color, _width: f32) {}
    fn round_stroke_w(&self, r: Rect, _radius: f32, c: Color, _width: f32) {
        self.strokes.borrow_mut().push((r, c));
    }
    fn round_stroke(&self, _r: Rect, _radius: f32, _c: Color) {}

    fn text(&self, font: FontId, s: &str, r: Rect, _c: Color) {
        self.texts.borrow_mut().push((font, s.to_string(), r));
    }

    fn measure(&self, _font: FontId, s: &str) -> f32 {
        7.0 * s.chars().count() as f32
    }

    fn shell_icon(&self, _index: i32, _overlay: u8, x: f32, y: f32, size: f32) {
        self.icons.borrow_mut().push((x, y, size));
    }

    fn thumbnail(&self, _key: &thumbs::Key, _r: Rect) -> bool {
        false
    }

    fn big_image(&self, key: &thumbs::Key, r: Rect) -> bool {
        self.big.borrow_mut().push((key.clone(), r));
        self.big_ready.get()
    }

    fn forget_big(&self, src: &thumbs::Source) {
        self.forgotten.borrow_mut().push(src.clone());
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

/// Un pedido de `aux_show`: ventana, título y tamaño.
pub type AuxShow = (AuxWindow, String, (f32, f32));

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
    /// Preguntas de `ask_yes_no_cancel` (título y texto), en orden.
    questions: RefCell<Vec<(String, String)>>,
    drag_result: RefCell<Result<(), String>>,
    drawn: RefCell<Vec<(FontId, String, Rect)>>,
    drawn_fills: RefCell<Vec<(Rect, Color)>>,
    /// Cajas de texto abiertas: tipo, texto y si tienen el foco.
    text_boxes: RefCell<HashMap<u32, (TextKind, String, bool)>>,
    next_text_box: Cell<u32>,
    menu_choice: Cell<Option<u32>>,
    menus: RefCell<Vec<Vec<Entry>>>,
    /// La ventana que se le pasó a `attach` (la última).
    attached: Cell<Option<Handle>>,
    /// Imágenes grandes pedidas en el último cuadro dibujado.
    drawn_big: RefCell<Vec<thumbs::Key>>,
    /// Vista rápida: lo que se le pidió mostrar, cuántas veces se cerró y si tiene visor propio.
    quick_look_shown: RefCell<Vec<crate::quicklook::QuickLookItem>>,
    quick_look_closes: Cell<usize>,
    quick_look_native: Cell<bool>,
    quick_look_cloud: Cell<bool>,
    /// Ventanas secundarias: si el host las tiene (por defecto, sí), lo que se pidió mostrar
    /// (ventana, título, tamaño), los cambios de tamaño, los repintados, las que se ocultaron y si
    /// está a la vista.
    aux_native: Cell<bool>,
    aux_shows: RefCell<Vec<AuxShow>>,
    aux_sizes: RefCell<Vec<(AuxWindow, (f32, f32))>>,
    aux_invalidations: Cell<usize>,
    aux_hides: Cell<usize>,
    aux_visible: Cell<bool>,
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
            questions: RefCell::new(Vec::new()),
            drag_result: RefCell::new(Ok(())),
            drawn: RefCell::new(Vec::new()),
            drawn_fills: RefCell::new(Vec::new()),
            text_boxes: RefCell::new(HashMap::new()),
            next_text_box: Cell::new(1),
            menu_choice: Cell::new(None),
            menus: RefCell::new(Vec::new()),
            attached: Cell::new(None),
            drawn_big: RefCell::new(Vec::new()),
            quick_look_shown: RefCell::new(Vec::new()),
            quick_look_closes: Cell::new(0),
            quick_look_native: Cell::new(true),
            quick_look_cloud: Cell::new(false),
            aux_native: Cell::new(true),
            aux_shows: RefCell::new(Vec::new()),
            aux_sizes: RefCell::new(Vec::new()),
            aux_invalidations: Cell::new(0),
            aux_hides: Cell::new(0),
            aux_visible: Cell::new(false),
        })
    }

    /// Si el host tiene ventanas secundarias (por defecto, sí; sin ellas `aux_show` da `false`).
    pub fn set_aux_native(&self, on: bool) {
        self.aux_native.set(on);
    }

    /// Lo que se pidió mostrar con `aux_show` (ventana, título y tamaño), en orden.
    pub fn aux_shows(&self) -> Vec<AuxShow> {
        self.aux_shows.borrow().clone()
    }

    /// Los tamaños pedidos con `aux_resize`, en orden.
    pub fn aux_sizes(&self) -> Vec<(AuxWindow, (f32, f32))> {
        self.aux_sizes.borrow().clone()
    }

    /// Cuántas veces se pidió repintar una ventana secundaria.
    pub fn aux_invalidations(&self) -> usize {
        self.aux_invalidations.get()
    }

    /// Cuántas veces se ocultó una ventana secundaria desde la `App`.
    pub fn aux_hides(&self) -> usize {
        self.aux_hides.get()
    }

    /// La ventana secundaria está a la vista (`aux_show` sin `aux_hide` después).
    pub fn aux_visible(&self) -> bool {
        self.aux_visible.get()
    }

    /// El usuario cerró la ventana (lo que hace la plataforma antes de `App::aux_closed`).
    pub fn aux_user_closed(&self) {
        self.aux_visible.set(false);
    }

    /// Imágenes grandes pedidas en el último cuadro dibujado.
    pub fn drawn_big(&self) -> Vec<thumbs::Key> {
        self.drawn_big.borrow().clone()
    }

    /// Lo que se le pidió mostrar a la vista rápida, en orden.
    pub fn quick_look_shown(&self) -> Vec<crate::quicklook::QuickLookItem> {
        self.quick_look_shown.borrow().clone()
    }

    /// Cuántas veces se cerró la vista rápida desde la `App`.
    pub fn quick_look_closes(&self) -> usize {
        self.quick_look_closes.get()
    }

    /// Si el host tiene visor propio (por defecto, sí): sin él, `quick_look_show` devuelve `false`
    /// (y no anota nada) y la `App` dibuja su tarjeta.
    pub fn set_quick_look_native(&self, native: bool) {
        self.quick_look_native.set(native);
    }

    /// Si la ventana del host muestra también lo solo en la nube (Windows; por defecto, no).
    pub fn set_quick_look_shows_cloud(&self, on: bool) {
        self.quick_look_cloud.set(on);
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

    /// Los rectángulos rellenados del último cuadro dibujado.
    pub fn drawn_fills(&self) -> Vec<(Rect, Color)> {
        self.drawn_fills.borrow().clone()
    }

    /// Las cajas de texto abiertas de ese tipo (número y texto).
    pub fn text_boxes_of(&self, kind: TextKind) -> Vec<(TextBox, String)> {
        let mut v: Vec<(TextBox, String)> =
            self.text_boxes.borrow().iter().filter(|(_, (k, _, _))| *k == kind).map(|(id, (_, t, _))| (TextBox(*id), t.clone())).collect();
        v.sort_by_key(|(b, _)| b.0);
        v
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

    /// Las preguntas que se hicieron con `ask_yes_no_cancel` (título, texto).
    pub fn questions(&self) -> Vec<(String, String)> {
        self.questions.borrow().clone()
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

    fn ask_yes_no_cancel(&self, title: &str, text: &str) -> Option<bool> {
        self.questions.borrow_mut().push((title.to_string(), text.to_string()));
        self.answer.get()
    }

    /// Sí con `set_answer(Some(true))`; cualquier otra respuesta es No. Se anota en `questions`.
    fn ask_yes_no(&self, title: &str, text: &str) -> bool {
        self.questions.borrow_mut().push((title.to_string(), text.to_string()));
        self.answer.get() == Some(true)
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
        *self.drawn_fills.borrow_mut() = p.fills();
        *self.drawn_big.borrow_mut() = p.big_requests();
        Ok(())
    }

    fn quick_look_show(&self, item: &crate::quicklook::QuickLookItem) -> bool {
        if !self.quick_look_native.get() {
            return false;
        }
        self.quick_look_shown.borrow_mut().push(item.clone());
        true
    }

    fn quick_look_close(&self) {
        self.quick_look_closes.set(self.quick_look_closes.get() + 1);
    }

    fn quick_look_shows_cloud(&self) -> bool {
        self.quick_look_cloud.get()
    }

    fn aux_show(&self, w: AuxWindow, title: &str, size: (f32, f32)) -> bool {
        if !self.aux_native.get() {
            return false;
        }
        self.aux_shows.borrow_mut().push((w, title.to_string(), size));
        self.aux_visible.set(true);
        true
    }

    fn aux_set_title(&self, w: AuxWindow, title: &str) {
        if let Some(last) = self.aux_shows.borrow_mut().iter_mut().rev().find(|s| s.0 == w) {
            last.1 = title.to_string();
        }
    }

    fn aux_resize(&self, w: AuxWindow, size: (f32, f32)) {
        self.aux_sizes.borrow_mut().push((w, size));
    }

    fn aux_invalidate(&self, _w: AuxWindow) {
        self.aux_invalidations.set(self.aux_invalidations.get() + 1);
    }

    fn aux_hide(&self, _w: AuxWindow) {
        self.aux_hides.set(self.aux_hides.get() + 1);
        self.aux_visible.set(false);
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

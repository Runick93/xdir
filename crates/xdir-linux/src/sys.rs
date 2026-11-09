//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para Linux. `main` llama
//! `install` al arrancar, antes de crear la `App`.
//!
//! Por ahora `LinuxFileOps`, `LinuxDiskScan`, `LinuxShell` y `LinuxFileSystem` son mínimos: no
//! hacen nada o devuelven "No disponible en Linux", sin entrar en pánico.

use std::ffi::{OsStr, OsString};
use std::fs::{DirEntry, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::Sender;

use xdir_core::fs::{Entry as FsEntry, LoadEvent, LoadMsg, Location, OverlayRequest, QuickItem, WM_APP_LOAD};
use xdir_core::ops::{Op, OpResult, WM_APP_OP_DONE};
use xdir_core::shell::{Entry, Item};
use xdir_core::sys::{DiskScan, FileOps, FileSystem, Handle, Overlays, Services, Shell, Watch, Waker};
use xdir_core::xtree::scan::{DirListing, DriveEntry, FastScan, ListError, Lister, Progress};
use xdir_core::xtree::tree::Node;

use crate::env::LinuxEnv;

/// Lo que Linux todavía no puede hacer.
const UNAVAILABLE: &str = "No disponible en Linux";

/// Sin hilo de interfaz no hay a quién despertar: `post` no hace nada. Los resultados igual
/// quedan en sus canales; cuando haya ventana, este servicio la despierta.
struct LinuxWaker;

impl Waker for LinuxWaker {
    fn post(&self, _window: Handle, _msg: u32, _wparam: usize, _lparam: isize) {}
}

/// Linux no tiene superposiciones de íconos: el pedido no se responde.
struct LinuxOverlays;

impl Overlays for LinuxOverlays {
    fn request(&self, _req: OverlayRequest) {}
}

/// Operaciones de archivos: todavía no. Responde enseguida con el error.
struct LinuxFileOps;

impl FileOps for LinuxFileOps {
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, _op: Op) {
        let _ = tx.send(OpResult { error: Some(UNAVAILABLE.into()), ..Default::default() });
        owner.post(WM_APP_OP_DONE, 0, 0);
    }
}

/// Listador que no lista nada.
struct NoLister;

impl Lister for NoLister {
    fn list(&self, _path: &Path) -> Result<DirListing, ListError> {
        Err(ListError::Other)
    }
}

/// Escaneo de xtree: todavía no.
struct LinuxDiskScan;

impl DiskScan for LinuxDiskScan {
    fn lister(&self, _root: &Path) -> Box<dyn Lister> {
        Box::new(NoLister)
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

    fn whole_volume(&self, path: &Path) -> bool {
        path == Path::new("/")
    }

    fn default_root(&self) -> PathBuf {
        PathBuf::from("/")
    }

    fn permission_issue(&self, _p: &Progress) -> bool {
        false
    }

    fn permission_hint(&self) -> &'static str {
        ""
    }

    fn elevating_text(&self) -> &'static str {
        ""
    }
}

/// Integración con el escritorio: todavía no.
struct LinuxShell;

impl Shell for LinuxShell {
    fn copy_files(&self, _items: &[Item], _cut: bool) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn clipboard_files(&self) -> Option<(Vec<PathBuf>, bool)> {
        None
    }

    fn paste_virtual(&self, _owner: Handle, _dest: &Location) -> Result<bool, String> {
        Err(UNAVAILABLE.into())
    }

    fn clear_clipboard(&self) {}

    fn copy_text(&self, _owner: Handle, _text: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn pick_folder(&self, _owner: Handle, _title: &str) -> Option<PathBuf> {
        None
    }

    fn open_item(&self, _owner: Handle, _item: &Item) {}

    fn open_file(&self, _owner: Handle, _file: &Path, _dir: Option<&Path>) {}

    fn properties(&self, _owner: Handle, _items: &[Item]) {}

    fn context_menu(
        &self,
        _owner: Handle,
        _folder: &Location,
        _items: &[Item],
        _at: (f32, f32),
        _extended: bool,
        _intercept: bool,
        _own: &[Entry],
        _pinned: bool,
    ) -> Option<String> {
        None
    }

    fn invoke_verb(&self, _owner: Handle, _folder: &Location, _items: &[Item], _verb: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn pin_quick_access(&self, _owner: Handle, _path: &Path) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn unpin_quick_access(&self, _owner: Handle, _path: &Path) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn set_menu_mode(&self, _dark: Option<bool>) {}

    fn volume_of(&self, _path: &Path) -> Option<OsString> {
        None
    }
}

/// Sistema de archivos: todavía no se leen carpetas ni se vigilan.
struct LinuxFileSystem;

impl FileSystem for LinuxFileSystem {
    /// Responde enseguida que no se pudo listar.
    fn spawn_load(&self, hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, _current: Arc<AtomicU64>, _location: Location) {
        if tx.send(LoadEvent { panel, generation, msg: LoadMsg::Listed(Err(UNAVAILABLE.into())) }).is_ok() {
            hwnd.post(WM_APP_LOAD, panel, 0);
        }
    }

    fn with_search_thread(&self, f: Box<dyn FnOnce() + Send>) {
        f();
    }

    fn search_metadata(&self, item: &DirEntry, path: &Path) -> Option<Metadata> {
        std::fs::metadata(path).or_else(|_| item.metadata()).ok()
    }

    fn is_hidden(&self, _meta: &Metadata, name: &OsStr) -> bool {
        name.to_string_lossy().starts_with('.')
    }

    fn searchable_dir(&self, _name: &OsStr) -> bool {
        true
    }

    fn is_mount_point(&self, _path: &Path) -> bool {
        false
    }

    /// Carpeta o archivo, sin ícono ni fechas.
    fn disk_entry(&self, path: &Path, name: OsString, meta: &Metadata) -> FsEntry {
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
            full: Some(path.as_os_str().to_os_string()),
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

    fn user_folder(&self, _i: usize) -> Option<PathBuf> {
        None
    }

    fn drives(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn drive_name(&self, root: &Path) -> String {
        root.to_string_lossy().into_owned()
    }

    fn drive_label(&self, drive: &str) -> String {
        drive.to_string()
    }

    fn forget_drive_labels(&self) {}

    fn drive_missing(&self, _path: &Path) -> bool {
        false
    }

    fn root_title(&self, _path: &Path) -> Option<String> {
        None
    }

    fn subfolders(&self, _loc: &Location) -> Vec<(String, Location)> {
        Vec::new()
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

    /// En Linux las rutas distinguen mayúsculas: solo se ignora la barra final.
    fn same_path(&self, a: &Path, b: &Path) -> bool {
        a.components().eq(b.components())
    }

    fn watch(&self, _hwnd: Handle, _msg: u32, _panel: usize, _lparam: isize, _path: &Path) -> Option<Box<dyn Watch>> {
        None
    }
}

/// Instala los servicios de Linux en el registro del núcleo.
pub fn install() {
    xdir_core::sys::install(Services {
        waker: Box::new(LinuxWaker),
        env: Box::new(LinuxEnv),
        file_ops: Box::new(LinuxFileOps),
        disk_scan: Box::new(LinuxDiskScan),
        shell: Box::new(LinuxShell),
        file_system: Box::new(LinuxFileSystem),
        overlays: Box::new(LinuxOverlays),
    });
}

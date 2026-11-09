//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para Linux. `main` llama
//! `install` al arrancar, antes de crear la `App`.
//!
//! El sistema de archivos está en `crate::fs`. Por ahora `LinuxFileOps`, `LinuxDiskScan` y
//! `LinuxShell` son mínimos: no hacen nada o devuelven "No disponible en Linux", sin entrar en
//! pánico.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use xdir_core::fs::{Location, OverlayRequest};
use xdir_core::ops::{Op, OpResult, WM_APP_OP_DONE};
use xdir_core::shell::{Entry, Item};
use xdir_core::sys::{DiskScan, FileOps, Handle, Overlays, Services, Shell, Waker};
use xdir_core::xtree::scan::{DirListing, DriveEntry, FastScan, ListError, Lister, Progress};
use xdir_core::xtree::tree::Node;

use crate::env::LinuxEnv;
use crate::fs::LinuxFileSystem;

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

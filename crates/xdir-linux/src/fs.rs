//! Linux: el sistema de archivos del núcleo (`xdir_core::sys::FileSystem`). Lee carpetas con
//! `std::fs` en un hilo (`load`), conoce los discos y las carpetas del usuario (`places`) y vigila
//! cambios con inotify (`watch`). Lo del Shell de Windows (lugares virtuales, Acceso rápido,
//! recursos de red) no existe en Linux y devuelve vacío.

mod load;
mod places;
mod watch;

#[cfg(test)]
mod tests;

use std::ffi::{OsStr, OsString};
use std::fs::{DirEntry, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;

use xdir_core::fs::{Entry, LoadEvent, Location, QuickItem};
use xdir_core::sys::{FileSystem, Handle, Watch};

/// Carpetas de la raíz que no son archivos de verdad (núcleo, dispositivos): la búsqueda no entra.
const SYSTEM_DIRS: [&str; 3] = ["/proc", "/sys", "/dev"];

/// El sistema de archivos de Linux (lo que instala `sys::install`).
pub struct LinuxFileSystem;

impl FileSystem for LinuxFileSystem {
    fn spawn_load(&self, hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
        load::spawn_load(hwnd, tx, panel, generation, current, location);
    }

    /// La búsqueda no necesita preparar nada por hilo.
    fn with_search_thread(&self, f: Box<dyn FnOnce() + Send>) {
        f();
    }

    fn search_metadata(&self, item: &DirEntry, path: &Path) -> Option<Metadata> {
        load::metadata(item, path)
    }

    fn is_hidden(&self, _meta: &Metadata, name: &OsStr) -> bool {
        load::is_hidden(name)
    }

    /// Por nombre no se descarta nada: `/proc`, `/sys` y `/dev` se cortan en `is_mount_point`,
    /// que recibe la ruta y así no deja afuera a una `~/dev` del usuario.
    fn searchable_dir(&self, _name: &OsStr) -> bool {
        true
    }

    /// Otro disco montado adentro: su dispositivo (`st_dev`) es distinto del de la carpeta padre.
    /// También las carpetas del sistema de la raíz, aunque no estén montadas aparte.
    fn is_mount_point(&self, path: &Path) -> bool {
        if SYSTEM_DIRS.iter().any(|d| path == Path::new(d)) {
            return true;
        }
        let Some(parent) = path.parent() else { return false };
        match (std::fs::symlink_metadata(path), std::fs::metadata(parent)) {
            (Ok(m), Ok(p)) => m.dev() != p.dev(),
            _ => false,
        }
    }

    fn disk_entry(&self, path: &Path, name: OsString, meta: &Metadata) -> Entry {
        let mut e = load::entry_for(name, meta, 0, None);
        e.full = Some(path.as_os_str().to_os_string());
        e
    }

    fn user_folder(&self, i: usize) -> Option<PathBuf> {
        places::user_folder(i)
    }

    fn drives(&self) -> Vec<PathBuf> {
        places::drives()
    }

    fn drive_name(&self, root: &Path) -> String {
        places::drive_name(root)
    }

    fn drive_label(&self, drive: &str) -> String {
        places::drive_label(drive)
    }

    /// Los nombres salen de la ruta, no se guardan: no hay nada que olvidar.
    fn forget_drive_labels(&self) {}

    fn drive_missing(&self, path: &Path) -> bool {
        places::drive_missing(path)
    }

    /// La única raíz es `/`: "Sistema de archivos".
    fn root_title(&self, path: &Path) -> Option<String> {
        path.parent().is_none().then(|| places::drive_label(&places::drive_name(path)))
    }

    fn subfolders(&self, loc: &Location) -> Vec<(String, Location)> {
        places::subfolders(loc)
    }

    /// Sin íconos del sistema todavía.
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

    /// En Linux no hay nombres cortos.
    fn long_path(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    /// Las rutas distinguen mayúsculas: queda como se escribió.
    fn true_case(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn connect_share(&self, _owner: Handle, _path: &Path) -> bool {
        false
    }

    /// Misma ruta escrita, tramo por tramo y distinguiendo mayúsculas (como en macOS, sin tocar
    /// el disco: el núcleo la llama en bucles del hilo de interfaz y un `stat` sobre un montaje
    /// de red colgado congelaría la ventana). `Path::components` ignora la barra final y los `.`.
    fn same_path(&self, a: &Path, b: &Path) -> bool {
        a.components().eq(b.components())
    }

    fn watch(&self, hwnd: Handle, msg: u32, panel: usize, lparam: isize, path: &Path) -> Option<Box<dyn Watch>> {
        watch::Watcher::new(path, move || hwnd.post(msg, panel, lparam)).map(|w| Box::new(w) as Box<dyn Watch>)
    }
}

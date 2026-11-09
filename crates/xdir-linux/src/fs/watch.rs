//! Vigilancia de una carpeta con inotify (crate `notify`), como en macOS: junta las ráfagas de
//! cambios en un solo aviso y deja de vigilar al soltar el guardián.

use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

/// Guardián de una carpeta vigilada (`FileSystem::watch`). Al soltarlo se cierra el vigilante de
/// `notify`, eso cierra el canal y el hilo que avisa termina.
pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
}

impl xdir_core::sys::Watch for Watcher {}

impl Watcher {
    /// Vigila `path` (sin subcarpetas) y llama a `notify_change` una vez por ráfaga de cambios.
    /// `None` si no se puede vigilar (la carpeta no existe, se agotaron los vigilantes de inotify).
    pub fn new(path: &Path, notify_change: impl Fn() + Send + 'static) -> Option<Self> {
        use notify::Watcher as _;
        let (tx, rx) = channel::<()>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        })
        .ok()?;
        watcher.watch(path, notify::RecursiveMode::NonRecursive).ok()?;
        // Junta ráfagas de cambios (copias de muchos archivos) en un solo aviso.
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(250));
                while rx.try_recv().is_ok() {}
                notify_change();
            }
        });
        Some(Self { _watcher: watcher })
    }
}

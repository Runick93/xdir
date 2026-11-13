//! Historial de crecimiento en la vista de xtree (#58): guardar la foto de cada análisis en un hilo,
//! traer el historial de la raíz y lo que muestra la tarjeta (cambio en el período, gráfico, las
//! subcarpetas que más crecieron) y la flecha ▲ de la tabla.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::history::{self, Loaded};
use super::scan::WM_APP_XTREE;
use super::tree::Node;
use crate::sys::Handle;

/// Períodos del selector, en días.
pub const PERIODS: [u16; 3] = [7, 30, 90];

/// Lo que la vista sabe del historial de su raíz.
pub struct HistState {
    /// `[opciones] xtree_historial`: guardar una foto con cada análisis completo.
    pub on: bool,
    /// Período elegido (7, 30 o 90 días); se recuerda en la sesión.
    pub days: u16,
    /// Carpeta del historial (`<configuración>/xtree-historial`); sin ella no se guarda ni se lee.
    pub dir: Option<PathBuf>,
    /// Día de hoy fijo (pruebas y gancho de pruebas); si no, el del sistema en hora local.
    pub today_fix: Option<i32>,
    /// El historial de la raíz analizada, cuando llegó del hilo.
    pub loaded: Option<Arc<Loaded>>,
    /// Lo que deja el hilo: la raíz de la que es y su historial.
    slot: Arc<Mutex<Option<(PathBuf, Arc<Loaded>)>>>,
    /// Hay un hilo guardando o leyendo.
    pub pending: bool,
}

impl Default for HistState {
    fn default() -> Self {
        HistState { on: true, days: PERIODS[0], dir: None, today_fix: None, loaded: None, slot: Default::default(), pending: false }
    }
}

impl HistState {
    /// Día de hoy (días desde 1970 en hora local).
    pub fn today(&self) -> i32 {
        self.today_fix.unwrap_or_else(|| crate::sys::env().local_now().day as i32)
    }

    /// Terminó un análisis completo de `root` con el árbol `arena`: en un hilo guarda la foto (si
    /// está encendido) y lee el historial de la raíz; al terminar avisa a la ventana como el
    /// escáner (`WM_APP_XTREE` con `id`, `lparam` 0). Los discos de red no tienen historial.
    pub fn after_scan(&mut self, hwnd: Handle, id: usize, root: &Path, arena: &[Node]) {
        self.loaded = None;
        self.pending = false;
        let Some(dir) = self.dir.clone() else { return };
        if crate::sys::network().is_remote(root) {
            return;
        }
        let sizes = if self.on { history::select(arena) } else { Vec::new() };
        let (root, today, slot) = (root.to_path_buf(), self.today(), self.slot.clone());
        self.pending = true;
        let _ = std::thread::Builder::new().spawn(move || {
            if !sizes.is_empty() {
                let _ = history::save_snapshot(&dir, &root, today, &sizes);
            }
            let loaded = Arc::new(history::load_for(&dir, &root));
            if let Ok(mut s) = slot.lock() {
                *s = Some((root, loaded));
            }
            hwnd.post(WM_APP_XTREE, id, 0);
        });
    }

    /// Toma el historial que dejó el hilo, si es de `root`. Devuelve si llegó.
    pub fn take(&mut self, root: &Path) -> bool {
        let Some((from, loaded)) = self.slot.lock().ok().and_then(|mut s| s.take()) else { return false };
        if from != root {
            return false;
        }
        self.loaded = Some(loaded);
        self.pending = false;
        true
    }

    /// Se analiza otra cosa (o se borró el historial): lo que había ya no vale.
    pub fn reset(&mut self) {
        self.loaded = None;
        self.pending = false;
    }
}

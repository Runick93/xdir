//! Tamaño de lo elegido en la barra de estado, carpetas incluidas.
//!
//! Los archivos se suman con lo que ya trae la lista; las carpetas, en un hilo propio con
//! `info::size::folder_size` (o con el tamaño de xtree si ya analizó esa carpeta). Un cálculo por
//! selección: al cambiar lo elegido, la carpeta, la pestaña o el panel, se corta el anterior
//! (bandera compartida) y sus respuestas se descartan por generación. El tamaño de cada carpeta
//! se recuerda mientras no cambie (lo borra el vigilante de su carpeta o Actualizar). Las
//! carpetas de red no se calculan solas: la barra muestra «calcular tamaño» y al tocarlo se
//! calcula con el límite de tiempo por elemento de la red. Las carpetas solo en la nube no se
//! leen (no se baja nada): el total dice «al menos». Nada de esto lee el disco en el hilo de la
//! ventana.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use super::*;
use crate::tab::SelTotal;

/// Llegó el tamaño de una carpeta elegida.
pub(super) const WM_APP_SEL_SIZE: u32 = WM_APP + 35;

/// Qué se está sumando: el panel, la pestaña, su carpeta y las carpetas elegidas.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    pane: usize,
    tab: usize,
    location: Location,
    folders: Vec<PathBuf>,
}

/// Respuesta del hilo: lo que pesa una carpeta; `complete` es falso si no se leyó entera (sin
/// permiso, solo en la nube, la red no respondió o se cortó).
pub(super) struct Reply {
    pub generation: u64,
    pub path: PathBuf,
    pub bytes: u64,
    pub complete: bool,
}

pub(super) struct SelSize {
    key: Option<Key>,
    generation: u64,
    cancel: Arc<AtomicBool>,
    /// Lo sumado de las carpetas de esta selección y si alguna quedó a medias.
    sum: u64,
    partial: bool,
    /// Carpetas que faltan (en el hilo).
    pending: Vec<PathBuf>,
    /// Hay carpetas de red y todavía no se tocó «calcular tamaño».
    needs_click: bool,
    /// Tamaño de cada carpeta ya sumada (`complete`, como en `Reply`).
    cache: HashMap<PathBuf, (u64, bool)>,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
    /// Límite sin avance para una carpeta de red (`net::ITEM_LIMIT`; las pruebas lo achican).
    pub(super) remote_limit: Duration,
}

impl SelSize {
    pub(super) fn new() -> Self {
        let (tx, rx) = channel();
        SelSize {
            key: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            sum: 0,
            partial: false,
            pending: Vec::new(),
            needs_click: false,
            cache: HashMap::new(),
            tx,
            rx,
            remote_limit: crate::net::ITEM_LIMIT,
        }
    }

    /// Corta el cálculo en curso (sus respuestas quedan viejas).
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
    }

    /// Generación del cálculo en curso. Pruebas.
    #[cfg(test)]
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// La bandera de corte del cálculo en curso. Pruebas.
    #[cfg(test)]
    pub(super) fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Canal de las respuestas (las pruebas meten una vieja). Pruebas.
    #[cfg(test)]
    pub(super) fn sender(&self) -> Sender<Reply> {
        self.tx.clone()
    }

    /// ¿Se recuerda el tamaño de esa carpeta? Pruebas.
    #[cfg(test)]
    pub(super) fn cached(&self, path: &Path) -> Option<u64> {
        self.cache.get(path).map(|&(b, _)| b)
    }

    /// Lo que va en la barra de estado.
    pub(super) fn total(&self) -> SelTotal {
        match &self.key {
            None => SelTotal::Files,
            Some(_) if self.needs_click => SelTotal::NeedsClick,
            Some(_) if !self.pending.is_empty() => SelTotal::Computing,
            Some(_) => SelTotal::Done { bytes: self.sum, partial: self.partial },
        }
    }

    /// Algo cambió en `dir` (su vigilante avisó o se actualizó): se olvida lo que se sabía de
    /// las carpetas de adentro y, si lo elegido es de ahí, se vuelve a sumar.
    pub(super) fn invalidate(&mut self, dir: &Path) {
        let before = self.cache.len();
        self.cache.retain(|p, _| !p.starts_with(dir));
        let touched = self.key.as_ref().is_some_and(|k| k.folders.iter().any(|f| f.starts_with(dir)));
        if touched || before != self.cache.len() {
            self.cancel();
            self.key = None;
        }
    }
}

/// Corre `run` en otro hilo y espera su resultado. `run` recibe una bandera para cortarse y un
/// contador que sube cada vez que avanza: si pasa `limit` sin avance, o `cancel` se enciende, se
/// corta y devuelve `None` (el hilo queda abandonado si no responde).
fn watched<T: Send + 'static>(
    limit: Duration,
    cancel: &AtomicBool,
    run: impl FnOnce(Arc<AtomicBool>, Arc<AtomicU64>) -> T + Send + 'static,
) -> Option<T> {
    let (stop, beats) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
    let (tx, rx) = channel();
    let (s, b) = (stop.clone(), beats.clone());
    std::thread::Builder::new()
        .name("xdir-tamano-red".into())
        .spawn(move || {
            let _ = tx.send(run(s, b));
        })
        .ok()?;
    let (mut last, mut since) = (0, Instant::now());
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(v) => return Some(v),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        let now = beats.load(Ordering::Relaxed);
        if now != last {
            (last, since) = (now, Instant::now());
        }
        if cancel.load(Ordering::Relaxed) || since.elapsed() >= limit {
            stop.store(true, Ordering::Relaxed);
            return None;
        }
    }
}

/// Lo que pesa `path` (bytes, entera). Una carpeta solo en la nube no se lee (no se baja nada).
/// Una de red se lee vigilada: sin avance en `limit`, se corta y queda a medias.
fn folder_bytes(path: &Path, remote: bool, limit: Duration, cancel: &Arc<AtomicBool>) -> (u64, bool) {
    if !remote {
        if sys::file_system().cloud_only(path) {
            return (0, false);
        }
        // Una subcarpeta que no se recorre (solo en la nube) deja el total a medias.
        let skipped_cloud = std::cell::Cell::new(false);
        let cloud = |p: &Path| {
            let c = sys::file_system().cloud_only(p);
            skipped_cloud.set(skipped_cloud.get() | c);
            c
        };
        let mut last = 0;
        let skipped = crate::info::size::folder_size(path, &cloud, cancel, &mut |b, _, _| last = b);
        return (last, skipped == 0 && !skipped_cloud.get() && !cancel.load(Ordering::Relaxed));
    }
    let p = path.to_path_buf();
    let bytes_seen = Arc::new(AtomicU64::new(0));
    let seen = bytes_seen.clone();
    let out = watched(limit, cancel, move |stop, beats| {
        // También la primera pregunta va vigilada: un servidor colgado no traba el hilo.
        if sys::file_system().cloud_only(&p) {
            return (0, false);
        }
        beats.fetch_add(1, Ordering::Relaxed);
        let skipped_cloud = AtomicBool::new(false);
        let cloud = |q: &Path| {
            beats.fetch_add(1, Ordering::Relaxed);
            let c = sys::file_system().cloud_only(q);
            if c {
                skipped_cloud.store(true, Ordering::Relaxed);
            }
            c
        };
        let mut last = 0;
        let skipped = crate::info::size::folder_size(&p, &cloud, &stop, &mut |b, _, _| {
            last = b;
            seen.store(b, Ordering::Relaxed);
            beats.fetch_add(1, Ordering::Relaxed);
        });
        (last, skipped == 0 && !skipped_cloud.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed))
    });
    out.unwrap_or_else(|| (bytes_seen.load(Ordering::Relaxed), false))
}

impl App {
    /// Lo que dice la barra de estado de la pestaña activa (con el tamaño de lo elegido).
    pub(super) fn status_text(&self) -> String {
        self.panes[self.active].tab().status_with(self.sel_size.total())
    }

    /// Las carpetas elegidas en la pestaña activa, si ahí se suman: carpetas del disco (no la
    /// Papelera, «Este equipo», los comprimidos ni las carpetas virtuales).
    fn selected_folders(&self) -> Option<Key> {
        let tab = self.panes[self.active].tab();
        if !matches!(tab.location, Location::Path(_)) || sys::file_system().is_trash(&tab.location) {
            return None;
        }
        let folders: Vec<PathBuf> = tab.list.selected_indices().filter(|&i| tab.list.entries[i].is_dir).map(|i| tab.entry_path(i)).collect();
        (!folders.is_empty()).then(|| Key { pane: self.active, tab: tab.id, location: tab.location.clone(), folders })
    }

    /// Antes de cada cuadro: si lo elegido cambió, corta el cálculo anterior y arranca el nuevo.
    pub(super) fn sync_sel_size(&mut self) {
        let key = self.selected_folders();
        if key == self.sel_size.key {
            return;
        }
        self.sel_size.cancel();
        self.sel_size.key = key;
        self.start_sel_size(false);
    }

    /// «calcular tamaño»: suma también las carpetas de red de lo elegido.
    pub(super) fn request_sel_size(&mut self) {
        if self.sel_size.needs_click {
            self.sel_size.cancel();
            self.start_sel_size(true);
            self.redraw();
        }
    }

    /// Suma lo que se sabe (caché, xtree) y manda al hilo lo que falta. Sin `remote`, si falta
    /// alguna carpeta de red no se calcula nada: se espera el clic.
    fn start_sel_size(&mut self, remote: bool) {
        let s = &mut self.sel_size;
        (s.sum, s.partial, s.needs_click) = (0, false, false);
        s.pending.clear();
        let Some(key) = &s.key else { return };
        let mut todo = Vec::new();
        for f in &key.folders {
            if let Some(&(b, complete)) = s.cache.get(f) {
                s.sum += b;
                s.partial |= !complete;
            } else if let Some(b) = self.xtree.as_ref().and_then(|x| x.size_of(f)) {
                s.sum += b;
            } else {
                todo.push(f.clone());
            }
        }
        if todo.is_empty() {
            return;
        }
        let net = sys::network();
        if !remote && todo.iter().any(|f| net.is_remote(f)) {
            s.needs_click = true;
            return;
        }
        s.pending = todo.clone();
        let (hwnd, tx, generation, cancel, limit) = (self.hwnd, s.tx.clone(), s.generation, s.cancel.clone(), s.remote_limit);
        let spawned = std::thread::Builder::new().name("xdir-tamano-seleccion".into()).spawn(move || {
            for path in todo {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let remote = sys::network().is_remote(&path);
                let (bytes, complete) = folder_bytes(&path, remote, limit, &cancel);
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if tx.send(Reply { generation, path, bytes, complete }).is_err() {
                    return;
                }
                hwnd.post(WM_APP_SEL_SIZE, 0, 0);
            }
        });
        debug_assert!(spawned.is_ok(), "hilo del tamaño de la selección");
        if spawned.is_err() {
            self.sel_size.pending.clear();
            self.sel_size.partial = true;
        }
    }

    /// Llegó el tamaño de carpetas elegidas: se suma si es del cálculo en curso.
    pub(super) fn on_sel_size(&mut self) {
        let mut changed = false;
        while let Ok(r) = self.sel_size.rx.try_recv() {
            let s = &mut self.sel_size;
            if r.generation != s.generation {
                continue; // de una selección vieja
            }
            let Some(i) = s.pending.iter().position(|p| *p == r.path) else { continue };
            s.pending.remove(i);
            s.sum += r.bytes;
            s.partial |= !r.complete;
            s.cache.insert(r.path, (r.bytes, r.complete));
            changed = true;
        }
        if changed {
            self.redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sin avance en el límite, se corta; con avance seguido, termina aunque tarde más.
    #[test]
    fn watched_cuts_without_progress() {
        let cancel = AtomicBool::new(false);
        let t0 = Instant::now();
        let out = watched(Duration::from_millis(100), &cancel, |stop, _| {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
            1
        });
        assert_eq!(out, None);
        assert!(t0.elapsed() < Duration::from_secs(2));
        let out = watched(Duration::from_millis(100), &cancel, |_, beats| {
            for _ in 0..8 {
                std::thread::sleep(Duration::from_millis(40));
                beats.fetch_add(1, Ordering::Relaxed);
            }
            7
        });
        assert_eq!(out, Some(7));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(watched(Duration::from_secs(5), &cancel, |_, _| std::thread::sleep(Duration::from_millis(500))), None);
    }
}

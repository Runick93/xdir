//! Tamaño de lo elegido en la barra de estado, carpetas incluidas.
//!
//! Los archivos se suman con lo que ya trae la lista; las carpetas, en un hilo propio con
//! `info::size::folder_size`: el tamaño lógico (lo que pesan los archivos). El de xtree no se
//! usa aunque ya haya analizado la carpeta: mide lo que ocupa en el disco (bloques, enlaces
//! duros una vez), así que mezclarlos daría totales distintos para la misma carpeta. Un cálculo por
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

/// Qué se está sumando: el panel, la pestaña, su carpeta y las carpetas elegidas con su fecha
/// de la lista (si al releer la carpeta cambió la fecha de una, cambió lo de adentro).
#[derive(Clone, Debug, PartialEq)]
struct Key {
    pane: usize,
    tab: usize,
    location: Location,
    folders: Vec<(PathBuf, u64)>,
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
    /// Firma barata de lo elegido en el último cuadro (`selection_sig`): mientras no cambie, la
    /// clave (con sus rutas) no se vuelve a armar.
    sig: Option<u64>,
    generation: u64,
    cancel: Arc<AtomicBool>,
    /// Lo sumado de las carpetas de esta selección y si alguna quedó a medias.
    sum: u64,
    partial: bool,
    /// Carpetas que faltan (en el hilo).
    pending: Vec<PathBuf>,
    /// Hay carpetas de red y todavía no se tocó «calcular tamaño».
    needs_click: bool,
    /// Tamaño de cada carpeta ya sumada entera: (bytes, `complete`, su fecha en la lista). Vale
    /// mientras la fecha sea la misma.
    cache: HashMap<PathBuf, (u64, bool, u64)>,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
    /// Cuántas veces se armó la clave (pruebas).
    #[cfg(test)]
    pub(super) key_builds: usize,
    /// Límite sin avance para una carpeta de red (`net::ITEM_LIMIT`; las pruebas lo achican).
    pub(super) remote_limit: Duration,
}

impl SelSize {
    pub(super) fn new() -> Self {
        let (tx, rx) = channel();
        SelSize {
            key: None,
            sig: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            sum: 0,
            partial: false,
            pending: Vec::new(),
            needs_click: false,
            cache: HashMap::new(),
            tx,
            rx,
            #[cfg(test)]
            key_builds: 0,
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
        self.cache.get(path).map(|&(b, ..)| b)
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

    /// Actualizar en `dir`: se olvida lo que se sabía de las carpetas de adentro y de las que la
    /// contienen y, si lo elegido es una de ellas, se vuelve a sumar.
    pub(super) fn invalidate(&mut self, dir: &Path) {
        self.forget(|p| p.starts_with(dir) || dir.starts_with(p));
    }

    /// El vigilante de `dir` avisó un cambio: cambió lo que pesa `dir` y lo que pesan las carpetas
    /// que la contienen. Las de adentro no se tocan: al releer `dir`, la que cambió trae otra
    /// fecha (un archivo suelto que cambia no corta la suma de las carpetas elegidas).
    pub(super) fn changed_in(&mut self, dir: &Path) {
        self.forget(|p| dir.starts_with(p));
    }

    fn forget(&mut self, related: impl Fn(&Path) -> bool) {
        self.cache.retain(|p, _| !related(p));
        if self.key.as_ref().is_some_and(|k| k.folders.iter().any(|(f, _)| related(f))) {
            self.cancel();
            self.key = None;
            self.sig = None;
        }
    }
}

/// Avance de una suma: cuántas veces avanzó y lo sumado hasta ahora.
#[derive(Default)]
struct Beat {
    beats: AtomicU64,
    bytes: AtomicU64,
}

/// Corre `run` en otro hilo y espera su resultado. `run` recibe una bandera para cortarse y el
/// avance: si pasa `limit` sin avance, o `cancel` se enciende, se corta y devuelve `None` (el
/// hilo queda abandonado si no responde).
fn watched<T: Send + 'static>(limit: Duration, cancel: &AtomicBool, beat: Arc<Beat>, run: impl FnOnce(Arc<AtomicBool>, Arc<Beat>) -> T + Send + 'static) -> Option<T> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let (s, b) = (stop.clone(), beat.clone());
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
        let now = beat.beats.load(Ordering::Relaxed);
        if now != last {
            (last, since) = (now, Instant::now());
        }
        if cancel.load(Ordering::Relaxed) || since.elapsed() >= limit {
            stop.store(true, Ordering::Relaxed);
            return None;
        }
    }
}

/// Suma `path`: (bytes, entera). No sigue un enlace elegido, no lee lo que está solo en la nube
/// (no se baja nada), no cruza a otro disco montado adentro y, en una suma local, no entra en
/// una carpeta de red montada adentro. Todo lo que queda afuera deja el total «a medias».
fn walk(path: &Path, remote: bool, stop: &AtomicBool, beat: &Beat) -> (u64, bool) {
    let (fs, net) = (sys::file_system(), sys::network());
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_symlink() => {}
        _ => return (0, false),
    }
    beat.beats.fetch_add(1, Ordering::Relaxed);
    if fs.cloud_only(path) {
        return (0, false);
    }
    let volume = fs.volume_id(path);
    let left_out = AtomicBool::new(false);
    let skip = |p: &Path| {
        beat.beats.fetch_add(1, Ordering::Relaxed);
        // La red primero: preguntar por una carpeta de red colgada trabaría el hilo.
        // Otro disco: por su `st_dev` o, si comparte el número (en la Mac, /System/Volumes/Data),
        // porque ahí se monta otro.
        let out = (!remote && net.is_remote(p)) || fs.cloud_only(p) || (volume.is_some() && fs.volume_id(p) != volume) || fs.is_mount_point(p);
        if out {
            left_out.store(true, Ordering::Relaxed);
        }
        out
    };
    let mut last = 0;
    // En la red el avance cuenta con cada elemento: un servidor lento pero vivo no se corta.
    let every = if remote { 1 } else { crate::info::size::STEP };
    let skipped = crate::info::size::folder_size_every(path, &skip, stop, every, &mut |b, _, _| {
        last = b;
        beat.bytes.store(b, Ordering::Relaxed);
        beat.beats.fetch_add(1, Ordering::Relaxed);
    });
    (last, skipped == 0 && !left_out.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed))
}

/// Carpetas de red con una suma en curso (o colgada en el servidor): no se apila otra.
static IN_FLIGHT: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn in_flight() -> std::sync::MutexGuard<'static, Vec<PathBuf>> {
    IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lo que pesa `path` (bytes, entera). Una de red se suma vigilada: sin avance en `limit`, se
/// corta y queda a medias; si su suma anterior sigue colgada, no se arranca otra.
fn folder_bytes(path: &Path, remote: bool, limit: Duration, cancel: &Arc<AtomicBool>) -> (u64, bool) {
    if !remote {
        return walk(path, false, cancel, &Beat::default());
    }
    let p = path.to_path_buf();
    {
        let mut f = in_flight();
        if f.contains(&p) {
            return (0, false);
        }
        f.push(p.clone());
    }
    // La marca se suelta cuando el hilo termina (o si no llegó a arrancar).
    struct Release(PathBuf);
    impl Drop for Release {
        fn drop(&mut self) {
            in_flight().retain(|x| *x != self.0);
        }
    }
    let release = Release(p.clone());
    let beat = Arc::new(Beat::default());
    let out = watched(limit, cancel, beat.clone(), move |stop, beat| {
        let release = release;
        walk(&release.0, true, &stop, &beat)
    });
    out.unwrap_or_else(|| (beat.bytes.load(Ordering::Relaxed), false))
}

impl App {
    /// Lo que dice la barra de estado de la pestaña activa (con el tamaño de lo elegido).
    pub(super) fn status_text(&self) -> String {
        self.panes[self.active].tab().status_with(self.sel_size.total())
    }

    /// Las carpetas elegidas en la pestaña activa, si ahí se suman: carpetas del disco (no la
    /// Papelera, «Este equipo», los comprimidos ni las carpetas virtuales).
    fn selected_folders(&mut self) -> Option<Key> {
        let tab = self.panes[self.active].tab();
        if !matches!(tab.location, Location::Path(_)) || sys::file_system().is_trash(&tab.location) {
            return None;
        }
        #[cfg(test)]
        {
            self.sel_size.key_builds += 1;
        }
        let folders: Vec<(PathBuf, u64)> =
            tab.list.selected_indices().filter(|&i| tab.list.entries[i].is_dir).map(|i| (tab.entry_path(i), tab.list.entries[i].modified)).collect();
        (!folders.is_empty()).then(|| Key { pane: self.active, tab: tab.id, location: tab.location.clone(), folders })
    }

    /// Firma de lo elegido: panel, pestaña, navegación, cantidad de elementos y cada carpeta
    /// elegida (índice, nombre y fecha). Sin armar rutas: se calcula en cada cuadro.
    fn selection_sig(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let tab = self.panes[self.active].tab();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.active, tab.id, tab.nav_id(), tab.list.entries.len()).hash(&mut h);
        for i in tab.list.selected_indices().filter(|&i| tab.list.entries[i].is_dir) {
            let e = &tab.list.entries[i];
            (i, &e.name, e.modified).hash(&mut h);
        }
        h.finish()
    }

    /// Antes de cada cuadro: si lo elegido cambió, corta el cálculo anterior y arranca el nuevo.
    pub(super) fn sync_sel_size(&mut self) {
        let sig = self.selection_sig();
        if self.sel_size.sig == Some(sig) {
            return;
        }
        self.sel_size.sig = Some(sig);
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

    /// Suma lo que se sabe (caché) y manda al hilo lo que falta. Sin `remote`, si falta
    /// alguna carpeta de red no se calcula nada: se espera el clic.
    fn start_sel_size(&mut self, remote: bool) {
        let s = &mut self.sel_size;
        (s.sum, s.partial, s.needs_click) = (0, false, false);
        s.pending.clear();
        let Some(key) = &s.key else { return };
        let mut todo = Vec::new();
        for (f, modified) in &key.folders {
            if let Some(&(b, complete, _)) = s.cache.get(f).filter(|c| c.2 == *modified) {
                s.sum += b;
                s.partial |= !complete;
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
            // Solo se recuerda lo entero: lo que quedó a medias (la red no respondió, sin permiso)
            // se vuelve a intentar la próxima vez.
            if r.complete {
                let modified = s.key.as_ref().and_then(|k| k.folders.iter().find(|(f, _)| *f == r.path)).map_or(0, |&(_, m)| m);
                s.cache.insert(r.path, (r.bytes, true, modified));
            }
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

    /// Mientras una suma de una carpeta de red sigue en curso (o colgada), otra de la misma
    /// carpeta no arranca otro hilo: queda a medias. Al terminar la primera, se puede de nuevo.
    #[test]
    fn one_remote_walk_per_folder() {
        crate::sys::testing::install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-sel-red-una-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.bin"), vec![0u8; 70]).unwrap();
        crate::sys::testing::FakeNetwork::set_remote(&dir);
        let cancel = Arc::new(AtomicBool::new(false));
        in_flight().push(dir.clone());
        assert_eq!(folder_bytes(&dir, true, Duration::from_secs(5), &cancel), (0, false));
        in_flight().retain(|p| *p != dir);
        assert_eq!(folder_bytes(&dir, true, Duration::from_secs(5), &cancel), (70, true));
        assert!(!in_flight().contains(&dir), "al terminar se suelta");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sin avance en el límite, se corta; con avance seguido, termina aunque tarde más.
    #[test]
    fn watched_cuts_without_progress() {
        let cancel = AtomicBool::new(false);
        let t0 = Instant::now();
        let out = watched(Duration::from_millis(100), &cancel, Default::default(), |stop, _| {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
            1
        });
        assert_eq!(out, None);
        assert!(t0.elapsed() < Duration::from_secs(2));
        let out = watched(Duration::from_millis(100), &cancel, Default::default(), |_, beat| {
            for _ in 0..8 {
                std::thread::sleep(Duration::from_millis(40));
                beat.beats.fetch_add(1, Ordering::Relaxed);
            }
            7
        });
        assert_eq!(out, Some(7));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(watched(Duration::from_secs(5), &cancel, Default::default(), |_, _| std::thread::sleep(Duration::from_millis(500))), None);
    }
}

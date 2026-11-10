//! Trabajos del motor de copia (`transfer`) en la `App`: cada uno corre en su hilo con
//! `sys::copy_prims()`, espera su turno en la cola (`Scheduler`) si comparte un disco con otro, y
//! manda su estado por canal con `WM_APP_TRANSFER`. El hilo de interfaz nunca espera al disco:
//! solo arma los `JobView` con lo que llega.
//!
//! La ventana de copia es una ventana secundaria del sistema (`WindowHost::aux_show`) cuyo
//! contenido dibuja `transfer::view`: aparece si un trabajo lleva más de 1 s
//! (`WM_APP_TRANSFER_DUE`) o enseguida con un conflicto o un error, se ajusta al contenido y se
//! oculta sola con la cola vacía. Cerrarla no cancela nada: queda el indicador de la barra de
//! estado, que la vuelve a abrir. Donde el host no tiene ventanas (Linux sin interfaz) los
//! conflictos se responden solos con «Omitir» (`auto_skip`) y se avisa en la barra de estado.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use super::App;
use crate::geom::rect;
use crate::host::{AuxMouse, AuxWindow};
use crate::keys::{Key, keys::VK_ESCAPE};
use crate::ops::Undo;
use crate::paint::Painter;
use crate::transfer::view::{self, ViewHit};
use crate::sys::{self, CopyPrims};
use crate::transfer::sched::Scheduler;
use crate::transfer::speed::Speed;
use crate::transfer::{self, Answer, Choice, Control, JobId, JobOutcome, JobSpec, JobState, JobView, Update, WM_APP_TRANSFER};

/// Lo que la `App` guarda de cada trabajo además de su vista.
struct JobMeta {
    ctl: Arc<Control>,
    /// Mueve (para «Reintentar»).
    moving: bool,
    /// Es un «Devolver»: «Reintentar» vuelve a ser «Devolver» (con sus reglas), nunca un mover.
    restore: bool,
    /// «Copiar», «Duplicar»… para los avisos.
    label: &'static str,
    /// Rutas que toca (para saber si usa un disco que se quiere expulsar).
    paths: Vec<PathBuf>,
    /// Los discos que toca (identidad), cuando el hilo del trabajo los averiguó.
    volumes: Vec<OsString>,
    /// Comprimir y extraer: «Reintentar» vuelve a correr el mismo trabajo entero.
    rerun: Option<JobSpec>,
}

/// Estado de los trabajos de copia de la `App`.
pub(super) struct Transfers {
    tx: Sender<(JobId, Update)>,
    rx: Receiver<(JobId, Update)>,
    jobs: Vec<JobView>,
    meta: HashMap<JobId, JobMeta>,
    next: JobId,
    sched: Arc<Scheduler>,
    /// Sin ventana para preguntar: los conflictos se responden «Omitir» en el hilo del trabajo.
    /// Lo leen también los hilos de los trabajos: si el host no puede abrir la ventana, los que ya
    /// corren dejan de esperar respuesta.
    auto_skip: Arc<AtomicBool>,
    /// Expulsar espera que terminen (y limpien) los trabajos cancelados sobre el disco; varios
    /// pedidos esperan cada uno los suyos.
    pending_eject: Vec<PendingEject>,
    /// Los discos que se están expulsando (desde que se pide hasta que termina, en otro hilo):
    /// raíz, identidad (si se sabe) y nombre. Los lee la interfaz y el hilo de cada trabajo nuevo
    /// para no empezar sobre ellos.
    ejecting: Arc<Mutex<Vec<Ejecting>>>,
    /// Identidad de cada disco expulsable (la calcula en su hilo la tarea que los lista): así la
    /// interfaz nunca mira el disco.
    root_ids: HashMap<PathBuf, OsString>,
    window: CopyWindow,
    /// Lo que se lanzó (pruebas).
    #[cfg(test)]
    started: Vec<JobSpec>,
}

/// Estado de la ventana de copia.
#[derive(Default)]
struct CopyWindow {
    /// Está a la vista (la mostró la `App` y nadie la cerró).
    shown: bool,
    /// El trabajo que se eligió expandir (si no, el primero que corre).
    chosen: Option<JobId>,
    /// Se plegó con «Menos detalles».
    collapsed: bool,
    hover: Option<ViewHit>,
    pressed: Option<ViewHit>,
    /// El último tamaño pedido (para no repetir `aux_resize`) y el último título.
    size: (f32, f32),
    title: String,
}

/// Lo que sigue cuando terminan unos trabajos (soltar el disco al expulsar).
pub type AfterJobs = Box<dyn FnOnce(&mut App)>;

/// Un disco que se está expulsando (desde que se pide hasta que termina): no empiezan trabajos
/// que lo usen.
#[derive(Clone)]
struct Ejecting {
    root: PathBuf,
    volume: Option<OsString>,
    label: String,
}

/// Un «Expulsar» que espera a que terminen (y limpien) los trabajos sobre el disco.
struct PendingEject {
    jobs: Vec<JobId>,
    then: AfterJobs,
}

impl Transfers {
    pub(super) fn new() -> Transfers {
        let (tx, rx) = channel();
        Transfers {
            tx,
            rx,
            jobs: Vec::new(),
            meta: HashMap::new(),
            next: 1,
            sched: Arc::new(Scheduler::new()),
            // La plataforma con ventana de copia lo apaga (`set_auto_skip(false)`) y se pregunta.
            auto_skip: Arc::new(AtomicBool::new(true)),
            pending_eject: Vec::new(),
            ejecting: Arc::new(Mutex::new(Vec::new())),
            root_ids: HashMap::new(),
            window: CopyWindow::default(),
            #[cfg(test)]
            started: Vec::new(),
        }
    }
}

/// Rutas que toca un trabajo.
fn spec_paths(spec: &JobSpec) -> Vec<PathBuf> {
    fn undo_paths(u: &Undo, out: &mut Vec<PathBuf>) {
        match u {
            Undo::Batch(v) => v.iter().for_each(|u| undo_paths(u, out)),
            Undo::Created(v) => out.extend(v.iter().cloned()),
            Undo::CreatedIds(v) => out.extend(v.iter().map(|(p, _)| p.clone())),
            Undo::Moved(v) => v.iter().for_each(|(a, b)| out.extend([a.clone(), b.clone()])),
            Undo::Recycled(v) => out.extend(v.iter().map(|(_, p)| p.clone())),
            Undo::Rename { path, .. } => out.push(path.clone()),
            Undo::Renames(v) => out.extend(v.iter().map(|(p, _)| p.clone())),
            Undo::CreatedFolder(p) => out.push(p.clone()),
        }
    }
    let mut out = Vec::new();
    match spec {
        JobSpec::Copy { sources, dest } | JobSpec::Move { sources, dest } => {
            out.extend(sources.iter().cloned());
            out.push(dest.clone());
        }
        JobSpec::Duplicate { sources } => out.extend(sources.iter().cloned()),
        JobSpec::NewFolderWith { parent, items } => {
            out.push(parent.clone());
            out.extend(items.iter().cloned());
        }
        JobSpec::Restore { items } => out.extend(items.iter().cloned()),
        JobSpec::Undo(u) => undo_paths(u, &mut out),
        JobSpec::Retry { pairs, .. } => pairs.iter().for_each(|(a, b)| out.extend([a.clone(), b.clone()])),
        JobSpec::Compress { sources } => out.extend(sources.iter().cloned()),
        JobSpec::Extract { archive, .. } => out.push(archive.clone()),
    }
    out
}

/// De dónde a dónde va un trabajo (el resumen al terminar con errores).
fn spec_route(spec: &JobSpec) -> transfer::JobRoute {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let folder = |v: &[PathBuf]| v.first().and_then(|p| p.parent()).and_then(name);
    let (done_one, done_many, from, to, sources) = match spec {
        JobSpec::Copy { sources, dest } => ("Se copió", "Se copiaron", folder(sources), name(dest), sources.clone()),
        JobSpec::Move { sources, dest } => ("Se movió", "Se movieron", folder(sources), name(dest), sources.clone()),
        JobSpec::Duplicate { sources } => ("Se duplicó", "Se duplicaron", None, None, sources.clone()),
        JobSpec::NewFolderWith { parent, items } => ("Se movió", "Se movieron", None, name(parent), items.clone()),
        JobSpec::Restore { items } => ("Se devolvió", "Se devolvieron", None, None, items.clone()),
        JobSpec::Retry { moving, pairs } => {
            let srcs: Vec<PathBuf> = pairs.iter().map(|(s, _)| s.clone()).collect();
            let to = pairs.first().and_then(|(_, d)| d.parent()).and_then(name);
            let (one, many) = if *moving { ("Se movió", "Se movieron") } else { ("Se copió", "Se copiaron") };
            (one, many, folder(&srcs), to, srcs)
        }
        JobSpec::Undo(_) => ("Se deshizo", "Se deshicieron", None, None, Vec::new()),
        JobSpec::Compress { sources } => ("Se comprimió", "Se comprimieron", folder(sources), None, sources.clone()),
        JobSpec::Extract { archive, .. } => ("Se extrajo", "Se extrajeron", None, None, vec![archive.clone()]),
    };
    transfer::JobRoute { done_one, done_many, from, to, sources }
}

/// Cuántos elementos eligió el usuario (el título antes de recorrer).
fn spec_count(spec: &JobSpec) -> u64 {
    (match spec {
        JobSpec::Copy { sources, .. } | JobSpec::Move { sources, .. } | JobSpec::Duplicate { sources } => sources.len(),
        JobSpec::NewFolderWith { items, .. } | JobSpec::Restore { items } => items.len(),
        JobSpec::Retry { pairs, .. } => pairs.len(),
        JobSpec::Compress { sources } => sources.len(),
        JobSpec::Extract { .. } => 1,
        JobSpec::Undo(_) => 0,
    }) as u64
}

fn spec_label(spec: &JobSpec) -> (&'static str, bool) {
    match spec {
        JobSpec::Copy { .. } => ("Copiar", false),
        JobSpec::Move { .. } => ("Mover", true),
        JobSpec::Duplicate { .. } => ("Duplicar", false),
        JobSpec::NewFolderWith { .. } => ("Carpeta nueva con la selección", true),
        JobSpec::Restore { .. } => ("Devolver", true),
        JobSpec::Undo(_) => ("Deshacer", true),
        JobSpec::Retry { moving: true, .. } => ("Mover", true),
        JobSpec::Retry { moving: false, .. } => ("Copiar", false),
        JobSpec::Compress { .. } => ("Comprimir", false),
        JobSpec::Extract { .. } => ("Extraer", false),
    }
}

/// Discos que toca un trabajo (lee el disco: corre en el hilo del trabajo).
fn volumes(prims: &dyn CopyPrims, paths: &[PathBuf]) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    for p in paths {
        // Un destino que todavía no existe: el disco de su carpeta.
        let v = prims.volume(p).or_else(|| p.parent().and_then(|d| prims.volume(d)));
        if let Some(v) = v
            && !out.contains(&v.id)
        {
            out.push(v.id);
        }
    }
    out
}

/// Suelta el turno de la cola aunque el trabajo termine mal.
struct Release(Arc<Scheduler>, JobId);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.release(self.1);
    }
}

/// «Hay 2 operaciones sobre "USB (E:)". ¿Cancelarlas y expulsar?» (antes de expulsar un disco).
pub fn eject_question(jobs: usize, label: &str) -> String {
    if jobs == 1 {
        format!("Hay 1 operación sobre \"{label}\". ¿Cancelarla y expulsar?")
    } else {
        format!("Hay {jobs} operaciones sobre \"{label}\". ¿Cancelarlas y expulsar?")
    }
}

/// «Se omitieron 2 elementos que no se pueden crear» (al extraer: enlaces en Windows, enlaces
/// duros y dispositivos de un tar).
pub fn omitted_text(n: usize) -> String {
    if n == 1 { "Se omitió 1 elemento que no se puede crear".into() } else { format!("Se omitieron {n} elementos que no se pueden crear") }
}

/// «Se omitieron 3 elementos que ya existían».
fn skipped_text(n: usize) -> String {
    if n == 1 { "Se omitió 1 elemento que ya existía".into() } else { format!("Se omitieron {n} elementos que ya existían") }
}

/// Los clics en un conflicto recién aparecido no cuentan durante este tiempo.
const CONFLICT_GUARD: std::time::Duration = std::time::Duration::from_millis(400);

/// «Hay 2 operaciones en curso. ¿Cancelarlas y salir?» (al cerrar xdir).
pub fn close_question(jobs: usize) -> String {
    if jobs == 1 {
        "Hay 1 operación en curso. ¿Cancelarla y salir?".into()
    } else {
        format!("Hay {jobs} operaciones en curso. ¿Cancelarlas y salir?")
    }
}

/// Manda `WM_APP_TRANSFER_DUE` del trabajo `id` dentro de 1 s.
fn post_due(hwnd: crate::sys::Handle, id: JobId) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        hwnd.post(transfer::WM_APP_TRANSFER_DUE, 0, id as isize);
    });
}

fn plural_errors(n: usize) -> String {
    if n == 1 { "1 error".into() } else { format!("{n} errores") }
}

impl App {
    /// Lanza un trabajo del motor en su hilo y vuelve enseguida.
    /// Si toca un disco que se está por expulsar (espera a que limpien sus trabajos), no empieza:
    /// lo avisa y devuelve 0.
    pub fn start_transfer(&mut self, spec: JobSpec) -> JobId {
        #[cfg(test)]
        self.transfers.started.push(spec.clone());
        let paths = spec_paths(&spec);
        let busy = self.transfers.ejecting.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|e| paths.iter().any(|p| p.starts_with(&e.root))).cloned();
        if let Some(e) = busy {
            self.notice = Some(format!("Se está expulsando \"{}\"", e.label));
            self.redraw();
            return 0;
        }
        let t = &mut self.transfers;
        let id = t.next;
        t.next += 1;
        let ctl = Control::new();
        let (label, moving) = spec_label(&spec);
        let paths = spec_paths(&spec);
        let now = Instant::now();
        t.jobs.push(JobView {
            id,
            // Hasta que termine el recorrido, el título cuenta lo elegido.
            title: transfer::title(&spec, spec_count(&spec)),
            state: JobState::Preparing,
            items: (0, 0),
            bytes: (0, 0),
            current: String::new(),
            speed: Speed::new(now, 0),
            conflict: None,
            errors: Vec::new(),
            stopped: None,
            paused: false,
            started: now,
            apply_all: false,
            conflict_since: None,
            route: spec_route(&spec),
            done_items: None,
        });
        let restore = matches!(spec, JobSpec::Restore { .. });
        let rerun = matches!(spec, JobSpec::Compress { .. } | JobSpec::Extract { .. }).then(|| spec.clone());
        t.meta.insert(id, JobMeta { ctl: ctl.clone(), moving, restore, label, paths: paths.clone(), volumes: Vec::new(), rerun });
        let (tx, hwnd, sched, auto_skip, ejecting) = (t.tx.clone(), self.hwnd, t.sched.clone(), t.auto_skip.clone(), t.ejecting.clone());
        std::thread::spawn(move || {
            let prims = sys::copy_prims();
            let send = |u: Update| {
                if tx.send((id, u)).is_ok() {
                    hwnd.post(WM_APP_TRANSFER, 0, 0);
                }
            };
            let vols = volumes(prims, &paths);
            send(Update::Volumes(vols.clone()));
            // Un disco que se está por expulsar (también por otro camino): no empieza.
            let busy = ejecting
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|e| e.volume.as_ref().is_some_and(|v| vols.contains(v)))
                .map(|e| e.label.clone());
            if let Some(label) = busy {
                send(Update::Finished(JobOutcome { stopped: Some(format!("Se está expulsando \"{label}\"")), ..Default::default() }));
                return;
            }
            if !sched.acquire_notify(id, &vols, &ctl, &mut || send(Update::Waiting)) {
                send(Update::Finished(JobOutcome { cancelled: true, ..Default::default() }));
                return;
            }
            let _turn = Release(sched, id);
            transfer::run(spec, prims, &ctl, &mut |u| {
                if auto_skip.load(Ordering::SeqCst) && matches!(u, Update::Conflict(_)) {
                    ctl.answer(Answer { choice: Choice::Skip, all: false });
                }
                send(u)
            });
        });
        post_due(self.hwnd, id);
        id
    }

    /// Vacía el canal de los trabajos (`WM_APP_TRANSFER`).
    pub fn on_transfer_updates(&mut self) {
        let now = Instant::now();
        let mut finished = Vec::new();
        let mut show = false;
        let auto = self.auto_skip();
        // Conflictos que llegaron sin a quién preguntar (el hilo los mandó antes de saberlo).
        let mut skip_now = Vec::new();
        // Preguntas de los trabajos (se hacen después de vaciar el canal).
        let mut confirms = Vec::new();
        while let Ok((id, update)) = self.transfers.rx.try_recv() {
            if let Update::Volumes(vols) = &update {
            if let Some(m) = self.transfers.meta.get_mut(&id) {
                m.volumes = vols.clone();
            }
            continue;
        }
        let Some(v) = self.transfers.jobs.iter_mut().find(|j| j.id == id) else { continue };
            match update {
                Update::Volumes(_) => {}
                Update::Preparing { items } => {
                    v.state = JobState::Preparing;
                    v.items.1 = items;
                }
                Update::Waiting => v.state = JobState::Waiting,
                Update::Started { title, items, bytes } => {
                    v.title = title;
                    v.items = (0, items);
                    v.bytes = (0, bytes);
                    v.speed = Speed::new(now, bytes);
                    v.state = JobState::Running;
                }
                Update::Progress { items, bytes, current } => {
                    v.items.0 = items;
                    v.bytes.0 = bytes;
                    v.current = current;
                    if v.state == JobState::Conflict {
                        // Ya se respondió (solo, con «Omitir», o desde la ventana).
                        v.conflict = None;
                        v.speed.resume(now);
                    }
                    v.speed.push(now, bytes);
                    v.state = JobState::Running;
                }
                Update::Conflict(info) => {
                    // El tiempo esperando la respuesta no cuenta para la velocidad promedio.
                    v.speed.pause(now);
                    if !auto {
                        self.notice = Some(format!("Ya existe «{}» en «{}»", info.name, info.folder));
                    } else {
                        skip_now.push(id);
                    }
                    v.conflict = Some(info);
                    v.state = JobState::Conflict;
                    v.apply_all = false;
                    v.conflict_since = Some(now);
                    show = show || !auto;
                }
                Update::Confirm(q) => confirms.push((id, q)),
                Update::Finished(outcome) => {
                    v.state = JobState::Done;
                    v.conflict = None;
                    v.errors = outcome.errors.clone();
                    v.stopped = outcome.stopped.clone();
                    v.done_items = Some(outcome.done_items);
                    finished.push((id, outcome));
                }
            }
        }
        for id in skip_now {
            self.answer_job(id, Answer { choice: Choice::Skip, all: false });
        }
        for (id, q) in confirms {
            let label = self.transfers.meta.get(&id).map_or("xdir", |m| m.label);
            let yes = self.host.clone().ask_yes_no(label, &q);
            if let Some(c) = self.job_ctl(id) {
                c.confirm(yes);
            }
        }
        for (id, outcome) in finished {
            // Terminó con errores o sin poder hacerlo: la ventana lo muestra enseguida.
            show = show || !outcome.errors.is_empty() || outcome.stopped.is_some();
            self.on_job_done(id, outcome);
        }
        if show {
            self.show_transfers();
        } else {
            self.sync_transfers_window();
        }
    }

    /// Pasó 1 s desde que empezó el trabajo `id` (`WM_APP_TRANSFER_DUE`): si sigue, aparece la
    /// ventana de copia.
    pub(super) fn on_transfer_due(&mut self, id: JobId) {
        self.on_transfer_updates();
        if self.transfers.jobs.iter().any(|j| j.id == id && j.state != JobState::Done) && !self.transfers.window.shown {
            self.show_transfers();
        }
    }

    /// El trabajo con los detalles y el gráfico: el elegido, o el primero que está corriendo.
    pub(super) fn transfers_expanded(&self) -> Option<JobId> {
        let w = &self.transfers.window;
        if w.collapsed {
            return None;
        }
        let active = |j: &&JobView| matches!(j.state, JobState::Running | JobState::Preparing);
        let jobs = &self.transfers.jobs;
        w.chosen.and_then(|id| jobs.iter().filter(active).find(|j| j.id == id)).or_else(|| jobs.iter().find(active)).map(|j| j.id)
    }

    fn transfers_size(&self) -> (f32, f32) {
        (view::WIDTH, view::content_height(&self.transfers.jobs, self.transfers_expanded()))
    }

    /// Muestra (o trae al frente) la ventana de copia. Sin ventana del host queda solo el
    /// indicador, y los conflictos vuelven a responderse solos.
    pub(super) fn show_transfers(&mut self) {
        if self.transfers.jobs.is_empty() {
            return;
        }
        let size = self.transfers_size();
        let host = self.host.clone();
        let title = view::window_title(&self.transfers.jobs);
        if host.aux_show(AuxWindow::Transfers, &title, size) {
            let w = &mut self.transfers.window;
            w.shown = true;
            w.size = size;
            w.title = title;
            host.aux_invalidate(AuxWindow::Transfers);
        } else {
            self.transfers.window.shown = false;
            if !self.auto_skip() {
                self.transfers.auto_skip.store(true, Ordering::SeqCst);
                // Los que ya esperan una respuesta no tienen a quién preguntar.
                let waiting: Vec<JobId> = self.transfers.jobs.iter().filter(|j| j.state == JobState::Conflict).map(|j| j.id).collect();
                for id in waiting {
                    self.answer_job(id, Answer { choice: Choice::Skip, all: false });
                }
            }
        }
        self.redraw();
    }

    /// La ventana sigue al contenido: cambia de alto, se repinta, y con la cola vacía se oculta.
    fn sync_transfers_window(&mut self) {
        if !self.transfers.window.shown {
            return;
        }
        let host = self.host.clone();
        if self.transfers.jobs.is_empty() {
            self.transfers.window = CopyWindow::default();
            host.aux_hide(AuxWindow::Transfers);
            return;
        }
        let size = self.transfers_size();
        if size != self.transfers.window.size {
            self.transfers.window.size = size;
            host.aux_resize(AuxWindow::Transfers, size);
        }
        let title = view::window_title(&self.transfers.jobs);
        if title != self.transfers.window.title {
            host.aux_set_title(AuxWindow::Transfers, &title);
            self.transfers.window.title = title;
        }
        host.aux_invalidate(AuxWindow::Transfers);
    }

    /// Avance total de la cola para el indicador de la barra de estado (`None` sin trabajos).
    pub(super) fn transfers_progress(&self) -> Option<f32> {
        let jobs = &self.transfers.jobs;
        if jobs.is_empty() {
            return None;
        }
        let open: Vec<&JobView> = jobs.iter().filter(|j| j.state != JobState::Done).collect();
        if open.is_empty() {
            return Some(1.0);
        }
        let (done, total) = open.iter().fold((0u64, 0u64), |(d, t), j| (d + j.bytes.0.min(j.bytes.1), t + j.bytes.1));
        Some(if total == 0 { 0.0 } else { (done as f64 / total as f64) as f32 })
    }

    /// Solo quedan trabajos que terminaron con errores (el indicador lo marca en vez del 100 %).
    pub(super) fn transfers_failed(&self) -> bool {
        !self.transfers.jobs.is_empty() && self.transfers.jobs.iter().all(|j| j.state == JobState::Done)
    }

    /// La ventana de copia se pinta (la llama la plataforma desde su ventana).
    pub fn paint_aux(&mut self, w: AuxWindow, p: &dyn Painter, width: f32, height: f32) {
        match w {
            AuxWindow::Transfers => {
                let expanded = self.transfers_expanded();
                view::draw(p, rect(0.0, 0.0, width, height), &self.transfers.jobs, expanded, self.transfers.window.hover, sys::env());
            }
        }
    }

    /// El mouse en la ventana de copia (DIPs de su área).
    pub fn aux_mouse(&mut self, w: AuxWindow, x: f32, y: f32, ev: AuxMouse) {
        let AuxWindow::Transfers = w;
        let (width, height) = self.transfers_size();
        let at = view::hit(rect(0.0, 0.0, width, height), &self.transfers.jobs, self.transfers_expanded(), x, y);
        let host = self.host.clone();
        match ev {
            AuxMouse::Move => {
                if self.transfers.window.hover != at {
                    self.transfers.window.hover = at;
                    host.aux_invalidate(w);
                }
            }
            AuxMouse::Leave => {
                if self.transfers.window.hover.take().is_some() {
                    host.aux_invalidate(w);
                }
            }
            AuxMouse::Down => self.transfers.window.pressed = at,
            AuxMouse::Up => {
                let pressed = self.transfers.window.pressed.take();
                if let Some(h) = at.filter(|h| pressed == Some(*h)) {
                    self.on_view_hit(h);
                    self.transfers.window.hover = None;
                    self.sync_transfers_window();
                    self.redraw();
                }
            }
        }
    }

    fn on_view_hit(&mut self, h: ViewHit) {
        if let ViewHit::Choose(id, _) | ViewHit::ApplyAll(id) = h
            && self.transfers.jobs.iter().any(|j| j.id == id && j.conflict_since.is_some_and(|t| t.elapsed() < CONFLICT_GUARD))
        {
            // El bloque recién apareció bajo el mouse: el clic era para otra cosa.
            return;
        }
        match h {
            ViewHit::Pause(id) => self.pause_job(id),
            ViewHit::Resume(id) => self.resume_job(id),
            ViewHit::Cancel(id) | ViewHit::Dismiss(id) => self.cancel_job(id),
            ViewHit::Expand(id) => {
                let w = &mut self.transfers.window;
                if !w.collapsed && self.transfers_expanded() == Some(id) {
                    self.transfers.window.collapsed = true;
                } else {
                    self.transfers.window.collapsed = false;
                    self.transfers.window.chosen = Some(id);
                }
            }
            ViewHit::Choose(id, choice) => {
                let all = self.transfers.jobs.iter().any(|j| j.id == id && j.apply_all);
                self.answer_job(id, Answer { choice, all });
            }
            ViewHit::ApplyAll(id) => {
                if let Some(v) = self.job_mut(id) {
                    v.apply_all = !v.apply_all;
                }
            }
            ViewHit::Retry(id) => {
                self.retry_job(id);
            }
        }
    }

    /// Una tecla en la ventana de copia: Esc la cierra (los trabajos siguen). `true` si se usó.
    pub fn aux_key(&mut self, w: AuxWindow, vk: Key) -> bool {
        if vk == VK_ESCAPE {
            self.host.clone().aux_hide(w);
            self.aux_closed(w);
            return true;
        }
        false
    }

    /// El usuario cerró la ventana: se oculta, los trabajos siguen y queda el indicador.
    pub fn aux_closed(&mut self, w: AuxWindow) {
        let AuxWindow::Transfers = w;
        let win = &mut self.transfers.window;
        win.shown = false;
        win.hover = None;
        win.pressed = None;
        self.redraw();
    }

    /// Comandos de prueba (feature `pruebas`): `copiar <destino>|<origen>|…` y `mover …` lanzan un
    /// trabajo; `ventana <control>` hace clic en la ventana de copia sobre el control del primer
    /// trabajo que lo tenga (`pausa`, `reanudar`, `cancelar`, `detalles`, `reemplazar`, `ambos`,
    /// `omitir`, `combinar`, `todos`, `reintentar`, `cerrar`). `comprimir <a>|<b>…`, `extraer_aqui
    /// <archivo>` y `extraer_en <archivo>` lanzan esos trabajos.
    #[cfg(feature = "pruebas")]
    pub(super) fn transfer_test(&mut self, verb: &str, rest: &str) {
        let parts: Vec<PathBuf> = rest.trim().split('|').map(PathBuf::from).collect();
        match verb {
            "copiar" | "mover" if parts.len() >= 2 => {
                let (dest, sources) = (parts[0].clone(), parts[1..].to_vec());
                self.start_transfer(if verb == "mover" { JobSpec::Move { sources, dest } } else { JobSpec::Copy { sources, dest } });
            }
            "comprimir" if !rest.trim().is_empty() => {
                self.start_transfer(JobSpec::Compress { sources: parts });
            }
            "extraer_aqui" | "extraer_en" if !rest.trim().is_empty() => {
                let archive = parts[0].clone();
                self.start_transfer(JobSpec::Extract { archive, into_folder: verb == "extraer_en" });
            }
            "ventana" => {
                let what = rest.trim();
                let wanted = |id: JobId| -> Option<ViewHit> {
                    Some(match what {
                        "pausa" => ViewHit::Pause(id),
                        "reanudar" => ViewHit::Resume(id),
                        "cancelar" => ViewHit::Cancel(id),
                        "detalles" => ViewHit::Expand(id),
                        "reemplazar" => ViewHit::Choose(id, Choice::Replace),
                        "ambos" => ViewHit::Choose(id, Choice::KeepBoth),
                        "omitir" => ViewHit::Choose(id, Choice::Skip),
                        "combinar" => ViewHit::Choose(id, Choice::Merge),
                        "todos" => ViewHit::ApplyAll(id),
                        "reintentar" => ViewHit::Retry(id),
                        "cerrar" => ViewHit::Dismiss(id),
                        _ => return None,
                    })
                };
                let (w, h) = self.transfers_size();
                let r = rect(0.0, 0.0, w, h);
                let jobs = self.transfers.jobs.clone();
                let expanded = self.transfers_expanded();
                for j in &jobs {
                    let Some(target) = wanted(j.id) else { return };
                    let mut y = 0.0;
                    while y < h {
                        let mut x = 0.0;
                        while x < w {
                            if view::hit(r, &jobs, expanded, x, y) == Some(target) {
                                for ev in [AuxMouse::Move, AuxMouse::Down, AuxMouse::Up] {
                                    self.aux_mouse(AuxWindow::Transfers, x + 1.0, y + 1.0, ev);
                                }
                                return;
                            }
                            x += 4.0;
                        }
                        y += 4.0;
                    }
                }
            }
            _ => {}
        }
    }

    /// Antes de cerrar xdir: con trabajos en curso pregunta si se cancelan; si sí, los cancela y
    /// espera (hasta 5 s) que borren lo a medio copiar. `false`: no se cierra.
    pub fn confirm_close(&mut self) -> bool {
        let n = self.running_jobs();
        if n == 0 {
            return true;
        }
        let host = self.host.clone();
        if host.ask_yes_no_cancel("xdir", &close_question(n)) != Some(true) {
            return false;
        }
        for m in self.transfers.meta.values() {
            m.ctl.cancel();
        }
        let until = Instant::now() + std::time::Duration::from_secs(5);
        while self.running_jobs() > 0 && Instant::now() < until {
            self.on_transfer_updates();
            if self.running_jobs() > 0 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        true
    }

    fn on_job_done(&mut self, id: JobId, outcome: JobOutcome) {
        let label = self.transfers.meta.get(&id).map_or("La operación", |m| m.label);
        let previous = self.notice.take();
        self.on_op_done(outcome.result);
        self.notice = if let Some(why) = &outcome.stopped {
            Some(format!("{label}: {why}"))
        } else if !outcome.errors.is_empty() {
            let first = &outcome.errors[0];
            let name = first.shown_name();
            Some(format!("{label} terminó con {} («{name}»: {})", plural_errors(outcome.errors.len()), first.message))
        } else if !outcome.skipped.is_empty() {
            Some(skipped_text(outcome.skipped.len()))
        } else if outcome.omitted > 0 {
            Some(omitted_text(outcome.omitted))
        } else if !self.auto_skip()
            && let Some(c) = self.transfers.jobs.iter().find_map(|j| j.conflict.as_ref())
        {
            // Otro trabajo espera una respuesta: su aviso sigue a la vista.
            Some(format!("Ya existe «{}» en «{}»", c.name, c.folder))
        } else {
            // No se borra un aviso de otro trabajo (los omitidos, por ejemplo).
            previous.filter(|_| self.running_jobs() > 0)
        };
        // Un zip con contraseña o en un formato que no se sabe leer: se ofrece abrirlo con el sistema.
        if let Some(e) = outcome.errors.iter().find(|e| e.message == sys::PASSWORD || e.message == sys::UNSUPPORTED)
            && let Some(JobSpec::Extract { archive, .. }) = self.transfers.meta.get(&id).and_then(|m| m.rerun.clone())
        {
            let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let q = format!("«{name}»: {}. ¿Abrir con el sistema?", e.message);
            if self.host.clone().ask_yes_no("Abrir con el sistema", &q) {
                sys::shell().open_item(self.hwnd, &archive.into());
            }
        }
        // Los que terminaron bien se van; los que tienen errores quedan para «Reintentar».
        if outcome.errors.is_empty() && outcome.stopped.is_none() {
            self.forget_job(id);
        } else if let Some(m) = self.transfers.meta.get_mut(&id) {
            // Ya no usa el disco (no frena «Expulsar»).
            m.paths.clear();
        }
        self.run_pending_eject();
    }

    /// Los «Expulsar» cuyos trabajos ya terminaron todos, siguen (en el orden en que se pidieron).
    fn run_pending_eject(&mut self) {
        let pending = std::mem::take(&mut self.transfers.pending_eject);
        let (ready, waiting): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .partition(|e| !e.jobs.iter().any(|id| self.transfers.jobs.iter().any(|j| j.id == *id && j.state != JobState::Done)));
        self.transfers.pending_eject = waiting;
        for e in ready {
            (e.then)(self);
        }
    }

    /// Con la ventana de copia se pregunta; sin ella (Linux sin interfaz), «Omitir».
    pub fn set_auto_skip(&mut self, on: bool) {
        self.transfers.auto_skip.store(on, Ordering::SeqCst);
    }

    fn auto_skip(&self) -> bool {
        self.transfers.auto_skip.load(Ordering::SeqCst)
    }

    fn forget_job(&mut self, id: JobId) {
        self.transfers.jobs.retain(|j| j.id != id);
        self.transfers.meta.remove(&id);
    }

    fn job_ctl(&self, id: JobId) -> Option<Arc<Control>> {
        self.transfers.meta.get(&id).map(|m| m.ctl.clone())
    }

    fn job_mut(&mut self, id: JobId) -> Option<&mut JobView> {
        self.transfers.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn pause_job(&mut self, id: JobId) {
        if let Some(c) = self.job_ctl(id) {
            c.pause();
        }
        if let Some(v) = self.job_mut(id) {
            v.paused = true;
            v.speed.pause(Instant::now());
        }
    }

    pub fn resume_job(&mut self, id: JobId) {
        if let Some(c) = self.job_ctl(id) {
            c.resume();
        }
        if let Some(v) = self.job_mut(id) {
            v.paused = false;
            v.speed.resume(Instant::now());
        }
    }

    /// Cancela (también uno que espera en la cola o un conflicto): lo a medio copiar se borra.
    pub fn cancel_job(&mut self, id: JobId) {
        if let Some(c) = self.job_ctl(id) {
            c.cancel();
        }
        // Uno ya terminado (con errores) se descarta.
        if self.job_mut(id).is_some_and(|v| v.state == JobState::Done) {
            self.forget_job(id);
            self.sync_transfers_window();
        }
    }

    /// Responde el conflicto que espera el trabajo.
    pub fn answer_job(&mut self, id: JobId, answer: Answer) {
        if let Some(c) = self.job_ctl(id) {
            c.answer(answer);
        }
        if let Some(v) = self.job_mut(id)
            && v.state == JobState::Conflict
        {
            v.conflict = None;
            v.state = JobState::Running;
            v.speed.resume(Instant::now());
        }
    }

    /// «Reintentar»: un trabajo nuevo con lo que falló de uno terminado.
    pub fn retry_job(&mut self, id: JobId) -> Option<JobId> {
        let v = self.transfers.jobs.iter().find(|j| j.id == id && j.state == JobState::Done)?;
        // Los avisos (lo que quedó con otro nombre) no se reintentan.
        let pairs: Vec<_> = v.errors.iter().filter(|e| e.retry).map(|e| (e.src.clone(), e.dst.clone())).collect();
        let moving = self.transfers.meta.get(&id).is_some_and(|m| m.moving);
        let restore = self.transfers.meta.get(&id).is_some_and(|m| m.restore);
        let rerun = self.transfers.meta.get(&id).and_then(|m| m.rerun.clone());
        let roots = v.route.sources.clone();
        self.forget_job(id);
        let spec = if let Some(spec) = rerun {
            // Comprimir y extraer se vuelven a hacer enteros (si hay algo que valga reintentar).
            if pairs.is_empty() {
                self.sync_transfers_window();
                return None;
            }
            spec
        } else if restore {
            // Lo de adentro de una carpeta devuelta no tiene lugar original propio: se devuelve
            // otra vez esa carpeta (lo elegido en la Papelera que la contiene).
            let mut items: Vec<PathBuf> = Vec::new();
            for (src, _) in pairs {
                let top = roots.iter().find(|r| src.starts_with(r)).cloned().unwrap_or(src);
                if !items.contains(&top) {
                    items.push(top);
                }
            }
            JobSpec::Restore { items }
        } else {
            JobSpec::Retry { moving, pairs }
        };
        let again = (spec_count(&spec) > 0).then(|| self.start_transfer(spec));
        self.sync_transfers_window();
        again
    }

    /// Los trabajos lanzados desde la última vez (pruebas).
    #[cfg(test)]
    pub(super) fn take_started(&mut self) -> Vec<JobSpec> {
        std::mem::take(&mut self.transfers.started)
    }

    /// Los trabajos (para la ventana de copia).
    pub fn jobs(&self) -> &[JobView] {
        &self.transfers.jobs
    }

    /// Trabajos sin terminar.
    pub fn running_jobs(&self) -> usize {
        self.transfers.jobs.iter().filter(|j| j.state != JobState::Done).count()
    }

    /// Trabajos sin terminar que tocan algo dentro de `root` (un disco que se quiere expulsar).
    pub fn jobs_on_volume(&self, root: &Path) -> Vec<JobId> {
        let id = self.root_volume(root);
        self.transfers
            .jobs
            .iter()
            .filter(|j| j.state != JobState::Done)
            .filter(|j| {
                self.transfers.meta.get(&j.id).is_some_and(|m| {
                    m.paths.iter().any(|p| p.starts_with(root)) || id.as_ref().is_some_and(|id| m.volumes.contains(id))
                })
            })
            .map(|j| j.id)
            .collect()
    }

    /// La identidad del disco de `root`, de lo que dejó la tarea que lista los expulsables (sin
    /// mirar el disco desde la interfaz). `None` si no se sabe: se compara solo por la ruta.
    fn root_volume(&self, root: &Path) -> Option<OsString> {
        self.transfers.root_ids.get(root).cloned()
    }

    /// Las identidades de los discos expulsables (las calcula la tarea que los lista, en su hilo).
    pub(super) fn remember_volume_ids(&mut self, ids: Vec<(PathBuf, OsString)>) {
        self.transfers.root_ids = ids.into_iter().collect();
    }

    /// Terminó de expulsar `root` (bien o mal): ya pueden empezar copias sobre él.
    pub(super) fn eject_done(&mut self, root: &Path) {
        self.transfers.ejecting.lock().unwrap_or_else(|e| e.into_inner()).retain(|e| e.root != root);
    }

    /// Antes de expulsar `root`: si hay trabajos sobre él, pregunta si se cancelan y, si sí, los
    /// cancela y espera que terminen (cancelar borra lo a medio copiar, en el disco) antes de
    /// seguir con `then` (soltar el disco). Sin trabajos, sigue enseguida. Nunca bloquea: `then`
    /// corre desde `on_transfer_updates` cuando llega el último `Finished`. `false` si no se
    /// expulsa (dijo que no).
    pub fn eject_after_jobs(&mut self, root: &Path, label: &str, then: AfterJobs) -> bool {
        let ids = self.jobs_on_volume(root);
        let ejecting = Ejecting { root: root.to_path_buf(), volume: self.root_volume(root), label: label.to_string() };
        if ids.is_empty() {
            // Hasta que termine de expulsar (`eject_done`) no empiezan copias sobre el disco.
            self.transfers.ejecting.lock().unwrap_or_else(|e| e.into_inner()).push(ejecting);
            then(self);
            return true;
        }
        let host = self.host.clone();
        if host.ask_yes_no_cancel("Expulsar", &eject_question(ids.len(), label)) != Some(true) {
            return false;
        }
        for &id in &ids {
            if let Some(c) = self.job_ctl(id) {
                c.cancel();
            }
        }
        self.transfers.ejecting.lock().unwrap_or_else(|e| e.into_inner()).push(ejecting);
        self.transfers.pending_eject.push(PendingEject { jobs: ids, then });
        true
    }
}

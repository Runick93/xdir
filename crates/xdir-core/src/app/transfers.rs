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
use std::sync::Arc;
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
    /// «Copiar», «Duplicar»… para los avisos.
    label: &'static str,
    /// Rutas que toca (para saber si usa un disco que se quiere expulsar).
    paths: Vec<PathBuf>,
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
    auto_skip: bool,
    /// Expulsar espera que terminen (y limpien) los trabajos cancelados sobre el disco; varios
    /// pedidos esperan cada uno los suyos.
    pending_eject: Vec<(Vec<JobId>, AfterJobs)>,
    window: CopyWindow,
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
    /// El último tamaño pedido (para no repetir `aux_resize`).
    size: (f32, f32),
}

/// Lo que sigue cuando terminan unos trabajos (soltar el disco al expulsar).
pub type AfterJobs = Box<dyn FnOnce(&mut App)>;

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
            auto_skip: true,
            pending_eject: Vec::new(),
            window: CopyWindow::default(),
        }
    }
}

/// Rutas que toca un trabajo.
fn spec_paths(spec: &JobSpec) -> Vec<PathBuf> {
    fn undo_paths(u: &Undo, out: &mut Vec<PathBuf>) {
        match u {
            Undo::Batch(v) => v.iter().for_each(|u| undo_paths(u, out)),
            Undo::Created(v) => out.extend(v.iter().cloned()),
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
    }
    out
}

/// Cuántos elementos eligió el usuario (el título antes de recorrer).
fn spec_count(spec: &JobSpec) -> u64 {
    (match spec {
        JobSpec::Copy { sources, .. } | JobSpec::Move { sources, .. } | JobSpec::Duplicate { sources } => sources.len(),
        JobSpec::NewFolderWith { items, .. } | JobSpec::Restore { items } => items.len(),
        JobSpec::Retry { pairs, .. } => pairs.len(),
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

/// «Se omitieron 3 elementos que ya existían».
fn skipped_text(n: usize) -> String {
    if n == 1 { "Se omitió 1 elemento que ya existía".into() } else { format!("Se omitieron {n} elementos que ya existían") }
}

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
    pub fn start_transfer(&mut self, spec: JobSpec) -> JobId {
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
        });
        t.meta.insert(id, JobMeta { ctl: ctl.clone(), moving, label, paths: paths.clone() });
        let (tx, hwnd, sched, auto_skip) = (t.tx.clone(), self.hwnd, t.sched.clone(), t.auto_skip);
        std::thread::spawn(move || {
            let prims = sys::copy_prims();
            let send = |u: Update| {
                if tx.send((id, u)).is_ok() {
                    hwnd.post(WM_APP_TRANSFER, 0, 0);
                }
            };
            let vols = volumes(prims, &paths);
            if !sched.acquire_notify(id, &vols, &ctl, &mut || send(Update::Waiting)) {
                send(Update::Finished(JobOutcome { cancelled: true, ..Default::default() }));
                return;
            }
            let _turn = Release(sched, id);
            transfer::run(spec, prims, &ctl, &mut |u| {
                if auto_skip && matches!(u, Update::Conflict(_)) {
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
        while let Ok((id, update)) = self.transfers.rx.try_recv() {
            let Some(v) = self.transfers.jobs.iter_mut().find(|j| j.id == id) else { continue };
            match update {
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
                    if !self.transfers.auto_skip {
                        self.notice = Some(format!("Ya existe «{}» en «{}»", info.name, info.folder));
                    }
                    v.conflict = Some(info);
                    v.state = JobState::Conflict;
                    v.apply_all = false;
                    show = show || !self.transfers.auto_skip;
                }
                Update::Finished(outcome) => {
                    v.state = JobState::Done;
                    v.conflict = None;
                    v.errors = outcome.errors.clone();
                    v.stopped = outcome.stopped.clone();
                    finished.push((id, outcome));
                }
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
        if host.aux_show(AuxWindow::Transfers, &view::window_title(&self.transfers.jobs), size) {
            let w = &mut self.transfers.window;
            w.shown = true;
            w.size = size;
            host.aux_invalidate(AuxWindow::Transfers);
        } else {
            self.transfers.window.shown = false;
            if !self.transfers.auto_skip {
                self.transfers.auto_skip = true;
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
    /// `omitir`, `combinar`, `todos`, `reintentar`, `cerrar`).
    #[cfg(feature = "pruebas")]
    pub(super) fn transfer_test(&mut self, verb: &str, rest: &str) {
        let parts: Vec<PathBuf> = rest.trim().split('|').map(PathBuf::from).collect();
        match verb {
            "copiar" | "mover" if parts.len() >= 2 => {
                let (dest, sources) = (parts[0].clone(), parts[1..].to_vec());
                self.start_transfer(if verb == "mover" { JobSpec::Move { sources, dest } } else { JobSpec::Copy { sources, dest } });
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
            let name = first.src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(format!("{label} terminó con {} («{name}»: {})", plural_errors(outcome.errors.len()), first.message))
        } else if !outcome.skipped.is_empty() {
            Some(skipped_text(outcome.skipped.len()))
        } else if !self.transfers.auto_skip
            && let Some(c) = self.transfers.jobs.iter().find_map(|j| j.conflict.as_ref())
        {
            // Otro trabajo espera una respuesta: su aviso sigue a la vista.
            Some(format!("Ya existe «{}» en «{}»", c.name, c.folder))
        } else {
            // No se borra un aviso de otro trabajo (los omitidos, por ejemplo).
            previous.filter(|_| self.running_jobs() > 0)
        };
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
            .partition(|(ids, _)| !ids.iter().any(|id| self.transfers.jobs.iter().any(|j| j.id == *id && j.state != JobState::Done)));
        self.transfers.pending_eject = waiting;
        for (_, then) in ready {
            then(self);
        }
    }

    /// Con la ventana de copia se pregunta; sin ella (hasta la Task 12), «Omitir».
    pub fn set_auto_skip(&mut self, on: bool) {
        self.transfers.auto_skip = on;
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
        self.forget_job(id);
        let again = (!pairs.is_empty()).then(|| self.start_transfer(JobSpec::Retry { moving, pairs }));
        self.sync_transfers_window();
        again
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
        self.transfers
            .jobs
            .iter()
            .filter(|j| j.state != JobState::Done)
            .filter(|j| self.transfers.meta.get(&j.id).is_some_and(|m| m.paths.iter().any(|p| p.starts_with(root))))
            .map(|j| j.id)
            .collect()
    }

    /// Antes de expulsar `root`: si hay trabajos sobre él, pregunta si se cancelan y, si sí, los
    /// cancela y espera que terminen (cancelar borra lo a medio copiar, en el disco) antes de
    /// seguir con `then` (soltar el disco). Sin trabajos, sigue enseguida. Nunca bloquea: `then`
    /// corre desde `on_transfer_updates` cuando llega el último `Finished`. `false` si no se
    /// expulsa (dijo que no).
    // TODO(merge tanda D): lo llama «Expulsar» (Task 5, otra rama) con lo que suelta el disco.
    pub fn eject_after_jobs(&mut self, root: &Path, label: &str, then: AfterJobs) -> bool {
        let ids = self.jobs_on_volume(root);
        if ids.is_empty() {
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
        self.transfers.pending_eject.push((ids, then));
        true
    }
}

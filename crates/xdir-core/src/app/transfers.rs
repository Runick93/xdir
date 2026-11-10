//! Trabajos del motor de copia (`transfer`) en la `App`: cada uno corre en su hilo con
//! `sys::copy_prims()`, espera su turno en la cola (`Scheduler`) si comparte un disco con otro, y
//! manda su estado por canal con `WM_APP_TRANSFER`. El hilo de interfaz nunca espera al disco:
//! solo arma los `JobView` con lo que llega.
//!
//! Todavía sin ventana de copia (Tasks 11–12): un conflicto o un error se ve como aviso en la barra
//! de estado; responder, pausar, cancelar y reintentar ya están (`answer_job`, `pause_job`…).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use super::App;
use crate::ops::Undo;
use crate::sys::{self, CopyPrims};
use crate::transfer::sched::Scheduler;
use crate::transfer::speed::Speed;
use crate::transfer::{self, Answer, Control, JobId, JobOutcome, JobSpec, JobState, JobView, Update, WM_APP_TRANSFER};

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
}

impl Transfers {
    pub(super) fn new() -> Transfers {
        let (tx, rx) = channel();
        Transfers { tx, rx, jobs: Vec::new(), meta: HashMap::new(), next: 1, sched: Arc::new(Scheduler::new()) }
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
            title: transfer::title(&spec, 0),
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
        });
        t.meta.insert(id, JobMeta { ctl: ctl.clone(), moving, label, paths: paths.clone() });
        let (tx, hwnd, sched) = (t.tx.clone(), self.hwnd, t.sched.clone());
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
            transfer::run(spec, prims, &ctl, &mut |u| send(u));
        });
        id
    }

    /// Vacía el canal de los trabajos (`WM_APP_TRANSFER`).
    pub fn on_transfer_updates(&mut self) {
        let now = Instant::now();
        let mut finished = Vec::new();
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
                    v.speed.push(now, bytes);
                    if v.state != JobState::Conflict {
                        v.state = JobState::Running;
                    }
                }
                Update::Conflict(info) => {
                    // TODO(task 12): la ventana de copia muestra el conflicto con sus botones.
                    self.notice = Some(format!("Ya existe «{}» en «{}»", info.name, info.folder));
                    v.conflict = Some(info);
                    v.state = JobState::Conflict;
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
            self.on_job_done(id, outcome);
        }
    }

    fn on_job_done(&mut self, id: JobId, outcome: JobOutcome) {
        let label = self.transfers.meta.get(&id).map_or("La operación", |m| m.label);
        self.on_op_done(outcome.result);
        self.notice = if let Some(why) = &outcome.stopped {
            Some(format!("{label}: {why}"))
        } else if !outcome.errors.is_empty() {
            let first = &outcome.errors[0];
            let name = first.src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(format!("{label} terminó con {} («{name}»: {})", plural_errors(outcome.errors.len()), first.message))
        } else {
            // Si otro trabajo espera una respuesta, su aviso sigue a la vista.
            self.transfers.jobs.iter().find_map(|j| j.conflict.as_ref()).map(|c| format!("Ya existe «{}» en «{}»", c.name, c.folder))
        };
        // Los que terminaron bien se van; los que tienen errores quedan para «Reintentar».
        if outcome.errors.is_empty() && outcome.stopped.is_none() {
            self.forget_job(id);
        } else if let Some(m) = self.transfers.meta.get_mut(&id) {
            // Ya no usa el disco (no frena «Expulsar»).
            m.paths.clear();
        }
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
        }
    }

    pub fn resume_job(&mut self, id: JobId) {
        if let Some(c) = self.job_ctl(id) {
            c.resume();
        }
        if let Some(v) = self.job_mut(id) {
            v.paused = false;
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
        }
    }

    /// «Reintentar»: un trabajo nuevo con lo que falló de uno terminado.
    pub fn retry_job(&mut self, id: JobId) -> Option<JobId> {
        let v = self.transfers.jobs.iter().find(|j| j.id == id && j.state == JobState::Done)?;
        let pairs: Vec<_> = v.errors.iter().map(|e| (e.src.clone(), e.dst.clone())).collect();
        let moving = self.transfers.meta.get(&id).is_some_and(|m| m.moving);
        self.forget_job(id);
        (!pairs.is_empty()).then(|| self.start_transfer(JobSpec::Retry { moving, pairs }))
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

    /// Antes de expulsar `root`: si hay trabajos sobre él, pregunta si se cancelan. `true` si se
    /// puede seguir expulsando (no había, o se cancelaron).
    // TODO(merge tanda D): lo llama «Expulsar» (Task 5, otra rama) antes de soltar el disco.
    pub fn confirm_eject_jobs(&mut self, root: &Path, label: &str) -> bool {
        let ids = self.jobs_on_volume(root);
        if ids.is_empty() {
            return true;
        }
        let host = self.host.clone();
        if host.ask_yes_no_cancel("Expulsar", &eject_question(ids.len(), label)) != Some(true) {
            return false;
        }
        for id in ids {
            self.cancel_job(id);
        }
        true
    }
}

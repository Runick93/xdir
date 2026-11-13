//! «Comparar contenido» (issue #59) en la `App`: qué dos archivos se comparan, el trabajo en otro
//! hilo (`compare::run_watched`, con su avance y Cancelar) y su resultado.
//!
//! Se compara con exactamente dos archivos elegidos: dos en el panel activo, o uno en el activo y
//! uno en el otro cuando se ven dos paneles. Con tamaños distintos (los de la lista) se responde
//! sin leer nada; si no, se lee byte a byte en un hilo. Nada de esto toca el disco en el hilo de la
//! ventana. Los trabajos de las sumas de verificación (de este panel y del de información) usan el
//! mismo canal (`JobReply`) y el mismo aviso (`WM_APP_COMPARE`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use super::*;
use crate::compare::{self, Failure, Progress, Source, Verdict};
use crate::fs::{ATTR_ARCHIVE_BAD, ATTR_LINK};
use crate::sys::HashAlgo;

/// Llegó el resultado de un trabajo (`wparam` 0) o pasó un rato con uno en curso (`wparam` 1: se
/// redibuja el avance).
pub(super) const WM_APP_COMPARE: u32 = WM_APP + 40;

/// Aviso cuando lo elegido no son dos archivos.
pub(super) const NEED_TWO: &str = "Para comparar, elegí dos archivos: uno en cada panel o dos en el mismo";

/// Cada cuánto se redibuja el avance de un trabajo en curso.
const TICK: Duration = Duration::from_millis(100);

/// Uno de los dos archivos: de dónde se lee y lo que se muestra (de la lista, sin leer nada).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Side {
    pub source: Source,
    pub name: String,
    /// La carpeta (o la carpeta de adentro del comprimido), como en la barra de dirección.
    pub folder: String,
    pub size: u64,
    pub icon: i32,
}

/// Un trabajo en otro hilo: su número, su corte y su avance.
pub(super) struct Job {
    pub id: u64,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<Progress>,
}

impl Job {
    pub(super) fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Lo que mandan los trabajos.
pub(super) enum JobReply {
    Compared { id: u64, result: Result<Verdict, Failure> },
    Hashed { id: u64, result: Result<String, Failure> },
}

/// Cómo va la comparación.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Status {
    Running,
    Done(Verdict),
    Failed(String),
    Cancelled,
}

/// La suma de uno de los dos archivos, pedida desde el panel.
#[derive(Default)]
pub(super) struct SideHash {
    pub job: Option<Job>,
    pub result: Option<Result<String, String>>,
}

/// El panel de comparar: los dos archivos, cómo va y sus sumas.
pub(super) struct CompareState {
    pub sides: [Side; 2],
    pub status: Status,
    pub job: Option<Job>,
    pub hashes: [SideHash; 2],
    /// El último clic fue en el panel: Esc lo cierra.
    pub focused: bool,
}

impl CompareState {
    /// Corta todo lo que está en curso.
    fn stop_all(&mut self) {
        for j in [self.job.as_ref(), self.hashes[0].job.as_ref(), self.hashes[1].job.as_ref()].into_iter().flatten() {
            j.stop();
        }
    }
}

/// Los trabajos de la `App` (comparar y sumas): el canal y el próximo número.
pub(super) struct Jobs {
    pub tx: Sender<JobReply>,
    pub rx: Receiver<JobReply>,
    next: u64,
    /// Sin avance en este tiempo, un trabajo de la red se abandona (`net::ITEM_LIMIT`; las pruebas
    /// lo achican).
    pub limit: Duration,
}

impl Jobs {
    pub(super) fn new() -> Jobs {
        let (tx, rx) = channel();
        Jobs { tx, rx, next: 0, limit: crate::net::ITEM_LIMIT }
    }
}

/// ¿Ese elemento de la lista se puede comparar? (un archivo, con un nombre que se puede leer).
fn comparable(tab: &Tab, i: usize) -> bool {
    let e = &tab.list.entries[i];
    !e.is_dir && (!tab.location.is_archive() || e.attrs & (ATTR_ARCHIVE_BAD | ATTR_LINK) == 0)
}

/// El lado del elemento `i` de la pestaña (sin leer nada).
fn side_of(tab: &Tab, i: usize) -> Option<Side> {
    let e = &tab.list.entries[i];
    let source = match &tab.location {
        Location::Path(_) => Source::Disk(tab.entry_path(i)),
        Location::Archive { file, nested, .. } => Source::Archive { file: file.clone(), nested: nested.clone(), rel: tab.archive_rel(i)? },
        _ => return None,
    };
    let folder = match &source {
        Source::Disk(p) => p.parent().map(|d| d.display().to_string()).unwrap_or_default(),
        Source::Archive { .. } => tab.location.address_text(),
    };
    Some(Side { source, name: e.display.clone(), folder, size: e.size, icon: e.icon })
}

impl App {
    /// Los dos archivos que se compararían ahora, o por qué no se puede.
    pub(super) fn compare_candidates(&self) -> Result<[Side; 2], &'static str> {
        let pick = |pane: usize| -> Option<Vec<Side>> {
            let tab = self.panes[pane].tab();
            // En el orden en que se ven en la lista.
            let sel: Vec<usize> = tab.list.selected_positions().into_iter().filter_map(|p| tab.list.entry_of_pos(p)).collect();
            if !sel.iter().all(|&i| comparable(tab, i)) {
                return None;
            }
            sel.iter().map(|&i| side_of(tab, i)).collect()
        };
        let active = pick(self.active).ok_or(NEED_TWO)?;
        let shown = self.shown_panes();
        let other = || shown.iter().copied().filter(|&p| p != self.active).collect::<Vec<_>>();
        match active.as_slice() {
            [a, b] => Ok([a.clone(), b.clone()]),
            [a] => match other().as_slice() {
                // Con dos paneles a la vista, el otro tiene que tener uno solo elegido.
                [o] => match pick(*o).as_deref() {
                    Some([b]) => Ok([a.clone(), b.clone()]),
                    _ => Err(NEED_TWO),
                },
                _ => Err(NEED_TWO),
            },
            _ => Err(NEED_TWO),
        }
    }

    /// «Comparar contenido»: con dos archivos elegidos, abre el panel y compara (tamaños distintos
    /// o el mismo archivo: al instante; si no, en otro hilo). Si no, avisa qué elegir.
    pub(super) fn compare_contents(&mut self) {
        let sides = match self.compare_candidates() {
            Ok(s) => s,
            Err(m) => {
                self.notice = Some(m.into());
                return;
            }
        };
        if let Some(old) = &mut self.compare {
            old.stop_all();
        }
        let status = if sides[0].source == sides[1].source {
            Status::Done(Verdict::SameFile)
        } else if sides[0].size != sides[1].size {
            Status::Done(Verdict::SizeDiffers { a: sides[0].size, b: sides[1].size })
        } else {
            Status::Running
        };
        let mut st = CompareState { sides, status, job: None, hashes: Default::default(), focused: false };
        if st.status == Status::Running {
            let (a, b) = (st.sides[0].source.clone(), st.sides[1].source.clone());
            let remote = a.remote_host().or_else(|| b.remote_host());
            st.job = Some(self.spawn_job(remote, move |stop, p| compare::compare_sources(&a, &b, stop, p), |id, result| JobReply::Compared { id, result }));
        }
        self.compare = Some(st);
        self.open_compare_panel();
    }

    /// Corre `work` en otro hilo, vigilado (`compare::run_watched`); el resultado vuelve por el
    /// canal de los trabajos con `WM_APP_COMPARE`, y mientras tanto se redibuja el avance.
    pub(super) fn spawn_job<T: Send + 'static>(
        &mut self,
        remote: Option<String>,
        work: impl FnOnce(&AtomicBool, &Progress) -> Result<T, Failure> + Send + 'static,
        wrap: impl FnOnce(u64, Result<T, Failure>) -> JobReply + Send + 'static,
    ) -> Job {
        self.jobs.next += 1;
        let id = self.jobs.next;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        let (hwnd, tx, limit) = (self.hwnd, self.jobs.tx.clone(), self.jobs.limit);
        let (c, p) = (cancel.clone(), progress.clone());
        let spawned = std::thread::Builder::new().name("xdir-comparar-vigia".into()).spawn(move || {
            let pw = p.clone();
            let tick = || hwnd.post(WM_APP_COMPARE, 1, 0);
            let result = compare::run_watched(remote, limit, &c, &p, TICK, &tick, move |stop| work(stop, &pw));
            // Cancelado: nadie lo espera.
            if !c.load(Ordering::Relaxed) && tx.send(wrap(id, result)).is_ok() {
                hwnd.post(WM_APP_COMPARE, 0, 0);
            }
        });
        if spawned.is_err() {
            let _ = self.jobs.tx.send(JobReply::Compared { id, result: Err(Failure::Failed("No se pudo crear un hilo".into())) });
        }
        Job { id, cancel, progress }
    }

    /// `WM_APP_COMPARE`: llegó un resultado (o hay que redibujar el avance).
    pub(super) fn on_compare_message(&mut self, wparam: usize) {
        if wparam == 1 {
            self.redraw();
            return;
        }
        while let Ok(r) = self.jobs.rx.try_recv() {
            match r {
                JobReply::Compared { id, result } => {
                    if let Some(st) = self.compare.as_mut().filter(|s| s.job.as_ref().is_some_and(|j| j.id == id)) {
                        st.job = None;
                        st.status = match result {
                            Ok(v) => Status::Done(v),
                            Err(Failure::Cancelled) => Status::Cancelled,
                            Err(Failure::Failed(m)) => Status::Failed(m),
                        };
                    }
                }
                JobReply::Hashed { id, result } => self.on_hashed(id, result),
            }
        }
        self.redraw();
    }

    /// Llegó una suma: del panel de comparar (de uno de los dos) o del de información.
    fn on_hashed(&mut self, id: u64, result: Result<String, Failure>) {
        if let Some(st) = &mut self.compare
            && let Some(h) = st.hashes.iter_mut().find(|h| h.job.as_ref().is_some_and(|j| j.id == id))
        {
            h.job = None;
            h.result = match result {
                Ok(s) => Some(Ok(s)),
                Err(Failure::Cancelled) => None,
                Err(Failure::Failed(m)) => Some(Err(m)),
            };
        }
    }

    /// Cancelar la comparación en curso.
    pub(super) fn cancel_compare(&mut self) {
        if let Some(st) = &mut self.compare
            && let Some(j) = st.job.take()
        {
            j.stop();
            st.status = Status::Cancelled;
        }
        self.redraw();
    }

    /// «Calcular SHA-256» de uno de los dos (o Cancelar, si ya se está calculando).
    pub(super) fn compare_hash(&mut self, side: usize) {
        let Some(st) = &mut self.compare else { return };
        if let Some(j) = st.hashes[side].job.take() {
            j.stop();
            self.redraw();
            return;
        }
        let src = st.sides[side].source.clone();
        let remote = src.remote_host();
        let job = self.spawn_job(remote, move |stop, p| compare::hash_source(&src, HashAlgo::Sha256, stop, p), |id, result| JobReply::Hashed { id, result });
        if let Some(st) = &mut self.compare {
            st.hashes[side] = SideHash { job: Some(job), result: None };
        }
        self.redraw();
    }

    /// Abre el panel de comparar en el lugar de la columna (lo implementa el panel).
    fn open_compare_panel(&mut self) {
        self.redraw();
    }

    /// Cierra el panel: corta lo que estaba en curso.
    pub(super) fn close_compare(&mut self) {
        if let Some(mut st) = self.compare.take() {
            st.stop_all();
        }
        self.redraw();
    }
}

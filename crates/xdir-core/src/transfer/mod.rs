//! Motor de copia propio: copiar, mover, duplicar, devolver de la Papelera y deshacer movimientos
//! archivo por archivo con las primitivas del sistema (`sys::CopyPrims`), con pausa, cancelar,
//! conflictos, errores que no cortan el trabajo y deshacer en un paso. Corre en el hilo de cada
//! trabajo; el estado llega al hilo de interfaz por canal (`Update`) con `WM_APP_TRANSFER`.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

pub use crate::sys::Flow;
use crate::ops::{OpResult, Undo};
use crate::sys::ItemStat;

pub mod engine;
pub mod sched;
pub mod speed;
pub mod view;
pub mod walk;

pub use engine::{run, title};

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;

/// Identifica un trabajo de la cola.
pub type JobId = u64;

/// Llegaron `Update`s de un trabajo al canal de la `App`.
pub const WM_APP_TRANSFER: u32 = crate::WM_APP + 27;

/// Pasó 1 s desde que empezó un trabajo (`lparam` = su `JobId`): si sigue, aparece la ventana de
/// copia.
pub const WM_APP_TRANSFER_DUE: u32 = crate::WM_APP + 28;

/// En qué está un trabajo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobState {
    Preparing,
    /// «En cola»: espera a otro que usa el mismo disco.
    Waiting,
    Running,
    /// Espera la respuesta a un conflicto.
    Conflict,
    Done,
}

/// Lo que la interfaz sabe de un trabajo (lo arma la `App` con los `Update`s).
#[derive(Clone, Debug)]
pub struct JobView {
    pub id: JobId,
    pub title: String,
    pub state: JobState,
    /// (hechos, total) de elementos y de bytes.
    pub items: (u64, u64),
    pub bytes: (u64, u64),
    pub current: String,
    pub speed: speed::Speed,
    pub conflict: Option<ConflictInfo>,
    pub errors: Vec<JobError>,
    pub stopped: Option<String>,
    pub paused: bool,
    pub started: Instant,
    /// La casilla «Hacer esto con todos» del conflicto está marcada.
    pub apply_all: bool,
    /// Desde cuándo se ve el conflicto (los clics de los primeros 400 ms no cuentan: venían para
    /// otra cosa que estaba ahí).
    pub conflict_since: Option<Instant>,
    /// De dónde a dónde, para el resumen al terminar con errores.
    pub route: JobRoute,
    /// Al terminar: cuántos elementos llegaron (`JobOutcome::done_items`).
    pub done_items: Option<u64>,
}

/// De dónde a dónde va un trabajo: «Se copiaron 1.281 de 1.284 elementos de «Descargas» a
/// «Respaldo»» y la carpeta de cada error («Descargas\Facturas»).
#[derive(Clone, Debug, Default)]
pub struct JobRoute {
    /// «Se copió» / «Se copiaron» (o «Se movió»…).
    pub done_one: &'static str,
    pub done_many: &'static str,
    /// La carpeta de lo elegido y el destino (sus nombres).
    pub from: Option<String>,
    pub to: Option<String>,
    /// Lo elegido.
    pub sources: Vec<PathBuf>,
}

/// Un trabajo del motor.
#[derive(Clone, PartialEq, Debug)]
pub enum JobSpec {
    Copy { sources: Vec<PathBuf>, dest: PathBuf },
    Move { sources: Vec<PathBuf>, dest: PathBuf },
    /// «nombre copia» al lado de cada uno.
    Duplicate { sources: Vec<PathBuf> },
    /// Crea «Carpeta nueva con N elementos» en `parent` y mueve ahí los elementos.
    NewFolderWith { parent: PathBuf, items: Vec<PathBuf> },
    /// Devuelve elementos de la Papelera a su lugar (`CopyPrims::trash_origin`).
    Restore { items: Vec<PathBuf> },
    /// Deshace un trabajo (`Moved`, `Batch` y lo que tengan adentro).
    Undo(Undo),
    /// Vuelve a intentar lo que falló: (origen, destino ya decidido).
    Retry { moving: bool, pairs: Vec<(PathBuf, PathBuf)> },
}

/// Lo que manda el hilo de un trabajo al de interfaz.
#[derive(Clone, PartialEq, Debug)]
pub enum Update {
    /// Los discos que toca el trabajo (su identidad: `VolumeInfo::id`), apenas se saben. Lo manda
    /// la cola de la `App`, no el motor.
    Volumes(Vec<std::ffi::OsString>),
    /// Recorriendo el origen: «Preparando… N elementos».
    Preparing { items: u64 },
    Started { title: String, items: u64, bytes: u64 },
    /// Espera a otro trabajo que usa el mismo disco («En cola»).
    Waiting,
    /// Hechos hasta ahora y el elemento actual (a lo sumo uno cada 100 ms).
    Progress { items: u64, bytes: u64, current: String },
    /// El destino ya existe: el trabajo espera `Control::answer`.
    Conflict(ConflictInfo),
    Finished(JobOutcome),
}

#[derive(Clone, PartialEq, Debug)]
pub struct ConflictInfo {
    /// Nombre del elemento que se copia.
    pub name: String,
    /// Carpeta donde ya existe.
    pub folder: String,
    pub is_dir: bool,
    /// Las dos son carpetas: se ofrece «Combinar».
    pub can_merge: bool,
    pub incoming: ItemStat,
    pub existing: ItemStat,
    /// Otros conflictos de la misma clase (archivos o carpetas) que quedan por delante en este
    /// trabajo, para «Hacer esto con todos»; `None` si no se puede saber (adentro de una carpeta
    /// que se combina, o con una carpeta por delante que se podría combinar y tener más).
    pub remaining: Option<usize>,
}

/// Algo que no se pudo hacer: el par sirve para «Reintentar».
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JobError {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub message: String,
    /// Se puede «Reintentar» (no, si es un aviso de lo que quedó con otro nombre).
    pub retry: bool,
}

/// Cómo terminó un trabajo.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct JobOutcome {
    /// Deshacer, selección y foco, como las demás operaciones (`App::on_op_done`).
    pub result: OpResult,
    pub errors: Vec<JobError>,
    /// Se detuvo sin terminar (o no empezó): el motivo.
    pub stopped: Option<String>,
    pub cancelled: bool,
    /// Lo que no se llevó porque ya existía y se eligió «Omitir» (los de origen).
    pub skipped: Vec<PathBuf>,
    /// Elementos que llegaron (para «Se copiaron N de M»): no cuenta lo omitido ni lo que falló,
    /// tampoco lo de adentro de una carpeta que no se pudo crear.
    pub done_items: u64,
}

/// Qué hacer con un conflicto (el destino ya existe).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Replace,
    KeepBoth,
    Skip,
    /// Solo carpetas sobre carpetas: entrar y resolver elemento por elemento.
    Merge,
}

/// La respuesta a un conflicto; `all`: «Hacer esto con todos» (los que quedan de ese trabajo, por
/// separado para archivos y para carpetas).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Answer {
    pub choice: Choice,
    pub all: bool,
}

#[derive(Default)]
struct ControlState {
    paused: bool,
    cancelled: bool,
    answer: Option<Answer>,
}

/// Mando de un trabajo, compartido entre el hilo de interfaz (pausa, cancelar, responder) y el
/// del trabajo (que se detiene en `checkpoint` y espera en `wait_answer`).
#[derive(Default)]
pub struct Control {
    state: Mutex<ControlState>,
    changed: Condvar,
}

impl Control {
    pub fn new() -> Arc<Control> {
        Arc::new(Control::default())
    }

    fn lock(&self) -> MutexGuard<'_, ControlState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, f: impl FnOnce(&mut ControlState)) {
        f(&mut self.lock());
        self.changed.notify_all();
    }

    pub fn pause(&self) {
        self.update(|s| s.paused = true);
    }

    pub fn resume(&self) {
        self.update(|s| s.paused = false);
    }

    pub fn cancel(&self) {
        self.update(|s| s.cancelled = true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    /// Responde el conflicto que está esperando el trabajo.
    pub fn answer(&self, a: Answer) {
        self.update(|s| s.answer = Some(a));
    }

    /// Olvida una respuesta vieja (antes de preguntar de nuevo).
    pub(crate) fn clear_answer(&self) {
        self.lock().answer = None;
    }

    /// Lo llama el trabajo en cada aviso de progreso: bloquea mientras está en pausa; `Stop` si
    /// se canceló.
    pub(crate) fn checkpoint(&self) -> Flow {
        let mut s = self.lock();
        loop {
            if s.cancelled {
                return Flow::Stop;
            }
            if !s.paused {
                return Flow::Continue;
            }
            s = self.changed.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Espera la respuesta a un conflicto; `None` si se canceló.
    pub(crate) fn wait_answer(&self) -> Option<Answer> {
        let mut s = self.lock();
        loop {
            if s.cancelled {
                return None;
            }
            if let Some(a) = s.answer.take() {
                return Some(a);
            }
            s = self.changed.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }
}

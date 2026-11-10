//! Motor de copia propio: copiar, mover, duplicar, devolver de la Papelera y deshacer movimientos
//! archivo por archivo con las primitivas del sistema (`sys::CopyPrims`), con pausa, cancelar,
//! conflictos, errores que no cortan el trabajo y deshacer en un paso. Corre en el hilo de cada
//! trabajo; el estado llega al hilo de interfaz por canal (`Update`) con `WM_APP_TRANSFER`.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

pub use crate::sys::Flow;
use crate::ops::{OpResult, Undo};
use crate::sys::ItemStat;

pub mod engine;
mod naming;
pub mod speed;
pub mod walk;

pub use engine::{run, title};

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod tests;

/// Identifica un trabajo de la cola.
pub type JobId = u64;

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
    /// Otros conflictos que se ven por delante en este trabajo (para «Hacer esto con todos»).
    pub remaining: usize,
}

/// Algo que no se pudo hacer: el par sirve para «Reintentar».
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JobError {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub message: String,
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

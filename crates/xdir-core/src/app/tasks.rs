//! Trabajos sueltos en otro hilo (renombrar varios, saber si un disco distingue mayúsculas): el
//! hilo de interfaz nunca espera al disco. Cada trabajo vuelve por el canal `task_tx` con un
//! `TaskDone` y avisa con `WM_APP_TASK`.
//
// La Task 4 de la tanda D (otra rama) crea este mismo módulo con `TaskDone::Original`; al juntar
// las ramas, las variantes se suman.

use super::*;

/// Terminó un trabajo de `spawn_task`; el resultado está en el canal.
pub const WM_APP_TASK: u32 = WM_APP + 26;

/// Lo que devuelve un trabajo suelto.
pub(super) enum TaskDone {
    /// Terminó Renombrar varios (o su deshacer, con `undoing`) en la carpeta `dir`.
    Renamed { dir: PathBuf, result: Result<Undo, String>, undoing: bool },
    /// Si el disco de `dir` distingue mayúsculas (para los choques de Renombrar varios).
    CaseInfo { dir: PathBuf, case_insensitive: bool },
}

impl App {
    /// Corre `f` en un hilo; su resultado vuelve por el canal y se procesa con `WM_APP_TASK`.
    pub(super) fn spawn_task(&self, f: impl FnOnce() -> TaskDone + Send + 'static) {
        let (tx, hwnd) = (self.task_tx.clone(), self.hwnd);
        std::thread::spawn(move || {
            if tx.send(f()).is_ok() {
                hwnd.post(WM_APP_TASK, 0, 0);
            }
        });
    }

    /// `WM_APP_TASK`: procesa lo que llegó.
    pub(super) fn on_tasks(&mut self) {
        while let Ok(t) = self.task_rx.try_recv() {
            self.on_task_done(t);
        }
        self.redraw();
    }

    pub(super) fn on_task_done(&mut self, t: TaskDone) {
        match t {
            TaskDone::Renamed { dir, result, undoing } => self.on_renamed(dir, result, undoing),
            TaskDone::CaseInfo { dir, case_insensitive } => self.on_case_info(&dir, case_insensitive),
        }
    }
}

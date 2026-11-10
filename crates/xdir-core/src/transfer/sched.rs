//! Quién espera a quién: los trabajos corren a la vez salvo que compartan un disco (de origen o
//! de destino); entonces esperan «En cola», en orden de llegada, para no hacer saltar el disco.

use std::ffi::OsString;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::{Control, JobId};

/// Cada cuánto mira si lo cancelaron mientras espera.
const POLL: Duration = Duration::from_millis(50);

struct Entry {
    id: JobId,
    vols: Vec<OsString>,
    running: bool,
}

#[derive(Default)]
pub struct Scheduler {
    entries: Mutex<Vec<Entry>>,
    changed: Condvar,
}

impl Scheduler {
    pub fn new() -> Scheduler {
        Scheduler::default()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Espera su turno: bloquea mientras un trabajo anterior (id menor) que corre o espera, o uno
    /// que ya corre, comparte un disco con `vols`. `false` si se canceló esperando.
    pub fn acquire(&self, id: JobId, vols: &[OsString], ctl: &Control) -> bool {
        self.acquire_notify(id, vols, ctl, &mut || {})
    }

    /// Como `acquire`; `on_wait` se llama una vez si tiene que esperar (para mandar «En cola»).
    pub fn acquire_notify(&self, id: JobId, vols: &[OsString], ctl: &Control, on_wait: &mut dyn FnMut()) -> bool {
        let mut entries = self.lock();
        entries.push(Entry { id, vols: vols.to_vec(), running: false });
        let mut told = false;
        loop {
            if ctl.is_cancelled() {
                entries.retain(|e| e.id != id);
                self.changed.notify_all();
                return false;
            }
            let blocked = entries
                .iter()
                .any(|e| e.id != id && (e.running || e.id < id) && e.vols.iter().any(|v| vols.contains(v)));
            if !blocked {
                if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                    e.running = true;
                }
                return true;
            }
            if !told {
                told = true;
                on_wait();
            }
            entries = self.changed.wait_timeout(entries, POLL).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Terminó (o se fue): los que esperaban pueden seguir.
    pub fn release(&self, id: JobId) {
        self.lock().retain(|e| e.id != id);
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::mpsc::channel;

    fn v(names: &[&str]) -> Vec<OsString> {
        names.iter().map(OsString::from).collect()
    }

    /// Lanza `acquire` en un hilo; avisa por el canal cuando le toca.
    fn spawn(s: &Arc<Scheduler>, id: JobId, vols: Vec<OsString>, ctl: Arc<Control>, tx: std::sync::mpsc::Sender<(JobId, bool)>) {
        let s = s.clone();
        std::thread::spawn(move || {
            let ok = s.acquire(id, &vols, &ctl);
            let _ = tx.send((id, ok));
        });
    }

    #[test]
    fn scheduler_parallel_on_distinct_volumes() {
        let s = Arc::new(Scheduler::new());
        assert!(s.acquire(1, &v(&["C"]), &Control::new()));
        let (tx, rx) = channel();
        spawn(&s, 2, v(&["D", "E"]), Control::new(), tx);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((2, true)));
    }

    #[test]
    fn scheduler_fifo_on_shared_volume() {
        let s = Arc::new(Scheduler::new());
        // A (1) usa C y D; B (2) usa D; C (3) usa E y D; el 4 usa solo E.
        assert!(s.acquire(1, &v(&["C", "D"]), &Control::new()));
        let (tx, rx) = channel();
        spawn(&s, 2, v(&["D"]), Control::new(), tx.clone());
        std::thread::sleep(Duration::from_millis(30));
        spawn(&s, 3, v(&["E", "D"]), Control::new(), tx.clone());
        // El 4 solo comparte con C, que espera: también espera (no se adelanta).
        std::thread::sleep(Duration::from_millis(30));
        spawn(&s, 4, v(&["E"]), Control::new(), tx);
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err(), "nadie sigue mientras corre A");
        s.release(1);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((2, true)));
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err(), "C no se adelanta a B");
        s.release(2);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((3, true)));
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
        s.release(3);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((4, true)));
    }

    #[test]
    fn scheduler_cancel_while_waiting() {
        let s = Arc::new(Scheduler::new());
        assert!(s.acquire(1, &v(&["C"]), &Control::new()));
        let (tx, rx) = channel();
        let ctl = Control::new();
        spawn(&s, 2, v(&["C"]), ctl.clone(), tx.clone());
        spawn(&s, 3, v(&["C"]), Control::new(), tx);
        std::thread::sleep(Duration::from_millis(30));
        ctl.cancel();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((2, false)));
        // El cancelado ya no bloquea al que venía detrás.
        s.release(1);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((3, true)));
    }

    #[test]
    fn scheduler_tells_when_it_waits() {
        let s = Scheduler::new();
        let mut waited = 0;
        assert!(s.acquire_notify(1, &v(&["C"]), &Control::new(), &mut || waited += 1));
        assert_eq!(waited, 0);
        let ctl = Control::new();
        ctl.cancel();
        assert!(!s.acquire_notify(2, &v(&["C"]), &ctl, &mut || waited += 1));
    }
}

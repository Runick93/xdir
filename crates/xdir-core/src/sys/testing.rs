//! Implementaciones falsas de los servicios del sistema, para los tests (de este crate y de
//! `xdir`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{FileOps, Handle, Services, SystemEnv, Waker};

/// Instala los servicios falsos. Se puede llamar desde todos los tests: solo la primera vez
/// instala, y si ya había servicios instalados no hace nada.
pub fn install_fakes() {
    super::install_once(|| Services {
        waker: Box::new(FakeWaker),
        env: Box::new(FakeEnv),
        file_ops: Box::new(FakeFileOps),
    });
}

/// Mensajes recibidos por el `FakeWaker`, por número de mensaje (de todo el proceso).
static POSTED: Mutex<BTreeMap<u32, usize>> = Mutex::new(BTreeMap::new());

/// `Waker` falso: no despierta a nadie, solo cuenta cuántos mensajes de cada número le llegaron.
pub struct FakeWaker;

impl FakeWaker {
    /// Cuántos mensajes `msg` se mandaron hasta ahora (en todo el proceso de tests: conviene
    /// usar un número de mensaje propio de cada test, o comparar contra el valor de antes).
    pub fn count(msg: u32) -> usize {
        POSTED.lock().unwrap_or_else(|e| e.into_inner()).get(&msg).copied().unwrap_or(0)
    }
}

impl Waker for FakeWaker {
    fn post(&self, _window: Handle, msg: u32, _wparam: usize, _lparam: isize) {
        *POSTED.lock().unwrap_or_else(|e| e.into_inner()).entry(msg).or_insert(0) += 1;
    }
}

pub struct FakeEnv;
impl SystemEnv for FakeEnv {}

pub struct FakeFileOps;
impl FileOps for FakeFileOps {}

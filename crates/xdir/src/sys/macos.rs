//! Servicios del sistema en macOS.

use std::sync::OnceLock;

use xdir_core::sys::{FileOps, Handle, Services, SystemEnv, Waker};

/// Quién atiende los mensajes de la aplicación en el hilo principal (lo fija la ventana al
/// arrancar).
static HANDLER: OnceLock<fn(u32, usize, isize)> = OnceLock::new();

/// Registra la función que recibe en el hilo principal los mensajes de `Handle::post`.
pub fn set_message_handler(f: fn(u32, usize, isize)) {
    let _ = HANDLER.set(f);
}

/// Despierta al hilo principal encolando el mensaje en la cola principal de GCD. Hay una sola
/// ventana, así que el `Handle` no se usa.
struct MacWaker;

impl Waker for MacWaker {
    fn post(&self, _window: Handle, msg: u32, wparam: usize, lparam: isize) {
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(f) = HANDLER.get() {
                f(msg, wparam, lparam);
            }
        });
    }
}

struct MacEnv;
impl SystemEnv for MacEnv {}

struct MacFileOps;
impl FileOps for MacFileOps {}

/// Instala los servicios de macOS en el registro del núcleo.
pub fn install() {
    xdir_core::sys::install(Services { waker: Box::new(MacWaker), env: Box::new(MacEnv), file_ops: Box::new(MacFileOps) });
}

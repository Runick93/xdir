//! Registro de servicios del sistema: lo que el núcleo necesita del sistema operativo, detrás de
//! traits. Cada plataforma instala sus implementaciones una sola vez al arrancar (`install`), antes
//! de crear la `App`, y el resto del código las pide con `waker()`, `env()` y `file_ops()`. Los
//! tests instalan implementaciones falsas con `testing::install_fakes()`.

use std::sync::OnceLock;

pub mod testing;

#[cfg(test)]
mod tests;

/// La ventana principal, vista desde cualquier hilo: sirve para mandarle mensajes de la
/// aplicación (`WM_APP + n`) que se procesan en el hilo de interfaz.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Handle(pub isize);

impl Handle {
    /// Encola un mensaje para el hilo de interfaz y vuelve enseguida.
    pub fn post(self, msg: u32, wparam: usize, lparam: isize) {
        waker().post(self, msg, wparam, lparam);
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Despierta al hilo de interfaz: encola un mensaje para la ventana y vuelve enseguida (no
/// bloquea). Se llama desde cualquier hilo.
pub trait Waker: Send + Sync {
    fn post(&self, window: Handle, msg: u32, wparam: usize, lparam: isize);
}

/// Datos del entorno del sistema (se completa en una tarea siguiente).
pub trait SystemEnv: Send + Sync {}

/// Operaciones de archivos del sistema (se completa en una tarea siguiente).
pub trait FileOps: Send + Sync {}

/// Las implementaciones de una plataforma, instaladas juntas con `install`.
pub struct Services {
    pub waker: Box<dyn Waker>,
    pub env: Box<dyn SystemEnv>,
    pub file_ops: Box<dyn FileOps>,
}

static SERVICES: OnceLock<Services> = OnceLock::new();

/// Instala los servicios del proceso. Va una sola vez, al arrancar; una segunda llamada se
/// ignora (y en debug avisa con `debug_assert!`).
pub fn install(s: Services) {
    let fresh = SERVICES.set(s).is_ok();
    debug_assert!(fresh, "servicios del sistema instalados dos veces");
}

/// Como `install`, pero sin quejarse si ya había servicios (para los falsos de los tests).
fn install_once(make: impl FnOnce() -> Services) {
    SERVICES.get_or_init(make);
}

fn get_or_panic(cell: &OnceLock<Services>) -> &Services {
    cell.get().unwrap_or_else(|| panic!("servicios del sistema no instalados"))
}

fn services() -> &'static Services {
    get_or_panic(&SERVICES)
}

pub fn waker() -> &'static dyn Waker {
    &*services().waker
}

pub fn env() -> &'static dyn SystemEnv {
    &*services().env
}

pub fn file_ops() -> &'static dyn FileOps {
    &*services().file_ops
}

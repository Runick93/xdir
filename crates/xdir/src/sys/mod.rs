//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para Windows. `run`
//! (en `app/win.rs`) llama `install` al arrancar, antes de crear la `App`.

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::install;
#[cfg(all(windows, test))]
use self::win32::{platform_env, platform_file_system};

#[cfg(test)]
mod tests;

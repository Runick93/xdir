//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para cada sistema. `run`
//! (en `app/win.rs` y `app/mac.rs`) llama `install` al arrancar, antes de crear la `App`.

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::install;
#[cfg(all(windows, test))]
use self::win32::platform_env;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::{install, set_message_handler};
#[cfg(all(target_os = "macos", test))]
use self::macos::platform_env;

#[cfg(test)]
mod tests;

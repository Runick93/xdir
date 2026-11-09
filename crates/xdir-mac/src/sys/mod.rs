//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para macOS. `run` (en
//! `app/mac.rs`) llama `install` al arrancar, antes de crear la `App`.

mod macos;
pub use self::macos::{install, set_message_handler};
#[cfg(test)]
use self::macos::{platform_env, platform_file_system};

#[cfg(test)]
mod tests;

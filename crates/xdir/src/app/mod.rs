//! Lo propio de cada sistema: arranque, ventana, controles nativos y menús (en el host). El estado
//! de la aplicación (`App`) vive en el núcleo (`xdir_core::app`); estos módulos solo usan su API
//! pública (y `Init`, `startup_args`, `ms_since`, `INITIAL_SIZE`).

#[cfg(windows)]
mod host_win;
#[cfg(windows)]
mod messages;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::run;
#[cfg(target_os = "macos")]
mod host_mac;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::run;

#[cfg(any(windows, target_os = "macos"))]
use xdir_core::app::{App, CursorKind, INITIAL_SIZE, Init, ms_since, startup_args};

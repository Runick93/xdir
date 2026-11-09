//! Lo propio de macOS: arranque, ventana, controles nativos y menús (en el host). El estado de la
//! aplicación (`App`) vive en el núcleo (`xdir_core::app`); estos módulos solo usan su API pública
//! (y `Init`, `startup_args`, `ms_since`, `INITIAL_SIZE`).

mod host_mac;
mod mac;
pub use mac::run;

use xdir_core::app::{App, CursorKind, INITIAL_SIZE, Init, ms_since, startup_args};

//! Lo propio de Windows: arranque, ventana, controles nativos y menús (en el host). El estado
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

use xdir_core::app::{App, CursorKind, INITIAL_SIZE, Init, ms_since, startup_args};

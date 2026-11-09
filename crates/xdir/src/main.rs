// En release no se abre consola; en debug sí, para ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drop;
mod fs;
mod icons;
mod ops;
mod platform;
mod render;
mod shell;
mod sys;
mod thumbs;
mod app;
pub(crate) use xdir_core::{config, theme, xtree};

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    app::run()
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    app::run()
}

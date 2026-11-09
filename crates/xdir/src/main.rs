// En release no se abre consola; en debug sí, para ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use xdir_core::commands;

mod console;
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
pub(crate) use xdir_core::{about, anim, config, favorites, groups, jump, list, palette, pane, session, tab, theme, typed, ui, xtree};
#[cfg(feature = "arbol")]
pub(crate) use xdir_core::tree;

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    app::run()
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    app::run()
}

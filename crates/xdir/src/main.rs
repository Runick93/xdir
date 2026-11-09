// En release no se abre consola; en debug sí, para ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod anim;
mod commands;
mod config;
mod console;
mod drop;
mod favorites;
mod format;
mod fs;
mod groups;
mod icons;
mod jump;
mod list;
mod ops;
mod pane;
mod palette;
mod platform;
mod render;
mod session;
mod shell;
mod sys;
mod tab;
mod theme;
mod thumbs;
#[cfg(feature = "arbol")]
mod tree;
mod typed;
mod ui;
mod xtree;
mod app;
pub(crate) use xdir_core::toml;

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    app::run()
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    app::run()
}

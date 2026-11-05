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
mod tab;
mod theme;
mod thumbs;
mod toml;
#[cfg(feature = "arbol")]
mod tree;
mod typed;
mod ui;
mod app;

fn main() -> windows::core::Result<()> {
    app::run()
}

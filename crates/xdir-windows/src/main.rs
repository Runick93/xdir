// En release no se abre consola; en debug sí, para ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! xdir para Windows: arranque, ventana y servicios del sistema. El estado de la aplicación y todo
//! lo neutral viven en `xdir-core`. En otros sistemas la crate compila vacía (un `main` que avisa
//! y sale), así `cargo build` y `cargo test` en la raíz del workspace andan en todos.

#[cfg(windows)]
mod drop;
#[cfg(windows)]
mod fs;
#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod ops;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod sys;
#[cfg(windows)]
mod thumbs;
#[cfg(windows)]
mod app;
#[cfg(windows)]
pub(crate) use xdir_core::{config, theme, xtree};

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("xdir-windows solo funciona en Windows");
}

// Fuera de Windows igual corren las pruebas que no tocan el sistema: el nombre del temporal del
// ayudante de xtree y la tabla de comandos de Windows contra el entorno falso.
#[cfg(all(test, not(windows)))]
#[path = "fs/xtree_temp.rs"]
mod xtree_temp;
#[cfg(all(test, not(windows)))]
#[path = "sys/tests.rs"]
mod sys_tests;

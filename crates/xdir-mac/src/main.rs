//! xdir para macOS: arranque, ventana y servicios del sistema. El estado de la aplicación y todo
//! lo neutral viven en `xdir-core`. En otros sistemas la crate compila vacía (un `main` que avisa
//! y sale), así `cargo build` y `cargo test` en la raíz del workspace andan en todos.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod drop;
#[cfg(target_os = "macos")]
mod fs;
#[cfg(target_os = "macos")]
mod icons;
#[cfg(target_os = "macos")]
mod metadata;
#[cfg(target_os = "macos")]
mod ops;
#[cfg(target_os = "macos")]
mod platform;
#[cfg(target_os = "macos")]
mod quicklook;
#[cfg(target_os = "macos")]
mod render;
#[cfg(target_os = "macos")]
mod shell;
#[cfg(target_os = "macos")]
mod sys;
#[cfg(target_os = "macos")]
mod thumbs;
#[cfg(target_os = "macos")]
mod trash;
#[cfg(target_os = "macos")]
pub(crate) use xdir_core::{config, theme, xtree};

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    app::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("xdir-mac solo funciona en macOS");
}

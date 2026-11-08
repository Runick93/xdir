//! xtree: mapa de uso de disco (treemap). Programa aparte que abre xdir desde su barra de estado.
//! Por ahora solo existe para Windows (escáner con MFT, Direct2D); en los demás sistemas compila
//! a un aviso para que `cargo build` funcione igual en todos.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    win::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("xtree todavía no está disponible en este sistema (solo Windows).");
    std::process::exit(1);
}

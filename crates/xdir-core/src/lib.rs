//! Núcleo neutro de xdir: todo lo que no depende del sistema operativo (geometría, teclas,
//! configuración TOML, tema, dibujo abstracto, formato de textos y datos de xtree). Sin
//! dependencias externas.

pub mod anim;
pub mod commands;
pub mod config;
pub mod console;
pub mod favorites;
pub mod format;
pub mod fs;
pub mod geom;
pub mod groups;
pub mod jump;
pub mod keys;
pub mod ops;
pub mod paint;
pub mod session;
pub mod shell;
pub mod sys;
pub mod theme;
pub mod thumbs;
pub mod toml;
pub mod typed;
pub mod xtree;

/// Primer número de mensaje libre para la aplicación (igual que `WM_APP` de Windows).
pub const WM_APP: u32 = 0x8000;

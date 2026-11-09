//! Núcleo neutro de xdir: todo lo que no depende del sistema operativo (geometría, teclas,
//! configuración TOML, tema, dibujo abstracto, lista de archivos, pestañas, paneles, paleta,
//! árbol lateral, sesión, barra de estado y diálogos, formato de textos, xtree con su vista y el
//! árbol de accesibilidad) y la ventana vista como trait (`host::WindowHost`). Sin dependencias
//! externas.

pub mod a11y;
pub mod about;
pub mod anim;
pub mod commands;
pub mod config;
pub mod console;
pub mod favorites;
pub mod format;
pub mod fs;
pub mod geom;
pub mod groups;
pub mod host;
pub mod jump;
pub mod keys;
pub mod list;
pub mod ops;
pub mod paint;
pub mod palette;
pub mod pane;
pub mod session;
pub mod shell;
pub mod sys;
pub mod tab;
pub mod theme;
pub mod thumbs;
pub mod toml;
/// Árbol de carpetas lateral (solo con la feature `arbol`).
#[cfg(feature = "arbol")]
pub mod tree;
pub mod typed;
/// `Suggest::draw`: el desplegable de rutas.
mod typed_draw;
pub mod ui;
pub mod xtree;

/// Primer número de mensaje libre para la aplicación (igual que `WM_APP` de Windows).
pub const WM_APP: u32 = 0x8000;

//! Núcleo neutro de xdir: todo lo que no depende del sistema operativo (geometría, teclas,
//! configuración TOML, tema, dibujo abstracto, lista de archivos, pestañas, paneles, paleta,
//! árbol lateral, sesión, barra de estado y diálogos, formato de textos, xtree con su vista y el
//! árbol de accesibilidad) y la ventana vista como trait (`host::WindowHost`). Sin dependencias
//! externas.

pub mod a11y;
pub mod app;
pub mod about;
pub mod anim;
pub mod archive;
pub mod archive_nav;
pub mod changelog;
pub mod changelog_draw;
pub mod commands;
pub mod config;
pub mod console;
pub mod drop;
pub mod favorites;
pub mod folder_views;
pub mod format;
pub mod fs;
pub mod geom;
pub mod groups;
pub mod host;
pub mod jump;
pub mod keys;
pub mod list;
pub mod names;
pub mod net;
pub mod ops;
pub mod transfer;
pub mod paint;
pub mod palette;
pub mod pane;
pub mod info;
pub mod preview;
pub mod quicklook;
/// Renombrar varios: nombres nuevos, choques y renombrado seguro.
pub mod rename_many;
/// El panel de Renombrar varios: dibujo y hit-test.
pub mod rename_many_draw;
pub mod session;
/// La pestaña Configuración: secciones, controles y dibujo.
pub mod settings;
pub mod shell;
pub mod sys;
pub mod tab;
pub mod theme;
pub mod thumbs;
pub mod toml;
/// Cambiar `config.toml` por líneas sin perder lo escrito a mano.
pub mod toml_edit;
/// Árbol de carpetas lateral (solo con la feature `arbol`).
#[cfg(feature = "arbol")]
pub mod tree;
pub mod typeahead;
pub mod typed;
/// `Suggest::draw`: el desplegable de rutas.
mod typed_draw;
pub mod ui;
pub mod xtree;

/// Primer número de mensaje libre para la aplicación (igual que `WM_APP` de Windows).
pub const WM_APP: u32 = 0x8000;

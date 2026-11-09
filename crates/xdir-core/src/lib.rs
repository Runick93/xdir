//! Núcleo neutro de xdir: todo lo que no depende del sistema operativo (geometría, teclas,
//! configuración TOML, formato de textos y datos de xtree). Sin dependencias externas.

pub mod format;
pub mod geom;
pub mod keys;
pub mod toml;
pub mod xtree;

/// Primer número de mensaje libre para la aplicación (igual que `WM_APP` de Windows).
pub const WM_APP: u32 = 0x8000;

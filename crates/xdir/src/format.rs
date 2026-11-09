//! Formato de fechas y tamaños según la configuración regional, y orden natural de nombres. Vive
//! en el núcleo (`xdir_core::format`); las fechas y tamaños los da la plataforma (`sys::env()`).

pub use xdir_core::format::{size_short, thousands};

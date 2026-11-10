//! Miniaturas e íconos grandes para las vistas de cuadrícula, en un hilo propio. La interfaz pide,
//! dibuja sin la imagen y el hilo avisa con `WM_APP_ICONS` cuando está lista.

mod win32;
pub use self::win32::*;

pub use xdir_core::thumbs::{BigCache, BigFailures, Key, Source, big_grow};

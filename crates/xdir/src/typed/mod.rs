//! Historial de rutas escritas y su desplegable: la lógica vive en el núcleo
//! (`xdir_core::typed`); acá queda el dibujo, que usa `Painter` (hasta la fase 3).

mod draw;

pub use self::draw::SuggestDraw;
pub use xdir_core::typed::{Suggest, push};

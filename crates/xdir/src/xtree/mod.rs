//! Núcleo neutro de xtree (uso de disco en un treemap): árbol de nodos, diseño del mapa y colores.
//! No depende del sistema operativo; el escáner y la vista se apoyan en estos módulos.

pub use xdir_core::xtree::{colors, tree, treemap};
pub mod scan;
pub mod view;

//! Claves de las miniaturas: qué elemento y de qué tamaño. La carga vive en cada plataforma
//! (`xdir::thumbs`).

use std::path::PathBuf;
use std::sync::Arc;

/// De qué elemento: una ruta de disco o un PIDL absoluto (elementos virtuales del Shell).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Path(PathBuf),
    Pidl(Arc<[u8]>),
}

/// (elemento, lado en píxeles).
pub type Key = (Source, u32);

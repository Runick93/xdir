//! Cuando algo falla porque otro programa tiene abierto el archivo (`ErrKind::InUse`): qué se
//! intentaba hacer y quién tiene cada archivo, para mostrarlo en la ventana de «archivo en uso».

use std::path::PathBuf;

use crate::ops::Op;
use crate::sys::Holder;

/// Un archivo que no se pudo tocar y los programas que lo tienen.
#[derive(Clone, Debug, PartialEq)]
pub struct InUseItem {
    pub path: PathBuf,
    pub holders: Vec<Holder>,
}

/// Una operación suelta que falló por archivos en uso.
#[derive(Clone, Debug, PartialEq)]
pub struct InUseFailure {
    /// La operación que falló (para reintentarla).
    pub op: Op,
    pub items: Vec<InUseItem>,
}

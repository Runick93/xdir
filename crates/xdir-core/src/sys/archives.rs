//! Comprimir y extraer (`Archives`): zip en los tres sistemas y tar.gz donde se puede. Lo
//! implementa la crate `xdir-archive` (las plataformas la instalan); el motor de copia
//! (`transfer`) decide nombres, conflictos, espacio y deshacer.

use std::path::{Path, PathBuf};

use super::{CopyError, ErrKind, Flow};
use crate::archive::{ArchiveEntry, ArchiveFormat};

/// Texto del error de un zip con contraseña (la App ofrece abrirlo con el sistema).
pub const PASSWORD: &str = "Este zip tiene contraseña";
/// Texto del error de un formato o método que no se sabe leer.
pub const UNSUPPORTED: &str = "Formato no soportado";
/// Texto del error de una entrada que saldría de la carpeta (o un enlace que apunta afuera).
pub const NOT_ALLOWED: &str = "Ruta no permitida dentro del archivo";

pub trait Archives: Send + Sync {
    /// Los formatos que este sistema sabe extraer (Windows: solo zip).
    fn formats(&self) -> &'static [ArchiveFormat];
    /// Las entradas del archivo; `cancel` se consulta mientras se lee (un tar.gz se descomprime
    /// entero para listarlo).
    fn list(&self, archive: &Path, cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, CopyError>;
    /// Extrae todo bajo `root` (una carpeta recién creada y vacía): nunca escribe fuera de ella
    /// ni sigue enlaces, crea los enlaces simbólicos al final y solo si apuntan adentro (y fuera de
    /// Windows). `progress` recibe los bytes escritos en total. Una entrada que no se puede
    /// extraer (ruta no permitida, enlace que sale, error de escritura) va a `failed(nombre,
    /// error)` y se sigue con las demás. Corta con `Err` si se cancela, si el disco se llena, si
    /// el zip tiene contraseña o si no se puede leer el archivo; lo ya escrito queda (lo borra
    /// quien llama). Devuelve cuántos elementos se omitieron porque no se pueden crear (enlaces
    /// en Windows, enlaces duros y dispositivos de un tar).
    fn extract_all(
        &self,
        archive: &Path,
        root: &Path,
        progress: &mut dyn FnMut(u64) -> Flow,
        failed: &mut dyn FnMut(&str, CopyError),
    ) -> Result<usize, CopyError>;
    /// Crea un zip en `out` (no debe existir; el nombre puede ser un parcial). `items` = (ruta en
    /// el disco, nombre adentro con `/`; las carpetas terminan en `/`). `progress` recibe los
    /// bytes leídos en total; `Stop` corta y borra `out`. Si falla, también lo borra.
    fn create(&self, out: &Path, items: &[(PathBuf, String)], progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError>;
}

/// Sin comprimir ni extraer (un sistema que todavía no lo tiene).
pub struct NoArchives;

impl Archives for NoArchives {
    fn formats(&self) -> &'static [ArchiveFormat] {
        &[]
    }
    fn list(&self, archive: &Path, _cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, CopyError> {
        Err(CopyError::new(ErrKind::Other(UNSUPPORTED.into()), archive))
    }
    fn extract_all(
        &self,
        archive: &Path,
        _root: &Path,
        _progress: &mut dyn FnMut(u64) -> Flow,
        _failed: &mut dyn FnMut(&str, CopyError),
    ) -> Result<usize, CopyError> {
        Err(CopyError::new(ErrKind::Other(UNSUPPORTED.into()), archive))
    }
    fn create(&self, out: &Path, _items: &[(PathBuf, String)], _progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        Err(CopyError::new(ErrKind::Other(UNSUPPORTED.into()), out))
    }
}

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
/// Texto del error de un zip que no se puede leer.
pub const DAMAGED_ZIP: &str = "El zip está dañado";
/// Texto del error de un archivo (tar.gz) que no se puede leer, o que escribe más de lo que declara.
pub const DAMAGED: &str = "El archivo está dañado";
/// Texto del error de una entrada que saldría de la carpeta (o un enlace que apunta afuera).
pub const NOT_ALLOWED: &str = "Ruta no permitida dentro del archivo";

/// Qué hacer con una entrada en `Archives::extract_some`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    Skip,
    Take,
    /// Extraerla y no seguir leyendo (un tar.gz no se descomprime hasta el final por un solo
    /// elemento).
    Last,
}

pub trait Archives: Send + Sync {
    /// Los formatos que este sistema sabe extraer (Windows: solo zip).
    fn formats(&self) -> &'static [ArchiveFormat];
    /// Las entradas del archivo, como mucho `max + 1` (con una de más se sabe que hay más que
    /// `max` sin leer el resto: un archivo con millones de entradas no infla la memoria). `cancel`
    /// se consulta mientras se lee (entre entradas, y en un tar.gz también mientras se
    /// descomprime). No lee el destino de los enlaces de un zip (`link` va vacío).
    fn list(&self, archive: &Path, max: usize, cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, CopyError>;
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
    /// De los enlaces simbólicos `links` (ya creados), los que resuelven fuera de `root` siguiendo
    /// lo que hay de verdad en el disco (también enlaces que ya estaban). Lo extraído se valida al
    /// escribirlo, pero al llevarlo a su lugar puede quedar al lado de un enlace del usuario que
    /// apunta afuera: el motor lo vuelve a mirar ahí y borra los que salen.
    fn links_outside(&self, root: &Path, links: &[PathBuf]) -> Vec<PathBuf>;
    /// Como `extract_all`, pero solo las entradas que elige `want` (recibe la posición de la
    /// entrada, la de `list`, y su nombre guardado sin sanear); las carpetas de arriba se crean al
    /// bajar. Con `Pick::Last` deja de leer después de esa. Para abrir o copiar algo de adentro sin
    /// extraer todo. Por defecto, sin soporte.
    fn extract_some(
        &self,
        archive: &Path,
        _root: &Path,
        _want: &dyn Fn(usize, &str) -> Pick,
        _progress: &mut dyn FnMut(u64) -> Flow,
        _failed: &mut dyn FnMut(&str, CopyError),
    ) -> Result<usize, CopyError> {
        Err(CopyError::new(ErrKind::Other(UNSUPPORTED.into()), archive))
    }
    /// Lo extraído de `archive` (`written`: archivos y carpetas recién escritos) hereda su origen:
    /// la marca de la web (`Zone.Identifier`) en Windows, la cuarentena en macOS. Así lo que vino
    /// de internet dentro de un zip sigue avisando al abrirlo. Si el archivo no tiene marca, nada.
    /// Por defecto, nada.
    fn propagate_origin(&self, _archive: &Path, _written: &[PathBuf]) {}
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
    fn list(&self, archive: &Path, _max: usize, _cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, CopyError> {
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
    fn links_outside(&self, _root: &Path, _links: &[PathBuf]) -> Vec<PathBuf> {
        Vec::new()
    }
    fn create(&self, out: &Path, _items: &[(PathBuf, String)], _progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        Err(CopyError::new(ErrKind::Other(UNSUPPORTED.into()), out))
    }
}

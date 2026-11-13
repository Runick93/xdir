//! Sumas de verificación (`Hashes`): leer un archivo por bloques y calcular su SHA-256 o MD5. Lo
//! implementa la crate `xdir-hash` (las plataformas la instalan); el núcleo no tiene dependencias.
//! Leer es bloqueante: solo en hilos de trabajo (la comparación y la suma corren en los suyos,
//! vigilados por falta de avance en la red: `compare::run_watched`).

use std::io::{self, Read};
use std::path::Path;

use super::{CopyError, ErrKind, Flow};

/// Los algoritmos que se ofrecen, en el orden del selector.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum HashAlgo {
    Sha256,
    Md5,
}

impl HashAlgo {
    pub const ALL: [HashAlgo; 2] = [HashAlgo::Sha256, HashAlgo::Md5];

    pub fn label(self) -> &'static str {
        match self {
            HashAlgo::Sha256 => "SHA-256",
            HashAlgo::Md5 => "MD5",
        }
    }

    /// Largo de la suma en hexadecimal.
    pub fn hex_len(self) -> usize {
        match self {
            HashAlgo::Sha256 => 64,
            HashAlgo::Md5 => 32,
        }
    }

    /// El algoritmo de una suma por su largo en hexadecimal (64 = SHA-256, 32 = MD5).
    pub fn from_hex_len(n: usize) -> Option<HashAlgo> {
        HashAlgo::ALL.into_iter().find(|a| a.hex_len() == n)
    }
}

/// Un lector de un archivo abierto con `Hashes::open`.
pub type Reader = Box<dyn Read + Send>;

pub trait Hashes: Send + Sync {
    /// Abre `path` para leerlo por bloques (lo sigue si es un enlace). Bloqueante: solo en hilos de
    /// trabajo.
    fn open(&self, path: &Path) -> Result<Reader, CopyError>;
    /// La suma de todo lo que da `r`, en hexadecimal en minúscula. `progress` recibe lo leído en
    /// total después de cada bloque; `Flow::Stop` corta y devuelve `Ok(None)`.
    fn hash(&self, r: &mut dyn Read, algo: HashAlgo, progress: &mut dyn FnMut(u64) -> Flow) -> io::Result<Option<String>>;
}

/// El error de abrir o leer `path`, como los de las primitivas de copia (sin lo propio de cada
/// sistema: eso lo agrega la plataforma).
pub fn io_error(e: &io::Error, path: &Path) -> CopyError {
    let kind = match e.kind() {
        io::ErrorKind::PermissionDenied => ErrKind::Denied,
        io::ErrorKind::NotFound => ErrKind::NotFound,
        io::ErrorKind::ResourceBusy => ErrKind::InUse,
        _ => ErrKind::Other(e.to_string()),
    };
    CopyError::new(kind, path)
}

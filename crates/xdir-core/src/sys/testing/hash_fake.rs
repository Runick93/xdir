//! `FakeHashes`: lee del disco falso de la copia (lo que extrae `archive_nav::temps` en las
//! pruebas) o, si no está ahí, del real; una ruta «colgada» (`FakeNetwork::hang`) duerme en cada
//! lectura; abrir una ruta remota en el hilo de interfaz se anota (`FakeNetwork::note_io`). La
//! suma es determinística (FNV-1a de 64 bits, distinta por algoritmo) con el largo de cada uno.

use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::{FakeCopyPrims, FakeNetwork};
use crate::sys::{CopyError, Flow, HashAlgo, Hashes, Opened, io_error};

pub struct FakeHashes;

/// Rutas lentas pero vivas: (carpeta, bytes por lectura, pausa antes de cada una).
static THROTTLED: Mutex<Vec<(PathBuf, usize, Duration)>> = Mutex::new(Vec::new());
/// Archivos abiertos ahora (uno por lector vivo).
static OPEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

impl FakeHashes {
    /// Lo que está bajo `prefix` se lee de a `chunk` bytes, con `delay` antes de cada lectura
    /// (una red lenta que sigue respondiendo). En todos los hilos; cada prueba usa rutas propias.
    pub fn throttle(prefix: &Path, chunk: usize, delay: Duration) {
        THROTTLED.lock().unwrap_or_else(|e| e.into_inner()).push((prefix.to_path_buf(), chunk, delay));
    }

    /// Cuántos lectores siguen abiertos bajo `prefix` (expulsar tiene que esperar a que se cierren).
    pub fn open_readers(prefix: &Path) -> usize {
        OPEN.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|p| p.starts_with(prefix)).count()
    }

    fn throttle_for(path: &Path) -> Option<(usize, Duration)> {
        THROTTLED.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(p, ..)| path.starts_with(p)).map(|&(_, c, d)| (c, d))
    }
}

/// Un lector que duerme `hang` antes de cada lectura y, si es lento, entrega de a `chunk`.
struct Slow<R> {
    inner: R,
    hang: Option<Duration>,
    chunk: Option<usize>,
    path: PathBuf,
}

impl<R> Slow<R> {
    fn new(inner: R, hang: Option<Duration>, chunk: Option<usize>, path: &Path) -> Slow<R> {
        OPEN.lock().unwrap_or_else(|e| e.into_inner()).push(path.to_path_buf());
        Slow { inner, hang, chunk, path: path.to_path_buf() }
    }
}

impl<R> Drop for Slow<R> {
    fn drop(&mut self) {
        let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = open.iter().position(|p| *p == self.path) {
            open.remove(i);
        }
    }
}

impl<R: Read> Read for Slow<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(d) = self.hang {
            std::thread::sleep(d);
        }
        let n = self.chunk.map_or(buf.len(), |c| c.min(buf.len()));
        self.inner.read(&mut buf[..n])
    }
}

impl Hashes for FakeHashes {
    fn open(&self, path: &Path) -> Result<Opened, CopyError> {
        FakeNetwork::note_io("open", path);
        let (chunk, slow) = Self::throttle_for(path).unzip();
        let hang = FakeNetwork::hang_for(&path.to_string_lossy()).or(slow);
        let disk = FakeCopyPrims::shared();
        if disk.exists(path) {
            let data = disk.read_content(path)?;
            let size = data.len() as u64;
            return Ok(Opened { reader: Box::new(Slow::new(Cursor::new(data), hang, chunk, path)), size });
        }
        let f = std::fs::File::open(path).map_err(|e| io_error(&e, path))?;
        let size = f.metadata().map_err(|e| io_error(&e, path))?.len();
        Ok(Opened { reader: Box::new(Slow::new(f, hang, chunk, path)), size })
    }

    fn hash(&self, r: &mut dyn Read, algo: HashAlgo, progress: &mut dyn FnMut(u64) -> Flow) -> io::Result<Option<String>> {
        let mut h: u64 = match algo {
            HashAlgo::Sha256 => 0xcbf2_9ce4_8422_2325,
            HashAlgo::Md5 => 0x8422_2325_cbf2_9ce4,
        };
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            for &b in &buf[..n] {
                h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
            }
            total += n as u64;
            if progress(total) == Flow::Stop {
                return Ok(None);
            }
        }
        let one = format!("{h:016x}");
        Ok(Some(one.repeat(algo.hex_len() / 16)))
    }
}

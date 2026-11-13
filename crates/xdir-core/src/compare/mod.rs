//! Comparar el contenido de dos archivos y sumas de verificación (issue #59).
//!
//! La comparación es byte a byte (no calcula sumas): lee los dos a la vez de a bloques de 1 MB y
//! corta en el primer byte distinto. Un archivo de adentro de un comprimido se extrae antes a la
//! temporal de la app (`archive_nav::temps`, con su límite y su origen) y se lee desde ahí. Todo es
//! bloqueante: corre en hilos de trabajo, vigilados por `run_watched` (en la red, sin avance en el
//! límite se abandona con «"srv" no responde», como la suma de carpetas de la D+). Lee con
//! `sys::hashes()` (`open`), que también calcula las sumas.
//!
//! Neutro: también arma lo que se muestra (la suma partida en líneas, la posición del primer byte
//! distinto) y entiende la suma publicada que se pega en «Comparar con…» (`parse_expected`).

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use crate::archive_nav::temps;
use crate::sys::{self, CopyError, ErrKind, Flow, HashAlgo};

#[cfg(test)]
mod tests;

/// Lo que se lee de cada archivo de una vez (1 MB).
pub const BLOCK: usize = 1 << 20;

/// Ancho de un carácter de la letra de ancho fijo (`FontId::Mono`, 11,5) para partir la suma.
pub const MONO_W: f32 = 7.0;

/// El resultado de comparar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Same,
    /// Las dos rutas son el mismo archivo.
    SameFile,
    /// Tamaños distintos: no se leyó el contenido.
    SizeDiffers { a: u64, b: u64 },
    /// Distintos desde ese byte (desde 0).
    DiffersAt(u64),
}

/// Por qué no hay resultado.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Failure {
    Cancelled,
    /// El texto para el usuario.
    Failed(String),
}

/// De dónde se lee un archivo.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    Disk(PathBuf),
    /// Un elemento (`rel`, con `/`) del comprimido `file` con la cadena de anidados `nested`.
    Archive { file: PathBuf, nested: Vec<String>, rel: String },
}

impl Source {
    /// El nombre del archivo.
    pub fn name(&self) -> String {
        match self {
            Source::Disk(p) => p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()),
            Source::Archive { rel, .. } => rel.rsplit('/').next().unwrap_or(rel).to_string(),
        }
    }

    /// El archivo del disco que hay que leer (el del disco o el comprimido de afuera).
    fn disk_path(&self) -> &Path {
        match self {
            Source::Disk(p) => p,
            Source::Archive { file, .. } => file,
        }
    }

    /// El servidor, si se lee de una carpeta de red (sin E/S: `Network::is_remote`).
    pub fn remote_host(&self) -> Option<String> {
        let p = self.disk_path();
        sys::network().is_remote(p).then(|| crate::tab::remote_host(p))
    }

    /// El archivo del disco para leer: el mismo, o la extracción temporal (hasta
    /// `temps::OPEN_MAX`). Bloqueante; `cancel` corta (y cuenta como avance de la extracción).
    pub fn resolve(&self, cancel: &dyn Fn() -> bool) -> Result<PathBuf, CopyError> {
        match self {
            Source::Disk(p) => Ok(p.clone()),
            Source::Archive { file, nested, rel } => temps::global().ensure(file, nested, rel, &temps::Limits { max: temps::OPEN_MAX, cancel }),
        }
    }
}

/// El avance de un trabajo, compartido con el hilo de interfaz (se lee al dibujar).
#[derive(Default, Debug)]
pub struct Progress {
    /// Bytes ya leídos (de cada archivo, en una comparación) y el total.
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// Extrayendo de un comprimido (todavía no se lee).
    pub extracting: AtomicBool,
    /// Cada cosa que avanza (un bloque, un pedazo extraído): la vigilancia mira si cambia.
    beats: AtomicU64,
}

impl Progress {
    pub fn beat(&self) {
        self.beats.fetch_add(1, Ordering::Relaxed);
    }

    /// La parte hecha (0–1; 0 sin total).
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 { 0.0 } else { (self.done.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0) as f32 }
    }
}

/// Compara lo que dan `a` y `b` de a bloques de `BLOCK` (de los dos a la vez). `progress` recibe lo
/// comparado en total después de cada bloque; `Flow::Stop` corta (`Ok(None)`). El error dice de
/// cuál fue (0 = `a`, 1 = `b`).
pub fn compare_readers(a: &mut dyn Read, b: &mut dyn Read, progress: &mut dyn FnMut(u64) -> Flow) -> Result<Option<Verdict>, (usize, io::Error)> {
    let (mut ba, mut bb) = (vec![0u8; BLOCK], vec![0u8; BLOCK]);
    let mut total = 0u64;
    loop {
        let na = read_block(a, &mut ba).map_err(|e| (0, e))?;
        let nb = read_block(b, &mut bb).map_err(|e| (1, e))?;
        let n = na.min(nb);
        if let Some(i) = ba[..n].iter().zip(&bb[..n]).position(|(x, y)| x != y) {
            return Ok(Some(Verdict::DiffersAt(total + i as u64)));
        }
        if na != nb {
            return Ok(Some(Verdict::DiffersAt(total + n as u64)));
        }
        if n == 0 {
            return Ok(Some(Verdict::Same));
        }
        total += n as u64;
        if progress(total) == Flow::Stop {
            return Ok(None);
        }
        if n < BLOCK {
            return Ok(Some(Verdict::Same));
        }
    }
}

/// Llena `buf` todo lo que se pueda (menos solo al final): una lectura corta de la red no corta el
/// bloque. Devuelve cuánto leyó.
fn read_block(r: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

fn failed(name: &str, e: &CopyError) -> Failure {
    match e.kind {
        ErrKind::Cancelled => Failure::Cancelled,
        _ => Failure::Failed(format!("No se pudo leer «{name}»: {}", e.message())),
    }
}

/// Abre `src` (extrayéndolo si es de un comprimido) sin bajar lo que está solo en la nube.
fn open(src: &Source, stop: &AtomicBool, progress: &Progress) -> Result<sys::Opened, Failure> {
    let name = src.name();
    if matches!(src, Source::Archive { .. }) {
        progress.extracting.store(true, Ordering::Relaxed);
    }
    let path = src
        .resolve(&|| {
            progress.beat();
            stop.load(Ordering::Relaxed)
        })
        .map_err(|e| failed(&name, &e))?;
    progress.extracting.store(false, Ordering::Relaxed);
    if stop.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled);
    }
    if matches!(src, Source::Disk(_)) && sys::file_system().cloud_only(&path) {
        return Err(Failure::Failed(format!("«{name}» está solo en la nube: no se baja para leerlo")));
    }
    progress.beat();
    sys::hashes().open(&path).map_err(|e| failed(&name, &e))
}

/// Compara el contenido de `a` y `b`: el mismo archivo, tamaños distintos (sin leer el contenido)
/// o byte a byte. Bloqueante (en hilos de trabajo); `stop` corta.
pub fn compare_sources(a: &Source, b: &Source, stop: &AtomicBool, progress: &Progress) -> Result<Verdict, Failure> {
    if a == b {
        return Ok(Verdict::SameFile);
    }
    let mut fa = open(a, stop, progress)?;
    let mut fb = open(b, stop, progress)?;
    if fa.size != fb.size {
        return Ok(Verdict::SizeDiffers { a: fa.size, b: fb.size });
    }
    progress.total.store(fa.size, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let res = compare_readers(&mut fa.reader, &mut fb.reader, &mut |n| {
        progress.done.store(n, Ordering::Relaxed);
        progress.beat();
        if stop.load(Ordering::Relaxed) { Flow::Stop } else { Flow::Continue }
    });
    match res {
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(Failure::Cancelled),
        Err((side, e)) => {
            let name = if side == 0 { a.name() } else { b.name() };
            Err(Failure::Failed(format!("No se pudo leer «{name}»: {e}")))
        }
    }
}

/// La suma `algo` de `src` en hexadecimal en minúscula. Bloqueante; `stop` corta.
pub fn hash_source(src: &Source, algo: HashAlgo, stop: &AtomicBool, progress: &Progress) -> Result<String, Failure> {
    let mut f = open(src, stop, progress)?;
    progress.total.store(f.size, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let res = sys::hashes().hash(&mut f.reader, algo, &mut |n| {
        progress.done.store(n, Ordering::Relaxed);
        progress.beat();
        if stop.load(Ordering::Relaxed) { Flow::Stop } else { Flow::Continue }
    });
    match res {
        Ok(Some(h)) => Ok(h),
        Ok(None) => Err(Failure::Cancelled),
        Err(e) => Err(Failure::Failed(format!("No se pudo leer «{}»: {e}", src.name()))),
    }
}

/// Corre `work` en un hilo propio y espera su resultado (bloqueante: se llama desde otro hilo de
/// trabajo, nunca desde el de interfaz). `work` recibe una bandera para cortarse. Si `cancel` se
/// enciende, corta y devuelve `Cancelled` enseguida; si es de la red (`remote`, el servidor) y
/// pasa `limit` sin que `progress` avance, corta con «"srv" no responde». En los dos casos el hilo
/// queda abandonado si no responde (un servidor colgado no traba nada). Mientras espera, llama a
/// `tick` cada `every` (para redibujar el avance).
pub fn run_watched<T: Send + 'static>(
    remote: Option<String>,
    limit: Duration,
    cancel: &AtomicBool,
    progress: &Arc<Progress>,
    every: Duration,
    tick: &dyn Fn(),
    work: impl FnOnce(&AtomicBool) -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let s = stop.clone();
    let spawned = std::thread::Builder::new().name("xdir-comparar".into()).spawn(move || {
        let _ = tx.send(work(&s));
    });
    if spawned.is_err() {
        return Err(Failure::Failed("No se pudo crear un hilo".into()));
    }
    let (mut last, mut since) = (progress.beats.load(Ordering::Relaxed), Instant::now());
    let mut ticked = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(r) => return r,
            Err(RecvTimeoutError::Disconnected) => return Err(Failure::Failed("La lectura falló".into())),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if cancel.load(Ordering::Relaxed) {
            stop.store(true, Ordering::Relaxed);
            return Err(Failure::Cancelled);
        }
        if ticked.elapsed() >= every {
            ticked = Instant::now();
            tick();
        }
        let now = progress.beats.load(Ordering::Relaxed);
        if now != last {
            (last, since) = (now, Instant::now());
        }
        if let Some(host) = &remote
            && since.elapsed() >= limit
        {
            stop.store(true, Ordering::Relaxed);
            return Err(Failure::Failed(format!("\"{host}\" no responde")));
        }
    }
}

/// La suma publicada que se pegó en «Comparar con…»: el algoritmo (por el largo: 64 = SHA-256,
/// 32 = MD5) y la suma en minúscula. Sin importar espacios ni mayúsculas; acepta `suma  nombre`
/// (`sha256sum`), `SHA256 (nombre) = suma` (BSD) y la suma sola en una línea de varias (certutil).
pub fn parse_expected(text: &str) -> Option<(HashAlgo, String)> {
    let hex = |s: &str| -> Option<(HashAlgo, String)> {
        let algo = HashAlgo::from_hex_len(s.len())?;
        s.chars().all(|c| c.is_ascii_hexdigit()).then(|| (algo, s.to_ascii_lowercase()))
    };
    text.lines().map(str::trim).filter(|l| !l.is_empty()).find_map(|line| {
        let line = line.rsplit_once('=').map_or(line, |(_, after)| after.trim());
        let joined: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        hex(&joined).or_else(|| line.split_whitespace().next().and_then(|t| hex(t.trim_start_matches('\\'))))
    })
}

/// La suma partida en líneas que entren en `width` (con la letra de ancho fijo): de 64, 32 o 16
/// caracteres.
pub fn hex_lines(hex: &str, width: f32) -> Vec<String> {
    let fits = (width / MONO_W).floor() as usize;
    let per = [64, 32, 16].into_iter().find(|&n| n <= fits).unwrap_or(16);
    hex.as_bytes().chunks(per).map(|c| String::from_utf8_lossy(c).into_owned()).collect()
}

/// «1.234.567 (0x12D687)»: la posición de un byte (desde 0) en decimal y en hexadecimal.
pub fn offset_text(at: u64) -> String {
    format!("{} (0x{at:X})", crate::format::thousands(at))
}

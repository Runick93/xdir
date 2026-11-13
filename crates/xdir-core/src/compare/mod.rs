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
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use crate::archive_nav::temps;
use crate::sys::{self, CopyError, ErrKind, Flow, HashAlgo};

pub mod draw;

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
    pub fn disk_path(&self) -> &Path {
        match self {
            Source::Disk(p) => p,
            Source::Archive { file, .. } => file,
        }
    }

    /// La conexión, si se lee de una carpeta de red (sin E/S: `Network::is_remote`): el servidor
    /// de una ruta UNC o el nombre de la carpeta de red montada (la más alta que es de red), no el
    /// archivo.
    pub fn remote_host(&self) -> Option<String> {
        let p = self.disk_path();
        let net = sys::network();
        if !net.is_remote(p) {
            return None;
        }
        if crate::fs::is_unc(p) {
            return Some(crate::tab::remote_host(p));
        }
        let mount = p.ancestors().filter(|a| net.is_remote(a)).last().unwrap_or(p);
        Some(crate::tab::remote_host(mount))
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
        // Uno se terminó antes: los largos leídos (el del otro, hasta acá).
        if na != nb {
            return Ok(Some(Verdict::SizeDiffers { a: total + na as u64, b: total + nb as u64 }));
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

/// Un lector que cuenta cada lectura del sistema como avance (no cada bloque de 1 MB: una red lenta
/// pero viva no se corta por falta de avance) y deja de leer apenas se pide cortar.
struct Counted<'a> {
    inner: sys::Reader,
    progress: &'a Progress,
    stop: &'a AtomicBool,
    /// Bytes leídos.
    read: u64,
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other("cancelado"));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        self.progress.beat();
        Ok(n)
    }
}

/// Abre `src` (extrayéndolo si es de un comprimido) sin bajar lo que está solo en la nube.
fn open(src: &Source, stop: &AtomicBool, progress: &Progress) -> Result<sys::Opened, Failure> {
    open_at(src, stop, progress).map(|(f, _)| f)
}

/// `open`, con la ruta del disco que se abrió (la temporal, si es de un comprimido).
fn open_at(src: &Source, stop: &AtomicBool, progress: &Progress) -> Result<(sys::Opened, PathBuf), Failure> {
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
    let f = sys::hashes().open(&path).map_err(|e| failed(&name, &e))?;
    Ok((f, path))
}

/// ¿Son el mismo archivo? Por su identidad (disco y número de archivo, la del motor de copia): dos
/// caminos al mismo (un enlace a la carpeta, la unidad de red y su UNC, otras mayúsculas).
fn same_item(a: &Path, b: &Path) -> bool {
    let prims = sys::copy_prims();
    let id = |p: &Path| prims.stat(p).ok().and_then(|s| s.file_id);
    id(a).is_some_and(|x| id(b) == Some(x))
}

/// Compara el contenido de `a` y `b`: el mismo archivo, tamaños distintos (sin leer el contenido)
/// o byte a byte. Bloqueante (en hilos de trabajo); `stop` corta.
pub fn compare_sources(a: &Source, b: &Source, stop: &AtomicBool, progress: &Progress) -> Result<Verdict, Failure> {
    if a == b {
        return Ok(Verdict::SameFile);
    }
    let (fa, pa) = open_at(a, stop, progress)?;
    let (fb, pb) = open_at(b, stop, progress)?;
    progress.beat();
    if same_item(&pa, &pb) {
        return Ok(Verdict::SameFile);
    }
    if fa.size != fb.size {
        return Ok(Verdict::SizeDiffers { a: fa.size, b: fb.size });
    }
    progress.total.store(fa.size, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let size = fa.size;
    let mut ra = Counted { inner: fa.reader, progress, stop, read: 0 };
    let mut rb = Counted { inner: fb.reader, progress, stop, read: 0 };
    let res = compare_readers(&mut ra, &mut rb, &mut |n| {
        progress.done.store(n, Ordering::Relaxed);
        progress.beat();
        if stop.load(Ordering::Relaxed) { Flow::Stop } else { Flow::Continue }
    });
    // Leído entero con otro largo que el que tenía al abrirlo (o uno se terminó antes): cambió
    // mientras se comparaba; no es una diferencia de contenido.
    let changed = || {
        let name = if ra.read != size { a.name() } else { b.name() };
        Err(Failure::Failed(format!("«{name}» cambió mientras se comparaba")))
    };
    match res {
        Ok(Some(Verdict::SizeDiffers { .. })) => changed(),
        Ok(Some(Verdict::Same)) if ra.read != size || rb.read != size => changed(),
        Ok(Some(v)) => Ok(v),
        Ok(None) => Err(Failure::Cancelled),
        Err(_) if stop.load(Ordering::Relaxed) => Err(Failure::Cancelled),
        Err((side, e)) => {
            let name = if side == 0 { a.name() } else { b.name() };
            Err(Failure::Failed(format!("No se pudo leer «{name}»: {e}")))
        }
    }
}

/// La suma `algo` de `src` en hexadecimal en minúscula. Bloqueante; `stop` corta.
pub fn hash_source(src: &Source, algo: HashAlgo, stop: &AtomicBool, progress: &Progress) -> Result<String, Failure> {
    let f = open(src, stop, progress)?;
    progress.total.store(f.size, Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);
    let mut r = Counted { inner: f.reader, progress, stop, read: 0 };
    let res = sys::hashes().hash(&mut r, algo, &mut |n| {
        progress.done.store(n, Ordering::Relaxed);
        progress.beat();
        if stop.load(Ordering::Relaxed) { Flow::Stop } else { Flow::Continue }
    });
    match res {
        Ok(Some(h)) => Ok(h),
        Ok(None) => Err(Failure::Cancelled),
        Err(_) if stop.load(Ordering::Relaxed) => Err(Failure::Cancelled),
        Err(e) => Err(Failure::Failed(format!("No se pudo leer «{}»: {e}", src.name()))),
    }
}

/// Corre `work` en un hilo propio y espera su resultado (bloqueante: se llama desde otro hilo de
/// trabajo, nunca desde el de interfaz). `work` recibe una bandera para cortarse. Si `cancel` se
/// enciende, corta y devuelve `Cancelled` enseguida; si es de la red (`remote`, el servidor) y
/// pasa `limit` sin que `progress` avance, corta con «"srv" no responde». En los dos casos el hilo
/// queda abandonado si no responde (un servidor colgado no traba nada). Mientras espera, llama a
/// `tick` cada `every` (para redibujar el avance). El hilo que lee queda en `worker` (expulsar un
/// disco espera a que suelte los archivos).
#[allow(clippy::too_many_arguments)]
pub fn run_watched<T: Send + 'static>(
    remote: Option<String>,
    limit: Duration,
    cancel: &AtomicBool,
    progress: &Arc<Progress>,
    every: Duration,
    tick: &dyn Fn(),
    worker: &Mutex<Option<JoinHandle<()>>>,
    work: impl FnOnce(&AtomicBool) -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let s = stop.clone();
    let spawned = std::thread::Builder::new().name("xdir-comparar".into()).spawn(move || {
        let _ = tx.send(work(&s));
    });
    match spawned {
        Ok(h) => *worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(h),
        Err(_) => return Err(Failure::Failed("No se pudo crear un hilo".into())),
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
/// (`sha256sum`, también con la suma en grupos y nombres con `=`), `nombre: suma`,
/// `SHA256 (nombre) = suma` (BSD), `sha256:suma`, la fila de la tabla de `Get-FileHash` y la suma
/// sola en una línea de varias (certutil).
pub fn parse_expected(text: &str) -> Option<(HashAlgo, String)> {
    text.lines().find_map(parse_line)
}

/// La suma elegida de lo pegado: con varias líneas (un archivo de sumas entero), la que nombra al
/// archivo `name`; si ninguna, la primera. `line` es su número (desde 1) y `of`, cuántas sumas
/// había.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    pub algo: HashAlgo,
    pub hex: String,
    pub line: usize,
    pub of: usize,
    pub by_name: bool,
}

pub fn pick_expected(text: &str, name: &str) -> Option<Picked> {
    let found: Vec<(usize, &str, (HashAlgo, String))> = text.lines().enumerate().filter_map(|(i, l)| parse_line(l).map(|p| (i + 1, l, p))).collect();
    // El nombre como palabra de la línea, sin `*` (modo binario), comillas ni la carpeta.
    let names = |l: &str| {
        l.split_whitespace().any(|t| {
            let t = t.trim_matches(|c: char| matches!(c, '*' | '(' | ')' | '"' | '\'' | ','));
            let base = t.rsplit(['/', '\\']).next().unwrap_or(t);
            !name.is_empty() && base.eq_ignore_ascii_case(name)
        })
    };
    let of = found.len();
    let by_name = found.iter().position(|(_, l, _)| names(l));
    let (line, _, (algo, hex)) = found.into_iter().nth(by_name.unwrap_or(0))?;
    Some(Picked { algo, hex, line, of, by_name: by_name.is_some() })
}

/// La suma de una línea (ver `parse_expected`).
fn parse_line(line: &str) -> Option<(HashAlgo, String)> {
    let hex = |s: &str| -> Option<(HashAlgo, String)> {
        let algo = HashAlgo::from_hex_len(s.len())?;
        s.chars().all(|c| c.is_ascii_hexdigit()).then(|| (algo, s.to_ascii_lowercase()))
    };
    fn is_hex(t: &&&str) -> bool {
        t.chars().all(|c| c.is_ascii_hexdigit())
    }
    // `\` delante (sha256sum con nombres raros); el nombre (con `*` en modo binario) no cuenta.
    let tokens: Vec<&str> = line.split_whitespace().map(|t| t.trim_start_matches('\\')).collect();
    let leading: String = tokens.iter().take_while(is_hex).copied().collect();
    let trailing: String = tokens.iter().rev().take_while(is_hex).copied().collect::<Vec<_>>().into_iter().rev().collect();
    // Una palabra sola con la suma, sin lo de antes de `:` o `=` («sha256:…», «(x)=…») ni signos.
    let single = || {
        tokens.iter().find_map(|t| {
            let t = t.rsplit([':', '=']).next().unwrap_or(t);
            hex(t.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        })
    };
    hex(&tokens.concat()).or_else(|| hex(&leading)).or_else(single).or_else(|| hex(&trailing))
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

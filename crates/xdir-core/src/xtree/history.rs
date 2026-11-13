//! Historial de tamaños de xtree: una foto por análisis completo con el tamaño de las carpetas de
//! los dos primeros niveles y de las que pesan 100 MB o más, guardada por raíz en la carpeta de
//! configuración (`xtree-historial/<clave>.bin`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::tree::{NO_PARENT, Node};

/// 100 MB: desde este tamaño una carpeta entra en la foto aunque esté más abajo del segundo nivel.
pub const BIG: u64 = 100 * 1024 * 1024;
/// Niveles bajo la raíz que entran siempre (con algo adentro).
pub const LEVELS: usize = 2;
/// Tope de carpetas por foto (quedan las más pesadas): un disco con miles de carpetas grandes no
/// hace crecer el archivo sin control.
pub const MAX_ENTRIES: usize = 10_000;

/// Las carpetas de la foto de un árbol terminado: ruta relativa a la raíz (tramos unidos con `/`;
/// `""` es la raíz) y tamaño. La raíz siempre; después las carpetas con algo adentro de los dos
/// primeros niveles y las de 100 MB o más. Nunca archivos ni el grupo de archivos chicos.
pub fn select(arena: &[Node]) -> Vec<(String, u64)> {
    let Some(root) = arena.first().filter(|n| n.parent == NO_PARENT) else { return Vec::new() };
    let mut out = vec![(String::new(), root.size)];
    // (nodo, nivel, ruta). Debajo del segundo nivel solo se baja por carpetas grandes: lo de
    // adentro de una carpeta chica es más chico todavía.
    let mut stack: Vec<(u32, usize, String)> = vec![(0, 0, String::new())];
    while let Some((id, depth, path)) = stack.pop() {
        let Some(n) = arena.get(id as usize) else { continue };
        for &c in &n.children {
            let Some(k) = arena.get(c as usize) else { continue };
            if !k.is_dir || k.small || k.size == 0 {
                continue;
            }
            let d = depth + 1;
            if d > LEVELS && k.size < BIG {
                continue;
            }
            let p = if path.is_empty() { k.name.to_string() } else { format!("{path}/{}", k.name) };
            if p.len() > MAX_PATH {
                continue;
            }
            out.push((p.clone(), k.size));
            stack.push((c, d, p));
        }
    }
    if out.len() > MAX_ENTRIES {
        out[1..].sort_unstable_by_key(|e| std::cmp::Reverse(e.1));
        out.truncate(MAX_ENTRIES);
    }
    out
}

/// Versión del formato de `xtree-historial/*.bin`. Un archivo de otra versión se ignora (y se
/// reemplaza con la próxima foto).
pub const VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"XDHT";
/// Tope del archivo: uno más grande no se lee.
pub const MAX_FILE: usize = 32 * 1024 * 1024;
/// Tope del largo de una ruta relativa (bytes UTF-8): una más larga no entra en la foto.
pub const MAX_PATH: usize = 4096;
/// Tope de rutas distintas por archivo.
pub const MAX_PATHS: usize = 100_000;
/// Tope de fotos por archivo (con la consolidación hay unas 84).
pub const MAX_SNAPS: usize = 200;
/// Tope de pares (ruta, tamaño) sumando todas las fotos de un archivo.
pub const MAX_PAIRS: usize = 1_000_000;

/// Una foto: el día (días desde 1970-01-01 en hora local) y el tamaño de cada carpeta, por índice
/// en la tabla de rutas del historial.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub day: i32,
    pub sizes: Vec<(u32, u64)>,
}

/// El historial de una raíz: su ruta, la tabla de rutas relativas (compartida por todas las fotos)
/// y las fotos de la más vieja a la más nueva (un día no se repite).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub root: String,
    pub paths: Vec<String>,
    pub snaps: Vec<Snapshot>,
}

/// Serializa el historial. Solo van las rutas que usa alguna foto.
pub fn encode(h: &History) -> Vec<u8> {
    encode_raw(&pruned(h))
}

/// El historial con solo las rutas que usa alguna foto (los índices se renumeran, en el mismo
/// orden).
fn pruned(h: &History) -> History {
    let mut used = vec![false; h.paths.len()];
    for s in &h.snaps {
        for &(i, _) in &s.sizes {
            if let Some(u) = used.get_mut(i as usize) {
                *u = true;
            }
        }
    }
    let mut map = vec![u32::MAX; h.paths.len()];
    let mut paths = Vec::new();
    for (i, p) in h.paths.iter().enumerate() {
        if used[i] {
            map[i] = paths.len() as u32;
            paths.push(p.clone());
        }
    }
    let snaps = h
        .snaps
        .iter()
        .map(|s| Snapshot { day: s.day, sizes: s.sizes.iter().filter_map(|&(i, v)| map.get(i as usize).map(|&j| (j, v))).collect() })
        .collect();
    History { root: h.root.clone(), paths, snaps }
}

fn put_uint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_uint(out, s.len() as u64);
    out.extend(s.as_bytes());
}

/// Little endian; enteros sin signo en LEB128: `XDHT`, `u16` versión, raíz (largo + UTF-8),
/// cantidad de rutas y cada una (largo + UTF-8, relativa, tramos con `/`), cantidad de fotos y por
/// foto `i32` día, cantidad de pares y cada par (índice de ruta, tamaño).
fn encode_raw(h: &History) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend(VERSION.to_le_bytes());
    put_str(&mut out, &h.root);
    put_uint(&mut out, h.paths.len() as u64);
    for p in &h.paths {
        put_str(&mut out, p);
    }
    put_uint(&mut out, h.snaps.len() as u64);
    for s in &h.snaps {
        out.extend(s.day.to_le_bytes());
        put_uint(&mut out, s.sizes.len() as u64);
        for &(i, v) in &s.sizes {
            put_uint(&mut out, i as u64);
            put_uint(&mut out, v);
        }
    }
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }
    fn uint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for i in 0..10 {
            let b = *self.take(1)?.first()?;
            let part = (b & 0x7F) as u64;
            // El décimo byte solo puede aportar el bit 63.
            if i == 9 && part > 1 {
                return None;
            }
            v |= part << (7 * i);
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    /// Una cantidad de cosas que ocupan al menos `min` bytes cada una: no puede pasar de lo que
    /// queda ni de `max`.
    fn count(&mut self, min: usize, max: usize) -> Option<usize> {
        let n = usize::try_from(self.uint()?).ok()?;
        (n <= max && n <= self.0.len() / min.max(1)).then_some(n)
    }
    fn str(&mut self) -> Option<String> {
        let n = self.count(1, MAX_PATH)?;
        std::str::from_utf8(self.take(n)?).ok().map(str::to_string)
    }
}

/// Inverso de `encode`. `None` ante cualquier cosa rara (otra versión, cortado, índices fuera de la
/// tabla, días desordenados, cantidades imposibles): nunca entra en pánico ni reserva más memoria
/// que la que justifica el largo de `bytes`.
pub fn decode(bytes: &[u8]) -> Option<History> {
    if bytes.len() > MAX_FILE {
        return None;
    }
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC || u16::from_le_bytes(r.take(2)?.try_into().ok()?) != VERSION {
        return None;
    }
    let root = r.str()?;
    // Cada ruta ocupa al menos el byte de su largo. La reserva inicial es chica: si la cantidad
    // miente, el archivo se termina antes de crecer.
    let n_paths = r.count(1, MAX_PATHS)?;
    let mut paths = Vec::with_capacity(n_paths.min(4096));
    for _ in 0..n_paths {
        paths.push(r.str()?);
    }
    // Cada foto ocupa al menos el día y la cantidad de pares.
    let n_snaps = r.count(5, MAX_SNAPS)?;
    let mut snaps: Vec<Snapshot> = Vec::with_capacity(n_snaps);
    let mut pairs = 0usize;
    // Índices ya vistos en la foto: uno repetido es un archivo roto.
    let mut seen = vec![u32::MAX; paths.len()];
    for k in 0..n_snaps {
        let day = i32::from_le_bytes(r.take(4)?.try_into().ok()?);
        if snaps.last().is_some_and(|s| s.day >= day) {
            return None;
        }
        // Cada par ocupa al menos dos bytes.
        let n = r.count(2, MAX_ENTRIES)?;
        pairs += n;
        if pairs > MAX_PAIRS {
            return None;
        }
        let mut sizes = Vec::with_capacity(n);
        for _ in 0..n {
            let i = u32::try_from(r.uint()?).ok().filter(|&i| (i as usize) < paths.len())?;
            if std::mem::replace(&mut seen[i as usize], k as u32) == k as u32 {
                return None;
            }
            sizes.push((i, r.uint()?));
        }
        snaps.push(Snapshot { day, sizes });
    }
    r.0.is_empty().then_some(History { root, paths, snaps })
}

/// Lee un historial (bloqueante: en un hilo de trabajo). `None` si no existe, es más grande que
/// `MAX_FILE` o no se entiende.
pub fn read_file(path: &Path) -> Option<History> {
    use std::io::Read;
    #[cfg(test)]
    crate::sys::testing::FakeNetwork::note_disk_io("historial leer", path);
    let file = std::fs::File::open(path).ok()?;
    // Uno más grande que el tope ni se lee.
    if file.metadata().ok()?.len() > MAX_FILE as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes).ok()?;
    decode(&bytes)
}

/// Escribe un historial de forma atómica: un temporal propio de esta escritura en la misma carpeta
/// y renombrar (bloqueante: en un hilo de trabajo).
pub fn write_file(path: &Path, h: &History) -> std::io::Result<()> {
    #[cfg(test)]
    crate::sys::testing::FakeNetwork::note_disk_io("historial escribir", path);
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = path.parent().ok_or(std::io::ErrorKind::NotFound)?;
    std::fs::create_dir_all(dir)?;
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().ok_or(std::io::ErrorKind::NotFound)?.to_string_lossy();
    let tmp = dir.join(format!("{name}.{}.{n}.tmp", std::process::id()));
    std::fs::write(&tmp, encode(h))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Días con una foto por día (los más nuevos).
pub const DAILY_DAYS: i32 = 30;
/// Hasta cuántos días atrás se guarda (una por semana después de los diarios).
pub const KEEP_DAYS: i32 = 365;

impl History {
    /// Agrega la foto `sizes` del día `day`; una del mismo día se reemplaza.
    pub fn add(&mut self, day: i32, sizes: &[(String, u64)]) {
        let mut index: std::collections::HashMap<&str, u32> = self.paths.iter().enumerate().map(|(i, p)| (p.as_str(), i as u32)).collect();
        let mut new_paths = Vec::new();
        let mut pairs = Vec::with_capacity(sizes.len());
        for (p, v) in sizes {
            let i = match index.get(p.as_str()) {
                Some(&i) => i,
                None => {
                    let i = (self.paths.len() + new_paths.len()) as u32;
                    index.insert(p.as_str(), i);
                    new_paths.push(p.clone());
                    i
                }
            };
            pairs.push((i, *v));
        }
        drop(index);
        self.paths.extend(new_paths);
        let snap = Snapshot { day, sizes: pairs };
        match self.snaps.binary_search_by_key(&day, |s| s.day) {
            Ok(i) => self.snaps[i] = snap,
            Err(i) => self.snaps.insert(i, snap),
        }
    }

    /// Una foto por día los últimos 30 días, una por semana (la más nueva de cada una) hasta un año;
    /// lo más viejo se descarta. Una foto de un día posterior a `today` (el reloj volvió atrás)
    /// también.
    pub fn consolidate(&mut self, today: i32) {
        self.snaps.retain(|s| s.day <= today && today - s.day <= KEEP_DAYS);
        // De las viejas, la más nueva de cada semana: como están en orden, una se queda si la que
        // le sigue es de otra semana (o ya es de las diarias).
        let n = self.snaps.len();
        let keep: Vec<bool> = (0..n)
            .map(|i| {
                let d = self.snaps[i].day;
                today - d < DAILY_DAYS || self.snaps.get(i + 1).is_none_or(|next| today - next.day < DAILY_DAYS || next.day.div_euclid(7) != d.div_euclid(7))
            })
            .collect();
        let mut it = keep.into_iter();
        self.snaps.retain(|_| it.next().unwrap_or(true));
        // Los topes del archivo: si no entra, se van las fotos más viejas.
        while self.snaps.len() > 1 && !self.fits() {
            self.snaps.remove(0);
        }
    }

    /// ¿Entra en los topes de `decode` (fotos, pares y rutas usadas)?
    fn fits(&self) -> bool {
        let pairs: usize = self.snaps.iter().map(|s| s.sizes.len()).sum();
        let mut used = vec![false; self.paths.len()];
        for s in &self.snaps {
            for &(i, _) in &s.sizes {
                if let Some(u) = used.get_mut(i as usize) {
                    *u = true;
                }
            }
        }
        self.snaps.len() <= MAX_SNAPS && pairs <= MAX_PAIRS && used.iter().filter(|&&u| u).count() <= MAX_PATHS
    }
}

/// Carpeta del historial dentro de la de configuración.
pub const DIR_NAME: &str = "xtree-historial";

/// La ruta de la raíz como se compara: sin separador final (salvo que sea solo eso) y en
/// minúsculas.
fn norm(root: &Path) -> String {
    let text = root.to_string_lossy().to_lowercase();
    let trimmed = text.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() { text } else { trimmed.to_string() }
}

/// Nombre del archivo de una raíz (sin `.bin`): su último tramo legible y un FNV-1a de la ruta
/// normalizada, así dos raíces con el mismo nombre no chocan.
pub fn key(root: &Path) -> String {
    let n = norm(root);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in n.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let last = n.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("");
    let mut name: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).take(40).collect();
    if name.trim_matches(['_', '.']).is_empty() {
        name = "raiz".to_string();
    }
    format!("{name}-{hash:016x}")
}

/// Contador de «Borrar historial»: lo tiene la App y se lo pasa a la vista. El hilo de la ventana
/// solo lo lee (atómico, sin candado); sube al borrar.
pub type Clears = std::sync::Arc<std::sync::atomic::AtomicU64>;

/// Candado de E/S del historial: lo toman solo los hilos de trabajo mientras guardan, leen o
/// borran (nunca el hilo de la ventana), así un guardado y un borrado no se intercalan.
fn io_lock() -> std::sync::MutexGuard<'static, ()> {
    static IO: std::sync::Mutex<()> = std::sync::Mutex::new(());
    IO.lock().unwrap_or_else(|e| e.into_inner())
}

/// El candado de E/S tomado (pruebas).
#[cfg(test)]
pub fn hold_io_lock_for_tests() -> impl Drop {
    io_lock()
}

/// Corre `f` (guardar o leer el historial) con el candado de E/S, solo si nadie borró el historial
/// desde que el contador `clears` valía `seen`. `None` si se borró. Bloqueante: en un hilo de
/// trabajo.
pub fn if_current<T>(clears: &std::sync::atomic::AtomicU64, seen: u64, f: impl FnOnce() -> T) -> Option<T> {
    let _io = io_lock();
    (clears.load(std::sync::atomic::Ordering::SeqCst) == seen).then(f)
}

/// Borra la carpeta del historial `dir` entera (bloqueante: en un hilo de trabajo). Primero sube
/// `clears` (lo que estaba por guardarse ya no se guarda) y después espera a lo que esté en curso.
/// Que no exista no es un error.
pub fn clear(dir: &Path, clears: &std::sync::atomic::AtomicU64) -> Result<(), String> {
    clears.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _io = io_lock();
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("No se pudo borrar {}: {e}", dir.display())),
    }
}

/// Archivo del historial de `root` en la carpeta del historial `dir`.
pub fn file_for(dir: &Path, root: &Path) -> PathBuf {
    dir.join(format!("{}.bin", key(root)))
}

/// El historial de una raíz listo para consultar: por foto, su día, los niveles bajo la raíz que
/// cubre seguro (2 en la propia; menos si viene de una carpeta de más arriba) y los tamaños por
/// ruta relativa a la raíz.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub days: Vec<i32>,
    levels: Vec<i32>,
    index: HashMap<String, u32>,
    /// Por foto, pares (índice en `index`, tamaño) ordenados por índice.
    sizes: Vec<Vec<(u32, u64)>>,
}

impl Loaded {
    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// Tamaño de la carpeta `rel` en la foto `snap`: el guardado; 0 si no está pero la foto cubre
    /// su nivel (no existía o estaba vacía); `None` si no se sabe (más abajo y chica).
    pub fn value(&self, snap: usize, rel: &str) -> Option<u64> {
        let pairs = self.sizes.get(snap)?;
        if let Some(&i) = self.index.get(rel)
            && let Ok(k) = pairs.binary_search_by_key(&i, |p| p.0)
        {
            return Some(pairs[k].1);
        }
        let depth = if rel.is_empty() { 0 } else { rel.split('/').count() as i32 };
        (depth <= self.levels[snap]).then_some(0)
    }

    /// Índice de la ruta `rel` (la agrega si no estaba).
    fn intern(&mut self, rel: &str) -> u32 {
        if let Some(&i) = self.index.get(rel) {
            return i;
        }
        let n = self.index.len() as u32;
        self.index.insert(rel.to_string(), n);
        n
    }

    /// Une una foto (`levels`: niveles cubiertos; pares por índice de `intern`) si su día no está
    /// todavía.
    fn merge(&mut self, day: i32, levels: i32, mut sizes: Vec<(u32, u64)>) {
        let Err(at) = self.days.binary_search(&day) else { return };
        sizes.sort_unstable_by_key(|p| p.0);
        self.days.insert(at, day);
        self.levels.insert(at, levels);
        self.sizes.insert(at, sizes);
    }
}

/// Ruta relativa de `root` dentro de `top` (tramos con `/`); `None` si no está adentro.
fn rel_within(top: &Path, root: &Path) -> Option<String> {
    let rest = root.strip_prefix(top).ok()?;
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    Some(parts.join("/"))
}

/// El historial guardado de `root` en `dir`, si es de esa raíz y se entiende.
fn read_own(dir: &Path, root: &Path) -> Option<History> {
    read_file(&file_for(dir, root)).filter(|h| norm(Path::new(&h.root)) == norm(root))
}

/// Lee el historial de `root` de la carpeta del historial `dir` (bloqueante: en un hilo de
/// trabajo): el archivo propio y los de las carpetas de más arriba que la cubran, recortados a
/// `root` y unidos por día (gana el propio, después el más cercano).
pub fn load_for(dir: &Path, root: &Path) -> Loaded {
    let mut out = Loaded::default();
    for top in root.ancestors() {
        let Some(prefix) = rel_within(top, root) else { continue };
        let Some(h) = read_own(dir, top) else { continue };
        let depth = if prefix.is_empty() { 0 } else { prefix.split('/').count() as i32 };
        let under = format!("{prefix}/");
        // Cada ruta del archivo se recorta y se busca una sola vez: índice del archivo → índice
        // en `out` (o nada si queda fuera de la subcarpeta).
        let map: Vec<Option<u32>> = h
            .paths
            .iter()
            .map(|p| {
                let rel = if prefix.is_empty() {
                    p.as_str()
                } else if *p == prefix {
                    ""
                } else {
                    p.strip_prefix(under.as_str())?
                };
                Some(out.intern(rel))
            })
            .collect();
        let Some(top_id) = h.paths.iter().position(|p| *p == prefix).and_then(|i| map[i]) else { continue };
        for snap in &h.snaps {
            // La foto sirve si tiene a la subcarpeta.
            let pairs: Vec<(u32, u64)> = snap.sizes.iter().filter_map(|&(i, v)| map.get(i as usize).copied().flatten().map(|j| (j, v))).collect();
            if !pairs.iter().any(|&(j, _)| j == top_id) {
                continue;
            }
            out.merge(snap.day, LEVELS as i32 - depth, pairs);
        }
    }
    out
}

/// Agrega la foto `sizes` del día `today` al historial de `root` y lo escribe (bloqueante: en un
/// hilo de trabajo). Un archivo roto, de otra versión o de otra raíz se reemplaza.
pub fn save_snapshot(dir: &Path, root: &Path, today: i32, sizes: &[(String, u64)]) -> std::io::Result<()> {
    let mut h = read_own(dir, root).unwrap_or_else(|| History { root: root.to_string_lossy().into_owned(), ..Default::default() });
    h.add(today, sizes);
    h.consolidate(today);
    write_file(&file_for(dir, root), &h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children }
    }

    const MB: u64 = 1024 * 1024;

    /// raíz ─ a ─ b ─ c (200 MB) ─ d (150 MB) ─ e (50 MB)
    ///      │       └ chica (1 MB)
    ///      ├ vacia (0)
    ///      └ f.bin (archivo)
    fn sample() -> Vec<Node> {
        let mut t = vec![
            node("/r", NO_PARENT, 300 * MB, true, vec![1, 6, 7]),
            node("a", 0, 290 * MB, true, vec![2]),
            node("b", 1, 290 * MB, true, vec![3, 5]),
            node("c", 2, 200 * MB, true, vec![4]),
            node("d", 3, 150 * MB, true, vec![8]),
            node("chica", 2, MB, true, vec![]),
            node("vacia", 0, 0, true, vec![]),
            node("f.bin", 0, 10 * MB, false, vec![]),
            node("e", 4, 50 * MB, true, vec![9]),
            node("… 3 archivos pequeños", 8, 50 * MB, false, vec![]),
        ];
        t[9].small = true;
        t
    }

    #[test]
    fn select_two_levels_or_big() {
        let mut got = select(&sample());
        got.sort();
        let want: Vec<(String, u64)> = vec![
            ("".into(), 300 * MB),
            ("a".into(), 290 * MB),
            ("a/b".into(), 290 * MB),
            ("a/b/c".into(), 200 * MB),
            ("a/b/c/d".into(), 150 * MB),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn select_keeps_root_and_biggest_when_too_many() {
        let n = MAX_ENTRIES + 50;
        let mut t = vec![node("/r", NO_PARENT, 0, true, (1..=n as u32).collect())];
        for i in 0..n {
            t.push(node(&format!("d{i}"), 0, i as u64 + 1, true, vec![]));
        }
        let got = select(&t);
        assert_eq!(got.len(), MAX_ENTRIES);
        assert!(got.iter().any(|(p, _)| p.is_empty()), "la raíz queda");
        assert!(got.iter().all(|(p, s)| p.is_empty() || *s > 51), "quedan las más pesadas");
    }

    #[test]
    fn select_empty_arena() {
        assert!(select(&[]).is_empty());
    }

    fn hist() -> History {
        History {
            root: "/Users/x/Proyectos".into(),
            paths: vec!["".into(), "a".into(), "a/ñandú".into(), "sin usar".into()],
            snaps: vec![
                Snapshot { day: 20_400, sizes: vec![(0, 10), (1, 7)] },
                Snapshot { day: 20_401, sizes: vec![(0, 12 * 1024 * MB), (2, u64::MAX)] },
            ],
        }
    }

    /// Lo mismo sin la ruta que no usa ninguna foto (los índices se renumeran).
    fn hist_pruned() -> History {
        History {
            root: "/Users/x/Proyectos".into(),
            paths: vec!["".into(), "a".into(), "a/ñandú".into()],
            snaps: hist().snaps,
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let bytes = encode(&hist());
        assert_eq!(&bytes[..4], b"XDHT");
        assert_eq!(decode(&bytes), Some(hist_pruned()));
        let empty = History { root: "C:\\".into(), ..Default::default() };
        assert_eq!(decode(&encode(&empty)), Some(empty));
    }

    #[test]
    fn encode_dedups_paths_across_snapshots() {
        let mut h = hist_pruned();
        let one = encode(&h).len();
        h.snaps.push(Snapshot { day: 20_402, sizes: vec![(0, 1), (1, 1), (2, 1)] });
        // La foto nueva suma el día, la cantidad y tres pares chicos: las rutas no se repiten.
        assert!(encode(&h).len() - one < 16);
    }

    #[test]
    fn decode_rejects_foreign_version_and_magic() {
        let mut bytes = encode(&hist());
        bytes[4] = 2;
        assert_eq!(decode(&bytes), None);
        let mut bytes = encode(&hist());
        bytes[0] = b'Y';
        assert_eq!(decode(&bytes), None);
        assert_eq!(decode(b""), None);
    }

    #[test]
    fn decode_rejects_bad_index_and_order() {
        let mut h = hist_pruned();
        h.snaps[1].day = h.snaps[0].day;
        assert_eq!(decode(&encode_raw(&h)), None, "dos fotos del mismo día");
        let mut h = hist_pruned();
        h.snaps[0].sizes[0].0 = 9;
        assert_eq!(decode(&encode_raw(&h)), None, "índice fuera de la tabla");
    }

    /// Basura determinística (xorshift).
    fn noise(seed: &mut u64, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                *seed as u8
            })
            .collect()
    }

    #[test]
    fn decode_survives_truncation_and_garbage() {
        let bytes = encode(&hist());
        for cut in 0..bytes.len() {
            assert_eq!(decode(&bytes[..cut]), None, "cortado en {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert_eq!(decode(&longer), None, "bytes de más");
        let mut seed = 0x9E37_79B9_7F4A_7C15;
        for i in 0..3000 {
            let junk = noise(&mut seed, i % 300);
            let _ = decode(&junk);
            // Cabecera buena y el resto basura.
            let mut head = bytes[..6].to_vec();
            head.extend(junk);
            let _ = decode(&head);
        }
        // Cada byte cambiado de un archivo bueno.
        for i in 0..bytes.len() {
            for v in [0u8, 1, 0x7F, 0x80, 0xFF] {
                let mut b = bytes.clone();
                b[i] = v;
                let _ = decode(&b);
            }
        }
    }

    #[test]
    fn decode_huge_counts_do_not_allocate() {
        // Una tabla que dice tener 2^63 rutas, y una ruta de 2^40 bytes.
        let mut b = MAGIC.to_vec();
        b.extend(VERSION.to_le_bytes());
        b.push(0); // raíz vacía
        b.extend([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]);
        assert_eq!(decode(&b), None);
        let mut b = MAGIC.to_vec();
        b.extend(VERSION.to_le_bytes());
        b.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x20]);
        assert_eq!(decode(&b), None);
        // Un entero de más de 10 bytes.
        let mut b = MAGIC.to_vec();
        b.extend(VERSION.to_le_bytes());
        b.extend([0xFF; 12]);
        assert_eq!(decode(&b), None);
    }

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xdir-historial-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn write_file_is_atomic_and_read_back() {
        let dir = tmp_dir("escribir");
        let path = dir.join("x.bin");
        write_file(&path, &hist()).unwrap();
        assert_eq!(read_file(&path), Some(hist_pruned()));
        let mut h = hist_pruned();
        h.snaps.pop();
        write_file(&path, &h).unwrap();
        assert_eq!(read_file(&path), Some(pruned(&h)), "la ruta que ya no usa nadie no se escribe");
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["x.bin"], "sin temporales");
        std::fs::write(&path, b"basura").unwrap();
        assert_eq!(read_file(&path), None);
        assert_eq!(read_file(&dir.join("no-existe.bin")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_file_skips_oversized() {
        let dir = tmp_dir("grande");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.bin");
        let mut bytes = encode(&hist());
        bytes.resize(MAX_FILE + 1, 0);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read_file(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn days(h: &History) -> Vec<i32> {
        h.snaps.iter().map(|s| s.day).collect()
    }

    #[test]
    fn add_replaces_same_day_and_keeps_order() {
        let mut h = History::default();
        h.add(10, &[("".into(), 5), ("a".into(), 3)]);
        h.add(8, &[("".into(), 4)]);
        h.add(10, &[("".into(), 9), ("b".into(), 1)]);
        assert_eq!(days(&h), [8, 10]);
        let last = &h.snaps[1];
        let named: Vec<(&str, u64)> = last.sizes.iter().map(|&(i, v)| (h.paths[i as usize].as_str(), v)).collect();
        assert_eq!(named, [("", 9), ("b", 1)]);
        assert_eq!(h.paths.iter().filter(|p| p.is_empty()).count(), 1, "rutas sin repetir");
    }

    #[test]
    fn consolidate_daily_weekly_and_drop() {
        let today = 20_000;
        let mut h = History::default();
        // Un año y medio de fotos diarias.
        for d in (today - 540)..=today {
            h.add(d, &[("".into(), d as u64)]);
        }
        h.consolidate(today);
        let ds = days(&h);
        // Los últimos 30 días, todos.
        assert!(((today - DAILY_DAYS + 1)..=today).all(|d| ds.contains(&d)));
        // Antes, una por semana.
        let old: Vec<i32> = ds.iter().copied().filter(|&d| d <= today - DAILY_DAYS).collect();
        let weeks: Vec<i32> = old.iter().map(|d| d.div_euclid(7)).collect();
        let mut uniq = weeks.clone();
        uniq.dedup();
        assert_eq!(weeks, uniq, "una por semana");
        assert!(old.len() >= 47 && old.len() <= 50, "{}", old.len());
        // La más nueva de cada semana.
        assert!(old.iter().all(|d| d.rem_euclid(7) == 6 || *d == today - DAILY_DAYS));
        assert!(ds.iter().all(|&d| d >= today - KEEP_DAYS), "nada de más de un año");
        assert!(ds.len() < 85);
        // Consolidar de nuevo no cambia nada.
        let again = ds.clone();
        h.consolidate(today);
        assert_eq!(days(&h), again);
    }

    #[test]
    fn consolidate_drops_future_days() {
        let mut h = History::default();
        h.add(100, &[("".into(), 1)]);
        h.add(105, &[("".into(), 2)]);
        h.consolidate(101);
        assert_eq!(days(&h), [100]);
    }

    fn base(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("xdir-raices-{name}"))
    }

    #[test]
    fn key_is_stable_and_safe() {
        let r = base("clave").join("Proyectos");
        let with_sep = PathBuf::from(format!("{}{}", r.display(), std::path::MAIN_SEPARATOR));
        assert_eq!(key(&r), key(&with_sep));
        assert_eq!(key(&r), key(&PathBuf::from(r.to_string_lossy().to_uppercase())));
        assert_ne!(key(&r), key(&base("clave").join("Otros")));
        assert_ne!(key(&r), key(&base("clave2").join("Proyectos")), "mismo nombre, otra carpeta");
        let k = key(&base("clave").join("año 2026: ñ*?"));
        assert!(k.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)), "{k}");
        assert!(k.starts_with("a_o_2026"), "{k}");
        assert!(!key(Path::new("/")).is_empty());
    }

    fn values(l: &Loaded, rel: &str) -> Vec<Option<u64>> {
        (0..l.days.len()).map(|i| l.value(i, rel)).collect()
    }

    #[test]
    fn save_then_load_own_root() {
        let dir = tmp_dir("propio");
        let root = base("propio").join("r");
        save_snapshot(&dir, &root, 100, &[("".into(), 10), ("a".into(), 7), ("a/b/c/d".into(), 300 * MB)]).unwrap();
        save_snapshot(&dir, &root, 101, &[("".into(), 12), ("x".into(), 2)]).unwrap();
        save_snapshot(&dir, &root, 101, &[("".into(), 13), ("x".into(), 3)]).unwrap();
        let l = load_for(&dir, &root);
        assert_eq!(l.days, [100, 101]);
        assert_eq!(values(&l, ""), [Some(10), Some(13)]);
        assert_eq!(values(&l, "a"), [Some(7), Some(0)], "no estar en el primer nivel es pesar 0");
        assert_eq!(values(&l, "x"), [Some(0), Some(3)]);
        assert_eq!(values(&l, "a/b/c/d"), [Some(300 * MB), None], "más abajo y sin dato: no se sabe");
        assert!(load_for(&dir, &base("propio").join("otra")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subfolder_uses_ancestor_snapshots() {
        let dir = tmp_dir("sub");
        let top = base("sub");
        let sub = top.join("a").join("b");
        save_snapshot(&dir, &top, 100, &[("".into(), 50), ("a".into(), 40), ("a/b".into(), 30), ("a/b/c".into(), 20)]).unwrap();
        // Una foto de arriba que no tiene a la subcarpeta no sirve.
        save_snapshot(&dir, &top, 101, &[("".into(), 50), ("a".into(), 40)]).unwrap();
        save_snapshot(&dir, &top, 102, &[("".into(), 60), ("a/b".into(), 35)]).unwrap();
        // La propia gana el mismo día.
        save_snapshot(&dir, &sub, 102, &[("".into(), 36), ("c".into(), 1)]).unwrap();
        save_snapshot(&dir, &sub, 103, &[("".into(), 37)]).unwrap();
        let l = load_for(&dir, &sub);
        assert_eq!(l.days, [100, 102, 103]);
        assert_eq!(values(&l, ""), [Some(30), Some(36), Some(37)]);
        // La foto de arriba cubre dos niveles bajo `top`: bajo `a/b` ninguno seguro.
        assert_eq!(values(&l, "c"), [Some(20), Some(1), Some(0)]);
        assert_eq!(values(&l, "d"), [None, Some(0), Some(0)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_replaces_broken_or_foreign_file() {
        let dir = tmp_dir("roto");
        let root = base("roto").join("r");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file_for(&dir, &root), b"XDHT\x09\x00basura").unwrap();
        assert!(load_for(&dir, &root).is_empty());
        save_snapshot(&dir, &root, 100, &[("".into(), 1)]).unwrap();
        assert_eq!(load_for(&dir, &root).days, [100]);
        // Un archivo con esta clave pero de otra raíz (choque de clave) no se mezcla.
        let other = History { root: base("roto").join("otra").to_string_lossy().into_owned(), paths: vec!["".into()], snaps: vec![Snapshot { day: 90, sizes: vec![(0, 5)] }] };
        write_file(&file_for(&dir, &root), &other).unwrap();
        assert!(load_for(&dir, &root).is_empty());
        save_snapshot(&dir, &root, 100, &[("".into(), 2)]).unwrap();
        assert_eq!(load_for(&dir, &root).days, [100]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_consolidates() {
        let dir = tmp_dir("consolida");
        let root = base("consolida").join("r");
        save_snapshot(&dir, &root, 100, &[("".into(), 1)]).unwrap();
        save_snapshot(&dir, &root, 100 + KEEP_DAYS + 1, &[("".into(), 2)]).unwrap();
        assert_eq!(load_for(&dir, &root).days, [100 + KEEP_DAYS + 1]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un guardado que salió antes de «Borrar historial» no vuelve a crear la carpeta.
    #[test]
    fn clear_cancels_pending_save() {
        let dir = tmp_dir("generacion");
        let root = base("generacion").join("r");
        let clears = Clears::default();
        let g = clears.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(if_current(&clears, g, || save_snapshot(&dir, &root, 100, &[("".into(), 1)]).is_ok()), Some(true));
        assert!(dir.exists());
        clear(&dir, &clears).unwrap();
        assert_eq!(if_current(&clears, g, || save_snapshot(&dir, &root, 101, &[("".into(), 1)]).is_ok()), None);
        assert!(!dir.exists(), "no se vuelve a crear");
    }

    /// Topes duros: rutas por archivo, largo de una ruta, pares por archivo e índices repetidos en
    /// una foto se rechazan enseguida (un archivo armado a mano no cuesta memoria ni tiempo).
    #[test]
    fn decode_hard_caps() {
        let head = |n_paths: usize| {
            let mut b = MAGIC.to_vec();
            b.extend(VERSION.to_le_bytes());
            b.push(0);
            put_uint(&mut b, n_paths as u64);
            b
        };
        // Una ruta de más que el tope (todas vacías, cabe en el archivo).
        let mut b = head(MAX_PATHS + 1);
        b.extend(std::iter::repeat_n(0u8, MAX_PATHS + 1));
        b.push(0);
        assert_eq!(decode(&b), None, "demasiadas rutas");
        let mut ok = head(MAX_PATHS);
        ok.extend(std::iter::repeat_n(0u8, MAX_PATHS));
        ok.push(0);
        assert!(decode(&ok).is_some(), "justo el tope");
        // Una ruta más larga que el tope.
        let mut b = head(1);
        put_str(&mut b, &"x".repeat(MAX_PATH + 1));
        b.push(0);
        assert_eq!(decode(&b), None, "ruta larga");
        // El mismo índice dos veces en una foto.
        let h = History { root: "r".into(), paths: vec!["".into(), "a".into()], snaps: vec![Snapshot { day: 1, sizes: vec![(1, 5), (1, 6)] }] };
        assert_eq!(decode(&encode_raw(&h)), None, "índice repetido");
        // Más pares en total que el tope.
        let paths: Vec<String> = (0..MAX_ENTRIES).map(|i| i.to_string()).collect();
        let sizes: Vec<(u32, u64)> = (0..MAX_ENTRIES as u32).map(|i| (i, 1)).collect();
        let n = MAX_PAIRS / MAX_ENTRIES + 1;
        let snaps = (0..n).map(|d| Snapshot { day: d as i32, sizes: sizes.clone() }).collect();
        let h = History { root: "r".into(), paths, snaps };
        assert_eq!(decode(&encode_raw(&h)), None, "demasiados pares");
    }

    /// Lo que se escribe siempre entra en los topes: si hay demasiadas rutas distintas, se
    /// descartan las fotos más viejas.
    #[test]
    fn save_fits_caps() {
        let mut h = History::default();
        let per = MAX_ENTRIES;
        let snaps = MAX_PATHS / per + 3;
        for d in 0..snaps {
            let sizes: Vec<(String, u64)> = (0..per).map(|i| (format!("d{d}/{i}"), 1)).collect();
            h.add(d as i32, &sizes);
        }
        h.consolidate(snaps as i32);
        assert!(decode(&encode(&h)).is_some());
        assert_eq!(h.snaps.last().unwrap().day, snaps as i32 - 1, "quedan las nuevas");
    }
}

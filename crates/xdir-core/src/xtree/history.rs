//! Historial de tamaños de xtree: una foto por análisis completo con el tamaño de las carpetas de
//! los dos primeros niveles y de las que pesan 100 MB o más, guardada por raíz en la carpeta de
//! configuración (`xtree-historial/<clave>.bin`).

use std::path::Path;

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
            out.push((p.clone(), k.size));
            stack.push((c, d, p));
        }
    }
    if out.len() > MAX_ENTRIES {
        out[1..].sort_unstable_by(|a, b| b.1.cmp(&a.1));
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
/// Tope del largo de una ruta (bytes UTF-8).
const MAX_PATH: usize = 32 * 1024;
/// Tope de fotos por archivo (con la consolidación hay unas 83).
const MAX_SNAPS: usize = 1000;

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
    let n_paths = r.count(1, MAX_SNAPS * MAX_ENTRIES)?;
    let mut paths = Vec::with_capacity(n_paths.min(4096));
    for _ in 0..n_paths {
        paths.push(r.str()?);
    }
    // Cada foto ocupa al menos el día y la cantidad de pares.
    let n_snaps = r.count(5, MAX_SNAPS)?;
    let mut snaps: Vec<Snapshot> = Vec::with_capacity(n_snaps);
    for _ in 0..n_snaps {
        let day = i32::from_le_bytes(r.take(4)?.try_into().ok()?);
        if snaps.last().is_some_and(|s| s.day >= day) {
            return None;
        }
        // Cada par ocupa al menos dos bytes.
        let n = r.count(2, MAX_ENTRIES)?;
        let mut sizes = Vec::with_capacity(n);
        for _ in 0..n {
            let i = u32::try_from(r.uint()?).ok().filter(|&i| (i as usize) < paths.len())?;
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
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_FILE as u64 + 1).read_to_end(&mut bytes).ok()?;
    decode(&bytes)
}

/// Escribe un historial de forma atómica: un temporal propio de esta escritura en la misma carpeta
/// y renombrar (bloqueante: en un hilo de trabajo).
pub fn write_file(path: &Path, h: &History) -> std::io::Result<()> {
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
    }
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
}

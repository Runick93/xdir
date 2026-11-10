//! De dónde vino lo que está en la Papelera de macOS: el `.DS_Store` de cada Papelera (registros
//! `ptbL` y `ptbN` que escribe el Finder, solo lectura) y un registro propio de xdir
//! (`<config>/papelera.tsv`) con lo que manda él, por si el `.DS_Store` no lo tiene.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// ¿Es una Papelera? `~/.Trash` o `/Volumes/<disco>/.Trashes/<uid>` (sin E/S).
pub fn is_trash_dir(path: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.is_some_and(|h| path == h.join(".Trash")) {
        return true;
    }
    // SAFETY: `getuid` no falla ni toca memoria.
    let uid = unsafe { libc::getuid() }.to_string();
    let parts: Vec<_> = path.components().collect();
    parts.len() == 5
        && path.starts_with("/Volumes")
        && parts[3].as_os_str() == ".Trashes"
        && parts[4].as_os_str() == uid.as_str()
}

/// Raíz del disco de una Papelera: `/Volumes/X` para `/Volumes/X/.Trashes/…`; si no, `/`.
fn volume_root(trash_dir: &Path) -> PathBuf {
    let parts: Vec<_> = trash_dir.components().take(3).collect();
    if trash_dir.starts_with("/Volumes") && parts.len() == 3 { parts.iter().collect() } else { PathBuf::from("/") }
}

/// Lugar original de un elemento de la Papelera: primero el registro propio, después el
/// `.DS_Store` de esa Papelera.
pub fn origin(in_trash: &Path) -> Option<PathBuf> {
    origin_in(log_file().as_deref(), in_trash)
}

/// `origin` con el registro en `log` (los tests usan uno propio).
pub fn origin_in(log: Option<&Path>, in_trash: &Path) -> Option<PathBuf> {
    if let Some(found) = log.and_then(|l| read_log(l).into_iter().find(|(t, _)| t == in_trash).map(|(_, o)| o)) {
        return Some(found);
    }
    let dir = in_trash.parent()?;
    let name = in_trash.file_name()?.to_string_lossy().into_owned();
    let bytes = std::fs::read(dir.join(".DS_Store")).ok()?;
    let (folder, original) = parse_ds_store(&bytes).remove(&name)?;
    safe_origin(&volume_root(dir), &folder, &original)
}

/// `root` + `ptbL` + `ptbN`, solo si no se sale del disco: `ptbN` tiene que ser un nombre suelto y
/// `ptbL` solo nombres de carpeta (sin «..», «.» ni raíz más que la «/» inicial). El `.DS_Store`
/// viene del disco: uno armado no puede mandar a escribir en otro lado.
pub fn safe_origin(root: &Path, folder: &str, name: &str) -> Option<PathBuf> {
    use std::path::Component;
    let mut name_parts = Path::new(name).components();
    let (Some(Component::Normal(n)), None) = (name_parts.next(), name_parts.next()) else { return None };
    let folder = Path::new(folder.trim_start_matches('/'));
    if !folder.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let out = root.join(folder).join(n);
    out.starts_with(root).then_some(out)
}

/// `<config>/papelera.tsv`.
fn log_file() -> Option<PathBuf> {
    xdir_core::session::config_dir().map(|d| d.join("papelera.tsv"))
}

/// Anota que `original` quedó en la Papelera como `in_trash`.
pub fn log_trashed(in_trash: &Path, original: &Path) {
    if let Some(log) = log_file() {
        log_trashed_in(&log, in_trash, original);
    }
}

/// Olvida un elemento que salió de la Papelera.
pub fn forget(in_trash: &Path) {
    if let Some(log) = log_file() {
        forget_in(&log, in_trash);
    }
}

pub fn log_trashed_in(log: &Path, in_trash: &Path, original: &Path) {
    let mut lines = read_log(log);
    lines.retain(|(t, _)| t != in_trash);
    lines.push((in_trash.to_path_buf(), original.to_path_buf()));
    write_log(log, &lines);
}

pub fn forget_in(log: &Path, in_trash: &Path) {
    let mut lines = read_log(log);
    let before = lines.len();
    lines.retain(|(t, _)| t != in_trash);
    if lines.len() != before {
        write_log(log, &lines);
    }
}

fn read_log(log: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(log) else { return Vec::new() };
    text.lines().filter_map(|l| l.split_once('\t')).map(|(t, o)| (PathBuf::from(t), PathBuf::from(o))).collect()
}

/// Reescribe el registro sin lo que ya no está en la Papelera.
fn write_log(log: &Path, lines: &[(PathBuf, PathBuf)]) {
    let text: String = lines
        .iter()
        .filter(|(t, _)| std::fs::symlink_metadata(t).is_ok())
        .map(|(t, o)| format!("{}\t{}\n", t.display(), o.display()))
        .collect();
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(log, text);
}

/// Lector mínimo del `.DS_Store` (formato «Bud1», big endian): nombre en la Papelera →
/// (`ptbL`, `ptbN`), la carpeta (relativa a la raíz del disco) y el nombre originales. Ante
/// cualquier inconsistencia devuelve un mapa vacío; nunca entra en pánico.
pub fn parse_ds_store(bytes: &[u8]) -> HashMap<String, (String, String)> {
    let mut fields: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    if read_records(bytes, &mut fields).is_none() {
        return HashMap::new();
    }
    fields.into_iter().filter_map(|(k, (l, n))| Some((k, (l?, n?)))).collect()
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn utf16_at(b: &[u8], at: usize, units: usize) -> Option<String> {
    let s = b.get(at..at.checked_add(units.checked_mul(2)?)?)?;
    let u: Vec<u16> = s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    String::from_utf16(&u).ok()
}

fn read_records(b: &[u8], out: &mut HashMap<String, (Option<String>, Option<String>)>) -> Option<()> {
    if u32_at(b, 0)? != 1 || b.get(4..8)? != b"Bud1" {
        return None;
    }
    // Los desplazamientos del archivo no cuentan los 4 bytes del principio.
    let root = u32_at(b, 8)? as usize + 4;
    let count = u32_at(b, root)? as usize;
    if count > 1 << 16 {
        return None;
    }
    let mut at = root + 8;
    let mut addrs = Vec::with_capacity(count);
    for _ in 0..count {
        addrs.push(u32_at(b, at)?);
        at += 4;
    }
    // Relleno hasta 256 direcciones.
    if !count.is_multiple_of(256) {
        at += 4 * (256 - count % 256);
    }
    let names = u32_at(b, at)? as usize;
    at += 4;
    let mut dsdb = None;
    for _ in 0..names.min(1 << 16) {
        let len = *b.get(at)? as usize;
        let name = b.get(at + 1..at + 1 + len)?;
        let id = u32_at(b, at + 1 + len)?;
        at += 1 + len + 4;
        if name == b"DSDB" {
            dsdb = Some(id as usize);
        }
    }
    let block = |id: usize| -> Option<usize> {
        let addr = *addrs.get(id)?;
        Some((addr & !0x1f) as usize + 4)
    };
    let db = block(dsdb?)?;
    let root_node = u32_at(b, db)? as usize;
    let mut stack = vec![root_node];
    let mut seen = 0usize;
    while let Some(node) = stack.pop() {
        seen += 1;
        if seen > count + 1 {
            return None; // Ciclo o basura: no se sigue.
        }
        let mut at = block(node)?;
        let p = u32_at(b, at)?;
        let n = u32_at(b, at + 4)? as usize;
        at += 8;
        for _ in 0..n.min(1 << 16) {
            if p != 0 {
                stack.push(u32_at(b, at)? as usize);
                at += 4;
            }
            at = read_record(b, at, out)?;
        }
        if p != 0 {
            stack.push(p as usize);
        }
    }
    Some(())
}

/// Lee un registro en `at` y devuelve dónde empieza el siguiente.
fn read_record(b: &[u8], mut at: usize, out: &mut HashMap<String, (Option<String>, Option<String>)>) -> Option<usize> {
    let len = u32_at(b, at)? as usize;
    let name = utf16_at(b, at + 4, len)?;
    at += 4 + len * 2;
    let code = b.get(at..at + 4)?;
    let kind = b.get(at + 4..at + 8)?;
    at += 8;
    let mut text = None;
    at += match kind {
        b"bool" => 1,
        b"long" | b"shor" | b"type" => 4,
        b"comp" | b"dutc" => 8,
        b"blob" => 4 + u32_at(b, at)? as usize,
        b"ustr" => {
            let units = u32_at(b, at)? as usize;
            text = Some(utf16_at(b, at + 4, units)?);
            4 + units * 2
        }
        _ => return None,
    };
    let entry = out.entry(name).or_default();
    match code {
        b"ptbL" => entry.0 = text,
        b"ptbN" => entry.1 = text,
        _ => {}
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ustr(s: &str) -> Vec<u8> {
        let u: Vec<u16> = s.encode_utf16().collect();
        let mut v = (u.len() as u32).to_be_bytes().to_vec();
        for c in u {
            v.extend(c.to_be_bytes());
        }
        v
    }

    /// Un registro: nombre, código y valor `ustr`.
    fn record(name: &str, code: &[u8; 4], value: &str) -> Vec<u8> {
        let mut v = ustr(name);
        v.extend(code);
        v.extend(b"ustr");
        v.extend(ustr(value));
        v
    }

    /// Un registro de otro tipo (que se salta).
    fn record_long(name: &str) -> Vec<u8> {
        let mut v = ustr(name);
        v.extend(b"Iloc");
        v.extend(b"blob");
        v.extend(4u32.to_be_bytes());
        v.extend([0, 0, 0, 1]);
        v
    }

    /// Arma un `.DS_Store` con estos nodos (cada uno ya codificado, sin direcciones): bloque 0 =
    /// raíz, 1 = DSDB, 2.. = nodos; cada bloque ocupa 2048 bytes. `root` es el id del nodo raíz.
    fn ds_store(nodes: &[Vec<u8>], root: u32) -> Vec<u8> {
        const SIZE: usize = 2048;
        let blocks = 2 + nodes.len();
        let addr = |i: usize| ((SIZE * (i + 1)) as u32) | 11;
        let mut file = vec![0u8; 4 + SIZE * (blocks + 1)];
        file[0..4].copy_from_slice(&1u32.to_be_bytes());
        file[4..8].copy_from_slice(b"Bud1");
        file[8..12].copy_from_slice(&(SIZE as u32).to_be_bytes());
        let put = |file: &mut Vec<u8>, i: usize, data: &[u8]| {
            let at = SIZE * (i + 1) + 4;
            file[at..at + data.len()].copy_from_slice(data);
        };
        let mut root_block = (blocks as u32).to_be_bytes().to_vec();
        root_block.extend(0u32.to_be_bytes());
        for i in 0..blocks {
            root_block.extend(addr(i).to_be_bytes());
        }
        root_block.extend(vec![0u8; 4 * (256 - blocks)]);
        root_block.extend(1u32.to_be_bytes());
        root_block.push(4);
        root_block.extend(b"DSDB");
        root_block.extend(1u32.to_be_bytes());
        put(&mut file, 0, &root_block);
        let mut db = root.to_be_bytes().to_vec();
        db.extend([0u8; 16]);
        put(&mut file, 1, &db);
        for (i, n) in nodes.iter().enumerate() {
            put(&mut file, i + 2, n);
        }
        file
    }

    fn leaf(records: &[Vec<u8>]) -> Vec<u8> {
        let mut v = 0u32.to_be_bytes().to_vec();
        v.extend((records.len() as u32).to_be_bytes());
        for r in records {
            v.extend(r);
        }
        v
    }

    #[test]
    fn ds_store_reads_ptb_records() {
        let recs = [
            record("informe.pdf", b"ptbL", "Users/javier/Documents/"),
            record_long("informe.pdf"),
            record("informe.pdf", b"ptbN", "informe.pdf"),
            record("foto 10.12.03.jpg", b"ptbL", "Users/javier/Desktop/"),
            record("foto 10.12.03.jpg", b"ptbN", "foto.jpg"),
        ];
        let want = HashMap::from([
            ("informe.pdf".to_string(), ("Users/javier/Documents/".to_string(), "informe.pdf".to_string())),
            ("foto 10.12.03.jpg".to_string(), ("Users/javier/Desktop/".to_string(), "foto.jpg".to_string())),
        ]);
        // Una hoja con todo.
        let one = ds_store(&[leaf(&recs)], 2);
        assert_eq!(parse_ds_store(&one), want);
        // Un nodo interno (P = hoja derecha) con un registro y su hijo izquierdo.
        let left = leaf(&recs[..2]);
        let right = leaf(&recs[3..]);
        let mut inner = 4u32.to_be_bytes().to_vec();
        inner.extend(1u32.to_be_bytes());
        inner.extend(3u32.to_be_bytes());
        inner.extend(&recs[2]);
        let tree = ds_store(&[inner, left, right], 2);
        assert_eq!(parse_ds_store(&tree), want);
        // Bytes cortados o «Bud1» falso: vacío, sin pánico.
        assert!(parse_ds_store(&one[..3000]).is_empty());
        let mut fake = one.clone();
        fake[4..8].copy_from_slice(b"Bud2");
        assert!(parse_ds_store(&fake).is_empty());
        assert!(parse_ds_store(&[]).is_empty());
        for cut in [10, 100, 2100, 4100, 6200] {
            let _ = parse_ds_store(&one[..cut.min(one.len())]);
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-mac-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// El registro propio manda sobre el `.DS_Store`; sin registro, el `.DS_Store`; y al
    /// reescribir el registro se descartan los que ya no están en la Papelera.
    #[test]
    fn origin_prefers_own_log() {
        let dir = tmp("origen");
        let bin = dir.join("Trash");
        std::fs::create_dir_all(&bin).unwrap();
        let item = bin.join("a.txt");
        std::fs::write(&item, "a").unwrap();
        std::fs::write(bin.join(".DS_Store"), ds_store(&[leaf(&[record("a.txt", b"ptbL", "Users/x/Desktop/"), record("a.txt", b"ptbN", "a.txt")])], 2)).unwrap();
        let log = dir.join("papelera.tsv");
        assert_eq!(origin_in(Some(&log), &item), Some(PathBuf::from("/Users/x/Desktop/a.txt")));
        log_trashed_in(&log, &item, Path::new("/tmp/propio/a.txt"));
        assert_eq!(origin_in(Some(&log), &item), Some(PathBuf::from("/tmp/propio/a.txt")));
        // Otro que ya no está en la Papelera se descarta al reescribir.
        log_trashed_in(&log, &bin.join("ya-no.txt"), Path::new("/x"));
        assert_eq!(read_log(&log).len(), 1);
        forget_in(&log, &item);
        assert_eq!(origin_in(Some(&log), &item), Some(PathBuf::from("/Users/x/Desktop/a.txt")));
        assert_eq!(origin_in(Some(&log), &bin.join("otro.txt")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un `.DS_Store` armado (un disco ajeno) no puede mandar a devolver fuera del disco: un
    /// `ptbN` absoluto o con «/», o un `ptbL` con «..», dejan el origen desconocido.
    #[test]
    fn ds_store_origin_rejects_escapes() {
        let dir = tmp("escape");
        let bin = dir.join("Volumes/USB/.Trashes/501");
        std::fs::create_dir_all(&bin).unwrap();
        let cases = [
            ("abs", "docs/", "/Users/x/Library/LaunchAgents/evil.plist"),
            ("sub", "docs/", "a/b.txt"),
            ("dots", "docs/", ".."),
            ("up", "../../Users/x/Library/LaunchAgents/", "evil.plist"),
            ("up2", "docs/../../..", "evil.plist"),
            ("ok", "/docs/", "bien.txt"),
        ];
        let mut recs = Vec::new();
        for (name, l, n) in cases {
            std::fs::write(bin.join(name), "x").unwrap();
            recs.push(record(name, b"ptbL", l));
            recs.push(record(name, b"ptbN", n));
        }
        std::fs::write(bin.join(".DS_Store"), ds_store(&[leaf(&recs)], 2)).unwrap();
        for (name, ..) in &cases[..5] {
            assert_eq!(origin_in(None, &bin.join(name)), None, "{name}");
        }
        assert_eq!(safe_origin(Path::new("/Volumes/USB"), "/docs/", "bien.txt"), Some(PathBuf::from("/Volumes/USB/docs/bien.txt")));
        assert_eq!(safe_origin(Path::new("/"), "Users/x/", "a.txt"), Some(PathBuf::from("/Users/x/a.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trash_dirs_and_volume_roots() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(is_trash_dir(&home.join(".Trash")));
        assert!(!is_trash_dir(&home));
        let uid = unsafe { libc::getuid() };
        assert!(is_trash_dir(&PathBuf::from(format!("/Volumes/USB/.Trashes/{uid}"))));
        assert!(!is_trash_dir(Path::new("/Volumes/USB/.Trashes")));
        assert_eq!(volume_root(Path::new("/Volumes/USB/.Trashes/501")), PathBuf::from("/Volumes/USB"));
        assert_eq!(volume_root(&home.join(".Trash")), PathBuf::from("/"));
    }
}

//! Escaneo acelerado de volúmenes NTFS: lee la tabla maestra de archivos (MFT) directamente en
//! lugar de recorrer carpeta por carpeta. Requiere ejecutar como administrador (abre `\\.\C:`).
//!
//! Formato NTFS según su documentación pública; implementación propia.
//!
//! 1. `FSCTL_GET_NTFS_VOLUME_DATA` da el tamaño de cluster y de registro y dónde empieza la MFT.
//! 2. El registro 0 (`$MFT`) describe, con su lista de extensiones ("data runs"), dónde está el
//!    resto de la MFT en el disco.
//! 3. Varios hilos leen la MFT en bloques de 4 MiB y cada uno interpreta sus registros (fixups,
//!    `$FILE_NAME`, `$DATA`), guardando nombre, carpeta padre y tamaño ocupado.
//! 4. Se unen los registros de extensión a su registro base, se cuenta una vez cada archivo con
//!    enlaces duros y se arma el árbol desde la carpeta raíz (registro 5).

use crate::xtree::scan::{MAX_FILES_PER_DIR, Progress};
use crate::xtree::tree::{NO_PARENT, Node};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_BEGIN, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, SetFilePointerEx,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::PCWSTR;

const CHUNK: usize = 4 << 20;
const ROOT_RECORD: u32 = 5;
const FSCTL_GET_NTFS_VOLUME_DATA: u32 = 0x0009_0064;
const GENERIC_READ: u32 = 0x8000_0000;

const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const REC_IN_USE: u16 = 0x0001;
const REC_DIR: u16 = 0x0002;

const FA_REPARSE: u32 = 0x400;
const FA_CLOUD: u32 = 0x1000 | 0x40000 | 0x400000; // offline, recall on open, recall on data access

// Bits de `info` por registro.
const I_DIR: u8 = 1;
const I_REPARSE: u8 = 2;
const I_CLOUD: u8 = 4;

#[derive(Debug)]
pub enum Fail {
    /// Sin permisos para abrir el volumen: hay que ejecutar como administrador.
    Denied,
    /// No es NTFS, MFT fragmentada con lista de atributos, error de lectura, etc.
    Other,
}

struct Vol(HANDLE);
impl Drop for Vol {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open_volume(drive: &str) -> Result<Vol, Fail> {
    let path: Vec<u16> = format!("\\\\.\\{drive}").encode_utf16().chain(Some(0)).collect();
    let h = unsafe {
        CreateFileW(PCWSTR(path.as_ptr()), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, Default::default(), None)
    };
    match h {
        Ok(h) => Ok(Vol(h)),
        Err(e) if e.code().0 as u32 & 0xFFFF == 5 => Err(Fail::Denied),
        Err(_) => Err(Fail::Other),
    }
}

fn read_at(h: HANDLE, off: u64, buf: &mut [u8]) -> bool {
    unsafe {
        if SetFilePointerEx(h, off as i64, None, FILE_BEGIN).is_err() {
            return false;
        }
        let mut got = 0u32;
        ReadFile(h, Some(buf), Some(&mut got), None).is_ok() && got as usize == buf.len()
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Deshace la protección de sectores: los últimos 2 bytes de cada bloque de 512 se guardaron en
/// la matriz de actualización (USA) y en su lugar hay un número de secuencia.
fn apply_fixup(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let usa = u16_at(rec, 4) as usize;
    let n = u16_at(rec, 6) as usize;
    if n < 2 || usa + 2 * n > rec.len() || n * 512 < rec.len() {
        return false;
    }
    for i in 1..n {
        let end = i * 512;
        if end > rec.len() {
            return false;
        }
        if rec[end - 2..end] != rec[usa..usa + 2] {
            return false; // registro a medio escribir
        }
        let (a, b) = (usa + 2 * i, end - 2);
        rec[b] = rec[a];
        rec[b + 1] = rec[a + 1];
    }
    true
}

fn read_uint(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64)
}

fn read_int(b: &[u8]) -> i64 {
    let n = b.len();
    let mut v = read_uint(b) as i64;
    if n < 8 && b[n - 1] & 0x80 != 0 {
        v |= -1i64 << (8 * n);
    }
    v
}

/// Lista de extensiones: `(cluster inicial o None si es dispersa, cantidad de clusters)`.
fn parse_runs(b: &[u8]) -> Option<Vec<(Option<u64>, u64)>> {
    let mut out = Vec::new();
    let (mut i, mut lcn) = (0usize, 0i64);
    while i < b.len() && b[i] != 0 {
        let h = b[i];
        let (ls, os) = ((h & 0xF) as usize, (h >> 4) as usize);
        i += 1;
        if ls == 0 || ls > 8 || os > 8 || i + ls + os > b.len() {
            return None;
        }
        let len = read_uint(&b[i..i + ls]);
        i += ls;
        if os == 0 {
            out.push((None, len));
        } else {
            lcn += read_int(&b[i..i + os]);
            i += os;
            out.push((Some(lcn as u64), len));
        }
    }
    Some(out)
}

struct VolInfo {
    cluster: u64,
    rec_size: u64,
    mft_lcn: u64,
}

fn volume_info(h: HANDLE) -> Option<VolInfo> {
    let mut buf = [0u8; 256];
    let mut ret = 0u32;
    unsafe {
        DeviceIoControl(h, FSCTL_GET_NTFS_VOLUME_DATA, None, 0, Some(buf.as_mut_ptr() as *mut c_void), 256, Some(&mut ret), None).ok()?;
    }
    if ret < 72 {
        return None;
    }
    let (cluster, rec_size, lcn) = (u32_at(&buf, 44) as u64, u32_at(&buf, 48) as u64, u64_at(&buf, 64));
    // Los registros son de 1 KiB o 4 KiB, y 4 MiB tiene que ser múltiplo exacto.
    (cluster >= 512 && (rec_size == 1024 || rec_size == 4096) && CHUNK as u64 % rec_size == 0).then_some(VolInfo { cluster, rec_size, mft_lcn: lcn })
}

struct Chunk {
    disk_off: u64,
    len: usize,
    first_rec: u64,
}

/// Dónde está la MFT en el disco: se lee del propio registro 0 (`$MFT`).
fn mft_chunks(vol: HANDLE, info: &VolInfo) -> Option<(Vec<Chunk>, u64)> {
    let mut buf = vec![0u8; 8192];
    if !read_at(vol, info.mft_lcn * info.cluster, &mut buf) {
        return None;
    }
    let rec = &mut buf[..info.rec_size as usize];
    if !apply_fixup(rec) {
        return None;
    }
    let used = (u32_at(rec, 24) as usize).min(rec.len());
    let mut off = u16_at(rec, 20) as usize;
    let mut found = None;
    while off + 16 <= used {
        let ty = u32_at(rec, off);
        let len = u32_at(rec, off + 4) as usize;
        if ty == 0xFFFF_FFFF || len < 16 || off + len > used {
            break;
        }
        if ty == ATTR_DATA && rec[off + 8] != 0 && rec[off + 9] == 0 && off + 64 <= used {
            let runs_at = off + u16_at(rec, off + 32) as usize;
            let init = u64_at(rec, off + 56);
            found = Some((parse_runs(rec.get(runs_at..off + len)?)?, init));
            break;
        }
        off += len;
    }
    let (runs, init) = found?;
    let total_recs = init / info.rec_size;
    let mut budget = total_recs * info.rec_size;
    let (mut chunks, mut file_off) = (Vec::new(), 0u64);
    for (lcn, clusters) in runs {
        let run_bytes = clusters * info.cluster;
        let lcn = lcn?; // una MFT dispersa no es posible
        if run_bytes % info.rec_size != 0 {
            return None;
        }
        let take = run_bytes.min(budget);
        let mut pos = 0u64;
        while pos < take {
            let len = (CHUNK as u64).min(take - pos);
            chunks.push(Chunk { disk_off: lcn * info.cluster + pos, len: len as usize, first_rec: (file_off + pos) / info.rec_size });
            pos += len;
        }
        file_off += run_bytes;
        budget -= take;
        if budget == 0 {
            break;
        }
    }
    // Si la MFT está partida con una lista de atributos, lo que se vio acá no la cubre toda.
    (budget == 0).then_some((chunks, total_recs))
}

/// Un nombre de archivo: carpeta padre y posición del texto en el almacén del hilo.
struct NameEnt {
    parent: u32,
    off: u32,
    len: u16,
}

struct Rec {
    idx: u32,
    /// 0 si es un registro base; si no, el registro base al que complementa.
    base: u32,
    dir: bool,
    attrs: u32,
    size: u64,
    name_start: u32,
    name_count: u16,
}

#[derive(Default)]
struct Parsed {
    recs: Vec<Rec>,
    names: Vec<NameEnt>,
    arena: Vec<u16>,
}

fn parse_record(rec: &[u8], idx: u32, out: &mut Parsed, progress: &Progress) {
    let flags = u16_at(rec, 22);
    if flags & REC_IN_USE == 0 {
        return;
    }
    let base = (u64_at(rec, 32) & 0x0000_FFFF_FFFF_FFFF) as u32;
    let used = (u32_at(rec, 24) as usize).min(rec.len());
    let mut off = u16_at(rec, 20) as usize;
    let (mut size, mut attrs, mut have_attrs) = (0u64, 0u32, false);
    let name_start = out.names.len();
    while off + 16 <= used {
        let ty = u32_at(rec, off);
        let len = u32_at(rec, off + 4) as usize;
        if ty == 0xFFFF_FFFF || len < 16 || off + len > used {
            break;
        }
        let nonres = rec[off + 8] != 0;
        let name_len = rec[off + 9];
        if ty == ATTR_FILE_NAME && !nonres {
            let v = off + u16_at(rec, off + 20) as usize;
            if v + 66 <= off + len {
                let nl = rec[v + 64] as usize;
                // Espacio de nombres 2 = nombre corto DOS (8.3): es un duplicado.
                if rec[v + 65] != 2 && v + 66 + nl * 2 <= off + len {
                    if !have_attrs {
                        attrs = u32_at(rec, v + 56);
                        have_attrs = true;
                    }
                    let start = out.arena.len() as u32;
                    for k in 0..nl {
                        out.arena.push(u16_at(rec, v + 66 + 2 * k));
                    }
                    let parent = (u64_at(rec, v) & 0x0000_FFFF_FFFF_FFFF) as u32;
                    out.names.push(NameEnt { parent, off: start, len: nl as u16 });
                }
            }
        } else if ty == ATTR_DATA && name_len == 0 && nonres && off + 64 <= used {
            // Solo la primera extensión del flujo (VCN 0) trae los tamaños.
            if u64_at(rec, off + 16) == 0 {
                let compression_unit = u16_at(rec, off + 34);
                size = if compression_unit != 0 && len >= 72 { u64_at(rec, off + 64) } else { u64_at(rec, off + 40) };
            }
        }
        off += len;
    }
    let name_count = (out.names.len() - name_start) as u16;
    if base == 0 || name_count > 0 || size > 0 {
        if base == 0 && flags & REC_DIR == 0 {
            progress.files.fetch_add(1, Ordering::Relaxed);
        }
        if size > 0 {
            progress.bytes.fetch_add(size, Ordering::Relaxed);
        }
        out.recs.push(Rec { idx, base, dir: flags & REC_DIR != 0, attrs, size, name_start: name_start as u32, name_count });
    }
}

fn worker(drive: &str, chunks: &[Chunk], next: &AtomicUsize, rec_size: usize, progress: &Progress, cancel: &AtomicBool) -> Option<Parsed> {
    let vol = open_volume(drive).ok()?;
    let mut buf = vec![0u8; CHUNK];
    let mut out = Parsed::default();
    loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        if i >= chunks.len() || cancel.load(Ordering::Relaxed) {
            break;
        }
        let c = &chunks[i];
        if !read_at(vol.0, c.disk_off, &mut buf[..c.len]) {
            return None;
        }
        for r in 0..c.len / rec_size {
            let rec = &mut buf[r * rec_size..(r + 1) * rec_size];
            if apply_fixup(rec) {
                parse_record(rec, (c.first_rec + r as u64) as u32, &mut out, progress);
            }
        }
        progress.mft_done.fetch_add(c.len as u64, Ordering::Relaxed);
    }
    Some(out)
}

struct Link {
    owner: u32,
    parent: u32,
    wk: u8,
    off: u32,
    len: u16,
}

/// Escanea el volumen `drive` (por ejemplo `C:`). `label` es el nombre de la raíz.
pub fn scan(drive: &str, label: &str, progress: &Progress, cancel: &AtomicBool) -> Result<Vec<Node>, Fail> {
    let vol = open_volume(drive)?;
    let info = volume_info(vol.0).ok_or(Fail::Other)?;
    let (chunks, total) = mft_chunks(vol.0, &info).ok_or(Fail::Other)?;
    let total = total.min(u32::MAX as u64 - 1) as usize;
    progress.mft_total.store(chunks.iter().map(|c| c.len as u64).sum(), Ordering::Relaxed);
    progress.mft_done.store(0, Ordering::Relaxed);
    drop(vol);

    // Lectura e interpretación en paralelo.
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8);
    let parts: Vec<Option<Parsed>> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads).map(|_| s.spawn(|| worker(drive, &chunks, &next, info.rec_size as usize, progress, cancel))).collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(Fail::Other);
    }
    let parts: Vec<Parsed> = parts.into_iter().collect::<Option<_>>().ok_or(Fail::Other)?;
    progress.phase.store(2, Ordering::Relaxed);

    // Tamaño y tipo de cada registro base; los registros de extensión suman al suyo.
    let mut size = vec![0u64; total];
    let mut flag = vec![0u8; total];
    let mut links: Vec<Link> = Vec::new();
    for (wk, p) in parts.iter().enumerate() {
        for r in &p.recs {
            let owner = if r.base == 0 { r.idx } else { r.base } as usize;
            if owner >= total {
                continue;
            }
            size[owner] += r.size;
            if r.base == 0 {
                let mut f = 0;
                if r.dir {
                    f |= I_DIR;
                }
                if r.attrs & FA_REPARSE != 0 {
                    f |= I_REPARSE;
                }
                if r.attrs & FA_CLOUD != 0 {
                    f |= I_CLOUD;
                }
                flag[owner] = f | 0x80; // 0x80: el registro base existe y está en uso
            }
            for n in &p.names[r.name_start as usize..r.name_start as usize + r.name_count as usize] {
                links.push(Link { owner: owner as u32, parent: n.parent, wk: wk as u8, off: n.off, len: n.len });
            }
        }
    }

    // Hijos de cada carpeta (ordenados por carpeta padre).
    let mut start = vec![0u32; total + 2];
    for l in &links {
        if (l.parent as usize) < total {
            start[l.parent as usize + 1] += 1;
        }
    }
    for i in 0..=total {
        start[i + 1] += start[i];
    }
    let mut order = vec![0u32; start[total + 1] as usize];
    let mut fill = start.clone();
    for (i, l) in links.iter().enumerate() {
        if (l.parent as usize) < total {
            order[fill[l.parent as usize] as usize] = i as u32;
            fill[l.parent as usize] += 1;
        }
    }
    drop(fill);

    let name_of = |l: &Link| -> Box<str> {
        let a = &parts[l.wk as usize].arena[l.off as usize..l.off as usize + l.len as usize];
        String::from_utf16_lossy(a).into()
    };

    let mut nodes: Vec<Node> = Vec::new();
    nodes.push(Node { name: label.into(), parent: NO_PARENT, size: 0, is_dir: true, small: false, children: Vec::new() });
    let mut visited = vec![false; total];
    let mut counted = vec![false; total];
    let mut stack = vec![(ROOT_RECORD, 0u32)];
    if (ROOT_RECORD as usize) < total {
        visited[ROOT_RECORD as usize] = true;
    }
    while let Some((rec, nid)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Fail::Other);
        }
        let (a, b) = (start[rec as usize] as usize, start[rec as usize + 1] as usize);
        let mut dirs: Vec<&Link> = Vec::new();
        let mut files: Vec<(&Link, u64)> = Vec::new();
        for &li in &order[a..b] {
            let l = &links[li as usize];
            let o = l.owner as usize;
            if l.owner == rec || flag[o] & 0x80 == 0 {
                continue;
            }
            if flag[o] & I_DIR != 0 {
                if !visited[o] {
                    visited[o] = true;
                    dirs.push(l);
                }
            } else if !counted[o] {
                // Un archivo con varios enlaces duros ocupa espacio una sola vez.
                counted[o] = true;
                let s = if flag[o] & I_CLOUD != 0 { 0 } else { size[o] };
                if s > 0 {
                    files.push((l, s));
                }
            }
        }
        let (mut small_size, mut small_count) = (0u64, 0u32);
        if files.len() > MAX_FILES_PER_DIR {
            files.select_nth_unstable_by(MAX_FILES_PER_DIR, |x, y| y.1.cmp(&x.1));
            for (_, s) in files.drain(MAX_FILES_PER_DIR..) {
                small_size += s;
                small_count += 1;
            }
        }
        let base = nodes.len() as u32;
        let mut kids: Vec<u32> = Vec::with_capacity(dirs.len() + files.len() + 1);
        for l in &dirs {
            let id = nodes.len() as u32;
            nodes.push(Node { name: name_of(l), parent: nid, size: 0, is_dir: true, small: false, children: Vec::new() });
            kids.push(id);
            if flag[l.owner as usize] & I_REPARSE == 0 {
                stack.push((l.owner, id)); // las junctions y enlaces simbólicos no se siguen
            }
        }
        for (l, s) in &files {
            kids.push(nodes.len() as u32);
            nodes.push(Node { name: name_of(l), parent: nid, size: *s, is_dir: false, small: false, children: Vec::new() });
        }
        if small_count > 0 {
            kids.push(nodes.len() as u32);
            let name = format!("\u{2026} {small_count} archivos pequeños");
            nodes.push(Node { name: name.into(), parent: nid, size: small_size, is_dir: false, small: true, children: Vec::new() });
        }
        debug_assert!(kids.first().is_none_or(|&k| k == base));
        nodes[nid as usize].children = kids;
    }

    // Los hijos siempre tienen un id mayor que su padre: se suman de atrás hacia adelante.
    for i in (1..nodes.len()).rev() {
        let (p, s) = (nodes[i].parent as usize, nodes[i].size);
        nodes[p].size += s;
    }
    for id in 0..nodes.len() {
        let mut kids = std::mem::take(&mut nodes[id].children);
        kids.sort_unstable_by_key(|&c| std::cmp::Reverse(nodes[c as usize].size));
        nodes[id].children = kids;
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registro de 1 KiB con un `$FILE_NAME` (Win32) y un `$DATA` no residente.
    fn sample_record(parent: u64, name: &str, alloc: u64) -> Vec<u8> {
        let mut r = vec![0u8; 1024];
        r[0..4].copy_from_slice(b"FILE");
        r[4..6].copy_from_slice(&48u16.to_le_bytes()); // desplazamiento de la USA
        r[6..8].copy_from_slice(&3u16.to_le_bytes()); // 1 + 2 sectores
        r[20..22].copy_from_slice(&56u16.to_le_bytes()); // primer atributo
        r[22..24].copy_from_slice(&REC_IN_USE.to_le_bytes());
        let mut o = 56;
        // $FILE_NAME residente
        let nl = name.encode_utf16().count();
        let vlen = 66 + nl * 2;
        let alen = (24 + vlen + 7) & !7;
        r[o..o + 4].copy_from_slice(&ATTR_FILE_NAME.to_le_bytes());
        r[o + 4..o + 8].copy_from_slice(&(alen as u32).to_le_bytes());
        r[o + 16..o + 20].copy_from_slice(&(vlen as u32).to_le_bytes());
        r[o + 20..o + 22].copy_from_slice(&24u16.to_le_bytes());
        let v = o + 24;
        r[v..v + 8].copy_from_slice(&parent.to_le_bytes());
        r[v + 64] = nl as u8;
        r[v + 65] = 1;
        for (k, c) in name.encode_utf16().enumerate() {
            r[v + 66 + 2 * k..v + 68 + 2 * k].copy_from_slice(&c.to_le_bytes());
        }
        o += alen;
        // $DATA no residente sin nombre
        r[o..o + 4].copy_from_slice(&ATTR_DATA.to_le_bytes());
        r[o + 4..o + 8].copy_from_slice(&80u32.to_le_bytes());
        r[o + 8] = 1;
        r[o + 40..o + 48].copy_from_slice(&alloc.to_le_bytes());
        r[o + 48..o + 56].copy_from_slice(&(alloc - 100).to_le_bytes());
        o += 80;
        r[o..o + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        r[24..28].copy_from_slice(&((o + 8) as u32).to_le_bytes());
        // Protección de sectores: número de secuencia 0xABCD y los bytes originales en la USA.
        r[48..50].copy_from_slice(&0xABCDu16.to_le_bytes());
        for i in 1..3 {
            r[50 + 2 * (i - 1)..52 + 2 * (i - 1)].copy_from_slice(&[0x11 * i as u8, 0x22 * i as u8]);
            r[i * 512 - 2..i * 512].copy_from_slice(&0xABCDu16.to_le_bytes());
        }
        r
    }

    #[test]
    fn fixup_and_parse() {
        let mut r = sample_record(5, "hola.txt", 8192);
        assert!(apply_fixup(&mut r));
        assert_eq!(&r[510..512], &[0x11, 0x22]);
        let mut out = Parsed::default();
        parse_record(&r, 40, &mut out, &Progress::default());
        assert_eq!(out.recs.len(), 1);
        let rec = &out.recs[0];
        assert_eq!((rec.idx, rec.base, rec.size, rec.name_count), (40, 0, 8192, 1));
        let n = &out.names[0];
        assert_eq!(n.parent, 5);
        let name = String::from_utf16_lossy(&out.arena[n.off as usize..n.off as usize + n.len as usize]);
        assert_eq!(name, "hola.txt");
    }

    #[test]
    fn fixup_rejects_torn_record() {
        let mut r = sample_record(5, "x", 4096);
        r[511] ^= 0xFF;
        assert!(!apply_fixup(&mut r));
    }

    #[test]
    fn runs() {
        // 0x21: longitud de 1 byte, desplazamiento de 2 bytes. Segunda extensión con delta negativo.
        let b = [0x21, 0x10, 0x00, 0x10, 0x11, 0x08, 0xF0, 0x00];
        let r = parse_runs(&b).unwrap();
        assert_eq!(r, vec![(Some(0x1000), 0x10), (Some(0x1000 - 0x10), 8)]);
        // extensión dispersa
        assert_eq!(parse_runs(&[0x01, 0x05, 0x00]).unwrap(), vec![(None, 5)]);
    }

    #[test]
    fn signed_offsets() {
        assert_eq!(read_int(&[0xFF]), -1);
        assert_eq!(read_int(&[0x00, 0x80]), -32768);
        assert_eq!(read_int(&[0x34, 0x12]), 0x1234);
    }
}

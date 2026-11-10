use std::ffi::OsStr;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};

use crate::{
    ArchiveEntry, CancelRead, ErrKind, Error, Flow, ItemKind, MAX_LINK, OnFail, Shared, Source, cancelled, io_err, other,
    resolve_rel, write_entry,
};

type Reader<'a> = CancelRead<'a, GzDecoder<BufReader<File>>>;

fn kind_of_type(t: EntryType) -> ItemKind {
    match t {
        EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => ItemKind::File,
        EntryType::Directory => ItemKind::Dir,
        EntryType::Symlink => ItemKind::Symlink,
        // Enlaces duros, dispositivos, fifos: se listan pero no se extraen.
        _ => ItemKind::Omitted,
    }
}

fn open<'a>(path: &Path, check: &'a mut dyn FnMut() -> Flow) -> Result<Archive<Reader<'a>>, Error> {
    let f = File::open(path).map_err(|e| io_err(e, path))?;
    Ok(Archive::new(CancelRead { inner: GzDecoder::new(BufReader::new(f)), check }))
}

fn bad(e: std::io::Error, path: &Path) -> Error {
    // Un gzip o tar roto da InvalidInput/InvalidData/UnexpectedEof.
    match e.kind() {
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
            if e.get_ref().is_none_or(|i| !i.is::<crate::Cancelled>()) =>
        {
            other("El archivo está dañado", path)
        }
        _ => io_err(e, path),
    }
}

/// Destino del enlace; `None` si es demasiado largo.
fn link_of<R: std::io::Read>(e: &tar::Entry<'_, R>) -> Option<String> {
    let l = e.link_name().ok().flatten()?.to_string_lossy().into_owned();
    (l.len() as u64 <= MAX_LINK).then_some(l)
}

pub fn list(path: &Path, cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, Error> {
    let mut a = open(path, cancel)?;
    let mut out = Vec::new();
    for e in a.entries().map_err(|e| bad(e, path))? {
        let e = e.map_err(|e| bad(e, path))?;
        let kind = kind_of_type(e.header().entry_type());
        let name = e.path().map_err(|e| bad(e, path))?.to_string_lossy().into_owned();
        let link = if kind == ItemKind::Symlink { link_of(&e) } else { None };
        out.push(ArchiveEntry {
            index: out.len(),
            name,
            // Tamaño real (PAX / GNU sparse), no el de la cabecera.
            size: e.size(),
            kind,
            link,
            modified: e.header().mtime().unwrap_or(0),
            packed: 0,
            encrypted: false,
        });
    }
    Ok(out)
}

/// Recorre hasta la entrada (el orden de un tar.gz no se puede saltar); si hay muchas, conviene
/// `extract_all`.
pub fn extract(
    archive: &Path,
    entry: &ArchiveEntry,
    root: &Path,
    override_name: Option<&OsStr>,
    progress: &mut dyn FnMut(u64) -> Flow,
) -> Result<(), Error> {
    let sh = Shared::new(progress);
    let mut check = || sh.check();
    let mut a = open(archive, &mut check)?;
    let mut n = 0usize;
    for e in a.entries().map_err(|e| bad(e, archive))? {
        let mut e = e.map_err(|e| bad(e, archive))?;
        let kind = kind_of_type(e.header().entry_type());
        if n != entry.index {
            n += 1;
            continue;
        }
        let name = e.path().map_err(|e| bad(e, archive))?.to_string_lossy().into_owned();
        if kind == ItemKind::Omitted {
            return Err(other("Tipo de elemento no soportado", Path::new(&name)));
        }
        let Some(rel) = resolve_rel(&name, override_name, kind == ItemKind::Dir)? else {
            return Ok(());
        };
        let link = if kind == ItemKind::Symlink { link_of(&e) } else { None };
        let (declared, mode) = (e.size(), e.header().mode().ok());
        return write_entry(
            Source { kind, rel: &rel, link: link.as_deref(), declared, mode, reader: &mut e },
            root,
            &mut |b| sh.report(0, b),
        )
        .map(|_| ());
    }
    Err(Error { kind: ErrKind::NotFound, path: archive.to_path_buf() })
}

/// Una sola pasada; los enlaces simbólicos se crean al final.
pub fn extract_all(
    archive: &Path,
    root: &Path,
    want: &dyn Fn(&str) -> bool,
    progress: &mut dyn FnMut(u64) -> Flow,
    on_fail: OnFail,
) -> Result<usize, Error> {
    let sh = Shared::new(progress);
    let mut check = || sh.check();
    let mut a = open(archive, &mut check)?;
    let mut base = 0u64;
    let mut omitted = 0usize;
    let mut links: Vec<(String, std::path::PathBuf, Option<String>)> = Vec::new();
    for e in a.entries().map_err(|e| bad(e, archive))? {
        let mut e = e.map_err(|e| bad(e, archive))?;
        let kind = kind_of_type(e.header().entry_type());
        let name = e.path().map_err(|e| bad(e, archive))?.to_string_lossy().into_owned();
        if !want(&name) {
            continue;
        }
        if kind == ItemKind::Omitted {
            omitted += 1;
            continue;
        }
        let rel = match resolve_rel(&name, None, kind == ItemKind::Dir) {
            Ok(Some(rel)) => rel,
            Ok(None) => continue,
            Err(err) => {
                on_fail(&name, err)?;
                continue;
            }
        };
        if kind == ItemKind::Symlink {
            links.push((name, rel, link_of(&e)));
            continue;
        }
        let (declared, mode) = (e.size(), e.header().mode().ok());
        match write_entry(
            Source { kind, rel: &rel, link: None, declared, mode, reader: &mut e },
            root,
            &mut |b| sh.report(base, b),
        ) {
            Ok(n) => base += n,
            Err(err) => on_fail(&name, err)?,
        }
        if sh.report(base, 0) == Flow::Stop {
            return Err(cancelled(archive));
        }
    }
    drop(a);
    for (name, rel, link) in links {
        if cfg!(not(unix)) {
            omitted += 1;
            continue;
        }
        if let Err(err) = write_entry(
            Source { kind: ItemKind::Symlink, rel: &rel, link: link.as_deref(), declared: 0, mode: None, reader: &mut std::io::empty() },
            root,
            &mut |_| Flow::Continue,
        ) {
            on_fail(&name, err)?;
        }
        if sh.report(base, 0) == Flow::Stop {
            return Err(cancelled(archive));
        }
    }
    Ok(omitted)
}

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};

use crate::{ArchiveEntry, Error, Flow, ItemKind, Source, extract_to, io_err};

fn kind_of_type(t: EntryType) -> Option<ItemKind> {
    match t {
        EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => Some(ItemKind::File),
        EntryType::Directory => Some(ItemKind::Dir),
        EntryType::Symlink => Some(ItemKind::Symlink),
        // Enlaces duros, dispositivos, fifos: no se extraen ni se listan.
        _ => None,
    }
}

fn open(path: &Path) -> Result<Archive<GzDecoder<BufReader<File>>>, Error> {
    let f = File::open(path).map_err(|e| io_err(e, path))?;
    Ok(Archive::new(GzDecoder::new(BufReader::new(f))))
}

fn bad(e: std::io::Error, path: &Path) -> Error {
    // Un gzip o tar roto da InvalidInput/InvalidData/UnexpectedEof.
    match e.kind() {
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof => {
            Error::other("El archivo está dañado", path)
        }
        _ => io_err(e, path),
    }
}

pub fn list(path: &Path) -> Result<Vec<ArchiveEntry>, Error> {
    let mut a = open(path)?;
    let mut out = Vec::new();
    for e in a.entries().map_err(|e| bad(e, path))? {
        let e = e.map_err(|e| bad(e, path))?;
        let Some(kind) = kind_of_type(e.header().entry_type()) else { continue };
        let name = e.path().map_err(|e| bad(e, path))?.to_string_lossy().into_owned();
        let link = if kind == ItemKind::Symlink {
            e.link_name().ok().flatten().map(|l| l.to_string_lossy().into_owned())
        } else {
            None
        };
        out.push(ArchiveEntry {
            index: out.len(),
            name,
            size: e.header().size().unwrap_or(0),
            kind,
            link,
            modified: e.header().mtime().unwrap_or(0),
        });
    }
    Ok(out)
}

/// Recorre hasta la entrada (el orden de un tar.gz no se puede saltar).
pub fn extract(
    archive: &Path,
    entry: &ArchiveEntry,
    dst: &Path,
    progress: &mut dyn FnMut(u64) -> Flow,
) -> Result<(), Error> {
    let mut a = open(archive)?;
    let mut n = 0usize;
    for e in a.entries().map_err(|e| bad(e, archive))? {
        let mut e = e.map_err(|e| bad(e, archive))?;
        let Some(kind) = kind_of_type(e.header().entry_type()) else { continue };
        if n != entry.index {
            n += 1;
            continue;
        }
        let name = e.path().map_err(|e| bad(e, archive))?.to_string_lossy().into_owned();
        let link = e.link_name().ok().flatten().map(|l| l.to_string_lossy().into_owned());
        let declared = e.header().size().unwrap_or(0);
        let mode = e.header().mode().ok();
        return extract_to(
            Source { kind, name: &name, link: link.as_deref(), declared, mode, reader: &mut e },
            dst,
            progress,
        );
    }
    Err(Error::other("Ya no existe", archive))
}

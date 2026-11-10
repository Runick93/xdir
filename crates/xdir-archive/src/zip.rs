use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use ::zip::result::ZipError;
use ::zip::write::SimpleFileOptions;
use ::zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};
use xdir_core::archive::safe_entry_path;

use crate::{ArchiveEntry, ErrKind, Error, Flow, ItemKind, Source, extract_to, io_err};

const PASSWORD: &str = "Este zip tiene contraseña";
/// Los enlaces guardan su destino como datos: se acota.
const MAX_LINK: u64 = 4096;
/// Desde acá la entrada se marca zip64.
const ZIP64_AT: u64 = 0xFFFF_0000;

pub(crate) fn map_zip_ref(e: &ZipError, path: &Path) -> Error {
    match e {
        ZipError::InvalidPassword => Error::other(PASSWORD, path),
        ZipError::UnsupportedArchive(m) if *m == ZipError::PASSWORD_REQUIRED => Error::other(PASSWORD, path),
        ZipError::CompressionMethodNotSupported(_) | ZipError::UnsupportedArchive(_) => {
            Error::other("Formato no soportado", path)
        }
        _ => Error::other("El zip está dañado", path),
    }
}

fn map_zip(e: ZipError, path: &Path) -> Error {
    match e {
        ZipError::Io(e) => io_err(e, path),
        ZipError::InvalidPassword => Error::other(PASSWORD, path),
        ZipError::UnsupportedArchive(m) if m == ZipError::PASSWORD_REQUIRED => Error::other(PASSWORD, path),
        ZipError::CompressionMethodNotSupported(_) | ZipError::UnsupportedArchive(_) => {
            Error::other("Formato no soportado", path)
        }
        _ => Error::other("El zip está dañado", path),
    }
}

fn open(path: &Path) -> Result<ZipArchive<BufReader<File>>, Error> {
    let f = File::open(path).map_err(|e| io_err(e, path))?;
    ZipArchive::new(BufReader::new(f)).map_err(|e| map_zip(e, path))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// La hora de un zip no lleva zona: se toma como UTC.
fn epoch_of(dt: Option<DateTime>) -> u64 {
    let Some(t) = dt else { return 0 };
    let days = days_from_civil(t.year() as i64, t.month() as i64, t.day() as i64);
    let secs = days * 86400 + t.hour() as i64 * 3600 + t.minute() as i64 * 60 + t.second() as i64;
    secs.max(0) as u64
}

fn dos_time(secs: u64) -> Option<DateTime> {
    let (y, m, d) = civil_from_days((secs / 86400) as i64);
    let r = secs % 86400;
    DateTime::from_date_and_time(y as u16, m as u8, d as u8, (r / 3600) as u8, (r % 3600 / 60) as u8, (r % 60) as u8).ok()
}

fn read_link(z: &mut impl Read) -> Option<String> {
    let mut b = Vec::new();
    z.take(MAX_LINK).read_to_end(&mut b).ok()?;
    String::from_utf8(b).ok()
}

pub fn list(path: &Path) -> Result<Vec<ArchiveEntry>, Error> {
    let mut a = open(path)?;
    let mut out = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        // `by_index_raw` no descifra: así las entradas con contraseña también se listan.
        let (name, kind, size, modified, readable_link) = {
            let f = a.by_index_raw(i).map_err(|e| map_zip(e, path))?;
            let kind = if f.is_dir() {
                ItemKind::Dir
            } else if f.is_symlink() {
                ItemKind::Symlink
            } else {
                ItemKind::File
            };
            let name = f.name().map_err(|e| map_zip(e, path))?.into_owned();
            let ok = kind == ItemKind::Symlink && !f.encrypted() && f.compressed_size() <= MAX_LINK;
            (name, kind, f.size(), epoch_of(f.last_modified()), ok)
        };
        let link = if readable_link {
            let mut z = a.by_index(i).map_err(|e| map_zip(e, path))?;
            read_link(&mut z)
        } else {
            None
        };
        out.push(ArchiveEntry { index: i, name, size, kind, link, modified });
    }
    Ok(out)
}

pub fn extract(
    archive: &Path,
    entry: &ArchiveEntry,
    dst: &Path,
    progress: &mut dyn FnMut(u64) -> Flow,
) -> Result<(), Error> {
    let mut a = open(archive)?;
    if entry.index >= a.len() {
        return Err(Error::other("Ya no existe", archive));
    }
    {
        let raw = a.by_index_raw(entry.index).map_err(|e| map_zip(e, archive))?;
        if raw.encrypted() {
            return Err(Error::other(PASSWORD, archive));
        }
    }
    let mut z = a.by_index(entry.index).map_err(|e| map_zip(e, archive))?;
    // Lo que manda es el nombre guardado en el zip, no el que trae `entry`.
    let name = z.name().map_err(|e| map_zip(e, archive))?.into_owned();
    let (kind, mode, declared) = (
        if z.is_dir() { ItemKind::Dir } else if z.is_symlink() { ItemKind::Symlink } else { ItemKind::File },
        z.unix_mode(),
        z.size(),
    );
    let link = if kind == ItemKind::Symlink { read_link(&mut z) } else { None };
    extract_to(
        Source { kind, name: &name, link: link.as_deref(), declared, mode, reader: &mut z },
        dst,
        progress,
    )
}

pub fn create(out: &Path, items: &[(PathBuf, String)], progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), Error> {
    let f = OpenOptions::new().write(true).create_new(true).open(out).map_err(|e| io_err(e, out))?;
    let res = write_all(f, out, items, progress);
    if res.is_err() {
        let _ = std::fs::remove_file(out);
    }
    res
}

fn write_all(
    f: File,
    out: &Path,
    items: &[(PathBuf, String)],
    progress: &mut dyn FnMut(u64) -> Flow,
) -> Result<(), Error> {
    let mut zw = ZipWriter::new(BufWriter::new(f));
    let zerr = |e: ZipError| map_zip(e, out);
    let mut done = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    for (src, name) in items {
        if safe_entry_path(name).is_none() {
            return Err(Error::other("Nombre no permitido dentro del zip", src));
        }
        let meta = std::fs::symlink_metadata(src).map_err(|e| io_err(e, src))?;
        let mut opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        if let Some(t) = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).and_then(|d| dos_time(d.as_secs())) {
            opts = opts.last_modified_time(t);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            opts = opts.unix_permissions(meta.permissions().mode() & 0o777);
        }
        if name.ends_with('/') {
            zw.add_directory(name.trim_end_matches('/'), opts).map_err(zerr)?;
        } else if meta.file_type().is_symlink() {
            let target = std::fs::read_link(src).map_err(|e| io_err(e, src))?;
            zw.add_symlink(name.as_str(), target.to_string_lossy().replace('\\', "/"), opts).map_err(zerr)?;
        } else {
            if meta.len() >= ZIP64_AT {
                opts = opts.large_file(true);
            }
            zw.start_file(name.as_str(), opts).map_err(zerr)?;
            let mut r = File::open(src).map_err(|e| io_err(e, src))?;
            loop {
                let n = r.read(&mut buf).map_err(|e| io_err(e, src))?;
                if n == 0 {
                    break;
                }
                zw.write_all(&buf[..n]).map_err(|e| io_err(e, out))?;
                done += n as u64;
                if progress(done) == Flow::Stop {
                    return Err(Error { kind: ErrKind::Cancelled, path: out.to_path_buf() });
                }
            }
        }
    }
    let mut w = zw.finish().map_err(zerr)?;
    w.flush().map_err(|e| io_err(e, out))
}

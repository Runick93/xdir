//! Papelera de Linux según la especificación de freedesktop: lo borrado va a
//! `$XDG_DATA_HOME/Trash/files/<nombre>` y al lado queda `info/<nombre>.trashinfo` con la ruta
//! original y la fecha. Las funciones reciben la carpeta de la papelera, así se prueban en una
//! carpeta temporal sin tocar la del usuario.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use crate::ops::{MoveError, move_path};

/// Carpeta de datos del usuario: `$XDG_DATA_HOME` si es una ruta absoluta (la especificación
/// manda ignorar las relativas), si no `$HOME/.local/share`. Recibe los valores en vez de leerlos.
pub fn data_home_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    if let Some(x) = xdg.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(x);
    }
    home.filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".local").join("share"))
}

/// Carpeta de datos del usuario según las variables del proceso.
pub fn data_home() -> Option<PathBuf> {
    data_home_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

/// Papelera dentro de la carpeta de datos del usuario.
pub fn trash_dir(data_home: &Path) -> PathBuf {
    data_home.join("Trash")
}

/// Ruta codificada como URL para `Path=` del `.trashinfo`: queda todo byte fuera de los no
/// reservados de RFC 3986 y de `/` como `%XX`.
fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Hora local de ahora como `AAAA-MM-DDTHH:MM:SS` (la que pide `DeletionDate=`).
fn local_now() -> String {
    // SAFETY: `time` acepta un puntero nulo; `localtime_r` escribe en `tm`, que es nuestro (la
    // versión `_r` no usa memoria compartida entre hilos).
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return "1970-01-01T00:00:00".into();
    }
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Candidato `n` para `name` dentro de la papelera: el nombre tal cual y después "a (2).txt",
/// "a (3).txt"... (el número va antes de la extensión, salvo en carpetas y nombres con punto inicial).
fn candidate(name: &OsStr, is_dir: bool, n: u32) -> OsString {
    if n == 1 {
        return name.to_os_string();
    }
    let bytes = name.as_bytes();
    let dot = bytes.iter().rposition(|&b| b == b'.').filter(|&d| d > 0 && !is_dir);
    let (stem, ext) = match dot {
        Some(d) => (&bytes[..d], &bytes[d..]),
        None => (bytes, &b""[..]),
    };
    let mut out = stem.to_vec();
    out.extend_from_slice(format!(" ({n})").as_bytes());
    out.extend_from_slice(ext);
    OsString::from_vec(out)
}

/// `.trashinfo` de un nombre de la papelera.
fn info_path(trash: &Path, name: &OsStr) -> PathBuf {
    let mut file = name.to_os_string();
    file.push(".trashinfo");
    trash.join("info").join(file)
}

/// Carpeta de la papelera con permisos solo para el usuario (como pide la especificación).
fn make_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// Manda `path` a la papelera `trash`; devuelve su nombre dentro de `files/`. Primero reserva el
/// `.trashinfo` (creándolo en exclusiva, así dos borrados con el mismo nombre no se pisan) y
/// después mueve; si el movimiento falla, el original queda y se quita la reserva.
pub fn recycle(path: &Path, trash: &Path) -> io::Result<String> {
    recycle_with(path, trash, move_path)
}

/// `recycle` con la forma de mover recibida (los tests fuerzan el camino de copiar y borrar).
/// Si la copia a la papelera quedó completa pero el original no se pudo borrar del todo, la
/// entrada de la papelera (archivo y `.trashinfo`) queda: es la única copia entera.
fn recycle_with(path: &Path, trash: &Path, mover: impl Fn(&Path, &Path) -> Result<(), MoveError>) -> io::Result<String> {
    let meta = std::fs::symlink_metadata(path)?;
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ruta sin nombre"))?;
    let original = std::path::absolute(path)?;
    let (files, info) = (trash.join("files"), trash.join("info"));
    make_dir(&files)?;
    make_dir(&info)?;
    let text = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", encode_path(&original), local_now());
    for n in 1.. {
        let cand = candidate(name, meta.is_dir(), n);
        let dst = files.join(&cand);
        if std::fs::symlink_metadata(&dst).is_ok() {
            continue;
        }
        let ip = info_path(trash, &cand);
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&ip) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = f.write_all(text.as_bytes());
        drop(f);
        let moved = written.map_err(MoveError::Copy).and_then(|()| mover(path, &dst));
        return match moved {
            Ok(()) => Ok(cand.to_string_lossy().into_owned()),
            Err(MoveError::Copy(e)) => {
                let _ = std::fs::remove_file(&ip);
                Err(e)
            }
            Err(MoveError::Remove(e)) => Err(io::Error::other(format!(
                "se copió a la Papelera, pero no se pudo borrar del todo el original: {e}"
            ))),
        };
    }
    unreachable!("siempre hay un nombre libre")
}

/// Lugar original de `files/<name>`: el `Path=` de su `.trashinfo`, decodificado (`%20` → espacio).
pub fn origin(name: &str, trash: &Path) -> io::Result<PathBuf> {
    let text = std::fs::read_to_string(info_path(trash, OsStr::new(name)))?;
    let raw = text
        .lines()
        .find_map(|l| l.strip_prefix("Path="))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "el .trashinfo no tiene Path="))?;
    let path = PathBuf::from(OsString::from_vec(decode_path(raw.trim())));
    // Una ruta relativa es relativa a la carpeta que tiene a la papelera (papeleras de discos):
    // solo nombres de carpeta, sin «..» que la saquen del disco.
    if path.is_absolute() {
        return Ok(path);
    }
    if !path.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Path= se sale del disco"));
    }
    Ok(trash.parent().unwrap_or(Path::new("/")).join(path))
}

/// `%XX` → byte.
fn decode_path(text: &str) -> Vec<u8> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Devuelve `files/<name>` de la papelera a `original` y borra su `.trashinfo`. Si en `original`
/// ya hay algo, no se pisa: error y todo queda como estaba.
pub fn restore(name: &str, original: &Path, trash: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(original).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("ya existe {}", original.display())));
    }
    move_path(&trash.join("files").join(name), original).map_err(|e| match e {
        MoveError::Copy(e) => e,
        e @ MoveError::Remove(_) => io::Error::other(e.to_string()),
    })?;
    let _ = std::fs::remove_file(info_path(trash, OsStr::new(name)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Carpeta temporal propia de la prueba.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-linux-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `origin` lee el `Path=` del `.trashinfo` y lo decodifica.
    #[test]
    fn trashinfo_origin_decodes_path() {
        let dir = tmp("origen");
        let trash = dir.join("Trash");
        let f = dir.join("mi informe ñ.txt");
        std::fs::write(&f, "x").unwrap();
        let name = recycle(&f, &trash).unwrap();
        assert_eq!(origin(&name, &trash).unwrap(), std::path::absolute(&f).unwrap());
        std::fs::write(trash.join("info/raro.trashinfo"), "[Trash Info]\nPath=/home/a/con%20espacio/b%25.txt\n").unwrap();
        assert_eq!(origin("raro", &trash).unwrap(), PathBuf::from("/home/a/con espacio/b%.txt"));
        assert!(origin("no-esta", &trash).is_err());
        // Relativa (papelera de un disco): dentro del disco; con «..», no.
        std::fs::write(trash.join("info/rel.trashinfo"), "[Trash Info]\nPath=fotos/a.jpg\n").unwrap();
        assert_eq!(origin("rel", &trash).unwrap(), dir.join("fotos/a.jpg"));
        std::fs::write(trash.join("info/sube.trashinfo"), "[Trash Info]\nPath=../../etc/x\n").unwrap();
        assert!(origin("sube", &trash).is_err());
        assert_eq!(decode_path("a%2"), b"a%2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_home_uses_xdg_or_home() {
        assert_eq!(data_home_from(Some("/x/datos".into()), Some("/home/a".into())), Some(PathBuf::from("/x/datos")));
        assert_eq!(data_home_from(Some("rel".into()), Some("/home/a".into())), Some(PathBuf::from("/home/a/.local/share")));
        assert_eq!(data_home_from(None, Some("/home/a".into())), Some(PathBuf::from("/home/a/.local/share")));
        assert_eq!(data_home_from(None, None), None);
        assert_eq!(trash_dir(Path::new("/home/a/.local/share")), PathBuf::from("/home/a/.local/share/Trash"));
    }

    #[test]
    fn recycle_writes_trashinfo() {
        let dir = tmp("info");
        let trash = dir.join("Trash");
        let src = dir.join("con espacio ñ");
        std::fs::create_dir(&src).unwrap();
        let file = src.join("a.txt");
        std::fs::write(&file, "hola").unwrap();
        let name = recycle(&file, &trash).unwrap();
        assert_eq!(name, "a.txt");
        assert!(!file.exists());
        assert_eq!(std::fs::read_to_string(trash.join("files/a.txt")).unwrap(), "hola");
        let info = std::fs::read_to_string(trash.join("info/a.txt.trashinfo")).unwrap();
        let mut lines = info.lines();
        assert_eq!(lines.next(), Some("[Trash Info]"));
        let encoded = format!("{}/con%20espacio%20%C3%B1/a.txt", dir.display());
        assert_eq!(lines.next(), Some(format!("Path={encoded}").as_str()));
        let date = lines.next().unwrap().strip_prefix("DeletionDate=").expect("DeletionDate=");
        // AAAA-MM-DDTHH:MM:SS
        let b = date.as_bytes();
        assert_eq!(b.len(), 19, "{date}");
        assert!(b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        }), "{date}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycle_same_name_twice_keeps_both() {
        let dir = tmp("dos");
        let trash = dir.join("Trash");
        let (a, b) = (dir.join("uno"), dir.join("dos"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("a.txt"), "primero").unwrap();
        std::fs::write(b.join("a.txt"), "segundo").unwrap();
        assert_eq!(recycle(&a.join("a.txt"), &trash).unwrap(), "a.txt");
        assert_eq!(recycle(&b.join("a.txt"), &trash).unwrap(), "a (2).txt");
        assert_eq!(std::fs::read_to_string(trash.join("files/a.txt")).unwrap(), "primero");
        assert_eq!(std::fs::read_to_string(trash.join("files/a (2).txt")).unwrap(), "segundo");
        assert!(trash.join("info/a (2).txt.trashinfo").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycle_keeps_entry_when_source_removal_fails() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("recycle_keeps_entry_when_source_removal_fails: se saltea como root (los permisos no aplican)");
            return;
        }
        let dir = tmp("a-medias");
        let trash = dir.join("Trash");
        let src = dir.join("carpeta");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/f"), "f").unwrap();
        // Una subcarpeta de solo lectura: se copia, pero su archivo no se puede borrar.
        let ro = std::fs::Permissions::from_mode(0o500);
        std::fs::set_permissions(src.join("sub"), ro).unwrap();
        // Camino de otro disco: copiar y borrar.
        let r = recycle_with(&src, &trash, crate::ops::copy_then_remove);
        let rw = std::fs::Permissions::from_mode(0o755);
        let _ = std::fs::set_permissions(src.join("sub"), rw.clone());
        let _ = std::fs::set_permissions(trash.join("files/carpeta/sub"), rw);
        let e = r.expect_err("el original no se pudo borrar del todo");
        assert!(e.to_string().contains("no se pudo borrar del todo"), "{e}");
        assert_eq!(std::fs::read_to_string(trash.join("files/carpeta/sub/f")).unwrap(), "f");
        assert!(trash.join("info/carpeta.trashinfo").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_puts_file_back() {
        let dir = tmp("restaurar");
        let trash = dir.join("Trash");
        let carpeta = dir.join("carpeta");
        std::fs::create_dir_all(carpeta.join("sub")).unwrap();
        std::fs::write(carpeta.join("sub/x"), "x").unwrap();
        let name = recycle(&carpeta, &trash).unwrap();
        assert!(!carpeta.exists());
        restore(&name, &carpeta, &trash).unwrap();
        assert_eq!(std::fs::read_to_string(carpeta.join("sub/x")).unwrap(), "x");
        assert!(!trash.join("files").join(&name).exists());
        assert!(!trash.join("info").join(format!("{name}.trashinfo")).exists());
        // Si el original volvió a aparecer, no se pisa.
        let name = recycle(&carpeta, &trash).unwrap();
        std::fs::create_dir(&carpeta).unwrap();
        assert!(restore(&name, &carpeta, &trash).is_err());
        assert!(trash.join("files").join(&name).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Lugares de Linux: discos montados (`/` y lo de `/media`, `/mnt` y `/run/media`), carpetas del
//! usuario según `user-dirs.dirs` (XDG), la Papelera y el espacio de un volumen. Lo que lee
//! archivos o variables del proceso tiene una versión pura (`mounts_from`, `user_dirs_from`) que
//! es la que se prueba.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use xdir_core::fs::Location;

/// Carpetas donde se montan los discos externos.
const MOUNT_BASES: [&str; 3] = ["/media", "/mnt", "/run/media"];

/// Nombre de la raíz del sistema en títulos y en "Este equipo".
const ROOT_LABEL: &str = "Sistema de archivos";

/// Carpetas del usuario, en el orden del núcleo (Escritorio, Descargas, Documentos, Imágenes,
/// Música, Vídeos): la clave de `user-dirs.dirs` y los nombres que se prueban si no está.
const USER_FOLDERS: [(&str, [&str; 2]); 6] = [
    ("DESKTOP", ["Escritorio", "Desktop"]),
    ("DOWNLOAD", ["Descargas", "Downloads"]),
    ("DOCUMENTS", ["Documentos", "Documents"]),
    ("PICTURES", ["Imágenes", "Pictures"]),
    ("MUSIC", ["Música", "Music"]),
    ("VIDEOS", ["Vídeos", "Videos"]),
];

pub const USER_FOLDER_COUNT: usize = USER_FOLDERS.len();

/// Carpeta personal (`$HOME`), si está definida.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// Variable XDG con una ruta absoluta (la especificación manda ignorar las relativas).
fn xdg_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// Deshace los escapes octales de `/proc/self/mounts` (`\040` es un espacio). Solo `\000` a
/// `\377` caben en un byte; lo demás queda tal cual.
fn unescape_mount(s: &str) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && (b'0'..=b'3').contains(&b[i + 1])
            && b[i + 2..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    OsString::from_vec(out)
}

/// Puntos de montaje de discos externos en el texto de `/proc/self/mounts`: los que están dentro
/// de `/media`, `/mnt` o `/run/media`, ordenados y sin repetir. Los que están dentro de otro de la
/// lista no son discos aparte (en WSL, `/mnt/wslg/distro` dentro de `/mnt/wslg`).
pub fn mounts_from(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = text
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|m| PathBuf::from(unescape_mount(m)))
        .filter(|p| MOUNT_BASES.iter().any(|b| p.starts_with(b) && p != Path::new(b)))
        .collect();
    out.sort();
    out.dedup();
    // Ordenadas, un montaje anidado viene después del que lo contiene.
    let mut kept: Vec<PathBuf> = Vec::with_capacity(out.len());
    for m in out {
        if !kept.iter().any(|k| m.starts_with(k)) {
            kept.push(m);
        }
    }
    kept
}

/// Discos montados: la raíz y los externos.
pub fn drives() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/")];
    if let Ok(text) = std::fs::read_to_string("/proc/self/mounts") {
        out.extend(mounts_from(&text));
    }
    out
}

/// Identificador de un disco para `drive_label`: su ruta de montaje.
pub fn drive_name(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

/// Nombre de un disco: el último tramo de su punto de montaje, o "Sistema de archivos" para `/`.
pub fn drive_label(drive: &str) -> String {
    match Path::new(drive).file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => ROOT_LABEL.to_string(),
    }
}

/// ¿Es una ruta de un disco externo que ya no está montado?
pub fn drive_missing(path: &Path) -> bool {
    MOUNT_BASES.iter().any(|b| path.starts_with(b) && path != Path::new(b))
        && !path.exists()
        && !drives().iter().any(|d| d != Path::new("/") && path.starts_with(d))
}

/// Capacidad (total, libre) en bytes del volumen que contiene `path`.
pub fn volume_size(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` es una cadena C válida y `st` una estructura del tamaño que espera `statvfs`.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let unit = st.f_frsize as u64;
    Some((st.f_blocks as u64 * unit, st.f_bavail as u64 * unit))
}

/// Carpetas de `user-dirs.dirs`: (clave, ruta), como `("DOWNLOAD", "/home/a/Descargas")`. Acepta
/// `"$HOME/..."` y rutas absolutas, como manda la especificación; lo demás se ignora.
pub fn user_dirs_from(text: &str, home: &Path) -> Vec<(String, PathBuf)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (key, value) = l.split_once('=')?;
            let key = key.trim().strip_prefix("XDG_")?.strip_suffix("_DIR")?;
            let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
            let path = if value == "$HOME" {
                home.to_path_buf()
            } else if let Some(rest) = value.strip_prefix("$HOME/") {
                home.join(rest)
            } else if value.starts_with('/') {
                PathBuf::from(value)
            } else {
                return None;
            };
            Some((key.to_string(), path))
        })
        .collect()
}

/// Carpeta del usuario `i` (ver `USER_FOLDERS`) entre las de `user-dirs.dirs`, o la primera de
/// los nombres de siempre que exista. Una carpeta XDG igual a la personal está desactivada.
fn user_folder_from(i: usize, home: &Path, xdg: &[(String, PathBuf)], is_dir: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let (key, names) = USER_FOLDERS.get(i)?;
    if let Some((_, p)) = xdg.iter().find(|(k, _)| k == key) {
        return (p != home && is_dir(p)).then(|| p.clone());
    }
    names.iter().map(|n| home.join(n)).find(|p| is_dir(p))
}

/// Ruta de la carpeta del usuario `i`, si existe.
pub fn user_folder(i: usize) -> Option<PathBuf> {
    let home = home()?;
    let config = xdg_var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    user_folder_from(i, &home, &user_dirs_from(&text, &home), Path::is_dir)
}

/// Archivos de la Papelera (especificación de freedesktop), si existe.
pub fn trash_dir() -> Option<PathBuf> {
    let data = xdg_var("XDG_DATA_HOME").or_else(|| home().map(|h| h.join(".local").join("share")))?;
    Some(data.join("Trash").join("files")).filter(|p| p.is_dir())
}

/// Subcarpetas de un lugar, para el menú de las flechas de la barra de dirección.
pub fn subfolders(loc: &Location) -> Vec<(String, Location)> {
    const MAX: usize = 500;
    match loc {
        Location::Computer => drives().into_iter().map(|d| (drive_label(&drive_name(&d)), Location::Path(d))).collect(),
        Location::Path(dir) => {
            let mut out: Vec<(String, Location)> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), Location::Path(e.path())))
                .collect();
            out.sort_by_key(|(n, _)| n.to_lowercase());
            out.truncate(MAX);
            out
        }
        Location::Shell { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_folder_prefers_xdg_then_fallback_names() {
        let home = Path::new("/home/a");
        let xdg = user_dirs_from("XDG_DOWNLOAD_DIR=\"/datos/bajadas\"\nXDG_DESKTOP_DIR=\"$HOME\"\n", home);
        let exists = |p: &Path| p != Path::new("/home/a/Escritorio");
        assert_eq!(user_folder_from(1, home, &xdg, exists), Some(PathBuf::from("/datos/bajadas")));
        // El escritorio XDG igual a la carpeta personal está desactivado.
        assert_eq!(user_folder_from(0, home, &xdg, exists), None);
        // Sin XDG: el nombre en castellano y, si no existe, el inglés.
        assert_eq!(user_folder_from(2, home, &[], exists), Some(PathBuf::from("/home/a/Documentos")));
        assert_eq!(user_folder_from(0, home, &[], exists), Some(PathBuf::from("/home/a/Desktop")));
        assert_eq!(user_folder_from(6, home, &[], exists), None);
    }

    #[test]
    fn mounts_from_unescapes_spaces() {
        assert_eq!(mounts_from("/dev/sdb1 /media/a/Mi\\040disco vfat rw 0 0\n"), [PathBuf::from("/media/a/Mi disco")]);
    }

    #[test]
    fn unescape_mount_keeps_out_of_range_escapes() {
        // `\440` no cabe en un byte: queda literal en vez de desbordar.
        assert_eq!(unescape_mount("/media/a\\440b"), OsString::from("/media/a\\440b"));
        assert_eq!(unescape_mount("/media/a\\134b"), OsString::from("/media/a\\b"));
    }

    #[test]
    fn mounts_from_drops_nested_mounts() {
        let text = "\
none /mnt/wsl tmpfs rw,relatime 0 0
none /mnt/wslg tmpfs rw,relatime 0 0
/dev/sdc /mnt/wslg/distro ext4 ro,relatime 0 0
none /mnt/wslg/versions.txt overlay rw,relatime 0 0
C:\\134 /mnt/c 9p rw,noatime 0 0
";
        assert_eq!(mounts_from(text), [PathBuf::from("/mnt/c"), PathBuf::from("/mnt/wsl"), PathBuf::from("/mnt/wslg")]);
    }

    #[test]
    fn drive_label_of_root_and_mounts() {
        assert_eq!(drive_label("/"), ROOT_LABEL);
        assert_eq!(drive_label("/media/usb"), "usb");
    }
}

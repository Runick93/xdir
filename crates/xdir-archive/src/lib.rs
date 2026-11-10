//! Zip y tar.gz para las tres plataformas, con las reglas de seguridad de `xdir_core::archive`.
//!
//! Los tipos de error y de avance (`Flow`, `ErrKind`, `Error`, `ItemKind`) tienen la misma forma
//! que los de las primitivas de copia del motor (Task 6); la Task 15 los reemplaza por esos y
//! enchufa `Service` detrás de `sys::Archives`.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use xdir_core::archive::{ArchiveFormat, format_of, safe_entry_path, symlink_inside};

mod targz;
mod zip;

#[cfg(test)]
mod tests;

/// Qué hacer después de avisar el avance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    Continue,
    Stop,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrKind {
    InUse,
    Denied,
    NotFound,
    DiskFull,
    TooBig,
    Exists,
    Cancelled,
    Other(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    pub kind: ErrKind,
    pub path: PathBuf,
}

impl Error {
    pub fn other(msg: &str, path: &Path) -> Error {
        Error { kind: ErrKind::Other(msg.to_string()), path: path.to_path_buf() }
    }

    pub fn message(&self) -> String {
        match &self.kind {
            ErrKind::InUse => "Lo está usando otro programa".into(),
            ErrKind::Denied => "No tenés permiso".into(),
            ErrKind::NotFound => "Ya no existe".into(),
            ErrKind::DiskFull => "El disco está lleno".into(),
            ErrKind::TooBig => "Es demasiado grande para el formato del disco de destino (FAT32)".into(),
            ErrKind::Exists => "Ya existe".into(),
            ErrKind::Cancelled => "Cancelado".into(),
            ErrKind::Other(s) => s.clone(),
        }
    }
}

/// Mismo mapeo que las primitivas de copia.
pub fn kind_of(e: &io::Error) -> ErrKind {
    match e.kind() {
        io::ErrorKind::PermissionDenied => ErrKind::Denied,
        io::ErrorKind::NotFound => ErrKind::NotFound,
        io::ErrorKind::AlreadyExists => ErrKind::Exists,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => ErrKind::DiskFull,
        io::ErrorKind::FileTooLarge => ErrKind::TooBig,
        io::ErrorKind::ResourceBusy => ErrKind::InUse,
        _ => match e.raw_os_error() {
            #[cfg(windows)]
            Some(32) | Some(33) => ErrKind::InUse,
            #[cfg(windows)]
            Some(112) => ErrKind::DiskFull,
            _ => ErrKind::Other(e.to_string()),
        },
    }
}

pub(crate) fn io_err(e: io::Error, path: &Path) -> Error {
    // La crate `zip` esconde sus errores dentro de un `io::Error` al leer los datos.
    if let Some(z) = e.get_ref().and_then(|i| i.downcast_ref::<::zip::result::ZipError>()) {
        return zip::map_zip_ref(z, path);
    }
    Error { kind: kind_of(&e), path: path.to_path_buf() }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    File,
    Dir,
    Symlink,
}

/// Una entrada del archivo, tal como figura adentro (sin sanear: `extract` la valida).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArchiveEntry {
    pub index: usize,
    pub name: String,
    pub size: u64,
    pub kind: ItemKind,
    /// Destino del enlace simbólico.
    pub link: Option<String>,
    /// Segundos desde 1970 (0 si no figura).
    pub modified: u64,
}

pub struct Service {
    /// Windows: `false` (solo zip).
    pub tar_gz: bool,
}

impl Service {
    pub fn formats(&self) -> &'static [ArchiveFormat] {
        if self.tar_gz { &[ArchiveFormat::Zip, ArchiveFormat::TarGz] } else { &[ArchiveFormat::Zip] }
    }

    fn format(&self, p: &Path) -> Result<ArchiveFormat, Error> {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match format_of(&name) {
            Some(f) if self.formats().contains(&f) => Ok(f),
            _ => Err(Error::other("Formato no soportado", p)),
        }
    }

    pub fn list(&self, archive: &Path) -> Result<Vec<ArchiveEntry>, Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::list(archive),
            ArchiveFormat::TarGz => targz::list(archive),
        }
    }

    /// Extrae `entry` en `dst` (la ruta final de esa entrada, que el llamador arma con la
    /// carpeta destino y `safe_entry_path(entry.name)`). Crea las carpetas que falten; nunca
    /// pisa (falla con `Exists`); lo escrito a medias se borra. `progress` recibe los bytes
    /// escritos de esta entrada.
    pub fn extract(
        &self,
        archive: &Path,
        entry: &ArchiveEntry,
        dst: &Path,
        progress: &mut dyn FnMut(u64) -> Flow,
    ) -> Result<(), Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::extract(archive, entry, dst, progress),
            ArchiveFormat::TarGz => targz::extract(archive, entry, dst, progress),
        }
    }

    /// Crea un zip en `out` (con `create_new`). `items` = (archivo en disco, nombre adentro con
    /// `/`; las carpetas, con nombre terminado en `/`). `progress` recibe los bytes leídos en
    /// total; `Stop` borra `out`.
    pub fn create(
        &self,
        out: &Path,
        items: &[(PathBuf, String)],
        progress: &mut dyn FnMut(u64) -> Flow,
    ) -> Result<(), Error> {
        if self.format(out)? != ArchiveFormat::Zip {
            return Err(Error::other("Formato no soportado", out));
        }
        zip::create(out, items, progress)
    }
}

/// Lo común de extraer una entrada: validaciones y escritura de carpetas, enlaces y archivos.
pub(crate) struct Source<'a> {
    pub kind: ItemKind,
    pub name: &'a str,
    pub link: Option<&'a str>,
    pub declared: u64,
    pub mode: Option<u32>,
    pub reader: &'a mut dyn Read,
}

pub(crate) fn extract_to(src: Source, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), Error> {
    let rel = safe_entry_path(src.name).ok_or_else(|| Error::other("Ruta no permitida dentro del archivo", dst))?;
    match src.kind {
        ItemKind::Dir => {
            std::fs::create_dir_all(dst).map_err(|e| io_err(e, dst))
        }
        ItemKind::Symlink => {
            let target = src.link.unwrap_or("");
            if !symlink_inside(&rel, target) {
                return Err(Error::other("Enlace que apunta fuera de la carpeta", dst));
            }
            make_symlink(target, dst, &rel)
        }
        ItemKind::File => {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_err(e, parent))?;
            }
            let mut f = OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| io_err(e, dst))?;
            let res = copy_limited(src.reader, &mut f, src.declared, progress, dst);
            drop(f);
            if let Err(e) = res {
                let _ = std::fs::remove_file(dst);
                return Err(e);
            }
            #[cfg(unix)]
            if let Some(m) = src.mode {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode((m & 0o777) | 0o600));
            }
            #[cfg(not(unix))]
            let _ = src.mode;
            Ok(())
        }
    }
}

/// Copia por bloques; corta si se escribe más de lo declarado (bomba o archivo dañado).
fn copy_limited(
    r: &mut dyn Read,
    w: &mut dyn Write,
    declared: u64,
    progress: &mut dyn FnMut(u64) -> Flow,
    path: &Path,
) -> Result<(), Error> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = r.read(&mut buf).map_err(|e| io_err(e, path))?;
        if n == 0 {
            return Ok(());
        }
        done += n as u64;
        if done > declared {
            return Err(Error::other("El archivo está dañado", path));
        }
        w.write_all(&buf[..n]).map_err(|e| io_err(e, path))?;
        if progress(done) == Flow::Stop {
            return Err(Error { kind: ErrKind::Cancelled, path: path.to_path_buf() });
        }
    }
}

#[cfg(unix)]
fn make_symlink(target: &str, dst: &Path, rel: &Path) -> Result<(), Error> {
    let parent = dst.parent().unwrap_or(dst);
    std::fs::create_dir_all(parent).map_err(|e| io_err(e, parent))?;
    // La raíz de la extracción es `dst` sin los componentes de la entrada.
    let mut root = dst.to_path_buf();
    for _ in rel.components() {
        root.pop();
    }
    if !physically_inside(&root, parent, target) {
        return Err(Error::other("Enlace que apunta fuera de la carpeta", dst));
    }
    std::os::unix::fs::symlink(target, dst).map_err(|e| io_err(e, dst))
}

/// En Windows los enlaces simbólicos piden privilegios: se omiten.
#[cfg(not(unix))]
fn make_symlink(_target: &str, _dst: &Path, _rel: &Path) -> Result<(), Error> {
    Ok(())
}

/// Resuelve `target` desde `parent` siguiendo los enlaces que ya existen en el disco (el texto
/// puede estar dentro y el camino real afuera, p. ej. `d/l -> ..` y luego `x -> d/l/..`) y
/// comprueba que el resultado siga bajo `root`.
#[cfg(unix)]
fn physically_inside(root: &Path, parent: &Path, target: &str) -> bool {
    let (Ok(root), Ok(mut cur)) = (root.canonicalize(), parent.canonicalize()) else {
        return false;
    };
    for c in target.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if !cur.pop() || !cur.starts_with(&root) {
                    return false;
                }
            }
            c => {
                cur.push(c);
                if let Ok(real) = cur.canonicalize() {
                    cur = real;
                }
            }
        }
    }
    cur.starts_with(&root)
}

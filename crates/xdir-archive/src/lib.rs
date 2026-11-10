//! Zip y tar.gz para las tres plataformas, con las reglas de seguridad de `xdir_core::archive`.
//!
//! Los tipos de error y de avance (`Flow`, `ErrKind`, `Error`, `ItemKind`) tienen la misma forma
//! que los de las primitivas de copia del motor (Task 6); la Task 15 los reemplaza por esos y
//! enchufa `Service` detrás de `sys::Archives`.

use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use xdir_core::archive::{ArchiveFormat, format_of, is_root_entry, safe_entry_path, symlink_inside};

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

/// Marca de una lectura cortada porque el llamador pidió cancelar.
#[derive(Debug)]
pub(crate) struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelado")
    }
}
impl std::error::Error for Cancelled {}

pub(crate) fn cancelled(path: &Path) -> Error {
    Error { kind: ErrKind::Cancelled, path: path.to_path_buf() }
}

/// Lector que consulta `check` antes de cada lectura (descomprimir un tar.gz entero lleva tiempo).
pub(crate) struct CancelRead<'a, R> {
    pub inner: R,
    pub check: &'a mut dyn FnMut() -> Flow,
}

impl<R: Read> Read for CancelRead<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if (self.check)() == Flow::Stop {
            return Err(io::Error::other(Cancelled));
        }
        self.inner.read(buf)
    }
}

pub(crate) fn io_err(e: io::Error, path: &Path) -> Error {
    if e.get_ref().is_some_and(|i| i.is::<Cancelled>()) {
        return cancelled(path);
    }
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
    /// Enlace duro, dispositivo o FIFO: se lista pero no se extrae (se cuenta como omitido).
    Omitted,
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

const NOT_ALLOWED: &str = "Ruta no permitida dentro del archivo";
/// Los enlaces guardan su destino como datos: más largo que esto se rechaza.
pub(crate) const MAX_LINK: u64 = 4096;

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
        self.list_cancellable(archive, &mut || Flow::Continue)
    }

    /// Como `list`, pero consulta `cancel` mientras lee (un tar.gz grande se descomprime entero
    /// para listarlo); `Stop` da `ErrKind::Cancelled`.
    pub fn list_cancellable(&self, archive: &Path, cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::list(archive),
            ArchiveFormat::TarGz => targz::list(archive, cancel),
        }
    }

    /// Bytes que declara el archivo para sus archivos (lo que ocupará al extraerlo). El llamador
    /// lo compara con el espacio libre del destino antes de extraer: lo declarado no se confía
    /// (por eso `extract` corta si una entrada escribe más de lo que declara), pero sirve para
    /// avisar «No hay lugar» antes de empezar.
    pub fn declared_total(&self, archive: &Path) -> Result<u64, Error> {
        Ok(self.list(archive)?.iter().filter(|e| e.kind == ItemKind::File).map(|e| e.size).sum())
    }

    /// Extrae `entry` bajo `root` (que ya existe). La ruta final sale del nombre guardado en el
    /// archivo (`safe_entry_path`); `override_name` cambia solo el último componente (para
    /// «Conservar ambos»). Nunca recorre un enlace simbólico del disco, crea las carpetas de a
    /// una, no pisa (falla con `Exists`) y borra lo escrito a medias. `progress` recibe los bytes
    /// escritos de esta entrada. Quien extrae entrada por entrada debe dejar los enlaces
    /// simbólicos para el final (o usar `extract_all`).
    pub fn extract(
        &self,
        archive: &Path,
        entry: &ArchiveEntry,
        root: &Path,
        override_name: Option<&OsStr>,
        progress: &mut dyn FnMut(u64) -> Flow,
    ) -> Result<(), Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::extract(archive, entry, root, override_name, progress),
            ArchiveFormat::TarGz => targz::extract(archive, entry, root, override_name, progress),
        }
    }

    /// Extrae todo bajo `root`: primero carpetas y archivos y al final los enlaces simbólicos
    /// (así ninguno puede redirigir lo que se escribe). Un tar.gz se recorre una sola vez.
    /// `progress` recibe los bytes totales escritos y se consulta también entre entradas y
    /// mientras se descomprime. Devuelve cuántos enlaces se omitieron (en Windows todos). Si se
    /// corta o falla, lo ya extraído queda: lo limpia quien llama.
    pub fn extract_all(&self, archive: &Path, root: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<usize, Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::extract_all(archive, root, progress),
            ArchiveFormat::TarGz => targz::extract_all(archive, root, progress),
        }
    }

    /// Crea un zip en `out` (con `create_new`). `items` = (archivo en disco, nombre adentro con
    /// `/`; las carpetas, con nombre terminado en `/`). `progress` recibe los bytes leídos en
    /// total, también después de cada elemento; `Stop` borra `out`.
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

/// Ruta relativa final de una entrada: la segura del nombre, con el último componente cambiado
/// si hay `override_name`. Todos los componentes son normales. `None` si es la propia carpeta de
/// extracción y la entrada es una carpeta (no hay nada que hacer). Los errores llevan el nombre
/// de la entrada.
pub(crate) fn resolve_rel(name: &str, override_name: Option<&OsStr>, is_dir: bool) -> Result<Option<PathBuf>, Error> {
    let at = Path::new(name);
    let bad = || Error::other(NOT_ALLOWED, at);
    if is_dir && is_root_entry(name) {
        return Ok(None);
    }
    let mut rel = safe_entry_path(name).ok_or_else(bad)?;
    if let Some(o) = override_name {
        let o = o.to_str().ok_or_else(bad)?;
        let one = safe_entry_path(o).ok_or_else(bad)?;
        if one.components().count() != 1 || o.contains(['/', '\\']) {
            return Err(bad());
        }
        rel.set_file_name(one.as_os_str());
    }
    if !rel.is_relative() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(bad());
    }
    Ok(Some(rel))
}

/// Lo común de extraer una entrada.
pub(crate) struct Source<'a> {
    pub kind: ItemKind,
    pub rel: &'a Path,
    pub link: Option<&'a str>,
    pub declared: u64,
    pub mode: Option<u32>,
    pub reader: &'a mut dyn Read,
}

/// Baja de `root` hacia `rel` componente por componente sin pasar por enlaces simbólicos (ni
/// los que ya había en el disco), creando las carpetas de a una. Devuelve la ruta final (el
/// último componente no se toca) y no sale nunca de `root`.
fn walk_down(root: &Path, rel: &Path) -> Result<PathBuf, Error> {
    let mut cur = root.to_path_buf();
    let comps: Vec<_> = rel.components().collect();
    let Some((last, parents)) = comps.split_last() else {
        return Err(Error::other(NOT_ALLOWED, root));
    };
    for c in parents {
        cur.push(c);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.is_dir() => {}
            Ok(m) if m.file_type().is_symlink() => return Err(Error::other(NOT_ALLOWED, &cur)),
            Ok(_) => return Err(Error { kind: ErrKind::Exists, path: cur }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur).map_err(|e| io_err(e, &cur))?;
            }
            Err(e) => return Err(io_err(e, &cur)),
        }
    }
    cur.push(last);
    if !cur.starts_with(root) {
        return Err(Error::other(NOT_ALLOWED, &cur));
    }
    Ok(cur)
}

/// Escribe una entrada bajo `root`; devuelve los bytes escritos.
pub(crate) fn write_entry(src: Source, root: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<u64, Error> {
    let dst = walk_down(root, src.rel)?;
    match src.kind {
        ItemKind::Dir => {
            match std::fs::symlink_metadata(&dst) {
                Ok(m) if m.is_dir() => {}
                Ok(m) if m.file_type().is_symlink() => return Err(Error::other(NOT_ALLOWED, &dst)),
                Ok(_) => return Err(Error { kind: ErrKind::Exists, path: dst }),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    std::fs::create_dir(&dst).map_err(|e| io_err(e, &dst))?;
                }
                Err(e) => return Err(io_err(e, &dst)),
            }
            Ok(0)
        }
        ItemKind::Omitted => Err(Error::other("Tipo de elemento no soportado", &dst)),
        ItemKind::Symlink => {
            let Some(target) = src.link else {
                return Err(Error::other("Enlace no válido", &dst));
            };
            if !symlink_inside(src.rel, target) {
                return Err(Error::other("Enlace que apunta fuera de la carpeta", &dst));
            }
            make_symlink(target, root, &dst)?;
            Ok(0)
        }
        ItemKind::File => {
            let mut f = OpenOptions::new().write(true).create_new(true).open(&dst).map_err(|e| io_err(e, &dst))?;
            let res = copy_limited(src.reader, &mut f, src.declared, progress, &dst);
            drop(f);
            let n = match res {
                Ok(n) => n,
                Err(e) => {
                    let _ = std::fs::remove_file(&dst);
                    return Err(e);
                }
            };
            #[cfg(unix)]
            if let Some(m) = src.mode {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode((m & 0o777) | 0o600));
            }
            #[cfg(not(unix))]
            let _ = src.mode;
            Ok(n)
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
) -> Result<u64, Error> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = r.read(&mut buf).map_err(|e| io_err(e, path))?;
        if n == 0 {
            return Ok(done);
        }
        done += n as u64;
        if done > declared {
            return Err(Error::other("El archivo está dañado", path));
        }
        w.write_all(&buf[..n]).map_err(|e| io_err(e, path))?;
        if progress(done) == Flow::Stop {
            return Err(cancelled(path));
        }
    }
}

#[cfg(unix)]
fn make_symlink(target: &str, root: &Path, dst: &Path) -> Result<(), Error> {
    // El texto puede quedar adentro y el camino real afuera: se resuelve con lo que hay.
    let parent = dst.parent().unwrap_or(dst);
    if !physically_inside(root, parent, target) {
        return Err(Error::other("Enlace que apunta fuera de la carpeta", dst));
    }
    std::os::unix::fs::symlink(target, dst).map_err(|e| io_err(e, dst))
}

/// En Windows los enlaces simbólicos piden privilegios: no se crean.
#[cfg(not(unix))]
fn make_symlink(_target: &str, _root: &Path, dst: &Path) -> Result<(), Error> {
    Err(Error::other("Enlace omitido", dst))
}

/// Resuelve `target` desde `parent` siguiendo los enlaces que ya existen y comprueba que el
/// resultado siga bajo `root`. Un `..` después de un componente que todavía no existe no se
/// puede resolver (podría ser un enlace que se cree después): se rechaza.
#[cfg(unix)]
fn physically_inside(root: &Path, parent: &Path, target: &str) -> bool {
    let (Ok(root), Ok(mut cur)) = (root.canonicalize(), parent.canonicalize()) else {
        return false;
    };
    let mut unresolved = false;
    for c in target.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if unresolved || !cur.pop() || !cur.starts_with(&root) {
                    return false;
                }
            }
            c => {
                cur.push(c);
                match cur.canonicalize() {
                    Ok(real) => cur = real,
                    Err(_) => unresolved = true,
                }
            }
        }
    }
    cur.starts_with(&root)
}

/// Nombre de una entrada al crear: sin `\`, `.` ni `..`, normalizado y con `/`. A propósito no
/// usa `safe_entry_path`: en macOS y Linux `:` y los nombres como «con» son legales; se guardan
/// tal cual y se rechazan recién al extraer (en cualquier sistema).
pub(crate) fn stored_name(name: &str, at: &Path) -> Result<String, Error> {
    let bad = || Error::other("Nombre no permitido dentro del zip", at);
    if name.contains(['\\', '\0']) || name.starts_with('/') {
        return Err(bad());
    }
    let mut parts = Vec::new();
    for c in name.split('/') {
        match c {
            "" => {}
            "." | ".." => return Err(bad()),
            c => parts.push(c),
        }
    }
    if parts.is_empty() {
        return Err(bad());
    }
    let mut s = parts.join("/");
    if name.ends_with('/') {
        s.push('/');
    }
    Ok(s)
}

/// Comparte el avance entre la lectura (que consulta si cancelar) y la escritura.
pub(crate) struct Shared<'a> {
    pub progress: RefCell<&'a mut dyn FnMut(u64) -> Flow>,
    pub total: Cell<u64>,
}

impl<'a> Shared<'a> {
    pub fn new(p: &'a mut dyn FnMut(u64) -> Flow) -> Shared<'a> {
        Shared { progress: RefCell::new(p), total: Cell::new(0) }
    }
    /// Bytes de la entrada actual sobre lo ya terminado.
    pub fn report(&self, base: u64, entry_bytes: u64) -> Flow {
        self.total.set(base + entry_bytes);
        (self.progress.borrow_mut())(base + entry_bytes)
    }
    pub fn check(&self) -> Flow {
        (self.progress.borrow_mut())(self.total.get())
    }
}

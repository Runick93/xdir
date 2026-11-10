//! Zip y tar.gz para las tres plataformas, con las reglas de seguridad de `xdir_core::archive`.
//!
//! Los tipos de error y de avance son los de las primitivas de copia del motor (`Error` es
//! `xdir_core::sys::CopyError`) y `Service` se instala detrás de `sys::Archives`.

use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use xdir_core::sys::Archives;
use xdir_core::archive::{ArchiveFormat, format_of, is_root_entry, safe_entry_path, symlink_inside};

mod targz;
mod zip;

#[cfg(test)]
mod tests;

pub use xdir_core::archive::{ArchiveEntry, EntryKind as ItemKind};
pub use xdir_core::sys::{CopyError as Error, ErrKind, Flow};

/// Un error con su texto.
pub(crate) fn other(msg: &str, path: &Path) -> Error {
    Error::new(ErrKind::Other(msg.to_string()), path)
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

/// Qué hacer con una entrada que no se pudo extraer (nombre, error): `Err` corta todo.
pub(crate) type OnFail<'a> = &'a mut dyn FnMut(&str, Error) -> Result<(), Error>;

/// Errores que cortan la extracción entera (seguir no tendría sentido o no se debe).
fn fatal(e: &Error) -> bool {
    matches!(&e.kind, ErrKind::Cancelled | ErrKind::DiskFull) || matches!(&e.kind, ErrKind::Other(m) if m == xdir_core::sys::PASSWORD)
}

pub struct Service {
    /// Windows: `false` (solo zip).
    pub tar_gz: bool,
    /// Diferencia de la hora local con UTC (segundos, -10800 es UTC-3) en el instante `unix`
    /// (segundos desde 1970, UTC), según la zona del sistema. Las horas de un zip (DOS) son
    /// locales: con esto se escriben y se leen. La da cada plataforma.
    pub utc_offset: fn(i64) -> i64,
}

use xdir_core::sys::NOT_ALLOWED;
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
            _ => Err(other(xdir_core::sys::UNSUPPORTED, p)),
        }
    }

    pub fn list(&self, archive: &Path) -> Result<Vec<ArchiveEntry>, Error> {
        self.list_cancellable(archive, &mut || Flow::Continue)
    }

    /// Como `list`, pero consulta `cancel` mientras lee (un tar.gz grande se descomprime entero
    /// para listarlo); `Stop` da `ErrKind::Cancelled`.
    pub fn list_cancellable(&self, archive: &Path, cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::list(archive, self.utc_offset),
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
        self.extract_each(archive, root, &|_| true, progress, &mut |_, e| Err(e))
    }

    /// Como `extract_all`, pero cada entrada que falla va a `on_fail`, que decide si se sigue.
    fn extract_each(
        &self,
        archive: &Path,
        root: &Path,
        want: &dyn Fn(&str) -> bool,
        progress: &mut dyn FnMut(u64) -> Flow,
        on_fail: OnFail,
    ) -> Result<usize, Error> {
        match self.format(archive)? {
            ArchiveFormat::Zip => zip::extract_all(archive, root, want, progress, on_fail),
            ArchiveFormat::TarGz => targz::extract_all(archive, root, want, progress, on_fail),
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
            return Err(other(xdir_core::sys::UNSUPPORTED, out));
        }
        zip::create(out, items, progress, self.utc_offset)
    }
}

/// Ruta relativa final de una entrada: la segura del nombre, con el último componente cambiado
/// si hay `override_name`. Todos los componentes son normales. `None` si es la propia carpeta de
/// extracción y la entrada es una carpeta (no hay nada que hacer). Los errores llevan el nombre
/// de la entrada.
pub(crate) fn resolve_rel(name: &str, override_name: Option<&OsStr>, is_dir: bool) -> Result<Option<PathBuf>, Error> {
    let at = Path::new(name);
    let bad = || other(NOT_ALLOWED, at);
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
        return Err(other(NOT_ALLOWED, root));
    };
    for c in parents {
        cur.push(c);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.is_dir() => {}
            Ok(m) if m.file_type().is_symlink() => return Err(other(NOT_ALLOWED, &cur)),
            Ok(_) => return Err(Error { kind: ErrKind::Exists, path: cur }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&cur).map_err(|e| io_err(e, &cur))?;
            }
            Err(e) => return Err(io_err(e, &cur)),
        }
    }
    cur.push(last);
    if !cur.starts_with(root) {
        return Err(other(NOT_ALLOWED, &cur));
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
                Ok(m) if m.file_type().is_symlink() => return Err(other(NOT_ALLOWED, &dst)),
                Ok(_) => return Err(Error { kind: ErrKind::Exists, path: dst }),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    std::fs::create_dir(&dst).map_err(|e| io_err(e, &dst))?;
                }
                Err(e) => return Err(io_err(e, &dst)),
            }
            Ok(0)
        }
        ItemKind::Omitted => Err(other("Tipo de elemento no soportado", &dst)),
        ItemKind::Symlink => {
            let Some(target) = src.link else {
                return Err(other("Enlace no válido", &dst));
            };
            if !symlink_inside(src.rel, target) {
                return Err(other(NOT_ALLOWED, &dst));
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
            return Err(other("El archivo está dañado", path));
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
        return Err(other(NOT_ALLOWED, dst));
    }
    std::os::unix::fs::symlink(target, dst).map_err(|e| io_err(e, dst))
}

/// ¿El enlace `link` (ya creado) resuelve dentro de `root`, siguiendo lo que hay en el disco?
#[cfg(unix)]
fn link_inside(root: &Path, link: &Path) -> bool {
    let Ok(target) = std::fs::read_link(link) else { return true };
    let target = target.to_string_lossy();
    let parent = link.parent().unwrap_or(link);
    !target.starts_with('/') && physically_inside(root, parent, &target)
}

/// En Windows no se crean enlaces al extraer.
#[cfg(not(unix))]
fn link_inside(_root: &Path, _link: &Path) -> bool {
    true
}

/// En Windows los enlaces simbólicos piden privilegios: no se crean.
#[cfg(not(unix))]
fn make_symlink(_target: &str, _root: &Path, dst: &Path) -> Result<(), Error> {
    Err(other("Enlace omitido", dst))
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
                    // Un enlace que todavía no lleva a nada: podría llevar afuera mañana.
                    Err(_) if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) => return false,
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
    let bad = || other("Nombre no permitido dentro del zip", at);
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

impl Archives for Service {
    fn formats(&self) -> &'static [ArchiveFormat] {
        Service::formats(self)
    }

    fn list(&self, archive: &Path, cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, Error> {
        self.list_cancellable(archive, cancel)
    }

    fn extract_all(
        &self,
        archive: &Path,
        root: &Path,
        progress: &mut dyn FnMut(u64) -> Flow,
        failed: &mut dyn FnMut(&str, Error),
    ) -> Result<usize, Error> {
        self.extract_some(archive, root, &|_| true, progress, failed)
    }

    fn extract_some(
        &self,
        archive: &Path,
        root: &Path,
        want: &dyn Fn(&str) -> bool,
        progress: &mut dyn FnMut(u64) -> Flow,
        failed: &mut dyn FnMut(&str, Error),
    ) -> Result<usize, Error> {
        self.extract_each(archive, root, want, progress, &mut |name, e| {
            if fatal(&e) {
                return Err(e);
            }
            failed(name, e);
            Ok(())
        })
    }

    fn links_outside(&self, root: &Path, links: &[PathBuf]) -> Vec<PathBuf> {
        links.iter().filter(|l| !link_inside(root, l)).cloned().collect()
    }

    /// Siempre zip (el nombre puede ser un parcial del motor, sin `.zip`).
    fn create(&self, out: &Path, items: &[(PathBuf, String)], progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), Error> {
        zip::create(out, items, progress, self.utc_offset)
    }
}

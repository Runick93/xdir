//! Primitivas de copia de macOS (`xdir_core::sys::CopyPrims`) para el motor de copia del núcleo:
//! `copyfile` por archivo (clona en APFS, conserva atributos extendidos, etiquetas y ACL) con
//! aviso de progreso por bloque, `renamex_np(RENAME_EXCL)` para renombrar sin pisar y la
//! Papelera de `NSFileManager`. Lo que está solo en la nube (`SF_DATALESS`) se informa sin leerlo.

use std::ffi::{CString, OsString, c_char, c_int, c_void};
use std::io;
use std::os::macos::fs::MetadataExt as MacMetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use xdir_core::sys::{CopyError, CopyPrims, ErrKind, Flow, ItemKind, ItemStat, VolumeInfo};

/// Todo: datos, permisos, fechas, ACL y atributos extendidos.
const COPYFILE_ALL: u32 = libc::COPYFILE_METADATA | libc::COPYFILE_DATA;
/// `SF_DATALESS` de `chflags`: solo en la nube.
const SF_DATALESS: u32 = 0x4000_0000;
/// Segundos entre 1601 y 1970 (para pasar fechas a FILETIME).
const EPOCH_1601: i64 = 11_644_473_600;

pub struct MacCopyPrims;

/// Código de error del sistema → `ErrKind`.
pub(crate) fn err_kind(errno: i32) -> ErrKind {
    match errno {
        libc::EBUSY | libc::ETXTBSY => ErrKind::InUse,
        libc::EACCES | libc::EPERM => ErrKind::Denied,
        libc::ENOENT => ErrKind::NotFound,
        libc::ENOSPC | libc::EDQUOT => ErrKind::DiskFull,
        libc::EFBIG => ErrKind::TooBig,
        libc::EEXIST => ErrKind::Exists,
        libc::ECANCELED => ErrKind::Cancelled,
        libc::EXDEV => ErrKind::CrossDevice,
        n => ErrKind::Other(io::Error::from_raw_os_error(n).to_string()),
    }
}

fn err(e: io::Error, p: &Path) -> CopyError {
    let kind = match e.raw_os_error() {
        Some(n) => err_kind(n),
        None => match e.kind() {
            io::ErrorKind::NotFound => ErrKind::NotFound,
            io::ErrorKind::AlreadyExists => ErrKind::Exists,
            io::ErrorKind::PermissionDenied => ErrKind::Denied,
            _ => ErrKind::Other(e.to_string()),
        },
    };
    CopyError::new(kind, p)
}

fn last(p: &Path) -> CopyError {
    err(io::Error::last_os_error(), p)
}

fn c_path(p: &Path) -> Result<CString, CopyError> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| CopyError::new(ErrKind::Other("Ruta inválida".into()), p))
}

fn filetime(secs: i64, nanos: i64) -> u64 {
    ((secs + EPOCH_1601).max(0) as u64) * 10_000_000 + (nanos.max(0) as u64) / 100
}

/// Lo que recibe el aviso de `copyfile`.
struct Ctx<'a> {
    progress: &'a mut dyn FnMut(u64) -> Flow,
    stopped: bool,
}

extern "C" fn status(what: c_int, stage: c_int, state: libc::copyfile_state_t, _src: *const c_char, _dst: *const c_char, ctx: *mut c_void) -> c_int {
    if what == libc::COPYFILE_COPY_DATA && stage == libc::COPYFILE_PROGRESS {
        let mut copied: libc::off_t = 0;
        unsafe { libc::copyfile_state_get(state, libc::COPYFILE_STATE_COPIED as u32, (&mut copied as *mut libc::off_t).cast()) };
        // SAFETY: `ctx` es el `Ctx` de `copy_file`, vivo mientras dura `copyfile`.
        let ctx = unsafe { &mut *(ctx as *mut Ctx) };
        // Un pánico no puede cruzar a C: si pasa, se corta la copia.
        let flow = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (ctx.progress)(copied.max(0) as u64))).unwrap_or(Flow::Stop);
        if flow == Flow::Stop {
            ctx.stopped = true;
            return libc::COPYFILE_QUIT;
        }
    }
    libc::COPYFILE_CONTINUE
}

/// ¿Es renombrar solo las mayúsculas del mismo elemento? (en un disco que no las distingue). Los
/// nombres tienen que ser iguales salvo mayúsculas: dos enlaces duros con otro nombre son el mismo
/// archivo, pero renombrar uno sobre el otro borraría un nombre.
fn case_only_rename(a: &Path, b: &Path) -> bool {
    let same_name = a.parent() == b.parent()
        && match (a.file_name(), b.file_name()) {
            (Some(x), Some(y)) => x.to_string_lossy().to_lowercase() == y.to_string_lossy().to_lowercase(),
            _ => false,
        };
    same_name
        && match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
}

/// `copyfile` de un archivo con aviso de progreso (`clone`: `COPYFILE_CLONE`, o 0 para copiar
/// los datos siempre, lo que usan las pruebas para ver los avisos en APFS).
fn copy_data(src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow, clone: u32) -> Result<(), CopyError> {
    let (from, to) = (c_path(src)?, c_path(dst)?);
    let meta = std::fs::symlink_metadata(src).map_err(|e| err(e, src))?;
    if !meta.is_file() {
        return Err(CopyError::new(ErrKind::Other("No es un archivo".into()), src));
    }
    let mut ctx = Ctx { progress, stopped: false };
    let state = unsafe { libc::copyfile_state_alloc() };
    if state.is_null() {
        return Err(last(src));
    }
    let r = unsafe {
        // El valor de `COPYFILE_STATE_STATUS_CB` es la función misma (no un puntero a ella).
        let cb: extern "C" fn(c_int, c_int, libc::copyfile_state_t, *const c_char, *const c_char, *mut c_void) -> c_int = status;
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CB as u32, cb as *const c_void);
        libc::copyfile_state_set(state, libc::COPYFILE_STATE_STATUS_CTX as u32, (&mut ctx as *mut Ctx).cast());
        let flags = COPYFILE_ALL | clone | libc::COPYFILE_EXCL | libc::COPYFILE_NOFOLLOW_SRC;
        libc::copyfile(from.as_ptr(), to.as_ptr(), state, flags)
    };
    let errno = io::Error::last_os_error();
    unsafe { libc::copyfile_state_free(state) };
    if r == 0 && !ctx.stopped {
        return Ok(());
    }
    let e = if ctx.stopped { CopyError::new(ErrKind::Cancelled, dst) } else { err(errno, dst) };
    // Con `COPYFILE_EXCL` el destino lo creó esta copia (salvo que ya existiera): lo a medio
    // copiar se borra.
    if e.kind != ErrKind::Exists {
        let _ = std::fs::remove_file(dst);
    }
    Err(e)
}

impl CopyPrims for MacCopyPrims {
    fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
        let m = std::fs::symlink_metadata(p).map_err(|e| err(e, p))?;
        let kind = if m.file_type().is_symlink() {
            ItemKind::Symlink
        } else if m.is_dir() {
            ItemKind::Dir
        } else {
            ItemKind::File
        };
        let size = if kind == ItemKind::Dir { 0 } else { m.len() };
        let cloud_only = kind == ItemKind::File && m.st_flags() & SF_DATALESS != 0;
        Ok(ItemStat { kind, size, modified: filetime(m.mtime(), m.mtime_nsec()), cloud_only, file_id: Some((m.dev(), m.ino())) })
    }

    fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
        let rd = std::fs::read_dir(p).map_err(|e| err(e, p))?;
        rd.map(|e| e.map(|e| e.file_name()).map_err(|e| err(e, p))).collect()
    }

    fn volume(&self, p: &Path) -> Option<VolumeInfo> {
        let m = std::fs::metadata(p).ok()?;
        let cp = c_path(p).ok()?;
        let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(cp.as_ptr(), &mut fs) } != 0 {
            return None;
        }
        let cstr = |b: &[c_char]| {
            let bytes: Vec<u8> = b.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let fstype = cstr(&fs.f_fstypename);
        let mount = cstr(&fs.f_mntonname);
        let flags = fs.f_flags as i64;
        let read_only = flags & libc::MNT_RDONLY as i64 != 0;
        let local = flags & libc::MNT_LOCAL as i64 != 0;
        // 1: distingue; 0: no; -1 (no se sabe): se trata como que no distingue (más conflictos,
        // nunca menos).
        let sensitive = unsafe { libc::pathconf(cp.as_ptr(), libc::_PC_CASE_SENSITIVE) } == 1;
        Some(VolumeInfo {
            id: OsString::from(m.dev().to_string()),
            label: crate::fs::macos::drive_label(&mount),
            case_insensitive: !sensitive,
            has_trash: !read_only && local,
            max_file: (fstype == "msdos").then_some((4u64 << 30) - 1),
            free: fs.f_bavail * fs.f_bsize as u64,
        })
    }

    fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        copy_data(src, dst, progress, libc::COPYFILE_CLONE)
    }

    fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
        let (from, to) = (c_path(src)?, c_path(dst)?);
        let flags = COPYFILE_ALL | libc::COPYFILE_NOFOLLOW | libc::COPYFILE_EXCL;
        if unsafe { libc::copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), flags) } != 0 {
            return Err(last(dst));
        }
        Ok(())
    }

    fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
        std::fs::create_dir(dst).map_err(|e| err(e, dst))
    }

    fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
        let (from, to) = (c_path(like)?, c_path(dst)?);
        if unsafe { libc::copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), libc::COPYFILE_METADATA) } != 0 {
            return Err(last(dst));
        }
        Ok(())
    }

    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        let (f, t) = (c_path(from)?, c_path(to)?);
        if unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // Solo cambian las mayúsculas en un disco que no las distingue: es el mismo.
            Some(libc::EEXIST) if case_only_rename(from, to) => std::fs::rename(from, to).map_err(|e| err(e, from)),
            Some(libc::EEXIST) => Err(CopyError::new(ErrKind::Exists, to)),
            Some(libc::ENOTSUP) => {
                if std::fs::symlink_metadata(to).is_ok() && !case_only_rename(from, to) {
                    return Err(CopyError::new(ErrKind::Exists, to));
                }
                std::fs::rename(from, to).map_err(|e| err(e, from))
            }
            _ => Err(err(e, from)),
        }
    }

    fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        std::fs::rename(from, to).map_err(|e| err(e, to))
    }

    /// `F_FULLFSYNC`: que la copia esté en el disco (no solo en su caché) antes de borrar el
    /// original al mover.
    fn sync(&self, p: &Path) -> Result<(), CopyError> {
        let f = std::fs::File::open(p).map_err(|e| err(e, p))?;
        if unsafe { libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&f), libc::F_FULLFSYNC) } == -1 {
            // Discos que no lo tienen (red, FAT): lo que haya.
            f.sync_all().map_err(|e| err(e, p))?;
        }
        Ok(())
    }

    fn remove(&self, p: &Path) -> Result<(), CopyError> {
        let m = std::fs::symlink_metadata(p).map_err(|e| err(e, p))?;
        if m.is_dir() { std::fs::remove_dir(p) } else { std::fs::remove_file(p) }.map_err(|e| err(e, p))
    }

    // TODO(merge tanda D): anotar lo mandado a la Papelera (`log_trashed`, Task 3) y
    // `trash_origin` = `trash::origin` (Task 3, otra rama). Sin él, «Devolver» dice «No se sabe
    // de dónde vino».
    fn trash(&self, p: &Path) -> Result<String, CopyError> {
        let in_trash = super::macos::trash(p).map_err(|e| CopyError::new(ErrKind::Other(e), p))?;
        if in_trash.as_os_str().is_empty() {
            // Sin saber dónde quedó no se puede devolver.
            return Err(CopyError::new(ErrKind::Other("No se sabe dónde quedó en la Papelera".into()), p));
        }
        Ok(in_trash.to_string_lossy().into_owned())
    }

    /// Adentro de la Papelera se le pone el nombre de `shown` si está libre (así se reconoce lo
    /// que reemplazó); «Deshacer» lo devuelve igual, por la ruta que se devuelve acá.
    fn trash_as(&self, p: &Path, shown: &Path) -> Result<String, CopyError> {
        let in_trash = PathBuf::from(self.trash(p)?);
        if let (Some(dir), Some(name)) = (in_trash.parent(), shown.file_name()) {
            let named = dir.join(name);
            if named != in_trash && self.rename_noreplace(&in_trash, &named).is_ok() {
                return Ok(named.to_string_lossy().into_owned());
            }
        }
        Ok(in_trash.to_string_lossy().into_owned())
    }

    fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError> {
        self.rename_noreplace(Path::new(in_trash), original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use xdir_core::ops::Undo;
    use xdir_core::transfer::{Answer, Choice, Control, JobOutcome, JobSpec, Update, run};

    /// Carpeta temporal propia de la prueba (bajo `TMPDIR`).
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-mac-prims-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn big(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn prims_copy_file_progress_and_cancel() {
        let dir = tmp("progreso");
        let (src, dst) = (dir.join("grande.bin"), dir.join("copia.bin"));
        std::fs::write(&src, big(20 << 20)).unwrap();
        let mut seen = Vec::new();
        MacCopyPrims
            .copy_file(&src, &dst, &mut |n| {
                seen.push(n);
                Flow::Continue
            })
            .unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&src).unwrap());
        // En APFS clona (sin avisos); si copia, los avisos crecen.
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        let other = dir.join("cortada.bin");
        let mut calls = 0;
        let r = MacCopyPrims.copy_file(&src, &other, &mut |_| {
            calls += 1;
            Flow::Stop
        });
        if calls > 0 {
            assert_eq!(r.unwrap_err().kind, ErrKind::Cancelled);
            assert!(std::fs::symlink_metadata(&other).is_err(), "quedó el archivo a medio copiar");
        } else {
            // Se clonó entero sin avisos: no hubo nada que cortar.
            r.unwrap();
        }
    }

    #[test]
    fn prims_copy_without_clone_reports_and_cancels() {
        // Sin clonar (como entre discos): avisos crecientes y cortar a la mitad no deja nada.
        let dir = tmp("progreso-datos");
        let (src, dst) = (dir.join("grande.bin"), dir.join("copia.bin"));
        std::fs::write(&src, big(20 << 20)).unwrap();
        let mut seen = Vec::new();
        copy_data(&src, &dst, &mut |n| {
            seen.push(n);
            Flow::Continue
        }, 0)
        .unwrap();
        assert!(seen.len() > 1 && seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&src).unwrap());
        let other = dir.join("cortada.bin");
        let e = copy_data(&src, &other, &mut |n| if n >= 10 << 20 { Flow::Stop } else { Flow::Continue }, 0).unwrap_err();
        assert_eq!(e.kind, ErrKind::Cancelled);
        assert!(std::fs::symlink_metadata(&other).is_err(), "quedó el archivo a medio copiar");
    }

    #[test]
    fn prims_copy_keeps_mtime_and_mode() {
        let dir = tmp("metadatos");
        let (src, dst) = (dir.join("a.sh"), dir.join("b.sh"));
        std::fs::write(&src, b"#!/bin/sh").unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o751)).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        File::options().write(true).open(&src).unwrap().set_modified(old).unwrap();
        MacCopyPrims.copy_file(&src, &dst, &mut |_| Flow::Continue).unwrap();
        let m = std::fs::metadata(&dst).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o751);
        assert_eq!(m.modified().unwrap(), old);
    }

    #[test]
    fn prims_copy_refuses_existing() {
        let dir = tmp("existe");
        let (src, dst) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&src, b"nuevo").unwrap();
        std::fs::write(&dst, b"viejo").unwrap();
        let e = MacCopyPrims.copy_file(&src, &dst, &mut |_| Flow::Continue).unwrap_err();
        assert_eq!(e.kind, ErrKind::Exists);
        assert_eq!(std::fs::read(&dst).unwrap(), b"viejo");
    }

    #[test]
    fn prims_rename_noreplace_refuses() {
        let dir = tmp("renombrar");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert_eq!(MacCopyPrims.rename_noreplace(&a, &b).unwrap_err().kind, ErrKind::Exists);
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        // Dos enlaces duros con otro nombre son el mismo archivo, pero no se pisan.
        std::fs::hard_link(&b, dir.join("duro.txt")).unwrap();
        assert_eq!(MacCopyPrims.rename_noreplace(&b, &dir.join("duro.txt")).unwrap_err().kind, ErrKind::Exists);
        assert!(b.exists() && dir.join("duro.txt").exists());
        assert!(MacCopyPrims.stat(&b).unwrap().same_item(&MacCopyPrims.stat(&dir.join("duro.txt")).unwrap()));
        MacCopyPrims.sync(&b).unwrap();
        // Solo mayúsculas (en APFS sin distinción): se puede.
        let upper = dir.join("A.txt");
        MacCopyPrims.rename_noreplace(&a, &upper).unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name() == "A.txt"));
    }

    #[test]
    fn prims_replace_is_atomic() {
        let dir = tmp("reemplazar");
        let (tmp_file, dst) = (dir.join(".xdir-1.tmp"), dir.join("informe.pdf"));
        std::fs::write(&tmp_file, b"nuevo").unwrap();
        std::fs::write(&dst, b"viejo").unwrap();
        MacCopyPrims.replace(&tmp_file, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"nuevo");
        assert!(std::fs::symlink_metadata(&tmp_file).is_err());
    }

    #[test]
    fn prims_symlink_copied_as_link() {
        let dir = tmp("enlace");
        std::os::unix::fs::symlink("/no/existe/destino", dir.join("l")).unwrap();
        assert_eq!(MacCopyPrims.stat(&dir.join("l")).unwrap().kind, ItemKind::Symlink);
        MacCopyPrims.copy_symlink(&dir.join("l"), &dir.join("l2")).unwrap();
        assert_eq!(std::fs::read_link(dir.join("l2")).unwrap(), PathBuf::from("/no/existe/destino"));
    }

    #[test]
    fn prims_finish_dir_keeps_mtime() {
        let dir = tmp("carpeta");
        let (src, dst) = (dir.join("orig"), dir.join("copia"));
        std::fs::create_dir(&src).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_200_000_000);
        File::open(&src).unwrap().set_modified(old).unwrap();
        MacCopyPrims.create_dir(&dst).unwrap();
        assert_eq!(MacCopyPrims.create_dir(&dst).unwrap_err().kind, ErrKind::Exists);
        std::fs::write(dst.join("adentro.txt"), b"x").unwrap();
        MacCopyPrims.finish_dir(&dst, &src).unwrap();
        assert_eq!(std::fs::metadata(&dst).unwrap().modified().unwrap(), old);
        assert!(MacCopyPrims.remove(&dst).is_err());
    }

    #[test]
    fn prims_err_kind_mapping() {
        assert_eq!(err_kind(libc::EBUSY), ErrKind::InUse);
        assert_eq!(err_kind(libc::ETXTBSY), ErrKind::InUse);
        assert_eq!(err_kind(libc::EACCES), ErrKind::Denied);
        assert_eq!(err_kind(libc::EPERM), ErrKind::Denied);
        assert_eq!(err_kind(libc::ENOENT), ErrKind::NotFound);
        assert_eq!(err_kind(libc::ENOSPC), ErrKind::DiskFull);
        assert_eq!(err_kind(libc::EDQUOT), ErrKind::DiskFull);
        assert_eq!(err_kind(libc::EFBIG), ErrKind::TooBig);
        assert_eq!(err_kind(libc::EEXIST), ErrKind::Exists);
        assert_eq!(err_kind(libc::EXDEV), ErrKind::CrossDevice);
        assert!(matches!(err_kind(libc::EIO), ErrKind::Other(_)));
    }

    #[test]
    fn prims_volume_and_stat() {
        let dir = tmp("disco");
        std::fs::write(dir.join("a.txt"), b"hola").unwrap();
        let v = MacCopyPrims.volume(&dir).unwrap();
        assert!(v.free > 0 && !v.label.is_empty());
        assert_eq!(v.max_file, None);
        assert_eq!(MacCopyPrims.volume(&dir.join("a.txt")).unwrap().id, v.id);
        assert!(MacCopyPrims.volume(&dir.join("no-esta")).is_none());
        let s = MacCopyPrims.stat(&dir.join("a.txt")).unwrap();
        assert_eq!((s.kind, s.size, s.cloud_only), (ItemKind::File, 4, false));
        assert_eq!(MacCopyPrims.stat(&dir.join("no-esta")).unwrap_err().kind, ErrKind::NotFound);
    }

    /// Las primitivas reales, sin Papelera (para no tocar la del usuario) y, si se pide, con la
    /// carpeta `other` como si fuera otro disco (mover entre discos de verdad).
    struct Real {
        other: Option<PathBuf>,
    }

    impl CopyPrims for Real {
        fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
            MacCopyPrims.stat(p)
        }
        fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
            MacCopyPrims.read_dir(p)
        }
        fn volume(&self, p: &Path) -> Option<VolumeInfo> {
            let mut v = MacCopyPrims.volume(p)?;
            v.has_trash = false;
            if self.other.as_ref().is_some_and(|o| p.starts_with(o)) {
                v.id = "otro".into();
            }
            Some(v)
        }
        fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
            MacCopyPrims.copy_file(src, dst, progress)
        }
        fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
            MacCopyPrims.copy_symlink(src, dst)
        }
        fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
            MacCopyPrims.create_dir(dst)
        }
        fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
            MacCopyPrims.finish_dir(dst, like)
        }
        fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
            MacCopyPrims.rename_noreplace(from, to)
        }
        fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
            MacCopyPrims.replace(from, to)
        }
        fn remove(&self, p: &Path) -> Result<(), CopyError> {
            MacCopyPrims.remove(p)
        }
        fn trash(&self, p: &Path) -> Result<String, CopyError> {
            Err(CopyError::new(ErrKind::Other("sin Papelera en la prueba".into()), p))
        }
        fn untrash(&self, _in_trash: &str, original: &Path) -> Result<(), CopyError> {
            Err(CopyError::new(ErrKind::Other("sin Papelera en la prueba".into()), original))
        }
    }

    fn go(prims: &dyn CopyPrims, spec: JobSpec, answers: &[Answer], ctl: &Arc<Control>, hook: &mut dyn FnMut(&Update)) -> JobOutcome {
        let mut answers = answers.iter().copied();
        let mut last = None;
        run(spec, prims, ctl, &mut |u| {
            if let Update::Conflict(_) = &u {
                ctl.answer(answers.next().expect("conflicto sin respuesta"));
            }
            hook(&u);
            if let Update::Finished(o) = u {
                last = Some(o);
            }
        });
        last.unwrap()
    }

    fn no_tmp(dir: &Path) {
        let left: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with(".xdir-")).collect();
        assert!(left.is_empty(), "quedaron temporales: {left:?}");
    }

    #[test]
    fn engine_real_copy_tree() {
        let dir = tmp("motor");
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        std::fs::create_dir_all(src.join("Fotos/sub")).unwrap();
        std::fs::write(src.join("Fotos/a.jpg"), big(3 << 20)).unwrap();
        std::fs::write(src.join("Fotos/sub/b.jpg"), b"bbb").unwrap();
        std::os::unix::fs::symlink("a.jpg", src.join("Fotos/enlace")).unwrap();
        std::fs::create_dir(&dst).unwrap();
        let prims = Real { other: None };
        let out = go(&prims, JobSpec::Copy { sources: vec![src.join("Fotos")], dest: dst.clone() }, &[], &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(std::fs::read(dst.join("Fotos/a.jpg")).unwrap(), big(3 << 20));
        assert_eq!(std::fs::read(dst.join("Fotos/sub/b.jpg")).unwrap(), b"bbb");
        assert_eq!(std::fs::read_link(dst.join("Fotos/enlace")).unwrap(), PathBuf::from("a.jpg"));
        assert_eq!(out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![dst.join("Fotos")])])));
        // «Foto.JPG» sobre «b.jpg» con otras mayúsculas: en APFS (sin distinción) es conflicto.
        if !MacCopyPrims.volume(&dir).unwrap().case_insensitive {
            return;
        }
        std::fs::write(src.join("B.JPG"), b"otra").unwrap();
        let out = go(&prims, JobSpec::Copy { sources: vec![src.join("B.JPG")], dest: dst.join("Fotos/sub") }, &[Answer { choice: Choice::KeepBoth, all: false }], &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty());
        assert_eq!(std::fs::read(dst.join("Fotos/sub/b.jpg")).unwrap(), b"bbb");
        assert_eq!(std::fs::read(dst.join("Fotos/sub/B 2.JPG")).unwrap(), b"otra");
    }

    #[test]
    fn engine_real_replace_and_cancel_keep_original() {
        let dir = tmp("motor-reemplazar");
        std::fs::create_dir_all(dir.join("s")).unwrap();
        std::fs::create_dir_all(dir.join("d")).unwrap();
        std::fs::write(dir.join("s/informe.pdf"), big(20 << 20)).unwrap();
        std::fs::write(dir.join("d/informe.pdf"), b"viejo").unwrap();
        let prims = Real { other: None };
        let spec = || JobSpec::Copy { sources: vec![dir.join("s/informe.pdf")], dest: dir.join("d") };
        let replace = [Answer { choice: Choice::Replace, all: false }];
        // Cancelar en el primer aviso (si APFS clona, no hay avisos: termina bien).
        let ctl = Control::new();
        let c = ctl.clone();
        let out = go(&prims, spec(), &replace, &ctl, &mut |u| {
            if matches!(u, Update::Progress { bytes, .. } if *bytes > 0) {
                c.cancel();
            }
        });
        if out.cancelled {
            assert_eq!(std::fs::read(dir.join("d/informe.pdf")).unwrap(), b"viejo");
        }
        no_tmp(&dir.join("d"));
        let out = go(&prims, spec(), &replace, &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(std::fs::read(dir.join("d/informe.pdf")).unwrap(), big(20 << 20));
        no_tmp(&dir.join("d"));
    }

    #[test]
    fn engine_real_move_between_volumes() {
        let dir = tmp("motor-mover");
        let (src, other) = (dir.join("interno"), dir.join("usb"));
        std::fs::create_dir_all(src.join("Dir/sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(src.join("Dir/a.txt"), b"a").unwrap();
        std::fs::write(src.join("Dir/sub/b.txt"), b"b").unwrap();
        std::fs::write(src.join("Dir/sub/cerrado.txt"), b"c").unwrap();
        std::fs::set_permissions(src.join("Dir/sub/cerrado.txt"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let prims = Real { other: Some(other.clone()) };
        let out = go(&prims, JobSpec::Move { sources: vec![src.join("Dir")], dest: other.clone() }, &[], &Control::new(), &mut |_| {});
        assert_eq!(std::fs::read(other.join("Dir/a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(other.join("Dir/sub/b.txt")).unwrap(), b"b");
        assert!(!src.join("Dir/a.txt").exists() && !src.join("Dir/sub/b.txt").exists());
        // El que no se pudo leer queda en el origen, con su carpeta.
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert_eq!(out.errors[0].message, "No tenés permiso");
        assert!(std::fs::symlink_metadata(src.join("Dir/sub/cerrado.txt")).is_ok());
        assert!(std::fs::symlink_metadata(other.join("Dir/sub/cerrado.txt")).is_err());
        let _ = std::fs::set_permissions(src.join("Dir/sub/cerrado.txt"), std::fs::Permissions::from_mode(0o644));
    }

    /// Primitivas reales con dos trucos de las revisiones: `alias` es otro nombre (un enlace) de
    /// la misma carpeta pero el disco dice que es otro, y la Papelera puede ser una que borra y
    /// falla (`trash_mode` 1, como Windows sin Papelera con `FOFX_RECYCLEONDELETE`) o una carpeta
    /// `bin` (2). 0: sin Papelera.
    struct Sim {
        alias: Option<PathBuf>,
        trash_mode: u8,
        bin: PathBuf,
    }

    impl CopyPrims for Sim {
        fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
            MacCopyPrims.stat(p)
        }
        fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
            MacCopyPrims.read_dir(p)
        }
        fn volume(&self, p: &Path) -> Option<VolumeInfo> {
            let mut v = MacCopyPrims.volume(p)?;
            v.has_trash = self.trash_mode != 0;
            if self.alias.as_ref().is_some_and(|o| p.starts_with(o)) {
                v.id = "otro".into();
            }
            Some(v)
        }
        fn copy_file(&self, s: &Path, d: &Path, pr: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
            // Los archivos «lentos» dan tiempo a que llegue un aviso de progreso a mitad.
            let slow = s.file_name().is_some_and(|n| n == "b.bin");
            copy_data(s, d, &mut |n| {
                if slow {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
                pr(n)
            }, 0)
        }
        fn copy_symlink(&self, s: &Path, d: &Path) -> Result<(), CopyError> {
            MacCopyPrims.copy_symlink(s, d)
        }
        fn create_dir(&self, d: &Path) -> Result<(), CopyError> {
            MacCopyPrims.create_dir(d)
        }
        fn finish_dir(&self, d: &Path, l: &Path) -> Result<(), CopyError> {
            MacCopyPrims.finish_dir(d, l)
        }
        fn rename_noreplace(&self, f: &Path, t: &Path) -> Result<(), CopyError> {
            MacCopyPrims.rename_noreplace(f, t)
        }
        fn replace(&self, f: &Path, t: &Path) -> Result<(), CopyError> {
            MacCopyPrims.replace(f, t)
        }
        fn remove(&self, p: &Path) -> Result<(), CopyError> {
            MacCopyPrims.remove(p)
        }
        fn trash(&self, p: &Path) -> Result<String, CopyError> {
            match self.trash_mode {
                1 => {
                    let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                    Err(CopyError::new(ErrKind::Other("E_FAIL".into()), p))
                }
                _ => {
                    std::fs::create_dir_all(&self.bin).unwrap();
                    let n = std::fs::read_dir(&self.bin).unwrap().count();
                    let to = self.bin.join(format!("{n}-{}", p.file_name().unwrap().to_string_lossy()));
                    std::fs::rename(p, &to).map_err(|e| err(e, p))?;
                    Ok(to.to_string_lossy().into_owned())
                }
            }
        }
        fn untrash(&self, t: &str, o: &Path) -> Result<(), CopyError> {
            MacCopyPrims.rename_noreplace(Path::new(t), o)
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }

    /// Revisión 1, C1: «Reemplazar» moviendo un archivo sobre sí mismo por otro camino (una carpeta
    /// enlazada que el disco informa como otra) lo borraba.
    #[test]
    fn regression_alias_path_replace_on_move_keeps_file() {
        let dir = tmp("regresion-alias");
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("alias")).unwrap();
        std::fs::write(dir.join("real/a.txt"), b"datos unicos").unwrap();
        let prims = Sim { alias: Some(dir.join("alias")), trash_mode: 0, bin: dir.join("bin") };
        let replace = [Answer { choice: Choice::Replace, all: false }];
        go(&prims, JobSpec::Move { sources: vec![dir.join("real/a.txt")], dest: dir.join("alias") }, &replace, &Control::new(), &mut |_| {});
        assert_eq!(std::fs::read(dir.join("real/a.txt")).unwrap(), b"datos unicos");
        // Copiarlo «sobre sí mismo» por el otro camino lo duplica.
        go(&prims, JobSpec::Copy { sources: vec![dir.join("real/a.txt")], dest: dir.join("alias") }, &replace, &Control::new(), &mut |_| {});
        assert_eq!(std::fs::read(dir.join("real/a.txt")).unwrap(), b"datos unicos");
        assert_eq!(names(&dir.join("real")), vec!["a copia.txt", "a.txt"]);
    }

    /// Revisión 1, C2: una Papelera que borra definitivamente y falla dejaba sin lo viejo y sin lo
    /// nuevo. Ahora lo nuevo ya está en su lugar antes de tocar lo viejo.
    #[test]
    fn regression_trash_that_deletes_keeps_new_in_place() {
        let dir = tmp("regresion-papelera-borra");
        std::fs::create_dir_all(dir.join("s/Carpeta")).unwrap();
        std::fs::create_dir_all(dir.join("d/Carpeta")).unwrap();
        std::fs::write(dir.join("s/informe.pdf"), b"nuevo").unwrap();
        std::fs::write(dir.join("d/informe.pdf"), b"viejo").unwrap();
        std::fs::write(dir.join("s/Carpeta/x"), b"x").unwrap();
        std::fs::write(dir.join("d/Carpeta/tesis.docx"), b"tesis").unwrap();
        let prims = Sim { alias: None, trash_mode: 1, bin: dir.join("bin") };
        let replace = Answer { choice: Choice::Replace, all: false };
        let out = go(&prims, JobSpec::Copy { sources: vec![dir.join("s/informe.pdf"), dir.join("s/Carpeta")], dest: dir.join("d") }, &[replace, replace], &Control::new(), &mut |_| {});
        assert_eq!(std::fs::read(dir.join("d/informe.pdf")).unwrap(), b"nuevo");
        assert_eq!(std::fs::read(dir.join("d/Carpeta/x")).unwrap(), b"x");
        // Lo que pasó con lo viejo se informa (no «no pasó nada»).
        assert_eq!(out.errors.len(), 2, "{:?}", out.errors);
        no_tmp(&dir.join("d"));
    }

    /// Revisión 1, I3: cancelar a mitad de «Reemplazar» una carpeta dejaba la nueva a medias y la
    /// vieja en la Papelera. Ahora vuelve la vieja.
    #[test]
    fn regression_replace_folder_cancel_restores_old() {
        let dir = tmp("regresion-cancelar-carpeta");
        std::fs::create_dir_all(dir.join("s/Carpeta")).unwrap();
        std::fs::create_dir_all(dir.join("d/Carpeta")).unwrap();
        std::fs::write(dir.join("s/Carpeta/a.bin"), big(4 << 20)).unwrap();
        std::fs::write(dir.join("s/Carpeta/b.bin"), big(40 << 20)).unwrap();
        std::fs::write(dir.join("d/Carpeta/viejo.txt"), b"viejo").unwrap();
        let prims = Sim { alias: None, trash_mode: 2, bin: dir.join("bin") };
        let ctl = Control::new();
        let c = ctl.clone();
        let out = go(&prims, JobSpec::Copy { sources: vec![dir.join("s/Carpeta")], dest: dir.join("d") }, &[Answer { choice: Choice::Replace, all: false }], &ctl, &mut |u| {
            if matches!(u, Update::Progress { bytes, .. } if *bytes > (6 << 20)) {
                c.cancel();
            }
        });
        assert!(out.cancelled);
        assert_eq!(names(&dir.join("d/Carpeta")), vec!["viejo.txt"]);
        assert!(names(&dir.join("bin")).is_empty());
        no_tmp(&dir.join("d"));
        assert_eq!(out.result.undo, None);
    }

    /// Pruebas de la revisión 2 del motor (puertos de las del revisor), sobre disco real.
    mod rv {
        use super::*;
        use std::sync::Mutex;

        #[derive(Default)]
        struct Hook {
            bin: PathBuf,
            /// 0 papelera-carpeta, 1 borra y falla, 2 falla sin tocar, 3 pánico
            trash_mode: u8,
            no_trash_volume: bool,
            volume_none: bool,
            alias: Option<PathBuf>,
            fail_copy: Option<(String, ErrKind)>,
            cancel_on_aside: Option<Arc<Control>>,
            log: Mutex<Vec<String>>,
        }
        impl CopyPrims for Hook {
            fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> { MacCopyPrims.stat(p) }
            fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> { MacCopyPrims.read_dir(p) }
            fn volume(&self, p: &Path) -> Option<VolumeInfo> {
                if self.volume_none { return None; }
                let mut v = MacCopyPrims.volume(p)?;
                v.has_trash = !self.no_trash_volume;
                if self.alias.as_ref().is_some_and(|o| p.starts_with(o)) { v.id = "otro".into(); }
                Some(v)
            }
            fn copy_file(&self, s: &Path, d: &Path, pr: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
                if let Some((n, k)) = &self.fail_copy && s.file_name().is_some_and(|x| x.to_string_lossy() == *n) {
                    return Err(CopyError::new(k.clone(), d));
                }
                copy_data(s, d, pr, 0)
            }
            fn copy_symlink(&self, s: &Path, d: &Path) -> Result<(), CopyError> { MacCopyPrims.copy_symlink(s, d) }
            fn create_dir(&self, d: &Path) -> Result<(), CopyError> { MacCopyPrims.create_dir(d) }
            fn finish_dir(&self, d: &Path, l: &Path) -> Result<(), CopyError> { MacCopyPrims.finish_dir(d, l) }
            fn rename_noreplace(&self, f: &Path, t: &Path) -> Result<(), CopyError> {
                let r = MacCopyPrims.rename_noreplace(f, t);
                self.log.lock().unwrap().push(format!("rename {} -> {} = {:?}", f.display(), t.display(), r.as_ref().err().map(|e| &e.kind)));
                // Apartar lo que estaba: el destino es «… (reemplazado)».
                let aside = t.file_name().is_some_and(|n| n.to_string_lossy().contains("(reemplazado"));
                if r.is_ok() && aside && let Some(c) = &self.cancel_on_aside { c.cancel(); }
                r
            }
            fn replace(&self, f: &Path, t: &Path) -> Result<(), CopyError> { MacCopyPrims.replace(f, t) }
            fn remove(&self, p: &Path) -> Result<(), CopyError> { MacCopyPrims.remove(p) }
            fn trash(&self, p: &Path) -> Result<String, CopyError> {
                self.log.lock().unwrap().push(format!("trash {}", p.display()));
                match self.trash_mode {
                    1 => {
                        let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                        Err(CopyError::new(ErrKind::Other("E_FAIL".into()), p))
                    }
                    2 => Err(CopyError::new(ErrKind::Other("no".into()), p)),
                    3 => panic!("crash simulado en trash"),
                    _ => {
                        std::fs::create_dir_all(&self.bin).unwrap();
                        let n = std::fs::read_dir(&self.bin).unwrap().count();
                        let to = self.bin.join(format!("{n}-{}", p.file_name().unwrap().to_string_lossy()));
                        std::fs::rename(p, &to).map_err(|e| err(e, p))?;
                        Ok(to.to_string_lossy().into_owned())
                    }
                }
            }
            fn untrash(&self, t: &str, o: &Path) -> Result<(), CopyError> { MacCopyPrims.rename_noreplace(Path::new(t), o) }
        }
        fn rep() -> Answer { Answer { choice: Choice::Replace, all: true } }
        fn all_names(dir: &Path) -> Vec<String> { names(dir) }
        fn rd(p: &Path) -> String { String::from_utf8_lossy(&std::fs::read(p).unwrap_or_else(|e| format!("<{e}>").into_bytes())).into_owned() }

        #[test]
        fn rv_aside_collision_existing_numbered() {
            let dir = tmp("rv-colision");
            std::fs::create_dir_all(dir.join("s")).unwrap();
            std::fs::create_dir_all(dir.join("d")).unwrap();
            std::fs::write(dir.join("s/informe.pdf"), b"nuevo").unwrap();
            std::fs::write(dir.join("d/informe.pdf"), b"viejo").unwrap();
            std::fs::write(dir.join("d/informe 2.pdf"), b"otro").unwrap();
            let p = Hook { bin: dir.join("bin"), trash_mode: 2, ..Default::default() };
            let out = go(&p, JobSpec::Copy { sources: vec![dir.join("s/informe.pdf")], dest: dir.join("d") }, &[rep()], &Control::new(), &mut |_| {});
            eprintln!("errors {:?}\nd={:?}", out.errors, all_names(&dir.join("d")));
            assert_eq!(rd(&dir.join("d/informe.pdf")), "nuevo");
            assert_eq!(rd(&dir.join("d/informe 2.pdf")), "otro");
            // La Papelera falló: lo viejo queda a la vista con el nombre que lo explica.
            assert_eq!(rd(&dir.join("d/informe (reemplazado).pdf")), "viejo");
            assert_eq!(out.errors.len(), 1);
            no_tmp(&dir.join("d"));
        }

        #[test]
        fn rv_replace_dst_symlink_keeps_target() {
            let dir = tmp("rv-enlace");
            std::fs::create_dir_all(dir.join("s")).unwrap();
            std::fs::create_dir_all(dir.join("d")).unwrap();
            std::fs::create_dir_all(dir.join("real")).unwrap();
            std::fs::write(dir.join("real/tesis.docx"), b"tesis").unwrap();
            std::os::unix::fs::symlink(dir.join("real"), dir.join("d/X")).unwrap();
            std::fs::create_dir_all(dir.join("s/X")).unwrap();
            std::fs::write(dir.join("s/X/n.txt"), b"n").unwrap();
            let p = Hook { bin: dir.join("bin"), ..Default::default() };
            let out = go(&p, JobSpec::Copy { sources: vec![dir.join("s/X")], dest: dir.join("d") }, &[rep(), rep()], &Control::new(), &mut |_| {});
            eprintln!("errors {:?} bin={:?}", out.errors, all_names(&dir.join("bin")));
            assert_eq!(rd(&dir.join("real/tesis.docx")), "tesis");
            assert_eq!(rd(&dir.join("d/X/n.txt")), "n");
            // y un archivo sobre el enlace
            std::fs::remove_dir_all(dir.join("d/X")).unwrap();
            std::os::unix::fs::symlink(dir.join("real"), dir.join("d/X")).unwrap();
            std::fs::remove_dir_all(dir.join("s/X")).unwrap();
            std::fs::write(dir.join("s/X"), b"archivo").unwrap();
            let p2 = Hook { bin: dir.join("bin2"), no_trash_volume: true, ..Default::default() };
            let out = go(&p2, JobSpec::Copy { sources: vec![dir.join("s/X")], dest: dir.join("d") }, &[rep()], &Control::new(), &mut |_| {});
            eprintln!("errors {:?}", out.errors);
            assert_eq!(rd(&dir.join("real/tesis.docx")), "tesis");
            assert_eq!(rd(&dir.join("d/X")), "archivo");
        }

        #[test]
        fn rv_replace_folder_containing_source_via_alias() {
            let dir = tmp("rv-contiene");
            std::fs::create_dir_all(dir.join("w/X/X")).unwrap();
            std::fs::write(dir.join("w/X/X/dato"), b"dato").unwrap();
            std::fs::write(dir.join("w/X/hermano"), b"h").unwrap();
            std::os::unix::fs::symlink(dir.join("w"), dir.join("alias")).unwrap();
            for (moving, alias) in [(true, false), (true, true), (false, true), (false, false)] {
                let p = Hook { bin: dir.join("bin"), alias: alias.then(|| dir.join("alias")), ..Default::default() };
                let dest = if alias { dir.join("alias") } else { dir.join("w") };
                let spec = if moving { JobSpec::Move { sources: vec![dir.join("w/X/X")], dest } } else { JobSpec::Copy { sources: vec![dir.join("w/X/X")], dest } };
                let out = go(&p, spec, &[rep(), rep()], &Control::new(), &mut |_| {});
                eprintln!("moving={moving} alias={alias} errors {:?}", out.errors);
                assert_eq!(rd(&dir.join("w/X/X/dato")), "dato");
                assert_eq!(rd(&dir.join("w/X/hermano")), "h");
            }
        }

        #[test]
        fn rv_undo_replace_file_and_folder() {
            for moving in [false, true] {
                let dir = tmp(&format!("rv-deshacer-{moving}"));
                std::fs::create_dir_all(dir.join("s/C")).unwrap();
                std::fs::create_dir_all(dir.join("d/C")).unwrap();
                std::fs::write(dir.join("s/f.txt"), b"nuevo").unwrap();
                std::fs::write(dir.join("d/f.txt"), b"viejo").unwrap();
                std::fs::write(dir.join("s/C/n"), b"n").unwrap();
                std::fs::write(dir.join("d/C/v"), b"v").unwrap();
                let p = Hook { bin: dir.join("bin"), alias: Some(dir.join("d")), ..Default::default() };
                let srcs = vec![dir.join("s/f.txt"), dir.join("s/C")];
                let spec = if moving { JobSpec::Move { sources: srcs, dest: dir.join("d") } } else { JobSpec::Copy { sources: srcs, dest: dir.join("d") } };
                let out = go(&p, spec, &[rep(), rep()], &Control::new(), &mut |_| {});
                eprintln!("moving={moving} errors {:?} undo {:?} bin={:?}", out.errors, out.result.undo, all_names(&dir.join("bin")));
                assert_eq!(rd(&dir.join("d/f.txt")), "nuevo");
                let undo = out.result.undo.clone().unwrap();
                let out2 = go(&p, JobSpec::Undo(undo), &[], &Control::new(), &mut |_| {});
                eprintln!("undo errors {:?} d={:?} s={:?} bin={:?}", out2.errors, all_names(&dir.join("d")), all_names(&dir.join("s")), all_names(&dir.join("bin")));
                assert_eq!(rd(&dir.join("d/f.txt")), "viejo");
                assert_eq!(rd(&dir.join("d/C/v")), "v");
                assert!(!dir.join("d/C/n").exists());
                if moving {
                    assert_eq!(rd(&dir.join("s/f.txt")), "nuevo");
                    assert_eq!(rd(&dir.join("s/C/n")), "n");
                }
                no_tmp(&dir.join("d"));
            }
        }

        #[test]
        fn rv_cancel_right_after_aside() {
            for moving in [false, true] {
                let dir = tmp(&format!("rv-cancel-aparte-{moving}"));
                std::fs::create_dir_all(dir.join("s/C")).unwrap();
                std::fs::create_dir_all(dir.join("d/C")).unwrap();
                std::fs::write(dir.join("s/f.txt"), b"nuevo").unwrap();
                std::fs::write(dir.join("d/f.txt"), b"viejo").unwrap();
                std::fs::write(dir.join("s/C/n"), b"n").unwrap();
                std::fs::write(dir.join("d/C/v"), b"v").unwrap();
                for src in ["f.txt", "C"] {
                    let ctl = Control::new();
                    let p = Hook { bin: dir.join("bin"), alias: Some(dir.join("d")), cancel_on_aside: Some(ctl.clone()), ..Default::default() };
                    let srcs = vec![dir.join("s").join(src)];
                    let spec = if moving { JobSpec::Move { sources: srcs, dest: dir.join("d") } } else { JobSpec::Copy { sources: srcs, dest: dir.join("d") } };
                    let out = go(&p, spec, &[rep()], &ctl, &mut |_| {});
                    eprintln!("moving={moving} {src}: cancelled={} errors {:?} d={:?} s={:?} bin={:?}", out.cancelled, out.errors, all_names(&dir.join("d")), all_names(&dir.join("s")), all_names(&dir.join("bin")));
                    no_tmp(&dir.join("d"));
                }
                // no se perdió nada: o viejo en d, o en bin
                let old_f = rd(&dir.join("d/f.txt")) == "viejo" || all_names(&dir.join("bin")).iter().any(|n| rd(&dir.join("bin").join(n)) == "viejo");
                assert!(old_f);
                assert!(dir.join("d/C/v").exists() || all_names(&dir.join("bin")).iter().any(|n| dir.join("bin").join(n).join("v").exists()));
                assert!(dir.join("s/C/n").exists() || dir.join("d/C/n").exists());
            }
        }

        #[test]
        fn rv_disk_full_and_error_in_folder_replace() {
            for (moving, kind) in [(false, ErrKind::DiskFull), (true, ErrKind::DiskFull), (true, ErrKind::Other("x".into())), (false, ErrKind::Other("x".into()))] {
                let dir = tmp(&format!("rv-lleno-{moving}-{kind:?}").replace(['(', ')', '"', ' '], ""));
                std::fs::create_dir_all(dir.join("s/C")).unwrap();
                std::fs::create_dir_all(dir.join("d/C")).unwrap();
                std::fs::write(dir.join("s/C/a.bin"), b"a").unwrap();
                std::fs::write(dir.join("s/C/b.bin"), b"b").unwrap();
                std::fs::write(dir.join("s/C/c.bin"), b"c").unwrap();
                std::fs::write(dir.join("d/C/v"), b"v").unwrap();
                let p = Hook { bin: dir.join("bin"), alias: Some(dir.join("d")), fail_copy: Some(("b.bin".into(), kind.clone())), ..Default::default() };
                let srcs = vec![dir.join("s/C")];
                let spec = if moving { JobSpec::Move { sources: srcs, dest: dir.join("d") } } else { JobSpec::Copy { sources: srcs, dest: dir.join("d") } };
                let out = go(&p, spec, &[rep()], &Control::new(), &mut |_| {});
                eprintln!("moving={moving} {kind:?}: stopped={:?} errors {:?} d={:?} d/C={:?} s/C={:?} bin={:?} undo={:?}", out.stopped, out.errors, all_names(&dir.join("d")), all_names(&dir.join("d/C")), all_names(&dir.join("s/C")), all_names(&dir.join("bin")), out.result.undo);
                assert_eq!(rd(&dir.join("d/C/v")), "v");
                for f in ["a.bin", "b.bin", "c.bin"] {
                    let somewhere = dir.join("s/C").join(f).exists() || dir.join("d/C 2").join(f).exists();
                    assert!(somewhere, "{f} perdido");
                }
                no_tmp(&dir.join("d"));
            }
        }

        #[test]
        #[ignore = "revisión 2, punto 3"]
        fn rv_walk_error_inside_replaced_folder() {
            let dir = tmp("rv-recorrido");
            std::fs::create_dir_all(dir.join("s/C/cerrada")).unwrap();
            std::fs::write(dir.join("s/C/cerrada/x"), b"x").unwrap();
            std::fs::write(dir.join("s/C/a"), b"a").unwrap();
            std::fs::create_dir_all(dir.join("d/C")).unwrap();
            std::fs::write(dir.join("d/C/v"), b"v").unwrap();
            std::fs::set_permissions(dir.join("s/C/cerrada"), std::fs::Permissions::from_mode(0o000)).unwrap();
            let p = Hook { bin: dir.join("bin"), ..Default::default() };
            let out = go(&p, JobSpec::Copy { sources: vec![dir.join("s/C")], dest: dir.join("d") }, &[rep()], &Control::new(), &mut |_| {});
            std::fs::set_permissions(dir.join("s/C/cerrada"), std::fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("errors {:?} d/C={:?} bin={:?}", out.errors, all_names(&dir.join("d/C")), all_names(&dir.join("bin")));
            // Con un error adentro (aunque sea del recorrido), vuelve la carpeta vieja.
            assert!(!out.errors.is_empty());
            assert_eq!(all_names(&dir.join("d/C")), vec!["v"]);
            assert!(all_names(&dir.join("bin")).is_empty());
            no_tmp(&dir.join("d"));
        }

        #[test]
        fn rv_volume_none_trash_deletes() {
            let dir = tmp("rv-sin-volumen");
            std::fs::create_dir_all(dir.join("s")).unwrap();
            std::fs::create_dir_all(dir.join("d")).unwrap();
            std::fs::write(dir.join("s/f"), b"nuevo").unwrap();
            std::fs::write(dir.join("d/f"), b"viejo").unwrap();
            let p = Hook { bin: dir.join("bin"), volume_none: true, trash_mode: 1, ..Default::default() };
            let out = go(&p, JobSpec::Move { sources: vec![dir.join("s/f")], dest: dir.join("d") }, &[rep()], &Control::new(), &mut |_| {});
            eprintln!("errors {:?} d={:?} s={:?}", out.errors, all_names(&dir.join("d")), all_names(&dir.join("s")));
            assert_eq!(rd(&dir.join("d/f")), "nuevo");
        }

        #[test]
        fn rv_crash_after_aside_leaves_visible_old() {
            let dir = tmp("rv-crash");
            std::fs::create_dir_all(dir.join("s")).unwrap();
            std::fs::create_dir_all(dir.join("d")).unwrap();
            std::fs::write(dir.join("s/f"), b"nuevo").unwrap();
            std::fs::write(dir.join("d/f"), b"viejo").unwrap();
            let p = Hook { bin: dir.join("bin"), trash_mode: 3, ..Default::default() };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                go(&p, JobSpec::Copy { sources: vec![dir.join("s/f")], dest: dir.join("d") }, &[rep()], &Control::new(), &mut |_| {});
            }));
            eprintln!("panicked={} d={:?}", r.is_err(), all_names(&dir.join("d")));
            // Si xdir se cae entre apartar y tirar a la Papelera, lo viejo queda a la vista.
            assert!(r.is_err());
            assert_eq!(rd(&dir.join("d/f (reemplazado)")), "viejo");
            assert_eq!(rd(&dir.join("d/f")), "nuevo");
        }

        #[test]
        fn rv_trash_name_is_visible() {
            let dir = tmp("rv-nombre-papelera");
            std::fs::create_dir_all(dir.join("s")).unwrap();
            std::fs::create_dir_all(dir.join("d")).unwrap();
            std::fs::write(dir.join("s/informe.pdf"), b"nuevo").unwrap();
            std::fs::write(dir.join("d/informe.pdf"), b"viejo").unwrap();
            let p = Hook { bin: dir.join("bin"), ..Default::default() };
            let out = go(&p, JobSpec::Copy { sources: vec![dir.join("s/informe.pdf")], dest: dir.join("d") }, &[rep()], &Control::new(), &mut |_| {});
            let calls: Vec<String> = p.log.lock().unwrap().iter().filter(|l| l.starts_with("trash")).cloned().collect();
            eprintln!("undo {:?} trash calls {calls:?}", out.result.undo);
            assert_eq!(calls, vec![format!("trash {}", dir.join("d/informe (reemplazado).pdf").display())]);
            // Deshacer lo devuelve a su nombre.
            go(&p, JobSpec::Undo(out.result.undo.unwrap()), &[], &Control::new(), &mut |_| {});
            assert_eq!(rd(&dir.join("d/informe.pdf")), "viejo");
        }
    }
}

//! Primitivas de copia de macOS (`xdir_core::sys::CopyPrims`) para el motor de copia del núcleo:
//! `copyfile` por archivo (clona en APFS, conserva atributos extendidos, etiquetas y ACL) con
//! aviso de progreso por bloque, `renamex_np(RENAME_EXCL)` para renombrar sin pisar y la
//! Papelera de `NSFileManager`. Lo que está solo en la nube (`SF_DATALESS`) se informa sin leerlo.

use std::ffi::{CString, OsString, c_char, c_int, c_void};
use std::io;
use std::os::macos::fs::MetadataExt as MacMetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

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

    fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError> {
        self.rename_noreplace(Path::new(in_trash), original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
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
}

//! Primitivas de copia de Linux (`xdir_core::sys::CopyPrims`) para el motor de copia del núcleo:
//! `ioctl(FICLONE)` (clona en Btrfs/XFS) y si no `copy_file_range` en bloques de 8 MiB (y
//! `read`/`write` si tampoco), abriendo el destino con `O_CREAT | O_EXCL`; después permisos,
//! fechas (`futimens`) y atributos `user.*`. Renombrar sin pisar con `renameat2(RENAME_NOREPLACE)`.
//! La Papelera es la de freedesktop (`crate::trash`).

use std::ffi::{CString, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use xdir_core::sys::{CopyError, CopyPrims, ErrKind, Flow, ItemKind, ItemStat, VolumeInfo};

use crate::trash;

/// Bloque de copia (un aviso de progreso por bloque).
const BLOCK: usize = 8 * 1024 * 1024;

/// Segundos entre 1601 y 1970 (para pasar fechas a FILETIME).
const EPOCH_1601: i64 = 11_644_473_600;

pub struct LinuxCopyPrims;

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

fn c_path(p: &Path) -> Result<CString, CopyError> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| CopyError::new(ErrKind::Other("Ruta inválida".into()), p))
}

fn filetime(secs: i64, nanos: i64) -> u64 {
    ((secs + EPOCH_1601).max(0) as u64) * 10_000_000 + (nanos.max(0) as u64) / 100
}

fn trash_dir(p: &Path) -> Result<PathBuf, CopyError> {
    trash::data_home().map(|d| trash::trash_dir(&d)).ok_or_else(|| CopyError::new(ErrKind::Other("No hay Papelera".into()), p))
}

/// Copia los atributos `user.*` de `from` a `to` (lo que se pueda: no todos los discos los
/// tienen).
fn copy_xattrs(from: i32, to: i32) {
    let mut names = vec![0u8; 4096];
    let n = unsafe { libc::flistxattr(from, names.as_mut_ptr().cast(), names.len()) };
    if n <= 0 {
        return;
    }
    for name in names[..n as usize].split(|b| *b == 0).filter(|n| n.starts_with(b"user.")) {
        let Ok(cname) = CString::new(name) else { continue };
        let mut value = vec![0u8; 64 * 1024];
        let len = unsafe { libc::fgetxattr(from, cname.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
        if len >= 0 {
            unsafe { libc::fsetxattr(to, cname.as_ptr(), value.as_ptr().cast(), len as usize, 0) };
        }
    }
}

/// Fechas de acceso y modificación de `meta` en el archivo abierto `fd`.
fn set_times(fd: i32, meta: &std::fs::Metadata) {
    let times = [
        libc::timespec { tv_sec: meta.atime() as _, tv_nsec: meta.atime_nsec() as _ },
        libc::timespec { tv_sec: meta.mtime() as _, tv_nsec: meta.mtime_nsec() as _ },
    ];
    unsafe { libc::futimens(fd, times.as_ptr()) };
}

/// El contenido de `src` en `dst` (ya abiertos), con progreso. `Ok(false)` si se cortó.
fn copy_data(src: &mut File, dst: &mut File, size: u64, progress: &mut dyn FnMut(u64) -> Flow) -> io::Result<bool> {
    // Clonar: instantáneo y sin ocupar lugar, donde el disco puede.
    if unsafe { libc::ioctl(dst.as_raw_fd(), libc::FICLONE, src.as_raw_fd()) } == 0 {
        return Ok(progress(size) == Flow::Continue);
    }
    let mut done = 0u64;
    let mut range = true;
    let mut buf = Vec::new();
    loop {
        let n = if range {
            let r = unsafe {
                libc::copy_file_range(src.as_raw_fd(), std::ptr::null_mut(), dst.as_raw_fd(), std::ptr::null_mut(), BLOCK, 0)
            };
            if r < 0 {
                let e = io::Error::last_os_error();
                match e.raw_os_error() {
                    // Sin `copy_file_range` entre estos discos: a mano, desde donde iba.
                    Some(libc::EXDEV | libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP) if done == 0 => {
                        range = false;
                        continue;
                    }
                    _ => return Err(e),
                }
            }
            r as usize
        } else {
            if buf.is_empty() {
                buf = vec![0u8; BLOCK];
            }
            let r = src.read(&mut buf)?;
            dst.write_all(&buf[..r])?;
            r
        };
        if n == 0 {
            return Ok(true);
        }
        done += n as u64;
        if progress(done) == Flow::Stop {
            return Ok(false);
        }
    }
}

impl LinuxCopyPrims {
    fn copy_file_inner(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        let mut from = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(src).map_err(|e| err(e, src))?;
        let meta = from.metadata().map_err(|e| err(e, src))?;
        if !meta.is_file() {
            return Err(CopyError::new(ErrKind::Other("No es un archivo".into()), src));
        }
        // `create_new` = O_CREAT | O_EXCL: nunca pisa nada.
        let mut to = OpenOptions::new().write(true).create_new(true).mode(meta.permissions().mode() | 0o200).open(dst).map_err(|e| err(e, dst))?;
        // De acá en adelante el destino es nuestro: si algo falla, se borra.
        let result = match copy_data(&mut from, &mut to, meta.len(), progress) {
            Ok(true) => {
                copy_xattrs(from.as_raw_fd(), to.as_raw_fd());
                let perms = std::fs::Permissions::from_mode(meta.permissions().mode());
                let _ = to.set_permissions(perms);
                set_times(to.as_raw_fd(), &meta);
                to.sync_data().map_err(|e| err(e, dst))
            }
            Ok(false) => Err(CopyError::new(ErrKind::Cancelled, dst)),
            Err(e) => Err(err(e, dst)),
        };
        drop(to);
        if result.is_err() {
            let _ = std::fs::remove_file(dst);
        }
        result
    }
}

impl CopyPrims for LinuxCopyPrims {
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
        Ok(ItemStat { kind, size, modified: filetime(m.mtime(), m.mtime_nsec()), cloud_only: false })
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
        let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
        let read_only = unsafe { libc::statvfs(cp.as_ptr(), &mut vfs) } == 0 && vfs.f_flag & libc::ST_RDONLY != 0;
        // Los tipos de `f_type` y de la constante cambian según la arquitectura.
        #[allow(clippy::unnecessary_cast)]
        let fat = fs.f_type as i64 == libc::MSDOS_SUPER_MAGIC as i64;
        let mount = mount_point(p);
        let label = mount.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| mount.display().to_string());
        Some(VolumeInfo {
            id: OsString::from(m.dev().to_string()),
            label,
            case_insensitive: false,
            has_trash: !read_only,
            max_file: fat.then_some((4u64 << 30) - 1),
            free: fs.f_bavail as u64 * fs.f_bsize as u64,
        })
    }

    fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        self.copy_file_inner(src, dst, progress)
    }

    fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
        let target = std::fs::read_link(src).map_err(|e| err(e, src))?;
        std::os::unix::fs::symlink(target, dst).map_err(|e| err(e, dst))
    }

    fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
        std::fs::create_dir(dst).map_err(|e| err(e, dst))
    }

    fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
        let meta = std::fs::metadata(like).map_err(|e| err(e, like))?;
        let dir = File::open(dst).map_err(|e| err(e, dst))?;
        let from = File::open(like).map_err(|e| err(e, like))?;
        copy_xattrs(from.as_raw_fd(), dir.as_raw_fd());
        set_times(dir.as_raw_fd(), &meta);
        std::fs::set_permissions(dst, meta.permissions()).map_err(|e| err(e, dst))
    }

    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        let (f, t) = (c_path(from)?, c_path(to)?);
        let r = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) };
        if r == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // Discos sin `RENAME_NOREPLACE`: se comprueba y se renombra.
            Some(libc::EINVAL | libc::ENOSYS) => {
                if std::fs::symlink_metadata(to).is_ok() {
                    return Err(CopyError::new(ErrKind::Exists, to));
                }
                std::fs::rename(from, to).map_err(|e| err(e, from))
            }
            Some(libc::EEXIST) => Err(CopyError::new(ErrKind::Exists, to)),
            _ => Err(err(e, from)),
        }
    }

    fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        std::fs::rename(from, to).map_err(|e| err(e, to))
    }

    fn remove(&self, p: &Path) -> Result<(), CopyError> {
        let m = std::fs::symlink_metadata(p).map_err(|e| err(e, p))?;
        if m.is_dir() { std::fs::remove_dir(p) } else { std::fs::remove_file(p) }.map_err(|e| err(e, p))
    }

    fn trash(&self, p: &Path) -> Result<String, CopyError> {
        trash::recycle(p, &trash_dir(p)?).map_err(|e| err(e, p))
    }

    // TODO(merge tanda D): `trash_origin` = `trash::origin` (Task 3, otra rama). Sin él, «Devolver»
    // dice «No se sabe de dónde vino».

    fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError> {
        trash::restore(in_trash, original, &trash_dir(original)?).map_err(|e| err(e, original))
    }
}

/// Punto de montaje de `p` (de `/proc/self/mountinfo`: el más largo que la contiene).
fn mount_point(p: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    info.lines()
        .filter_map(|l| l.split(' ').nth(4))
        .map(|m| PathBuf::from(m.replace("\\040", " ")))
        .filter(|m| abs.starts_with(m))
        .max_by_key(|m| m.components().count())
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use xdir_core::ops::Undo;
    use xdir_core::transfer::{Answer, Choice, Control, JobSpec, Update, run};

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-linux-prims-{name}-{}", std::process::id()));
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
        LinuxCopyPrims.copy_file(&src, &dst, &mut |n| {
            seen.push(n);
            Flow::Continue
        })
        .unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&src).unwrap());
        assert!(!seen.is_empty() && seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert_eq!(*seen.last().unwrap(), 20 << 20);
        // Cortar en el primer aviso: Cancelled y sin destino.
        let other = dir.join("cortada.bin");
        let e = LinuxCopyPrims.copy_file(&src, &other, &mut |_| Flow::Stop).unwrap_err();
        assert_eq!(e.kind, ErrKind::Cancelled);
        assert!(std::fs::symlink_metadata(&other).is_err());
    }

    #[test]
    fn prims_copy_keeps_mtime_and_mode() {
        let dir = tmp("metadatos");
        let (src, dst) = (dir.join("a.sh"), dir.join("b.sh"));
        std::fs::write(&src, b"#!/bin/sh").unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o751)).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        File::options().write(true).open(&src).unwrap().set_modified(old).unwrap();
        LinuxCopyPrims.copy_file(&src, &dst, &mut |_| Flow::Continue).unwrap();
        let m = std::fs::metadata(&dst).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o751);
        assert_eq!(m.modified().unwrap(), old);
        assert_eq!(LinuxCopyPrims.stat(&dst).unwrap().modified, LinuxCopyPrims.stat(&src).unwrap().modified);
    }

    #[test]
    fn prims_copy_refuses_existing() {
        let dir = tmp("existe");
        let (src, dst) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&src, b"nuevo").unwrap();
        std::fs::write(&dst, b"viejo").unwrap();
        let e = LinuxCopyPrims.copy_file(&src, &dst, &mut |_| Flow::Continue).unwrap_err();
        assert_eq!(e.kind, ErrKind::Exists);
        assert_eq!(std::fs::read(&dst).unwrap(), b"viejo");
    }

    #[test]
    fn prims_rename_noreplace_refuses() {
        let dir = tmp("renombrar");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert_eq!(LinuxCopyPrims.rename_noreplace(&a, &b).unwrap_err().kind, ErrKind::Exists);
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        LinuxCopyPrims.rename_noreplace(&a, &dir.join("c.txt")).unwrap();
        assert_eq!(std::fs::read(dir.join("c.txt")).unwrap(), b"a");
    }

    #[test]
    fn prims_replace_is_atomic() {
        let dir = tmp("reemplazar");
        let (tmp_file, dst) = (dir.join(".xdir-1.tmp"), dir.join("informe.pdf"));
        std::fs::write(&tmp_file, b"nuevo").unwrap();
        std::fs::write(&dst, b"viejo").unwrap();
        LinuxCopyPrims.replace(&tmp_file, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"nuevo");
        assert!(std::fs::symlink_metadata(&tmp_file).is_err());
    }

    #[test]
    fn prims_symlink_copied_as_link() {
        let dir = tmp("enlace");
        std::os::unix::fs::symlink("/no/existe/destino", dir.join("l")).unwrap();
        assert_eq!(LinuxCopyPrims.stat(&dir.join("l")).unwrap().kind, ItemKind::Symlink);
        LinuxCopyPrims.copy_symlink(&dir.join("l"), &dir.join("l2")).unwrap();
        assert_eq!(std::fs::read_link(dir.join("l2")).unwrap(), PathBuf::from("/no/existe/destino"));
    }

    #[test]
    fn prims_finish_dir_keeps_mtime() {
        let dir = tmp("carpeta");
        let (src, dst) = (dir.join("orig"), dir.join("copia"));
        std::fs::create_dir(&src).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_200_000_000);
        File::open(&src).unwrap().set_modified(old).unwrap();
        LinuxCopyPrims.create_dir(&dst).unwrap();
        assert_eq!(LinuxCopyPrims.create_dir(&dst).unwrap_err().kind, ErrKind::Exists);
        std::fs::write(dst.join("adentro.txt"), b"x").unwrap();
        LinuxCopyPrims.finish_dir(&dst, &src).unwrap();
        assert_eq!(std::fs::metadata(&dst).unwrap().modified().unwrap(), old);
        // `remove` no borra carpetas con algo adentro.
        assert!(LinuxCopyPrims.remove(&dst).is_err());
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
        assert!(matches!(err_kind(libc::EIO), ErrKind::Other(_)));
    }

    #[test]
    fn prims_volume_and_stat() {
        let dir = tmp("disco");
        std::fs::write(dir.join("a.txt"), b"hola").unwrap();
        let v = LinuxCopyPrims.volume(&dir).unwrap();
        assert!(!v.case_insensitive);
        assert!(v.free > 0);
        assert_eq!(LinuxCopyPrims.volume(&dir.join("a.txt")).unwrap().id, v.id);
        assert!(LinuxCopyPrims.volume(&dir.join("no-esta")).is_none());
        let s = LinuxCopyPrims.stat(&dir.join("a.txt")).unwrap();
        assert_eq!((s.kind, s.size, s.cloud_only), (ItemKind::File, 4, false));
        assert_eq!(LinuxCopyPrims.stat(&dir.join("no-esta")).unwrap_err().kind, ErrKind::NotFound);
    }

    /// Las primitivas reales, pero sin Papelera (para no tocar la del usuario) y, si se pide, con
    /// la carpeta `other` como si fuera otro disco (mover entre discos de verdad).
    struct Real {
        other: Option<PathBuf>,
    }

    impl CopyPrims for Real {
        fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
            LinuxCopyPrims.stat(p)
        }
        fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
            LinuxCopyPrims.read_dir(p)
        }
        fn volume(&self, p: &Path) -> Option<VolumeInfo> {
            let mut v = LinuxCopyPrims.volume(p)?;
            v.has_trash = false;
            if self.other.as_ref().is_some_and(|o| p.starts_with(o)) {
                v.id = "otro".into();
            }
            Some(v)
        }
        fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
            LinuxCopyPrims.copy_file(src, dst, progress)
        }
        fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
            LinuxCopyPrims.copy_symlink(src, dst)
        }
        fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
            LinuxCopyPrims.create_dir(dst)
        }
        fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
            LinuxCopyPrims.finish_dir(dst, like)
        }
        fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
            LinuxCopyPrims.rename_noreplace(from, to)
        }
        fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
            LinuxCopyPrims.replace(from, to)
        }
        fn remove(&self, p: &Path) -> Result<(), CopyError> {
            LinuxCopyPrims.remove(p)
        }
        fn trash(&self, p: &Path) -> Result<String, CopyError> {
            Err(CopyError::new(ErrKind::Other("sin Papelera en la prueba".into()), p))
        }
        fn untrash(&self, _in_trash: &str, original: &Path) -> Result<(), CopyError> {
            Err(CopyError::new(ErrKind::Other("sin Papelera en la prueba".into()), original))
        }
    }

    fn go(prims: &dyn CopyPrims, spec: JobSpec, answers: &[Answer], ctl: &Arc<Control>, hook: &mut dyn FnMut(&Update)) -> xdir_core::transfer::JobOutcome {
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

        // Conflicto «Conservar ambos» y pegar en la misma carpeta (nombres de Duplicar).
        let out = go(&prims, JobSpec::Copy { sources: vec![src.join("Fotos/sub/b.jpg")], dest: dst.join("Fotos/sub") }, &[Answer { choice: Choice::KeepBoth, all: false }], &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty());
        assert_eq!(std::fs::read(dst.join("Fotos/sub/b 2.jpg")).unwrap(), b"bbb");
        go(&prims, JobSpec::Copy { sources: vec![dst.join("Fotos/sub/b.jpg")], dest: dst.join("Fotos/sub") }, &[], &Control::new(), &mut |_| {});
        assert!(dst.join("Fotos/sub/b copia.jpg").exists());
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
        // Cancelar a mitad del reemplazo: el viejo queda intacto y no queda el temporal.
        let ctl = Control::new();
        let c = ctl.clone();
        let out = go(&prims, spec(), &replace, &ctl, &mut |u| {
            if matches!(u, Update::Progress { bytes, .. } if *bytes > 0) {
                c.cancel();
            }
        });
        assert!(out.cancelled);
        assert_eq!(std::fs::read(dir.join("d/informe.pdf")).unwrap(), b"viejo");
        no_tmp(&dir.join("d"));
        // Sin cancelar: queda el nuevo (en un disco sin Papelera el viejo lo borra el reemplazo).
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
        // Uno que no se puede leer: queda en el origen con su carpeta (salvo que la prueba corra
        // como root, que lee todo).
        std::fs::write(src.join("Dir/sub/cerrado.txt"), b"c").unwrap();
        std::fs::set_permissions(src.join("Dir/sub/cerrado.txt"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let root = unsafe { libc::geteuid() } == 0;
        let prims = Real { other: Some(other.clone()) };
        let out = go(&prims, JobSpec::Move { sources: vec![src.join("Dir")], dest: other.clone() }, &[], &Control::new(), &mut |_| {});
        assert_eq!(std::fs::read(other.join("Dir/a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(other.join("Dir/sub/b.txt")).unwrap(), b"b");
        assert!(!src.join("Dir/a.txt").exists() && !src.join("Dir/sub/b.txt").exists());
        if root {
            assert!(out.errors.is_empty());
            assert!(!src.join("Dir").exists());
        } else {
            assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
            assert_eq!(out.errors[0].message, "No tenés permiso");
            assert!(src.join("Dir/sub/cerrado.txt").exists());
            assert!(!other.join("Dir/sub/cerrado.txt").exists());
        }
        let _ = std::fs::set_permissions(src.join("Dir/sub/cerrado.txt"), std::fs::Permissions::from_mode(0o644));
    }

    #[test]
    fn prims_trash_and_untrash() {
        // La Papelera de freedesktop del usuario de la prueba (en Docker, la del contenedor).
        let dir = tmp("papelera");
        let f = dir.join("borrar.txt");
        std::fs::write(&f, b"x").unwrap();
        let name = LinuxCopyPrims.trash(&f).unwrap();
        assert!(!f.exists());
        LinuxCopyPrims.untrash(&name, &f).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"x");
    }
}

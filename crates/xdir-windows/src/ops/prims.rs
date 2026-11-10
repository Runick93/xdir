//! Primitivas de copia de Windows (`xdir_core::sys::CopyPrims`) para el motor de copia del núcleo:
//! `CopyFile2` (copia flujos alternativos y atributos, clona en ReFS solo) con aviso de progreso
//! por bloque, `MoveFileExW` sin pisar (o reemplazando, atómico), la Papelera del Shell
//! (`IFileOperation`) y rutas largas con `\\?\`. Lo que está solo en la nube (atributos
//! `OFFLINE`/`RECALL_*`) se informa sin leerlo.

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, Prefix};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_DISK_FULL, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_FILE_TOO_LARGE, ERROR_HANDLE_DISK_FULL, ERROR_LOCK_VIOLATION, ERROR_NOT_SAME_DEVICE, ERROR_PATH_NOT_FOUND,
    ERROR_REQUEST_ABORTED, ERROR_SHARING_VIOLATION, FILETIME, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    COPY_FILE_COPY_SYMLINK, COPY_FILE_FAIL_IF_EXISTS, COPYFILE2_CALLBACK_CHUNK_FINISHED, COPYFILE2_EXTENDED_PARAMETERS,
    COPYFILE2_MESSAGE, COPYFILE2_MESSAGE_ACTION, COPYFILE2_PROGRESS_CANCEL, COPYFILE2_PROGRESS_CONTINUE, CopyFile2,
    CreateDirectoryW, CreateFileW, DeleteFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_OPEN_NO_RECALL, FILE_GENERIC_WRITE, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_WRITE_ATTRIBUTES, FileIdInfo, FindClose, FlushFileBuffers, GetFileInformationByHandle, GetFileInformationByHandleEx, FindFirstFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetFileAttributesExW,
    GetFileExInfoStandard, GetVolumeInformationW, GetVolumePathNameW, MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING, RemoveDirectoryW, SetFileAttributesW, SetFileTime,
    WIN32_FILE_ATTRIBUTE_DATA, WIN32_FIND_DATAW,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::SystemServices::{FILE_CASE_SENSITIVE_SEARCH, FILE_READ_ONLY_VOLUME, IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK};
use windows::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};
use windows::core::PCWSTR;

use xdir_core::sys::{CopyError, CopyPrims, ErrKind, Flow, ItemKind, ItemStat, VolumeInfo};

/// `GetDriveTypeW` (de `Win32_System_WindowsProgramming`, que no se incluye por una constante).
const DRIVE_FIXED: u32 = 3;

pub struct WinCopyPrims;

/// Código de error de Windows → `ErrKind`.
pub(crate) fn err_kind(code: u32) -> ErrKind {
    match code {
        c if c == ERROR_SHARING_VIOLATION.0 || c == ERROR_LOCK_VIOLATION.0 => ErrKind::InUse,
        c if c == ERROR_ACCESS_DENIED.0 => ErrKind::Denied,
        c if c == ERROR_FILE_NOT_FOUND.0 || c == ERROR_PATH_NOT_FOUND.0 => ErrKind::NotFound,
        c if c == ERROR_DISK_FULL.0 || c == ERROR_HANDLE_DISK_FULL.0 => ErrKind::DiskFull,
        c if c == ERROR_FILE_TOO_LARGE.0 => ErrKind::TooBig,
        c if c == ERROR_FILE_EXISTS.0 || c == ERROR_ALREADY_EXISTS.0 => ErrKind::Exists,
        c if c == ERROR_REQUEST_ABORTED.0 => ErrKind::Cancelled,
        c if c == ERROR_NOT_SAME_DEVICE.0 => ErrKind::CrossDevice,
        c => ErrKind::Other(std::io::Error::from_raw_os_error(c as i32).to_string()),
    }
}

/// Error de la API de Windows (un `HRESULT`, casi siempre de un código Win32).
fn err(e: windows::core::Error, p: &Path) -> CopyError {
    let hr = e.code().0 as u32;
    // FACILITY_WIN32: el código está en los 16 bits de abajo.
    let kind = if hr >> 16 == 0x8007 { err_kind(hr & 0xFFFF) } else { ErrKind::Other(e.message()) };
    CopyError::new(kind, p)
}

fn io_err(e: std::io::Error, p: &Path) -> CopyError {
    match e.raw_os_error() {
        Some(n) => CopyError::new(err_kind(n as u32), p),
        None => CopyError::new(ErrKind::Other(e.to_string()), p),
    }
}

/// La ruta en UTF-16 con `\\?\` (rutas de más de 260 caracteres), terminada en 0.
pub(crate) fn long_path(p: &Path) -> Vec<u16> {
    let verbatim: OsString = match p.components().next() {
        Some(Component::Prefix(pre)) => match pre.kind() {
            Prefix::Verbatim(_) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(_) => p.as_os_str().to_owned(),
            Prefix::UNC(..) => {
                // \\server\share\x → \\?\UNC\server\share\x
                let mut s = OsString::from(r"\\?\UNC\");
                s.push(&p.as_os_str().to_string_lossy()[2..]);
                s
            }
            Prefix::Disk(_) => {
                let mut s = OsString::from(r"\\?\");
                s.push(p.as_os_str());
                s
            }
        },
        _ => p.as_os_str().to_owned(),
    };
    verbatim.encode_wide().map(|c| if c == u16::from(b'/') { u16::from(b'\\') } else { c }).chain(Some(0)).collect()
}

fn attr_data(p: &Path) -> Result<WIN32_FILE_ATTRIBUTE_DATA, CopyError> {
    let wide = long_path(p);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe { GetFileAttributesExW(PCWSTR(wide.as_ptr()), GetFileExInfoStandard, (&mut data as *mut WIN32_FILE_ATTRIBUTE_DATA).cast()) }
        .map_err(|e| err(e, p))?;
    Ok(data)
}

/// Etiqueta del punto de reanálisis (enlace simbólico, unión…), si es uno.
fn reparse_tag(p: &Path) -> Option<u32> {
    let wide = long_path(p);
    let mut find = WIN32_FIND_DATAW::default();
    let h = unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut find) }.ok()?;
    unsafe {
        let _ = FindClose(h);
    }
    (find.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0).then_some(find.dwReserved0)
}

/// Abre `p` sin pedir sus datos (ni bajarlo de la nube) para leer o escribir atributos.
fn open_meta(p: &Path, access: u32) -> Result<HANDLE, CopyError> {
    let wide = long_path(p);
    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
            None,
        )
    }
    .map_err(|e| err(e, p))
}

/// Identidad del elemento: número de serie del disco y número de archivo (`FileIdInfo`, de 128
/// bits en ReFS; si no está, el de 64 de `GetFileInformationByHandle`).
fn file_id(p: &Path) -> Option<(u64, u64)> {
    let h = open_meta(p, FILE_READ_ATTRIBUTES.0).ok()?;
    let mut info = FILE_ID_INFO::default();
    let id = unsafe { GetFileInformationByHandleEx(h, FileIdInfo, (&mut info as *mut FILE_ID_INFO).cast(), size_of::<FILE_ID_INFO>() as u32) }
        .ok()
        .map(|()| {
            let b = info.FileId.Identifier;
            let lo = u64::from_le_bytes(b[..8].try_into().unwrap());
            let hi = u64::from_le_bytes(b[8..].try_into().unwrap());
            (info.VolumeSerialNumber ^ hi.rotate_left(17), lo)
        })
        .or_else(|| {
            let mut by = BY_HANDLE_FILE_INFORMATION::default();
            unsafe { GetFileInformationByHandle(h, &mut by) }.ok()?;
            Some((u64::from(by.dwVolumeSerialNumber), (u64::from(by.nFileIndexHigh) << 32) | u64::from(by.nFileIndexLow)))
        });
    unsafe {
        let _ = CloseHandle(h);
    }
    id
}

fn filetime(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Inicia COM en el hilo mientras viva (la Papelera del Shell lo necesita).
struct Com(bool);

impl Com {
    fn new() -> Com {
        Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() })
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Lo que recibe el aviso de `CopyFile2`.
struct Ctx<'a> {
    progress: &'a mut dyn FnMut(u64) -> Flow,
    stopped: bool,
}

unsafe extern "system" fn chunk_done(msg: *const COPYFILE2_MESSAGE, ctx: *const c_void) -> COPYFILE2_MESSAGE_ACTION {
    // SAFETY: `msg` lo da `CopyFile2` y `ctx` es el `Ctx` de `copy_file`, vivo mientras dura.
    let msg = unsafe { &*msg };
    if msg.Type != COPYFILE2_CALLBACK_CHUNK_FINISHED {
        return COPYFILE2_PROGRESS_CONTINUE;
    }
    let ctx = unsafe { &mut *(ctx as *mut Ctx) };
    let done = unsafe { msg.Info.ChunkFinished.uliTotalBytesTransferred };
    // Un pánico no puede cruzar a Windows: si pasa, se corta la copia.
    let flow = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (ctx.progress)(done))).unwrap_or(Flow::Stop);
    if flow == Flow::Stop {
        ctx.stopped = true;
        // Cancelar borra lo copiado a medias.
        return COPYFILE2_PROGRESS_CANCEL;
    }
    COPYFILE2_PROGRESS_CONTINUE
}

fn delete_file(p: &Path) -> Result<(), CopyError> {
    let wide = long_path(p);
    unsafe { DeleteFileW(PCWSTR(wide.as_ptr())) }.map_err(|e| err(e, p))
}

/// Raíz del disco de `p` («C:\», «\\server\share\», el punto de montaje).
fn volume_root(p: &Path) -> Option<Vec<u16>> {
    let wide = long_path(p);
    let mut buf = vec![0u16; 32_768];
    unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut buf) }.ok()?;
    let len = buf.iter().position(|&c| c == 0)?;
    buf.truncate(len + 1);
    Some(buf)
}

impl CopyPrims for WinCopyPrims {
    fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
        let data = attr_data(p)?;
        let attrs = data.dwFileAttributes;
        let link = attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            && reparse_tag(p).is_some_and(|t| t == IO_REPARSE_TAG_SYMLINK || t == IO_REPARSE_TAG_MOUNT_POINT);
        let kind = if link {
            ItemKind::Symlink
        } else if attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            ItemKind::Dir
        } else {
            ItemKind::File
        };
        let size = if kind == ItemKind::File { (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow) } else { 0 };
        let cloud = FILE_ATTRIBUTE_OFFLINE.0 | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0 | FILE_ATTRIBUTE_RECALL_ON_OPEN.0;
        Ok(ItemStat {
            kind,
            size,
            modified: filetime(data.ftLastWriteTime),
            cloud_only: kind == ItemKind::File && attrs & cloud != 0,
            file_id: file_id(p),
        })
    }

    fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
        let rd = std::fs::read_dir(p).map_err(|e| io_err(e, p))?;
        rd.map(|e| e.map(|e| e.file_name()).map_err(|e| io_err(e, p))).collect()
    }

    fn volume(&self, p: &Path) -> Option<VolumeInfo> {
        attr_data(p).ok()?;
        let root = volume_root(p)?;
        let mut name = [0u16; 261];
        let mut fs_name = [0u16; 261];
        let mut flags = 0u32;
        unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut name), None, None, Some(&mut flags), Some(&mut fs_name)) }.ok()?;
        let text = |b: &[u16]| OsString::from_wide(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).to_string_lossy().into_owned();
        let (name, fs_name) = (text(&name), text(&fs_name));
        let mut free = 0u64;
        unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), None, None) }.ok()?;
        let drive = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
        let root_text = text(&root);
        let root_text = root_text.trim_start_matches(r"\\?\").to_string();
        let letter = root_text.strip_suffix(":\\").filter(|l| l.len() == 1);
        let label = match (name.is_empty(), letter) {
            (false, Some(l)) => format!("{name} ({l}:)"),
            (true, Some(l)) => format!("Disco local ({l}:)"),
            (false, None) => name,
            (true, None) => root_text.clone(),
        };
        Some(VolumeInfo {
            id: OsString::from(root_text.to_lowercase()),
            label,
            case_insensitive: flags & FILE_CASE_SENSITIVE_SEARCH == 0,
            // Los extraíbles y los de red no tienen Papelera (mandar ahí lo borra): solo discos
            // fijos donde el sistema la informa.
            has_trash: drive == DRIVE_FIXED && flags & FILE_READ_ONLY_VOLUME == 0 && {
                let mut q = SHQUERYRBINFO { cbSize: size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
                unsafe { SHQueryRecycleBinW(PCWSTR(root.as_ptr()), &mut q) }.is_ok()
            },
            max_file: fs_name.eq_ignore_ascii_case("FAT32").then_some((4u64 << 30) - 1),
            free,
        })
    }

    fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        let (from, to) = (long_path(src), long_path(dst));
        let mut ctx = Ctx { progress, stopped: false };
        let params = COPYFILE2_EXTENDED_PARAMETERS {
            dwSize: size_of::<COPYFILE2_EXTENDED_PARAMETERS>() as u32,
            dwCopyFlags: COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK,
            pfCancel: std::ptr::null_mut(),
            pProgressRoutine: Some(chunk_done),
            pvCallbackContext: (&mut ctx as *mut Ctx).cast(),
        };
        let r = unsafe { CopyFile2(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), Some(&params)) };
        let stopped = ctx.stopped;
        match r {
            Ok(()) if !stopped => Ok(()),
            other => {
                let e = match other {
                    _ if stopped => CopyError::new(ErrKind::Cancelled, dst),
                    Err(e) => err(e, dst),
                    Ok(()) => CopyError::new(ErrKind::Cancelled, dst),
                };
                // Con `FAIL_IF_EXISTS` el destino lo creó esta copia (salvo que ya existiera): lo
                // a medio copiar se borra.
                if e.kind != ErrKind::Exists {
                    let _ = delete_file(dst);
                }
                Err(e)
            }
        }
    }

    fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
        use std::os::windows::fs::FileTypeExt;
        let target = std::fs::read_link(src).map_err(|e| io_err(e, src))?;
        let is_dir = std::fs::symlink_metadata(src).map_err(|e| io_err(e, src))?.file_type().is_symlink_dir();
        let r = if is_dir { std::os::windows::fs::symlink_dir(&target, dst) } else { std::os::windows::fs::symlink_file(&target, dst) };
        r.map_err(|e| io_err(e, dst))
    }

    fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
        let wide = long_path(dst);
        unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), None) }.map_err(|e| err(e, dst))
    }

    fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
        let data = attr_data(like)?;
        let wide = long_path(dst);
        unsafe {
            let h = CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_WRITE_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
            .map_err(|e| err(e, dst))?;
            let r = SetFileTime(h, Some(&data.ftCreationTime), Some(&data.ftLastAccessTime), Some(&data.ftLastWriteTime));
            let _ = CloseHandle(h);
            r.map_err(|e| err(e, dst))?;
            // Atributos (oculto, solo lectura, sistema…), sin los que no se ponen a mano.
            let keep = data.dwFileAttributes & !(FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0);
            SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(keep)).map_err(|e| err(e, dst))
        }
    }

    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        let (f, t) = (long_path(from), long_path(to));
        // Sin `MOVEFILE_COPY_ALLOWED` (no copia entre discos) ni `REPLACE_EXISTING` (no pisa).
        unsafe { MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVE_FILE_FLAGS(0)) }.map_err(|e| err(e, from))
    }

    fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        let (f, t) = (long_path(from), long_path(to));
        unsafe { MoveFileExW(PCWSTR(f.as_ptr()), PCWSTR(t.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }.map_err(|e| err(e, to))
    }

    /// `FlushFileBuffers`: que la copia esté en el disco antes de borrar el original al mover.
    fn sync(&self, p: &Path) -> Result<(), CopyError> {
        let h = open_meta(p, FILE_GENERIC_WRITE.0)?;
        let r = unsafe { FlushFileBuffers(h) };
        unsafe {
            let _ = CloseHandle(h);
        }
        r.map_err(|e| err(e, p))
    }

    fn remove(&self, p: &Path) -> Result<(), CopyError> {
        let data = attr_data(p)?;
        let wide = long_path(p);
        if data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            // Carpeta vacía, o enlace a una carpeta (se borra el enlace, no lo que apunta).
            return unsafe { RemoveDirectoryW(PCWSTR(wide.as_ptr())) }.map_err(|e| err(e, p));
        }
        if data.dwFileAttributes & FILE_ATTRIBUTE_READONLY.0 != 0 {
            let attrs = FILE_FLAGS_AND_ATTRIBUTES(data.dwFileAttributes & !FILE_ATTRIBUTE_READONLY.0);
            unsafe { SetFileAttributesW(PCWSTR(wide.as_ptr()), attrs) }.map_err(|e| err(e, p))?;
        }
        delete_file(p)
    }

    /// Devuelve «nombre en la Papelera» + `\n` + de dónde salió (el motor manda a la Papelera lo
    /// apartado con un nombre temporal; `untrash` lo lleva después a donde corresponde). Si el
    /// sistema no lo mandó a la Papelera, es un error: «se borró definitivamente» si ya no está.
    fn trash(&self, p: &Path) -> Result<String, CopyError> {
        let _com = Com::new();
        match super::win32::recycle_one(p) {
            Ok(Some(in_bin)) => Ok(format!("{in_bin}\n{}", p.display())),
            Ok(None) | Err(_) if attr_data(p).is_err() => {
                Err(CopyError::new(ErrKind::Other("Se borró definitivamente (no entraba en la Papelera)".into()), p))
            }
            Ok(None) => Err(CopyError::new(ErrKind::Other("No se mandó a la Papelera".into()), p)),
            Err(e) => Err(err(e, p)),
        }
    }

    fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError> {
        if attr_data(original).is_ok() {
            return Err(CopyError::new(ErrKind::Exists, original));
        }
        let (in_bin, from) = in_trash.split_once('\n').map_or((in_trash, None), |(a, b)| (a, Some(Path::new(b))));
        let _com = Com::new();
        // «Restaurar» del Shell: vuelve a donde estaba al mandarlo; si era un nombre temporal, de
        // ahí al lugar pedido (sin pisar).
        super::win32::restore_from_recycle_bin(in_bin).map_err(|e| err(e, original))?;
        match from {
            Some(from) if from != original => {
                // El verbo corre sin asincronía (`CMIC_MASK_NOASYNC`), pero algunas extensiones
                // de la Papelera terminan después: se espera un poco a que aparezca.
                for _ in 0..50 {
                    if attr_data(from).is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                self.rename_noreplace(from, original)
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use xdir_core::ops::Undo;
    use xdir_core::transfer::{Answer, Choice, Control, JobOutcome, JobSpec, Update, run};

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-win-prims-{name}-{}", std::process::id()));
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
        WinCopyPrims
            .copy_file(&src, &dst, &mut |n| {
                seen.push(n);
                Flow::Continue
            })
            .unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&src).unwrap());
        assert!(!seen.is_empty() && seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        let other = dir.join("cortada.bin");
        let e = WinCopyPrims.copy_file(&src, &other, &mut |_| Flow::Stop).unwrap_err();
        assert_eq!(e.kind, ErrKind::Cancelled);
        assert!(std::fs::symlink_metadata(&other).is_err(), "quedó el archivo a medio copiar");
    }

    #[test]
    fn prims_copy_refuses_existing() {
        let dir = tmp("existe");
        let (src, dst) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&src, b"nuevo").unwrap();
        std::fs::write(&dst, b"viejo").unwrap();
        let e = WinCopyPrims.copy_file(&src, &dst, &mut |_| Flow::Continue).unwrap_err();
        assert_eq!(e.kind, ErrKind::Exists);
        assert_eq!(std::fs::read(&dst).unwrap(), b"viejo");
    }

    #[test]
    fn prims_rename_noreplace_refuses() {
        let dir = tmp("renombrar");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert_eq!(WinCopyPrims.rename_noreplace(&a, &b).unwrap_err().kind, ErrKind::Exists);
        assert_eq!((std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()), (b"a".to_vec(), b"b".to_vec()));
        // Solo mayúsculas (NTFS no las distingue): se puede.
        WinCopyPrims.rename_noreplace(&a, &dir.join("A.txt")).unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name() == "A.txt"));
    }

    #[test]
    fn prims_replace_is_atomic() {
        let dir = tmp("reemplazar");
        let (tmp_file, dst) = (dir.join(".xdir-1.tmp"), dir.join("informe.pdf"));
        std::fs::write(&tmp_file, b"nuevo").unwrap();
        std::fs::write(&dst, b"viejo").unwrap();
        WinCopyPrims.replace(&tmp_file, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"nuevo");
        assert!(std::fs::symlink_metadata(&tmp_file).is_err());
    }

    #[test]
    fn prims_symlink_copied_as_link() {
        let dir = tmp("enlace");
        std::fs::write(dir.join("destino.txt"), b"x").unwrap();
        // Crear enlaces pide el modo de desarrollador o permisos de administrador.
        if std::os::windows::fs::symlink_file(dir.join("destino.txt"), dir.join("l")).is_err() {
            return;
        }
        assert_eq!(WinCopyPrims.stat(&dir.join("l")).unwrap().kind, ItemKind::Symlink);
        WinCopyPrims.copy_symlink(&dir.join("l"), &dir.join("l2")).unwrap();
        assert_eq!(std::fs::read_link(dir.join("l2")).unwrap(), dir.join("destino.txt"));
    }

    #[test]
    fn prims_finish_dir_keeps_mtime() {
        let dir = tmp("carpeta");
        let (src, dst) = (dir.join("orig"), dir.join("copia"));
        std::fs::create_dir(&src).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_200_000_000);
        set_dir_modified(&src, old);
        WinCopyPrims.create_dir(&dst).unwrap();
        assert_eq!(WinCopyPrims.create_dir(&dst).unwrap_err().kind, ErrKind::Exists);
        std::fs::write(dst.join("adentro.txt"), b"x").unwrap();
        WinCopyPrims.finish_dir(&dst, &src).unwrap();
        assert_eq!(std::fs::metadata(&dst).unwrap().modified().unwrap(), old);
        assert!(WinCopyPrims.remove(&dst).is_err());
    }

    /// Fecha de modificación de una carpeta (se abre con `FILE_FLAG_BACKUP_SEMANTICS`).
    fn set_dir_modified(p: &Path, t: std::time::SystemTime) {
        use std::os::windows::fs::OpenOptionsExt;
        let f = std::fs::OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES.0).custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0).open(p).unwrap();
        f.set_modified(t).unwrap();
    }

    #[test]
    fn prims_long_path() {
        let dir = tmp("larga");
        let mut deep = dir.clone();
        while deep.as_os_str().len() < 300 {
            deep = deep.join("carpeta con un nombre bastante largo");
        }
        WinCopyPrims.create_dir(&deep).unwrap_err(); // falta la de arriba
        let mut cur = dir.clone();
        for c in deep.strip_prefix(&dir).unwrap().components() {
            cur = cur.join(c);
            WinCopyPrims.create_dir(&cur).unwrap();
        }
        assert!(deep.as_os_str().len() >= 300);
        let (src, dst) = (deep.join("a.txt"), deep.join("b.txt"));
        std::fs::write(&src, b"lejos").unwrap();
        WinCopyPrims.copy_file(&src, &dst, &mut |_| Flow::Continue).unwrap();
        assert_eq!(WinCopyPrims.stat(&dst).unwrap().size, 5);
        WinCopyPrims.rename_noreplace(&dst, &deep.join("c.txt")).unwrap();
        WinCopyPrims.remove(&deep.join("c.txt")).unwrap();
        assert!(WinCopyPrims.volume(&deep).is_some());
    }

    #[test]
    fn prims_err_kind_mapping() {
        assert_eq!(err_kind(ERROR_SHARING_VIOLATION.0), ErrKind::InUse);
        assert_eq!(err_kind(ERROR_LOCK_VIOLATION.0), ErrKind::InUse);
        assert_eq!(err_kind(ERROR_ACCESS_DENIED.0), ErrKind::Denied);
        assert_eq!(err_kind(ERROR_FILE_NOT_FOUND.0), ErrKind::NotFound);
        assert_eq!(err_kind(ERROR_PATH_NOT_FOUND.0), ErrKind::NotFound);
        assert_eq!(err_kind(ERROR_DISK_FULL.0), ErrKind::DiskFull);
        assert_eq!(err_kind(ERROR_HANDLE_DISK_FULL.0), ErrKind::DiskFull);
        assert_eq!(err_kind(ERROR_FILE_TOO_LARGE.0), ErrKind::TooBig);
        assert_eq!(err_kind(ERROR_FILE_EXISTS.0), ErrKind::Exists);
        assert_eq!(err_kind(ERROR_ALREADY_EXISTS.0), ErrKind::Exists);
        assert_eq!(err_kind(ERROR_REQUEST_ABORTED.0), ErrKind::Cancelled);
        assert_eq!(err_kind(ERROR_NOT_SAME_DEVICE.0), ErrKind::CrossDevice);
        assert!(matches!(err_kind(1117), ErrKind::Other(_)));
        // Los HRESULT de Win32 se traducen por su código.
        let e = windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(ERROR_SHARING_VIOLATION.0));
        assert_eq!(err(e, Path::new("x")).kind, ErrKind::InUse);
    }

    #[test]
    fn long_paths_get_the_prefix() {
        let s = |p: &str| String::from_utf16_lossy(&long_path(Path::new(p))).trim_end_matches('\0').to_string();
        assert_eq!(s(r"C:\a\b"), r"\\?\C:\a\b");
        assert_eq!(s(r"\\srv\share\x"), r"\\?\UNC\srv\share\x");
        assert_eq!(s(r"\\?\C:\a"), r"\\?\C:\a");
    }

    /// Las primitivas reales, sin Papelera (para no tocar la del usuario) y, si se pide, con la
    /// carpeta `other` como si fuera otro disco.
    struct Real {
        other: Option<PathBuf>,
    }

    impl CopyPrims for Real {
        fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
            WinCopyPrims.stat(p)
        }
        fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
            WinCopyPrims.read_dir(p)
        }
        fn volume(&self, p: &Path) -> Option<VolumeInfo> {
            let mut v = WinCopyPrims.volume(p)?;
            v.has_trash = false;
            if self.other.as_ref().is_some_and(|o| p.starts_with(o)) {
                v.id = "otro".into();
            }
            Some(v)
        }
        fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
            WinCopyPrims.copy_file(src, dst, progress)
        }
        fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
            WinCopyPrims.copy_symlink(src, dst)
        }
        fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
            WinCopyPrims.create_dir(dst)
        }
        fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
            WinCopyPrims.finish_dir(dst, like)
        }
        fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
            WinCopyPrims.rename_noreplace(from, to)
        }
        fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
            WinCopyPrims.replace(from, to)
        }
        fn remove(&self, p: &Path) -> Result<(), CopyError> {
            WinCopyPrims.remove(p)
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
        std::fs::create_dir_all(src.join("Fotos").join("sub")).unwrap();
        std::fs::write(src.join("Fotos").join("a.jpg"), big(3 << 20)).unwrap();
        std::fs::write(src.join("Fotos").join("sub").join("b.jpg"), b"bbb").unwrap();
        std::fs::create_dir(&dst).unwrap();
        let prims = Real { other: None };
        let out = go(&prims, JobSpec::Copy { sources: vec![src.join("Fotos")], dest: dst.clone() }, &[], &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(std::fs::read(dst.join("Fotos").join("a.jpg")).unwrap(), big(3 << 20));
        assert_eq!(std::fs::read(dst.join("Fotos").join("sub").join("b.jpg")).unwrap(), b"bbb");
        assert_eq!(out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![dst.join("Fotos")])])));
        // «B.JPG» sobre «b.jpg»: en NTFS es conflicto.
        std::fs::write(src.join("B.JPG"), b"otra").unwrap();
        let sub = dst.join("Fotos").join("sub");
        let out = go(&prims, JobSpec::Copy { sources: vec![src.join("B.JPG")], dest: sub.clone() }, &[Answer { choice: Choice::KeepBoth, all: false }], &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty());
        assert_eq!(std::fs::read(sub.join("b.jpg")).unwrap(), b"bbb");
        assert_eq!(std::fs::read(sub.join("B 2.JPG")).unwrap(), b"otra");
    }

    #[test]
    fn engine_real_replace_and_cancel_keep_original() {
        let dir = tmp("motor-reemplazar");
        std::fs::create_dir_all(dir.join("s")).unwrap();
        std::fs::create_dir_all(dir.join("d")).unwrap();
        std::fs::write(dir.join("s").join("informe.pdf"), big(20 << 20)).unwrap();
        std::fs::write(dir.join("d").join("informe.pdf"), b"viejo").unwrap();
        let prims = Real { other: None };
        let spec = || JobSpec::Copy { sources: vec![dir.join("s").join("informe.pdf")], dest: dir.join("d") };
        let replace = [Answer { choice: Choice::Replace, all: false }];
        let ctl = Control::new();
        let c = ctl.clone();
        let out = go(&prims, spec(), &replace, &ctl, &mut |u| {
            if matches!(u, Update::Progress { bytes, .. } if *bytes > 0) {
                c.cancel();
            }
        });
        if out.cancelled {
            assert_eq!(std::fs::read(dir.join("d").join("informe.pdf")).unwrap(), b"viejo");
        }
        no_tmp(&dir.join("d"));
        let out = go(&prims, spec(), &replace, &Control::new(), &mut |_| {});
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(std::fs::read(dir.join("d").join("informe.pdf")).unwrap(), big(20 << 20));
        no_tmp(&dir.join("d"));
    }

    #[test]
    fn engine_real_move_between_volumes() {
        let dir = tmp("motor-mover");
        let (src, other) = (dir.join("interno"), dir.join("usb"));
        std::fs::create_dir_all(src.join("Dir").join("sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(src.join("Dir").join("a.txt"), b"a").unwrap();
        std::fs::write(src.join("Dir").join("sub").join("b.txt"), b"b").unwrap();
        // Abierto sin compartir: «Lo está usando otro programa».
        let busy = src.join("Dir").join("sub").join("abierto.txt");
        std::fs::write(&busy, b"c").unwrap();
        let lock = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(0).open(&busy).unwrap()
        };
        let prims = Real { other: Some(other.clone()) };
        let out = go(&prims, JobSpec::Move { sources: vec![src.join("Dir")], dest: other.clone() }, &[], &Control::new(), &mut |_| {});
        drop(lock);
        assert_eq!(std::fs::read(other.join("Dir").join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(other.join("Dir").join("sub").join("b.txt")).unwrap(), b"b");
        assert!(!src.join("Dir").join("a.txt").exists());
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert_eq!(out.errors[0].message, "Lo está usando otro programa");
        assert_eq!(std::fs::read(&busy).unwrap(), b"c");
        assert!(!other.join("Dir").join("sub").join("abierto.txt").exists());
    }
}

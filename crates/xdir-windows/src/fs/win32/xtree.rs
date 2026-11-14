//! xtree en Windows: listado de carpetas con una sola llamada nativa por bloque
//! (`NtQueryDirectoryFile`), escaneo rápido de discos NTFS leyendo la MFT, el ayudante elevado
//! (`xtree_permisos`) y la lista de unidades.

use std::cell::RefCell;
use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError, HANDLE, SetLastError, WAIT_TIMEOUT, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetCompressedFileSizeW,
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{PCWSTR, w};

use super::mft;
use crate::fs::xtree_temp::{helper_file_name, is_helper_file};
use crate::platform::{Handle, HandleExt};
use crate::xtree::colors::fmt_size;
use crate::xtree::scan::{DirListing, DriveEntry, FastScan, FileEntry, ListError, Lister, Progress};
use crate::xtree::tree::{Node, decode, encode};

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Archivos que solo existen en la nube (OneDrive y similares): no ocupan lugar en el disco.
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;
const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
const CLUSTER: i64 = 4096;
/// FileIdFullDirectoryInformation
const FILE_ID_FULL_DIR_INFO: u32 = 38;
/// Buffer de `NtQueryDirectoryFile` (128 KiB), uno por hilo de trabajo.
const LIST_BUF_WORDS: usize = 128 * 1024 / 8;

/// Argumento con el que xdir se relanza como administrador para leer la MFT (`xtree_permisos`).
pub const XTREE_HELPER_ARG: &str = "--xtree-mft";
/// Pedidos al ayudante elevado de este proceso: cada uno usa su propio archivo temporal.
static NEXT_HELPER_FILE: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
struct IoStatus {
    status: isize,
    info: usize,
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryDirectoryFile(
        file: *mut c_void,
        event: *mut c_void,
        apc: *mut c_void,
        apc_ctx: *mut c_void,
        iosb: *mut IoStatus,
        buf: *mut c_void,
        len: u32,
        class: u32,
        single: u8,
        name: *mut c_void,
        restart: u8,
    ) -> i32;
}

thread_local! {
    /// Buffer del listado: cada hilo del escáner reusa el suyo en todas sus carpetas.
    static LIST_BUF: RefCell<Vec<u64>> = RefCell::new(vec![0u64; LIST_BUF_WORDS]);
}

// ---------- listado ----------

/// Ruta ancha con prefijo `\\?\` (rutas largas), sin barra final ni NUL.
fn long_path(path: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    while wide.last() == Some(&(b'\\' as u16)) {
        wide.pop();
    }
    let bs = b'\\' as u16;
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    if wide.starts_with(&prefix) {
        wide
    } else if wide.len() >= 2 && wide[0] == bs && wide[1] == bs {
        let mut out: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
        out.extend_from_slice(&wide[2..]);
        out
    } else {
        let mut out = prefix;
        out.extend_from_slice(&wide);
        out
    }
}

/// Espacio realmente ocupado (archivos comprimidos o dispersos), preguntándoselo al sistema.
fn disk_size(dir: &[u16], name: &[u16]) -> Option<u64> {
    let mut p = Vec::with_capacity(dir.len() + name.len() + 2);
    p.extend_from_slice(dir);
    p.push(b'\\' as u16);
    p.extend_from_slice(name);
    p.push(0);
    unsafe {
        SetLastError(WIN32_ERROR(0));
        let mut high = 0u32;
        let low = GetCompressedFileSizeW(PCWSTR(p.as_ptr()), Some(&mut high));
        if low == u32::MAX && GetLastError().0 != 0 {
            return None;
        }
        Some(((high as u64) << 32) | low as u64)
    }
}

/// Lista una carpeta (`path` ancha, con `\\?\` y sin barra final).
fn list_dir(path: &[u16], buf: &mut [u64]) -> Result<DirListing, ListError> {
    let mut wide = path.to_vec();
    wide.push(b'\\' as u16);
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            1, // FILE_LIST_DIRECTORY
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|e| if e.code() == ERROR_ACCESS_DENIED.to_hresult() { ListError::Denied } else { ListError::Other })?;

    let mut out = DirListing::default();
    let mut restart = 1u8;
    loop {
        let mut iosb = IoStatus { status: 0, info: 0 };
        let status = unsafe {
            NtQueryDirectoryFile(
                handle.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut iosb,
                buf.as_mut_ptr() as *mut c_void,
                (buf.len() * 8) as u32,
                FILE_ID_FULL_DIR_INFO,
                0,
                std::ptr::null_mut(),
                restart,
            )
        };
        restart = 0;
        if status < 0 {
            break; // STATUS_NO_MORE_FILES u otro error: se terminó la carpeta
        }
        let base = buf.as_ptr() as *const u8;
        let mut off = 0usize;
        loop {
            // Campos de FILE_ID_FULL_DIR_INFORMATION (desplazamientos fijos del formato nativo).
            let (next, eof, alloc, attrs, name_len, file_id, name) = unsafe {
                let p = base.add(off);
                let rd32 = |o: usize| std::ptr::read_unaligned(p.add(o) as *const u32);
                let rd64 = |o: usize| std::ptr::read_unaligned(p.add(o) as *const i64);
                let len = rd32(60) as usize / 2;
                (rd32(0), rd64(40), rd64(48), rd32(56), len, rd64(72), std::slice::from_raw_parts(p.add(80) as *const u16, len))
            };
            let dots = name_len == 1 && name[0] == b'.' as u16 || name_len == 2 && name[0] == b'.' as u16 && name[1] == b'.' as u16;
            if !dots {
                if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    // Junctions y enlaces simbólicos no se siguen: contarían dos veces el mismo espacio.
                    if attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                        out.dirs.push(OsString::from_wide(name));
                    }
                } else if attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) == 0 {
                    let mut size = alloc.max(0) as u64;
                    // El listado a veces trae 0 en archivos grandes, comprimidos o dispersos: se pregunta aparte.
                    if size == 0 && eof > 0 && (eof > CLUSTER || attrs & (FILE_ATTRIBUTE_SPARSE_FILE | FILE_ATTRIBUTE_COMPRESSED) != 0) {
                        size = disk_size(path, name).unwrap_or(eof as u64);
                    }
                    // Los vacíos los descarta el núcleo; el id solo sirve si existe (enlaces duros).
                    if size > 0 {
                        let id = (file_id != 0).then_some(file_id as u64);
                        out.files.push(FileEntry { name: String::from_utf16_lossy(name).into(), size, id });
                    }
                }
            }
            if next == 0 {
                break;
            }
            off += next as usize;
        }
    }
    let _ = unsafe { CloseHandle(handle) };
    Ok(out)
}

struct WinLister;

impl Lister for WinLister {
    fn list(&self, path: &Path) -> Result<DirListing, ListError> {
        let wide = long_path(path);
        LIST_BUF.with(|b| list_dir(&wide, &mut b.borrow_mut()))
    }
}

/// Listador de carpetas para el escáner de xtree.
pub fn xtree_lister(_root: &Path) -> impl Lister + 'static {
    WinLister
}

// ---------- MFT ----------

/// `C:` si `text` es la raíz de una unidad (sin barra final).
fn volume_drive(text: &str) -> Option<String> {
    let b = text.as_bytes();
    (b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| text.to_ascii_uppercase())
}

/// Unidad (`C:`) si `path` es la raíz de una unidad.
fn root_drive(path: &Path) -> Option<String> {
    volume_drive(path.to_string_lossy().trim_end_matches('\\'))
}

/// El escaneo por MFT solo existe en NTFS; los pendrives suelen ser FAT32 o exFAT.
fn is_ntfs(drive: &str) -> bool {
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(Some(0)).collect();
    let mut fs = [0u16; 32];
    let ok = unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), None, None, None, None, Some(&mut fs)) }.is_ok();
    ok && utf16_str(&fs).eq_ignore_ascii_case("NTFS")
}

/// Disco completo en NTFS y con permisos: se lee la MFT directamente (mucho más rápido).
pub fn xtree_fast_scan(root: &Path, progress: &Progress, cancel: &AtomicBool) -> FastScan {
    let Some(drive) = root_drive(root).filter(|d| is_ntfs(d)) else { return FastScan::NotApplicable };
    progress.phase.store(1, Ordering::Relaxed);
    match mft::scan(&drive, &format!("{drive}\\"), progress, cancel) {
        Ok(arena) => FastScan::Done(arena),
        Err(mft::Fail::Denied) => FastScan::NeedsAdmin,
        Err(mft::Fail::Other) => FastScan::NotApplicable,
    }
}

// ---------- ayudante elevado (`xtree_permisos`) ----------

/// Tecla A: relanza xdir como administrador (Windows pide confirmación, delante de la ventana de
/// xdir) con `--xtree-mft C: %TEMP%\xdir-xtree-<pid>-<n>.bin <pid>`; ese proceso lee la MFT, deja
/// el árbol en el archivo y termina. Acá se espera, se lee el árbol y se borra el archivo.
/// Si se cancela (F5, otro análisis o xdir se cierra) se termina el ayudante; si Windows no lo
/// deja, el ayudante igual se corta solo cuando xdir se cierra (vigila el `<pid>`).
pub fn xtree_elevate(hwnd: Handle, root: &Path, _progress: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>> {
    let drive = root_drive(root)?;
    let exe = std::env::current_exe().ok()?;
    let pid = std::process::id();
    let n = NEXT_HELPER_FILE.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(helper_file_name(pid, n));
    let _ = std::fs::remove_file(&file);
    let params = format!("{XTREE_HELPER_ARG} {drive} \"{}\" {pid}", file.display());
    let exe_w: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let params_w: Vec<u16> = params.encode_utf16().chain(Some(0)).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        // Con la ventana de xdir como dueña, la confirmación de Windows sale adelante.
        hwnd: hwnd.hwnd(),
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe_w.as_ptr()),
        lpParameters: PCWSTR(params_w.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // ShellExecuteEx puede usar extensiones del Shell: necesita COM en este hilo.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok();
    let launched = unsafe { ShellExecuteExW(&mut info) }.is_ok();
    if com {
        unsafe { CoUninitialize() };
    }
    // Sin proceso: el usuario dijo que no a la confirmación o no se pudo lanzar.
    if !launched || info.hProcess.is_invalid() {
        return None;
    }
    let mut code = 1u32;
    let finished = loop {
        if cancel.load(Ordering::Relaxed) {
            break false;
        }
        if unsafe { WaitForSingleObject(info.hProcess, 200) } != WAIT_TIMEOUT {
            break unsafe { GetExitCodeProcess(info.hProcess, &mut code) }.is_ok();
        }
    };
    if !finished {
        // Ya no hace falta: se termina el ayudante (si Windows no lo permite, el error se ignora).
        // Un hilo aparte espera a que salga y borra el archivo que haya podido dejar.
        let _ = unsafe { TerminateProcess(info.hProcess, 1) };
        let process = info.hProcess.0 as usize;
        let _ = std::thread::Builder::new().spawn(move || {
            let process = HANDLE(process as *mut c_void);
            unsafe { WaitForSingleObject(process, INFINITE) };
            let _ = unsafe { CloseHandle(process) };
            let _ = std::fs::remove_file(&file);
        });
        return None;
    }
    let _ = unsafe { CloseHandle(info.hProcess) };
    let bytes = std::fs::read(&file).ok();
    let _ = std::fs::remove_file(&file);
    if code != 0 {
        return None;
    }
    decode(&bytes?)
}

/// Proceso elevado (`xdir --xtree-mft <unidad> <archivo> <pid>`): lee la MFT de la unidad y guarda
/// el árbol en el archivo. `args` son los argumentos que siguen a `--xtree-mft`; `<pid>` es el xdir
/// que lo pidió: si se cierra, la lectura se corta y no se escribe nada. El archivo solo puede
/// llamarse `xdir-xtree-<dígitos>[-<dígitos>].bin`. Devuelve el código de salida: 0 bien, 2 sin
/// permisos, 1 cualquier otro error (o xdir se cerró).
pub fn xtree_helper_main(args: &[String]) -> i32 {
    let [drive, file, pid] = args else { return 1 };
    // Proceso de administrador: no escribe donde se le pida, solo el temporal de xtree.
    if !is_helper_file(file) {
        return 1;
    }
    let Ok(pid) = pid.parse::<u32>() else { return 1 };
    let Some(drive) = volume_drive(drive.trim_end_matches('\\')).filter(|d| is_ntfs(d)) else { return 1 };
    // Si xdir ya no existe no hay a quién devolverle nada.
    let Ok(parent) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else { return 1 };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let (cancel, parent) = (cancel.clone(), parent.0 as usize);
        let _ = std::thread::Builder::new().spawn(move || {
            unsafe { WaitForSingleObject(HANDLE(parent as *mut c_void), INFINITE) };
            cancel.store(true, Ordering::Relaxed);
        });
    }
    let progress = Progress::default();
    match mft::scan(&drive, &format!("{drive}\\"), &progress, &cancel) {
        Ok(_) if cancel.load(Ordering::Relaxed) => 1,
        Ok(arena) => {
            // Proceso de administrador escribiendo en %TEMP% del usuario: solo un archivo nuevo.
            // Si ya existe algo con ese nombre (archivo o enlace puesto mientras se esperaba el
            // UAC) no se abre, para no pisar otro archivo con permisos elevados.
            let written = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file)
                .and_then(|mut f| std::io::Write::write_all(&mut f, &encode(&arena)));
            if written.is_ok() { 0 } else { 1 }
        }
        Err(mft::Fail::Denied) => 2,
        Err(mft::Fail::Other) => 1,
    }
}

// ---------- unidades y volumen ----------

fn utf16_str(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Unidades conectadas ahora mismo (discos, pendrives y unidades de red); se vuelve a leer al abrir el menú.
pub fn xtree_drives() -> Vec<DriveEntry> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
        let ty = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
        let kind = match ty {
            2 => "Unidad extraíble",
            3 => "Disco local",
            4 => "Unidad de red",
            6 => "Disco RAM",
            _ => continue, // lectores de CD sin disco, etc.
        };
        let (mut label, mut fs) = (String::new(), String::new());
        // Una unidad de red desconectada puede tardar en responder: no se consulta.
        if ty != 4 {
            let (mut vol, mut fsb) = ([0u16; 64], [0u16; 32]);
            let ok = unsafe { GetVolumeInformationW(PCWSTR(wide.as_ptr()), Some(&mut vol), None, None, None, Some(&mut fsb)) }.is_ok();
            if !ok {
                continue; // sin medio o sin formato
            }
            label = utf16_str(&vol);
            fs = utf16_str(&fsb);
        }
        let name = if label.is_empty() { kind.to_string() } else { label };
        let mut detail = fs;
        if ty != 4
            && let Some((total, _)) = volume_space(Path::new(&root))
        {
            detail = if detail.is_empty() { fmt_size(total) } else { format!("{detail} \u{B7} {}", fmt_size(total)) };
        }
        out.push(DriveEntry { path: PathBuf::from(root), label: format!("[{letter}:] {name}"), detail });
    }
    out
}

/// Espacio total y libre del volumen que contiene `path`.
pub fn volume_space(path: &Path) -> Option<(u64, u64)> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    while wide.last() == Some(&(b'\\' as u16)) {
        wide.pop();
    }
    wide.push(b'\\' as u16);
    wide.push(0);
    let (mut total, mut free) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), None, Some(&mut total), Some(&mut free)) }.ok()?;
    (total > 0).then_some((total, free))
}

/// `true` si `path` es la raíz de una unidad (`C:\`).
pub fn whole_volume(path: &Path) -> bool {
    root_drive(path).is_some()
}

/// Carpeta que se analiza al abrir xtree sin elegir nada: la unidad del sistema.
pub fn xtree_default_root() -> PathBuf {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    PathBuf::from(format!("{drive}\\"))
}

/// En Windows lo que falta es leer la MFT, que requiere administrador (`xtree_permisos`).
pub fn xtree_permission_issue(p: &Progress) -> bool {
    p.need_admin.load(Ordering::Relaxed)
}

/// Aviso de la barra de datos cuando el escaneo rápido necesita administrador (`xtree_permisos`).
pub const XTREE_PERMISSION_HINT: &str = "Escaneo rápido de la MFT: requiere administrador ({teclas})";

/// Texto de la barra de datos y de estado mientras corre el escaneo con permisos (`xtree_permisos`).
pub const XTREE_ELEVATING_TEXT: &str = "Leyendo la tabla de archivos (MFT) como administrador\u{2026}";

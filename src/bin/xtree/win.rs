//! xtree: mapa de uso de disco (treemap) al estilo WizTree. Programa aparte que lanza xdir.
//! Uso: xtree.exe [carpeta]
//!
//! Escáner recursivo con FindFirstFileEx en un hilo (sin leer la MFT), treemap "squarified" con
//! Direct2D, tabla de lo más pesado de la carpeta actual y tarjeta de detalle con acciones.
//! Clic selecciona; doble clic entra en la carpeta; clic derecho o Retroceso suben; F5 reescanea.

use std::cell::Cell as StdCell;
use std::ffi::c_void;
use std::collections::{HashSet, VecDeque};
use std::os::windows::ffi::OsStrExt;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::{BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW, ShellExecuteW};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use windows::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D_SIZE_U, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ValidateRect};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetCompressedFileSizeW,
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING,
};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_BACK, VK_ESCAPE, VK_F5, VK_T};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
    GetMessageW, IDC_ARROW, KillTimer, LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOW, SetTimer,
    SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_SIZE, WM_TIMER, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, PWSTR, Result, w};
use windows_numerics::Vector2;

type Rect = D2D_RECT_F;

const WM_SCAN_DONE: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;

// Medidas (DIPs).
const MARGIN: f32 = 8.0;
const TOP_H: f32 = 40.0;
const STATUS_H: f32 = 58.0;
const LIST_W: f32 = 372.0;
const TABLE_H: f32 = 34.0;
const ROW_H: f32 = 32.0;
const CARD_H: f32 = 76.0;
const LEGEND_H: f32 = 40.0;
const GAP: f32 = 6.0;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Archivos que solo existen en la nube (OneDrive y similares): no ocupan lugar en el disco.
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;

static FILES: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
/// 0: recorriendo carpetas; 1: leyendo la MFT; 2: armando el Ã¡rbol a partir de la MFT.
static PHASE: AtomicU32 = AtomicU32::new(0);
static MFT_DONE: AtomicU64 = AtomicU64::new(0);
static MFT_TOTAL: AtomicU64 = AtomicU64::new(0);
/// El escaneo rÃ¡pido de un disco completo necesita administrador y no se pudo usar.
static NEED_ADMIN: AtomicBool = AtomicBool::new(false);

mod mft;

// ---------- escáner ----------
//
// Un grupo de hilos toma carpetas de una cola (anchura primero: el mapa crece desde los niveles
// altos mientras se escanea). Cada hilo lista su carpeta con una sola llamada nativa por bloque
// (NtQueryDirectoryFile, que trae tamaño lógico, espacio asignado e id de archivo) y después inserta
// el resultado en el árbol compartido bajo un bloqueo de escritura muy corto.

struct Node {
    name: Box<str>,
    parent: u32,
    size: u64,
    is_dir: bool,
    /// Grupo "… N archivos pequeños": suma de los archivos que no entran como nodos propios.
    small: bool,
    children: Vec<u32>,
}

const NO_PARENT: u32 = u32::MAX;
/// Archivos que se guardan como nodos propios por carpeta; el resto se suma en un solo nodo.
const MAX_FILES_PER_DIR: usize = 1500;
const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
const CLUSTER: i64 = 4096;
const ID_SHARDS: usize = 64;
/// FileIdFullDirectoryInformation
const FILE_ID_FULL_DIR_INFO: u32 = 38;

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

type Tree = Arc<RwLock<Vec<Node>>>;

struct Job {
    node: u32,
    /// Ruta ancha de la carpeta, con prefijo `\\?\`, sin barra final ni NUL.
    path: Vec<u16>,
}

struct Ctx<'a> {
    tree: &'a RwLock<Vec<Node>>,
    queue: Mutex<(VecDeque<Job>, usize)>,
    cv: Condvar,
    /// Ids de archivo ya contados: un archivo con varios enlaces duros ocupa espacio una sola vez.
    seen: Vec<Mutex<HashSet<u64>>>,
    cancel: &'a AtomicBool,
}

impl Ctx<'_> {
    fn first_time(&self, id: u64) -> bool {
        let shard = ((id ^ (id >> 20)) as usize) % ID_SHARDS;
        self.seen[shard].lock().unwrap().insert(id)
    }
}

#[derive(Default)]
struct Listing {
    dirs: Vec<Vec<u16>>,
    files: Vec<(Box<str>, u64)>,
    small_size: u64,
    small_count: u32,
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

/// Lista una carpeta. `None` si no se pudo abrir (sin permisos, etc.).
fn list_dir(path: &[u16], ctx: &Ctx, buf: &mut [u64]) -> Option<Listing> {
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
    .ok()?;

    let mut out = Listing::default();
    let mut files: Vec<(Box<str>, u64)> = Vec::new();
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
                        out.dirs.push(name.to_vec());
                    }
                } else if attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) == 0 {
                    FILES.fetch_add(1, Ordering::Relaxed);
                    let mut size = alloc.max(0) as u64;
                    // El listado a veces trae 0 en archivos grandes, comprimidos o dispersos: se pregunta aparte.
                    if size == 0 && eof > 0 && (eof > CLUSTER || attrs & (FILE_ATTRIBUTE_SPARSE_FILE | FILE_ATTRIBUTE_COMPRESSED) != 0) {
                        size = disk_size(path, name).unwrap_or(eof as u64);
                    }
                    if size > 0 && file_id != 0 && !ctx.first_time(file_id as u64) {
                        size = 0; // otro enlace duro del mismo archivo ya se contó
                    }
                    if size > 0 {
                        BYTES.fetch_add(size, Ordering::Relaxed);
                        files.push((String::from_utf16_lossy(name).into(), size));
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

    // Solo los más pesados entran como nodos propios; el resto se agrupa.
    if files.len() > MAX_FILES_PER_DIR {
        files.select_nth_unstable_by(MAX_FILES_PER_DIR, |a, b| b.1.cmp(&a.1));
        for (_, size) in files.drain(MAX_FILES_PER_DIR..) {
            out.small_size += size;
            out.small_count += 1;
        }
    }
    out.files = files;
    Some(out)
}

/// Agrega el listado al árbol compartido y encola las subcarpetas.
fn insert_listing(ctx: &Ctx, job: &Job, l: Listing) {
    let mut nodes: Vec<Node> = Vec::with_capacity(l.dirs.len() + l.files.len() + 1);
    for d in &l.dirs {
        nodes.push(Node { name: String::from_utf16_lossy(d).into(), parent: job.node, size: 0, is_dir: true, small: false, children: Vec::new() });
    }
    let mut total = l.small_size;
    for (name, size) in l.files {
        total += size;
        nodes.push(Node { name, parent: job.node, size, is_dir: false, small: false, children: Vec::new() });
    }
    if l.small_count > 0 {
        let name = format!("\u{2026} {} archivos pequeños", l.small_count);
        nodes.push(Node { name: name.into(), parent: job.node, size: l.small_size, is_dir: false, small: true, children: Vec::new() });
    }
    let count = nodes.len() as u32;
    let base = {
        let mut t = ctx.tree.write().unwrap();
        let base = t.len() as u32;
        t.extend(nodes);
        t[job.node as usize].children = (base..base + count).collect();
        // El tamaño sube por toda la cadena de padres: el árbol siempre está al día.
        let mut cur = job.node;
        loop {
            let n = &mut t[cur as usize];
            n.size += total;
            if n.parent == NO_PARENT {
                break;
            }
            cur = n.parent;
        }
        base
    };
    if !l.dirs.is_empty() {
        let mut q = ctx.queue.lock().unwrap();
        for (i, d) in l.dirs.iter().enumerate() {
            let mut path = Vec::with_capacity(job.path.len() + 1 + d.len());
            path.extend_from_slice(&job.path);
            path.push(b'\\' as u16);
            path.extend_from_slice(d);
            q.0.push_back(Job { node: base + i as u32, path });
        }
    }
}

fn worker(ctx: &Ctx) {
    let mut buf = vec![0u64; 128 * 1024 / 8];
    loop {
        let job = {
            let mut st = ctx.queue.lock().unwrap();
            loop {
                if ctx.cancel.load(Ordering::Relaxed) {
                    ctx.cv.notify_all();
                    return;
                }
                if let Some(j) = st.0.pop_front() {
                    st.1 += 1;
                    break j;
                }
                if st.1 == 0 {
                    ctx.cv.notify_all();
                    return;
                }
                st = ctx.cv.wait_timeout(st, std::time::Duration::from_millis(100)).unwrap().0;
            }
        };
        // Un fallo en una carpeta no debe colgar todo el análisis.
        let listing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list_dir(&job.path, ctx, &mut buf))).ok().flatten();
        if let Some(l) = listing {
            insert_listing(ctx, &job, l);
        }
        ctx.queue.lock().unwrap().1 -= 1;
        ctx.cv.notify_all();
    }
}

/// `C:` si `text` es la raÃ­z de una unidad (sin barra final).
fn volume_drive(text: &str) -> Option<String> {
    let b = text.as_bytes();
    (b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| text.to_ascii_uppercase())
}

/// El escaneo por MFT solo existe en NTFS; los pendrives suelen ser FAT32 o exFAT.
fn is_ntfs(drive: &str) -> bool {
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(Some(0)).collect();
    let mut fs = [0u16; 32];
    let ok = unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), None, None, None, None, Some(&mut fs)) }.is_ok();
    let len = fs.iter().position(|&c| c == 0).unwrap_or(fs.len());
    ok && String::from_utf16_lossy(&fs[..len]).eq_ignore_ascii_case("NTFS")
}

fn long_path(text: &str) -> String {
    if text.starts_with("\\\\?\\") {
        text.to_string()
    } else if let Some(rest) = text.strip_prefix("\\\\") {
        format!("\\\\?\\UNC\\{rest}")
    } else {
        format!("\\\\?\\{text}")
    }
}

fn start_scan(hwnd: HWND, root: PathBuf, scan_id: usize, cancel: Arc<AtomicBool>, live: Tree) {
    FILES.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    NEED_ADMIN.store(false, Ordering::Relaxed);
    let hwnd = hwnd.0 as isize;
    let _ = std::thread::Builder::new().spawn(move || {
        let text = root.to_string_lossy().trim_end_matches('\\').to_string();
        let label = if text.is_empty() { "\\".to_string() } else { format!("{text}\\") };
        // Disco completo en NTFS y con permisos: se lee la MFT directamente (mucho más rápido).
        PHASE.store(0, Ordering::Relaxed);
        if let Some(drive) = volume_drive(&text).filter(|d| is_ntfs(d)) {
            PHASE.store(1, Ordering::Relaxed);
            match mft::scan(&drive, &label, &cancel) {
                Ok(arena) => {
                    if !cancel.load(Ordering::Relaxed) {
                        let boxed = Box::into_raw(Box::new(arena));
                        unsafe {
                            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_SCAN_DONE, WPARAM(scan_id), LPARAM(boxed as isize));
                        }
                    }
                    return;
                }
                Err(mft::Fail::Denied) => NEED_ADMIN.store(true, Ordering::Relaxed),
                Err(_) => {}
            }
            PHASE.store(0, Ordering::Relaxed);
            FILES.store(0, Ordering::Relaxed);
            BYTES.store(0, Ordering::Relaxed);
            if cancel.load(Ordering::Relaxed) {
                return;
            }
        }
        let wide: Vec<u16> = long_path(&text).encode_utf16().collect();
        live.write().unwrap().push(Node { name: label.into(), parent: NO_PARENT, size: 0, is_dir: true, small: false, children: Vec::new() });
        let ctx = Ctx {
            tree: &live,
            queue: Mutex::new((VecDeque::from([Job { node: 0, path: wide }]), 0)),
            cv: Condvar::new(),
            seen: (0..ID_SHARDS).map(|_| Mutex::new(HashSet::new())).collect(),
            cancel: &cancel,
        };
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 8);
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| worker(&ctx));
            }
        });
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        // Orden final: hijos de mayor a menor, para la tabla y el mapa.
        let arena = {
            let mut t = live.write().unwrap();
            for id in 0..t.len() {
                let mut kids = std::mem::take(&mut t[id].children);
                kids.sort_unstable_by_key(|&c| std::cmp::Reverse(t[c as usize].size));
                t[id].children = kids;
            }
            std::mem::take(&mut *t)
        };
        let boxed = Box::into_raw(Box::new(arena));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_SCAN_DONE, WPARAM(scan_id), LPARAM(boxed as isize));
        }
    });
}

// ---------- treemap ----------

struct Cell {
    r: Rect,
    node: u32,
    /// Hijo directo de la carpeta actual que contiene este bloque (a donde lleva un doble clic).
    top: u32,
    dir: bool,
    /// Profundidad de anidado: 0 para los hijos directos de la carpeta actual.
    depth: u8,
    /// Posición (0..1) en la rueda de colores: la hereda de su carpeta de primer nivel.
    hue: f32,
    /// Grupo de archivos pequeños.
    small: bool,
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> Rect {
    Rect { left: l, top: t, right: r, bottom: b }
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Reparte `areas` (ordenadas de mayor a menor, que suman el área de `r`) en `r` con el algoritmo
/// "squarified": filas de bloques lo más cuadrados posible.
fn squarify(areas: &[f64], r: Rect) -> Vec<Rect> {
    let n = areas.len();
    let mut out = Vec::with_capacity(n);
    let (mut x, mut y) = (r.left as f64, r.top as f64);
    let (mut w, mut h) = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
    let mut i = 0;
    while i < n {
        let short = w.min(h);
        if short <= 0.0 {
            out.resize(n, rect(x as f32, y as f32, x as f32, y as f32));
            break;
        }
        let mut end = i;
        let mut sum = 0.0;
        let mut best = f64::MAX;
        while end < n {
            let s = sum + areas[end];
            let worst = (short * short * areas[i] / (s * s)).max(s * s / (short * short * areas[end]));
            if worst > best {
                break;
            }
            best = worst;
            sum = s;
            end += 1;
        }
        if w >= h {
            let cw = sum / h;
            let mut yy = y;
            for &a in &areas[i..end] {
                let ch = a / cw;
                out.push(rect(x as f32, yy as f32, (x + cw) as f32, (yy + ch) as f32));
                yy += ch;
            }
            x += cw;
            w -= cw;
        } else {
            let rh = sum / w;
            let mut xx = x;
            for &a in &areas[i..end] {
                let cw = a / rh;
                out.push(rect(xx as f32, y as f32, (xx + cw) as f32, (y + rh) as f32));
                xx += cw;
            }
            y += rh;
            h -= rh;
        }
        i = end;
    }
    out
}

const MAX_CELLS: usize = 400_000;

/// Alto de la franja con el nombre de una carpeta, según su profundidad y el tamaño del bloque.
fn header_h(depth: u8, w: f32, h: f32) -> f32 {
    if w < 64.0 || h < 36.0 {
        0.0
    } else if depth == 0 {
        24.0
    } else {
        17.0
    }
}

fn layout(arena: &[Node], id: u32, r: Rect, top: Option<(u32, f32)>, depth: u8, out: &mut Vec<Cell>) {
    let node = &arena[id as usize];
    if node.size == 0 || out.len() >= MAX_CELLS {
        return;
    }
    let mut kids: Vec<u32> = node.children.iter().copied().filter(|&c| arena[c as usize].size > 0).collect();
    // Durante el escaneo los hijos todavía no están ordenados.
    if !kids.windows(2).all(|w| arena[w[0] as usize].size >= arena[w[1] as usize].size) {
        kids.sort_unstable_by_key(|&c| std::cmp::Reverse(arena[c as usize].size));
    }
    let area = ((r.right - r.left) * (r.bottom - r.top)) as f64;
    if kids.is_empty() || area <= 0.0 {
        return;
    }
    let sum: f64 = kids.iter().map(|&c| arena[c as usize].size as f64).sum();
    let areas: Vec<f64> = kids.iter().map(|&c| arena[c as usize].size as f64 / sum * area).collect();
    // Color: cada hijo directo de la carpeta actual recibe un matiz según su posición acumulada.
    let mut acc = 0.0f64;
    for ((&c, a), cr) in kids.iter().zip(&areas).zip(squarify(&areas, r)) {
        let size = arena[c as usize].size as f64;
        let mid = ((acc + size / 2.0) / sum) as f32;
        acc += size;
        if *a < 1.0 {
            break;
        }
        let (top_id, hue) = top.unwrap_or((c, mid));
        let child = &arena[c as usize];
        let (w, h) = (cr.right - cr.left, cr.bottom - cr.top);
        if child.is_dir && w >= 10.0 && h >= 10.0 {
            out.push(Cell { r: cr, node: c, top: top_id, dir: true, depth, hue, small: false });
            let pad = if depth == 0 { 3.0 } else { 2.0 };
            let head = header_h(depth, w, h);
            layout(arena, c, rect(cr.left + pad, cr.top + pad + head, cr.right - pad, cr.bottom - pad), Some((top_id, hue)), depth + 1, out);
        } else {
            out.push(Cell { r: cr, node: c, top: top_id, dir: false, depth, hue, small: child.small });
        }
    }
}

// ---------- colores y textos ----------

fn color(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

fn rgb((r, g, b): (f32, f32, f32)) -> D2D1_COLOR_F {
    color(r, g, b)
}

const BG: (f32, f32, f32) = (0.045, 0.07, 0.115);
const PANEL: (f32, f32, f32) = (0.065, 0.10, 0.155);
const PANEL_DEEP: (f32, f32, f32) = (0.085, 0.125, 0.19);
const BORDER: (f32, f32, f32) = (0.13, 0.18, 0.26);
const TEXT: (f32, f32, f32) = (0.93, 0.94, 0.96);
const DIM: (f32, f32, f32) = (0.62, 0.67, 0.74);
const ACCENT: (f32, f32, f32) = (0.10, 0.52, 0.98);
const SELECT: (f32, f32, f32) = (0.25, 0.85, 1.0);
const FOLDER_ICON: (f32, f32, f32) = (0.98, 0.78, 0.30);
const GREEN: (f32, f32, f32) = (0.20, 0.72, 0.38);

/// Categorías de archivo: nombre, color y extensiones. La última recibe todo lo demás.
const CATS: [(&str, (f32, f32, f32), &[&str]); 8] = [
    ("Video", (0.60, 0.35, 0.95), &["mp4", "mkv", "avi", "mov", "wmv", "webm", "vhdx", "vhd", "vmdk"]),
    ("Imágenes", (0.95, 0.40, 0.72), &["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "psd", "heic"]),
    ("Audio", (0.98, 0.62, 0.25), &["mp3", "wav", "flac", "m4a", "ogg"]),
    ("Comprimidos", (0.55, 0.58, 0.64), &["zip", "rar", "7z", "iso", "tar", "gz", "cab", "msi", "img", "etl", "qcow2"]),
    ("Documentos", (0.38, 0.64, 0.98), &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "json", "xml", "log"]),
    ("Código", (0.70, 0.48, 0.96), &["rs", "js", "ts", "cs", "py", "java", "c", "cpp", "h", "html", "css", "go", "toml", "lock"]),
    ("Programas", (0.86, 0.38, 0.64), &["exe", "dll", "lib", "rlib", "rmeta", "pdb", "obj", "o", "sys", "a", "so", "d"]),
    ("Otros", (0.98, 0.80, 0.28), &[]),
];

fn category(name: &str) -> usize {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    CATS.iter().position(|(_, _, exts)| exts.contains(&ext.as_str())).unwrap_or(CATS.len() - 1)
}

fn file_color(name: &str) -> D2D1_COLOR_F {
    rgb(CATS[category(name)].1)
}

fn lighten(c: D2D1_COLOR_F, k: f32) -> D2D1_COLOR_F {
    color((c.r + k).min(1.0), (c.g + k).min(1.0), (c.b + k).min(1.0))
}

/// Rueda de color: azul, violeta, magenta, rosa, naranja, amarillo, lima, verde, turquesa, celeste.
const WHEEL: [(f32, f32, f32); 11] = [
    (0.224, 0.420, 1.000),
    (0.545, 0.302, 1.000),
    (0.851, 0.275, 0.937),
    (1.000, 0.310, 0.604),
    (1.000, 0.439, 0.353),
    (1.000, 0.702, 0.278),
    (1.000, 0.894, 0.361),
    (0.722, 0.941, 0.290),
    (0.298, 0.878, 0.416),
    (0.184, 0.847, 0.753),
    (0.227, 0.784, 1.000),
];

/// Color de la rueda en `t` (0..1, cíclico), mezclando entre los dos puntos vecinos.
fn wheel_at(t: f32) -> (f32, f32, f32) {
    let x = t.rem_euclid(1.0) * WHEEL.len() as f32;
    let i = (x.floor() as usize) % WHEEL.len();
    let (a, b, f) = (WHEEL[i], WHEEL[(i + 1) % WHEEL.len()], x.fract());
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f)
}

fn to_hsl((r, g, b): (f32, f32, f32)) -> (f32, f32, f32) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

/// Color de un bloque según el matiz heredado y la profundidad: las carpetas son un marco más
/// oscuro que se aclara al anidarse, y los archivos son las teselas vivas de adentro.
fn position_color(hue: f32, depth: u8, dir: bool) -> D2D1_COLOR_F {
    let (h, s, l) = to_hsl(wheel_at(hue));
    let d = depth as f32;
    let (s2, l2) = if dir { (s * 0.78, l * 0.5 + d * 0.035) } else { (s * 0.88, l + 0.03 + d * 0.01) };
    rgb(from_hsl(h, s2.clamp(0.0, 1.0), l2.clamp(0.1, 0.86)))
}

/// Negro azulado o blanco, el que mejor se lee sobre `c`.
fn text_on(c: D2D1_COLOR_F) -> D2D1_COLOR_F {
    let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    if lum > 0.62 { color(0.07, 0.06, 0.12) } else { color(1.0, 1.0, 1.0) }
}

fn fmt_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.2} {}", UNITS[u]).replace('.', ",") }
}

fn node_path(arena: &[Node], mut id: u32) -> String {
    let mut parts = Vec::new();
    loop {
        let n = &arena[id as usize];
        parts.push(n.name.trim_end_matches('\\').to_string());
        if n.parent == NO_PARENT {
            break;
        }
        id = n.parent;
    }
    parts.reverse();
    let mut s = parts.join("\\");
    if parts.len() == 1 {
        s.push('\\');
    }
    s
}

fn copy_text(hwnd: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = (|| -> Result<()> {
            EmptyClipboard()?;
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            std::ptr::copy_nonoverlapping(wide.as_ptr(), GlobalLock(mem) as *mut u16, wide.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0)))?;
            Ok(())
        })();
        let _ = CloseClipboard();
    }
}

// ---------- ventana ----------

/// Rectángulos de cada zona de la ventana.
struct Ui {
    list: Rect,
    map: Rect,
    card: Rect,
    legend: Rect,
    status: Rect,
    /// "Mostrar en xdir", "Copiar ruta", "Explorar aquí".
    buttons: [Rect; 3],
    /// Desplegable de unidades y botón "Analizar", en la barra superior.
    combo: Rect,
    scan_btn: Rect,
}

const MENU_ROW: f32 = 34.0;
const MENU_W: f32 = 400.0;

/// Lista desplegable de unidades (más la fila "Seleccionar carpeta…"), justo debajo del desplegable.
fn menu_rect(ui: &Ui, rows: usize) -> Rect {
    rect(ui.combo.left, ui.combo.bottom + 2.0, ui.combo.left + MENU_W, ui.combo.bottom + 2.0 + rows as f32 * MENU_ROW + 8.0)
}

struct DriveEntry {
    /// Raíz de la unidad, por ejemplo `C:\`.
    path: String,
    label: String,
    /// Sistema de archivos y tamaño, por ejemplo `NTFS · 475,85 GB`.
    detail: String,
}

fn utf16_str(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Unidades conectadas ahora mismo (discos, pendrives y unidades de red); se vuelve a leer al abrir el menú.
fn list_drives() -> Vec<DriveEntry> {
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
        if ty != 4 {
            if let Some((total, _)) = volume_space(std::path::Path::new(&root)) {
                detail = if detail.is_empty() { fmt_size(total) } else { format!("{detail} \u{B7} {}", fmt_size(total)) };
            }
        }
        out.push(DriveEntry { path: root, label: format!("[{letter}:] {name}"), detail });
    }
    out
}

/// Diálogo del sistema para elegir una carpeta.
fn pick_folder(owner: HWND) -> Option<PathBuf> {
    unsafe {
        let mut name = [0u16; 260];
        let bi = BROWSEINFOW {
            hwndOwner: owner,
            pszDisplayName: PWSTR(name.as_mut_ptr()),
            lpszTitle: w!("Elige la carpeta a analizar"),
            ulFlags: 0x41, // solo carpetas del sistema de archivos + diálogo moderno
            ..Default::default()
        };
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut path = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, &mut path).as_bool();
        CoTaskMemFree(Some(pidl as *const c_void));
        ok.then(|| PathBuf::from(utf16_str(&path)))
    }
}

/// `legend_h` es 0 cuando no hay leyenda (color por carpeta): el mapa gana ese espacio.
fn ui_layout(w: f32, h: f32, legend_h: f32) -> Ui {
    let body_bottom = h - STATUS_H - 4.0;
    let x0 = MARGIN + LIST_W + MARGIN;
    let list = rect(MARGIN, TOP_H + 4.0, MARGIN + LIST_W, body_bottom);
    let legend = rect(x0, body_bottom - legend_h, w - MARGIN, body_bottom);
    let card = rect(x0, legend.top - GAP - CARD_H, w - MARGIN, legend.top - GAP);
    let map = rect(x0, TOP_H + 4.0, w - MARGIN, card.top - GAP);
    let widths = [168.0, 134.0, 150.0];
    let (top, bottom) = ((card.top + card.bottom) / 2.0 - 20.0, (card.top + card.bottom) / 2.0 + 20.0);
    let mut x = card.right - 14.0;
    let mut buttons = [rect(0.0, 0.0, 0.0, 0.0); 3];
    for i in (0..3).rev() {
        buttons[i] = rect(x - widths[i], top, x, bottom);
        x -= widths[i] + 8.0;
    }
    let combo = rect(MARGIN, 6.0, MARGIN + 250.0, TOP_H - 6.0);
    let scan_btn = rect(combo.right + 8.0, 6.0, combo.right + 108.0, TOP_H - 6.0);
    Ui { list, map, card, legend, status: rect(0.0, h - STATUS_H, w, h), buttons, combo, scan_btn }
}

struct Fonts {
    ui: IDWriteTextFormat,
    bold: IDWriteTextFormat,
    big: IDWriteTextFormat,
    small: IDWriteTextFormat,
    head: IDWriteTextFormat,
    right: IDWriteTextFormat,
    center: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
}

struct App {
    hwnd: HWND,
    root: PathBuf,
    rt: Option<ID2D1HwndRenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    factory: ID2D1Factory,
    fonts: Fonts,
    arena: Vec<Node>,
    current: u32,
    selected: Option<u32>,
    scanning: bool,
    cells: Vec<Cell>,
    cells_for: (u32, i32, i32),
    hover: Option<usize>,
    hover_row: Option<usize>,
    hover_btn: Option<usize>,
    scroll: f32,
    dpi: f32,
    /// Escaneo en curso: generación (para ignorar resultados viejos), cancelación y árbol en vivo.
    scan_id: usize,
    cancel: Arc<AtomicBool>,
    live: Tree,
    /// Colorear por tipo de archivo en vez de por posición (tecla T).
    by_type: bool,
    /// Espacio del volumen que contiene la carpeta: (total, libre).
    vol: Option<(u64, u64)>,
    /// Unidad o carpeta elegida en el desplegable; se analiza al pulsar "Analizar".
    pick: PathBuf,
    drives: Vec<DriveEntry>,
    menu_open: bool,
    menu_hover: Option<usize>,
}

/// Espacio total y libre del volumen que contiene `root`.
fn volume_space(root: &std::path::Path) -> Option<(u64, u64)> {
    let mut wide: Vec<u16> = root.as_os_str().encode_wide().collect();
    while wide.last() == Some(&(b'\\' as u16)) {
        wide.pop();
    }
    wide.push(b'\\' as u16);
    wide.push(0);
    let (mut total, mut free) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), None, Some(&mut total), Some(&mut free)) }.ok()?;
    (total > 0).then_some((total, free))
}

thread_local! {
    static APP: StdCell<*mut App> = const { StdCell::new(std::ptr::null_mut()) };
}

/// Filas de la tabla: `None` es "subir"; el resto son los hijos de `current`, de mayor a menor.
fn rows_of(arena: &[Node], current: u32) -> Vec<Option<u32>> {
    let Some(cur) = arena.get(current as usize) else { return Vec::new() };
    let mut rows = Vec::new();
    if cur.parent != NO_PARENT {
        rows.push(None);
    }
    let mut kids: Vec<u32> = cur.children.iter().copied().filter(|&c| arena[c as usize].size > 0).collect();
    if !kids.windows(2).all(|w| arena[w[0] as usize].size >= arena[w[1] as usize].size) {
        kids.sort_unstable_by_key(|&c| std::cmp::Reverse(arena[c as usize].size));
    }
    rows.extend(kids.into_iter().map(Some));
    rows
}

fn text_format(dwrite: &IDWriteFactory, family: PCWSTR, size: f32, bold: bool) -> Result<IDWriteTextFormat> {
    unsafe {
        let weight = if bold { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
        let f = dwrite.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("es-es"),
        )?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }
}

impl App {
    fn size_dips(&self) -> (f32, f32) {
        let mut rc = windows::Win32::Foundation::RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        let k = 96.0 / self.dpi;
        (rc.right as f32 * k, rc.bottom as f32 * k)
    }

    fn ui(&self) -> Ui {
        let (w, h) = self.size_dips();
        ui_layout(w, h, self.legend_h())
    }

    fn legend_h(&self) -> f32 {
        if self.by_type { LEGEND_H } else { 0.0 }
    }

    fn ensure_target(&mut self) -> Result<()> {
        if self.rt.is_some() {
            return Ok(());
        }
        let mut rc = windows::Win32::Foundation::RECT::default();
        unsafe {
            GetClientRect(self.hwnd, &mut rc)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES { dpiX: self.dpi, dpiY: self.dpi, ..Default::default() };
            let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: rc.right.max(1) as u32, height: rc.bottom.max(1) as u32 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let rt = self.factory.CreateHwndRenderTarget(&props, &hprops)?;
            self.brush = Some(rt.CreateSolidColorBrush(&color(1.0, 1.0, 1.0), None)?);
            self.rt = Some(rt);
        }
        Ok(())
    }

    fn resize(&mut self) {
        let mut rc = windows::Win32::Foundation::RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        if let Some(rt) = &self.rt {
            let _ = unsafe { rt.Resize(&D2D_SIZE_U { width: rc.right.max(1) as u32, height: rc.bottom.max(1) as u32 }) };
        }
        self.invalidate();
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn update_title(&self) {
        let title = if self.arena.is_empty() {
            format!("xtree - {}", self.root.display())
        } else {
            format!("xtree - {}", node_path(&self.arena, self.current))
        };
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(wide.as_ptr()));
        }
    }

    fn rescan(&mut self) {
        // Cancela el escaneo anterior, si seguía en marcha.
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.live = Arc::new(RwLock::new(Vec::new()));
        self.scan_id += 1;
        self.arena.clear();
        self.cells.clear();
        self.cells_for = (u32::MAX, 0, 0);
        self.current = 0;
        self.hover = None;
        self.hover_row = None;
        self.selected = None;
        self.scanning = true;
        self.vol = volume_space(&self.root);
        self.update_title();
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_ID, 250, None);
        }
        start_scan(self.hwnd, self.root.clone(), self.scan_id, self.cancel.clone(), self.live.clone());
        self.invalidate();
    }

    fn go(&mut self, node: u32) {
        self.current = node;
        self.cells_for = (u32::MAX, 0, 0);
        self.hover = None;
        self.hover_row = None;
        self.selected = None;
        self.scroll = 0.0;
        self.update_title();
        self.invalidate();
    }

    fn go_up(&mut self) {
        if let Some(n) = self.arena.get(self.current as usize) {
            if n.parent != NO_PARENT {
                self.go(n.parent);
            }
        }
    }

    /// Filas de la tabla: `None` es "subir"; el resto son los hijos de la carpeta actual.
    fn rows(&self) -> Vec<Option<u32>> {
        rows_of(&self.arena, self.current)
    }

    fn to_dips(&self, x: f32, y: f32) -> (f32, f32) {
        let k = 96.0 / self.dpi;
        (x * k, y * k)
    }

    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (x, y) = self.to_dips(x, y);
        let ui = self.ui();
        let top = ui.list.top + TABLE_H;
        if !inside(&ui.list, x, y) || y < top {
            return None;
        }
        let i = ((y - top + self.scroll) / ROW_H) as usize;
        (i < self.rows().len()).then_some(i)
    }

    fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
        let (x, y) = self.to_dips(x, y);
        self.cells.iter().rposition(|c| inside(&c.r, x, y))
    }

    fn button_at(&self, x: f32, y: f32) -> Option<usize> {
        let (x, y) = self.to_dips(x, y);
        self.ui().buttons.iter().position(|b| inside(b, x, y))
    }

    /// Fila del menú de unidades bajo el cursor (coordenadas de pantalla).
    fn menu_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (x, y) = self.to_dips(x, y);
        let n = self.drives.len() + 1;
        let m = menu_rect(&self.ui(), n);
        if !inside(&m, x, y) || y < m.top + 4.0 {
            return None;
        }
        let i = ((y - m.top - 4.0) / MENU_ROW) as usize;
        (i < n).then_some(i)
    }

    /// Elige una fila del menú: una unidad, o el diálogo para elegir una carpeta.
    fn choose(&mut self, i: usize) {
        if let Some(d) = self.drives.get(i) {
            self.pick = PathBuf::from(&d.path);
        } else if let Some(p) = pick_folder(self.hwnd) {
            self.pick = p;
        }
        self.invalidate();
    }

    /// Analiza lo que está elegido en el desplegable.
    fn scan_pick(&mut self) {
        if self.pick.as_os_str().is_empty() {
            return;
        }
        self.root = self.pick.clone();
        self.rescan();
    }

    /// Texto del desplegable: la unidad elegida o, si es una carpeta, su ruta.
    fn pick_label(&self) -> String {
        let p = self.pick.to_string_lossy();
        let p = p.trim_end_matches('\\');
        match self.drives.iter().find(|d| d.path.trim_end_matches('\\').eq_ignore_ascii_case(p)) {
            Some(d) => d.label.clone(),
            None => p.to_string(),
        }
    }

    fn on_mouse_move(&mut self, x: f32, y: f32) {
        if self.menu_open {
            let row = self.menu_row_at(x, y);
            if row != self.menu_hover {
                self.menu_hover = row;
                self.invalidate();
            }
            return;
        }
        if self.scanning {
            return; // mientras se escanea el mapa es solo de lectura
        }
        let (row, cell, btn) = (self.row_at(x, y), self.cell_at(x, y), self.button_at(x, y));
        if row != self.hover_row || cell != self.hover || btn != self.hover_btn {
            self.hover_row = row;
            self.hover = cell;
            self.hover_btn = btn;
            self.invalidate();
        }
    }

    fn on_wheel(&mut self, delta: i16) {
        let ui = self.ui();
        let visible = ui.list.bottom - ui.list.top - TABLE_H;
        let max = (self.rows().len() as f32 * ROW_H - visible).max(0.0);
        self.scroll = (self.scroll - delta as f32 / 120.0 * ROW_H * 3.0).clamp(0.0, max);
        self.invalidate();
    }

    /// Elemento sobre el que actúan los botones y que muestra la tarjeta: el seleccionado, o la
    /// carpeta actual si no hay selección.
    fn target(&self) -> u32 {
        self.selected.unwrap_or(self.current)
    }

    fn on_click(&mut self, x: f32, y: f32) {
        // Barra superior: desplegable de unidades y botón "Analizar" (también mientras se escanea).
        if self.menu_open {
            let row = self.menu_row_at(x, y);
            self.menu_open = false;
            self.menu_hover = None;
            if let Some(i) = row {
                self.choose(i);
            }
            self.invalidate();
            return;
        }
        let (dx, dy) = self.to_dips(x, y);
        let ui = self.ui();
        if inside(&ui.combo, dx, dy) {
            self.drives = list_drives();
            self.menu_open = true;
            self.invalidate();
            return;
        }
        if inside(&ui.scan_btn, dx, dy) {
            self.scan_pick();
            return;
        }
        if self.arena.is_empty() {
            return;
        }
        if let Some(b) = self.button_at(x, y) {
            let t = self.target();
            match b {
                0 => self.show_in_xdir(t),
                1 => copy_text(self.hwnd, &node_path(&self.arena, t)),
                _ if self.arena[t as usize].is_dir && t != self.current => self.go(t),
                _ => {}
            }
            return;
        }
        if let Some(i) = self.row_at(x, y) {
            match self.rows()[i] {
                None => self.go_up(),
                Some(n) => self.selected = Some(n),
            }
        } else if let Some(i) = self.cell_at(x, y) {
            self.selected = Some(self.cells[i].node);
        }
        self.invalidate();
    }

    fn on_double_click(&mut self, x: f32, y: f32) {
        if self.scanning {
            return;
        }
        let node = if let Some(i) = self.row_at(x, y) {
            self.rows()[i]
        } else {
            self.cell_at(x, y).map(|i| self.cells[i].top)
        };
        if let Some(n) = node {
            if self.arena[n as usize].is_dir {
                self.go(n);
            }
        }
    }

    /// Vuelve a abrir xtree como administrador sobre la misma carpeta (pide confirmación a Windows).
    /// Solo tiene sentido cuando el escaneo rápido de la MFT no se pudo usar por falta de permisos.
    fn relaunch_elevated(&self) {
        if !NEED_ADMIN.load(Ordering::Relaxed) {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let exe: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let arg = format!("\"{}\"", self.root.to_string_lossy().trim_end_matches('\\'));
        let arg: Vec<u16> = arg.encode_utf16().chain(Some(0)).collect();
        let r = unsafe { ShellExecuteW(None, w!("runas"), PCWSTR(exe.as_ptr()), PCWSTR(arg.as_ptr()), PCWSTR::null(), SW_SHOW) };
        if r.0 as usize > 32 {
            self.cancel.store(true, Ordering::Relaxed);
            unsafe { PostQuitMessage(0) };
        }
    }

    /// Abre xdir (junto a xtree.exe) en la carpeta del elemento.
    fn show_in_xdir(&self, id: u32) {
        let n = &self.arena[id as usize];
        let folder = if n.is_dir || n.parent == NO_PARENT { id } else { n.parent };
        let path = node_path(&self.arena, folder);
        let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("xdir.exe")));
        let program = exe.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("xdir.exe"));
        let _ = std::process::Command::new(program).arg(path).spawn();
    }

    fn paint(&mut self) {
        if self.ensure_target().is_err() {
            return;
        }
        let (w, h) = self.size_dips();
        let ui = ui_layout(w, h, self.legend_h());
        // Mientras se escanea se dibuja el árbol compartido (con el bloqueo de lectura tomado).
        let live = self.live.clone();
        let guard = if self.scanning { Some(live.read().unwrap()) } else { None };
        let arena: &[Node] = match &guard {
            Some(g) => g.as_slice(),
            None => self.arena.as_slice(),
        };
        if !arena.is_empty() {
            let key = (self.current, w as i32, h as i32);
            if self.cells_for != key {
                self.cells.clear();
                if ui.map.right - ui.map.left > 8.0 && ui.map.bottom - ui.map.top > 8.0 {
                    layout(arena, self.current, ui.map, None, 0, &mut self.cells);
                }
                self.cells_for = key;
                self.hover = None;
            }
        }
        let visible = (ui.list.bottom - ui.list.top - TABLE_H).max(0.0);
        self.scroll = self.scroll.clamp(0.0, (rows_of(arena, self.current).len() as f32 * ROW_H - visible).max(0.0));

        let (Some(rt), Some(brush)) = (self.rt.clone(), self.brush.clone()) else { return };
        let f = &self.fonts;
        let fill = |r: &Rect, c: D2D1_COLOR_F| unsafe {
            brush.SetColor(&c);
            rt.FillRectangle(r, &brush);
        };
        let rfill = |r: &Rect, radius: f32, c: D2D1_COLOR_F| unsafe {
            brush.SetColor(&c);
            rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: *r, radiusX: radius, radiusY: radius }, &brush);
        };
        let rstroke = |r: &Rect, radius: f32, c: D2D1_COLOR_F, width: f32| unsafe {
            brush.SetColor(&c);
            rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: *r, radiusX: radius, radiusY: radius }, &brush, width, None);
        };
        let stroke = |r: &Rect, c: D2D1_COLOR_F, width: f32| unsafe {
            brush.SetColor(&c);
            rt.DrawRectangle(r, &brush, width, None);
        };
        let dot = |x: f32, y: f32, radius: f32, c: D2D1_COLOR_F| unsafe {
            brush.SetColor(&c);
            rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: radius, radiusY: radius }, &brush);
        };
        let text = |fmt: &IDWriteTextFormat, s: &str, r: &Rect, c: D2D1_COLOR_F| unsafe {
            brush.SetColor(&c);
            let wide: Vec<u16> = s.encode_utf16().collect();
            rt.DrawText(&wide, fmt, r, &brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        };
        let clip = |r: &Rect| unsafe { rt.PushAxisAlignedClip(r, D2D1_ANTIALIAS_MODE_ALIASED) };
        let unclip = || unsafe { rt.PopAxisAlignedClip() };
        // Íconos simples: carpeta dorada o página.
        let icon = |x: f32, y: f32, dir: bool, c: D2D1_COLOR_F| {
            if dir {
                rfill(&rect(x, y + 3.0, x + 7.0, y + 7.0), 1.5, c);
                rfill(&rect(x, y + 5.0, x + 16.0, y + 15.0), 2.0, c);
            } else {
                rfill(&rect(x + 2.0, y, x + 14.0, y + 16.0), 2.0, c);
            }
        };

        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&rgb(BG)));
        }

        // Barra superior: unidad y total a la izquierda, estado del análisis a la derecha.
        let done = !self.scanning && !arena.is_empty();
        let top_text = if done {
            let root = &arena[0];
            format!("{}  \u{B7}  {} analizados", root.name.trim_end_matches('\\'), fmt_size(root.size))
        } else {
            format!(
                "{}  \u{B7}  {} archivos - {}",
                self.root.display(),
                FILES.load(Ordering::Relaxed),
                fmt_size(BYTES.load(Ordering::Relaxed))
            )
        };
        text(&f.big, &top_text, &rect(ui.scan_btn.right + 18.0, 0.0, w - 220.0, TOP_H), rgb(TEXT));
        // Desplegable de unidades y botón "Analizar".
        let cb = ui.combo;
        rfill(&cb, 6.0, rgb(PANEL_DEEP));
        rstroke(&cb, 6.0, rgb(if self.menu_open { ACCENT } else { BORDER }), 1.0);
        text(&f.ui, &self.pick_label(), &rect(cb.left + 10.0, cb.top, cb.right - 28.0, cb.bottom), rgb(TEXT));
        text(&f.glyph, "\u{E70D}", &rect(cb.right - 26.0, cb.top, cb.right - 6.0, cb.bottom), rgb(DIM));
        rfill(&ui.scan_btn, 6.0, rgb(ACCENT));
        text(&f.center, "Analizar", &ui.scan_btn, color(1.0, 1.0, 1.0));
        let state = if done { "Análisis completado" } else { "Escaneando…" };
        dot(w - 178.0, TOP_H / 2.0, 9.0, rgb(if done { GREEN } else { FOLDER_ICON }));
        if done {
            text(&f.center, "\u{2713}", &rect(w - 187.0, 0.0, w - 169.0, TOP_H), color(1.0, 1.0, 1.0));
        }
        text(&f.ui, state, &rect(w - 160.0, 0.0, w - MARGIN, TOP_H), rgb(TEXT));

        if arena.is_empty() {
            let r = rect(ui.map.left, ui.map.top, ui.map.right, ui.map.bottom);
            rfill(&r, 8.0, rgb(PANEL));
            let msg = match PHASE.load(Ordering::Relaxed) {
                1 => {
                    let total = MFT_TOTAL.load(Ordering::Relaxed).max(1);
                    let pct = (MFT_DONE.load(Ordering::Relaxed) * 100 / total).min(100);
                    format!("Leyendo la tabla de archivos (MFT)… {pct} %")
                }
                2 => "Armando el árbol de carpetas…".to_string(),
                _ => "Escaneando el disco…".to_string(),
            };
            text(&f.center, &msg, &r, rgb(DIM));
        } else {
            let cur = &arena[self.current as usize];
            let rows = rows_of(arena, self.current);
            let hover_node = self
                .hover
                .and_then(|i| self.cells.get(i))
                .map(|c| c.node)
                .or_else(|| self.hover_row.and_then(|i| rows.get(i).copied().flatten()));
            // `hot_top`: hijo directo de la carpeta actual bajo el mouse; se resalta en mapa y tabla.
            let hot_top = self
                .hover
                .and_then(|i| self.cells.get(i))
                .map(|c| c.top)
                .or_else(|| self.hover_row.and_then(|i| rows.get(i).copied().flatten()));

            // ----- tabla -----
            let l = ui.list;
            rfill(&l, 8.0, rgb(PANEL));
            rstroke(&l, 8.0, rgb(BORDER), 1.0);
            let header_y = (l.top, l.top + TABLE_H);
            text(&f.ui, "Nombre", &rect(l.left + 36.0, header_y.0, l.right - 214.0, header_y.1), rgb(DIM));
            text(&f.right, "Tamaño", &rect(l.right - 214.0, header_y.0, l.right - 124.0, header_y.1), rgb(DIM));
            text(&f.right, "%", &rect(l.right - 124.0, header_y.0, l.right - 72.0, header_y.1), rgb(DIM));
            fill(&rect(l.left + 1.0, l.top + TABLE_H - 1.0, l.right - 1.0, l.top + TABLE_H), rgb(BORDER));
            clip(&rect(l.left, l.top + TABLE_H, l.right, l.bottom - 2.0));
            for (i, row) in rows.iter().enumerate() {
                let top = l.top + TABLE_H + i as f32 * ROW_H - self.scroll;
                if top + ROW_H < l.top + TABLE_H || top > l.bottom {
                    continue;
                }
                let band = rect(l.left + 3.0, top + 1.0, l.right - 3.0, top + ROW_H - 1.0);
                let is_sel = row.is_some() && *row == self.selected;
                let is_hot = self.hover_row == Some(i) || (row.is_some() && *row == hot_top);
                if is_sel {
                    rfill(&band, 5.0, rgb((0.10, 0.20, 0.34)));
                    rstroke(&band, 5.0, rgb(SELECT), 1.0);
                } else if is_hot {
                    rfill(&band, 5.0, rgb(PANEL_DEEP));
                }
                match row {
                    None => text(&f.ui, "\u{2191}  Subir", &rect(l.left + 12.0, top, l.right - 8.0, top + ROW_H), rgb(DIM)),
                    Some(id) => {
                        let n = &arena[*id as usize];
                        let frac = if cur.size > 0 { n.size as f32 / cur.size as f32 } else { 0.0 };
                        let ic = if n.is_dir { rgb(FOLDER_ICON) } else { color(0.85, 0.88, 0.93) };
                        icon(l.left + 12.0, top + (ROW_H - 16.0) / 2.0, n.is_dir, ic);
                        text(&f.ui, &n.name, &rect(l.left + 36.0, top, l.right - 214.0, top + ROW_H), rgb(TEXT));
                        text(&f.right, &fmt_size(n.size), &rect(l.right - 214.0, top, l.right - 124.0, top + ROW_H), rgb(TEXT));
                        let pct = format!("{:.1} %", frac * 100.0).replace('.', ",");
                        text(&f.right, &pct, &rect(l.right - 124.0, top, l.right - 72.0, top + ROW_H), rgb(DIM));
                        let track = rect(l.right - 62.0, top + ROW_H / 2.0 - 4.0, l.right - 12.0, top + ROW_H / 2.0 + 4.0);
                        rfill(&track, 4.0, rgb(BORDER));
                        let bar_w = ((track.right - track.left) * frac).max(3.0);
                        rfill(&rect(track.left, track.top, track.left + bar_w, track.bottom), 4.0, rgb(ACCENT));
                    }
                }
            }
            unclip();

            // ----- mapa -----
            rfill(&ui.map, 8.0, rgb(PANEL));
            clip(&ui.map);
            for c in &self.cells {
                let n = &arena[c.node as usize];
                let same = hot_top == Some(c.top);
                let exact = hover_node == Some(c.node);
                let sel = self.selected == Some(c.node);
                let (cw, ch) = (c.r.right - c.r.left, c.r.bottom - c.r.top);
                if c.dir {
                    let lift = if exact { 0.07 } else if same { 0.035 } else { 0.0 };
                    let tinted = !self.by_type;
                    let base = if tinted {
                        position_color(c.hue, c.depth, true)
                    } else {
                        rgb(if c.depth == 0 { PANEL_DEEP } else { (0.105, 0.15, 0.225) })
                    };
                    rfill(&c.r, 3.0, lighten(base, lift));
                    rstroke(&c.r, 3.0, if tinted { rgb(BG) } else { rgb(BORDER) }, 1.0);
                    if header_h(c.depth, cw, ch) > 0.0 {
                        let label = format!("{}   {}", n.name, fmt_size(n.size));
                        let hh = header_h(c.depth, cw, ch);
                        let font = if c.depth == 0 { &f.head } else { &f.small };
                        let tc = if tinted { text_on(base) } else if c.depth == 0 { rgb(TEXT) } else { rgb(DIM) };
                        text(font, &label, &rect(c.r.left + 8.0, c.r.top + 2.0, c.r.right - 4.0, c.r.top + 2.0 + hh), tc);
                    }
                } else {
                    let col = if c.small {
                        rgb((0.075, 0.10, 0.155)) // grupo de archivos pequeños: casi del color del fondo
                    } else if self.by_type {
                        file_color(&n.name)
                    } else {
                        position_color(c.hue, c.depth, false)
                    };
                    let col = if exact { lighten(col, 0.22) } else if same { lighten(col, 0.08) } else { col };
                    fill(&rect(c.r.left + 0.5, c.r.top + 0.5, c.r.right - 0.5, c.r.bottom - 0.5), col);
                    let tc = if c.small { rgb(DIM) } else if self.by_type { color(1.0, 1.0, 1.0) } else { text_on(col) };
                    if cw >= 60.0 && ch >= 38.0 {
                        text(&f.small, &n.name, &rect(c.r.left + 6.0, c.r.top + 3.0, c.r.right - 3.0, c.r.top + 19.0), tc);
                        let sz = fmt_size(n.size);
                        text(&f.small, &sz, &rect(c.r.left + 6.0, c.r.top + 19.0, c.r.right - 3.0, c.r.top + 35.0), tc);
                    } else if cw >= 60.0 && ch >= 18.0 {
                        text(&f.small, &n.name, &rect(c.r.left + 6.0, c.r.top, c.r.right - 3.0, c.r.bottom), tc);
                    }
                }
                if sel {
                    stroke(&rect(c.r.left + 1.0, c.r.top + 1.0, c.r.right - 1.0, c.r.bottom - 1.0), rgb(SELECT), 2.0);
                }
            }
            unclip();
            rstroke(&ui.map, 8.0, rgb(BORDER), 1.0);

            // ----- tarjeta de detalle -----
            let shown = hover_node.or(self.selected).unwrap_or(self.current);
            let n = &arena[shown as usize];
            let cd = ui.card;
            rfill(&cd, 8.0, rgb(PANEL));
            rstroke(&cd, 8.0, rgb(BORDER), 1.0);
            let cy = (cd.top + cd.bottom) / 2.0;
            if n.is_dir {
                rfill(&rect(cd.left + 18.0, cy - 12.0, cd.left + 30.0, cy - 6.0), 2.0, rgb(FOLDER_ICON));
                rfill(&rect(cd.left + 18.0, cy - 8.0, cd.left + 46.0, cy + 12.0), 3.0, rgb(FOLDER_ICON));
            } else {
                rfill(&rect(cd.left + 20.0, cy - 15.0, cd.left + 44.0, cy + 15.0), 3.0, color(0.93, 0.94, 0.96));
            }
            let text_right = ui.buttons[0].left - 14.0;
            let title = format!("{}  \u{B7}  {}", n.name.trim_end_matches('\\'), fmt_size(n.size));
            text(&f.big, &title, &rect(cd.left + 62.0, cd.top + 8.0, text_right, cd.top + 38.0), rgb(TEXT));
            text(&f.ui, &node_path(&arena, shown), &rect(cd.left + 62.0, cd.top + 38.0, text_right, cd.top + 66.0), rgb(DIM));
            let labels = [("\u{E838}", "Mostrar en xdir"), ("\u{E8C8}", "Copiar ruta"), ("\u{E8A7}", "Explorar aquí")];
            let target_is_dir = arena[self.target() as usize].is_dir && self.target() != self.current;
            for (i, (glyph, label)) in labels.iter().enumerate() {
                let b = ui.buttons[i];
                let enabled = i != 2 || target_is_dir;
                let hot = self.hover_btn == Some(i) && enabled;
                rfill(&b, 7.0, if hot { rgb((0.12, 0.18, 0.28)) } else { rgb(PANEL_DEEP) });
                rstroke(&b, 7.0, if hot { rgb(ACCENT) } else { rgb(BORDER) }, 1.0);
                let tc = if enabled { rgb(TEXT) } else { rgb((0.40, 0.44, 0.50)) };
                text(&f.glyph, glyph, &rect(b.left + 12.0, b.top, b.left + 34.0, b.bottom), tc);
                text(&f.ui, label, &rect(b.left + 40.0, b.top, b.right - 6.0, b.bottom), tc);
            }

            // ----- leyenda (solo al colorear por tipo; por carpeta el color no necesita explicación) -----
            if self.by_type {
                rfill(&ui.legend, 8.0, rgb(PANEL));
                rstroke(&ui.legend, 8.0, rgb(BORDER), 1.0);
                let ly = (ui.legend.top + ui.legend.bottom) / 2.0;
                let mut entries: Vec<(&str, (f32, f32, f32))> = CATS.iter().map(|(n, c, _)| (*n, *c)).collect();
                entries.push(("Carpeta", ACCENT));
                let slot = (ui.legend.right - ui.legend.left) / entries.len() as f32;
                for (i, (name, c)) in entries.iter().enumerate() {
                    let x = ui.legend.left + 16.0 + slot * i as f32;
                    rfill(&rect(x, ly - 9.0, x + 18.0, ly + 9.0), 4.0, rgb(*c));
                    text(&f.ui, name, &rect(x + 26.0, ly - 12.0, x + slot - 4.0, ly + 12.0), rgb(TEXT));
                }
            }
        }

        // ----- barra inferior -----
        // Datos del disco: carpeta actual, espacio total, utilizado y libre del volumen.
        let bar = rect(MARGIN, ui.status.top + 4.0, w - MARGIN, h - 6.0);
        rfill(&bar, 8.0, rgb(PANEL));
        rstroke(&bar, 8.0, rgb(BORDER), 1.0);
        let mut cols: Vec<(&str, String)> = Vec::new();
        if done {
            let cur = &arena[self.current as usize];
            let count = cur.children.iter().filter(|&&c| arena[c as usize].size > 0).count();
            cols.push(("Carpeta actual", format!("{} \u{B7} {} elementos", fmt_size(cur.size), count)));
        } else {
            cols.push(("Analizado", fmt_size(BYTES.load(Ordering::Relaxed))));
        }
        let pct = |v: u64, of: u64| format!("{:.1} %", v as f64 / of.max(1) as f64 * 100.0).replace('.', ",");
        if let Some((total, free)) = self.vol {
            let free = free.min(total);
            let used = total - free;
            cols.push(("Espacio total", fmt_size(total)));
            cols.push(("Utilizado", format!("{} ({})", fmt_size(used), pct(used, total))));
            cols.push(("Libre", format!("{} ({})", fmt_size(free), pct(free, total))));
            if done {
                let seen = arena[0].size;
                let whole = volume_drive(self.root.to_string_lossy().trim_end_matches('\\')).is_some();
                if whole {
                    cols.push(("Sin analizar", fmt_size(used.saturating_sub(seen))));
                } else {
                    cols.push(("Analizado", format!("{} ({} del uso)", fmt_size(seen), pct(seen, used))));
                }
            }
            // Barra de ocupación del volumen: roja cuando queda poco espacio libre.
            let track = rect(bar.left + 16.0, bar.bottom - 10.0, bar.right - 16.0, bar.bottom - 6.0);
            rfill(&track, 2.0, rgb(BORDER));
            let frac = used as f32 / total as f32;
            let fillc = if frac > 0.9 { rgb((0.92, 0.32, 0.34)) } else { rgb(ACCENT) };
            rfill(&rect(track.left, track.top, track.left + (track.right - track.left) * frac, track.bottom), 2.0, fillc);
        }
        let widths = [250.0, 120.0, 190.0, 180.0, 190.0];
        let mut x = bar.left + 16.0;
        for (i, (label, value)) in cols.iter().enumerate() {
            let cw = widths[i.min(widths.len() - 1)];
            text(&f.small, label, &rect(x, bar.top + 4.0, x + cw, bar.top + 20.0), rgb(DIM));
            text(&f.bold, value, &rect(x, bar.top + 19.0, x + cw, bar.top + 37.0), rgb(TEXT));
            x += cw;
        }
        if NEED_ADMIN.load(Ordering::Relaxed) {
            let hint = "Escaneo rápido de la MFT: ejecuta como administrador (tecla A)";
            text(&f.right, hint, &rect((x + 10.0).max(bar.right - 420.0), bar.top + 10.0, bar.right - 16.0, bar.top + 30.0), rgb(FOLDER_ICON));
        } else if done {
            let hint = if self.by_type { "T: color por carpeta" } else { "T: color por tipo de archivo" };
            text(&f.right, hint, &rect((x + 10.0).max(bar.right - 300.0), bar.top + 10.0, bar.right - 16.0, bar.top + 30.0), rgb(DIM));
        }
        // Lista de unidades: va al final para quedar por encima de todo.
        if self.menu_open {
            let n = self.drives.len() + 1;
            let m = menu_rect(&ui, n);
            rfill(&m, 8.0, rgb(PANEL));
            rstroke(&m, 8.0, rgb(ACCENT), 1.0);
            for i in 0..n {
                let top = m.top + 4.0 + i as f32 * MENU_ROW;
                let row = rect(m.left + 4.0, top, m.right - 4.0, top + MENU_ROW);
                if self.menu_hover == Some(i) {
                    rfill(&row, 5.0, rgb((0.10, 0.20, 0.34)));
                }
                if let Some(d) = self.drives.get(i) {
                    text(&f.ui, &d.label, &rect(row.left + 10.0, top, row.right - 170.0, top + MENU_ROW), rgb(TEXT));
                    text(&f.right, &d.detail, &rect(row.right - 170.0, top, row.right - 10.0, top + MENU_ROW), rgb(DIM));
                } else {
                    text(&f.ui, "Seleccionar carpeta\u{2026}", &rect(row.left + 10.0, top, row.right - 10.0, top + MENU_ROW), rgb(TEXT));
                }
            }
        }
        let res = unsafe { rt.EndDraw(None, None) };
        if res.is_err() {
            // Se perdió el destino de dibujo (cambio de pantalla, etc.): se recrea en el próximo WM_PAINT.
            self.rt = None;
            self.brush = None;
            self.invalidate();
        }
    }
}

fn app() -> Option<&'static mut App> {
    let p = APP.with(|a| a.get());
    if p.is_null() { None } else { Some(unsafe { &mut *p }) }
}

fn xy(lparam: LPARAM) -> (f32, f32) {
    ((lparam.0 & 0xFFFF) as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as i16 as f32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(app) = app() else { return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } };
    match msg {
        WM_PAINT => {
            app.paint();
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            app.resize();
            LRESULT(0)
        }
        WM_TIMER => {
            if app.scanning {
                // El mapa en vivo se recalcula con lo que lleva descubierto el escáner.
                app.cells_for = (u32::MAX, 0, 0);
                app.invalidate();
            }
            LRESULT(0)
        }
        WM_SCAN_DONE => {
            let arena = unsafe { *Box::from_raw(lparam.0 as *mut Vec<Node>) };
            if wparam.0 != app.scan_id {
                return LRESULT(0); // resultado de un escaneo que ya se reemplazó
            }
            app.arena = arena;
            app.vol = volume_space(&app.root); // el espacio libre cambió mientras se escaneaba
            app.current = 0;
            app.scanning = false;
            app.cells_for = (u32::MAX, 0, 0);
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
            }
            app.update_title();
            app.invalidate();
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = xy(lparam);
            app.on_mouse_move(x, y);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            app.on_wheel(((wparam.0 >> 16) & 0xFFFF) as i16);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = xy(lparam);
            app.on_click(x, y);
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            let (x, y) = xy(lparam);
            app.on_double_click(x, y);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            app.go_up();
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wparam.0 as u16 {
                k if k == VK_BACK.0 => app.go_up(),
                k if k == VK_ESCAPE.0 => {
                    app.menu_open = false;
                    app.invalidate();
                }
                k if k == VK_A.0 => app.relaunch_elevated(),
                k if k == VK_T.0 => {
                    app.by_type = !app.by_type;
                    app.invalidate();
                }
                k if k == VK_F5.0 => app.rescan(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            app.cancel.store(true, Ordering::Relaxed);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn main() -> Result<()> {
    let root = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        PathBuf::from(format!("{drive}\\"))
    });
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // para el diálogo de carpetas
        let instance = GetModuleHandleW(None)?;
        let class_name = w!("XtreeWindow");
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let seg = w!("Segoe UI");
        let right = text_format(&dwrite, seg, 13.0, false)?;
        right.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
        let center = text_format(&dwrite, seg, 14.0, false)?;
        center.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        let fonts = Fonts {
            ui: text_format(&dwrite, seg, 13.0, false)?,
            bold: text_format(&dwrite, seg, 13.0, true)?,
            big: text_format(&dwrite, seg, 16.0, true)?,
            small: text_format(&dwrite, seg, 11.5, false)?,
            head: text_format(&dwrite, seg, 12.5, true)?,
            right,
            center,
            glyph: text_format(&dwrite, w!("Segoe MDL2 Assets"), 15.0, false)?,
        };
        let _ = &fonts.bold;
        let app = Box::into_raw(Box::new(App {
            hwnd: HWND::default(),
            root,
            rt: None,
            brush: None,
            factory,
            fonts,
            arena: Vec::new(),
            current: 0,
            selected: None,
            scanning: false,
            cells: Vec::new(),
            cells_for: (u32::MAX, 0, 0),
            hover: None,
            hover_row: None,
            hover_btn: None,
            scroll: 0.0,
            dpi: 96.0,
            scan_id: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            live: Arc::new(RwLock::new(Vec::new())),
            by_type: false,
            vol: None,
            pick: PathBuf::new(),
            drives: Vec::new(),
            menu_open: false,
            menu_hover: None,
        }));
        APP.with(|a| a.set(app));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("xtree"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1400,
            820,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let a = &mut *app;
        a.hwnd = hwnd;
        a.dpi = GetDpiForWindow(hwnd) as f32;
        a.pick = a.root.clone();
        a.drives = list_drives();
        let _ = ShowWindow(hwnd, SW_SHOW);
        a.rescan();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

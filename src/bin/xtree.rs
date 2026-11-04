//! xtree: mapa de uso de disco (treemap) al estilo WizTree. Programa aparte que lanza xdir.
//! Uso: xtree.exe [carpeta]
//!
//! Escáner recursivo con FindFirstFileEx en un hilo (sin leer la MFT), treemap "squarified" con
//! Direct2D, tabla de lo más pesado de la carpeta actual y tarjeta de detalle con acciones.
//! Clic selecciona; doble clic entra en la carpeta; clic derecho o Retroceso suben; F5 reescanea.

#![windows_subsystem = "windows"]

use std::cell::Cell as StdCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
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
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    WIN32_FIND_DATAW,
};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_BACK, VK_F5};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect,
    GetMessageW, IDC_ARROW, KillTimer, LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOW, SetTimer,
    SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDOWN, WM_SIZE, WM_TIMER, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, Result, w};
use windows_numerics::Vector2;

type Rect = D2D_RECT_F;

const WM_SCAN_DONE: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;

// Medidas (DIPs).
const MARGIN: f32 = 8.0;
const TOP_H: f32 = 40.0;
const STATUS_H: f32 = 26.0;
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

// ---------- escáner ----------

struct Node {
    name: Box<str>,
    parent: u32,
    size: u64,
    is_dir: bool,
    children: Vec<u32>,
}

const NO_PARENT: u32 = u32::MAX;

/// Recorre `dir` (ruta ancha, sin barra final) y agrega sus nodos al arreglo. Devuelve el id del nodo.
fn scan(arena: &mut Vec<Node>, dir: &mut Vec<u16>, name: &str, parent: u32) -> u32 {
    let id = arena.len() as u32;
    arena.push(Node { name: name.into(), parent, size: 0, is_dir: true, children: Vec::new() });

    let base = dir.len();
    dir.extend("\\*".encode_utf16());
    dir.push(0);
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(dir.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut c_void,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    dir.truncate(base);
    let Ok(handle) = handle else { return id };

    let mut children = Vec::new();
    let mut total = 0u64;
    loop {
        let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
        let fname = &data.cFileName[..len];
        let attrs = data.dwFileAttributes;
        let dots = fname == [b'.' as u16] || fname == [b'.' as u16, b'.' as u16];
        if !dots {
            let name = String::from_utf16_lossy(fname);
            if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
                // Junctions y enlaces simbólicos no se siguen: contarían dos veces el mismo espacio.
                if attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                    dir.push(b'\\' as u16);
                    dir.extend_from_slice(fname);
                    let child = scan(arena, dir, &name, id);
                    dir.truncate(base);
                    total += arena[child as usize].size;
                    children.push(child);
                }
            } else if attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) == 0 {
                let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
                let child = arena.len() as u32;
                arena.push(Node { name: name.into(), parent: id, size, is_dir: false, children: Vec::new() });
                total += size;
                children.push(child);
                FILES.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(size, Ordering::Relaxed);
            }
        }
        if unsafe { FindNextFileW(handle, &mut data) }.is_err() {
            break;
        }
    }
    let _ = unsafe { FindClose(handle) };

    children.sort_by_key(|&c| std::cmp::Reverse(arena[c as usize].size));
    let node = &mut arena[id as usize];
    node.size = total;
    node.children = children;
    id
}

fn start_scan(hwnd: HWND, root: PathBuf) {
    FILES.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let hwnd = hwnd.0 as isize;
    let _ = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut text = root.to_string_lossy().trim_end_matches('\\').to_string();
        let label = if text.is_empty() { "\\".to_string() } else { format!("{text}\\") };
        // Prefijo de ruta larga: sin el límite de 260 caracteres.
        if !text.starts_with("\\\\?\\") {
            text = format!("\\\\?\\{text}");
        }
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut arena = Vec::new();
        scan(&mut arena, &mut wide, &label, NO_PARENT);
        let boxed = Box::into_raw(Box::new(arena));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_SCAN_DONE, WPARAM(0), LPARAM(boxed as isize));
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

fn layout(arena: &[Node], id: u32, r: Rect, top: Option<u32>, depth: u8, out: &mut Vec<Cell>) {
    let node = &arena[id as usize];
    let total = node.size as f64;
    if total <= 0.0 || out.len() >= MAX_CELLS {
        return;
    }
    let kids: Vec<u32> = node.children.iter().copied().filter(|&c| arena[c as usize].size > 0).collect();
    let area = ((r.right - r.left) * (r.bottom - r.top)) as f64;
    if kids.is_empty() || area <= 0.0 {
        return;
    }
    let areas: Vec<f64> = kids.iter().map(|&c| arena[c as usize].size as f64 / total * area).collect();
    for ((&c, a), cr) in kids.iter().zip(&areas).zip(squarify(&areas, r)) {
        if *a < 1.0 {
            break;
        }
        let top = top.unwrap_or(c);
        let child = &arena[c as usize];
        let (w, h) = (cr.right - cr.left, cr.bottom - cr.top);
        if child.is_dir && w >= 10.0 && h >= 10.0 {
            out.push(Cell { r: cr, node: c, top, dir: true, depth });
            let pad = if depth == 0 { 3.0 } else { 2.0 };
            let head = header_h(depth, w, h);
            layout(arena, c, rect(cr.left + pad, cr.top + pad + head, cr.right - pad, cr.bottom - pad), Some(top), depth + 1, out);
        } else {
            out.push(Cell { r: cr, node: c, top, dir: false, depth });
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
}

fn ui_layout(w: f32, h: f32) -> Ui {
    let body_bottom = h - STATUS_H - 4.0;
    let x0 = MARGIN + LIST_W + MARGIN;
    let list = rect(MARGIN, TOP_H + 4.0, MARGIN + LIST_W, body_bottom);
    let legend = rect(x0, body_bottom - LEGEND_H, w - MARGIN, body_bottom);
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
    Ui { list, map, card, legend, status: rect(0.0, h - STATUS_H, w, h), buttons }
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
}

thread_local! {
    static APP: StdCell<*mut App> = const { StdCell::new(std::ptr::null_mut()) };
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
        ui_layout(w, h)
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
        self.arena.clear();
        self.cells.clear();
        self.hover = None;
        self.hover_row = None;
        self.selected = None;
        self.scanning = true;
        self.update_title();
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_ID, 120, None);
        }
        start_scan(self.hwnd, self.root.clone());
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
        let Some(cur) = self.arena.get(self.current as usize) else { return Vec::new() };
        let mut rows = Vec::new();
        if cur.parent != NO_PARENT {
            rows.push(None);
        }
        rows.extend(cur.children.iter().copied().filter(|&c| self.arena[c as usize].size > 0).map(Some));
        rows
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

    fn on_mouse_move(&mut self, x: f32, y: f32) {
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
        let ui = ui_layout(w, h);
        if !self.scanning && !self.arena.is_empty() {
            let key = (self.current, w as i32, h as i32);
            if self.cells_for != key {
                self.cells.clear();
                if ui.map.right - ui.map.left > 8.0 && ui.map.bottom - ui.map.top > 8.0 {
                    layout(&self.arena, self.current, ui.map, None, 0, &mut self.cells);
                }
                self.cells_for = key;
                self.hover = None;
            }
        }
        let visible = (ui.list.bottom - ui.list.top - TABLE_H).max(0.0);
        self.scroll = self.scroll.clamp(0.0, (self.rows().len() as f32 * ROW_H - visible).max(0.0));

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
        let done = !self.scanning && !self.arena.is_empty();
        let top_text = if done {
            let root = &self.arena[0];
            format!("{}  \u{B7}  {} analizados", root.name.trim_end_matches('\\'), fmt_size(root.size))
        } else {
            format!(
                "{}  \u{B7}  {} archivos - {}",
                self.root.display(),
                FILES.load(Ordering::Relaxed),
                fmt_size(BYTES.load(Ordering::Relaxed))
            )
        };
        text(&f.big, &top_text, &rect(MARGIN + 4.0, 0.0, w - 220.0, TOP_H), rgb(TEXT));
        let state = if done { "Análisis completado" } else { "Escaneando…" };
        dot(w - 178.0, TOP_H / 2.0, 9.0, rgb(if done { GREEN } else { FOLDER_ICON }));
        if done {
            text(&f.center, "\u{2713}", &rect(w - 187.0, 0.0, w - 169.0, TOP_H), color(1.0, 1.0, 1.0));
        }
        text(&f.ui, state, &rect(w - 160.0, 0.0, w - MARGIN, TOP_H), rgb(TEXT));

        if !done {
            let r = rect(ui.map.left, ui.map.top, ui.map.right, ui.map.bottom);
            rfill(&r, 8.0, rgb(PANEL));
            text(&f.center, "Escaneando el disco…", &r, rgb(DIM));
        } else {
            let cur = &self.arena[self.current as usize];
            let rows = self.rows();
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
                        let n = &self.arena[*id as usize];
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
                let n = &self.arena[c.node as usize];
                let same = hot_top == Some(c.top);
                let exact = hover_node == Some(c.node);
                let sel = self.selected == Some(c.node);
                let (cw, ch) = (c.r.right - c.r.left, c.r.bottom - c.r.top);
                if c.dir {
                    let base = if c.depth == 0 { PANEL_DEEP } else { (0.105, 0.15, 0.225) };
                    let lift = if exact { 0.07 } else if same { 0.035 } else { 0.0 };
                    rfill(&c.r, 3.0, lighten(rgb(base), lift));
                    rstroke(&c.r, 3.0, rgb(BORDER), 1.0);
                    if header_h(c.depth, cw, ch) > 0.0 {
                        let label = format!("{}   {}", n.name, fmt_size(n.size));
                        let hh = header_h(c.depth, cw, ch);
                        let font = if c.depth == 0 { &f.head } else { &f.small };
                        let tc = if c.depth == 0 { rgb(TEXT) } else { rgb(DIM) };
                        text(font, &label, &rect(c.r.left + 8.0, c.r.top + 2.0, c.r.right - 4.0, c.r.top + 2.0 + hh), tc);
                    }
                } else {
                    let col = file_color(&n.name);
                    let col = if exact { lighten(col, 0.22) } else if same { lighten(col, 0.08) } else { col };
                    fill(&rect(c.r.left + 0.5, c.r.top + 0.5, c.r.right - 0.5, c.r.bottom - 0.5), col);
                    let white = color(1.0, 1.0, 1.0);
                    if cw >= 60.0 && ch >= 38.0 {
                        text(&f.small, &n.name, &rect(c.r.left + 6.0, c.r.top + 3.0, c.r.right - 3.0, c.r.top + 19.0), white);
                        let sz = fmt_size(n.size);
                        text(&f.small, &sz, &rect(c.r.left + 6.0, c.r.top + 19.0, c.r.right - 3.0, c.r.top + 35.0), color(1.0, 1.0, 1.0));
                    } else if cw >= 60.0 && ch >= 18.0 {
                        text(&f.small, &n.name, &rect(c.r.left + 6.0, c.r.top, c.r.right - 3.0, c.r.bottom), white);
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
            let n = &self.arena[shown as usize];
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
            text(&f.ui, &node_path(&self.arena, shown), &rect(cd.left + 62.0, cd.top + 38.0, text_right, cd.top + 66.0), rgb(DIM));
            let labels = [("\u{E838}", "Mostrar en xdir"), ("\u{E8C8}", "Copiar ruta"), ("\u{E8A7}", "Explorar aquí")];
            let target_is_dir = self.arena[self.target() as usize].is_dir && self.target() != self.current;
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

            // ----- leyenda -----
            rfill(&ui.legend, 8.0, rgb(PANEL));
            rstroke(&ui.legend, 8.0, rgb(BORDER), 1.0);
            let mut entries: Vec<(&str, (f32, f32, f32))> = CATS.iter().map(|(n, c, _)| (*n, *c)).collect();
            entries.push(("Carpeta", ACCENT));
            let slot = (ui.legend.right - ui.legend.left) / entries.len() as f32;
            let ly = (ui.legend.top + ui.legend.bottom) / 2.0;
            for (i, (name, c)) in entries.iter().enumerate() {
                let x = ui.legend.left + 16.0 + slot * i as f32;
                rfill(&rect(x, ly - 9.0, x + 18.0, ly + 9.0), 4.0, rgb(*c));
                text(&f.ui, name, &rect(x + 26.0, ly - 12.0, x + slot - 4.0, ly + 12.0), rgb(TEXT));
            }
        }

        // ----- barra inferior -----
        if done {
            let cur = &self.arena[self.current as usize];
            let count = cur.children.iter().filter(|&&c| self.arena[c as usize].size > 0).count();
            let s = format!("{} en {} elementos", fmt_size(cur.size), count);
            text(&f.ui, &s, &rect(MARGIN + 4.0, ui.status.top, w - MARGIN, ui.status.bottom), rgb(DIM));
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
                app.invalidate();
            }
            LRESULT(0)
        }
        WM_SCAN_DONE => {
            let arena = unsafe { *Box::from_raw(lparam.0 as *mut Vec<Node>) };
            app.arena = arena;
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
                k if k == VK_F5.0 => app.rescan(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> Result<()> {
    let root = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        PathBuf::from(format!("{drive}\\"))
    });
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
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

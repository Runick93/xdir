//! Íconos del sistema en un hilo propio. Sacar un ícono de Windows puede tardar decenas de
//! milisegundos la primera vez (carga DLLs y handlers de íconos), así que la interfaz nunca lo
//! hace: pide el ícono, dibuja sin él, y el hilo avisa con `WM_APP_ICONS` cuando está listo.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, ReleaseDC,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_OVERLAYINDEX, SHGFI_SYSICONINDEX, SHGetFileInfoW, SHGetImageList,
    SHIL_EXTRALARGE, SHIL_LARGE, SHIL_SMALL,
};
use windows::core::PCWSTR;

use crate::fs::{LoadEvent, LoadMsg, WM_APP_LOAD, wide_path};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO, PostMessageW, WM_APP};

/// Hay íconos nuevos listos para dibujar.
pub const WM_APP_ICONS: u32 = WM_APP + 3;

/// Píxeles BGRA premultiplicados.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Qué lista de imágenes del sistema usar (pequeña, grande o extra grande).
pub type ListKind = u32;
/// (lista, índice del ícono, superposición: 0 = ninguna).
type Key = (ListKind, i32, u8);

struct Shared {
    ready: HashMap<Key, Option<Arc<Pixels>>>,
    pending: HashSet<Key>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared { ready: HashMap::new(), pending: HashSet::new() }))
}

fn worker() -> &'static Mutex<Sender<(Key, usize)>> {
    static W: OnceLock<Mutex<Sender<(Key, usize)>>> = OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::spawn(move || run(rx));
        Mutex::new(tx)
    })
}

/// Lista a usar para dibujar íconos de 16 DIPs a `dpi`.
pub fn list_for_dpi(dpi: u32) -> ListKind {
    let px = 16 * dpi / 96;
    if px <= 16 {
        SHIL_SMALL
    } else if px <= 32 {
        SHIL_LARGE
    } else {
        SHIL_EXTRALARGE
    }
}

/// Arranca el hilo y carga la lista de imágenes por adelantado (se llama al iniciar).
pub fn prewarm(kind: ListKind) {
    let _ = worker().lock().unwrap().send(((kind, -1, 0), 0));
}

/// Bloquea hasta que el hilo de íconos haya creado las listas de imágenes del sistema.
///
/// Si `SHGetFileInfoW` corre en otro hilo mientras se crean, el Shell a veces devuelve tipo
/// e ícono genéricos ("Archivo"); por eso los lectores de carpetas esperan esto primero.
pub fn wait_shell_ready() {
    worker();
    let (lock, cv) = ready();
    let mut done = lock.lock().unwrap();
    while !*done {
        done = cv.wait(done).unwrap();
    }
}

fn ready() -> &'static (Mutex<bool>, Condvar) {
    static R: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
    R.get_or_init(|| (Mutex::new(false), Condvar::new()))
}

/// Devuelve el ícono si ya está; si no, lo encarga y devuelve None.
/// `Some(None)` significa que el ícono no se pudo obtener (no volver a pedirlo).
pub fn get(hwnd: HWND, kind: ListKind, index: i32, overlay: u8) -> Option<Option<Arc<Pixels>>> {
    let key = (kind, index, overlay);
    let mut s = shared().lock().unwrap();
    if let Some(hit) = s.ready.get(&key) {
        return Some(hit.clone());
    }
    if s.pending.insert(key) {
        let _ = worker().lock().unwrap().send((key, hwnd.0 as usize));
    }
    None
}

fn run(rx: Receiver<(Key, usize)>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let mut lists: HashMap<ListKind, Option<IImageList>> = HashMap::new();
    for kind in [SHIL_SMALL, SHIL_LARGE] {
        lists.insert(kind, unsafe { SHGetImageList::<IImageList>(kind as i32).ok() });
    }
    {
        let (lock, cv) = ready();
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    while let Ok(first) = rx.recv() {
        // Se atiende todo lo que esté en cola y se avisa una sola vez por tanda.
        let batch: Vec<_> = std::iter::once(first).chain(rx.try_iter()).collect();
        let mut notify = HashSet::new();
        for (key, hwnd) in batch {
            let list = lists
                .entry(key.0)
                .or_insert_with(|| unsafe { SHGetImageList::<IImageList>(key.0 as i32).ok() });
            if key.1 < 0 {
                continue; // solo precarga
            }
            let pixels = list.as_ref().and_then(|l| unsafe {
                // INDEXTOOVERLAYMASK: la lista dibuja el ícono con la superposición encima.
                let icon = l.GetIcon(key.1, ILD_TRANSPARENT.0 | (key.2 as u32) << 8).ok()?;
                let px = icon_pixels(icon);
                let _ = DestroyIcon(icon);
                px.map(Arc::new)
            });
            let mut s = shared().lock().unwrap();
            s.pending.remove(&key);
            s.ready.insert(key, pixels);
            notify.insert(hwnd);
        }
        for hwnd in notify {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_ICONS, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Pedido de superposiciones (flecha de acceso directo, estado de OneDrive, Git...) para
/// elementos de una pestaña. El resultado vuelve como `LoadMsg::Overlays` por el canal de carga.
pub struct OverlayRequest {
    pub hwnd: HWND,
    pub tx: Sender<LoadEvent>,
    pub panel: usize,
    pub generation: u64,
    /// (posición del elemento, ruta).
    pub items: Vec<(u32, PathBuf)>,
}

/// Encola un pedido. Corre en un hilo aparte del de íconos: los handlers de superposición de
/// otros programas pueden ser lentos y no deben demorar los íconos comunes.
pub fn request_overlays(req: OverlayRequest) {
    struct Job(usize, Sender<LoadEvent>, usize, u64, Vec<(u32, PathBuf)>);
    static W: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let worker = W.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            wait_shell_ready();
            while let Ok(Job(hwnd, tx, panel, generation, items)) = rx.recv() {
                let found: Vec<(u32, u8)> = items
                    .iter()
                    .filter_map(|(pos, path)| Some((*pos, overlay_of(path)?)))
                    .collect();
                if found.is_empty() {
                    continue;
                }
                let msg = LoadMsg::Overlays(found);
                if tx.send(LoadEvent { panel, generation, msg }).is_ok() {
                    unsafe {
                        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_LOAD, WPARAM(panel), LPARAM(0));
                    }
                }
            }
        });
        Mutex::new(tx)
    });
    let job = Job(req.hwnd.0 as usize, req.tx, req.panel, req.generation, req.items);
    let _ = worker.lock().unwrap().send(job);
}

/// Índice de superposición de un archivo (1..15), o None si no tiene.
fn overlay_of(path: &Path) -> Option<u8> {
    let wide = wide_path(path);
    let mut info = SHFILEINFOW::default();
    unsafe {
        // SHGFI_OVERLAYINDEX exige pedir también el ícono; se libera enseguida.
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_OVERLAYINDEX | SHGFI_SYSICONINDEX,
        );
        if !info.hIcon.is_invalid() {
            let _ = DestroyIcon(info.hIcon);
        }
        // El índice de superposición viene en los 8 bits altos de iIcon.
        let overlay = ((info.iIcon >> 24) & 0xFF) as u8;
        (ok != 0 && overlay > 0).then_some(overlay)
    }
}

/// Lee los píxeles BGRA premultiplicados de un HICON.
unsafe fn icon_pixels(icon: HICON) -> Option<Pixels> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let result = (|| {
            let mut bm = BITMAP::default();
            if GetObjectW(info.hbmColor.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
            let mut color = read_bits(info.hbmColor, w, h)?;
            // Íconos viejos sin canal alfa: la transparencia sale de la máscara.
            if color.chunks_exact(4).all(|p| p[3] == 0) {
                let mask = read_bits(info.hbmMask, w, h)?;
                for (p, m) in color.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            for p in color.chunks_exact_mut(4) {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = (*c as u32 * a / 255) as u8;
                }
            }
            Some(Pixels { width: w, height: h, data: color })
        })();
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor.into());
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
        result
    }
}

unsafe fn read_bits(bitmap: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            // Negativo: filas de arriba hacia abajo.
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let hdc = GetDC(None);
        let lines = GetDIBits(hdc, bitmap, 0, h, Some(px.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        (lines == h as i32).then_some(px)
    }
}

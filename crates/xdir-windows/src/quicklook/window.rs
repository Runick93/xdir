//! La ventana de la vista rápida (`xdir.quicklook`): `WS_POPUP | WS_THICKFRAME` sin marco del
//! sistema (todo es área de la ventana; esquinas redondeadas de DWM), dueña de la principal.
//!
//! - Una hija (`xdir.quicklook.canvas`) es la superficie de Direct2D, con un `Renderer` propio:
//!   la barra de título del núcleo y, sin visor, la imagen grande (`draw_fallback`). Con el visor
//!   a la vista se achica a la barra: el visor (otra ventana, de otro hilo y otro proceso) nunca
//!   queda debajo de lo que dibuja Direct2D.
//! - El visor va en `pure::viewer_rect`, dejando libre el borde para redimensionar.
//! - La ventana se crea la primera vez y después solo se oculta y se vuelve a mostrar.
//!
//! Estado en un `thread_local` del hilo de interfaz. Nunca queda prestado mientras se llama a
//! algo que mande mensajes a estas ventanas (`SetWindowPos`, `ShowWindow`, diálogos).

use std::cell::RefCell;
use std::sync::Once;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromWindow, PAINTSTRUCT, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::Shell::{OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPENASINFO, SHOpenWithDialog};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};

use xdir_core::geom::Rect;
use xdir_core::keys::keys::{VK_ESCAPE, VK_LEFT, VK_RIGHT};
use xdir_core::paint::Painter;
use xdir_core::quicklook::{self, FallbackHit, QuickLookItem, TitleHit};
use xdir_core::theme::Theme;

use crate::platform::{Handle, HandleExt, handle_from_hwnd};
use crate::render::Renderer;

use super::handler::{self, Job, Viewer};
use super::pure::{
    HTCLIENT, KEYDOWN, Mode, Outcome, Slot, Status, WM_APP_QL_CLOSED, WM_APP_QL_KEY, WM_QL_DIED, WM_QL_SETTLED, bar, body,
    canvas_height, decode_outcome, forward_key, initial_rect, min_size, nc_hit, viewer_rect,
};

const POPUP_CLASS: PCWSTR = w!("xdir.quicklook");
const CANVAS_CLASS: PCWSTR = w!("xdir.quicklook.canvas");
/// `WM_NCHITTEST`: el punto es de la ventana de abajo (la de la vista rápida).
const HTTRANSPARENT: isize = -1;
/// Ícono grande del cuerpo mientras se espera al visor.
const ICON: f32 = 96.0;

/// La vista rápida (una por proceso, la de la ventana principal).
struct Ql {
    main: HWND,
    popup: HWND,
    canvas: HWND,
    theme: Theme,
    /// Lo que se muestra (`None`: cerrada).
    item: Option<QuickLookItem>,
    slot: Slot,
    viewer: Option<Viewer>,
    /// Lo que está bajo el mouse en la barra y en el cuerpo.
    hover: (Option<TitleHit>, Option<FallbackHit>),
    /// Ya se pidió `WM_MOUSELEAVE` a la superficie.
    tracking: bool,
}

thread_local! {
    static QL: RefCell<Option<Ql>> = const { RefCell::new(None) };
    /// Direct2D de la superficie; aparte de `QL` porque dibujar no debe tener el estado prestado.
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
    /// El DPI que tiene el renderizador (`set_dpi` vacía sus cachés: solo si cambió).
    static RENDERER_DPI: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Lleva el renderizador al DPI de la ventana, si cambió.
fn sync_dpi(dpi: u32) {
    if RENDERER_DPI.with(|d| d.replace(dpi)) != dpi {
        with_renderer(|r| r.set_dpi(dpi));
    }
}

/// Corre `f` con el estado, si existe y no está prestado (un mensaje anidado no hace nada).
fn with<R>(f: impl FnOnce(&mut Ql) -> R) -> Option<R> {
    QL.with(|q| {
        let mut q = q.try_borrow_mut().ok()?;
        q.as_mut().map(f)
    })
}

fn with_renderer(f: impl FnOnce(&mut Renderer)) {
    RENDERER.with(|r| {
        if let Ok(mut r) = r.try_borrow_mut()
            && let Some(r) = r.as_mut()
        {
            f(r);
        }
    });
}

// ---------- lo que llama el host ----------

/// Muestra `item` (o pasa a él). `false` si la ventana no se pudo crear: la `App` dibuja su
/// tarjeta. Si es el mismo archivo que ya se ve, solo cambia la barra («k de n»).
pub fn show(main: Handle, item: &QuickLookItem, theme: &Theme) -> bool {
    let Some((popup, canvas)) = ensure_window(main.hwnd(), theme) else { return false };
    let visible = unsafe { IsWindowVisible(popup) }.as_bool();
    let same = with(|q| {
        // Si el hilo del visor descubrió que estaba en la nube, eso queda (la `App` no lo sabe).
        let same = visible
            && q.item.as_ref().is_some_and(|i| i.path == item.path && i.pidl == item.pidl && (i.cloud_only || !item.cloud_only));
        if let Some(cur) = q.item.as_mut().filter(|_| same) {
            let cloud = cur.cloud_only;
            *cur = item.clone();
            cur.cloud_only = cloud;
        }
        same
    });
    if same == Some(true) {
        invalidate(canvas);
        return true;
    }
    // Otro archivo: el visor anterior se descarga y, si el tipo tiene, se encarga uno nuevo.
    let clsid = match (&item.path, item.cloud_only) {
        (Some(p), false) => p.extension().and_then(|e| handler::viewer_for(&e.to_string_lossy())),
        _ => None,
    };
    if !visible {
        place_initial(main.hwnd(), popup);
    }
    let (w, h, k) = client_dips(popup);
    let job_rect = to_px(viewer_rect(w, h), k);
    let dpi = unsafe { GetDpiForWindow(popup) }.max(96);
    // El renderizador sigue el DPI de la ventana (pudo abrirse en otro monitor).
    sync_dpi(dpi);
    let started = with(|q| {
        retire(q);
        let generation = q.slot.start(clsid.is_some(), item.cloud_only);
        q.item = Some(item.clone());
        q.hover = (None, None);
        if let (Some(clsid), Some(path)) = (clsid, &item.path) {
            let job = Job {
                clsid,
                path: path.clone(),
                parent: popup.0 as isize,
                main: q.main.0 as isize,
                rect: job_rect,
                generation,
                back: colorref(q.theme.surface).0,
                text: colorref(q.theme.text).0,
                dpi,
            };
            q.viewer = Some(handler::start(job));
        }
    });
    if started.is_none() {
        return false;
    }
    let title = HSTRING::from(item.name.as_str());
    unsafe {
        let _ = SetWindowTextW(popup, &title);
    }
    layout();
    unsafe {
        if !visible {
            let _ = ShowWindow(popup, SW_SHOWNOACTIVATE);
        } else if GetForegroundWindow() == popup {
            // El foco pudo quedar en el visor anterior: vuelve a la ventana (las teclas siguen).
            let _ = SetFocus(Some(popup));
        }
    }
    invalidate(canvas);
    true
}

/// Cierra la ventana (la oculta; se reusa la próxima vez) y descarga el visor.
pub fn close() {
    let Some(popup) = with(|q| {
        retire(q);
        q.item = None;
        q.slot.start(false, false);
        q.popup
    }) else {
        return;
    };
    unsafe {
        let _ = ShowWindow(popup, SW_HIDE);
    }
}

/// Suelta el visor actual: si todavía se lo espera (puede estar colgado arrancando), se lo
/// abandona; si no, se lo descarga.
fn retire(q: &mut Ql) {
    if let Some(v) = q.viewer.take() {
        if q.slot.mode == Mode::Loading { v.abandon() } else { v.stop() }
    }
}

/// Al cerrar xdir (con el estado ya guardado): cierra la vista rápida y dice si hay que terminar
/// el proceso sin destruir ventanas porque un visor abandonado sigue colgado.
pub fn exit_now() -> bool {
    close();
    handler::must_exit_hard()
}

/// El tema cambió (o se dibujó la ventana principal): si es otro, se repinta con el nuevo.
pub fn set_theme(theme: &Theme) {
    let changed = with(|q| {
        let t = &q.theme;
        let same = t.dark == theme.dark && t.accent == theme.accent && t.surface == theme.surface && t.text == theme.text;
        q.theme = *theme;
        (!same).then_some((q.popup, q.canvas))
    });
    if let Some(Some((popup, canvas))) = changed {
        apply_dwm(popup, theme.dark);
        unsafe {
            let _ = InvalidateRect(Some(popup), None, true);
        }
        invalidate(canvas);
    }
}

/// La ventana principal se destruye: se sueltan el visor, el estado y el renderizador (en este
/// hilo). La ventana de la vista rápida la destruye Windows con su dueña.
pub fn shutdown() {
    let state = QL.with(|q| q.try_borrow_mut().ok().and_then(|mut q| q.take()));
    drop(state);
    let renderer = RENDERER.with(|r| r.try_borrow_mut().ok().and_then(|mut r| r.take()));
    drop(renderer);
}

// ---------- ventana ----------

/// La ventana y su superficie, creándolas la primera vez.
fn ensure_window(main: HWND, theme: &Theme) -> Option<(HWND, HWND)> {
    if let Some(found) = with(|q| (q.popup, q.canvas))
        && unsafe { IsWindow(Some(found.0)) }.as_bool()
    {
        set_theme(theme);
        return Some(found);
    }
    shutdown();
    static REGISTER: Once = Once::new();
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        REGISTER.call_once(|| {
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            for (name, proc_) in [(POPUP_CLASS, Some(popup_proc as _)), (CANVAS_CLASS, Some(canvas_proc as _))] as [(PCWSTR, WNDPROC); 2] {
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: proc_,
                    hInstance: instance.into(),
                    hCursor: cursor,
                    lpszClassName: name,
                    ..Default::default()
                };
                RegisterClassExW(&class);
            }
        });
        let renderer = Renderer::new().ok()?;
        let popup = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            POPUP_CLASS,
            w!("Vista rápida"),
            WS_POPUP | WS_THICKFRAME | WS_SYSMENU | WS_CLIPCHILDREN,
            0,
            0,
            960,
            680,
            Some(main),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        let canvas = match CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CANVAS_CLASS,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(popup),
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(c) => c,
            Err(_) => {
                let _ = DestroyWindow(popup);
                return None;
            }
        };
        apply_dwm(popup, theme.dark);
        let ql = Ql {
            main,
            popup,
            canvas,
            theme: *theme,
            item: None,
            slot: Slot::default(),
            viewer: None,
            hover: (None, None),
            tracking: false,
        };
        QL.with(|q| *q.borrow_mut() = Some(ql));
        RENDERER.with(|r| *r.borrow_mut() = Some(renderer));
        RENDERER_DPI.with(|d| d.set(0));
        sync_dpi(GetDpiForWindow(popup).max(96));
        Some((popup, canvas))
    }
}

/// Esquinas redondeadas, borde del modo claro u oscuro y la sombra de DWM (sin marco).
fn apply_dwm(popup: HWND, dark: bool) {
    unsafe {
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const std::ffi::c_void,
            size_of_val(&corner) as u32,
        );
        let dark = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(popup, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const std::ffi::c_void, size_of::<BOOL>() as u32);
        // Un píxel de marco extendido arriba (lo tapa la barra): DWM le da sombra a la ventana.
        let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 };
        let _ = DwmExtendFrameIntoClientArea(popup, &margins);
    }
}

/// 960×680 centrada sobre xdir, dentro del área de trabajo de su monitor.
fn place_initial(main: HWND, popup: HWND) {
    unsafe {
        let mut owner = RECT::default();
        let _ = GetWindowRect(main, &mut owner);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let work = if GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            info.rcWork
        } else {
            owner
        };
        let dpi = GetDpiForWindow(main).max(96);
        let [l, t, r, b] = initial_rect([owner.left, owner.top, owner.right, owner.bottom], [work.left, work.top, work.right, work.bottom], dpi);
        let _ = SetWindowPos(popup, None, l, t, r - l, b - t, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
}

/// Tamaño del área de la ventana en DIPs y píxeles por DIP.
fn client_dips(hwnd: HWND) -> (f32, f32, f32) {
    let mut rc = RECT::default();
    let k = unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
        GetDpiForWindow(hwnd).max(96) as f32 / 96.0
    };
    ((rc.right - rc.left) as f32 / k, (rc.bottom - rc.top) as f32 / k, k)
}

fn to_px(r: Rect, k: f32) -> RECT {
    let px = |v: f32| (v * k).round() as i32;
    RECT { left: px(r.left), top: px(r.top), right: px(r.right), bottom: px(r.bottom) }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// La superficie ocupa la barra (con el visor) o toda la ventana; el visor, su lugar.
fn layout() {
    let Some((popup, canvas, mode)) = with(|q| (q.popup, q.canvas, q.slot.mode)) else { return };
    let (w, h, k) = client_dips(popup);
    let ch = canvas_height(h, mode);
    unsafe {
        let _ = SetWindowPos(canvas, None, 0, 0, (w * k).round() as i32, (ch * k).round() as i32, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    let r = to_px(viewer_rect(w, h), k);
    with(|q| {
        if let Some(v) = &q.viewer {
            v.place(r);
        }
    });
    unsafe {
        let _ = InvalidateRect(Some(popup), None, true);
    }
}

/// Terminó la espera del visor `generation`.
fn on_settled(generation: u64, outcome: Outcome<Status>) {
    let changed = with(|q| {
        let mode = q.slot.settle(generation, outcome)?;
        if mode == Mode::Viewer {
            if let Some(v) = &q.viewer {
                v.show();
            }
        } else if let Some(v) = q.viewer.take() {
            if outcome == Outcome::Timeout { v.abandon() } else { v.stop() }
        }
        if outcome == Outcome::Done(Status::Cloud)
            && let Some(item) = &mut q.item
        {
            item.cloud_only = true;
        }
        Some(q.canvas)
    });
    if let Some(Some(canvas)) = changed {
        layout();
        invalidate(canvas);
    }
}

/// El visor que se veía murió: la imagen grande.
fn on_died(generation: u64) {
    let changed = with(|q| {
        if !q.slot.died(generation) {
            return None;
        }
        if let Some(v) = q.viewer.take() {
            v.abandon();
        }
        Some(q.canvas)
    });
    if let Some(Some(canvas)) = changed {
        layout();
        invalidate(canvas);
    }
}

/// Espacio, Esc y las flechas (sin modificadores) en la ventana van a la `App`.
fn on_key(vk: u16) -> bool {
    let modifiers = [VK_CONTROL, VK_MENU, VK_SHIFT].iter().any(|k| unsafe { GetKeyState(k.0 as i32) } < 0);
    if !forward_key(KEYDOWN, vk, modifiers) {
        return false;
    }
    post_key(vk);
    true
}

fn post_key(vk: u16) {
    if let Some(main) = with(|q| q.main) {
        unsafe {
            let _ = PostMessageW(Some(main), WM_APP_QL_KEY, WPARAM(vk as usize), LPARAM(0));
        }
    }
}

/// Posición del mouse de un mensaje (`lparam`, píxeles del área) en DIPs.
fn mouse_dips(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
    let k = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    (x / k, y / k)
}

/// `WM_NCHITTEST` (punto de pantalla en `lparam`) contra la ventana de la vista rápida.
fn hit_test(popup: HWND, lparam: LPARAM) -> u32 {
    let mut pt = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
    unsafe {
        let _ = ScreenToClient(popup, &mut pt);
    }
    let (w, h, k) = client_dips(popup);
    nc_hit(w, h, pt.x as f32 / k, pt.y as f32 / k)
}

/// Mouse sobre la superficie: lo que queda bajo el mouse (y si es un clic, lo que hace).
fn on_mouse(canvas: HWND, x: f32, y: f32, click: bool) {
    let (w, h, _) = client_dips(canvas);
    let Some((hit, changed, tracking)) = with(|q| {
        let title = quicklook::title_hit(bar(w), x, y).filter(|t| *t != TitleHit::Drag);
        let missing = q.slot.mode == Mode::Fallback { viewer_missing: true } && !q.item.as_ref().is_some_and(|i| i.cloud_only);
        let fallback = if title.is_none() { quicklook::fallback_hit(body(w, h), missing, x, y) } else { None };
        let changed = q.hover != (title, fallback);
        q.hover = (title, fallback);
        let tracking = std::mem::replace(&mut q.tracking, true);
        ((title, fallback), changed, tracking)
    }) else {
        return;
    };
    if changed {
        invalidate(canvas);
    }
    if !tracking {
        let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: canvas, dwHoverTime: 0 };
        unsafe {
            let _ = TrackMouseEvent(&mut tme);
        }
    }
    if !click {
        return;
    }
    match hit {
        (Some(TitleHit::Prev), _) => post_key(VK_LEFT.0),
        (Some(TitleHit::Next), _) => post_key(VK_RIGHT.0),
        (Some(TitleHit::Close), _) => post_key(VK_ESCAPE.0),
        (Some(TitleHit::Open), _) | (_, Some(FallbackHit::Open)) => open_item(),
        (_, Some(FallbackHit::OpenWith)) => open_with(),
        _ => {}
    }
}

/// «Abrir»: con su programa (como el doble clic).
fn open_item() {
    let Some((main, Some(item))) = with(|q| (q.main, q.item.clone())) else { return };
    let Some(path) = item.path else { return };
    let target = xdir_core::shell::Item { path, pidl: item.pidl };
    xdir_core::sys::shell().open_item(handle_from_hwnd(main), &target);
}

/// «Abrir con…»: el diálogo de Windows para elegir el programa.
fn open_with() {
    let Some((popup, Some(path))) = with(|q| (q.popup, q.item.as_ref().and_then(|i| i.path.clone()))) else { return };
    let file = HSTRING::from(path.as_path());
    let info = OPENASINFO { pcszFile: PCWSTR(file.as_ptr()), pcszClass: PCWSTR::null(), oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC };
    unsafe {
        let _ = SHOpenWithDialog(Some(popup), &info);
    }
}

/// Dibuja la superficie: la barra de título y, sin visor, el cuerpo.
fn paint(canvas: HWND) {
    let Some(Some((item, mode, hover, theme))) = with(|q| q.item.clone().map(|i| (i, q.slot.mode, q.hover, q.theme))) else { return };
    let dpi = unsafe { GetDpiForWindow(canvas) }.max(96);
    with_renderer(|r| {
        let _ = r.draw(handle_from_hwnd(canvas), dpi, &theme, &[], |p, w, h| draw(p, w, h, &item, mode, hover));
    });
}

fn draw(p: &dyn Painter, w: f32, h: f32, item: &QuickLookItem, mode: Mode, hover: (Option<TitleHit>, Option<FallbackHit>)) {
    let t = *p.theme();
    p.fill(xdir_core::geom::rect(0.0, 0.0, w, h), t.surface);
    quicklook::draw_title_bar(p, bar(w), item, hover.0);
    let body = body(w, h);
    match mode {
        Mode::Viewer => {}
        Mode::Loading => {
            let (cx, cy) = ((body.left + body.right) / 2.0, (body.top + body.bottom) / 2.0);
            p.shell_icon(item.icon, 0, cx - ICON / 2.0, cy - ICON / 2.0, ICON);
        }
        Mode::Fallback { viewer_missing } => {
            let mut item = item.clone();
            // El escalón de la imagen con la escala de este monitor.
            item.key.1 = xdir_core::thumbs::big_side(p.thumb_px(quicklook::IMAGE_DIPS));
            p.clip(body, &mut || quicklook::draw_fallback(p, body, &item, viewer_missing, hover.1));
        }
    }
}

fn colorref(c: xdir_core::geom::Color) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | ch(c.g) << 8 | ch(c.b) << 16)
}

// ---------- procedimientos de ventana ----------

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Todo es área de la ventana (sin marco ni barra del sistema).
            WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
            // Sin marco que repintar al activarse.
            WM_NCACTIVATE => DefWindowProcW(hwnd, msg, wparam, LPARAM(-1)),
            WM_NCHITTEST => LRESULT(hit_test(hwnd, lparam) as isize),
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                let (w, h) = min_size(GetDpiForWindow(hwnd).max(96));
                info.ptMinTrackSize = POINT { x: w, y: h };
                LRESULT(0)
            }
            WM_SIZE => {
                layout();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xFFFF) as u32;
                sync_dpi(dpi);
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                LRESULT(0)
            }
            // Lo que no tapa la superficie (el borde alrededor del visor): el fondo de la tarjeta.
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(color) = with(|q| q.theme.surface) {
                    let brush = CreateSolidBrush(colorref(color));
                    FillRect(hdc, &ps.rcPaint, brush);
                    let _ = DeleteObject(brush.into());
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_KEYDOWN if on_key(wparam.0 as u16) => LRESULT(0),
            WM_CHAR => LRESULT(0),
            // Alt+F4: se cierra y la `App` se entera.
            WM_CLOSE => {
                let main = with(|q| q.main);
                close();
                if let Some(main) = main {
                    let _ = PostMessageW(Some(main), WM_APP_QL_CLOSED, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_QL_SETTLED => {
                on_settled(wparam.0 as u64, decode_outcome(lparam.0));
                LRESULT(0)
            }
            WM_QL_DIED => {
                on_died(wparam.0 as u64);
                LRESULT(0)
            }
            WM_DESTROY => {
                shutdown();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn canvas_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Bordes y zona de arrastre de la barra: los atiende la ventana de abajo.
            WM_NCHITTEST => match GetParent(hwnd) {
                Ok(popup) if hit_test(popup, lparam) != HTCLIENT => LRESULT(HTTRANSPARENT),
                _ => LRESULT(HTCLIENT as isize),
            },
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                let (width, height) = ((lparam.0 & 0xFFFF) as u32, ((lparam.0 >> 16) & 0xFFFF) as u32);
                let dpi = GetDpiForWindow(hwnd).max(96);
                with_renderer(|r| r.resize(width, height, dpi));
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = mouse_dips(hwnd, lparam);
                on_mouse(hwnd, x, y, false);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = mouse_dips(hwnd, lparam);
                on_mouse(hwnd, x, y, true);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if with(|q| {
                    q.tracking = false;
                    std::mem::take(&mut q.hover) != (None, None)
                }) == Some(true)
                {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN if on_key(wparam.0 as u16) => LRESULT(0),
            WM_CHAR => LRESULT(0),
            // Terminó una imagen grande (o un ícono) pedida por esta superficie.
            xdir_core::paint::WM_APP_ICONS => {
                with_renderer(|r| r.absorb_big());
                invalidate(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

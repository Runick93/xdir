//! Ventanas secundarias del sistema cuyo contenido dibuja el núcleo (`WindowHost::aux_*`): por
//! ahora la ventana de copia (`xdir.aux`), como la de Windows: barra de título del sistema con
//! minimizar y cerrar, dueña la principal, sin redimensionar (el alto lo pone el contenido con
//! `aux_resize`), abajo a la derecha del monitor de xdir la primera vez y después donde la dejó el
//! usuario (mientras dure la sesión). Dibuja con un `Renderer` propio llamando a
//! `App::paint_aux`; el mouse y las teclas van a `App::aux_mouse` / `App::aux_key`; cerrarla la
//! oculta (`App::aux_closed`) y los trabajos siguen.
//!
//! Sigue el modelo de `quicklook/window.rs`: estado en un `thread_local` del hilo de interfaz que
//! nunca queda prestado mientras se llama a algo que mande mensajes a la ventana.

use std::cell::{Cell, RefCell};
use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};

use xdir_core::app::App;
use xdir_core::host::{AuxMouse, AuxWindow};
use xdir_core::keys::Key;
use xdir_core::theme::Theme;

use crate::platform::handle_from_hwnd;
use crate::render::Renderer;

const CLASS: PCWSTR = w!("xdir.aux");
/// Separación de los bordes del área de trabajo (píxeles).
const MARGIN: i32 = 16;

fn style() -> WINDOW_STYLE {
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
}

/// Rectángulo de la ventana (`size` en píxeles, con el borde) abajo a la derecha de `work`.
pub fn initial_rect(work: RECT, size: SIZE) -> RECT {
    let right = work.right - MARGIN;
    let bottom = work.bottom - MARGIN;
    let left = (right - size.cx).max(work.left);
    let top = (bottom - size.cy).max(work.top);
    RECT { left, top, right: left + size.cx, bottom: top + size.cy }
}

/// Tamaño de la ventana para un área de `client` píxeles; `border` es lo que da
/// `AdjustWindowRectExForDpi` para un área vacía (izquierda y arriba negativos).
pub fn client_to_window_size(client: SIZE, border: RECT) -> SIZE {
    SIZE { cx: client.cx + (border.right - border.left), cy: client.cy + (border.bottom - border.top) }
}

struct Aux {
    main: HWND,
    hwnd: HWND,
    theme: Theme,
    /// Tamaño del área en DIPs (el que pidió la `App`).
    size: (f32, f32),
    tracking: bool,
}

thread_local! {
    static AUX: RefCell<Option<Aux>> = const { RefCell::new(None) };
    /// Aparte de `AUX`: dibujar no debe tener el estado prestado.
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
    /// Dónde la dejó el usuario (esquina de arriba a la izquierda, en pantalla), para la sesión.
    static PLACE: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Aux) -> R) -> Option<R> {
    AUX.with(|a| {
        let mut a = a.try_borrow_mut().ok()?;
        a.as_mut().map(f)
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

/// La `App` de la ventana principal (la guarda `wndproc` en `GWLP_USERDATA`).
fn with_app(main: HWND, f: impl FnOnce(&mut App)) {
    let app = unsafe { GetWindowLongPtrW(main, GWLP_USERDATA) } as *mut App;
    if !app.is_null() {
        // SAFETY: la `App` vive mientras exista la ventana principal (se suelta en su
        // `WM_NCDESTROY`, después de destruir esta, que es su dueña). Los mensajes llegan en el
        // hilo de interfaz, como los de la principal.
        f(unsafe { &mut *app });
    }
}

fn dpi_of(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96)
}

/// Tamaño de la ventana (con el borde del sistema) para un área de `size` DIPs.
fn window_size(hwnd: HWND, size: (f32, f32)) -> SIZE {
    let dpi = dpi_of(hwnd);
    let k = dpi as f32 / 96.0;
    let client = SIZE { cx: (size.0 * k).round() as i32, cy: (size.1 * k).round() as i32 };
    let mut border = RECT::default();
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut border, style(), false, WINDOW_EX_STYLE::default(), dpi);
    }
    client_to_window_size(client, border)
}

fn apply_dark(hwnd: HWND, dark: bool) {
    let dark = BOOL::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const std::ffi::c_void, size_of::<BOOL>() as u32);
    }
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn ensure(main: HWND, theme: &Theme) -> Option<HWND> {
    if let Some(h) = with(|a| a.hwnd)
        && unsafe { IsWindow(Some(h)) }.as_bool()
    {
        return Some(h);
    }
    shutdown();
    static REGISTER: Once = Once::new();
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        REGISTER.call_once(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(aux_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class);
        });
        let renderer = Renderer::new().ok()?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Copiando…"),
            style(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            560,
            400,
            Some(main),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        apply_dark(hwnd, theme.dark);
        AUX.with(|a| *a.borrow_mut() = Some(Aux { main, hwnd, theme: *theme, size: (0.0, 0.0), tracking: false }));
        RENDERER.with(|r| *r.borrow_mut() = Some(renderer));
        with_renderer(|r| r.set_dpi(dpi_of(hwnd)));
        Some(hwnd)
    }
}

/// Lo de `WindowHost::aux_show`: crea la ventana o la trae al frente, con `title` y un área de
/// `size` DIPs. `false` si no se pudo crear.
pub fn show(main: HWND, w: AuxWindow, title: &str, size: (f32, f32), theme: &Theme) -> bool {
    let AuxWindow::Transfers = w;
    let Some(hwnd) = ensure(main, theme) else { return false };
    with(|a| a.size = size);
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let ws = window_size(hwnd, size);
    unsafe {
        let _ = SetWindowTextW(hwnd, &HSTRING::from(title));
        if visible {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, ws.cx, ws.cy, SWP_NOMOVE | SWP_NOACTIVATE);
        } else {
            let (x, y) = PLACE.with(|p| p.get()).unwrap_or_else(|| {
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let work = if GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
                    info.rcWork
                } else {
                    RECT { left: 0, top: 0, right: 1280, bottom: 800 }
                };
                let r = initial_rect(work, ws);
                (r.left, r.top)
            });
            let _ = SetWindowPos(hwnd, Some(HWND_TOP), x, y, ws.cx, ws.cy, SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    invalidate(hwnd);
    true
}

/// El contenido cambió de alto.
pub fn resize(w: AuxWindow, size: (f32, f32)) {
    let AuxWindow::Transfers = w;
    let Some(hwnd) = with(|a| {
        a.size = size;
        a.hwnd
    }) else {
        return;
    };
    let ws = window_size(hwnd, size);
    unsafe {
        let _ = SetWindowPos(hwnd, None, 0, 0, ws.cx, ws.cy, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    invalidate(hwnd);
}

pub fn invalidate_aux(w: AuxWindow) {
    let AuxWindow::Transfers = w;
    if let Some(hwnd) = with(|a| a.hwnd) {
        invalidate(hwnd);
    }
}

/// La oculta y recuerda dónde estaba.
pub fn hide(w: AuxWindow) {
    let AuxWindow::Transfers = w;
    if let Some(hwnd) = with(|a| a.hwnd) {
        remember(hwnd);
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn remember(hwnd: HWND) {
    unsafe {
        if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_ok() {
                PLACE.with(|p| p.set(Some((r.left, r.top))));
            }
        }
    }
}

/// El tema cambió (lo llama el host al dibujar la principal).
pub fn set_theme(theme: &Theme) {
    let changed = with(|a| {
        let t = &a.theme;
        let same = t.dark == theme.dark && t.accent == theme.accent && t.surface == theme.surface && t.text == theme.text;
        a.theme = *theme;
        (!same).then_some(a.hwnd)
    });
    if let Some(Some(hwnd)) = changed {
        apply_dark(hwnd, theme.dark);
        invalidate(hwnd);
    }
}

/// Al cerrar xdir: se sueltan el estado y el renderizador en este hilo (la ventana la destruye
/// Windows con su dueña).
pub fn shutdown() {
    AUX.with(|a| {
        if let Ok(mut a) = a.try_borrow_mut() {
            *a = None;
        }
    });
    let renderer = RENDERER.with(|r| r.try_borrow_mut().ok().and_then(|mut r| r.take()));
    drop(renderer);
}

fn paint(hwnd: HWND) {
    let Some((main, theme)) = with(|a| (a.main, a.theme)) else { return };
    let dpi = dpi_of(hwnd);
    with_renderer(|r| {
        let _ = r.draw(handle_from_hwnd(hwnd), dpi, &theme, &[], |p, w, h| {
            with_app(main, |app| app.paint_aux(AuxWindow::Transfers, p, w, h));
        });
    });
}

fn mouse_dips(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
    let k = dpi_of(hwnd) as f32 / 96.0;
    (x / k, y / k)
}

fn mouse(hwnd: HWND, lparam: LPARAM, ev: AuxMouse) {
    let Some(main) = with(|a| a.main) else { return };
    let (x, y) = mouse_dips(hwnd, lparam);
    with_app(main, |app| app.aux_mouse(AuxWindow::Transfers, x, y, ev));
}

unsafe extern "system" fn aux_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
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
                let dpi = dpi_of(hwnd);
                with_renderer(|r| r.resize(width, height, dpi));
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xFFFF) as u32;
                with_renderer(|r| r.set_dpi(dpi));
                let r = &*(lparam.0 as *const RECT);
                // El área sigue midiendo lo mismo en DIPs: el alto se recalcula con el DPI nuevo.
                let size = with(|a| a.size).unwrap_or((560.0, 400.0));
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                let ws = window_size(hwnd, size);
                let _ = SetWindowPos(hwnd, None, 0, 0, ws.cx, ws.cy, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if with(|a| std::mem::replace(&mut a.tracking, true)) == Some(false) {
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                }
                mouse(hwnd, lparam, AuxMouse::Move);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                mouse(hwnd, lparam, AuxMouse::Down);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                mouse(hwnd, lparam, AuxMouse::Up);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with(|a| a.tracking = false);
                mouse(hwnd, LPARAM(-1), AuxMouse::Leave);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let mut used = false;
                if let Some(main) = with(|a| a.main) {
                    with_app(main, |app| used = app.aux_key(AuxWindow::Transfers, Key(wparam.0 as u16)));
                }
                if used { LRESULT(0) } else { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            // Cerrar (la ✕ o Alt+F4) la oculta: los trabajos siguen y queda el indicador.
            WM_CLOSE => {
                remember(hwnd);
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Some(main) = with(|a| a.main) {
                    with_app(main, |app| app.aux_closed(AuxWindow::Transfers));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_position_bottom_right() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let r = initial_rect(work, SIZE { cx: 576, cy: 450 });
        assert_eq!((r.left, r.top, r.right, r.bottom), (1920 - 16 - 576, 1040 - 16 - 450, 1920 - 16, 1040 - 16));
        // Un segundo monitor a la izquierda (coordenadas negativas).
        let work = RECT { left: -1280, top: 0, right: 0, bottom: 760 };
        let r = initial_rect(work, SIZE { cx: 576, cy: 450 });
        assert_eq!((r.right, r.bottom), (-16, 744));
        // Más grande que el área: queda pegada arriba a la izquierda.
        let r = initial_rect(RECT { left: 0, top: 0, right: 400, bottom: 300 }, SIZE { cx: 576, cy: 450 });
        assert_eq!((r.left, r.top), (0, 0));
    }

    #[test]
    fn client_to_window_size_adds_border() {
        // Borde de 8 a los lados y abajo y barra de 31 arriba (lo típico a 96 DPI).
        let border = RECT { left: -8, top: -31, right: 8, bottom: 8 };
        let s = client_to_window_size(SIZE { cx: 560, cy: 410 }, border);
        assert_eq!((s.cx, s.cy), (576, 449));
        // Y con el borde de verdad del sistema, el área queda igual.
        let mut real = RECT::default();
        unsafe {
            let _ = AdjustWindowRectExForDpi(&mut real, style(), false, WINDOW_EX_STYLE::default(), 96);
        }
        let s = client_to_window_size(SIZE { cx: 560, cy: 410 }, real);
        assert!(s.cx > 560 && s.cy > 410);
    }
}

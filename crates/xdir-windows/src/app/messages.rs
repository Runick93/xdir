//! Despacho de mensajes de la ventana (`WM_*`) hacia los métodos de `App` (solo Windows).
//! Lo que no es propio de Win32 está en `events.rs`.

use super::win::*;
use super::{App, CursorKind};
use super::host_win::Win32Host;
use super::uia;
use crate::drop::{DragEvent, WM_APP_DRAG};
use crate::platform::{Key, WM_APP};
use crate::shell;

/// Atiende un mensaje de la ventana `hwnd` (la de `app`).
pub(super) fn handle(app: &mut App, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Las imágenes grandes terminadas pasan a la caché aunque no se dibuje ninguna.
    if msg == xdir_core::paint::WM_APP_ICONS
        && let Some(host) = Win32Host::current()
    {
        host.absorb_big();
    }
    if msg >= WM_APP && app.on_app_message(msg, wparam.0, lparam.0) {
        return LRESULT(0);
    }
    unsafe {
        match msg {
            WM_PAINT => {
                // BeginPaint valida antes de pintar (si `paint` pide otro frame, no se
                // pierde) y dispara el borrado de fondo cuando hace falta.
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
                app.paint();
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                // Con Mica, la superficie GDI de la ventana va en negro: con el marco
                // extendido, el negro es transparente y deja ver la composición y el fondo.
                // En modo opaco todo el fondo lo pinta Direct2D.
                if app.transparent() {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    FillRect(HDC(wparam.0 as *mut _), &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                }
                LRESULT(1)
            }
            WM_SIZE => {
                let width = (lparam.0 & 0xFFFF) as u32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                if let Some(host) = Win32Host::current() {
                    host.resize(width, height, app.dpi());
                }
                app.on_resized();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                app.set_dpi((wparam.0 & 0xFFFF) as u32);
                app.on_resized();
                if let Some(host) = Win32Host::current() {
                    host.set_dpi(app.dpi());
                }
                app.restyle_text();
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize.x = scale(560, app.dpi());
                info.ptMinTrackSize.y = scale(360, app.dpi());
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                // "Mostrar animaciones en Windows" (Accesibilidad → Efectos visuales).
                app.apply_animation_setting();
                // Windows avisa así cuando el usuario cambia entre modo claro y oscuro.
                let area = PCWSTR(lparam.0 as *const u16);
                if !area.is_null() && area.to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                    app.on_system_theme_changed();
                }
                LRESULT(0)
            }
            WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                let (x, y) = app.cursor_pos();
                let cursor = match app.cursor_at(x, y) {
                    CursorKind::SizeWE => IDC_SIZEWE,
                    CursorKind::SizeNS => IDC_SIZENS,
                    CursorKind::Arrow => IDC_ARROW,
                };
                if let Ok(c) = LoadCursorW(None, cursor) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            // Se enchufó (0x8000) o se quitó (0x8004) una unidad: se actualiza como el Explorador.
            WM_DEVICECHANGE => {
                if matches!(wparam.0, 0x8000 | 0x8004) {
                    app.drives_changed();
                }
                LRESULT(1)
            }
            WM_CTLCOLOREDIT => match Win32Host::current() {
                Some(host) => host.ctl_color_edit(HDC(wparam.0 as *mut _)),
                None => LRESULT(0),
            },
            // El filtro y la paleta se actualizan con cada tecla.
            WM_COMMAND if (wparam.0 >> 16) & 0xFFFF == EN_CHANGE_CODE => {
                let edit = HWND(lparam.0 as *mut _);
                if let Some(b) = Win32Host::current().and_then(|h| h.text_box_of(edit)) {
                    app.on_text_changed(b.0 as isize);
                }
                LRESULT(0)
            }
            // Solo con `--features pruebas`: otro proceso maneja la app por texto (capturas y
            // pruebas automáticas sin mouse ni teclado reales).
            #[cfg(feature = "pruebas")]
            WM_COPYDATA => {
                let cds = &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                let wide = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
                let text = String::from_utf16_lossy(wide);
                app.test_hook(text.trim_end_matches('\0'));
                LRESULT(1)
            }
            // Clic derecho dentro de un menú emergente (el de favoritos abre ahí su menú contextual).
            WM_MENURBUTTONUP => {
                if Win32Host::current().is_some_and(|h| h.on_menu_right_click(lparam.0, wparam.0 as u32)) {
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            // Al volver a la ventana se releen los favoritos (por si se editó el archivo).
            WM_ACTIVATEAPP if wparam.0 != 0 => {
                app.on_app_activated();
                LRESULT(0)
            }
            // La ventana de la vista rápida: una tecla suya (o ‹ › ×) y el cierre con Alt+F4.
            crate::quicklook::WM_APP_QL_KEY => {
                app.on_key(Key(wparam.0 as u16));
                LRESULT(0)
            }
            crate::quicklook::WM_APP_QL_CLOSED => {
                app.on_quick_look_closed();
                LRESULT(0)
            }
            WM_APP_DRAG => {
                let ev = &mut *(lparam.0 as *mut DragEvent);
                app.on_drag(ev);
                LRESULT(0)
            }
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR => {
                match shell::win32::handle_menu_message(msg, wparam, lparam) {
                    Some(r) => r,
                    None => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_RBUTTONDOWN => {
                let (x, y) = mouse_pos(app, lparam);
                app.on_right_down(x, y);
                LRESULT(0)
            }
            WM_CONTEXTMENU => {
                // lparam -1: tecla de menú o Shift+F10.
                let at = (lparam.0 != -1).then(|| {
                    let mut pt = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    let k = 96.0 / app.dpi() as f32;
                    (pt.x as f32 * k, pt.y as f32 * k)
                });
                app.on_context_menu_request(at);
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if app.on_key(Key(wparam.0 as u16)) {
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_SYSKEYDOWN => {
                // Alt+flechas como en el Explorador; el resto (Alt+F4, Alt+Espacio) sigue de largo.
                if app.on_alt_key(Key(wparam.0 as u16)) {
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CHAR => {
                if let Some(c) = char::from_u32(wparam.0 as u32) {
                    app.on_char(c);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = mouse_pos(app, lparam);
                // El desplegable de la barra de dirección no le quita el foco a la caja.
                app.focus_for_click(x, y);
                app.on_left_down(x, y, msg == WM_LBUTTONDBLCLK);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = mouse_pos(app, lparam);
                app.on_left_up(x, y);
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                let (x, y) = mouse_pos(app, lparam);
                app.on_middle_click(x, y);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = mouse_pos(app, lparam);
                app.on_mouse_move(x, y);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if let Some(host) = Win32Host::current() {
                    host.mouse_left();
                }
                app.on_mouse_left();
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let (x, y) = app.cursor_pos();
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                app.on_wheel(x, y, delta, wheel_lines());
                LRESULT(0)
            }
            // Rueda horizontal (touchpad, mouse con inclinación).
            0x020E => {
                let (x, y) = app.cursor_pos();
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                app.on_wheel_h(x, y, delta, wheel_lines());
                LRESULT(0)
            }
            WM_XBUTTONUP => {
                let (x, y) = mouse_pos(app, lparam);
                match ((wparam.0 >> 16) & 0xFFFF) as u16 {
                    1 => app.on_back_forward(x, y, false),
                    2 => app.on_back_forward(x, y, true),
                    _ => {}
                }
                LRESULT(1)
            }
            WM_CLOSE => {
                // Con operaciones de copia en curso se pregunta; «No» o «Cancelar» no cierran.
                if !app.confirm_close() {
                    return LRESULT(0);
                }
                app.on_closing();
                // Un visor de la vista rápida quedó colgado en `prevhost.exe`: destruir las
                // ventanas esperaría por él para siempre. Con el estado ya guardado, se sale.
                if crate::quicklook::exit_now() {
                    std::process::exit(0);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            // Un lector de pantalla pregunta por la raíz de UI Automation (ver uia.rs).
            WM_GETOBJECT if lparam.0 as i32 == windows::Win32::UI::Accessibility::UiaRootObjectId => {
                let provider = uia::root_provider(app);
                windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider)
            }
            WM_DESTROY => {
                // Se desconectan los lectores: los proveedores que les quedan dejan de responder.
                uia::shutdown();
                let _ = windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(
                    hwnd,
                    WPARAM(0),
                    LPARAM(0),
                    None::<&windows::Win32::UI::Accessibility::IRawElementProviderSimple>,
                );
                let _ = RevokeDragDrop(hwnd);
                crate::quicklook::shutdown();
                if let Some(host) = Win32Host::current() {
                    host.release_drop_target();
                    host.release_edit_resources();
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Posición del mouse de un mensaje (`lparam`, en píxeles) en DIPs.
fn mouse_pos(app: &App, lparam: LPARAM) -> (f32, f32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
    let k = 96.0 / app.dpi() as f32;
    (x * k, y * k)
}

/// Líneas por "clic" de la rueda que fija Windows (las da el host).
fn wheel_lines() -> f32 {
    use xdir_core::host::WindowHost;
    Win32Host::current().map_or(3.0, |h| h.wheel_lines())
}

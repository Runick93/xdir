//! Despacho de mensajes de la ventana (`WM_*`) hacia los métodos de `App` (solo Windows).
//! Lo que no es propio de Win32 está en `events.rs`.

use super::*;
use super::win::*;

impl App {
    pub(super) fn handle(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg >= crate::platform::WM_APP && self.on_app_message(msg, wparam.0, lparam.0) {
            return LRESULT(0);
        }
        unsafe {
            match msg {
                WM_PAINT => {
                    // BeginPaint valida antes de pintar (si `paint` pide otro frame, no se
                    // pierde) y dispara el borrado de fondo cuando hace falta.
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(self.hwnd.hwnd(), &mut ps);
                    let _ = EndPaint(self.hwnd.hwnd(), &ps);
                    self.paint();
                    LRESULT(0)
                }
                WM_ERASEBKGND => {
                    // Con Mica, la superficie GDI de la ventana va en negro: con el marco
                    // extendido, el negro es transparente y deja ver la composición y el fondo.
                    // En modo opaco todo el fondo lo pinta Direct2D.
                    if self.transparent {
                        let mut rc = RECT::default();
                        let _ = GetClientRect(self.hwnd.hwnd(), &mut rc);
                        FillRect(HDC(wparam.0 as *mut _), &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                    }
                    LRESULT(1)
                }
                WM_SIZE => {
                    let width = (lparam.0 & 0xFFFF) as u32;
                    let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                    self.renderer.resize(width, height, self.dpi);
                    self.on_resized();
                    LRESULT(0)
                }
                WM_DPICHANGED => {
                    self.dpi = (wparam.0 & 0xFFFF) as u32;
                    self.on_resized();
                    self.renderer.set_dpi(self.dpi);
                    self.update_edit_resources();
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd.hwnd(),
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
                    info.ptMinTrackSize.x = scale(560, self.dpi);
                    info.ptMinTrackSize.y = scale(360, self.dpi);
                    LRESULT(0)
                }
                WM_SETTINGCHANGE => {
                    // "Mostrar animaciones en Windows" (Accesibilidad → Efectos visuales).
                    self.apply_animation_setting();
                    // Windows avisa así cuando el usuario cambia entre modo claro y oscuro.
                    let area = PCWSTR(lparam.0 as *const u16);
                    if !area.is_null() && area.to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        self.on_system_theme_changed();
                    }
                    LRESULT(0)
                }
                WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                    let (x, y) = self.cursor_pos();
                    let cursor = match self.cursor_at(x, y) {
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
                        self.drives_changed();
                    }
                    LRESULT(1)
                }
                WM_CTLCOLOREDIT => {
                    let hdc = HDC(wparam.0 as *mut _);
                    SetTextColor(hdc, colorref(self.theme.text));
                    SetBkColor(hdc, colorref(self.theme.surface_alt));
                    LRESULT(self.sys.edit_brush.0 as isize)
                }
                // El filtro y la paleta se actualizan con cada tecla.
                WM_COMMAND if (wparam.0 >> 16) & 0xFFFF == EN_CHANGE_CODE => {
                    self.on_text_changed(lparam.0);
                    LRESULT(0)
                }
                // Solo con `--features pruebas`: otro proceso maneja la app por texto (capturas y
                // pruebas automáticas sin mouse ni teclado reales).
                #[cfg(feature = "pruebas")]
                WM_COPYDATA => {
                    let cds = &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                    let wide = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
                    let text = String::from_utf16_lossy(wide);
                    self.test_hook(text.trim_end_matches('\0'));
                    LRESULT(1)
                }
                WM_MENURBUTTONUP if self.sys.fav_menu.is_some() => {
                    self.on_fav_menu_right_click(lparam.0, wparam.0 as u32);
                    LRESULT(0)
                }
                // Al volver a la ventana se releen los favoritos (por si se editó el archivo).
                WM_ACTIVATEAPP if wparam.0 != 0 => {
                    self.on_app_activated();
                    LRESULT(0)
                }
                WM_APP_DRAG => {
                    let ev = &mut *(lparam.0 as *mut DragEvent);
                    self.on_drag(ev);
                    LRESULT(0)
                }
                WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR => {
                    match shell::win32::handle_menu_message(msg, wparam, lparam) {
                        Some(r) => r,
                        None => DefWindowProcW(self.hwnd.hwnd(), msg, wparam, lparam),
                    }
                }
                WM_RBUTTONDOWN => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_right_down(x, y);
                    LRESULT(0)
                }
                WM_CONTEXTMENU => {
                    // lparam -1: tecla de menú o Shift+F10.
                    let at = (lparam.0 != -1).then(|| {
                        let mut pt = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
                        let _ = ScreenToClient(self.hwnd.hwnd(), &mut pt);
                        let k = 96.0 / self.dpi as f32;
                        (pt.x as f32 * k, pt.y as f32 * k)
                    });
                    self.on_context_menu_request(at);
                    LRESULT(0)
                }
                WM_KEYDOWN => {
                    if self.on_key(Key(wparam.0 as u16)) {
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd.hwnd(), msg, wparam, lparam)
                    }
                }
                WM_SYSKEYDOWN => {
                    // Alt+flechas como en el Explorador; el resto (Alt+F4, Alt+Espacio) sigue de largo.
                    if self.on_alt_key(Key(wparam.0 as u16)) {
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd.hwnd(), msg, wparam, lparam)
                    }
                }
                WM_CHAR => {
                    if let Some(c) = char::from_u32(wparam.0 as u32) {
                        self.on_char(c);
                    }
                    LRESULT(0)
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    let (x, y) = self.mouse_pos(lparam);
                    // El desplegable de la barra de dirección no le quita el foco a la caja.
                    if !matches!(self.hit(x, y), Hit::Suggest(_) | Hit::Pane(_, PaneHit::AddressClear)) {
                        self.host.focus_main();
                    }
                    self.on_left_down(x, y, msg == WM_LBUTTONDBLCLK);
                    LRESULT(0)
                }
                WM_LBUTTONUP => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_left_up(x, y);
                    LRESULT(0)
                }
                WM_MBUTTONUP => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_middle_click(x, y);
                    LRESULT(0)
                }
                WM_MOUSEMOVE => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_mouse_move(x, y);
                    LRESULT(0)
                }
                WM_MOUSELEAVE => {
                    if let Some(host) = Win32Host::current() {
                        host.mouse_left();
                    }
                    self.on_mouse_left();
                    LRESULT(0)
                }
                WM_MOUSEWHEEL => {
                    let (x, y) = self.cursor_pos();
                    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                    self.on_wheel(x, y, delta, self.host.wheel_lines());
                    LRESULT(0)
                }
                // Rueda horizontal (touchpad, mouse con inclinación).
                0x020E => {
                    let (x, y) = self.cursor_pos();
                    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                    self.on_wheel_h(x, y, delta, self.host.wheel_lines());
                    LRESULT(0)
                }
                WM_XBUTTONUP => {
                    let (x, y) = self.mouse_pos(lparam);
                    match ((wparam.0 >> 16) & 0xFFFF) as u16 {
                        1 => self.on_back_forward(x, y, false),
                        2 => self.on_back_forward(x, y, true),
                        _ => {}
                    }
                    LRESULT(1)
                }
                WM_CLOSE => {
                    self.on_closing();
                    let _ = DestroyWindow(self.hwnd.hwnd());
                    LRESULT(0)
                }
                // Un lector de pantalla pregunta por la raíz de UI Automation (ver uia.rs).
                WM_GETOBJECT if lparam.0 as i32 == windows::Win32::UI::Accessibility::UiaRootObjectId => {
                    let provider = uia::root_provider(self);
                    windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(self.hwnd.hwnd(), wparam, lparam, &provider)
                }
                WM_DESTROY => {
                    // Se desconectan los lectores: los proveedores que les quedan dejan de responder.
                    uia::shutdown();
                    let _ = windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(
                        self.hwnd.hwnd(),
                        WPARAM(0),
                        LPARAM(0),
                        None::<&windows::Win32::UI::Accessibility::IRawElementProviderSimple>,
                    );
                    let _ = RevokeDragDrop(self.hwnd.hwnd());
                    let _ = DeleteObject(self.sys.edit_font.into());
                    let _ = DeleteObject(self.sys.edit_brush.into());
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(self.hwnd.hwnd(), msg, wparam, lparam),
            }
        }
    }

    pub(super) fn mouse_pos(&self, lparam: LPARAM) -> (f32, f32) {
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        let k = 96.0 / self.dpi as f32;
        (x * k, y * k)
    }
}

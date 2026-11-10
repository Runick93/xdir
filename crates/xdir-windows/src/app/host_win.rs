//! Windows: la ventana de la aplicación (`WindowHost`) sobre el `HWND` principal. Lee el DPI de la
//! ventana para pasar de píxeles a DIPs. macOS tiene lo mismo en `host_mac.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use xdir_core::a11y::El;
use xdir_core::host::{Accessibility, WindowHost};
use xdir_core::shell::Entry;
use xdir_core::paint::Painter;

use crate::config::ColorRule;
use crate::render::Renderer;

use std::collections::HashMap;

use super::INITIAL_SIZE;
use super::win::*;
use crate::drop;
use crate::platform::keys::{VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_PRIOR, VK_RETURN, VK_UP};
use crate::platform::{Color, Handle, HandleExt, Key};
use crate::theme::Theme;
use crate::{render, shell};
use super::uia;
use xdir_core::host::{MenuPos, TextBox, TextKind, WM_APP_EDIT_DONE, WM_APP_PALETTE_KEY, WM_APP_SUGGEST_KEY};

thread_local! {
    /// El host de la ventana, para el código de Win32 que necesita el tipo concreto (`wndproc`,
    /// `messages.rs`): la `App` lo tiene como `dyn WindowHost`.
    static HOST: RefCell<Option<Rc<Win32Host>>> = const { RefCell::new(None) };
}

/// La ventana principal de Windows.
pub(super) struct Win32Host {
    hwnd: Cell<Handle>,
    /// Ya se pidió el aviso de `WM_MOUSELEAVE` y todavía no llegó.
    tracking_mouse: Cell<bool>,
    /// Colores personalizados del selector de Windows, para que se recuerden entre usos.
    custom_colors: Cell<[COLORREF; 16]>,
    /// Destino de arrastre de la ventana: se mantiene vivo mientras ella exista.
    drop_target: RefCell<Option<IDropTarget>>,
    /// Direct2D / DirectWrite: fábricas, fuentes y el destino de dibujo de la ventana.
    /// Nunca se presta a través de una llamada que pueda bombear mensajes: un `WM_SIZE` o
    /// `WM_DPICHANGED` anidado durante `draw` lo pediría otra vez y entraría en pánico.
    renderer: RefCell<Renderer>,
    /// Fuente, fondo y colores (texto, fondo) de los controles de edición: siguen el DPI y el tema
    /// (los arma `text_restyle`).
    edit_font: Cell<HFONT>,
    edit_brush: Cell<HBRUSH>,
    edit_colors: Cell<(COLORREF, COLORREF)>,
    /// Controles de edición abiertos, por número de `TextBox`.
    text_boxes: RefCell<HashMap<u32, HWND>>,
    next_text_box: Cell<u32>,
    /// Menús emergentes abiertos (anidados: el último es el de más adentro). No se prestan
    /// mientras un menú está abierto: el clic derecho y los menús anidados llegan adentro.
    menus: RefCell<Vec<OpenMenu>>,
    /// Se está atendiendo un clic derecho dentro de un menú (no se atiende otro adentro).
    in_menu_context: Cell<bool>,
}

/// La respuesta al clic derecho dentro de un menú (la clausura de `popup_menu`, sin su tiempo de
/// vida: vive mientras el menú esté en la pila de `menus`).
type OnContext = *mut (dyn FnMut(&[usize]) -> bool + 'static);

/// Un menú emergente abierto con `popup_menu`.
struct OpenMenu {
    /// Camino de posiciones de cada elemento, por (submenú, posición dentro de él). Solo si hay
    /// `on_context`.
    positions: HashMap<(isize, u32), Vec<usize>>,
    on_context: Option<OnContext>,
    /// `on_context` pidió cerrarlo: `popup_menu` devuelve `None`.
    closed: bool,
}

impl Win32Host {
    /// Crea el host (todavía sin ventana) con su renderizador (arranca la creación del
    /// dispositivo de GPU en otro hilo) y lo deja en el `thread_local` de la plataforma.
    pub(super) fn install() -> windows::core::Result<Rc<Win32Host>> {
        let host = Rc::new(Win32Host {
            hwnd: Cell::new(Handle::default()),
            tracking_mouse: Cell::new(false),
            custom_colors: Cell::new([COLORREF(0x00FFFFFF); 16]),
            drop_target: RefCell::new(None),
            renderer: RefCell::new(Renderer::new()?),
            edit_font: Cell::new(HFONT::default()),
            edit_brush: Cell::new(HBRUSH::default()),
            edit_colors: Cell::new((COLORREF(0), COLORREF(0x00FFFFFF))),
            text_boxes: RefCell::new(HashMap::new()),
            next_text_box: Cell::new(1),
            menus: RefCell::new(Vec::new()),
            in_menu_context: Cell::new(false),
        });
        HOST.with(|h| *h.borrow_mut() = Some(host.clone()));
        Ok(host)
    }

    /// El host instalado con `install`; `None` antes de instalarlo o después de `uninstall` (un
    /// mensaje tardío no hace nada).
    pub(super) fn current() -> Option<Rc<Win32Host>> {
        HOST.with(|h| h.borrow().clone())
    }

    /// Pasa a la caché las imágenes grandes que terminaron (al llegar `WM_APP_ICONS`).
    pub(super) fn absorb_big(&self) {
        if let Ok(r) = self.renderer.try_borrow() {
            r.absorb_big();
        }
    }

    /// Saca el host del `thread_local` y lo suelta, en el hilo de la ventana y antes de que
    /// termine `run`: así el renderizador (D3D/DXGI/DComp/D2D) no queda para los destructores de
    /// `ExitProcess`. Se llama cuando la `App` ya se soltó (`WM_NCDESTROY`) y no queda otra copia.
    pub(super) fn uninstall() {
        let host = HOST.with(|h| h.borrow_mut().take());
        debug_assert!(host.as_ref().is_none_or(|h| Rc::strong_count(h) == 1), "queda otra copia del host");
        drop(host);
    }

    fn hwnd(&self) -> HWND {
        self.hwnd.get().hwnd()
    }

    /// DPI de la ventana (96 sin ventana todavía).
    fn dpi(&self) -> u32 {
        match unsafe { GetDpiForWindow(self.hwnd()) } {
            0 => 96,
            dpi => dpi,
        }
    }

    /// Registra la ventana como destino de arrastrar y soltar.
    pub(super) fn register_drop_target(&self) {
        *self.drop_target.borrow_mut() = drop::register(self.hwnd()).ok();
    }

    /// `WM_DESTROY`, después de `RevokeDragDrop`: suelta el destino de arrastre.
    pub(super) fn release_drop_target(&self) {
        self.drop_target.borrow_mut().take();
    }

    /// Llegó `WM_MOUSELEAVE`: el próximo movimiento tiene que volver a pedir el aviso.
    pub(super) fn mouse_left(&self) {
        self.tracking_mouse.set(false);
    }

    /// `WM_SIZE`: el área de la ventana cambió (en píxeles).
    pub(super) fn resize(&self, width: u32, height: u32, dpi: u32) {
        self.renderer.borrow_mut().resize(width, height, dpi);
    }

    /// `WM_DPICHANGED`: la ventana pasó a un monitor con otro DPI.
    pub(super) fn set_dpi(&self, dpi: u32) {
        self.renderer.borrow_mut().set_dpi(dpi);
    }

    /// El control de edición de una caja (copiado: el registro no queda prestado mientras
    /// Windows manda mensajes del control).
    fn edit_hwnd(&self, b: TextBox) -> Option<HWND> {
        self.text_boxes.borrow().get(&b.0).copied()
    }

    /// La caja de un control de edición (`EN_CHANGE` llega con el `HWND` del control).
    pub(super) fn text_box_of(&self, hwnd: HWND) -> Option<TextBox> {
        self.text_boxes.borrow().iter().find(|(_, h)| **h == hwnd).map(|(id, _)| TextBox(*id))
    }

    /// `WM_CTLCOLOREDIT`: colores del tema en los controles de edición.
    pub(super) fn ctl_color_edit(&self, hdc: HDC) -> LRESULT {
        let (text, back) = self.edit_colors.get();
        unsafe {
            SetTextColor(hdc, text);
            SetBkColor(hdc, back);
        }
        LRESULT(self.edit_brush.get().0 as isize)
    }

    /// `WM_DESTROY`: suelta la fuente y el fondo de los controles de edición.
    pub(super) fn release_edit_resources(&self) {
        unsafe {
            let _ = DeleteObject(self.edit_font.get().into());
            let _ = DeleteObject(self.edit_brush.get().into());
        }
    }

    /// `WM_MENURBUTTONUP`: clic derecho sobre el elemento `pos` del menú `hmenu`. Si es de un menú
    /// abierto con `on_context`, la llama con el camino del elemento y, si pide cerrar, cierra el
    /// menú. Devuelve si el clic era para un menú así.
    pub(super) fn on_menu_right_click(&self, hmenu: isize, pos: u32) -> bool {
        if self.in_menu_context.get() {
            return false;
        }
        let found = self
            .menus
            .borrow()
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, m)| Some((i, m.on_context?, m.positions.get(&(hmenu, pos))?.clone())));
        let Some((i, on_context, path)) = found else { return false };
        self.in_menu_context.set(true);
        // SAFETY: la clausura vive en el marco de `popup_menu` que registró el menú, que sigue
        // esperando en `TrackPopupMenuEx` (el menú todavía está en la pila); mientras tanto nadie
        // más la usa (`in_menu_context` evita que se llame de nuevo desde adentro).
        let close = unsafe { (*on_context)(&path) };
        self.in_menu_context.set(false);
        if close {
            if let Some(m) = self.menus.borrow_mut().get_mut(i) {
                m.closed = true;
            }
            unsafe {
                let _ = EndMenu();
            }
        }
        true
    }
}

impl WindowHost for Win32Host {
    fn attach(&self, window: Handle) {
        self.hwnd.set(window);
    }

    /// El primer cuadro crea el destino de dibujo (espera al hilo que crea el dispositivo de GPU).
    fn draw(&self, theme: &Theme, file_colors: &[ColorRule], paint: &mut dyn FnMut(&dyn Painter, f32, f32)) -> Result<(), String> {
        self.renderer
            .borrow_mut()
            .draw(self.hwnd.get(), self.dpi(), theme, file_colors, |p, w, h| paint(p, w, h))
            .map_err(|e| e.to_string())
    }

    fn text_width(&self, s: &str, bold: bool) -> f32 {
        self.renderer.borrow().text_width(s, bold)
    }

    fn invalidate(&self) {
        // Sin ventana todavía no hay nada que repintar: un HWND nulo invalidaría todas las ventanas.
        if self.hwnd.get().is_null() {
            return;
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd()), None, false);
        }
    }

    fn client_size(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd(), &mut rc);
        }
        let k = 96.0 / self.dpi() as f32;
        ((rc.right - rc.left) as f32 * k, (rc.bottom - rc.top) as f32 * k)
    }

    fn cursor_dips(&self) -> (f32, f32) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
            let _ = ScreenToClient(self.hwnd(), &mut pt);
        }
        let k = 96.0 / self.dpi() as f32;
        (pt.x as f32 * k, pt.y as f32 * k)
    }

    fn set_title(&self, title: &str) {
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = SetWindowTextW(self.hwnd(), PCWSTR(wide.as_ptr()));
        }
    }

    fn request_close(&self) {
        self.hwnd.get().post(WM_CLOSE, 0, 0);
    }

    fn focus_main(&self) {
        unsafe {
            let _ = SetFocus(Some(self.hwnd()));
        }
    }

    fn focus_main_if_lost(&self) {
        unsafe {
            if GetFocus().is_invalid() {
                let _ = SetFocus(Some(self.hwnd()));
            }
        }
    }

    fn capture_mouse(&self) {
        unsafe {
            SetCapture(self.hwnd());
        }
    }

    fn release_mouse(&self) {
        unsafe {
            let _ = ReleaseCapture();
        }
    }

    fn track_mouse_leave(&self) {
        if !self.tracking_mouse.get() {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd(),
                dwHoverTime: 0,
            };
            self.tracking_mouse.set(unsafe { TrackMouseEvent(&mut tme) }.is_ok());
        }
    }

    fn beep(&self) {
        unsafe {
            let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONWARNING);
        }
    }

    fn ask_yes_no_cancel(&self, title: &str, text: &str) -> Option<bool> {
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let answer =
            unsafe { MessageBoxW(Some(self.hwnd()), PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_YESNOCANCEL | MB_ICONQUESTION) };
        if answer == IDYES {
            Some(true)
        } else if answer == IDNO {
            Some(false)
        } else {
            None
        }
    }

    fn open_url(&self, url: &str) {
        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        unsafe {
            ShellExecuteW(Some(self.hwnd()), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }

    /// Abre el archivo con el programa asociado; si `.toml` no tiene ninguno, con el Bloc de notas.
    fn open_in_editor(&self, path: &std::path::Path) {
        use std::os::windows::ffi::OsStrExt;
        let hwnd = self.hwnd();
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let r = ShellExecuteW(Some(hwnd), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
            // Menor o igual a 32 es un error (p. ej. SE_ERR_NOASSOC: la extensión no tiene programa).
            if r.0 as isize <= 32 {
                let arg: Vec<u16> = format!("\"{}\"", path.display()).encode_utf16().chain(Some(0)).collect();
                let _ = ShellExecuteW(Some(hwnd), w!("open"), w!("notepad.exe"), PCWSTR(arg.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
            }
        }
    }

    /// Líneas por "clic" de la rueda según la configuración de Windows.
    fn wheel_lines(&self) -> f32 {
        let mut lines = 3u32;
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                Some(&mut lines as *mut _ as *mut std::ffi::c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }
        // u32::MAX significa "una pantalla por clic"; se aproxima con 10 filas.
        if lines == u32::MAX { 10.0 } else { lines as f32 }
    }

    /// Expande variables de entorno: "%USERPROFILE%\Descargas".
    fn expand_env(&self, s: &str) -> String {
        let src: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
        let mut buf = vec![0u16; 32 * 1024];
        let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) } as usize;
        if n == 0 || n > buf.len() {
            return s.to_string();
        }
        String::from_utf16_lossy(&buf[..n - 1])
    }

    /// Barra de título oscura y fondo Mica (Windows 11). Si Mica quedó activo, el marco se
    /// extiende a toda la ventana para que el fondo se vea donde la aplicación no pinta. En
    /// Windows la barra de título sigue al tema siempre (`follow_system` no cambia nada).
    fn apply_theme(&self, theme: &Theme, _follow_system: bool) -> bool {
        use std::ffi::c_void;
        use windows::Win32::Graphics::Dwm::{
            DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea,
            DwmSetWindowAttribute,
        };
        use windows::Win32::UI::Controls::MARGINS;
        use windows::core::BOOL;
        let hwnd = self.hwnd();
        let dark = BOOL::from(theme.dark);
        unsafe {
            // Los errores se ignoran: en versiones viejas de Windows el atributo no existe.
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as *const c_void,
                size_of::<BOOL>() as u32,
            );
            let backdrop = DWMSBT_MAINWINDOW;
            let mica = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop as *const _ as *const c_void,
                size_of_val(&backdrop) as u32,
            )
            .is_ok();
            let mica = mica && {
                let all = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
                DwmExtendFrameIntoClientArea(hwnd, &all).is_ok()
            };
            // Con Mica el renderizador presenta con DirectComposition (fondo translúcido).
            self.renderer.borrow_mut().set_transparent(mica);
            mica
        }
    }

    /// El selector de color de Windows (espectro, RGB y HSL), modal.
    fn choose_color(&self, current: u32) -> Option<u32> {
        // COLORREF es 0x00BBGGRR.
        let bgr = |c: u32| COLORREF((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16));
        let mut cc = CHOOSECOLORW {
            lStructSize: size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: self.hwnd(),
            rgbResult: bgr(current),
            lpCustColors: self.custom_colors.as_ptr() as *mut COLORREF,
            Flags: CC_FULLOPEN | CC_RGBINIT | CC_ANYCOLOR,
            ..Default::default()
        };
        if !unsafe { ChooseColorW(&mut cc) }.as_bool() {
            return None;
        }
        let c = cc.rgbResult.0;
        Some((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF)
    }

    /// Arrastre del Shell (bucle modal: mientras dura, los eventos de soltar llegan por
    /// `WM_APP_DRAG`). Cancelar con Esc no es un error; que Windows no pueda arrastrar, sí.
    fn start_drag(&self, items: &[shell::Item]) -> Result<(), String> {
        let obj = shell::win32::data_object(items).map_err(|e| e.to_string())?;
        unsafe {
            let _ = ReleaseCapture();
            SHDoDragDrop(Some(self.hwnd()), &obj, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)
        }
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// Posición y tamaño normales (en píxeles de pantalla) y si está maximizada.
    fn placement(&self) -> Option<([i32; 4], bool)> {
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        unsafe { GetWindowPlacement(self.hwnd(), &mut wp) }.ok()?;
        let r = wp.rcNormalPosition;
        Some(([r.left, r.top, r.right - r.left, r.bottom - r.top], wp.showCmd == SW_SHOWMAXIMIZED.0 as u32))
    }

    fn show(&self, saved: Option<([i32; 4], bool)>) {
        let hwnd = self.hwnd();
        unsafe {
            match saved {
                Some(([x, y, w, h], maximized)) => {
                    let wp = WINDOWPLACEMENT {
                        length: size_of::<WINDOWPLACEMENT>() as u32,
                        showCmd: if maximized { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                        rcNormalPosition: RECT { left: x, top: y, right: x + w, bottom: y + h },
                        ..Default::default()
                    };
                    // Ajusta solo si la posición guardada quedó fuera de los monitores actuales.
                    let _ = SetWindowPlacement(hwnd, &wp);
                }
                None => {
                    let dpi = GetDpiForWindow(hwnd);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        scale(INITIAL_SIZE.0, dpi),
                        scale(INITIAL_SIZE.1, dpi),
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
            }
        }
    }

    /// Control de edición nativo de una línea, hijo de la ventana.
    fn text_create(&self, kind: TextKind, r: render::Rect, text: &str, _theme: &Theme) -> Option<TextBox> {
        let k = self.dpi() as f32 / 96.0;
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
        if matches!(kind, TextKind::Rename | TextKind::FavRename) {
            style |= WS_BORDER;
        }
        unsafe {
            let edit = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                style,
                (r.left * k) as i32,
                (r.top * k) as i32,
                ((r.right - r.left) * k) as i32,
                ((r.bottom - r.top) * k) as i32,
                Some(self.hwnd()),
                None,
                None,
                None,
            )
            .ok()?;
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.get().0 as usize)), Some(LPARAM(1)));
            if kind == TextKind::Palette {
                let _ = SetWindowSubclass(edit, Some(palette_proc), 2, 0);
            } else {
                let _ = SetWindowSubclass(edit, Some(edit_proc), 1, kind.done_msg() as usize);
            }
            let id = self.next_text_box.replace(self.next_text_box.get() + 1);
            self.text_boxes.borrow_mut().insert(id, edit);
            Some(TextBox(id))
        }
    }

    fn text_get(&self, b: TextBox) -> String {
        self.edit_hwnd(b).map(window_text).unwrap_or_default()
    }

    fn text_set(&self, b: TextBox, text: &str) {
        let Some(edit) = self.edit_hwnd(b) else { return };
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
        }
    }

    fn text_select(&self, b: TextBox, from: usize, to: isize) {
        let Some(edit) = self.edit_hwnd(b) else { return };
        unsafe {
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(from)), Some(LPARAM(to)));
        }
    }

    fn text_focus(&self, b: TextBox) {
        let Some(edit) = self.edit_hwnd(b) else { return };
        unsafe {
            let _ = SetFocus(Some(edit));
        }
    }

    fn text_has_focus(&self, b: TextBox) -> bool {
        self.edit_hwnd(b).is_some_and(|edit| unsafe { GetFocus() == edit })
    }

    fn text_destroy(&self, b: TextBox) {
        let removed = self.text_boxes.borrow_mut().remove(&b.0);
        if let Some(edit) = removed {
            unsafe {
                let _ = DestroyWindow(edit);
            }
        }
    }

    /// Fuente y fondo de los controles de edición según el DPI y el tema (los que ya están
    /// abiertos conservan su fuente: se cierran al cambiar el tamaño).
    fn text_restyle(&self, theme: &Theme) {
        unsafe {
            if !self.edit_font.get().is_invalid() {
                let _ = DeleteObject(self.edit_font.get().into());
            }
            if !self.edit_brush.get().is_invalid() {
                let _ = DeleteObject(self.edit_brush.get().into());
            }
            self.edit_font.set(CreateFontW(
                -(13 * self.dpi() as i32 / 96),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            ));
            self.edit_brush.set(CreateSolidBrush(colorref(theme.surface_alt)));
        }
        self.edit_colors.set((colorref(theme.text), colorref(theme.surface_alt)));
    }

    /// Menú emergente modal (`TrackPopupMenuEx`). Con `on_context`, el clic derecho dentro del
    /// menú llega por `WM_MENURBUTTONUP` a `on_menu_right_click`.
    fn popup_menu(&self, entries: &[Entry], pos: MenuPos, dot: Option<(u32, Color)>, on_context: Option<xdir_core::host::OnContext<'_>>) -> Option<u32> {
        /// Agrega las entradas a `hmenu`. `path`: camino del submenú; `positions`: dónde anotar
        /// el camino de cada elemento (solo si hace falta para el clic derecho). En los menús, "&"
        /// marca el atajo: se duplica.
        unsafe fn append(hmenu: HMENU, entries: &[Entry], path: &mut Vec<usize>, positions: &mut Option<HashMap<(isize, u32), Vec<usize>>>) {
            let mut pos = 0u32;
            for (i, e) in entries.iter().enumerate() {
                path.push(i);
                let added = unsafe {
                    match e {
                        Entry::Sep => {
                            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
                            true
                        }
                        Entry::Item { id, label, enabled, checked } => {
                            let mut flags = MF_STRING;
                            if !enabled {
                                flags |= MF_GRAYED;
                            }
                            if *checked {
                                flags |= MF_CHECKED;
                            }
                            let text: Vec<u16> = label.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                            let _ = AppendMenuW(hmenu, flags, *id as usize, PCWSTR(text.as_ptr()));
                            true
                        }
                        Entry::Sub(label, children) => match CreatePopupMenu() {
                            Ok(sub) => {
                                append(sub, children, path, positions);
                                let text: Vec<u16> = label.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                                let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, PCWSTR(text.as_ptr()));
                                true
                            }
                            Err(_) => false,
                        },
                    }
                };
                if added {
                    if let (Some(p), false) = (positions.as_mut(), matches!(e, Entry::Sep)) {
                        p.insert((hmenu.0 as isize, pos), path.clone());
                    }
                    pos += 1;
                }
                path.pop();
            }
        }
        let hwnd = self.hwnd();
        let dpi = self.dpi();
        let k = dpi as f32 / 96.0;
        unsafe {
            // Que se vea lo que cambió antes del menú (la flecha abierta de la barra de dirección).
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
            let hmenu = CreatePopupMenu().ok()?;
            let mut positions = on_context.is_some().then(HashMap::new);
            append(hmenu, entries, &mut Vec::new(), &mut positions);
            let bitmap = dot.and_then(|(id, color)| {
                let bitmap = accent_dot(color, dpi)?;
                let mii = MENUITEMINFOW { cbSize: size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, hbmpItem: bitmap, ..Default::default() };
                let _ = SetMenuItemInfoW(hmenu, id, false, &mii);
                Some(bitmap)
            });
            let client = |x: f32, y: f32| {
                let mut pt = POINT { x: (x * k) as i32, y: (y * k) as i32 };
                let _ = ClientToScreen(hwnd, &mut pt);
                pt
            };
            let cursor = || {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                pt
            };
            let (pt, flags) = match pos {
                MenuPos::At(x, y) => (client(x, y), TPM_LEFTALIGN | TPM_TOPALIGN),
                MenuPos::Above(x, y) => (client(x, y), TPM_LEFTALIGN | TPM_BOTTOMALIGN),
                MenuPos::Cursor => (cursor(), TPM_RIGHTBUTTON),
                MenuPos::CursorNested => (cursor(), TPM_RIGHTBUTTON | TPM_RECURSE),
            };
            // SAFETY: solo se le quita el tiempo de vida al puntero; se usa mientras este menú
            // está en la pila, y se saca antes de volver (ver `on_menu_right_click`).
            let on_context = on_context.map(|f| std::mem::transmute::<*mut (dyn FnMut(&[usize]) -> bool + '_), OnContext>(f as *mut _));
            self.menus.borrow_mut().push(OpenMenu { positions: positions.unwrap_or_default(), on_context, closed: false });
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | flags).0, pt.x, pt.y, hwnd, None).0;
            let menu = self.menus.borrow_mut().pop();
            let _ = DestroyMenu(hmenu);
            if let Some(b) = bitmap {
                let _ = DeleteObject(b.into());
            }
            // Si dentro del menú se eligió algo con clic derecho, el menú se cerró sin elección.
            if menu.is_some_and(|m| m.closed) {
                return None;
            }
            (cmd > 0).then_some(cmd as u32)
        }
    }

    fn accessibility(&self) -> Option<&dyn Accessibility> {
        Some(self)
    }
}

/// Los lectores de pantalla de Windows: UI Automation (ver `uia.rs`).
impl Accessibility for Win32Host {
    fn listening(&self) -> bool {
        uia::listening()
    }

    fn notify_focus(&self, el: El) {
        uia::notify_focus(self.hwnd(), el);
    }
}

/// Subclase de los controles de edición (dirección y renombrado): Enter confirma, Escape
/// cancela. `done_msg` es el mensaje que se le manda a la ventana al terminar.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    done_msg: usize,
) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd).unwrap_or_default();
        match msg {
            WM_KEYDOWN
                if done_msg as u32 == WM_APP_EDIT_DONE
                    && matches!(Key(wparam.0 as u16), VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT) =>
            {
                let _ = PostMessageW(Some(parent), WM_APP_SUGGEST_KEY, wparam, LPARAM(0));
                return LRESULT(0);
            }
            WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize || wparam.0 == VK_ESCAPE.0 as usize => {
                let commit = wparam.0 == VK_RETURN.0 as usize;
                let _ = PostMessageW(Some(parent), done_msg as u32, WPARAM(commit as usize), LPARAM(0));
                return LRESULT(0);
            }
            // Evita el "ding" de Enter/Escape en un edit de una línea.
            WM_CHAR if wparam.0 == '\r' as usize || wparam.0 == 0x1B => return LRESULT(0),
            // Al perder el foco: el renombrado se aplica (como en el Explorador), la dirección no.
            WM_KILLFOCUS => {
                let commit = done_msg as u32 != WM_APP_EDIT_DONE;
                let _ = PostMessageW(Some(parent), done_msg as u32, WPARAM(commit as usize), LPARAM(0));
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Subclase del control de edición de la paleta: las flechas, Re Pág / Av Pág, Enter, Escape y
/// Ctrl+P van a la ventana (que mueve la selección o abre el resultado); al perder el foco, avisa.
unsafe extern "system" fn palette_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd).unwrap_or_default();
        let ctrl = xdir_core::sys::env().key_down(VK_CONTROL);
        let post = |vk: usize| {
            let _ = PostMessageW(Some(parent), WM_APP_PALETTE_KEY, WPARAM(vk), LPARAM(ctrl as isize));
        };
        match msg {
            WM_KEYDOWN => {
                let vk = Key(wparam.0 as u16);
                if matches!(vk, VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN | VK_ESCAPE) || (ctrl && vk == Key(0x50)) {
                    post(wparam.0);
                    return LRESULT(0);
                }
            }
            // Evita el "ding" de Enter/Escape (y el carácter de Ctrl+P) en un edit de una línea.
            WM_CHAR if wparam.0 == '\r' as usize || wparam.0 == 0x1B || wparam.0 == 0x10 => return LRESULT(0),
            WM_KILLFOCUS => post(0),
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Texto de un control de edición.
fn window_text(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 32 * 1024];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Punto de color para marcar un ítem de menú (bitmap de 32 bits con alfa).
fn accent_dot(c: Color, dpi: u32) -> Option<windows::Win32::Graphics::Gdi::HBITMAP> {
    use windows::Win32::Graphics::Gdi::{BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateDIBSection, DIB_RGB_COLORS};
    let size = (16.0 * dpi as f32 / 96.0).round().max(8.0) as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        return None;
    }
    let px = unsafe { std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize) };
    let (c_r, c_g, c_b) = ((c.r * 255.0) as u32, (c.g * 255.0) as u32, (c.b * 255.0) as u32);
    let (center, radius) = (size as f32 / 2.0, size as f32 * 0.25);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - center, y as f32 + 0.5 - center);
            // Borde suave de un píxel; el color va premultiplicado por el alfa.
            let cover = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let a = (cover * 255.0) as u32;
            px[(y * size + x) as usize] = (a << 24) | ((c_r * a / 255) << 16) | ((c_g * a / 255) << 8) | (c_b * a / 255);
        }
    }
    Some(bitmap)
}

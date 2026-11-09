//! Windows: la ventana de la aplicación (`WindowHost`) sobre el `HWND` principal. Lee el DPI de la
//! ventana para pasar de píxeles a DIPs. macOS tiene lo mismo en `host_mac.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use xdir_core::host::WindowHost;
use xdir_core::paint::Painter;

use crate::config::ColorRule;
use crate::render::Renderer;

use super::*;

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
    renderer: RefCell<Renderer>,
    /// Fuente y fondo de los controles de edición (siguen el DPI y el tema; los arma
    /// `App::update_edit_resources`).
    pub(super) edit_font: Cell<HFONT>,
    pub(super) edit_brush: Cell<HBRUSH>,
    /// Menú de favoritos abierto (para el clic derecho dentro del menú). No se presta mientras
    /// el menú está abierto: el clic derecho llega anidado.
    pub(super) fav_menu: RefCell<Option<FavMenu>>,
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
            fav_menu: RefCell::new(None),
        });
        HOST.with(|h| *h.borrow_mut() = Some(host.clone()));
        Ok(host)
    }

    /// El host instalado con `install`.
    pub(super) fn current() -> Option<Rc<Win32Host>> {
        HOST.with(|h| h.borrow().clone())
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
}

//! Windows: arranque y ventana Win32, controles de edición nativos, menús emergentes, diálogos y
//! los métodos de `App` que son puro pegamento con el sistema. macOS tiene lo mismo en `mac.rs`.

use super::*;

pub(super) use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
pub(super) use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, ClientToScreen, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DeleteObject, EndPaint, FW_NORMAL, FillRect, GetStockObject, HBRUSH, HDC, HFONT, InvalidateRect,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, ScreenToClient, SetBkColor, SetTextColor,
};
pub(super) use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
pub(super) use windows::Win32::System::LibraryLoader::GetModuleHandleW;
pub(super) use windows::Win32::System::Ole::{
    DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, IDropTarget, OleInitialize, RevokeDragDrop,
};
pub(super) use windows::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
pub(super) use windows::Win32::UI::Controls::{EM_SETSEL, WM_MOUSELEAVE};
pub(super) use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
pub(super) use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
pub(super) use windows::Win32::UI::Shell::{DefSubclassProc, SHDoDragDrop, SetWindowSubclass, ShellExecuteW};
pub(super) use windows::Win32::UI::WindowsAndMessaging::*;
pub(super) use windows::core::{PCWSTR, w};

const CLASS_NAME: PCWSTR = w!("xdir.main");
/// Notificación de un control de edición: el texto cambió.
pub(super) const EN_CHANGE_CODE: usize = 0x0300;

// ---------- estado propio de Windows ----------

/// Lo que la ventana de Windows necesita guardar además del estado de la aplicación.
pub(super) struct Sys {
    /// Fuente y fondo de los controles de edición (siguen el DPI y el tema).
    pub edit_font: HFONT,
    pub edit_brush: HBRUSH,
    /// Menú de favoritos abierto (para el clic derecho dentro del menú).
    pub fav_menu: Option<FavMenu>,
}

impl Sys {
    pub(super) fn new() -> Self {
        Sys {
            edit_font: HFONT::default(),
            edit_brush: HBRUSH::default(),
            fav_menu: None,
        }
    }
}

/// Menú de favoritos abierto: a qué favorito corresponde cada comando y cada posición
/// (para el clic derecho dentro del menú).
pub(super) struct FavMenu {
    ids: Vec<FavPath>,
    positions: HashMap<(isize, u32), FavPath>,
    /// Acción elegida con clic derecho dentro del menú.
    action: Option<(FavPath, FavAct)>,
}

/// Arma el menú (y submenús) de favoritos. En los menús, "&" marca el atajo: se duplica.
unsafe fn build_fav_menu(menu: &mut FavMenu, hmenu: HMENU, entries: &[(FavPath, Fav)]) {
    for (pos, (path, fav)) in entries.iter().enumerate() {
        let label: Vec<u16> = fav.name().replace('&', "&&").encode_utf16().chain(Some(0)).collect();
        unsafe {
            match fav {
                Fav::Link { .. } => {
                    menu.ids.push(path.clone());
                    let _ = AppendMenuW(hmenu, MF_STRING, menu.ids.len(), PCWSTR(label.as_ptr()));
                }
                Fav::Group { items, .. } => {
                    let Ok(sub) = CreatePopupMenu() else { continue };
                    let children: Vec<(FavPath, Fav)> =
                        items.iter().enumerate().map(|(j, f)| ([path.as_slice(), &[j]].concat(), f.clone())).collect();
                    if children.is_empty() {
                        let _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, w!("(vacía)"));
                    }
                    build_fav_menu(menu, sub, &children);
                    let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, PCWSTR(label.as_ptr()));
                }
            }
        }
        menu.positions.insert((hmenu.0 as isize, pos as u32), path.clone());
    }
}

// ---------- arranque ----------

pub fn run() -> windows::core::Result<()> {
    // `xdir --xtree-mft <unidad> <archivo> <pid>`: proceso elevado de xtree (tecla A). Lee la MFT, deja
    // el árbol en el archivo y sale, sin ventana, sesión ni configuración.
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if args.first().is_some_and(|a| a == crate::fs::XTREE_HELPER_ARG) {
        std::process::exit(crate::fs::xtree_helper_main(&args[1..]));
    }
    let started = Instant::now();
    // Servicios del sistema para el núcleo: antes que nada (salvo el ayudante de arriba, que no
    // los usa), porque la `App` ya los usa.
    crate::sys::install();
    let (session, probe, start_path) = startup_args();

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        // OLE (STA): lo exigen el Shell, el portapapeles y arrastrar y soltar.
        OleInitialize(None)?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // Ícono del exe (recurso 1, ver assets/xdir.rc): grande para Alt+Tab, chico para la barra de tareas.
            hIcon: app_icon(instance.into(), SM_CXICON),
            hIconSm: app_icon(instance.into(), SM_CXSMICON),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let marks = vec![("init", ms_since(started))];
        let init = Init::load(session, probe, start_path, started, marks);
        shell::set_menu_mode(init.session.dark_mode);
        let host = Win32Host::install();
        let app = App::new(init, host.clone())?;
        let app = Box::into_raw(Box::new(app));

        // La ventana se crea oculta para aplicar tema y tamaño antes de mostrarla (sin flash blanco).
        // El HWND queda guardado en `App` desde WM_NCCREATE.
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS_NAME,
            w!("xdir"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(instance.into()),
            Some(app as *const _),
        )?;

        // Sin ícono en la barra de título (la barra de tareas y Alt+Tab siguen usando el de la clase).
        let no_icon = windows::Win32::UI::Controls::WTA_OPTIONS {
            dwFlags: windows::Win32::UI::Controls::WTNCA_NODRAWICON,
            dwMask: windows::Win32::UI::Controls::WTNCA_NODRAWICON,
        };
        let _ = windows::Win32::UI::Controls::SetWindowThemeAttribute(
            (*app).hwnd.hwnd(),
            windows::Win32::UI::Controls::WTA_NONCLIENT,
            &no_icon as *const _ as *const std::ffi::c_void,
            size_of::<windows::Win32::UI::Controls::WTA_OPTIONS>() as u32,
        );

        // Las carpetas visibles empiezan a leerse antes de mostrar la ventana.
        (*app).sync_visibility();
        host.register_drop_target();
        #[cfg(feature = "arbol")]
        (*app).start_quick_access();
        (*app).marks.push(("ventana creada", ms_since(started)));
        let saved = (*app).window_state.take();
        host.show(saved);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            let app = cs.lpCreateParams as *mut App;
            (*app).hwnd = crate::platform::handle_from_hwnd(hwnd);
            (*app).dpi = GetDpiForWindow(hwnd);
            (*app).host.attach((*app).hwnd);
            (*app).apply_window_theme();
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if app.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        if msg == WM_NCDESTROY {
            drop(Box::from_raw(app));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        (*app).handle(msg, wparam, lparam)
    }
}

// ---------- controles de edición ----------

/// Para qué es una caja de edición: decide qué mensaje manda al terminar y qué teclas reenvía.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TextKind {
    /// Barra de dirección: las flechas recorren el desplegable de rutas; perder el foco cancela.
    Address,
    /// Renombrado en línea: perder el foco aplica (como en el Explorador).
    Rename,
    /// Renombrado de un favorito.
    FavRename,
    /// Filtro rápido.
    Filter,
    /// Paleta: flechas, Re Pág / Av Pág, Enter, Escape y Ctrl+P van a la ventana.
    Palette,
}

impl TextKind {
    fn done_msg(self) -> u32 {
        match self {
            TextKind::Address => WM_APP_EDIT_DONE,
            TextKind::Rename => WM_APP_RENAME_DONE,
            TextKind::FavRename => WM_APP_FAVEDIT_DONE,
            TextKind::Filter => WM_APP_FILTER_DONE,
            TextKind::Palette => WM_APP_PALETTE_KEY,
        }
    }
}

/// Control de edición nativo de una línea, hijo de la ventana.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TextBox(HWND);

impl TextBox {
    /// Crea la caja sobre `r` (DIPs del área de la ventana) con `text`, sin foco ni selección.
    pub(super) fn create(app: &App, kind: TextKind, r: render::Rect, text: &str) -> Option<TextBox> {
        let k = app.dpi as f32 / 96.0;
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
                Some(app.hwnd.hwnd()),
                None,
                None,
                None,
            )
            .ok()?;
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(app.sys.edit_font.0 as usize)), Some(LPARAM(1)));
            if kind == TextKind::Palette {
                let _ = SetWindowSubclass(edit, Some(palette_proc), 2, 0);
            } else {
                let _ = SetWindowSubclass(edit, Some(edit_proc), 1, kind.done_msg() as usize);
            }
            Some(TextBox(edit))
        }
    }

    pub(super) fn id(self) -> isize {
        self.0.0 as isize
    }

    pub(super) fn text(self) -> String {
        window_text(self.0)
    }

    pub(super) fn set_text(self, text: &str) {
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = SetWindowTextW(self.0, PCWSTR(wide.as_ptr()));
        }
    }

    /// Selecciona de `from` a `to` (en unidades UTF-16; `to` = -1: hasta el final).
    pub(super) fn select(self, from: usize, to: isize) {
        unsafe {
            SendMessageW(self.0, EM_SETSEL, Some(WPARAM(from)), Some(LPARAM(to)));
        }
    }

    pub(super) fn focus(self) {
        unsafe {
            let _ = SetFocus(Some(self.0));
        }
    }

    pub(super) fn has_focus(self) -> bool {
        unsafe { GetFocus() == self.0 }
    }

    pub(super) fn destroy(self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
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
        let ctrl = key_down(VK_CONTROL);
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

// ---------- menús ----------

/// Dónde abrir un menú emergente.
#[derive(Clone, Copy, Debug)]
pub(super) enum MenuPos {
    /// Esquina superior izquierda en este punto (DIPs del área de la ventana).
    At(f32, f32),
    /// Esquina inferior izquierda en este punto: el menú crece hacia arriba.
    Above(f32, f32),
    /// Junto al mouse (menú de clic derecho).
    Cursor,
    /// Junto al mouse, abierto desde otro menú todavía abierto.
    CursorNested,
}

/// Muestra un menú y espera la elección. `dot`: el ítem con ese id lleva un punto de ese color.
/// Devuelve el id elegido (None si se cerró sin elegir).
pub(super) fn popup_menu(app: &App, entries: &[shell::Entry], pos: MenuPos, dot: Option<(u32, Color)>) -> Option<u32> {
    unsafe fn append(hmenu: HMENU, entries: &[shell::Entry]) {
        for e in entries {
            unsafe {
                match e {
                    shell::Entry::Sep => {
                        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
                    }
                    shell::Entry::Item { id, label, enabled, checked } => {
                        let mut flags = MF_STRING;
                        if !enabled {
                            flags |= MF_GRAYED;
                        }
                        if *checked {
                            flags |= MF_CHECKED;
                        }
                        let text: Vec<u16> = label.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                        let _ = AppendMenuW(hmenu, flags, *id as usize, PCWSTR(text.as_ptr()));
                    }
                    shell::Entry::Sub(label, children) => {
                        let Ok(sub) = CreatePopupMenu() else { continue };
                        append(sub, children);
                        let text: Vec<u16> = label.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                        let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, PCWSTR(text.as_ptr()));
                    }
                }
            }
        }
    }
    let hwnd = app.hwnd.hwnd();
    let k = app.dpi as f32 / 96.0;
    unsafe {
        // Que se vea lo que cambió antes del menú (la flecha abierta de la barra de dirección).
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
        let hmenu = CreatePopupMenu().ok()?;
        append(hmenu, entries);
        let bitmap = dot.and_then(|(id, color)| {
            let bitmap = accent_dot(color, app.dpi)?;
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
        let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | flags).0, pt.x, pt.y, hwnd, None).0;
        let _ = DestroyMenu(hmenu);
        if let Some(b) = bitmap {
            let _ = DeleteObject(b.into());
        }
        (cmd > 0).then_some(cmd as u32)
    }
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

/// Ícono de la app (recurso 1) al tamaño de la métrica dada; nulo si no está (la clase usa el genérico).
fn app_icon(instance: HINSTANCE, metric: SYSTEM_METRICS_INDEX) -> HICON {
    unsafe {
        let size = GetSystemMetrics(metric);
        LoadImageW(Some(instance), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_default()
    }
}

pub(super) fn colorref(c: Color) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | ch(c.g) << 8 | ch(c.b) << 16)
}

pub(super) fn scale(dips: i32, dpi: u32) -> i32 {
    dips * dpi as i32 / 96
}

// ---------- métodos de App propios de Windows ----------

impl App {
    /// Fuente y fondo de los controles de edición según el DPI y el tema.
    pub(super) fn update_edit_resources(&mut self) {
        unsafe {
            if !self.sys.edit_font.is_invalid() {
                let _ = DeleteObject(self.sys.edit_font.into());
            }
            if !self.sys.edit_brush.is_invalid() {
                let _ = DeleteObject(self.sys.edit_brush.into());
            }
            self.sys.edit_font = CreateFontW(
                -(13 * self.dpi as i32 / 96),
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
            );
            self.sys.edit_brush = CreateSolidBrush(colorref(self.theme.surface_alt));
        }
    }

    /// Menú desplegable de una carpeta de favoritos o del desborde "»". Se puede hacer clic
    /// derecho sobre un favorito del menú (`WM_MENURBUTTONUP`).
    pub(super) fn show_fav_menu(&mut self, pane: usize, source: FavMenuSource, anchor: render::Rect) {
        let Some(entries) = self.fav_menu_entries(&source) else { return };
        let mut menu = FavMenu { ids: Vec::new(), positions: HashMap::new(), action: None };
        let k = self.dpi as f32 / 96.0;
        let mut pt = POINT { x: (anchor.left * k) as i32, y: (anchor.bottom * k) as i32 };
        unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            build_fav_menu(&mut menu, hmenu, &entries);
            let _ = ClientToScreen(self.hwnd.hwnd(), &mut pt);
            self.sys.fav_menu = Some(menu);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN).0, pt.x, pt.y, self.hwnd.hwnd(), None).0;
            let menu = self.sys.fav_menu.take();
            let _ = DestroyMenu(hmenu);
            let Some(menu) = menu else { return };
            // Si dentro del menú se eligió algo con clic derecho, eso manda.
            if let Some((path, act)) = menu.action {
                self.fav_action(pane, Some(path), act);
            } else if cmd > 0 {
                if let Some(path) = menu.ids.get(cmd as usize - 1).cloned() {
                    self.activate_fav(pane, &path, key_down(VK_CONTROL));
                }
            }
        }
    }

    /// Clic derecho sobre un elemento de un menú de favoritos (`WM_MENURBUTTONUP`).
    pub(super) fn on_fav_menu_right_click(&mut self, hmenu: isize, pos: u32) {
        let Some(path) = self.sys.fav_menu.as_ref().and_then(|m| m.positions.get(&(hmenu, pos)).cloned()) else { return };
        if let Some(act) = self.fav_context_menu(Some(&path), true) {
            if let Some(m) = self.sys.fav_menu.as_mut() {
                m.action = Some((path, act));
            }
            unsafe {
                let _ = EndMenu();
            }
        }
    }
}

// En release no se abre consola; en debug sí, para ver logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod commands;
mod config;
mod console;
mod drop;
mod favorites;
mod format;
mod fs;
mod groups;
mod icons;
mod jump;
mod list;
mod ops;
mod pane;
mod palette;
mod render;
mod session;
mod shell;
mod tab;
mod theme;
mod thumbs;
mod toml;
#[cfg(feature = "arbol")]
mod tree;
mod ui;
mod uia;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    ClientToScreen, DeleteObject, FW_NORMAL, HBRUSH, HDC, HFONT, InvalidateRect, OUT_DEFAULT_PRECIS, ScreenToClient,
    BLACK_BRUSH, BeginPaint, EndPaint, FillRect, GetStockObject, PAINTSTRUCT, SetBkColor, SetTextColor,
};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, OleInitialize,
    RevokeDragDrop,
};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::{EM_SETSEL, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VIRTUAL_KEY, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_PRIOR, VK_RETURN, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, SHACF_AUTOSUGGEST_FORCE_ON, SHACF_FILESYS_DIRS, SHAutoComplete, SHDoDragDrop,
    SetWindowSubclass, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Result, w};

use commands::Command;
use config::Config;
use console::Console;
use groups::{Group, Groups};
use jump::History;
use palette::{Palette, PaletteMode, Sources, Target};
use drop::{DragEvent, DragKind, WM_APP_DRAG};
use favorites::{Fav, FavPath, Favorites};
use fs::{Location, WM_APP_CHANGED, WM_APP_LOAD};
use ops::{Op, OpResult, Undo, WM_APP_OP_DONE};
use list::{Action, ColumnSpec};
use pane::{Button, FavDrop, FilterView, Pane, PaneHit};
use render::{Renderer, contains};
use session::{PaneState, Session};
use tab::Tab;
use theme::{Accent, Theme};
use ui::{Layout, Splits, Splitter};

const CLASS_NAME: PCWSTR = w!("xdir.main");
/// Tamaño inicial en DIPs; se escala según el DPI del monitor.
const INITIAL_SIZE: (i32, i32) = (1280, 800);
/// Siempre hay 4 paneles; el layout decide cuántos se ven. Los ocultos conservan sus pestañas.
const PANES: usize = 4;
/// La barra de dirección terminó de editarse. `wparam` = 1 si hay que navegar.
const WM_APP_EDIT_DONE: u32 = WM_APP + 10;
/// El renombrado en línea terminó. `wparam` = 1 si hay que aplicarlo.
const WM_APP_RENAME_DONE: u32 = WM_APP + 11;
/// Terminó el renombrado de un favorito. `wparam` = 1 si hay que aplicarlo.
const WM_APP_FAVEDIT_DONE: u32 = WM_APP + 12;
/// Terminó la edición del filtro rápido. `wparam` = 1 si se conserva, 0 si se quita (Escape).
const WM_APP_FILTER_DONE: u32 = WM_APP + 13;
/// Notificación de un control de edición: el texto cambió.
const EN_CHANGE_CODE: usize = 0x0300;
/// Tecla de la paleta: `wparam` = tecla virtual (0: perdió el foco), `lparam` = 1 si con Ctrl.
const WM_APP_PALETTE_KEY: u32 = WM_APP + 14;

/// Qué muestra un menú desplegable de favoritos.
enum FavMenuSource {
    /// El contenido de una carpeta de favoritos.
    Group(FavPath),
    /// Los favoritos de la raíz que no entraron en la barra, desde este índice.
    Overflow(usize),
}

/// Menú de favoritos abierto: a qué favorito corresponde cada comando y cada posición
/// (para el clic derecho dentro del menú).
struct FavMenu {
    ids: Vec<FavPath>,
    positions: HashMap<(isize, u32), FavPath>,
    /// Acción elegida con clic derecho dentro del menú.
    action: Option<(FavPath, FavAct)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FavAct {
    Open = 1,
    OpenNewTab,
    Rename,
    Delete,
    MoveOut,
    NewGroup,
    AddCurrent,
    EditFile,
    HideBar,
}

impl FavAct {
    fn from_id(id: i32) -> Option<Self> {
        use FavAct::*;
        [Open, OpenNewTab, Rename, Delete, MoveOut, NewGroup, AddCurrent, EditFile, HideBar]
            .into_iter()
            .find(|a| *a as i32 == id)
    }
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
/// Operaciones que se pueden deshacer (Ctrl+Z).
const UNDO_MAX: usize = 50;

/// Renombrado en línea en curso.
struct Rename {
    edit: HWND,
    path: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Pane(usize, PaneHit),
    Splitter(Splitter),
    Layout(Layout),
    /// Rueda dentada de la barra de estado.
    Gear,
    /// Botón de grupos guardados de la barra de estado.
    Groups,
    Picker(ui::PickerHit),
    /// Menú de consola abierto: una consola, o el menú sin nada debajo.
    TermMenu(Option<Console>),
    /// Paleta abierta: un resultado, o el panel sin nada debajo.
    Palette(Option<usize>),
}

/// Lo que se está arrastrando con el botón izquierdo.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    None,
    List(usize),
    Splitter(Splitter),
    Tab { pane: usize, index: usize, start_x: f32, start_y: f32, moved: bool },
    Button(usize, Button),
    /// Un favorito de la barra: clic al soltar sin mover, si no reordenar.
    Fav { pane: usize, index: usize, start_x: f32, moved: bool },
}

/// Transición de layout: desde dónde parte cada panel y cuándo empezó.
struct LayoutAnim {
    from: [Option<render::Rect>; PANES],
    start: Instant,
}

struct App {
    hwnd: HWND,
    dpi: u32,
    theme: Theme,
    renderer: Renderer,
    panes: Vec<Pane>,
    layout: Layout,
    splits: Splits,
    /// Panel maximizado temporalmente (ocupa toda el área); no se guarda en la sesión.
    maximized: Option<usize>,
    /// Rectángulos con que se dibujó cada panel el último frame, y la transición de layout en curso.
    displayed: [Option<render::Rect>; PANES],
    layout_anim: Option<LayoutAnim>,
    /// Cuándo fue el último frame de una animación (None: no hay ninguna en curso).
    last_frame: Option<Instant>,
    /// Último elemento con foco que se avisó a los lectores de pantalla: (panel, posición).
    uia_focus: Option<(usize, Option<usize>)>,
    /// Panel activo: recibe el teclado.
    active: usize,
    show_hidden: bool,
    next_tab_id: usize,
    tx: Sender<fs::LoadEvent>,
    rx: Receiver<fs::LoadEvent>,
    /// Control de edición nativo sobre la barra de dirección de un panel.
    edit: Option<(HWND, usize)>,
    edit_font: HFONT,
    edit_brush: HBRUSH,
    tracking_mouse: bool,
    hover: Hit,
    drag: Drag,
    title: String,
    started: Instant,
    first_frame_ms: Option<f64>,
    list_frame_done: bool,
    /// Tiempo total del hilo de interfaz procesando lotes de carga (para `--probe`).
    ui_load_ms: f64,
    /// `--probe <archivo>`: escribe los tiempos de arranque y cierra.
    probe: Option<String>,
    /// Hitos del arranque (ms desde `started`), volcados por `--probe`.
    marks: Vec<(&'static str, f64)>,
    /// Sesión guardada, para restaurar la ventana al mostrarla.
    window_state: Option<([i32; 4], bool)>,
    ops_tx: Sender<OpResult>,
    ops_rx: Receiver<OpResult>,
    undo: Vec<Undo>,
    rename: Option<Rename>,
    /// Carpeta recién creada sobre la que abrir el renombrado cuando aparezca en la lista.
    pending_rename: Option<PathBuf>,
    /// Mensaje para la barra de estado (errores de operaciones).
    notice: Option<String>,
    /// Se mantiene viva mientras exista la ventana.
    drop_target: Option<IDropTarget>,
    /// Fondo Mica activo (ventana con marco extendido y composición translúcida).
    transparent: bool,
    favorites: Favorites,
    /// Carpetas visitadas, para el salto rápido.
    history: History,
    /// Atajos y colores de `config.toml`.
    config: Config,
    groups: Groups,
    /// Salto rápido o paleta de comandos abierta, con su control de edición.
    palette: Option<(Palette, HWND)>,
    /// Árbol de carpetas lateral (experimental).
    #[cfg(feature = "arbol")]
    tree: tree::Tree,
    show_favorites: bool,
    /// Colores por tipo de archivo encendidos (switch de la rueda dentada).
    file_colors: bool,
    /// Dónde caería lo que se arrastra sobre la barra de favoritos (panel, posición).
    fav_drop: Option<(usize, FavDrop)>,
    /// Arrastrando una pestaña sobre otro panel: (panel destino, posición en su tira).
    tab_drop: Option<(usize, usize)>,
    fav_menu: Option<FavMenu>,
    /// Renombrado de un favorito en curso.
    fav_edit: Option<(HWND, FavPath)>,
    /// Control de edición del filtro rápido abierto y su panel.
    filter_edit: Option<(HWND, usize)>,
    /// El último clic derecho fue sobre la barra de favoritos: (panel, favorito).
    fav_ctx: Option<(usize, Option<FavPath>)>,
    /// Clic derecho en el encabezado de la lista de ese panel: menú de columnas.
    header_ctx: Option<usize>,
    /// Columnas de las listas (las mismas en todas).
    columns: Vec<ColumnSpec>,
    accent: Accent,
    /// Modo fijado con el switch; None sigue al modo de Windows.
    dark_mode: Option<bool>,
    /// Panel de colores abierto (rueda dentada).
    picker_open: bool,
    /// Menú de consola abierto, en este panel.
    term_menu: Option<usize>,
    /// Colores personalizados del selector de Windows, para que se recuerden entre usos.
    custom_colors: [COLORREF; 16],
}

fn main() -> Result<()> {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let probe = args.iter().position(|a| a == "--probe").and_then(|i| args.get(i + 1).cloned());
    let start_path = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--probe"))
        .map(|(_, a)| PathBuf::from(a))
        .filter(|p| p.is_dir());
    // `--probe` mide el arranque limpio: sin sesión, con la carpeta pedida en un solo panel.
    let mut session = if probe.is_some() { None } else { session::load() };
    if probe.is_some() {
        session = Some(Session { layout: 0, ..Default::default() });
    }

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
            // Ícono del exe (recurso 1, ver assets/xdir.rc): grande para Alt+Tab, chico para la barra de título.
            hIcon: app_icon(instance.into(), SM_CXICON),
            hIconSm: app_icon(instance.into(), SM_CXSMICON),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let mut marks = vec![("init", ms_since(started))];
        let session = session.unwrap_or_else(default_session);
        let theme = Theme::from_system(session.accent, session.dark_mode);
        shell::set_menu_mode(session.dark_mode);
        let renderer = Renderer::new()?;
        marks.push(("fabricas d2d/dwrite", ms_since(started)));
        let (tx, rx) = channel();
        let (ops_tx, ops_rx) = channel();
        // `--probe` mide un arranque limpio: sin historial.
        let history = if probe.is_none() { History::load() } else { History::default() };
        let config = if probe.is_none() { Config::load() } else { Config::default() };
        let config_errors = config.errors.first().map(|e| format!("config.toml, {e}"));
        anim::set_enabled(config.animations.unwrap_or_else(anim::system_allows));
        let mut app = App {
            hwnd: HWND::default(),
            dpi: 96,
            theme,
            renderer,
            panes: Vec::new(),
            layout: Layout::from_index(session.layout).unwrap_or(Layout::Four),
            splits: Splits { x: session.split_x, y: session.split_y },
            maximized: None,
            displayed: [None; PANES],
            layout_anim: None,
            last_frame: None,
            uia_focus: None,
            active: 0,
            show_hidden: session.show_hidden,
            next_tab_id: 1,
            tx,
            rx,
            edit: None,
            edit_font: HFONT::default(),
            edit_brush: HBRUSH::default(),
            tracking_mouse: false,
            hover: Hit::None,
            drag: Drag::None,
            title: String::new(),
            started,
            first_frame_ms: None,
            list_frame_done: false,
            ui_load_ms: 0.0,
            probe,
            marks,
            window_state: session.window.map(|w| (w, session.maximized)),
            ops_tx,
            ops_rx,
            undo: Vec::new(),
            rename: None,
            pending_rename: None,
            notice: config_errors,
            drop_target: None,
            transparent: false,
            favorites: Favorites::load(),
            history,
            config,
            groups: Groups::load(),
            palette: None,
            #[cfg(feature = "arbol")]
            tree: tree::Tree::new(),
            show_favorites: session.show_favorites,
            file_colors: session.file_colors,
            fav_drop: None,
            tab_drop: None,
            fav_menu: None,
            fav_edit: None,
            filter_edit: None,
            fav_ctx: None,
            header_ctx: None,
            columns: session.columns.clone(),
            accent: session.accent,
            dark_mode: session.dark_mode,
            picker_open: false,
            term_menu: None,
            custom_colors: [COLORREF(0x00FFFFFF); 16],
        };
        app.restore_panes(&session, start_path);
        #[cfg(feature = "arbol")]
        {
            app.tree.visible = session.show_tree;
        }
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

        // Las carpetas visibles empiezan a leerse antes de mostrar la ventana.
        (*app).sync_visibility();
        (*app).drop_target = drop::register((*app).hwnd).ok();
        (*app).marks.push(("ventana creada", ms_since(started)));
        (*app).show();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Primera vez: 4 paneles en "Este equipo".
fn default_session() -> Session {
    let panes = (0..PANES).map(|_| PaneState { tabs: vec![Location::Computer], active: 0 }).collect();
    Session { panes, ..Default::default() }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            let app = cs.lpCreateParams as *mut App;
            (*app).hwnd = hwnd;
            (*app).dpi = GetDpiForWindow(hwnd);
            let mica = (*app).theme.apply_to_window(hwnd);
            (*app).renderer.set_transparent(mica);
            (*app).transparent = mica;
            (*app).update_edit_resources();
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

impl App {
    fn handle(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_PAINT => {
                    // BeginPaint valida antes de pintar (si `paint` pide otro frame, no se
                    // pierde) y dispara el borrado de fondo cuando hace falta.
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(self.hwnd, &mut ps);
                    let _ = EndPaint(self.hwnd, &ps);
                    self.paint();
                    LRESULT(0)
                }
                WM_ERASEBKGND => {
                    // Con Mica, la superficie GDI de la ventana va en negro: con el marco
                    // extendido, el negro es transparente y deja ver la composición y el fondo.
                    // En modo opaco todo el fondo lo pinta Direct2D.
                    if self.transparent {
                        let mut rc = RECT::default();
                        let _ = GetClientRect(self.hwnd, &mut rc);
                        FillRect(HDC(wparam.0 as *mut _), &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                    }
                    LRESULT(1)
                }
                WM_SIZE => {
                    let width = (lparam.0 & 0xFFFF) as u32;
                    let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                    self.renderer.resize(width, height, self.dpi);
                    self.end_edit(false);
                    self.end_filter(true);
                    self.close_palette();
                    LRESULT(0)
                }
                WM_DPICHANGED => {
                    self.dpi = (wparam.0 & 0xFFFF) as u32;
                    self.end_edit(false);
                    self.end_filter(true);
                    self.close_palette();
                    self.renderer.set_dpi(self.dpi);
                    self.update_edit_resources();
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd,
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
                        self.theme = Theme::from_system(self.accent, self.dark_mode);
                        let mica = self.theme.apply_to_window(self.hwnd);
                        self.renderer.set_transparent(mica);
                        self.transparent = mica;
                        self.update_edit_resources();
                        self.redraw();
                    }
                    LRESULT(0)
                }
                WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                    let (x, y) = self.cursor_pos();
                    let splitter = match self.drag {
                        Drag::Splitter(s) => Some(s),
                        _ => match self.hit(x, y) {
                            Hit::Splitter(s) => Some(s),
                            _ => None,
                        },
                    };
                    // Borde de un encabezado de columna: cambiar el ancho.
                    let column_edge = match (self.drag, self.hit(x, y)) {
                        (Drag::List(i), _) | (_, Hit::Pane(i, PaneHit::List)) => self.panes[i].tab().list.wants_resize_cursor(x, y),
                        _ => false,
                    };
                    let cursor = match splitter {
                        Some(Splitter::Vertical) => IDC_SIZEWE,
                        Some(Splitter::Horizontal) => IDC_SIZENS,
                        None if column_edge => IDC_SIZEWE,
                        None => IDC_ARROW,
                    };
                    if let Ok(c) = LoadCursorW(None, cursor) {
                        SetCursor(Some(c));
                    }
                    LRESULT(1)
                }
                WM_APP_LOAD => {
                    let t0 = Instant::now();
                    let mut changed = false;
                    while let Ok(event) = self.rx.try_recv() {
                        if let Some(tab) = self.tab_by_id(event.panel) {
                            changed |= tab.on_load(event);
                            if let Some(path) = tab.visited.take() {
                                self.history.record(&path, jump::now());
                            }
                        }
                    }
                    self.ui_load_ms += ms_since(t0);
                    if changed {
                        self.try_pending_rename();
                        self.redraw();
                    }
                    LRESULT(0)
                }
                icons::WM_APP_ICONS => {
                    self.redraw();
                    LRESULT(0)
                }
                WM_APP_CHANGED => {
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    if let Some(tab) = self.tab_by_id(wparam.0) {
                        tab.on_changed(hwnd, &tx, lparam.0 as u64);
                    }
                    LRESULT(0)
                }
                WM_APP_EDIT_DONE => {
                    self.end_edit(wparam.0 == 1);
                    LRESULT(0)
                }
                WM_CTLCOLOREDIT => {
                    let hdc = HDC(wparam.0 as *mut _);
                    SetTextColor(hdc, colorref(self.theme.text));
                    SetBkColor(hdc, colorref(self.theme.surface_alt));
                    LRESULT(self.edit_brush.0 as isize)
                }
                WM_APP_FAVEDIT_DONE => {
                    self.end_fav_edit(wparam.0 == 1);
                    LRESULT(0)
                }
                WM_APP_FILTER_DONE => {
                    self.end_filter(wparam.0 == 1);
                    LRESULT(0)
                }
                // El filtro y la paleta se actualizan con cada tecla.
                WM_COMMAND if (wparam.0 >> 16) & 0xFFFF == EN_CHANGE_CODE => {
                    if let Some((edit, pane)) = self.filter_edit {
                        if lparam.0 == edit.0 as isize {
                            let text = window_text(edit);
                            self.panes[pane].tab_mut().list.set_filter(&text);
                            self.redraw();
                        }
                    }
                    if self.palette.as_ref().is_some_and(|(_, e)| lparam.0 == e.0 as isize) {
                        self.refresh_palette();
                    }
                    LRESULT(0)
                }
                #[cfg(feature = "arbol")]
                tree::WM_APP_TREE => {
                    self.tree.on_loaded();
                    self.redraw();
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
                WM_APP_PALETTE_KEY => {
                    self.on_palette_key(VIRTUAL_KEY(wparam.0 as u16), lparam.0 == 1);
                    LRESULT(0)
                }
                WM_MENURBUTTONUP if self.fav_menu.is_some() => {
                    self.on_fav_menu_right_click(lparam.0, wparam.0 as u32);
                    LRESULT(0)
                }
                // Al volver a la ventana se releen los favoritos (por si se editó el archivo).
                WM_ACTIVATEAPP if wparam.0 != 0 => {
                    // Al volver a la ventana, el lector tiene que leer de nuevo el elemento con foco.
                    self.uia_focus = None;
                    let fresh = Favorites::load();
                    if fresh != self.favorites {
                        self.favorites = fresh;
                        self.redraw();
                    }
                    if self.probe.is_none() {
                        self.reload_config();
                    }
                    LRESULT(0)
                }
                WM_APP_RENAME_DONE => {
                    self.end_rename(wparam.0 == 1);
                    LRESULT(0)
                }
                WM_APP_OP_DONE => {
                    while let Ok(result) = self.ops_rx.try_recv() {
                        self.on_op_done(result);
                    }
                    self.redraw();
                    LRESULT(0)
                }
                WM_APP_DRAG => {
                    let ev = &mut *(lparam.0 as *mut DragEvent);
                    self.on_drag(ev);
                    LRESULT(0)
                }
                WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR => {
                    match shell::handle_menu_message(msg, wparam, lparam) {
                        Some(r) => r,
                        None => DefWindowProcW(self.hwnd, msg, wparam, lparam),
                    }
                }
                WM_RBUTTONDOWN => {
                    let (x, y) = self.mouse_pos(lparam);
                    let _ = SetFocus(Some(self.hwnd));
                    self.fav_ctx = None;
                    self.header_ctx = None;
                    match self.hit(x, y) {
                        Hit::Pane(i, PaneHit::List) if self.panes[i].tab().list.in_header(x, y) => {
                            self.set_active(i);
                            self.header_ctx = Some(i);
                        }
                        Hit::Pane(i, PaneHit::List) => {
                            self.set_active(i);
                            self.panes[i].tab_mut().list.on_right_down(x, y);
                            self.redraw();
                        }
                        Hit::Pane(i, PaneHit::Fav(j)) => self.fav_ctx = Some((i, Some(vec![j]))),
                        Hit::Pane(i, PaneHit::FavBar | PaneHit::FavOverflow) => self.fav_ctx = Some((i, None)),
                        _ => {}
                    }
                    LRESULT(0)
                }
                WM_CONTEXTMENU => {
                    if let Some(pane) = self.header_ctx.take().filter(|_| lparam.0 != -1) {
                        let pt = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
                        self.show_columns_menu(pane, pt);
                        return LRESULT(0);
                    }
                    match self.fav_ctx.take() {
                        Some((pane, path)) if lparam.0 != -1 => {
                            if let Some(act) = self.fav_context_menu(path.as_ref(), false) {
                                self.fav_action(pane, path, act);
                            }
                        }
                        _ => self.on_context_menu(lparam),
                    }
                    LRESULT(0)
                }
                WM_KEYDOWN => {
                    if self.on_key(VIRTUAL_KEY(wparam.0 as u16)) {
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd, msg, wparam, lparam)
                    }
                }
                WM_SYSKEYDOWN => {
                    // Alt+flechas como en el Explorador; el resto (Alt+F4, Alt+Espacio) sigue de largo.
                    match self.config.keymap.lookup(VIRTUAL_KEY(wparam.0 as u16), key_down(VK_CONTROL), key_down(VK_SHIFT), true) {
                        Some(c) => self.run_command(c),
                        None => return DefWindowProcW(self.hwnd, msg, wparam, lparam),
                    }
                    LRESULT(0)
                }
                WM_CHAR => {
                    if let Some(c) = char::from_u32(wparam.0 as u32) {
                        if !c.is_control() && !key_down(VK_CONTROL) {
                            let action = self.tab_mut().list.on_char(c);
                            self.apply(self.active, action);
                        }
                    }
                    LRESULT(0)
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    let (x, y) = self.mouse_pos(lparam);
                    let _ = SetFocus(Some(self.hwnd));
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
                    self.tracking_mouse = false;
                    self.hover = Hit::None;
                    for p in &mut self.panes {
                        p.tab_mut().list.on_mouse_leave();
                    }
                    self.redraw();
                    LRESULT(0)
                }
                WM_MOUSEWHEEL => {
                    // La rueda va al panel bajo el mouse, aunque no sea el activo.
                    let (x, y) = self.cursor_pos();
                    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                    let pane = match self.hit(x, y) {
                        Hit::Palette(_) => {
                            if let Some((pal, _)) = &mut self.palette {
                                pal.on_wheel(delta.round() as i32 * 3);
                                self.redraw();
                            }
                            return LRESULT(0);
                        }
                        Hit::Pane(i, _) => i,
                        _ => self.active,
                    };
                    #[cfg(feature = "arbol")]
                    if self.tree.contains(x, y) {
                        self.tree.on_wheel(delta * wheel_lines());
                        self.redraw();
                        return LRESULT(0);
                    }
                    // Ctrl+rueda: tamaño de la vista, como en el Explorador.
                    if key_down(VK_CONTROL) {
                        self.cycle_view(pane, delta > 0.0);
                        return LRESULT(0);
                    }
                    let action = self.panes[pane].tab_mut().list.on_wheel(delta * wheel_lines());
                    self.apply(pane, action);
                    LRESULT(0)
                }
                WM_XBUTTONUP => {
                    // Botones laterales del mouse: atrás y adelante en el panel bajo el mouse.
                    let (x, y) = self.mouse_pos(lparam);
                    if let Hit::Pane(i, _) = self.hit(x, y) {
                        self.set_active(i);
                    }
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    match ((wparam.0 >> 16) & 0xFFFF) as u16 {
                        1 => self.tab_mut().go_back(hwnd, &tx),
                        2 => self.tab_mut().go_forward(hwnd, &tx),
                        _ => {}
                    }
                    self.redraw();
                    LRESULT(1)
                }
                WM_CLOSE => {
                    if self.probe.is_none() {
                        let _ = session::save(&self.session());
                        let _ = self.history.save();
                    }
                    let _ = DestroyWindow(self.hwnd);
                    LRESULT(0)
                }
                // Un lector de pantalla pregunta por la raíz de UI Automation (ver uia.rs).
                WM_GETOBJECT if lparam.0 as i32 == windows::Win32::UI::Accessibility::UiaRootObjectId => {
                    let provider = uia::root_provider(self);
                    windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(self.hwnd, wparam, lparam, &provider)
                }
                WM_DESTROY => {
                    // Se desconectan los lectores: los proveedores que les quedan dejan de responder.
                    uia::shutdown();
                    let _ = windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(
                        self.hwnd,
                        WPARAM(0),
                        LPARAM(0),
                        None::<&windows::Win32::UI::Accessibility::IRawElementProviderSimple>,
                    );
                    let _ = RevokeDragDrop(self.hwnd);
                    let _ = DeleteObject(self.edit_font.into());
                    let _ = DeleteObject(self.edit_brush.into());
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(self.hwnd, msg, wparam, lparam),
            }
        }
    }

    // ---------- paneles y pestañas ----------

    fn tab_mut(&mut self) -> &mut Tab {
        self.panes[self.active].tab_mut()
    }

    fn tab_by_id(&mut self, id: usize) -> Option<&mut Tab> {
        self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()).find(|t| t.id == id)
    }

    fn make_tab(&mut self, location: Location) -> Tab {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut tab = Tab::new(id, location, self.show_hidden);
        tab.list.columns = self.columns.clone();
        tab
    }

    /// Ancho que ocupa el árbol de carpetas a la izquierda (0 si no está o no se compiló).
    fn tree_width(&self) -> f32 {
        #[cfg(feature = "arbol")]
        if self.tree.visible {
            return tree::WIDTH + ui::GAP;
        }
        0.0
    }

    /// Clic en el árbol: expandir/contraer o abrir la carpeta en el panel activo. Devuelve si
    /// el clic era del árbol.
    #[cfg(feature = "arbol")]
    fn on_tree_click(&mut self, x: f32, y: f32) -> bool {
        if !self.tree.contains(x, y) {
            return false;
        }
        match self.tree.hit(x, y) {
            tree::TreeHit::Toggle(id) => self.tree.toggle(self.hwnd, id, self.show_hidden),
            tree::TreeHit::Open(loc) => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                self.tab_mut().navigate(hwnd, &tx, loc);
            }
            tree::TreeHit::Nothing => {}
        }
        self.redraw();
        true
    }

    /// Vista de un panel: se aplica a todas sus pestañas (y la toman las nuevas).
    fn set_view(&mut self, pane: usize, view: list::View) {
        for tab in &mut self.panes[pane].tabs {
            tab.list.set_view(view);
        }
        self.notice = Some(format!("Vista: {}", view.name().to_lowercase()));
        self.redraw();
    }

    /// Ctrl+rueda: vista siguiente (más grande) o anterior sobre el panel.
    fn cycle_view(&mut self, pane: usize, bigger: bool) {
        let i = self.panes[pane].tab().list.view.index();
        let j = if bigger { (i + 1).min(list::VIEWS.len() - 1) } else { i.saturating_sub(1) };
        if i != j {
            self.set_view(pane, list::VIEWS[j].0);
        }
    }

    /// Las columnas son las mismas en todas las listas (y se guardan en la sesión).
    fn set_columns(&mut self, columns: Vec<ColumnSpec>) {
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.list.columns = columns.clone();
        }
        self.columns = columns;
        self.redraw();
    }

    /// Menú del clic derecho en el encabezado: mostrar u ocultar columnas.
    fn show_columns_menu(&mut self, pane: usize, screen: POINT) {
        const RESET: usize = 100;
        let current = self.panes[pane].tab().list.columns.clone();
        let cmd = unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            for (i, &(col, title, _, _)) in list::COLUMNS.iter().enumerate() {
                let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
                let mut flags = MF_STRING;
                if current.iter().any(|c| c.col == col) {
                    flags |= MF_CHECKED;
                }
                // El nombre no se puede ocultar.
                if col == list::Column::Name {
                    flags |= MF_GRAYED;
                }
                let _ = AppendMenuW(hmenu, flags, i + 1, PCWSTR(wide.as_ptr()));
            }
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            let wide: Vec<u16> = "Columnas de fábrica".encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(hmenu, MF_STRING, RESET, PCWSTR(wide.as_ptr()));
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN).0, screen.x, screen.y, self.hwnd, None).0 as usize;
            let _ = DestroyMenu(hmenu);
            cmd
        };
        match cmd {
            RESET => self.set_columns(list::default_columns()),
            i if (1..=list::COLUMNS.len()).contains(&i) => {
                let list = &mut self.panes[pane].tab_mut().list;
                list.toggle_column(list::COLUMNS[i - 1].0);
                let cols = list.columns.clone();
                self.set_columns(cols);
            }
            _ => {}
        }
    }

    fn restore_panes(&mut self, s: &Session, start_path: Option<PathBuf>) {
        let fallback = default_session();
        for i in 0..PANES {
            let state = s.panes.get(i).or(fallback.panes.get(i)).cloned().unwrap_or_default();
            let locations = if state.tabs.is_empty() { vec![Location::Computer] } else { state.tabs };
            let view = s.views.get(i).copied().unwrap_or(list::View::Details);
            let tabs: Vec<Tab> = locations
                .into_iter()
                .map(|l| {
                    let mut t = self.make_tab(l);
                    t.list.view = view;
                    t
                })
                .collect();
            self.panes.push(Pane::new(tabs, state.active));
        }
        self.active = s.active_pane.min(self.layout.panes() - 1);
        // Una carpeta pasada por línea de comandos se abre en una pestaña nueva del panel activo.
        if let Some(path) = start_path {
            let tab = self.make_tab(Location::Path(path));
            self.panes[self.active].add_tab(tab, true);
        }
    }

    fn session(&self) -> Session {
        let mut s = Session {
            layout: self.layout.index(),
            split_x: self.splits.x,
            split_y: self.splits.y,
            active_pane: self.active,
            show_hidden: self.show_hidden,
            show_favorites: self.show_favorites,
            file_colors: self.file_colors,
            show_tree: self.tree_width() > 0.0,
            accent: self.accent,
            dark_mode: self.dark_mode,
            columns: self.columns.clone(),
            views: self.panes.iter().map(|p| p.tab().list.view).collect(),
            panes: self
                .panes
                .iter()
                .map(|p| PaneState { tabs: p.tabs.iter().map(|t| t.location.clone()).collect(), active: p.active })
                .collect(),
            ..Default::default()
        };
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        if unsafe { GetWindowPlacement(self.hwnd, &mut wp) }.is_ok() {
            let r = wp.rcNormalPosition;
            s.window = Some([r.left, r.top, r.right - r.left, r.bottom - r.top]);
            s.maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32;
        }
        s
    }

    fn show(&mut self) {
        unsafe {
            match self.window_state.take() {
                Some(([x, y, w, h], maximized)) => {
                    let wp = WINDOWPLACEMENT {
                        length: size_of::<WINDOWPLACEMENT>() as u32,
                        showCmd: if maximized { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                        rcNormalPosition: RECT { left: x, top: y, right: x + w, bottom: y + h },
                        ..Default::default()
                    };
                    // Ajusta solo si la posición guardada quedó fuera de los monitores actuales.
                    let _ = SetWindowPlacement(self.hwnd, &wp);
                }
                None => {
                    let dpi = GetDpiForWindow(self.hwnd);
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        0,
                        0,
                        scale(INITIAL_SIZE.0, dpi),
                        scale(INITIAL_SIZE.1, dpi),
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    let _ = ShowWindow(self.hwnd, SW_SHOW);
                }
            }
        }
    }

    /// Solo las pestañas a la vista cargan y vigilan su carpeta.
    fn sync_visibility(&mut self) {
        let (hwnd, tx, visible_panes) = (self.hwnd, self.tx.clone(), self.layout.panes());
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let active = pane.active;
            for (j, tab) in pane.tabs.iter_mut().enumerate() {
                tab.set_visible(hwnd, &tx, i < visible_panes && j == active);
            }
        }
    }

    fn set_active(&mut self, pane: usize) {
        if pane < self.layout.panes() && pane != self.active {
            self.end_edit(false);
            // Ir a otro panel restaura el layout si había uno maximizado.
            self.maximized = None;
            self.active = pane;
            self.redraw();
        }
    }

    /// Maximiza el panel (ocupa toda el área) o, si ya lo está, vuelve al layout.
    fn toggle_maximize(&mut self, pane: usize) {
        if self.layout.panes() < 2 || pane >= self.layout.panes() {
            return;
        }
        self.end_edit(false);
        self.term_menu = None;
        self.begin_layout_anim();
        self.maximized = if self.maximized == Some(pane) { None } else { Some(pane) };
        self.active = pane;
        self.redraw();
    }

    /// Índices de los paneles dibujados: todos los del layout o solo el maximizado.
    fn shown_panes(&self) -> Vec<usize> {
        match self.maximized {
            Some(i) => vec![i],
            None => (0..self.layout.panes()).collect(),
        }
    }

    /// Comandos de prueba (feature `pruebas`): `cmd <id>` ejecuta una acción de `commands::ALL`;
    /// `picker`, `term <panel>`, `tabdrop <panel> <pos>`, `droptarget <panel> <pos>`,
    /// `select <panel> <pos>`, `active <panel>` y `notice <texto>` dejan la interfaz en un estado
    /// que se quiere mostrar.
    #[cfg(feature = "pruebas")]
    fn test_hook(&mut self, text: &str) {
        let mut it = text.splitn(3, ' ');
        let (verb, a, b) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
        let (na, nb) = (a.parse::<usize>().unwrap_or(0), b.parse::<usize>().unwrap_or(0));
        match verb {
            "cmd" => {
                if let Some(s) = commands::ALL.iter().find(|s| s.id == a) {
                    self.run_command(s.command);
                }
            }
            "picker" => self.picker_open = true,
            "go" => self.go_to_text(na, b),
            // band <panel> <x0> <y0> <x1> <y1>: arrastre con rectángulo (en DIPs de la ventana).
            "band" => {
                let n: Vec<f32> = text.split_whitespace().skip(2).filter_map(|v| v.parse().ok()).collect();
                if let [x0, y0, x1, y1] = n[..] {
                    let list = &mut self.panes[na].tab_mut().list;
                    list.on_mouse_down(x0, y0, false, false);
                    list.on_mouse_move(x1, y1);
                }
            }
            "term" => self.term_menu = Some(na),
            "tabdrop" => self.tab_drop = Some((na, nb)),
            "droptarget" => self.panes[na].tab_mut().list.drop_target = Some(nb),
            "select" => self.panes[na].tab_mut().list.select_pos(nb),
            "active" => self.set_active(na),
            "notice" => self.notice = Some(format!("{a} {b}").trim().to_string()),
            "close" => self.close_palette(),
            _ => {}
        }
        self.redraw();
    }

    /// Animaciones: la opción de `config.toml` y la preferencia de Windows tienen que permitirlas.
    fn apply_animation_setting(&mut self) {
        anim::set_enabled(self.config.animations.unwrap_or_else(anim::system_allows));
        if !anim::enabled() {
            self.layout_anim = None;
        }
    }

    /// Empieza una transición desde lo que se está viendo. Se llama ANTES de cambiar layout,
    /// divisores o maximizado: el estado cambia al instante y solo se dibuja de a poco.
    fn begin_layout_anim(&mut self) {
        if anim::enabled() && self.displayed.iter().any(Option::is_some) {
            self.layout_anim = Some(LayoutAnim { from: self.displayed, start: Instant::now() });
        }
    }

    /// Rectángulos de los paneles para este frame: los de destino o, en plena transición, los
    /// intermedios. Devuelve también si la transición sigue en curso.
    fn animated_rects(&mut self, target: Vec<(usize, render::Rect)>) -> (Vec<(usize, render::Rect)>, bool) {
        let progress = self.layout_anim.as_ref().map(|a| a.start.elapsed().as_secs_f32() * 1000.0 / anim::LAYOUT_MS);
        let (rects, running) = match (&self.layout_anim, progress) {
            (Some(a), Some(t)) if t < 1.0 => {
                let e = anim::ease_out(t);
                // Un panel que no estaba "aparece" desde un poco más chico.
                let rects = target.iter().map(|&(i, r)| (i, anim::lerp_rect(a.from[i].unwrap_or_else(|| anim::grow_from(r)), r, e))).collect();
                (rects, true)
            }
            _ => {
                self.layout_anim = None;
                (target, false)
            }
        };
        self.displayed = [None; PANES];
        for &(i, r) in &rects {
            self.displayed[i] = Some(r);
        }
        (rects, running)
    }

    fn set_layout(&mut self, layout: Layout) {
        self.end_edit(false);
        if layout != self.layout || self.maximized.is_some() {
            self.begin_layout_anim();
        }
        self.maximized = None;
        self.layout = layout;
        self.active = self.active.min(layout.panes() - 1);
        self.sync_visibility();
        self.redraw();
    }

    fn new_tab(&mut self, pane: usize, location: Location, activate: bool) {
        let mut tab = self.make_tab(location);
        tab.list.view = self.panes[pane].tab().list.view;
        self.panes[pane].add_tab(tab, activate);
        self.sync_visibility();
        self.redraw();
    }

    fn close_tab(&mut self, pane: usize, index: usize) {
        if self.edit.is_some_and(|(_, p)| p == pane) {
            self.end_edit(false);
        }
        if self.panes[pane].close_tab(index) {
            self.sync_visibility();
            self.redraw();
        }
    }

    /// Lleva la pestaña `index` del panel `from` a la posición `at` del panel `to`, con su
    /// historial y su selección. Si era la única del panel de origen, ahí se abre "Este equipo".
    fn move_tab_to_pane(&mut self, from: usize, index: usize, to: usize, at: usize) {
        if from == to || index >= self.panes[from].tabs.len() {
            return;
        }
        self.end_edit(false);
        self.term_menu = None;
        let tab = self.panes[from].take_tab(index);
        if self.panes[from].tabs.is_empty() {
            let fresh = self.make_tab(Location::Computer);
            self.panes[from].insert_tab(0, fresh);
        }
        self.panes[to].insert_tab(at, tab);
        self.active = to;
        self.sync_visibility();
        self.redraw();
    }

    fn select_tab(&mut self, pane: usize, index: usize) {
        self.panes[pane].select_tab(index);
        self.sync_visibility();
        self.redraw();
    }

    // ---------- entrada ----------

    fn client_dips(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        let k = 96.0 / self.dpi as f32;
        ((rc.right - rc.left) as f32 * k, (rc.bottom - rc.top) as f32 * k)
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        let (w, h) = self.client_dips();
        // La paleta va encima de todo, y después los menús flotantes.
        if let Some((pal, _)) = &self.palette {
            if let Some(i) = pal.item_at(w, x, y) {
                return Hit::Palette(i);
            }
        }
        if let Some(pane) = self.term_menu {
            if let Some(c) = ui::terminal_menu_at(self.panes[pane].terminal_rect(), w, x, y) {
                return Hit::TermMenu(c);
            }
        }
        if self.picker_open {
            if let Some(k) = ui::picker_at(w, h, x, y) {
                return Hit::Picker(k);
            }
        }
        if contains(&ui::gear_rect(w, h), x, y) {
            return Hit::Gear;
        }
        if contains(&ui::groups_rect(w, h), x, y) {
            return Hit::Groups;
        }
        let area = ui::panes_area(w, h, self.tree_width());
        if self.maximized.is_none() {
            for (s, r) in ui::splitter_rects(self.layout, area, self.splits) {
                if contains(&r, x, y) {
                    return Hit::Splitter(s);
                }
            }
        }
        if let Some(l) = ui::layout_at(w, h, x, y) {
            return Hit::Layout(l);
        }
        for i in self.shown_panes() {
            match self.panes[i].hit(x, y) {
                PaneHit::None => {}
                h => return Hit::Pane(i, h),
            }
        }
        Hit::None
    }

    fn on_left_down(&mut self, x: f32, y: f32, double: bool) {
        let hit = self.hit(x, y);
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        // Con la paleta abierta: un resultado se abre, el resto del panel no hace nada y fuera
        // de ella se cierra (sin que el clic llegue a lo de abajo).
        if self.palette.is_some() {
            match hit {
                Hit::Palette(Some(i)) => self.activate_palette(i, key_down(VK_CONTROL)),
                Hit::Palette(None) => {
                    if let Some((_, edit)) = &self.palette {
                        unsafe {
                            let _ = SetFocus(Some(*edit));
                        }
                    }
                }
                _ => self.close_palette(),
            }
            return;
        }
        #[cfg(feature = "arbol")]
        if self.on_tree_click(x, y) {
            return;
        }
        // Un clic fuera del panel de colores lo cierra (y sigue su curso).
        if self.picker_open && !matches!(hit, Hit::Picker(_) | Hit::Gear) {
            self.picker_open = false;
            self.redraw();
        }
        if self.term_menu.is_some() && !matches!(hit, Hit::TermMenu(_) | Hit::Pane(_, PaneHit::Terminal)) {
            self.term_menu = None;
            self.redraw();
        }
        match hit {
            Hit::TermMenu(Some(c)) => {
                if let Some(pane) = self.term_menu.take() {
                    self.open_console(pane, c);
                }
            }
            Hit::TermMenu(None) | Hit::Palette(_) => {}
            Hit::Groups => self.show_groups_menu(),
            Hit::Gear => {
                self.picker_open = !self.picker_open;
                self.redraw();
            }
            Hit::Picker(ui::PickerHit::Preset(i)) => self.set_accent(Accent::Rgb(theme::PRESETS[i].1)),
            Hit::Picker(ui::PickerHit::System) => self.set_accent(Accent::System),
            Hit::Picker(ui::PickerHit::Custom) => self.choose_custom_color(),
            Hit::Picker(ui::PickerHit::DarkMode) => self.set_dark_mode(!self.theme.dark),
            Hit::Picker(ui::PickerHit::FileColors) => {
                self.file_colors = !self.file_colors;
                self.redraw();
            }
            Hit::Picker(ui::PickerHit::Tree) => self.run_command(Command::ToggleTree),
            Hit::Picker(ui::PickerHit::Panel) => {}
            Hit::Splitter(s) if double => {
                self.begin_layout_anim();
                self.splits.set(s, 0.5);
                self.redraw();
            }
            Hit::Splitter(s) => {
                unsafe { SetCapture(self.hwnd) };
                self.drag = Drag::Splitter(s);
            }
            Hit::Layout(l) => self.set_layout(l),
            Hit::Pane(i, ph) => {
                self.set_active(i);
                match ph {
                    PaneHit::Tab(j) => {
                        self.select_tab(i, j);
                        unsafe { SetCapture(self.hwnd) };
                        self.drag = Drag::Tab { pane: i, index: j, start_x: x, start_y: y, moved: false };
                    }
                    PaneHit::TabClose(j) => self.close_tab(i, j),
                    PaneHit::NewTab => {
                        let loc = self.panes[i].tab().location.clone();
                        self.new_tab(i, loc, true);
                    }
                    PaneHit::Maximize => self.toggle_maximize(i),
                    PaneHit::FilterButton | PaneHit::FilterBar => self.begin_filter(i),
                    PaneHit::FilterClose => self.clear_filter(i),
                    PaneHit::TabStrip if double => self.toggle_maximize(i),
                    PaneHit::TabStrip => {}
                    PaneHit::Button(b) => {
                        unsafe { SetCapture(self.hwnd) };
                        self.drag = Drag::Button(i, b);
                    }
                    PaneHit::Crumb(k) => {
                        if let Some(loc) = self.panes[i].crumb(k) {
                            self.panes[i].tab_mut().navigate(hwnd, &tx, loc);
                        }
                    }
                    PaneHit::Address => self.begin_edit(i),
                    PaneHit::Star => self.toggle_favorite(i),
                    PaneHit::Terminal => {
                        self.term_menu = if self.term_menu == Some(i) { None } else { Some(i) };
                    }
                    PaneHit::Fav(j) => {
                        unsafe { SetCapture(self.hwnd) };
                        self.drag = Drag::Fav { pane: i, index: j, start_x: x, moved: false };
                    }
                    PaneHit::FavOverflow => {
                        if let Some((r, first)) = self.panes[i].overflow() {
                            self.show_fav_menu(i, FavMenuSource::Overflow(first), r);
                        }
                    }
                    PaneHit::FavBar => {}
                    PaneHit::List if double => {
                        let action = self.panes[i].tab_mut().list.on_double_click(x, y);
                        self.apply(i, action);
                    }
                    PaneHit::List => {
                        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
                        let (action, capture) = self.panes[i].tab_mut().list.on_mouse_down(x, y, ctrl, shift);
                        if capture {
                            unsafe { SetCapture(self.hwnd) };
                            self.drag = Drag::List(i);
                        }
                        self.apply(i, action);
                    }
                    PaneHit::None => {}
                }
            }
            Hit::None => {}
        }
        self.redraw();
    }

    fn on_left_up(&mut self, x: f32, y: f32) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        unsafe {
            let _ = ReleaseCapture();
        }
        let tab_drop = self.tab_drop.take();
        match drag {
            Drag::Button(i, b) if self.hit(x, y) == Hit::Pane(i, PaneHit::Button(b)) => self.on_button(i, b),
            Drag::Tab { pane, index, .. } => {
                if let Some((to, at)) = tab_drop {
                    self.move_tab_to_pane(pane, index, to, at);
                }
            }
            Drag::Fav { pane, index, moved: false, .. } => self.activate_fav(pane, &[index], key_down(VK_CONTROL)),
            Drag::Fav { index, .. } => {
                // Soltado sobre otra carpeta de favoritos: va adentro; si no, se reordena.
                match self.fav_drop.take().map(|(_, d)| d) {
                    Some(FavDrop::Into(g)) if g != index => {
                        if let Some(f) = self.favorites.remove(&[index]) {
                            let g = if g > index { g - 1 } else { g };
                            self.favorites.push_into(&[g], f);
                        }
                    }
                    Some(FavDrop::Before(to)) => self.favorites.move_root(index, to),
                    _ => {}
                }
                self.save_favorites();
            }
            Drag::List(i) => {
                let action = self.panes[i].tab_mut().list.on_mouse_up();
                self.apply(i, action);
            }
            _ => {}
        }
        self.redraw();
    }

    fn on_middle_click(&mut self, x: f32, y: f32) {
        match self.hit(x, y) {
            Hit::Pane(i, PaneHit::Tab(j) | PaneHit::TabClose(j)) => self.close_tab(i, j),
            Hit::Pane(i, PaneHit::Fav(j)) => self.activate_fav(i, &[j], true),
            // Clic medio en una carpeta: la abre en una pestaña nueva, sin cambiar de pestaña.
            Hit::Pane(i, PaneHit::List) => {
                let tab = self.panes[i].tab();
                if let Some(idx) = tab.list.entry_at(x, y) {
                    if tab.list.entries[idx].is_dir {
                        let path = tab.entry_path(idx);
                        self.new_tab(i, Location::Path(path), false);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_mouse_move(&mut self, x: f32, y: f32) {
        if !self.tracking_mouse {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            self.tracking_mouse = unsafe { TrackMouseEvent(&mut tme) }.is_ok();
        }
        let mut redraw = false;
        match self.drag {
            Drag::Splitter(s) => {
                let (w, h) = self.client_dips();
                self.splits.set(s, ui::split_value(s, ui::panes_area(w, h, self.tree_width()), x, y));
                redraw = true;
            }
            Drag::Tab { pane, index, start_x, start_y, moved } => {
                if moved || (x - start_x).abs() > 6.0 || (y - start_y).abs() > 6.0 {
                    // Sobre otro panel: se marca dónde caería; sobre el propio, se reordena; sobre
                    // un divisor o la barra de estado, nada.
                    let hit = self.hit(x, y);
                    let target = match hit {
                        Hit::Pane(j, ph) if j != pane => {
                            let at = match ph {
                                PaneHit::Tab(_) | PaneHit::TabClose(_) | PaneHit::NewTab | PaneHit::TabStrip | PaneHit::Maximize => {
                                    self.panes[j].insert_index_at(x)
                                }
                                _ => self.panes[j].tabs.len(),
                            };
                            Some((j, at))
                        }
                        _ => None,
                    };
                    if target != self.tab_drop {
                        self.tab_drop = target;
                        redraw = true;
                    }
                    if matches!(hit, Hit::Pane(j, _) if j == pane) {
                        if let Some(to) = self.panes[pane].tab_index_at(x) {
                            self.panes[pane].move_tab(index, to);
                            self.drag = Drag::Tab { pane, index: to, start_x, start_y, moved: true };
                            redraw = true;
                        }
                    }
                    if let Drag::Tab { moved, .. } = &mut self.drag {
                        *moved = true;
                    }
                }
            }
            Drag::List(i) => {
                let action = self.panes[i].tab_mut().list.on_mouse_move(x, y);
                // Unos píxeles con el botón apretado sobre un elemento: arrastrar los seleccionados
                // (a otro panel, al Explorador o a otra aplicación).
                if action == Action::BeginDrag {
                    self.apply(i, action);
                    return;
                }
                redraw |= action != Action::None;
            }
            Drag::Fav { pane, index, start_x, moved } => {
                if moved || (x - start_x).abs() > 5.0 {
                    self.drag = Drag::Fav { pane, index, start_x, moved: true };
                    let target = self.panes[pane].fav_drop_at(&self.favorites, x);
                    self.fav_drop = Some((pane, target));
                    redraw = true;
                }
            }
            _ => {}
        }

        let hover = self.hit(x, y);
        redraw |= hover != self.hover;
        #[cfg(feature = "arbol")]
        {
            redraw |= self.tree.on_mouse_move(x, y);
        }
        if let Some((pal, _)) = &mut self.palette {
            let h = match hover {
                Hit::Palette(Some(i)) => Some(i),
                _ => None,
            };
            if pal.hover != h {
                pal.hover = h;
                redraw = true;
            }
        }
        // La fila resaltada solo en el panel bajo el mouse.
        for (i, pane) in self.panes.iter_mut().enumerate() {
            if self.drag == Drag::List(i) {
                continue;
            }
            let list = &mut pane.tab_mut().list;
            redraw |= if hover == Hit::Pane(i, PaneHit::List) {
                list.on_mouse_move(x, y) != Action::None
            } else {
                list.on_mouse_leave() != Action::None
            };
        }
        self.hover = hover;
        if redraw {
            self.redraw();
        }
    }

    /// Atajos de la ventana. Devuelve si la tecla se usó.
    fn on_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
        let active = self.active;
        match vk {
            VK_ESCAPE if self.picker_open => self.picker_open = false,
            VK_ESCAPE if self.term_menu.is_some() => self.term_menu = None,
            VK_ESCAPE if !self.panes[active].tab().list.filter().is_empty() => self.clear_filter(active),
            _ => match self.config.keymap.lookup(vk, ctrl, shift, false) {
                Some(c) => self.run_command(c),
                None => match self.tab_mut().list.on_key(vk.0, ctrl, shift) {
                    Some(action) => {
                        self.apply(active, action);
                        return true;
                    }
                    None => return false,
                },
            },
        }
        self.redraw();
        true
    }

    /// Ejecuta una acción sobre el panel activo (atajo de teclado o paleta de comandos).
    fn run_command(&mut self, c: Command) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let active = self.active;
        match c {
            Command::JumpTo => self.open_palette(PaletteMode::Folders),
            Command::Palette => self.open_palette(PaletteMode::All),
            Command::NewTab => {
                let loc = self.panes[active].tab().location.clone();
                self.new_tab(active, loc, true);
            }
            Command::CloseTab => {
                let i = self.panes[active].active;
                self.close_tab(active, i);
            }
            Command::ReopenTab => {
                if let Some(loc) = self.panes[active].pop_closed() {
                    self.new_tab(active, loc, true);
                }
            }
            Command::NextTab | Command::PrevTab => {
                self.panes[active].cycle_tab(c == Command::NextTab);
                self.sync_visibility();
            }
            Command::NextPane | Command::PrevPane => {
                let n = self.layout.panes();
                self.set_active(if c == Command::PrevPane { (active + n - 1) % n } else { (active + 1) % n });
            }
            Command::FocusPane(i) => self.set_active(i as usize),
            Command::Layout(i) => {
                if let Some(l) = Layout::from_index(i as usize) {
                    self.set_layout(l);
                }
            }
            Command::Maximize => self.toggle_maximize(active),
            Command::Back => self.tab_mut().go_back(hwnd, &tx),
            Command::Forward => self.tab_mut().go_forward(hwnd, &tx),
            Command::Up => self.tab_mut().go_up(hwnd, &tx),
            Command::Refresh => self.tab_mut().reload(hwnd, &tx, false),
            Command::EditAddress => self.begin_edit(active),
            Command::Filter => self.begin_filter(active),
            Command::GoComputer => self.tab_mut().navigate(hwnd, &tx, Location::Computer),
            Command::ToggleHidden => {
                // En todas las pestañas a la vez.
                self.show_hidden = !self.show_hidden;
                for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
                    if tab.list.show_hidden != self.show_hidden {
                        tab.list.toggle_hidden();
                    }
                }
            }
            Command::SelectAll => self.tab_mut().list.select_all(),
            Command::InvertSelection => self.tab_mut().list.invert_selection(),
            Command::Copy => self.copy_selection(false),
            Command::Cut => self.copy_selection(true),
            Command::Paste => self.paste(),
            Command::CopyPaths => {
                let text: Vec<String> = self.selection(active).iter().map(|p| format!("\"{}\"", p.display())).collect();
                if !text.is_empty() {
                    let _ = shell::copy_text(hwnd, &text.join("\r\n"));
                }
            }
            Command::Undo => {
                if let Some(undo) = self.undo.pop() {
                    ops::spawn(hwnd, self.ops_tx.clone(), Op::Undo(undo));
                }
            }
            Command::NewFolder => {
                if let Some(parent) = self.panes[active].tab().location.target() {
                    ops::spawn(hwnd, self.ops_tx.clone(), Op::NewFolder { parent });
                }
            }
            Command::Recycle => self.delete_selection(true),
            Command::Delete => self.delete_selection(false),
            Command::Rename => self.begin_rename(active),
            Command::Properties => {
                // De la selección, o de la carpeta si no hay nada seleccionado.
                let items = self.selection_items(active);
                match (&items[..], &self.panes[active].tab().location) {
                    ([], Location::Path(p)) => shell::properties(hwnd, &[p.clone().into()]),
                    _ => shell::properties(hwnd, &items),
                }
            }
            Command::ToggleFavorite => self.toggle_favorite(active),
            Command::ToggleFavBar => self.show_favorites = !self.show_favorites,
            Command::PowerShell => self.open_console(active, Console::PowerShell),
            Command::Cmd => self.open_console(active, Console::Cmd),
            Command::ToggleDarkMode => self.set_dark_mode(!self.theme.dark),
            Command::SaveGroup => self.open_palette(PaletteMode::GroupName),
            Command::View(i) => {
                if let Some(&(view, ..)) = list::VIEWS.get(i as usize) {
                    self.set_view(active, view);
                }
            }
            Command::SortBy(i) => {
                if let Some(&(col, ..)) = list::COLUMNS.get(i as usize) {
                    self.tab_mut().list.set_sort(col);
                }
            }
            Command::ToggleTree => {
                #[cfg(feature = "arbol")]
                {
                    self.begin_layout_anim();
                    self.tree.visible = !self.tree.visible;
                }
            }
            Command::OpenConfig => {
                if let Some(path) = config::file_path() {
                    if !path.exists() {
                        self.config = Config::load();
                    }
                    open_in_editor(hwnd, &path);
                }
            }
        }
        self.redraw();
    }

    /// Abre un grupo guardado: su layout, divisores y las pestañas de cada panel. Las pestañas
    /// que había quedan entre las cerradas (Ctrl+Shift+T las recupera).
    fn apply_group(&mut self, index: usize) {
        let Some(g) = self.groups.items.get(index).cloned() else { return };
        self.end_edit(false);
        self.end_filter(true);
        self.begin_layout_anim();
        self.maximized = None;
        self.layout = Layout::from_index(g.layout).unwrap_or(Layout::Four);
        self.splits = Splits { x: g.split_x, y: g.split_y };
        for (i, state) in g.panes.iter().enumerate().take(PANES) {
            if state.tabs.is_empty() {
                continue;
            }
            let tabs: Vec<Tab> = state.tabs.iter().cloned().map(|l| self.make_tab(l)).collect();
            self.panes[i].replace_tabs(tabs, state.active);
        }
        self.active = 0;
        self.notice = Some(format!("Grupo «{}»", g.name));
        self.sync_visibility();
        self.redraw();
    }

    /// Guarda los paneles y carpetas actuales como grupo (reemplaza uno con el mismo nombre).
    fn save_group(&mut self, name: &str) {
        let s = self.session();
        self.groups.put(Group { name: name.to_string(), layout: s.layout, split_x: s.split_x, split_y: s.split_y, panes: s.panes });
        self.notice = Some(match self.groups.save() {
            Ok(()) => format!("Grupo «{name}» guardado"),
            Err(e) => format!("No se pudo guardar el grupo: {e}"),
        });
        self.redraw();
    }

    /// Menú del botón de grupos: abrir uno, guardar el actual, eliminar o editar el archivo.
    fn show_groups_menu(&mut self) {
        const SAVE: usize = 1;
        const EDIT: usize = 2;
        const OPEN: usize = 1000;
        const DELETE: usize = 2000;
        let (w, h) = self.client_dips();
        let r = ui::groups_rect(w, h);
        let k = self.dpi as f32 / 96.0;
        let mut pt = POINT { x: (r.left * k) as i32, y: (r.top * k) as i32 };
        let cmd = unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            let add = |menu: HMENU, id: usize, text: &str| {
                let wide: Vec<u16> = text.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
            };
            for (i, g) in self.groups.items.iter().enumerate() {
                add(hmenu, OPEN + i, &g.name);
            }
            if self.groups.items.is_empty() {
                let wide: Vec<u16> = "Todavía no hay grupos".encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(hmenu, MF_STRING | MF_GRAYED, 0, PCWSTR(wide.as_ptr()));
            }
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            add(hmenu, SAVE, "Guardar paneles y carpetas como grupo…");
            if !self.groups.items.is_empty() {
                if let Ok(sub) = CreatePopupMenu() {
                    for (i, g) in self.groups.items.iter().enumerate() {
                        add(sub, DELETE + i, &g.name);
                    }
                    let wide: Vec<u16> = "Eliminar".encode_utf16().chain(Some(0)).collect();
                    let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, PCWSTR(wide.as_ptr()));
                }
                add(hmenu, EDIT, "Editar grupos.toml");
            }
            let _ = ClientToScreen(self.hwnd, &mut pt);
            // Hacia arriba desde el botón: la barra de estado está abajo de todo.
            let flags = TPM_RETURNCMD | TPM_LEFTALIGN | TPM_BOTTOMALIGN;
            let cmd = TrackPopupMenuEx(hmenu, flags.0, pt.x, pt.y, self.hwnd, None).0 as usize;
            let _ = DestroyMenu(hmenu);
            cmd
        };
        match cmd {
            SAVE => self.open_palette(PaletteMode::GroupName),
            EDIT => {
                if let Some(dir) = session::config_dir() {
                    open_in_editor(self.hwnd, &dir.join("grupos.toml"));
                }
            }
            c if c >= DELETE => {
                let i = c - DELETE;
                if let Some(g) = self.groups.items.get(i) {
                    self.notice = Some(format!("Grupo «{}» eliminado", g.name));
                    self.groups.remove(i);
                    let _ = self.groups.save();
                }
            }
            c if c >= OPEN => self.apply_group(c - OPEN),
            _ => {}
        }
        self.redraw();
    }

    /// Vuelve a leer `config.toml` y `grupos.toml` si cambiaron (se editan con la app abierta).
    fn reload_config(&mut self) {
        if self.config.changed_on_disk() {
            self.config = Config::load();
            self.apply_animation_setting();
            self.notice = Some(match self.config.errors.first() {
                Some(e) => format!("config.toml, {e}"),
                None => "Configuración aplicada".to_string(),
            });
            self.redraw();
        }
        let groups = Groups::load();
        if groups != self.groups {
            self.groups = groups;
        }
    }

    fn on_button(&mut self, pane: usize, b: Button) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab_mut();
        match b {
            Button::Back => tab.go_back(hwnd, &tx),
            Button::Forward => tab.go_forward(hwnd, &tx),
            Button::Up => tab.go_up(hwnd, &tx),
            Button::Refresh => tab.reload(hwnd, &tx, false),
        }
    }

    fn apply(&mut self, pane: usize, action: Action) {
        match action {
            Action::None => return,
            Action::Redraw => {}
            Action::Open(idx) => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                self.panes[pane].tab_mut().open(hwnd, &tx, idx);
            }
            Action::BeginDrag => {
                self.start_drag(pane);
                return;
            }
            Action::ColumnsChanged => {
                let cols = self.panes[pane].tab().list.columns.clone();
                self.set_columns(cols);
            }
        }
        self.redraw();
    }

    fn redraw(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn mouse_pos(&self, lparam: LPARAM) -> (f32, f32) {
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        let k = 96.0 / self.dpi as f32;
        (x * k, y * k)
    }

    fn cursor_pos(&self) -> (f32, f32) {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let k = 96.0 / self.dpi as f32;
        (pt.x as f32 * k, pt.y as f32 * k)
    }

    // ---------- operaciones de archivos ----------

    /// Rutas seleccionadas en la pestaña activa de un panel.
    fn selection(&self, pane: usize) -> Vec<PathBuf> {
        let tab = self.panes[pane].tab();
        tab.list.selected_indices().map(|i| tab.entry_path(i)).collect()
    }

    /// Selección como elementos del Shell (con PIDL si son virtuales).
    fn selection_items(&self, pane: usize) -> Vec<shell::Item> {
        let tab = self.panes[pane].tab();
        tab.list.selected_indices().map(|i| tab.entry_item(i)).collect()
    }

    fn copy_selection(&mut self, cut: bool) {
        let items = self.selection_items(self.active);
        if !items.is_empty() {
            if let Err(e) = shell::copy_files(&items, cut) {
                self.notice = Some(format!("No se pudo copiar: {}", e.message()));
            }
        }
    }

    fn paste(&mut self) {
        let Some(dest) = self.panes[self.active].tab().location.target() else { return };
        let Some((sources, cut)) = shell::clipboard_files() else { return };
        let op = if cut { Op::Move { sources, dest } } else { Op::Copy { sources, dest } };
        ops::spawn(self.hwnd, self.ops_tx.clone(), op);
        // Lo cortado se pega una sola vez, como en el Explorador.
        if cut {
            shell::clear_clipboard();
        }
    }

    fn delete_selection(&mut self, recycle: bool) {
        // En carpetas virtuales borra el propio Shell (en la Papelera: eliminar definitivamente).
        let location = self.panes[self.active].tab().location.clone();
        if let Location::Shell { .. } = location {
            let items = self.selection_items(self.active);
            if !items.is_empty() {
                let _ = shell::invoke_verb(self.hwnd, &location, &items, "delete");
                self.reload_shell_tabs();
            }
            return;
        }
        let paths = self.selection(self.active);
        if !paths.is_empty() && self.panes[self.active].tab().location.target().is_some() {
            ops::spawn(self.hwnd, self.ops_tx.clone(), Op::Delete { paths, recycle });
        }
    }

    fn on_op_done(&mut self, result: OpResult) {
        self.notice = result.error.map(|e| format!("Error: {e}"));
        if let Some(undo) = result.undo {
            self.undo.push(undo);
            if self.undo.len() > UNDO_MAX {
                self.undo.remove(0);
            }
        }
        self.reload_shell_tabs();
        let Some(path) = result.focus else { return };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        // Las pestañas que muestran esa carpeta enfocan el elemento al recargar.
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if tab.location.is_path(parent) && tab.is_loaded() {
                tab.focus_after_load(name.to_os_string());
                tab.reload(hwnd, &tx, true);
            }
        }
        if result.rename_after {
            self.pending_rename = Some(path);
        }
    }

    /// Las carpetas virtuales no tienen vigilancia: se recargan después de cada operación.
    fn reload_shell_tabs(&mut self) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let visible = self.layout.panes();
        for pane in &mut self.panes[..visible] {
            let tab = pane.tab_mut();
            if matches!(tab.location, Location::Shell { .. }) && !tab.loading {
                tab.reload(hwnd, &tx, true);
            }
        }
    }

    /// Abre el renombrado sobre la carpeta recién creada cuando ya aparece en la lista.
    fn try_pending_rename(&mut self) {
        let Some(path) = self.pending_rename.clone() else { return };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        let pane = self.active;
        let tab = self.panes[pane].tab_mut();
        if !tab.location.is_path(parent) || tab.loading {
            return;
        }
        if let Some(idx) = tab.list.entries.iter().position(|e| e.name.eq_ignore_ascii_case(name)) {
            tab.list.select_index(idx);
            self.pending_rename = None;
            self.begin_rename(pane);
        }
    }

    fn start_drag(&mut self, pane: usize) {
        let items = self.selection_items(pane);
        if items.is_empty() {
            return;
        }
        let obj = match shell::data_object(&items) {
            Ok(o) => o,
            Err(e) => {
                self.notice = Some(format!("No se pudo arrastrar: {e}"));
                self.redraw();
                return;
            }
        };
        let result = unsafe {
            let _ = ReleaseCapture();
            self.drag = Drag::None;
            // Bucle modal: mientras dura, los eventos de soltar llegan por `WM_APP_DRAG`.
            SHDoDragDrop(Some(self.hwnd), &obj, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)
        };
        // Cancelar con Esc no es un error; que Windows no pueda arrastrar, sí: se avisa.
        if let Err(e) = result {
            self.notice = Some(format!("No se pudo arrastrar: {e}"));
        }
        self.redraw();
    }

    /// Carpeta destino bajo un punto: una carpeta de la lista o la carpeta del panel.
    fn drop_folder(&self, x: f32, y: f32) -> Option<(usize, Option<usize>, PathBuf)> {
        let Hit::Pane(i, ph) = self.hit(x, y) else { return None };
        let tab = self.panes[i].tab();
        if ph == PaneHit::List {
            if let Some(idx) = tab.list.entry_at(x, y) {
                if tab.list.entries[idx].is_dir {
                    return Some((i, Some(idx), tab.entry_path(idx)));
                }
            }
        }
        tab.location.target().map(|t| (i, None, t))
    }

    fn on_drag(&mut self, ev: &mut DragEvent) {
        let mut pt = ev.point;
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let k = 96.0 / self.dpi as f32;
        if self.on_drag_favbar(ev, pt.x as f32 * k, pt.y as f32 * k) {
            return;
        }
        let target = match ev.kind {
            DragKind::Leave => None,
            _ => self.drop_folder(pt.x as f32 * k, pt.y as f32 * k),
        };
        ev.effect = match (&target, ev.paths) {
            (Some((_, _, dest)), Some(paths)) => drop_effect(paths, dest, ev.keys, ev.allowed),
            _ => DROPEFFECT_NONE,
        };

        // Resaltado de la carpeta destino.
        let mut changed = false;
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let want = match &target {
                Some((p, Some(idx), _)) if *p == i && ev.effect != DROPEFFECT_NONE && ev.kind != DragKind::Drop => {
                    Some(*idx)
                }
                _ => None,
            };
            let list = &mut pane.tab_mut().list;
            if list.drop_target != want {
                list.drop_target = want;
                changed = true;
            }
        }
        if changed {
            self.redraw();
        }

        if ev.kind == DragKind::Drop && ev.effect != DROPEFFECT_NONE {
            if let (Some((pane, _, dest)), Some(paths)) = (target, ev.paths) {
                let sources = paths.to_vec();
                let op = if ev.effect == DROPEFFECT_MOVE { Op::Move { sources, dest } } else { Op::Copy { sources, dest } };
                ops::spawn(self.hwnd, self.ops_tx.clone(), op);
                self.set_active(pane);
            }
        }
    }

    fn on_context_menu(&mut self, lparam: LPARAM) {
        let pane = self.active;
        // lparam -1: tecla de menú o Shift+F10; se abre sobre la fila con foco.
        let screen = if lparam.0 == -1 {
            let (x, y) = self.panes[pane].tab().list.focus_point().unwrap_or((40.0, 80.0));
            let k = self.dpi as f32 / 96.0;
            let mut pt = POINT { x: (x * k) as i32, y: (y * k) as i32 };
            unsafe {
                let _ = ClientToScreen(self.hwnd, &mut pt);
            }
            pt
        } else {
            POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 }
        };
        let location = self.panes[pane].tab().location.clone();
        let items = self.selection_items(pane);
        // En carpetas virtuales el Shell resuelve todo: conoce el contexto (p. ej. la Papelera).
        let intercept = !matches!(location, Location::Shell { .. });
        let chosen = shell::context_menu(self.hwnd, &location, &items, screen, key_down(VK_SHIFT), intercept);
        match chosen.as_deref() {
            Some("rename") => self.begin_rename(pane),
            Some("delete") => self.delete_selection(!key_down(VK_SHIFT)),
            Some("cut") => self.copy_selection(true),
            Some("copy") => self.copy_selection(false),
            Some("paste") => self.paste(),
            _ => {}
        }
        self.redraw();
    }

    // ---------- consola ----------

    /// Abre la consola en la carpeta del panel.
    fn open_console(&mut self, pane: usize, console: Console) {
        if let Location::Path(dir) = self.panes[pane].tab().location.clone() {
            if !console.open(self.hwnd, &dir) {
                self.notice = Some(format!("No se pudo abrir {}", console.label()));
            }
        }
        self.redraw();
    }

    // ---------- color de acento ----------

    fn set_accent(&mut self, accent: Accent) {
        self.accent = accent;
        self.theme = Theme::from_system(accent, self.dark_mode);
        self.update_edit_resources();
        self.redraw();
    }

    /// Switch del panel de la rueda dentada: fija oscuro o claro (deja de seguir a Windows).
    fn set_dark_mode(&mut self, dark: bool) {
        self.dark_mode = Some(dark);
        self.theme = Theme::from_system(self.accent, self.dark_mode);
        // Barra de título y menús nativos al mismo modo.
        let mica = self.theme.apply_to_window(self.hwnd);
        self.renderer.set_transparent(mica);
        self.transparent = mica;
        shell::set_menu_mode(self.dark_mode);
        self.update_edit_resources();
        self.redraw();
    }

    /// "Personalizado…": el selector de color de Windows (espectro, RGB y HSL).
    fn choose_custom_color(&mut self) {
        let current = match self.accent {
            Accent::Rgb(c) => c,
            Accent::System => theme::PRESETS[0].1,
        };
        // COLORREF es 0x00BBGGRR.
        let bgr = |c: u32| COLORREF((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16));
        let mut cc = CHOOSECOLORW {
            lStructSize: size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: self.hwnd,
            rgbResult: bgr(current),
            lpCustColors: self.custom_colors.as_mut_ptr(),
            Flags: CC_FULLOPEN | CC_RGBINIT | CC_ANYCOLOR,
            ..Default::default()
        };
        if unsafe { ChooseColorW(&mut cc) }.as_bool() {
            let c = cc.rgbResult.0;
            self.set_accent(Accent::Rgb((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF));
        }
    }

    // ---------- favoritos ----------

    fn save_favorites(&mut self) {
        if let Err(e) = self.favorites.save() {
            self.notice = Some(format!("No se pudieron guardar los favoritos: {e}"));
        }
        self.redraw();
    }

    /// Estrella / Ctrl+D: agrega la carpeta del panel a favoritos, o la quita si ya está.
    fn toggle_favorite(&mut self, pane: usize) {
        let location = self.panes[pane].tab().location.clone();
        let title = location.title();
        match self.favorites.find(&location) {
            Some(path) => {
                self.favorites.remove(&path);
                self.notice = Some(format!("Quitado de favoritos: {title}"));
            }
            None => {
                self.favorites.items.push(Fav::Link { name: title.clone(), location });
                self.notice = Some(format!("Agregado a favoritos: {title}"));
            }
        }
        self.save_favorites();
    }

    /// Abre un favorito: navega el panel (o abre una pestaña nueva). Una carpeta de favoritos
    /// despliega su menú; con `new_tab`, abre todo su contenido en pestañas nuevas.
    fn activate_fav(&mut self, pane: usize, path: &[usize], new_tab: bool) {
        match self.favorites.get(path).cloned() {
            Some(Fav::Link { location, .. }) if new_tab => self.new_tab(pane, location, false),
            Some(Fav::Link { location, .. }) => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                self.panes[pane].tab_mut().navigate(hwnd, &tx, location);
            }
            Some(Fav::Group { items, .. }) if new_tab => {
                for f in items {
                    if let Fav::Link { location, .. } = f {
                        self.new_tab(pane, location, false);
                    }
                }
            }
            Some(Fav::Group { .. }) => {
                let anchor = path.first().and_then(|&i| self.panes[pane].fav_rect(i)).unwrap_or(self.panes[pane].favbar);
                self.show_fav_menu(pane, FavMenuSource::Group(path.to_vec()), anchor);
            }
            None => {}
        }
        self.redraw();
    }

    /// Menú desplegable de una carpeta de favoritos o del desborde "»".
    fn show_fav_menu(&mut self, pane: usize, source: FavMenuSource, anchor: render::Rect) {
        let entries: Vec<(FavPath, Fav)> = match &source {
            FavMenuSource::Group(path) => match self.favorites.get(path) {
                Some(Fav::Group { items, .. }) => items
                    .iter()
                    .enumerate()
                    .map(|(j, f)| ([path.as_slice(), &[j]].concat(), f.clone()))
                    .collect(),
                _ => return,
            },
            FavMenuSource::Overflow(first) => {
                self.favorites.items.iter().enumerate().skip(*first).map(|(i, f)| (vec![i], f.clone())).collect()
            }
        };
        let mut menu = FavMenu { ids: Vec::new(), positions: HashMap::new(), action: None };
        let k = self.dpi as f32 / 96.0;
        let mut pt = POINT { x: (anchor.left * k) as i32, y: (anchor.bottom * k) as i32 };
        unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            build_fav_menu(&mut menu, hmenu, &entries);
            let _ = ClientToScreen(self.hwnd, &mut pt);
            self.fav_menu = Some(menu);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN).0, pt.x, pt.y, self.hwnd, None).0;
            let menu = self.fav_menu.take();
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
    fn on_fav_menu_right_click(&mut self, hmenu: isize, pos: u32) {
        let Some(path) = self.fav_menu.as_ref().and_then(|m| m.positions.get(&(hmenu, pos)).cloned()) else { return };
        if let Some(act) = self.fav_context_menu(Some(&path), true) {
            if let Some(m) = self.fav_menu.as_mut() {
                m.action = Some((path, act));
            }
            unsafe {
                let _ = EndMenu();
            }
        }
    }

    /// Menú contextual de la barra de favoritos (o de un favorito). Devuelve la acción elegida.
    fn fav_context_menu(&self, path: Option<&FavPath>, recurse: bool) -> Option<FavAct> {
        let fav = path.and_then(|p| self.favorites.get(p));
        unsafe {
            let hmenu = CreatePopupMenu().ok()?;
            let add = |id: FavAct, text: &str| {
                let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(hmenu, MF_STRING, id as usize, PCWSTR(wide.as_ptr()));
            };
            let sep = || {
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            };
            match fav {
                Some(Fav::Link { .. }) => {
                    add(FavAct::Open, "Abrir");
                    add(FavAct::OpenNewTab, "Abrir en pestaña nueva");
                    sep();
                }
                Some(Fav::Group { .. }) => {
                    add(FavAct::OpenNewTab, "Abrir todo en pestañas nuevas");
                    sep();
                }
                None => {}
            }
            if fav.is_some() {
                add(FavAct::Rename, "Renombrar");
                add(FavAct::Delete, "Eliminar");
                if path.is_some_and(|p| p.len() > 1) {
                    add(FavAct::MoveOut, "Sacar de la carpeta");
                }
                sep();
            }
            if !recurse {
                add(FavAct::NewGroup, "Nueva carpeta de favoritos");
                add(FavAct::AddCurrent, "Agregar la carpeta actual");
                sep();
                add(FavAct::EditFile, "Editar el archivo de favoritos");
                add(FavAct::HideBar, "Ocultar la barra de favoritos (Ctrl+Shift+B)");
            }
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
            if recurse {
                flags |= TPM_RECURSE;
            }
            let cmd = TrackPopupMenuEx(hmenu, flags.0, pt.x, pt.y, self.hwnd, None).0;
            let _ = DestroyMenu(hmenu);
            FavAct::from_id(cmd)
        }
    }

    fn fav_action(&mut self, pane: usize, path: Option<FavPath>, act: FavAct) {
        match (act, path) {
            (FavAct::Open, Some(p)) => self.activate_fav(pane, &p, false),
            (FavAct::OpenNewTab, Some(p)) => self.activate_fav(pane, &p, true),
            (FavAct::Rename, Some(p)) => self.begin_fav_edit(pane, p),
            (FavAct::Delete, Some(p)) => {
                self.favorites.remove(&p);
                self.save_favorites();
            }
            (FavAct::MoveOut, Some(p)) => {
                if let Some(f) = self.favorites.remove(&p) {
                    self.favorites.items.push(f);
                    self.save_favorites();
                }
            }
            (FavAct::NewGroup, p) => {
                // Al lado del favorito sobre el que se hizo clic, o al final.
                let at = p.and_then(|p| p.first().copied()).map(|i| i + 1).unwrap_or(self.favorites.items.len());
                let group = Fav::Group { name: "Nueva carpeta".into(), items: Vec::new() };
                self.favorites.insert_root(at, group);
                self.save_favorites();
                self.begin_fav_edit(pane, vec![at.min(self.favorites.items.len() - 1)]);
            }
            (FavAct::AddCurrent, _) => {
                if self.favorites.find(&self.panes[pane].tab().location).is_none() {
                    self.toggle_favorite(pane);
                }
            }
            (FavAct::EditFile, _) => {
                // Se guarda antes para que exista; al volver a la ventana se relee.
                self.save_favorites();
                if let Some(file) = Favorites::file_path() {
                    let wide = fs::wide_path(&file);
                    unsafe {
                        ShellExecuteW(Some(self.hwnd), w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL);
                    }
                }
            }
            (FavAct::HideBar, _) => {
                self.show_favorites = false;
                self.redraw();
            }
            _ => {}
        }
    }

    /// Renombrado de un favorito en el lugar (o sobre la barra si está dentro de una carpeta).
    fn begin_fav_edit(&mut self, pane: usize, path: FavPath) {
        if self.fav_edit.is_some() || self.edit.is_some() || self.rename.is_some() {
            return;
        }
        let Some(name) = self.favorites.get(&path).map(|f| f.name().to_string()) else { return };
        // Dibujar ya, para ubicar el favorito donde quedó.
        self.paint();
        let pane_ref = &self.panes[pane];
        let bar = pane_ref.favbar;
        let r = match path.as_slice() {
            [i] => pane_ref.fav_rect(*i).unwrap_or(bar),
            _ => bar,
        };
        let r = render::rect(r.left, r.top, r.right.max(r.left + 200.0).min(bar.right), r.bottom);
        let k = self.dpi as f32 / 96.0;
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                (r.left * k) as i32,
                (r.top * k) as i32,
                ((r.right - r.left) * k) as i32,
                ((r.bottom - r.top) * k) as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_FAVEDIT_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetFocus(Some(edit));
            self.fav_edit = Some((edit, path));
        }
    }

    fn end_fav_edit(&mut self, commit: bool) {
        let Some((edit, path)) = self.fav_edit.take() else { return };
        let mut buf = vec![0u16; 512];
        let text = unsafe {
            let len = GetWindowTextW(edit, &mut buf);
            let _ = DestroyWindow(edit);
            let _ = SetFocus(Some(self.hwnd));
            String::from_utf16_lossy(&buf[..len.max(0) as usize])
        };
        let name = text.trim().to_string();
        if commit && !name.is_empty() {
            if let Some(f) = self.favorites.get_mut(&path) {
                f.set_name(name);
            }
            self.save_favorites();
        }
        self.redraw();
    }

    /// Arrastre de carpetas (desde la lista o el Explorador) sobre la barra de favoritos.
    /// Devuelve si el punto está sobre la barra (y el evento quedó resuelto).
    fn on_drag_favbar(&mut self, ev: &mut DragEvent, x: f32, y: f32) -> bool {
        let over = match (self.show_favorites, ev.kind, self.hit(x, y)) {
            (true, DragKind::Enter | DragKind::Over | DragKind::Drop, Hit::Pane(i, PaneHit::Fav(_) | PaneHit::FavBar | PaneHit::FavOverflow)) => Some(i),
            _ => None,
        };
        let Some(pane) = over else {
            if self.fav_drop.take().is_some() {
                self.redraw();
            }
            return false;
        };
        let dirs: Vec<PathBuf> = ev.paths.map(|p| p.iter().filter(|p| p.is_dir()).cloned().collect()).unwrap_or_default();
        // Un favorito es un vínculo: no se copia ni se mueve nada.
        ev.effect = if dirs.is_empty() {
            DROPEFFECT_NONE
        } else if ev.allowed.0 & DROPEFFECT_LINK.0 != 0 {
            DROPEFFECT_LINK
        } else if ev.allowed.0 & DROPEFFECT_COPY.0 != 0 {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        };
        let target = self.panes[pane].fav_drop_at(&self.favorites, x);
        for p in &mut self.panes {
            p.tab_mut().list.drop_target = None;
        }
        if ev.kind == DragKind::Drop {
            self.fav_drop = None;
            if ev.effect != DROPEFFECT_NONE {
                let new: Vec<Fav> = dirs
                    .into_iter()
                    .map(|d| Fav::Link { name: Location::Path(d.clone()).title(), location: Location::Path(d) })
                    .collect();
                match target {
                    FavDrop::Into(g) => {
                        for f in new {
                            self.favorites.push_into(&[g], f);
                        }
                    }
                    FavDrop::Before(at) => {
                        for (n, f) in new.into_iter().enumerate() {
                            self.favorites.insert_root(at + n, f);
                        }
                    }
                }
                self.save_favorites();
            }
        } else {
            let want = (ev.effect != DROPEFFECT_NONE).then_some((pane, target));
            if want != self.fav_drop {
                self.fav_drop = want;
            }
        }
        self.redraw();
        true
    }

    // ---------- renombrado en línea ----------

    fn begin_rename(&mut self, pane: usize) {
        if self.rename.is_some() || self.edit.is_some() {
            return;
        }
        let tab = self.panes[pane].tab_mut();
        if tab.location.target().is_none() {
            return;
        }
        let Some(idx) = tab.list.focused_index() else { return };
        let Some(r) = tab.list.name_rect(idx) else { return };
        let name = tab.list.entries[idx].display.clone();
        let is_dir = tab.list.entries[idx].is_dir;
        let path = tab.entry_path(idx);
        // Se selecciona el nombre sin la extensión, como en el Explorador.
        let stem_len = match name.rfind('.') {
            Some(dot) if !is_dir && dot > 0 => name[..dot].encode_utf16().count(),
            _ => name.encode_utf16().count(),
        };
        self.redraw();

        let k = self.dpi as f32 / 96.0;
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                (r.left * k) as i32,
                (r.top * k) as i32,
                ((r.right - r.left) * k) as i32,
                ((r.bottom - r.top) * k) as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_RENAME_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem_len as isize)));
            let _ = SetFocus(Some(edit));
            self.rename = Some(Rename { edit, path });
        }
    }

    fn end_rename(&mut self, commit: bool) {
        let Some(Rename { edit, path }) = self.rename.take() else { return };
        let mut buf = vec![0u16; 1024];
        let text = unsafe {
            let len = GetWindowTextW(edit, &mut buf);
            let _ = DestroyWindow(edit);
            let _ = SetFocus(Some(self.hwnd));
            String::from_utf16_lossy(&buf[..len.max(0) as usize])
        };
        let new_name = text.trim().to_string();
        let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if commit && !new_name.is_empty() && new_name != old_name {
            ops::spawn(self.hwnd, self.ops_tx.clone(), Op::Rename { path, new_name });
        }
        self.redraw();
    }

    // ---------- barra de dirección editable ----------

    fn begin_edit(&mut self, pane: usize) {
        if self.edit.is_some() {
            return;
        }
        self.set_active(pane);
        let text = match &self.panes[pane].tab().location {
            Location::Computer => "Este equipo".to_string(),
            Location::Path(p) => p.to_string_lossy().into_owned(),
            Location::Shell { parsing, .. } => parsing.clone(),
        };
        let r = self.panes[pane].address;
        let k = self.dpi as f32 / 96.0;
        let (x, w) = ((r.left + 10.0) * k, (r.right - r.left - 20.0) * k);
        let h = 20.0 * k;
        let y = (r.top + r.bottom) / 2.0 * k - h / 2.0;
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                x as i32,
                y as i32,
                w as i32,
                h as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_EDIT_DONE as usize);
            // Autocompletado de rutas del propio Shell.
            let _ = SHAutoComplete(edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetFocus(Some(edit));
            self.edit = Some((edit, pane));
        }
        self.redraw();
    }

    fn end_edit(&mut self, commit: bool) {
        let Some((edit, pane)) = self.edit.take() else { return };
        let mut buf = vec![0u16; 32 * 1024];
        let text = unsafe {
            let len = GetWindowTextW(edit, &mut buf);
            let _ = DestroyWindow(edit);
            let _ = SetFocus(Some(self.hwnd));
            String::from_utf16_lossy(&buf[..len.max(0) as usize])
        };
        if commit {
            self.go_to_text(pane, &text);
        }
        self.redraw();
    }

    /// Abre el salto rápido o la paleta. Si ya está abierta, cambia de modo conservando lo escrito.
    fn open_palette(&mut self, mode: PaletteMode) {
        if let Some((pal, edit)) = &mut self.palette {
            pal.mode = mode;
            let edit = *edit;
            self.refresh_palette();
            unsafe {
                let _ = SetFocus(Some(edit));
            }
            return;
        }
        self.end_edit(false);
        self.end_filter(true);
        self.picker_open = false;
        self.term_menu = None;
        let (w, _) = self.client_dips();
        let items = palette::search(mode, "", &Sources { history: &self.history, favorites: &self.favorites, groups: &self.groups, keymap: &self.config.keymap }, expand_env);
        let pal = Palette::new(mode, items);
        let r = pal.input_rect(w);
        let k = self.dpi as f32 / 96.0;
        let h = 20.0 * k;
        let (x, width) = (r.left * k, (r.right - r.left) * k);
        let y = (r.top + r.bottom) / 2.0 * k - h / 2.0;
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                x as i32,
                y as i32,
                width as i32,
                h as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(palette_proc), 2, 0);
            let _ = SetFocus(Some(edit));
            self.palette = Some((pal, edit));
        }
        self.redraw();
    }

    /// Vuelve a buscar con lo escrito (cambió el texto o el modo).
    fn refresh_palette(&mut self) {
        let Some((pal, edit)) = &mut self.palette else { return };
        let query = window_text(*edit);
        pal.set_items(palette::search(pal.mode, &query, &Sources { history: &self.history, favorites: &self.favorites, groups: &self.groups, keymap: &self.config.keymap }, expand_env));
        self.redraw();
    }

    fn on_palette_key(&mut self, vk: VIRTUAL_KEY, ctrl: bool) {
        let Some((pal, edit)) = &mut self.palette else { return };
        match vk {
            VK_UP => pal.move_by(-1),
            VK_DOWN => pal.move_by(1),
            VK_PRIOR => pal.move_by(-(palette::MAX_ROWS as i32)),
            VK_NEXT => pal.move_by(palette::MAX_ROWS as i32),
            VK_RETURN => {
                let i = pal.selected;
                self.activate_palette(i, ctrl);
                return;
            }
            // Ctrl+P / Ctrl+Shift+P dentro de la paleta: cambiar de modo.
            VIRTUAL_KEY(0x50) => {
                let mode = if key_down(VK_SHIFT) { PaletteMode::All } else { PaletteMode::Folders };
                self.open_palette(mode);
                return;
            }
            VK_ESCAPE => self.close_palette(),
            // Perdió el foco: se cierra, salvo que el foco haya vuelto (clic dentro del panel).
            _ => {
                let edit = *edit;
                if unsafe { GetFocus() } != edit {
                    self.close_palette();
                }
            }
        }
        self.redraw();
    }

    fn close_palette(&mut self) {
        let Some((_, edit)) = self.palette.take() else { return };
        unsafe {
            let _ = DestroyWindow(edit);
            if GetFocus().is_invalid() {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        self.redraw();
    }

    /// Abre un resultado: ejecuta la acción o va a la carpeta (con Ctrl, en una pestaña nueva).
    fn activate_palette(&mut self, index: usize, ctrl: bool) {
        let Some(item) = self.palette.as_ref().and_then(|(p, _)| p.items.get(index).cloned()) else { return };
        self.close_palette();
        match item.target {
            Target::Command(c) => self.run_command(c),
            Target::Group(i) => self.apply_group(i),
            Target::SaveGroup(name) => self.save_group(&name),
            Target::Folder(loc) => {
                if let Location::Path(p) = &loc {
                    if !p.is_dir() {
                        // Ya no existe: sale del historial.
                        self.history.forget(p);
                        self.notice = Some(format!("No existe: {}", p.display()));
                        self.redraw();
                        return;
                    }
                }
                let active = self.active;
                if ctrl {
                    self.new_tab(active, loc, true);
                } else {
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    self.tab_mut().navigate(hwnd, &tx, loc);
                }
            }
        }
        self.redraw();
    }

    /// Abre el filtro rápido del panel (o le devuelve el foco si ya está abierto).
    fn begin_filter(&mut self, pane: usize) {
        if let Some((edit, p)) = self.filter_edit {
            if p == pane {
                unsafe {
                    let _ = SetFocus(Some(edit));
                }
                return;
            }
            self.end_filter(true);
        }
        self.end_edit(false);
        self.set_active(pane);
        let r = self.panes[pane].filter_text_rect();
        let k = self.dpi as f32 / 96.0;
        let h = 20.0 * k;
        let (x, w) = (r.left * k, (r.right - r.left) * k);
        let y = (r.top + r.bottom) / 2.0 * k - h / 2.0;
        let wide: Vec<u16> = self.panes[pane].tab().list.filter().encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                x as i32,
                y as i32,
                w as i32,
                h as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_FILTER_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetFocus(Some(edit));
            self.filter_edit = Some((edit, pane));
        }
        self.redraw();
    }

    /// Cierra el control del filtro. `keep`: Enter o perder el foco lo dejan aplicado; Escape lo quita.
    fn end_filter(&mut self, keep: bool) {
        let Some((edit, pane)) = self.filter_edit.take() else { return };
        unsafe {
            let _ = DestroyWindow(edit);
            if GetFocus().is_invalid() {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        if !keep {
            self.panes[pane].tab_mut().list.set_filter("");
        }
        self.redraw();
    }

    /// Quita el filtro del panel (cruz de la fila o Escape sobre la lista).
    fn clear_filter(&mut self, pane: usize) {
        if self.filter_edit.is_some_and(|(_, p)| p == pane) {
            self.end_filter(false);
        } else {
            self.panes[pane].tab_mut().list.set_filter("");
            self.redraw();
        }
    }

    /// Navega a lo que se escribió en la barra de dirección.
    fn go_to_text(&mut self, pane: usize, text: &str) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab_mut();
        let text = text.trim().trim_matches('"');
        if text.is_empty() || text.eq_ignore_ascii_case("este equipo") {
            tab.navigate(hwnd, &tx, Location::Computer);
            return;
        }
        // "shell:RecycleBinFolder", "::{GUID}": lugares del Shell.
        if text.starts_with("shell:") || text.starts_with("::") {
            match fs::shell_location(text) {
                Some(loc) => tab.navigate(hwnd, &tx, loc),
                None => unsafe {
                    let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONWARNING);
                },
            }
            return;
        }
        let mut path = PathBuf::from(expand_env(text));
        if path.is_relative() {
            if let Location::Path(base) = &tab.location {
                path = base.join(path);
            }
        }
        // "C:" solo es "la carpeta actual de C:"; se interpreta como la raíz.
        if path.as_os_str().len() == 2 && path.to_string_lossy().ends_with(':') {
            path.push("\\");
        }
        // Un recurso de red que pide usuario y contraseña (o que no se verifica): se muestra el
        // diálogo de Windows. Si igual no se puede ver, se navega y la barra de estado dice por qué.
        if fs::is_unc(&path) && !path.exists() {
            fs::connect_share(hwnd, &path);
            if !path.exists() {
                tab.navigate(hwnd, &tx, Location::Path(path));
                return;
            }
        }
        if path.is_dir() {
            // Con las mayúsculas del disco, no como se escribió (se ve en pestañas e historial).
            tab.navigate(hwnd, &tx, Location::Path(fs::true_case(&path)));
        } else if path.is_file() {
            // Un archivo: se abre su carpeta con el archivo enfocado.
            if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
                tab.navigate(hwnd, &tx, Location::Path(parent.to_path_buf()));
                tab.focus_after_load(name.to_os_string());
            }
        } else {
            unsafe {
                let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONWARNING);
            }
        }
    }

    fn update_edit_resources(&mut self) {
        unsafe {
            if !self.edit_font.is_invalid() {
                let _ = DeleteObject(self.edit_font.into());
            }
            if !self.edit_brush.is_invalid() {
                let _ = DeleteObject(self.edit_brush.into());
            }
            self.edit_font = CreateFontW(
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
            self.edit_brush = CreateSolidBrush(colorref(self.theme.surface_alt));
        }
    }

    // ---------- dibujo ----------

    fn all_listed(&self) -> bool {
        self.panes[..self.layout.panes()].iter().all(|p| p.tab().is_loaded() && !p.tab().loading)
    }

    fn paint(&mut self) {
        let title = format!("{} — xdir", self.panes[self.active].tab().location.title());
        if title != self.title {
            let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let _ = SetWindowTextW(self.hwnd, PCWSTR(wide.as_ptr()));
            }
            self.title = title;
        }

        if self.first_frame_ms.is_none() {
            self.marks.push(("WM_PAINT", ms_since(self.started)));
            if self.renderer.ensure_target(self.hwnd, self.dpi).is_ok() {
                self.marks.push(("render target", ms_since(self.started)));
            }
        }
        let listed = self.all_listed();
        if listed && !self.list_frame_done {
            self.marks.push(("inicio frame con la carpeta", ms_since(self.started)));
        }

        let mut status = self.panes[self.active].tab().status();
        if let Some(n) = &self.notice {
            status = format!("{n}   ·   {status}");
        }
        let tree_w = self.tree_width();
        #[cfg(feature = "arbol")]
        let current_folder = self.panes[self.active].tab().location.target();
        // Animaciones: rectángulos de este frame (transición de layout) y scroll suave de las
        // listas a la vista. Mientras algo siga en curso se pide otro frame más abajo.
        let (cw, ch) = self.client_dips();
        let target = ui::visible_rects(self.layout, self.maximized, ui::panes_area(cw, ch, tree_w), self.splits);
        let (rects, mut animating) = self.animated_rects(target);
        let now = Instant::now();
        let dt = self.last_frame.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        for i in self.shown_panes() {
            animating |= self.panes[i].tab_mut().list.tick(dt);
        }
        self.last_frame = animating.then_some(now);
        #[cfg(feature = "arbol")]
        let tree = &mut self.tree;
        let (panes, layout, splits, active, maximized) = (&mut self.panes, self.layout, self.splits, self.active, self.maximized);
        let (hover, drag, edit_pane) = (self.hover, self.drag, self.edit.map(|(_, p)| p));
        let (favorites, show_favorites, fav_drop) = (&self.favorites, self.show_favorites, self.fav_drop);
        let (picker_open, accent) = (self.picker_open, self.accent);
        let (file_colors, tree_on) = (self.file_colors, tree_w > 0.0);
        let term_menu = self.term_menu;
        let tab_drop = self.tab_drop;
        let filter_pane = self.filter_edit.map(|(_, p)| p);
        let palette = self.palette.as_ref().map(|(p, _)| p);
        let result = self.renderer.draw(self.hwnd, self.dpi, &self.theme, if self.file_colors { &self.config.colors } else { &[] }, |p, w, h| {
            let area = ui::panes_area(w, h, tree_w);
            #[cfg(feature = "arbol")]
            if tree.visible {
                let r = render::rect(ui::GAP, area.top, ui::GAP + tree::WIDTH, area.bottom);
                tree.draw(p, r, current_folder.as_deref());
            }
            // Botón de maximizar solo si hay más de un panel; encendido en el maximizado.
            let max_button = |i: usize| (layout.panes() > 1).then_some(maximized == Some(i));
            for &(i, r) in &rects {
                let ph = match hover {
                    Hit::Pane(j, ph) if j == i => ph,
                    _ => PaneHit::None,
                };
                let pressed = match drag {
                    Drag::Button(j, b) if j == i => PaneHit::Button(b),
                    _ => PaneHit::None,
                };
                let favs = show_favorites.then(|| (favorites, fav_drop.filter(|(j, _)| *j == i).map(|(_, d)| d)));
                // La fila del filtro se ve mientras se edita o si hay uno aplicado.
                let list = &panes[i].tab().list;
                let editing_filter = filter_pane == Some(i);
                let filter_text = list.filter().to_string();
                let filter = (editing_filter || !filter_text.is_empty()).then(|| FilterView {
                    text: &filter_text,
                    shown: list.count(),
                    total: list.unfiltered_count(),
                    editing: editing_filter,
                });
                panes[i].draw(p, r, i == active, ph, pressed, edit_pane == Some(i), favs, term_menu == Some(i), max_button(i), filter);
                if let Some((_, at)) = tab_drop.filter(|(j, _)| *j == i) {
                    panes[i].draw_tab_drop(p, at);
                }
            }
            let hot = match (drag, hover) {
                (Drag::Splitter(s), _) | (_, Hit::Splitter(s)) => Some(s),
                _ => None,
            };
            if let Some(s) = hot.filter(|_| maximized.is_none()) {
                for (which, r) in ui::splitter_rects(layout, area, splits) {
                    if which == s {
                        ui::draw_splitter(p, r);
                    }
                }
            }
            let layout_hover = match hover {
                Hit::Layout(l) => Some(l),
                _ => None,
            };
            ui::draw_status(p, w, h, &status, layout, layout_hover, picker_open || hover == Hit::Gear, hover == Hit::Groups);
            if let Some(pane) = term_menu {
                let term_hover = match hover {
                    Hit::TermMenu(c) => c,
                    _ => None,
                };
                ui::draw_terminal_menu(p, panes[pane].terminal_rect(), w, term_hover);
            }
            if picker_open {
                let picker_hover = match hover {
                    Hit::Picker(k) => Some(k),
                    _ => None,
                };
                ui::draw_picker(p, w, h, accent, picker_hover, file_colors, tree_on);
            }
            if let Some(pal) = palette {
                pal.draw(p, w);
            }
        });
        if result.is_err() {
            return;
        }
        if animating {
            self.redraw();
        }
        uia::notify_focus(self);
        // Superposiciones de las filas recién dibujadas.
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for i in self.shown_panes() {
            self.panes[i].tab_mut().request_overlays(hwnd, &tx);
        }

        if self.first_frame_ms.is_none() {
            let ms = ms_since(self.started);
            self.first_frame_ms = Some(ms);
            self.marks.push(("primer frame", ms));
        }
        if listed && !self.list_frame_done {
            self.list_frame_done = true;
            self.marks.push(("primer frame con la carpeta", ms_since(self.started)));
            if let Some(path) = &self.probe {
                let total = self.marks.last().unwrap().1;
                let mut out = format!(
                    "first_frame_ms={:.2}\nlisted_frame_ms={total:.2}\nitems={}\nui_load_ms={:.2}\ndpi={}\n",
                    self.first_frame_ms.unwrap(),
                    self.panes[self.active].tab().list.count(),
                    self.ui_load_ms,
                    self.dpi
                );
                for (name, t) in &self.marks {
                    out += &format!("  {t:7.2}  {name}\n");
                }
                let _ = std::fs::write(path, out);
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
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
                let vk = VIRTUAL_KEY(wparam.0 as u16);
                if matches!(vk, VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN | VK_ESCAPE) || (ctrl && vk == VIRTUAL_KEY(0x50)) {
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

/// Efecto al soltar como en el Explorador: Ctrl copia, Shift mueve; sin teclas, mueve dentro
/// de la misma unidad y copia entre unidades. Soltar en la misma carpeta (o dentro de sí
/// misma) no hace nada.
fn drop_effect(paths: &[PathBuf], dest: &std::path::Path, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
    if paths.iter().any(|p| p.parent() == Some(dest) || dest.starts_with(p)) {
        return DROPEFFECT_NONE;
    }
    let same_volume = paths.first().and_then(|p| shell::volume_of(p)) == shell::volume_of(dest);
    let want = if keys.0 & MK_CONTROL.0 != 0 {
        DROPEFFECT_COPY
    } else if keys.0 & MK_SHIFT.0 != 0 || same_volume {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    if allowed.0 & want.0 != 0 {
        want
    } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

/// Abre un archivo de configuración con el programa asociado; si `.toml` no tiene ninguno, con el
/// Bloc de notas.
fn open_in_editor(hwnd: HWND, path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
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

/// Texto de un control de edición.
fn window_text(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 32 * 1024];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
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

/// Líneas por "clic" de la rueda según la configuración de Windows.
fn wheel_lines() -> f32 {
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

fn expand_env(s: &str) -> String {
    let src: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 32 * 1024];
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..n - 1])
}

fn colorref(c: D2D1_COLOR_F) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | ch(c.g) << 8 | ch(c.b) << 16)
}

fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn scale(dips: i32, dpi: u32) -> i32 {
    dips * dpi as i32 / 96
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: DROPEFFECT = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0);

    fn effect(sources: &[&str], dest: &str, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
        let paths: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        drop_effect(&paths, std::path::Path::new(dest), keys, allowed)
    }

    #[test]
    fn drop_rules() {
        let none = MODIFIERKEYS_FLAGS(0);
        // Misma unidad: mueve; otra unidad: copia.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", none, ALL), DROPEFFECT_MOVE);
        assert_eq!(effect(&[r"D:\a\x.txt"], r"C:\b", none, ALL), DROPEFFECT_COPY);
        // Ctrl copia, Shift mueve.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", MK_CONTROL, ALL), DROPEFFECT_COPY);
        assert_eq!(effect(&[r"D:\a\x.txt"], r"C:\b", MK_SHIFT, ALL), DROPEFFECT_MOVE);
        // A la misma carpeta, o una carpeta dentro de sí misma: nada.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\a", none, ALL), DROPEFFECT_NONE);
        assert_eq!(effect(&[r"C:\a"], r"C:\a\sub", none, ALL), DROPEFFECT_NONE);
        // Si el origen solo permite copiar, se copia.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", none, DROPEFFECT_COPY), DROPEFFECT_COPY);
    }
}


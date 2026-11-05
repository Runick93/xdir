//! Ventana principal: el estado de la aplicación (`App`) y su arranque.
//! Los métodos de `App` se reparten por área en los submódulos.

mod actions;
mod appearance;
mod edit;
mod events;
mod favs;
mod fileops;
mod saved_groups;
mod input;
mod paint;
mod quick_jump;
mod panes;

// Lo propio de cada sistema: arranque, ventana, controles nativos y menús.
#[cfg(windows)]
mod messages;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win::*;
#[cfg(windows)]
pub use win::run;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::*;
#[cfg(target_os = "macos")]
pub use mac::run;

#[cfg(test)]
mod tests;

use events::CursorKind;

use crate::{about, anim, config, fs, icons, jump, list, ops, palette, pane, render, session, shell, theme, typed, ui};
#[cfg(windows)]
use crate::drop;
#[cfg(feature = "arbol")]
use crate::tree;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use crate::platform::{Color, Handle, Key, WM_APP, key_down};
use crate::platform::keys::{VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_PRIOR, VK_RETURN, VK_SHIFT, VK_UP};

use crate::commands::Command;
use crate::config::Config;
use crate::console::Console;
use crate::groups::{Group, Groups};
use crate::jump::History;
use crate::palette::{Palette, PaletteMode, Sources, Target};
use crate::drop::{DragEvent, DragKind, Effect};
#[cfg(windows)]
use crate::drop::WM_APP_DRAG;
use crate::favorites::{Fav, FavPath, Favorites};
use crate::fs::{Location, WM_APP_CHANGED, WM_APP_LOAD};
use crate::ops::{Op, OpResult, Undo, WM_APP_OP_DONE};
use crate::list::{Action, ColumnSpec};
use crate::pane::{Button, FavDrop, Pane, PaneHit, SearchView};
use crate::render::{Renderer, contains};
use crate::session::{PaneState, Session};
use crate::tab::Tab;
use crate::theme::{Accent, Theme};
use crate::ui::{Layout, Splits, Splitter};

/// Error de las llamadas al sistema (crear el renderizador, la ventana...).
#[cfg(windows)]
pub type SysResult<T> = windows::core::Result<T>;
#[cfg(target_os = "macos")]
pub type SysResult<T> = Result<T, String>;

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
/// Tecla de la paleta: `wparam` = tecla virtual (0: perdió el foco), `lparam` = 1 si con Ctrl.
const WM_APP_PALETTE_KEY: u32 = WM_APP + 14;
/// Flecha / Re Pág / Av Pág en la barra de dirección: recorren el desplegable de rutas.
const WM_APP_SUGGEST_KEY: u32 = WM_APP + 15;
/// Cambió el texto de una caja de edición. `lparam` = `TextBox::id`.
#[cfg_attr(windows, allow(dead_code))]
const WM_APP_TEXT_CHANGED: u32 = WM_APP + 16;
/// Pasó la pausa después de escribir en el buscador. `lparam` = turno de la búsqueda pendiente.
const WM_APP_SEARCH: u32 = WM_APP + 17;
/// Cuánto se espera después de la última tecla para empezar a buscar.
const SEARCH_DELAY_MS: u64 = 300;

/// Qué muestra un menú desplegable de favoritos.
enum FavMenuSource {
    /// El contenido de una carpeta de favoritos.
    Group(FavPath),
    /// Los favoritos de la raíz que no entraron en la barra, desde este índice.
    Overflow(usize),
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

/// Operaciones que se pueden deshacer (Ctrl+Z).
const UNDO_MAX: usize = 50;

/// Renombrado en línea en curso.
struct Rename {
    edit: TextBox,
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
    /// Botón que abre xtree, el mapa de uso de disco.
    Xtree,
    /// Barra de pestañas de grupos: una pestaña, o el botón para crear uno.
    GroupBar(ui::GroupHit),
    Picker(ui::PickerHit),
    /// Diálogo "Acerca de" abierto: un botón, o el diálogo sin nada debajo.
    About(about::AboutHit),
    /// Menú de consola abierto: una consola, o el menú sin nada debajo.
    TermMenu(Option<Console>),
    /// Paleta abierta: un resultado, o el panel sin nada debajo.
    Palette(Option<usize>),
    /// Desplegable de la barra de dirección: una ruta, o el panel sin nada debajo.
    Suggest(Option<usize>),
}

/// Lo que se está arrastrando con el botón izquierdo.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    None,
    List(usize),
    Splitter(Splitter),
    /// `grab`: cuánto de la pestaña queda a la izquierda del mouse al agarrarla.
    Tab { pane: usize, index: usize, start_x: f32, start_y: f32, moved: bool, grab: f32 },
    /// Borde derecho del árbol de carpetas: cambiar su ancho.
    #[cfg(feature = "arbol")]
    TreeEdge,
    /// Pulgar de una barra del árbol (y dónde se lo agarró).
    #[cfg(feature = "arbol")]
    TreeBar(tree::Bar, f32),
    Button(usize, Button),
    /// Un favorito de la barra: clic al soltar sin mover, si no reordenar.
    Fav { pane: usize, index: usize, start_x: f32, moved: bool },
}

/// Algo elegido en el menú del fondo ("Nuevo > Carpeta"...): si en los próximos segundos aparece un
/// elemento que no estaba, se abre su nombre para escribirlo, como hace el Explorador.
struct NewWatch {
    pane: usize,
    /// Nombres (en minúsculas) que había antes.
    before: std::collections::HashSet<String>,
    since: Instant,
}

/// Vista previa flotante de la pestaña que se arrastra: acompaña al mouse y se acopla a las tiras.
struct Ghost {
    title: String,
    w: f32,
    /// Posición dibujada (arriba a la izquierda) y hacia dónde va.
    x: f32,
    y: f32,
    tx: f32,
    ty: f32,
    /// 1: suelta del todo; 0: acoplada a una tira.
    detach: f32,
    docked: bool,
}

impl Ghost {
    fn step(&mut self, dt: f32) -> bool {
        let k = 1.0 - (-dt * 38.0).exp();
        let kd = 1.0 - (-dt * 18.0).exp();
        self.x += (self.tx - self.x) * k;
        self.y += (self.ty - self.y) * k;
        let goal = if self.docked { 0.0 } else { 1.0 };
        self.detach += (goal - self.detach) * kd;
        (self.tx - self.x).abs() > 0.3 || (self.ty - self.y).abs() > 0.3 || (goal - self.detach).abs() > 0.01
    }
}

/// Transición de layout: desde dónde parte cada panel y cuándo empezó.
struct LayoutAnim {
    from: [Option<render::Rect>; PANES],
    start: Instant,
}

struct App {
    hwnd: Handle,
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
    edit: Option<(TextBox, usize)>,
    /// Desplegable de rutas de la barra de dirección (vive mientras `edit`).
    suggest: Option<typed::Suggest>,
    /// Estado propio del sistema (fuentes de los controles, destino de arrastre...).
    #[cfg_attr(not(windows), allow(dead_code))]
    sys: Sys,
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
    new_watch: Option<NewWatch>,
    /// Mensaje para la barra de estado (errores de operaciones).
    notice: Option<String>,
    /// Fondo Mica activo (ventana con marco extendido y composición translúcida).
    transparent: bool,
    favorites: Favorites,
    /// Carpetas visitadas, para el salto rápido.
    history: History,
    /// Atajos y colores de `config.toml`.
    config: Config,
    groups: Groups,
    /// Grupo de paneles cargado o guardado en esta ejecución: va en el título de la ventana.
    group_name: Option<String>,
    /// Grupo al que el nombre que se escriba en la paleta le cambia el nombre (en vez de guardar uno).
    group_rename: Option<String>,
    /// Vigila los cambios del Acceso rápido de Windows para releerlo en el árbol.
    quick_watch: Option<fs::Watcher>,
    /// El último clic derecho fue sobre el árbol: la carpeta (si tiene ruta de disco).
    tree_ctx: Option<Option<PathBuf>>,
    /// Cuándo se descartó el menú de grupos sin elegir nada (para que el clic que lo cerró sobre su botón no lo reabra).
    groups_menu_closed: Option<Instant>,
    /// Salto rápido o paleta de comandos abierta, con su control de edición.
    palette: Option<(Palette, TextBox)>,
    /// Árbol de carpetas lateral (experimental).
    #[cfg(feature = "arbol")]
    tree: tree::Tree,
    show_favorites: bool,
    /// Barra de pestañas de grupos habilitada (se ve si además hay grupos guardados).
    show_group_bar: bool,
    /// Colores por tipo de archivo encendidos (switch de la rueda dentada).
    file_colors: bool,
    /// Dónde caería lo que se arrastra sobre la barra de favoritos (panel, posición).
    fav_drop: Option<(usize, FavDrop)>,
    /// Arrastrando una pestaña sobre otro panel: (panel destino, posición en su tira).
    tab_drop: Option<(usize, usize)>,
    /// Pestaña flotante mientras se arrastra una.
    ghost: Option<Ghost>,
    /// Renombrado de un favorito en curso.
    fav_edit: Option<(TextBox, FavPath)>,
    /// Control de edición del filtro rápido abierto y su panel.
    filter_edit: Option<(TextBox, usize)>,
    /// Búsqueda esperando que se deje de escribir: (panel, texto, turno).
    pending_search: Option<(usize, String, u64)>,
    /// Turno de la última tecla en el buscador (las pausas viejas se descartan).
    search_token: u64,
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
    /// Diálogo "Acerca de" abierto.
    about_open: bool,
    /// Menú de consola abierto, en este panel.
    term_menu: Option<usize>,
    /// Consola del menú de terminales sobre la que se hizo clic derecho.
    term_ctx: Option<(usize, Console)>,
    /// Tamaño de la ventana en DIPs fijado por las pruebas (sin ventana real no hay `GetClientRect`).
    #[cfg(test)]
    client: Option<(f32, f32)>,
}

/// Argumentos de la línea de comandos: sesión a restaurar, `--probe <archivo>` (mide el
/// arranque y cierra) y una carpeta para abrir.
pub(crate) fn startup_args() -> (Option<Session>, Option<String>, Option<PathBuf>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let probe = args.iter().position(|a| a == "--probe").and_then(|i| args.get(i + 1).cloned());
    let start_path = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--probe"))
        .map(|(_, a)| PathBuf::from(a))
        .filter(|p| p.is_dir());
    // `--probe` mide el arranque limpio: sin sesión, con la carpeta pedida en un solo panel.
    let session = if probe.is_some() { Some(Session { layout: 0, ..Default::default() }) } else { session::load() };
    (session, probe, start_path)
}

/// Todo lo que `App::new` toma del exterior (disco, argumentos, hora de inicio): así se puede armar
/// una `App` en pruebas sin leer ni escribir nada del usuario.
pub(crate) struct Init {
    pub session: Session,
    pub history: History,
    pub config: Config,
    pub favorites: Favorites,
    pub groups: Groups,
    pub probe: Option<String>,
    pub start_path: Option<PathBuf>,
    pub started: Instant,
    pub marks: Vec<(&'static str, f64)>,
}

impl Init {
    /// Lee del disco lo que hace falta para arrancar (historial, configuración, favoritos y
    /// grupos) y aplica la preferencia de animaciones.
    pub(crate) fn load(session: Option<Session>, probe: Option<String>, start_path: Option<PathBuf>, started: Instant, marks: Vec<(&'static str, f64)>) -> Init {
        let session = session.unwrap_or_else(default_session);
        // `--probe` mide un arranque limpio: sin historial ni configuración del usuario.
        let history = if probe.is_none() { History::load() } else { History::default() };
        let config = if probe.is_none() { Config::load() } else { Config::default() };
        anim::set_enabled(config.animations.unwrap_or_else(anim::system_allows));
        anim::set_off_by_config(config.animations == Some(false));
        Init { session, history, config, favorites: Favorites::load(), groups: Groups::load(), probe, start_path, started, marks }
    }

    /// Datos vacíos y sesión por defecto: nada del disco.
    #[cfg(test)]
    pub fn blank() -> Init {
        Init {
            session: default_session(),
            history: History::default(),
            config: Config::default(),
            favorites: Favorites::default(),
            groups: Groups::default(),
            probe: None,
            start_path: None,
            started: Instant::now(),
            marks: Vec::new(),
        }
    }
}

impl App {
    /// Arma la aplicación (sin ventana: el `HWND` llega en `WM_NCCREATE`) y abre sus paneles.
    pub(crate) fn new(init: Init) -> SysResult<App> {
        let Init { session, history, config, favorites, groups, probe, start_path, started, mut marks } = init;
        let theme = Theme::from_system(session.accent, session.dark_mode);
        let renderer = Renderer::new()?;
        marks.push(("fabricas d2d/dwrite", ms_since(started)));
        let (tx, rx) = channel();
        let (ops_tx, ops_rx) = channel();
        let config_errors = config.errors.first().map(|e| format!("config.toml, {e}"));
        let mut app = App {
            hwnd: Handle::default(),
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
            suggest: None,
            sys: Sys::new(),
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
            new_watch: None,
            notice: config_errors,
            transparent: false,
            favorites,
            history,
            config,
            groups,
            group_name: session.group.clone(),
            group_rename: None,
            quick_watch: None,
            tree_ctx: None,
            groups_menu_closed: None,
            palette: None,
            #[cfg(feature = "arbol")]
            tree: tree::Tree::new(),
            show_favorites: session.show_favorites,
            show_group_bar: session.group_bar,
            file_colors: session.file_colors,
            fav_drop: None,
            tab_drop: None,
            ghost: None,
            fav_edit: None,
            filter_edit: None,
            pending_search: None,
            search_token: 0,
            fav_ctx: None,
            header_ctx: None,
            columns: session.columns.clone(),
            accent: session.accent,
            dark_mode: session.dark_mode,
            picker_open: false,
            about_open: false,
            term_menu: None,
            term_ctx: None,
            #[cfg(test)]
            client: Some((1280.0, 800.0)),
        };
        app.restore_panes(&session, start_path);
        app.ensure_group();
        #[cfg(feature = "arbol")]
        {
            app.tree.visible = session.show_tree;
            app.tree.set_pinned(session.tree_pinned, false);
            app.tree.set_quick_open(session.quick_open);
        }
        Ok(app)
    }
}

pub(super) fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Primera vez: 4 paneles en "Este equipo".
fn default_session() -> Session {
    let panes = (0..PANES).map(|_| PaneState { tabs: vec![Location::Computer], active: 0 }).collect();
    Session { panes, ..Default::default() }
}

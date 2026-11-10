//! Ventana principal: el estado de la aplicación (`App`) y su arranque.
//! Los métodos de `App` se reparten por área en los submódulos. Cada sistema (en su crate) arma la
//! ventana y le habla a la `App` solo por su API pública.

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
mod tooltip;

#[cfg(test)]
mod tests;

pub use events::CursorKind;
pub use fileops::quoted_paths;
use tooltip::{Tip, WM_APP_TOOLTIP};
#[cfg(test)]
use tooltip::{tooltip_for, tooltip_rect};

use crate::{about, anim, config, jump, list, palette, pane, session, theme, typed, ui};
use crate::xtree::view::{self as xtree_view, Xtree, XtreeBadge};
#[cfg(feature = "arbol")]
use crate::tree;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use crate::geom::{self, contains};
use crate::keys::{Key, keys::{VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_SHIFT, VK_UP}};
use crate::sys::{self, Handle};
use crate::{WM_APP, fs, shell};

use crate::commands::Command;
use crate::config::Config;
use crate::console::Console;
use crate::groups::{Group, Groups};
use crate::jump::History;
use crate::palette::{Palette, PaletteMode, Sources, Target};
use crate::drop::{DragEvent, DragKind, Effect};
use crate::favorites::{Fav, FavPath, Favorites};
use crate::fs::{Location, WM_APP_CHANGED, WM_APP_LOAD};
use crate::ops::{Op, OpResult, Undo, WM_APP_OP_DONE};
use crate::folder_views::FolderViews;
use crate::list::Action;
use crate::pane::{Button, FavDrop, Pane, PaneHit, SearchView};
use crate::session::{PaneState, Session};
use crate::tab::Tab;
use crate::theme::{Accent, Theme};
use crate::ui::{Layout, Splits, Splitter};
use crate::a11y::{self, A11yAction, A11yView, ItemInfo};
use crate::host::{
    MenuPos, TextBox, TextKind, WM_APP_EDIT_DONE, WM_APP_FAVEDIT_DONE, WM_APP_FILTER_DONE, WM_APP_PALETTE_KEY, WM_APP_RENAME_DONE,
    WM_APP_SUGGEST_KEY, WM_APP_TEXT_CHANGED, WindowHost,
};

/// ¿La tecla está apretada ahora? (Ctrl, Shift, Alt…; en macOS Ctrl de los atajos es ⌘ y Alt es ⌥.)
fn key_down(k: Key) -> bool {
    sys::env().key_down(k)
}

/// Tamaño inicial en DIPs; se escala según el DPI del monitor.
pub const INITIAL_SIZE: (i32, i32) = (1280, 800);
/// Siempre hay 4 paneles; el layout decide cuántos se ven. Los ocultos conservan sus pestañas.
const PANES: usize = 4;
/// Pasó la pausa después de escribir en el buscador. `lparam` = turno de la búsqueda pendiente.
const WM_APP_SEARCH: u32 = WM_APP + 17;
/// Cuánto se espera después de la última tecla para empezar a buscar.
const SEARCH_DELAY_MS: u64 = 300;
/// Pasó la pausa después del último cambio de vista de una carpeta: se guarda `carpetas.toml`.
const WM_APP_VIEWS_SAVE: u32 = WM_APP + 18;
/// Cuánto se espera después del último cambio de vista para guardar `carpetas.toml`.
const VIEWS_SAVE_DELAY_MS: u64 = 1000;

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
    /// Botón que abre y cierra xtree, el mapa de uso de disco.
    Xtree,
    /// La vista de xtree abierta, sobre el área de los paneles.
    XtreeView,
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
    /// Desplegable de filtros de la lista de un panel: una opción, o el desplegable sin nada debajo.
    FilterMenu(usize, Option<list::ListHit>),
    /// La chincheta de arriba del árbol (fijarlo o soltarlo). El resto del árbol es `None`: lo
    /// maneja el árbol.
    // Solo existe con el árbol (feature `arbol`).
    #[cfg_attr(not(feature = "arbol"), allow(dead_code))]
    TreePin,
    /// La entrada «xtree» del árbol (abre o cierra la vista; su cartel dice cómo va el escaneo).
    #[cfg_attr(not(feature = "arbol"), allow(dead_code))]
    TreeXtree,
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
    from: [Option<geom::Rect>; PANES],
    start: Instant,
}

/// El estado de la ventana principal. La plataforma la arma con `new`, le fija la ventana con
/// `attach` y le pasa los eventos ya traducidos (`on_*`).
pub struct App {
    /// La ventana: la fija la plataforma con `attach` (es el dueño que usan los servicios).
    hwnd: Handle,
    /// La ventana: repintar, tamaño, título, foco, diálogos, tema y arrastre (falsa en las pruebas).
    host: Rc<dyn WindowHost>,
    /// DPI de la ventana (para `--probe` y las coordenadas del mouse): lo fija la plataforma con
    /// `attach` / `set_dpi`.
    dpi: u32,
    theme: Theme,
    panes: Vec<Pane>,
    layout: Layout,
    splits: Splits,
    /// Panel maximizado temporalmente (ocupa toda el área); no se guarda en la sesión.
    maximized: Option<usize>,
    /// Rectángulos con que se dibujó cada panel el último frame, y la transición de layout en curso.
    displayed: [Option<geom::Rect>; PANES],
    layout_anim: Option<LayoutAnim>,
    /// Cuándo fue el último frame de una animación (None: no hay ninguna en curso).
    last_frame: Option<Instant>,
    /// Último elemento con foco que se avisó a los lectores de pantalla: (panel, posición).
    a11y_focus: Option<(usize, Option<usize>)>,
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
    hover: Hit,
    /// Cartel de ayuda del botón bajo el mouse (esperando, a la vista o descartado).
    tip: Option<Tip>,
    /// Turno del último cartel pedido: los avisos de espera viejos se descartan.
    tip_token: u64,
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
    /// El aviso «Vista: …» que está en `notice`: el panel, la carpeta (`Tab::view_key`) y la vista
    /// que anunció. Se borra cuando la lista activa deja de ser esa (`sync_folder_views`).
    view_notice: Option<(usize, String, list::View)>,
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
    // Solo lo usa el árbol (feature `arbol`).
    #[cfg_attr(not(feature = "arbol"), allow(dead_code))]
    quick_watch: Option<Box<dyn sys::Watch>>,
    /// El último clic derecho fue sobre el árbol: la carpeta (si tiene ruta de disco).
    // Solo lo usa el árbol (feature `arbol`).
    #[cfg_attr(not(feature = "arbol"), allow(dead_code))]
    tree_ctx: Option<Option<PathBuf>>,
    /// Carpeta del panel activo que el árbol ya siguió (`follow_tree`); None: ninguna todavía.
    #[cfg(feature = "arbol")]
    tree_followed: Option<Location>,
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
    /// Nombres de archivo con su extensión (switch «Mostrar extensiones» de la rueda dentada).
    show_extensions: bool,
    /// Casillas de selección en las listas (switch «Casillas de selección»).
    checkboxes: bool,
    /// Switch «El árbol sigue al panel», si el usuario lo tocó; `None` sigue a `arbol_sigue_panel`
    /// de `config.toml` (ver `tree_follows`).
    tree_follow: Option<bool>,
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
    /// Vista recordada de cada carpeta (`carpetas.toml`) y la vista por defecto.
    folder_views: FolderViews,
    /// Último cambio de vista de una carpeta sin guardar todavía (se guarda un segundo después).
    views_changed_at: Option<Instant>,
    /// Hay un aviso de guardado en camino (`WM_APP_VIEWS_SAVE`).
    views_save_posted: bool,
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
    /// Clic derecho sobre Atrás (false) o Adelante (true) de este panel: su menú del historial.
    history_ctx: Option<(usize, bool)>,
    /// Mapa de uso de disco: se crea al abrirlo y sigue vivo (con su escaneo) al cerrarlo.
    xtree: Option<Xtree>,
    /// La vista de xtree tapa los paneles.
    xtree_open: bool,
}

/// Argumentos de la línea de comandos: sesión a restaurar, `--probe <archivo>` (mide el
/// arranque y cierra) y una carpeta para abrir.
pub fn startup_args() -> (Option<Session>, Option<String>, Option<PathBuf>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (clean, probe, start_path) = parse_args(&args);
    // `--probe` mide el arranque limpio: sin sesión, con la carpeta pedida en un solo panel.
    let session = if clean { Some(Session { layout: 0, ..Default::default() }) } else { session::load() };
    (session, probe, start_path)
}

/// Lee los argumentos (sin el del programa): si se pide sesión limpia (`--probe`), el archivo de
/// `--probe` y la primera carpeta que exista.
pub fn parse_args(args: &[String]) -> (bool, Option<String>, Option<PathBuf>) {
    let probe = args.iter().position(|a| a == "--probe").and_then(|i| args.get(i + 1).cloned());
    let start_path = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--probe"))
        .map(|(_, a)| PathBuf::from(a))
        .filter(|p| p.is_dir());
    (probe.is_some(), probe, start_path)
}

/// Todo lo que `App::new` toma del exterior (disco, argumentos, hora de inicio): así se puede armar
/// una `App` en pruebas sin leer ni escribir nada del usuario.
pub struct Init {
    pub session: Session,
    pub history: History,
    pub config: Config,
    pub favorites: Favorites,
    pub groups: Groups,
    /// Vista de cada carpeta (`carpetas.toml`).
    pub folder_views: FolderViews,
    pub probe: Option<String>,
    pub start_path: Option<PathBuf>,
    pub started: Instant,
    pub marks: Vec<(&'static str, f64)>,
}

impl Init {
    /// Lee del disco lo que hace falta para arrancar (historial, configuración, favoritos y
    /// grupos) y aplica la preferencia de animaciones.
    pub fn load(session: Option<Session>, probe: Option<String>, start_path: Option<PathBuf>, started: Instant, marks: Vec<(&'static str, f64)>) -> Init {
        let session = session.unwrap_or_else(default_session);
        // `--probe` mide un arranque limpio: sin historial ni configuración del usuario.
        let history = if probe.is_none() { History::load() } else { History::default() };
        let config = if probe.is_none() { Config::load() } else { Config::default() };
        anim::set_enabled(config.animations.unwrap_or_else(anim::system_allows));
        anim::set_off_by_config(config.animations == Some(false));
        // `--probe` tampoco lee ni escribe la vista de las carpetas.
        let folder_views = if probe.is_none() { FolderViews::load() } else { FolderViews::default() };
        Init { session, history, config, favorites: Favorites::load(), groups: Groups::load(), folder_views, probe, start_path, started, marks }
    }

    /// Datos vacíos y sesión por defecto: nada del disco.
    #[cfg(test)]
    pub fn blank() -> Init {
        // `Config::default()` arma el `Keymap`, que consulta el entorno del sistema.
        crate::sys::testing::install_fakes();
        Init {
            session: default_session(),
            history: History::default(),
            config: Config::default(),
            favorites: Favorites::default(),
            groups: Groups::default(),
            folder_views: FolderViews::default(),
            probe: None,
            start_path: None,
            started: Instant::now(),
            marks: Vec::new(),
        }
    }
}

impl App {
    /// Arma la aplicación (sin ventana: la plataforma la fija después con `App::attach`, que se la
    /// pasa al host) y abre sus paneles.
    pub fn new(init: Init, host: Rc<dyn WindowHost>) -> App {
        let Init { session, history, config, favorites, groups, mut folder_views, probe, start_path, started, marks } = init;
        // Primera vez con memoria por carpeta: la vista y las columnas de la sesión pasan a ser
        // la vista por defecto.
        if folder_views.is_fresh() {
            folder_views.migrate(&session);
        }
        let theme = Theme::from_system(session.accent, session.dark_mode);
        let (tx, rx) = channel();
        let (ops_tx, ops_rx) = channel();
        let config_errors = config.errors.first().map(|e| format!("config.toml, {e}"));
        let config_errors = config_errors.or_else(|| folder_views.errors.first().map(|e| format!("carpetas.toml, {e}")));
        let mut app = App {
            hwnd: Handle::default(),
            host,
            dpi: 96,
            theme,
            panes: Vec::new(),
            layout: Layout::from_index(session.layout).unwrap_or(Layout::Four),
            splits: Splits { x: session.split_x, y: session.split_y },
            maximized: None,
            displayed: [None; PANES],
            layout_anim: None,
            last_frame: None,
            a11y_focus: None,
            active: 0,
            show_hidden: session.show_hidden,
            next_tab_id: 1,
            tx,
            rx,
            edit: None,
            suggest: None,
            hover: Hit::None,
            tip: None,
            tip_token: 0,
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
            view_notice: None,
            transparent: false,
            favorites,
            history,
            config,
            groups,
            group_name: session.group.clone(),
            group_rename: None,
            quick_watch: None,
            tree_ctx: None,
            #[cfg(feature = "arbol")]
            tree_followed: None,
            groups_menu_closed: None,
            palette: None,
            #[cfg(feature = "arbol")]
            tree: tree::Tree::new(),
            show_favorites: session.show_favorites,
            show_group_bar: session.group_bar,
            file_colors: session.file_colors,
            show_extensions: session.show_extensions,
            checkboxes: session.checkboxes,
            tree_follow: session.tree_follow,
            fav_drop: None,
            tab_drop: None,
            ghost: None,
            fav_edit: None,
            filter_edit: None,
            pending_search: None,
            search_token: 0,
            fav_ctx: None,
            header_ctx: None,
            folder_views,
            views_changed_at: None,
            views_save_posted: false,
            accent: session.accent,
            dark_mode: session.dark_mode,
            picker_open: false,
            about_open: false,
            term_menu: None,
            term_ctx: None,
            history_ctx: None,
            xtree: None,
            xtree_open: false,
        };
        app.restore_panes(&session, start_path);
        app.sync_folder_views();
        app.ensure_group();
        #[cfg(feature = "arbol")]
        {
            app.tree.visible = session.show_tree;
            app.tree.set_pinned(session.tree_pinned, false);
            app.tree.set_quick_open(session.quick_open);
        }
        app
    }

    // ---------- API de la plataforma ----------

    /// Fija la ventana (dueña de diálogos y destino de los mensajes de los servicios) y su DPI, y
    /// le pasa la ventana al host. La plataforma la llama al crear la ventana.
    pub fn attach(&mut self, window: Handle, dpi: u32) {
        self.hwnd = window;
        self.dpi = dpi;
        self.host.attach(window);
    }

    /// Fija el DPI de la ventana (la plataforma la llama cuando pasa a un monitor con otro DPI).
    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
    }

    /// La ventana que fijó `attach`.
    pub fn window(&self) -> Handle {
        self.hwnd
    }

    /// El DPI de la ventana (96 hasta que la plataforma lo fije).
    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    /// Posición y estado (maximizada) de la ventana guardados en la sesión, para mostrarla igual;
    /// se entrega una sola vez.
    pub fn take_window_state(&mut self) -> Option<([i32; 4], bool)> {
        self.window_state.take()
    }

    /// Agrega un hito del arranque para `--probe`, con el tiempo desde que arrancó la aplicación.
    pub fn mark(&mut self, name: &'static str) {
        self.marks.push((name, ms_since(self.started)));
    }

    /// Modo fijado con el switch (`None`: sigue al del sistema).
    pub fn dark_mode(&self) -> Option<bool> {
        self.dark_mode
    }

    /// Fondo Mica activo: con el marco extendido, la superficie de la ventana va en negro (que es
    /// transparente) para dejar ver la composición.
    pub fn transparent(&self) -> bool {
        self.transparent
    }

    /// Vuelve a armar el estilo de las cajas de texto con el tema actual (cambió el DPI).
    pub fn restyle_text(&self) {
        self.host.text_restyle(&self.theme);
    }

    /// Antes de un clic con el botón izquierdo en `(x, y)`: el foco vuelve a la ventana, salvo
    /// sobre el desplegable de la barra de dirección y su botón de borrar (no se lo quitan a la caja).
    pub fn focus_for_click(&self, x: f32, y: f32) {
        if !matches!(self.hit(x, y), Hit::Suggest(_) | Hit::Pane(_, PaneHit::AddressClear)) {
            self.host.focus_main();
        }
    }
}

// ---------- accesibilidad ----------

/// La aplicación vista por los lectores de pantalla (ver `crate::a11y`).
impl A11yView for App {
    fn shown_panes(&self) -> Vec<usize> {
        App::shown_panes(self)
    }
    fn active(&self) -> usize {
        self.active
    }
    fn item_count(&self, pane: usize) -> usize {
        self.panes[pane].tab().list.count()
    }
    fn focus_pos(&self, pane: usize) -> Option<usize> {
        self.panes[pane].tab().list.focus_pos()
    }
    fn item_info(&self, pane: usize, pos: usize) -> Option<ItemInfo> {
        self.panes[pane].tab().list.item_info(pos)
    }
    fn pane_title(&self, pane: usize) -> String {
        self.panes[pane].tab().location.title()
    }
    fn tab_pos(&self, pane: usize) -> (usize, usize) {
        let pane = &self.panes[pane];
        (pane.active, pane.tabs.len())
    }
    fn address_text(&self, pane: usize) -> String {
        let location = &self.panes[pane].tab().location;
        match location.target() {
            Some(t) => t.display().to_string(),
            None => location.title(),
        }
    }
    fn status(&self) -> String {
        self.panes[self.active].tab().status()
    }
    fn pane_rects(&self, pane: usize) -> (geom::Rect, geom::Rect, geom::Rect) {
        let pane = &self.panes[pane];
        (pane.rect, pane.address, pane.list)
    }
    fn client(&self) -> (f32, f32) {
        self.client_dips()
    }
    fn selected_positions(&self, pane: usize) -> Vec<usize> {
        self.panes[pane].tab().list.selected_positions()
    }
    fn item_at(&self, pane: usize, x: f32, y: f32) -> Option<usize> {
        self.panes[pane].tab().list.pos_at_point(x, y)
    }
}

impl App {
    /// Lo que pide un lector de pantalla (seleccionar, abrir, mover el foco). Quien llama ya
    /// comprobó que el elemento existe.
    pub fn a11y_apply(&mut self, a: A11yAction) {
        match a {
            A11yAction::Focus(el) => {
                if let Some(p) = a11y::pane_of(el) {
                    self.set_active(p);
                }
                if let a11y::El::Item(p, pos) = el {
                    self.panes[p].tab_mut().list.select_pos(pos);
                }
                self.redraw();
            }
            A11yAction::Select(p, pos) => {
                self.set_active(p);
                self.panes[p].tab_mut().list.select_pos(pos);
                self.redraw();
            }
            A11yAction::AddToSelection(p, pos) | A11yAction::RemoveFromSelection(p, pos) => {
                let on = matches!(a, A11yAction::AddToSelection(..));
                self.panes[p].tab_mut().list.set_selected_pos(pos, on);
                self.redraw();
            }
            A11yAction::Invoke(p, pos) => {
                let Some(entry) = self.panes[p].tab().list.entry_of_pos(pos) else { return };
                self.set_active(p);
                self.apply(p, Action::Open(entry));
            }
        }
    }

    /// Avisa a los lectores de pantalla cuando cambia el elemento con foco. Se llama después de
    /// cada frame: es una comparación barata, y solo hace algo si hay un lector escuchando.
    pub(super) fn notify_a11y_focus(&mut self) {
        let host = self.host.clone();
        let Some(acc) = host.accessibility() else { return };
        if !acc.listening() {
            return;
        }
        let now = (self.active, self.panes[self.active].tab().list.focus_pos());
        if self.a11y_focus == Some(now) {
            return;
        }
        self.a11y_focus = Some(now);
        acc.notify_focus(a11y::focus_el(self));
    }
}

pub fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Primera vez: 4 paneles en "Este equipo".
fn default_session() -> Session {
    let panes = (0..PANES).map(|_| PaneState { tabs: vec![Location::Computer], active: 0 }).collect();
    Session { panes, ..Default::default() }
}

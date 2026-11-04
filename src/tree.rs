//! Árbol de carpetas lateral (F-3.5, experimental). Solo se compila con la feature `arbol`:
//! `scripts\build.cmd build --release --features arbol`.
//!
//! Panel a la izquierda de la ventana que navega el panel activo. Arranca con la carpeta del
//! usuario y las unidades; las subcarpetas se leen al expandir un nodo, en un hilo aparte (una
//! unidad de red lenta no congela la ventana), y el resultado vuelve con `WM_APP_TREE`.
//!
//! La barra se puede fijar (expandida, al lado de los paneles) o soltar: suelta queda una franja
//! angosta con solo los íconos y, al asomar el mouse, se expande encima de los paneles. La sección
//! "Acceso rápido" se lee del propio Shell (`fs::quick_access`) y se vuelve a leer en un hilo
//! cuando cambia (`WM_APP_QUICK`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::anim;
use crate::fs::{Location, QuickItem};
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::{Theme, alpha};

/// Terminó de leerse una carpeta del árbol.
pub const WM_APP_TREE: u32 = WM_APP + 20;
/// Terminó de leerse el Acceso rápido de Windows (o el watcher avisó que cambió).
pub const WM_APP_QUICK: u32 = WM_APP + 21;

/// Ancho inicial del panel; se puede cambiar arrastrando su borde derecho.
pub const WIDTH: f32 = 240.0;
pub const MIN_W: f32 = 160.0;
pub const MAX_W: f32 = 560.0;
/// Ancho de la franja colapsada (solo íconos).
pub const COLLAPSED_W: f32 = 48.0;
/// Grosor de las barras de desplazamiento y lugar que ocupan al pie / al costado.
const BAR: f32 = 6.0;
const BAR_ROOM: f32 = 12.0;
const ROW_H: f32 = 32.0;
const INDENT: f32 = 18.0;
/// Margen a los costados de las filas.
const SIDE: f32 = 8.0;
/// Franja de arriba: título y botón de colapsar; las filas arrancan debajo.
const TOP_PAD: f32 = 32.0;
/// Duración de la animación del ancho al fijar, soltar y expandir.
const SLIDE_SECS: f32 = 0.32;
/// Máximo de carpetas del Acceso rápido que se muestran.
const QUICK_MAX: usize = 20;

/// Qué es una fila, para su ícono y lo que abre.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Folder,
    /// Carpeta del usuario, fijada arriba como en el Explorador.
    Pinned,
    /// OneDrive.
    Cloud,
    /// "Inicio": lleva a "Este equipo"; sin subcarpetas en el árbol.
    Home,
    Drive,
    /// "Este equipo": no es una ruta.
    Computer,
    /// "Acceso rápido": agrupa las carpetas ancladas y frecuentes de Windows; no es una ruta.
    QuickAccess,
    /// "Red" y "Linux": carpetas virtuales del Shell (el nombre de análisis va en `path`).
    Network,
    Linux,
    /// Línea que separa grupos.
    Separator,
}

fn rgb(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}

/// Ícono y color de una carpeta del usuario fijada (la posición es la de `fs::user_folder`).
const PINNED: [(&str, &str, (f32, f32, f32)); 6] = [
    ("Escritorio", "\u{E70A}", (0.30, 0.65, 0.95)),
    ("Descargas", "\u{E896}", (0.25, 0.78, 0.55)),
    ("Documentos", "\u{E8A5}", (0.62, 0.70, 0.80)),
    ("Imágenes", "\u{E91B}", (0.35, 0.60, 0.95)),
    ("Música", "\u{E8D6}", (0.95, 0.45, 0.35)),
    ("Vídeos", "\u{E714}", (0.70, 0.45, 0.90)),
];

/// Chincheta: carpeta anclada al Acceso rápido.
const PIN_GLYPH: &str = "\u{E840}";

/// Valor animado (0..100) con curva suave: va de donde está al destino nuevo en `SLIDE_SECS`.
#[derive(Clone, Copy)]
struct Slide {
    value: f32,
    from: f32,
    to: f32,
    t: f32,
}

impl Slide {
    fn at(v: f32) -> Self {
        Self { value: v, from: v, to: v, t: SLIDE_SECS }
    }

    /// Salta al valor sin animar.
    fn jump(&mut self, v: f32) {
        *self = Self::at(v);
    }

    /// Avanza `dt` segundos hacia `target`. Devuelve si sigue en curso.
    fn tick(&mut self, target: f32, dt: f32) -> bool {
        if !anim::slide_enabled() {
            self.jump(target);
            return false;
        }
        // Un destino nuevo (también a mitad de camino) arranca de donde está ahora.
        if target != self.to {
            (self.from, self.to, self.t) = (self.value, target, 0.0);
        }
        self.t = (self.t + dt).min(SLIDE_SECS);
        let k = self.t / SLIDE_SECS;
        // Arranca y frena despacio (ease in-out cúbico): sin tirón al empezar ni al llegar.
        let eased = if k < 0.5 { 4.0 * k * k * k } else { 1.0 - (-2.0 * k + 2.0).powi(3) / 2.0 };
        self.value = anim::lerp(self.from, self.to, eased);
        self.t < SLIDE_SECS
    }
}

/// Barra de desplazamiento del panel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Bar {
    /// Horizontal, para llegar a los nombres largos y a las carpetas muy anidadas.
    H,
    V,
}

struct Node {
    /// Ancho del nombre (0: todavía sin medir).
    text_w: f32,
    kind: Kind,
    path: PathBuf,
    name: String,
    /// Posición en `PINNED` si es una carpeta fijada.
    pin: usize,
    /// Está anclada al Acceso rápido de Windows (con su chincheta).
    pinned: bool,
    depth: usize,
    /// None: sin leer todavía. Some(vacío): no tiene subcarpetas.
    children: Option<Vec<usize>>,
    expanded: bool,
    loading: bool,
}

impl Node {
    fn new(kind: Kind, path: PathBuf, name: String, depth: usize) -> Self {
        Self { text_w: 0.0, kind, path, name, pin: 0, pinned: false, depth, children: None, expanded: false, loading: false }
    }

    /// Lo que abre en el panel activo (None si no es navegable).
    fn location(&self) -> Option<Location> {
        match self.kind {
            Kind::Separator | Kind::QuickAccess => None,
            Kind::Computer | Kind::Home => Some(Location::Computer),
            Kind::Network | Kind::Linux => {
                Some(Location::Shell { parsing: self.path.to_string_lossy().into_owned(), name: self.name.clone(), parent: None })
            }
            _ => Some(Location::Path(self.path.clone())),
        }
    }
}

/// Qué tocó un clic.
pub enum TreeHit {
    /// Expandir o contraer el nodo.
    Toggle(usize),
    /// Abrir la carpeta en el panel activo.
    Open(Location),
    /// La chincheta: fijar la barra o dejar que quede en franja.
    Pin,
    /// La chincheta de una carpeta anclada: la desancla del Acceso rápido (como en el Explorador).
    Unpin(PathBuf),
    Nothing,
}

pub struct Tree {
    pub visible: bool,
    /// Fija al lado de los paneles; si no, queda la franja de íconos y se expande encima al asomar el mouse.
    pub pinned: bool,
    /// Sin fijar: el mouse está encima y se muestra expandida.
    open: bool,
    /// Avance de la animación: 0 franja de íconos, 100 expandida.
    expand: Slide,
    /// Lugar que se le reserva a los paneles (fija: el ancho; suelta: la franja). Se anima aparte
    /// para que los paneles se acomoden con suavidad aunque la barra siga expandida encima.
    lay: Slide,
    toggle_hot: bool,
    /// Fila cuya chincheta tiene el mouse encima.
    pin_hover: Option<usize>,
    nodes: Vec<Node>,
    roots: Vec<usize>,
    scroll: f32,
    /// Desplazamiento horizontal.
    hscroll: f32,
    /// Ancho del panel expandido (cambiable).
    pub width: f32,
    /// Lo que ocupa el contenido y lo que se ve, del último cuadro.
    content_w: f32,
    view_w: f32,
    view_h: f32,
    htrack: Rect,
    hthumb: Rect,
    vtrack: Rect,
    vthumb: Rect,
    hover: Option<usize>,
    area: Rect,
    /// Carpetas del usuario que ya están arriba (no se repiten en el Acceso rápido).
    known: Vec<PathBuf>,
    /// Rutas ancladas al Acceso rápido de Windows, de la última lectura.
    pinned_paths: Vec<PathBuf>,
    quick_loading: bool,
    quick_dirty: bool,
    tx: Sender<(usize, Vec<(String, PathBuf)>)>,
    rx: Receiver<(usize, Vec<(String, PathBuf)>)>,
    qtx: Sender<Vec<QuickItem>>,
    qrx: Receiver<Vec<QuickItem>>,
}

impl Tree {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let (qtx, qrx) = channel();
        let mut t = Self {
            visible: false,
            pinned: true,
            open: false,
            expand: Slide::at(100.0),
            lay: Slide::at(100.0),
            toggle_hot: false,
            pin_hover: None,
            nodes: Vec::new(),
            roots: Vec::new(),
            scroll: 0.0,
            hscroll: 0.0,
            width: WIDTH,
            content_w: 0.0,
            view_w: WIDTH,
            view_h: 0.0,
            htrack: rect(0.0, 0.0, 0.0, 0.0),
            hthumb: rect(0.0, 0.0, 0.0, 0.0),
            vtrack: rect(0.0, 0.0, 0.0, 0.0),
            vthumb: rect(0.0, 0.0, 0.0, 0.0),
            hover: None,
            area: rect(0.0, 0.0, 0.0, 0.0),
            known: Vec::new(),
            pinned_paths: Vec::new(),
            quick_loading: false,
            quick_dirty: false,
            tx,
            rx,
            qtx,
            qrx,
        };
        // Inicio (lleva a "Este equipo") y OneDrive.
        t.add_root(Kind::Home, PathBuf::new(), "Inicio".to_string(), 0);
        let home = t.roots[t.roots.len() - 1];
        t.nodes[home].children = Some(Vec::new());
        if let Some(cloud) = std::env::var_os("OneDrive").map(PathBuf::from).filter(|p| p.is_dir()) {
            t.add_root(Kind::Cloud, cloud, "OneDrive".to_string(), 0);
        }
        t.add_root(Kind::Separator, PathBuf::new(), String::new(), 0);
        // Carpetas del usuario, como en el Explorador.
        for (i, (name, ..)) in PINNED.iter().enumerate() {
            if let Some(path) = crate::fs::user_folder(i) {
                t.known.push(path.clone());
                t.add_root(Kind::Pinned, path, (*name).to_string(), i);
            }
        }
        // Acceso rápido de Windows: ya abierto; sus carpetas llegan de `request_quick` (en otro hilo).
        t.add_root(Kind::Separator, PathBuf::new(), String::new(), 0);
        t.add_root(Kind::QuickAccess, PathBuf::new(), "Acceso rápido".to_string(), 0);
        let quick = t.roots[t.roots.len() - 1];
        t.nodes[quick].expanded = true;
        t.nodes[quick].children = Some(Vec::new());
        t.add_root(Kind::Separator, PathBuf::new(), String::new(), 0);
        // "Este equipo", ya abierto, con sus unidades.
        t.add_root(Kind::Computer, PathBuf::new(), "Este equipo".to_string(), 0);
        let computer = t.roots[t.roots.len() - 1];
        t.nodes[computer].expanded = true;
        t.refresh_drives();
        // Como en el Explorador: Red y, si hay WSL, Linux. Sus hijos se leen al expandir.
        t.add_root(Kind::Network, PathBuf::from(crate::fs::NETWORK_PLACE), "Red".to_string(), 0);
        if crate::fs::shell_place_exists(crate::fs::LINUX_PLACE) {
            t.add_root(Kind::Linux, PathBuf::from(crate::fs::LINUX_PLACE), "Linux".to_string(), 0);
        }
        t
    }

    // ---------- Acceso rápido ----------

    /// Vuelve a leer el Acceso rápido de Windows en otro hilo (si ya hay una lectura en curso,
    /// se repite al terminar).
    pub fn request_quick(&mut self, hwnd: HWND) {
        if self.quick_loading {
            self.quick_dirty = true;
            return;
        }
        self.quick_loading = true;
        let (tx, hwnd) = (self.qtx.clone(), hwnd.0 as usize);
        std::thread::spawn(move || {
            let items = crate::fs::quick_access();
            if tx.send(items).is_ok() {
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_QUICK, WPARAM(1), LPARAM(0));
                }
            }
        });
    }

    /// Aplica la lectura terminada del Acceso rápido. Devuelve si hay que pedir otra (cambió
    /// algo mientras se leía).
    pub fn on_quick_loaded(&mut self) -> bool {
        self.quick_loading = false;
        let mut last = None;
        while let Ok(items) = self.qrx.try_recv() {
            last = Some(items);
        }
        if let Some(items) = last {
            self.apply_quick(items);
        }
        std::mem::take(&mut self.quick_dirty)
    }

    /// Rearma las carpetas del grupo "Acceso rápido" (sin repetir las del usuario que ya están
    /// arriba) y recuerda cuáles están ancladas, para ponerles la chincheta.
    fn apply_quick(&mut self, items: Vec<QuickItem>) {
        self.pinned_paths = items.iter().filter(|i| i.pinned).map(|i| i.path.clone()).collect();
        let Some(&quick) = self.roots.iter().find(|&&r| self.nodes[r].kind == Kind::QuickAccess) else { return };
        let old = self.nodes[quick].children.clone().unwrap_or_default();
        let mut ids = Vec::new();
        let shown = items.into_iter().filter(|i| !self.known.iter().any(|k| crate::fs::same_path(k, &i.path))).take(QUICK_MAX);
        for (n, item) in shown.enumerate() {
            let mut node = Node::new(Kind::Folder, item.path, item.name, 1);
            node.pinned = item.pinned;
            // Se reusa el hueco del nodo anterior en esa posición, para no crecer con cada lectura.
            if let Some(&id) = old.get(n) {
                self.nodes[id] = node;
                ids.push(id);
            } else {
                self.nodes.push(node);
                ids.push(self.nodes.len() - 1);
            }
        }
        self.nodes[quick].children = Some(ids);
    }

    /// ¿Está esa carpeta anclada al Acceso rápido de Windows (según la última lectura)?
    pub fn is_pinned(&self, path: &Path) -> bool {
        self.pinned_paths.iter().any(|p| crate::fs::same_path(p, path))
    }

    // ---------- unidades ----------

    /// Vuelve a leer las unidades de "Este equipo" (se enchufó o quitó una): las que siguen
    /// conservan su estado, las nuevas se agregan en su lugar.
    pub fn refresh_drives(&mut self) {
        let Some(&computer) = self.roots.iter().find(|&&r| self.nodes[r].kind == Kind::Computer) else { return };
        let old = self.nodes[computer].children.clone().unwrap_or_default();
        let drives = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let mut ids = Vec::new();
        for i in 0..26u32 {
            if drives & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let path = PathBuf::from(format!("{letter}:\\"));
            let name = crate::fs::drive_label(&format!("{letter}:"));
            if let Some(&id) = old.iter().find(|&&id| self.nodes[id].path == path) {
                self.nodes[id].name = name;
                self.nodes[id].text_w = 0.0;
                ids.push(id);
            } else {
                self.nodes.push(Node::new(Kind::Drive, path, name, 1));
                ids.push(self.nodes.len() - 1);
            }
        }
        self.nodes[computer].children = Some(ids);
    }

    fn add_root(&mut self, kind: Kind, path: PathBuf, name: String, pin: usize) {
        let mut node = Node::new(kind, path, name, 0);
        node.pin = pin;
        self.nodes.push(node);
        self.roots.push(self.nodes.len() - 1);
    }

    /// Nodos a la vista, en orden (los hijos de los expandidos, recursivamente; son los mismos con
    /// la barra fija o en franja, así nada se corre cuando se expande). Sin el Acceso rápido si está vacío ni separadores repetidos.
    fn rows(&self) -> Vec<usize> {
        fn walk(t: &Tree, ids: &[usize], out: &mut Vec<usize>) {
            for &id in ids {
                out.push(id);
                let n = &t.nodes[id];
                if n.expanded {
                    if let Some(c) = &n.children {
                        walk(t, c, out);
                    }
                }
            }
        }
        let mut raw = Vec::new();
        walk(self, &self.roots, &mut raw);
        let mut out: Vec<usize> = Vec::with_capacity(raw.len());
        for id in raw {
            let n = &self.nodes[id];
            if n.kind == Kind::QuickAccess && n.children.as_ref().is_some_and(|c| c.is_empty()) {
                continue;
            }
            if n.kind == Kind::Separator && out.last().is_none_or(|&l| self.nodes[l].kind == Kind::Separator) {
                continue;
            }
            out.push(id);
        }
        while out.last().is_some_and(|&l| self.nodes[l].kind == Kind::Separator) {
            out.pop();
        }
        out
    }

    // ---------- fijar / franja ----------

    /// Ancho que ocupa ahora (entre la franja y el ancho completo, según la animación).
    pub fn width_now(&self) -> f32 {
        COLLAPSED_W + (self.width - COLLAPSED_W) * (self.expand.value / 100.0).clamp(0.0, 1.0)
    }

    /// Ancho que se le reserva a la izquierda de los paneles: fija, el que tiene ahora; suelta,
    /// solo la franja (al expandirse se superpone).
    pub fn layout_width(&self) -> f32 {
        COLLAPSED_W + (self.width - COLLAPSED_W) * (self.lay.value / 100.0).clamp(0.0, 1.0)
    }

    /// Fija o suelta; sin `animate` salta al final (al arrancar). Al soltarla sigue expandida
    /// hasta que el mouse se vaya.
    pub fn set_pinned(&mut self, pinned: bool, animate: bool) {
        self.pinned = pinned;
        self.open = !pinned && animate;
        self.hover = None;
        if !animate {
            let v = if pinned { 100.0 } else { 0.0 };
            self.expand.jump(v);
            self.lay.jump(v);
        }
    }

    /// Avanza la animación del ancho. Devuelve si sigue en curso.
    pub fn tick(&mut self, dt: f32) -> bool {
        let shown = if self.pinned || self.open { 100.0 } else { 0.0 };
        let reserved = if self.pinned { 100.0 } else { 0.0 };
        let a = self.expand.tick(shown, dt);
        let b = self.lay.tick(reserved, dt);
        a || b
    }

    /// Expandida del todo (con el ancho a mano y las barras de desplazamiento).
    fn settled(&self) -> bool {
        self.expand.value >= 99.9
    }

    /// ¿Se ve (y responde) la chincheta de arriba? No en la franja de íconos.
    fn pin_btn_shown(&self) -> bool {
        self.expand.value >= 50.0
    }

    fn toggle_rect(&self) -> Rect {
        let e = (self.expand.value / 100.0).clamp(0.0, 1.0);
        let left = anim::lerp(self.area.left + (COLLAPSED_W - 24.0) / 2.0, self.area.right - 30.0, e);
        rect(left, self.area.top + 4.0, left + 24.0, self.area.top + 28.0)
    }

    // ---------- lectura de subcarpetas ----------

    /// Expande o contrae; la primera vez lee las subcarpetas en otro hilo.
    pub fn toggle(&mut self, hwnd: HWND, id: usize, show_hidden: bool) {
        let n = &mut self.nodes[id];
        if n.kind == Kind::Separator {
            return;
        }
        n.expanded = !n.expanded;
        if n.expanded {
            self.load_children(hwnd, id, show_hidden);
        }
    }

    /// Lee las subcarpetas de un nodo en otro hilo, si todavía no se leyeron.
    fn load_children(&mut self, hwnd: HWND, id: usize, show_hidden: bool) {
        let n = &mut self.nodes[id];
        if n.children.is_some() || n.loading {
            return;
        }
        n.loading = true;
        let (tx, path, virt) = (self.tx.clone(), n.path.clone(), matches!(n.kind, Kind::Network | Kind::Linux));
        let hwnd = hwnd.0 as usize;
        std::thread::spawn(move || {
            let dirs = if virt { crate::fs::shell_subfolders(&path.to_string_lossy()) } else { subfolders(&path, show_hidden) };
            if tx.send((id, dirs)).is_ok() {
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_TREE, WPARAM(0), LPARAM(0));
                }
            }
        });
    }

    /// Aplica las lecturas terminadas.
    pub fn on_loaded(&mut self) {
        while let Ok((id, dirs)) = self.rx.try_recv() {
            let depth = self.nodes[id].depth + 1;
            let mut ids = Vec::with_capacity(dirs.len());
            for (name, path) in dirs {
                self.nodes.push(Node::new(Kind::Folder, path, name, depth));
                ids.push(self.nodes.len() - 1);
            }
            let n = &mut self.nodes[id];
            n.children = Some(ids);
            n.loading = false;
        }
    }

    // ---------- mouse ----------

    /// Nodo de la fila bajo `y` (las filas arrancan `TOP_PAD` debajo del borde).
    fn node_at(&self, y: f32) -> Option<usize> {
        let row = ((y - self.area.top - TOP_PAD + self.scroll) / ROW_H).floor();
        if row < 0.0 {
            return None;
        }
        self.rows().get(row as usize).copied()
    }

    pub fn hit(&self, x: f32, y: f32) -> TreeHit {
        if !contains(&self.area, x, y) {
            return TreeHit::Nothing;
        }
        if self.pin_btn_shown() && contains(&self.toggle_rect(), x, y) {
            return TreeHit::Pin;
        }
        let Some(id) = self.node_at(y) else { return TreeHit::Nothing };
        let n = &self.nodes[id];
        if self.settled() && self.shows_pin(n) && self.on_row_pin(x) {
            return TreeHit::Unpin(n.path.clone());
        }
        let chevron_right = self.area.left + SIDE + 2.0 + n.depth as f32 * INDENT + 18.0 - self.hscroll;
        match n.kind {
            Kind::Separator => TreeHit::Nothing,
            _ if self.settled() && x < chevron_right => TreeHit::Toggle(id),
            // El grupo no es una carpeta: al tocarlo se abre o cierra.
            Kind::QuickAccess => TreeHit::Toggle(id),
            _ => n.location().map_or(TreeHit::Nothing, TreeHit::Open),
        }
    }

    /// Ruta de disco del elemento bajo el mouse, para su menú contextual.
    pub fn path_at(&self, x: f32, y: f32) -> Option<PathBuf> {
        if !(contains(&self.area, x, y) && y >= self.area.top + TOP_PAD) {
            return None;
        }
        let id = self.node_at(y)?;
        let n = &self.nodes[id];
        matches!(n.kind, Kind::Folder | Kind::Pinned | Kind::Cloud | Kind::Drive).then(|| n.path.clone())
    }

    /// Se ve ahora.
    pub fn shown(&self) -> bool {
        self.visible
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.visible && contains(&self.area, x, y)
    }

    /// `keep_open`: hay un arrastre en curso (no se abre ni se cierra la franja).
    pub fn on_mouse_move(&mut self, x: f32, y: f32, keep_open: bool) -> bool {
        let inside = self.visible && contains(&self.area, x, y);
        let hover = if inside && y >= self.area.top + TOP_PAD { self.node_at(y) } else { None };
        let toggle_hot = inside && self.pin_btn_shown() && contains(&self.toggle_rect(), x, y);
        // Sin fijar, asomar el mouse sobre la franja la expande; al salir vuelve a la franja.
        let open = if !self.pinned && self.visible && !keep_open { inside } else { self.open };
        let pin_hover = hover.filter(|&id| self.settled() && self.shows_pin(&self.nodes[id]) && self.on_row_pin(x));
        let changed = hover != self.hover || toggle_hot != self.toggle_hot || open != self.open || pin_hover != self.pin_hover;
        self.pin_hover = pin_hover;
        self.hover = hover;
        self.toggle_hot = toggle_hot;
        self.open = open;
        changed
    }

    /// El mouse salió de la ventana.
    pub fn on_mouse_leave(&mut self) {
        self.hover = None;
        self.toggle_hot = false;
        self.pin_hover = None;
        self.open = false;
    }

    pub fn on_wheel(&mut self, rows: f32) {
        self.scroll = (self.scroll - rows * ROW_H).clamp(0.0, self.max_scroll());
    }

    /// Rueda con Mayús o rueda horizontal: mueve el contenido a los costados.
    pub fn on_wheel_h(&mut self, rows: f32) {
        self.hscroll = (self.hscroll - rows * ROW_H).clamp(0.0, self.max_hscroll());
    }

    fn max_scroll(&self) -> f32 {
        (self.rows().len() as f32 * ROW_H + TOP_PAD - self.view_h).max(0.0)
    }

    fn max_hscroll(&self) -> f32 {
        if self.settled() { (self.content_w - self.view_w).max(0.0) } else { 0.0 }
    }

    /// El borde derecho, donde se agarra para cambiar el ancho (solo expandida).
    pub fn edge_at(&self, x: f32, y: f32) -> bool {
        self.visible
            && self.settled()
            && y >= self.area.top
            && y <= self.area.bottom
            && x >= self.area.right - 3.0
            && x <= self.area.right + crate::ui::GAP + 1.0
    }

    pub fn set_width(&mut self, w: f32) {
        self.width = w.clamp(MIN_W, MAX_W);
    }

    /// Barra de desplazamiento bajo el mouse y a qué distancia del inicio de su pulgar se agarró.
    pub fn bar_at(&self, x: f32, y: f32) -> Option<(Bar, f32)> {
        let grow = |t: Rect| rect(t.left - 4.0, t.top - 4.0, t.right + 3.0, t.bottom + 3.0);
        if self.shown() && self.htrack.right > self.htrack.left && contains(&grow(self.htrack), x, y) {
            let t = self.hthumb;
            let grab = if x >= t.left && x <= t.right { x - t.left } else { (t.right - t.left) / 2.0 };
            return Some((Bar::H, grab));
        }
        if self.shown() && self.vtrack.bottom > self.vtrack.top && contains(&grow(self.vtrack), x, y) {
            let t = self.vthumb;
            let grab = if y >= t.top && y <= t.bottom { y - t.top } else { (t.bottom - t.top) / 2.0 };
            return Some((Bar::V, grab));
        }
        None
    }

    /// Arrastrando el pulgar de una barra: el mouse está en (x, y).
    pub fn drag_bar(&mut self, bar: Bar, grab: f32, x: f32, y: f32) {
        match bar {
            Bar::H => {
                let span = (self.htrack.right - self.htrack.left) - (self.hthumb.right - self.hthumb.left);
                if span > 0.0 {
                    let frac = ((x - grab - self.htrack.left) / span).clamp(0.0, 1.0);
                    self.hscroll = frac * self.max_hscroll();
                }
            }
            Bar::V => {
                let span = (self.vtrack.bottom - self.vtrack.top) - (self.vthumb.bottom - self.vthumb.top);
                if span > 0.0 {
                    let frac = ((y - grab - self.vtrack.top) / span).clamp(0.0, 1.0);
                    self.scroll = frac * self.max_scroll();
                }
            }
        }
    }

    // ---------- dibujo ----------

    /// Ícono y color de un nodo.
    fn icon(&self, n: &Node, th: &Theme) -> (&'static str, D2D1_COLOR_F) {
        match n.kind {
            Kind::Cloud => ("\u{E753}", rgb(0.30, 0.65, 0.95)),
            Kind::Home => ("\u{E80F}", rgb(0.95, 0.60, 0.30)),
            Kind::Computer => ("\u{E7F4}", rgb(0.30, 0.65, 0.95)),
            Kind::QuickAccess => ("\u{E734}", rgb(0.95, 0.75, 0.25)),
            Kind::Drive => ("\u{EDA2}", th.text_dim),
            Kind::Network => ("\u{E839}", rgb(0.30, 0.65, 0.95)),
            Kind::Linux => ("\u{E756}", th.text_dim),
            Kind::Pinned => {
                let (_, glyph, (r, g, b)) = PINNED[n.pin];
                (glyph, rgb(r, g, b))
            }
            _ => ("\u{E8D5}", rgb(1.0, 0.79, 0.28)),
        }
    }

    /// ¿Está `x` sobre el lugar de la chincheta de una fila?
    fn on_row_pin(&self, x: f32) -> bool {
        let right = self.area.left + self.view_w - SIDE;
        x >= right - 28.0 && x <= right - 2.0
    }

    /// ¿Lleva la chincheta? (anclada en el Acceso rápido de Windows, esté arriba o abajo).
    fn shows_pin(&self, n: &Node) -> bool {
        n.pinned || (n.kind == Kind::Pinned && self.is_pinned(&n.path))
    }

    /// `current`: carpeta del panel activo, resaltada si está a la vista.
    pub fn draw(&mut self, p: &Painter, area: Rect, current: Option<&Path>) {
        self.area = area;
        let th = p.theme;
        let e = (self.expand.value / 100.0).clamp(0.0, 1.0);
        p.round_fill(area, 8.0, th.surface);
        let rows = self.rows();
        // Lo que ocupa cada fila de ancho (se mide una sola vez por nodo).
        let mut content_w = 0.0f32;
        for &id in &rows {
            let n = &mut self.nodes[id];
            if n.kind == Kind::Separator {
                continue;
            }
            if n.text_w == 0.0 {
                n.text_w = p.measure(&p.fonts.ui, &n.name);
            }
            content_w = content_w.max(SIDE + 2.0 + n.depth as f32 * INDENT + 48.0 + n.text_w + SIDE + 8.0);
        }
        // Qué barras hacen falta (una puede hacer necesaria la otra). Colapsada o a mitad de la
        // animación no hay barra horizontal.
        let total_h = rows.len() as f32 * ROW_H + TOP_PAD;
        let (full_w, full_h) = (area.right - area.left, area.bottom - area.top);
        let mut view_w = full_w;
        let mut view_h = full_h;
        let mut need_v = total_h > view_h;
        if need_v {
            view_w -= BAR_ROOM;
        }
        let allow_h = self.settled();
        let need_h = allow_h && content_w > view_w;
        if need_h {
            view_h -= BAR_ROOM;
            if !need_v && total_h > view_h {
                need_v = true;
                view_w -= BAR_ROOM;
            }
        }
        // En la franja no hay barras: se desplaza con la rueda.
        if !allow_h {
            need_v = false;
            view_w = full_w;
        }
        self.content_w = content_w;
        self.view_w = view_w;
        self.view_h = view_h;
        self.scroll = self.scroll.clamp(0.0, (total_h - view_h).max(0.0));
        self.hscroll = self.hscroll.clamp(0.0, if allow_h { (content_w - view_w).max(0.0) } else { 0.0 });
        let hs = self.hscroll;
        let body = rect(area.left + 1.0, area.top + TOP_PAD - 2.0, area.left + view_w - 1.0, area.top + view_h);
        let centered_x = area.left + (COLLAPSED_W - 22.0) / 2.0;
        p.clip(body, || {
            for (i, &id) in rows.iter().enumerate() {
                let y = area.top + TOP_PAD + i as f32 * ROW_H - self.scroll;
                if y + ROW_H < area.top || y > area.top + view_h {
                    continue;
                }
                let n = &self.nodes[id];
                if n.kind == Kind::Separator {
                    let mid = (y + ROW_H / 2.0).round();
                    p.round_fill(rect(area.left + SIDE + 4.0, mid, area.left + view_w - SIDE - 4.0, mid + 1.0), 0.5, alpha(th.text, 0.14));
                    continue;
                }
                let row = rect(area.left + SIDE, y + 1.0, area.left + view_w - SIDE, y + ROW_H - 1.0);
                let is_current = !matches!(n.kind, Kind::Computer | Kind::Home | Kind::Network | Kind::Linux | Kind::QuickAccess)
                    && current.is_some_and(|c| crate::fs::same_path(c, &n.path));
                if is_current {
                    p.round_fill(row, 5.0, alpha(th.text, 0.11));
                } else if self.hover == Some(id) {
                    p.round_fill(row, 5.0, alpha(th.text, 0.06));
                }
                let x = area.left + SIDE + 2.0 + n.depth as f32 * INDENT - hs;
                // El ícono se desliza de su lugar en la lista al centro de la franja.
                let icon_x = anim::lerp(centered_x, x + 20.0, e);
                // Chevron: derecha (contraído), abajo (expandido); nada si no tiene subcarpetas.
                let has_children = n.children.as_ref().is_none_or(|c| !c.is_empty());
                if has_children && e > 0.05 {
                    let glyph = if n.loading { "\u{E712}" } else if n.expanded { "\u{E70D}" } else { "\u{E76C}" };
                    p.text(&p.fonts.glyphs_small, glyph, rect(x, y, x + 16.0, y + ROW_H), alpha(th.text_dim, e * e));
                }
                let (icon, color) = self.icon(n, th);
                p.text(&p.fonts.glyphs, icon, rect(icon_x, y, icon_x + 22.0, y + ROW_H), color);
                if e > 0.05 {
                    p.text(&p.fonts.ui, &n.name, rect(icon_x + 28.0, y, icon_x + 28.0 + n.text_w + 12.0, y + ROW_H), alpha(th.text, e * e));
                    if self.shows_pin(n) {
                        let slot = rect(row.right - 26.0, y + 5.0, row.right - 4.0, y + ROW_H - 5.0);
                        let hot = self.pin_hover == Some(id);
                        if hot {
                            p.round_fill(slot, 5.0, alpha(th.text, 0.12));
                        }
                        let c = if hot { th.text } else { th.text_dim };
                        p.text(&p.fonts.glyphs_small, PIN_GLYPH, slot, alpha(c, e * e));
                    }
                }
            }
        });

        // Título y chincheta (acento si está fija).
        if e > 0.05 {
            p.text(&p.fonts.ui_bold, "Carpetas", rect(area.left + SIDE + 4.0, area.top + 4.0, area.right - 34.0, area.top + 28.0), alpha(th.text_dim, e * e));
        }
        // En la franja de íconos no hay chincheta: aparece al expandirse.
        if e > 0.05 {
            let btn = self.toggle_rect();
            if self.toggle_hot {
                p.round_fill(btn, 5.0, alpha(th.hover, e * e));
            }
            let (glyph, color) = if self.pinned { (PIN_GLYPH, th.accent) } else { ("\u{E718}", if self.toggle_hot { th.text } else { th.text_dim }) };
            p.text(&p.fonts.glyphs_small, glyph, btn, alpha(color, e * e));
        }

        // Contorno gris, como el de los paneles que no están activos.
        p.round_stroke(area, 8.0, th.border);

        // Barras de desplazamiento.
        self.htrack = rect(0.0, 0.0, 0.0, 0.0);
        self.hthumb = rect(0.0, 0.0, 0.0, 0.0);
        self.vtrack = rect(0.0, 0.0, 0.0, 0.0);
        self.vthumb = rect(0.0, 0.0, 0.0, 0.0);
        let thumb = alpha(th.text, 0.30);
        if need_h {
            let track = rect(area.left + 6.0, area.bottom - BAR - 4.0, area.left + view_w - 6.0, area.bottom - 4.0);
            let len = ((track.right - track.left) * view_w / content_w).clamp(28.0, track.right - track.left);
            let span = (track.right - track.left) - len;
            let left = track.left + span * (self.hscroll / (content_w - view_w).max(1.0));
            self.htrack = track;
            self.hthumb = rect(left, track.top, left + len, track.bottom);
            p.round_fill(track, BAR / 2.0, alpha(th.text, 0.06));
            p.round_fill(self.hthumb, BAR / 2.0, thumb);
        }
        if need_v {
            let track = rect(area.right - BAR - 4.0, area.top + 6.0, area.right - 4.0, area.top + view_h - 6.0);
            let h = track.bottom - track.top;
            let len = (h * view_h / total_h).clamp(28.0, h);
            let top = track.top + (h - len) * (self.scroll / (total_h - view_h).max(1.0));
            self.vtrack = track;
            self.vthumb = rect(track.left, top, track.right, top + len);
            p.round_fill(track, BAR / 2.0, alpha(th.text, 0.06));
            p.round_fill(self.vthumb, BAR / 2.0, thumb);
        }
    }
}

/// Subcarpetas ordenadas por nombre (sin las ocultas, salvo que se muestren).
fn subfolders(path: &Path, show_hidden: bool) -> Vec<(String, PathBuf)> {
    use std::os::windows::fs::MetadataExt;
    let Ok(reader) = std::fs::read_dir(path) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = reader
        .flatten()
        .filter(|e| {
            e.metadata().is_ok_and(|m| {
                let a = m.file_attributes();
                m.is_dir() && (show_hidden || a & 0x2 == 0)
            })
        })
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    out.sort_by(|a, b| crate::format::natural_cmp(&a.0.to_lowercase(), &b.0.to_lowercase()));
    out
}

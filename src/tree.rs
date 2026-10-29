//! Árbol de carpetas lateral (F-3.5, experimental). Solo se compila con la feature `arbol`:
//! `scripts\build.cmd build --release --features arbol`.
//!
//! Panel a la izquierda de la ventana que navega el panel activo. Arranca con la carpeta del
//! usuario y las unidades; las subcarpetas se leen al expandir un nodo, en un hilo aparte (una
//! unidad de red lenta no congela la ventana), y el resultado vuelve con `WM_APP_TREE`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::fs::Location;
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;

/// Terminó de leerse una carpeta del árbol.
pub const WM_APP_TREE: u32 = WM_APP + 20;

/// Ancho inicial del panel; se puede cambiar arrastrando su borde derecho.
pub const WIDTH: f32 = 240.0;
pub const MIN_W: f32 = 160.0;
pub const MAX_W: f32 = 560.0;
/// Grosor de las barras de desplazamiento y lugar que ocupan al pie / al costado.
const BAR: f32 = 6.0;
const BAR_ROOM: f32 = 12.0;
const ROW_H: f32 = 32.0;
const INDENT: f32 = 18.0;
/// Margen a los costados de las filas.
const SIDE: f32 = 8.0;
/// Margen entre el borde de arriba y la primera fila.
/// Franja de arriba: título y chincheta; las filas arrancan debajo.
const TOP_PAD: f32 = 32.0;

/// Qué es una fila, para su ícono y lo que abre.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Folder,
    /// Carpeta del usuario, fijada arriba como en el Explorador.
    Pinned,
    /// OneDrive.
    Cloud,
    /// Inicio y Galería: lugares del Shell sin subcarpetas en el árbol (posición en `PLACES`).
    Place,
    Drive,
    /// "Este equipo": no es una ruta.
    Computer,
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

/// Lugares fijos de arriba: nombre y nombre de análisis del Shell (Inicio, Galería).
const PLACES: [(&str, &str); 2] = [
    ("Inicio", "::{679F85CB-0220-4080-B29B-5540CC05AAB6}"),
    ("Galería", "::{E88865EA-0E1C-4E20-9AA6-EDCD0212C87C}"),
];
/// Íconos de `PLACES`.
const PLACE_ICONS: [(&str, (f32, f32, f32)); 2] = [("\u{E80F}", (0.95, 0.60, 0.30)), ("\u{E91B}", (0.35, 0.60, 0.95))];

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
    depth: usize,
    /// None: sin leer todavía. Some(vacío): no tiene subcarpetas.
    children: Option<Vec<usize>>,
    expanded: bool,
    loading: bool,
}

/// Qué tocó un clic.
pub enum TreeHit {
    /// Expandir o contraer el nodo.
    Toggle(usize),
    /// Abrir la carpeta en el panel activo.
    Open(Location),
    /// La chincheta: fijar el árbol o dejar que se oculte solo.
    Pin,
    Nothing,
}

pub struct Tree {
    pub visible: bool,
    /// Fijo al lado de los paneles; si no, se oculta y aparece encima al tocar el borde izquierdo.
    pub pinned: bool,
    /// Sin fijar: se está mostrando encima de los paneles.
    pub open: bool,
    pin_hot: bool,
    nodes: Vec<Node>,
    roots: Vec<usize>,
    scroll: f32,
    /// Desplazamiento horizontal.
    hscroll: f32,
    /// Ancho del panel (cambiable).
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
    tx: Sender<(usize, Vec<(String, PathBuf)>)>,
    rx: Receiver<(usize, Vec<(String, PathBuf)>)>,
}

impl Tree {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let mut t = Self { visible: false, pinned: true, open: false, pin_hot: false, nodes: Vec::new(), roots: Vec::new(), scroll: 0.0,
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
            tx,
            rx,
        };
        // Como el panel de navegación del Explorador: Inicio, Galería y OneDrive.
        for (i, (name, parsing)) in PLACES.iter().enumerate() {
            if crate::fs::shell_place_exists(parsing) {
                t.add_root(Kind::Place, PathBuf::from(parsing), (*name).to_string(), i);
                let id = t.nodes.len() - 1;
                t.nodes[id].children = Some(Vec::new());
            }
        }
        if let Some(cloud) = std::env::var_os("OneDrive").map(PathBuf::from).filter(|p| p.is_dir()) {
            t.add_root(Kind::Cloud, cloud, "OneDrive".to_string(), 0);
        }
        t.add_root(Kind::Separator, PathBuf::new(), String::new(), 0);
        // Acceso rápido: carpetas del usuario y, después, las ancladas o frecuentes del Explorador.
        let before = t.roots.len();
        let mut known = Vec::new();
        for (i, (name, ..)) in PINNED.iter().enumerate() {
            if let Some(path) = crate::fs::user_folder(i) {
                known.push(path.clone());
                t.add_root(Kind::Pinned, path, (*name).to_string(), i);
            }
        }
        for (name, path) in crate::fs::quick_access().into_iter().filter(|(_, p)| !known.iter().any(|k| crate::fs::same_path(k, p))).take(12) {
            t.add_root(Kind::Folder, path, name, 0);
        }
        if t.roots.len() > before {
            t.add_root(Kind::Separator, PathBuf::new(), String::new(), 0);
        }
        // "Este equipo", ya abierto, con sus unidades.
        t.add_root(Kind::Computer, PathBuf::new(), "Este equipo".to_string(), 0);
        let computer = t.roots[t.roots.len() - 1];
        let drives = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let mut ids = Vec::new();
        for i in 0..26u32 {
            if drives & (1 << i) != 0 {
                let letter = (b'A' + i as u8) as char;
                let label = crate::fs::drive_label(&format!("{letter}:"));
                t.nodes.push(Node {
                    kind: Kind::Drive,
                    path: PathBuf::from(format!("{letter}:\\")),
                    name: label,
                    pin: 0,
                    depth: 1,
                    children: None,
                    expanded: false,
                    loading: false,
                    text_w: 0.0,
                });
                ids.push(t.nodes.len() - 1);
            }
        }
        t.nodes[computer].children = Some(ids);
        t.nodes[computer].expanded = true;
        // Como en el Explorador: Red y, si hay WSL, Linux. Sus hijos se leen al expandir.
        t.add_root(Kind::Network, PathBuf::from(crate::fs::NETWORK_PLACE), "Red".to_string(), 0);
        if crate::fs::shell_place_exists(crate::fs::LINUX_PLACE) {
            t.add_root(Kind::Linux, PathBuf::from(crate::fs::LINUX_PLACE), "Linux".to_string(), 0);
        }
        t
    }

    fn add_root(&mut self, kind: Kind, path: PathBuf, name: String, pin: usize) {
        self.nodes.push(Node { kind, path, name, pin, depth: 0, children: None, expanded: false, loading: false, text_w: 0.0 });
        self.roots.push(self.nodes.len() - 1);
    }

    /// Nodos a la vista, en orden (los hijos de los expandidos, recursivamente).
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
        let mut out = Vec::new();
        walk(self, &self.roots, &mut out);
        out
    }

    /// Expande o contrae; la primera vez lee las subcarpetas en otro hilo.
    pub fn toggle(&mut self, hwnd: HWND, id: usize, show_hidden: bool) {
        let n = &mut self.nodes[id];
        if n.kind == Kind::Separator {
            return;
        }
        n.expanded = !n.expanded;
        if n.expanded && n.children.is_none() && !n.loading {
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
    }

    /// Aplica las lecturas terminadas.
    pub fn on_loaded(&mut self) {
        while let Ok((id, dirs)) = self.rx.try_recv() {
            let depth = self.nodes[id].depth + 1;
            let mut ids = Vec::with_capacity(dirs.len());
            for (name, path) in dirs {
                self.nodes.push(Node { kind: Kind::Folder, path, name, pin: 0, depth, children: None, expanded: false, loading: false, text_w: 0.0 });
                ids.push(self.nodes.len() - 1);
            }
            let n = &mut self.nodes[id];
            n.children = Some(ids);
            n.loading = false;
        }
    }

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
        if contains(&self.pin_rect(), x, y) {
            return TreeHit::Pin;
        }
        let Some(id) = self.node_at(y) else { return TreeHit::Nothing };
        let n = &self.nodes[id];
        let chevron_right = self.area.left + SIDE + 2.0 + n.depth as f32 * INDENT + 18.0 - self.hscroll;
        match n.kind {
            Kind::Separator => TreeHit::Nothing,
            _ if x < chevron_right => TreeHit::Toggle(id),
            Kind::Computer => TreeHit::Open(Location::Computer),
            Kind::Network | Kind::Linux | Kind::Place => {
                TreeHit::Open(Location::Shell { parsing: n.path.to_string_lossy().into_owned(), name: n.name.clone(), parent: None })
            }
            _ => TreeHit::Open(Location::Path(n.path.clone())),
        }
    }

    /// Se ve ahora: encendido y, si no está fijo, desplegado sobre los paneles.
    pub fn shown(&self) -> bool {
        self.visible && (self.pinned || self.open)
    }

    fn pin_rect(&self) -> Rect {
        rect(self.area.right - 30.0, self.area.top + 4.0, self.area.right - 6.0, self.area.top + 28.0)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.shown() && contains(&self.area, x, y)
    }

    pub fn on_mouse_move(&mut self, x: f32, y: f32) -> bool {
        let inside = self.shown() && contains(&self.area, x, y);
        let hover = if inside && y >= self.area.top + TOP_PAD { self.node_at(y) } else { None };
        let pin_hot = inside && contains(&self.pin_rect(), x, y);
        let changed = hover != self.hover || pin_hot != self.pin_hot;
        self.hover = hover;
        self.pin_hot = pin_hot;
        changed
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
        (self.content_w - self.view_w).max(0.0)
    }

    /// El borde derecho, donde se agarra para cambiar el ancho.
    pub fn edge_at(&self, x: f32, y: f32) -> bool {
        self.shown() && y >= self.area.top && y <= self.area.bottom && x >= self.area.right - 3.0 && x <= self.area.right + crate::ui::GAP + 1.0
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

    /// `current`: carpeta del panel activo, resaltada si está a la vista.
    pub fn draw(&mut self, p: &Painter, area: Rect, current: Option<&Path>) {
        self.area = area;
        let th = p.theme;
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
        // Qué barras hacen falta (una puede hacer necesaria la otra).
        let total_h = rows.len() as f32 * ROW_H + TOP_PAD;
        let (full_w, full_h) = (area.right - area.left, area.bottom - area.top);
        let mut view_w = full_w;
        let mut view_h = full_h;
        let mut need_v = total_h > view_h;
        if need_v {
            view_w -= BAR_ROOM;
        }
        let need_h = content_w > view_w;
        if need_h {
            view_h -= BAR_ROOM;
            if !need_v && total_h > view_h {
                need_v = true;
                view_w -= BAR_ROOM;
            }
        }
        self.content_w = content_w;
        self.view_w = view_w;
        self.view_h = view_h;
        self.scroll = self.scroll.clamp(0.0, (total_h - view_h).max(0.0));
        self.hscroll = self.hscroll.clamp(0.0, (content_w - view_w).max(0.0));
        let hs = self.hscroll;
        let folder = rgb(1.0, 0.79, 0.28);
        let body = rect(area.left + 1.0, area.top + TOP_PAD - 2.0, area.left + view_w - 1.0, area.top + view_h);
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
                let is_current = !matches!(n.kind, Kind::Computer | Kind::Network | Kind::Linux | Kind::Place) && current.is_some_and(|c| crate::fs::same_path(c, &n.path));
                if is_current {
                    p.round_fill(row, 5.0, alpha(th.text, 0.11));
                } else if self.hover == Some(id) {
                    p.round_fill(row, 5.0, alpha(th.text, 0.06));
                }
                let x = area.left + SIDE + 2.0 + n.depth as f32 * INDENT - hs;
                // Chevron: derecha (contraído), abajo (expandido); nada si no tiene subcarpetas.
                let has_children = n.children.as_ref().is_none_or(|c| !c.is_empty());
                if has_children {
                    let glyph = if n.loading { "\u{E712}" } else if n.expanded { "\u{E70D}" } else { "\u{E76C}" };
                    p.text(&p.fonts.glyphs_small, glyph, rect(x, y, x + 16.0, y + ROW_H), th.text_dim);
                }
                let (icon, color) = match n.kind {
                    Kind::Cloud => ("\u{E753}", rgb(0.30, 0.65, 0.95)),
                    Kind::Place => {
                        let (glyph, (r, g, b)) = PLACE_ICONS[n.pin];
                        (glyph, rgb(r, g, b))
                    }
                    Kind::Computer => ("\u{E7F4}", rgb(0.30, 0.65, 0.95)),
                    Kind::Drive => ("\u{EDA2}", th.text_dim),
                    Kind::Network => ("\u{E839}", rgb(0.30, 0.65, 0.95)),
                    Kind::Linux => ("\u{E756}", th.text_dim),
                    Kind::Pinned => {
                        let (_, glyph, (r, g, b)) = PINNED[n.pin];
                        (glyph, rgb(r, g, b))
                    }
                    _ => ("\u{E8D5}", folder),
                };
                p.text(&p.fonts.glyphs, icon, rect(x + 20.0, y, x + 42.0, y + ROW_H), color);
                p.text(&p.fonts.ui, &n.name, rect(x + 48.0, y, x + 48.0 + n.text_w + 12.0, y + ROW_H), th.text);
            }
        });

        // Título y chincheta (acento si está fijo).
        p.text(&p.fonts.ui_bold, "Carpetas", rect(area.left + SIDE + 4.0, area.top + 4.0, area.right - 34.0, area.top + 28.0), th.text_dim);
        let pin = self.pin_rect();
        if self.pin_hot {
            p.round_fill(pin, 5.0, th.hover);
        }
        let (glyph, color) = if self.pinned { ("\u{E840}", th.accent) } else { ("\u{E718}", if self.pin_hot { th.text } else { th.text_dim }) };
        p.text(&p.fonts.glyphs_small, glyph, pin, color);

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

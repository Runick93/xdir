//! Un panel: tira de pestañas, barra de navegación con dirección y la lista de la pestaña activa.
//! Guarda las áreas del último frame para resolver clics.

use crate::favorites::{Fav, Favorites};
use crate::fs::Location;
use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::tab::Tab;
use crate::theme::alpha;

pub const TABSTRIP_H: f32 = 34.0;
pub const NAVBAR_H: f32 = 42.0;
pub const FAVBAR_H: f32 = 30.0;
/// Franja al pie de cada panel, con el botón del tamaño de la vista a la derecha.
const FOOTER_H: f32 = 26.0;
const VIEW_BTN_W: f32 = 32.0;
const PANE_RADIUS: f32 = 8.0;
/// Inclinación de los lados de las pestañas.
const TAB_SLANT: f32 = 10.0;
/// Radio de las puntas de arriba de las pestañas y de su encuentro con el borde.
const TAB_RADIUS: f32 = 6.0;
const TAB_FOOT_RADIUS: f32 = 4.0;
/// Separación entre el borde izquierdo del panel y el pie de la primera pestaña: deja lugar a la
/// esquina del panel, para que su lado izquierdo sea simétrico al derecho.
const TAB_FIRST_INSET: f32 = 10.0;
/// Ancho reservado a la derecha de la barra de dirección en edición para la cruz que borra el texto.
pub const ADDRESS_CLEAR_W: f32 = 26.0;
/// Lugar del ícono de monitor delante de "Este equipo" en la barra de dirección.
const THIS_PC_ICON_W: f32 = 22.0;

/// Contorno de una pestaña sin base: (abajo izquierda, arriba izquierda, arriba derecha,
/// abajo derecha). `left_slant`: inclinación del lado izquierdo.
fn tab_points(r: Rect, left_slant: f32) -> Vec<(f32, f32, f32)> {
    let (l, t, rr, b) = (r.left + 0.5, r.top + 0.5, r.right - 0.5, r.bottom);
    vec![(l, b, 0.0), (l + left_slant, t, TAB_RADIUS), (rr - TAB_SLANT, t, TAB_RADIUS), (rr, b, 0.0)]
}

/// Contorno del panel como una sola figura: el cuerpo redondeado con la pestaña activa
/// integrada en el borde de arriba.
fn outline(body: Rect, tab: Option<Rect>) -> Vec<(f32, f32, f32)> {
    let h = 0.75; // medio trazo hacia adentro para que el borde quede nítido
    let (l, t, r, b) = (body.left + h, body.top + h, body.right - h, body.bottom - h);
    let Some(tab) = tab else {
        return vec![(l, b, PANE_RADIUS), (l, t, PANE_RADIUS), (r, t, PANE_RADIUS), (r, b, PANE_RADIUS)];
    };
    let (tl, tr, tt) = (tab.left + h, tab.right - h, tab.top + h);
    let mut pts = vec![(l, b, PANE_RADIUS)];
    pts.push((l, t, PANE_RADIUS));
    pts.push((tl, t, TAB_FOOT_RADIUS));
    pts.push((tl + TAB_SLANT, tt, TAB_RADIUS));
    pts.push((tr - TAB_SLANT, tt, TAB_RADIUS));
    pts.push((tr, t, TAB_FOOT_RADIUS));
    pts.push((r, t, PANE_RADIUS));
    pts.push((r, b, PANE_RADIUS));
    pts
}
const STAR_W: f32 = 28.0;
const TERM_W: f32 = 32.0;
const FAV_MAX_W: f32 = 170.0;
const OVERFLOW_W: f32 = 28.0;
const BTN: f32 = 30.0;
/// Botón «Devolver» de la Papelera (la barra de dirección se achica esto más el margen).
const RESTORE_W: f32 = 102.0;
const TAB_MIN_W: f32 = 90.0;
const TAB_MAX_W: f32 = 220.0;
const CHEVRON_W: f32 = 18.0;
const MAXIMIZE_W: f32 = 32.0;
/// Ancho de la caja de búsqueda de la barra: una parte del panel, entre estos límites.
const SEARCH_MIN_W: f32 = 150.0;
const SEARCH_MAX_W: f32 = 280.0;
/// Lugar a la derecha de la caja para el indicador de búsqueda en curso y su porcentaje.
const SEARCH_PROGRESS_W: f32 = 58.0;
/// Pestañas cerradas que se pueden reabrir con Ctrl+Shift+T.
const CLOSED_MAX: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Back,
    Forward,
    Up,
    Refresh,
}

/// Glifos de Segoe Fluent Icons / MDL2 Assets.
const BUTTONS: [(Button, &str); 4] = [
    (Button::Back, "\u{E72B}"),
    (Button::Forward, "\u{E72A}"),
    (Button::Up, "\u{E74A}"),
    (Button::Refresh, "\u{E72C}"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneHit {
    None,
    Tab(usize),
    TabClose(usize),
    NewTab,
    /// Botón de maximizar / restaurar el panel, a la derecha de las pestañas.
    Maximize,
    /// Parte vacía de la tira de pestañas (doble clic maximiza).
    TabStrip,
    Button(Button),
    Crumb(usize),
    /// Flecha entre dos tramos de la barra de dirección: abre las subcarpetas del tramo anterior.
    Chevron(usize),
    Address,
    /// Cruz de la barra de dirección en edición: borra lo escrito.
    AddressClear,
    /// Estrella de la barra de dirección: agregar o quitar de favoritos.
    Star,
    /// Botón de consola de la barra de dirección.
    Terminal,
    /// Caja de búsqueda de la barra: clic para escribir.
    Search,
    /// Cruz de la caja de búsqueda: termina la búsqueda.
    SearchClear,
    /// Favorito de la barra (índice en la raíz).
    Fav(usize),
    /// Botón "»" con los favoritos que no entran.
    FavOverflow,
    /// Parte vacía de la barra de favoritos.
    FavBar,
    /// Botón del pie del panel: pasa a la vista siguiente (como Ctrl+rueda).
    ViewButton,
    NewFolderButton,
    /// Botón «Devolver» junto a la barra de dirección (solo en la Papelera).
    Restore,
    List,
}

/// Lo que muestra la caja de búsqueda de la barra.
pub struct SearchView<'a> {
    /// Lo buscado (vacío: sin búsqueda).
    pub text: &'a str,
    /// Hay un control de edición encima del texto.
    pub editing: bool,
    /// Búsqueda en curso: fracción estimada del recorrido (0 a 1).
    pub running: Option<f32>,
    /// Se puede buscar en este lugar (no en carpetas virtuales).
    pub enabled: bool,
}

/// Dónde caería algo que se suelta sobre la barra de favoritos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FavDrop {
    /// Antes del favorito `i` de la raíz (`i == len`: al final).
    Before(usize),
    /// Dentro de la carpeta de favoritos `i`.
    Into(usize),
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
    closed: Vec<Location>,
    pub rect: Rect,
    tab_rects: Vec<(Rect, Rect)>,
    /// Lugar que le toca a cada pestaña (sin animar): con él se decide dónde cae una arrastrada.
    tab_slots: Vec<Rect>,
    /// Dónde se dibuja cada pestaña ahora (id, izquierda): se desliza hacia su lugar.
    tab_x: Vec<(usize, f32)>,
    /// Pestaña que se está arrastrando en este panel (índice, izquierda): sigue al mouse.
    pub drag_tab: Option<(usize, f32)>,
    /// Pestaña arrastrada que flota lejos de las tiras: su lugar queda vacío.
    pub drag_hidden: Option<usize>,
    /// Dónde está la pestaña arrastrada dentro de la tira en este cuadro (la flotante se acopla ahí).
    pub drag_rect: Option<Rect>,
    /// Segundos desde el cuadro anterior, para animar las pestañas.
    pub frame_dt: f32,
    /// Alguna pestaña todavía se está moviendo: hay que seguir dibujando.
    pub tab_busy: bool,
    new_tab: Rect,
    maximize: Rect,
    buttons: Vec<(Button, Rect)>,
    pub address: Rect,
    /// Cruz que borra el texto de la barra de dirección (vacía si no se está editando).
    address_clear: Rect,
    crumbs: Vec<(Rect, Location)>,
    /// Flechas de la barra de dirección, cada una con la carpeta cuyas subcarpetas muestra.
    chevrons: Vec<(Rect, Location)>,
    /// Flecha con su menú abierto (se dibuja hacia abajo y resaltada).
    pub open_chevron: Option<usize>,
    star: Rect,
    terminal: Rect,
    /// Caja de búsqueda de la barra y su cruz (vacía si no hay nada escrito).
    search: Rect,
    search_clear: Rect,
    /// Favoritos visibles de la raíz: (área, índice).
    favs: Vec<(Rect, usize)>,
    /// Botón de desborde y el primer favorito que no entró.
    fav_overflow: Option<(Rect, usize)>,
    pub favbar: Rect,
    view_button: Rect,
    new_folder_button: Rect,
    /// Con la Papelera a la vista, el botón «Devolver» (encendido si hay selección); lo pone la
    /// `App` antes de dibujar.
    pub restore: Option<bool>,
    restore_button: Rect,
    pub list: Rect,
}

impl Pane {
    pub fn new(tabs: Vec<Tab>, active: usize) -> Self {
        let zero = rect(0.0, 0.0, 0.0, 0.0);
        let active = active.min(tabs.len().saturating_sub(1));
        Self {
            tabs,
            active,
            closed: Vec::new(),
            rect: zero,
            tab_rects: Vec::new(),
            tab_slots: Vec::new(),
            tab_x: Vec::new(),
            drag_tab: None,
            drag_hidden: None,
            drag_rect: None,
            frame_dt: 1.0 / 60.0,
            tab_busy: false,
            new_tab: zero,
            maximize: zero,
            buttons: Vec::new(),
            address: zero,
            address_clear: zero,
            crumbs: Vec::new(),
            chevrons: Vec::new(),
            open_chevron: None,
            star: zero,
            terminal: zero,
            search: zero,
            search_clear: zero,
            favs: Vec::new(),
            fav_overflow: None,
            favbar: zero,
            view_button: zero,
            new_folder_button: zero,
            restore: None,
            restore_button: zero,
            list: zero,
        }
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Inserta una pestaña a la derecha de la activa. Devuelve su posición.
    pub fn add_tab(&mut self, tab: Tab, activate: bool) -> usize {
        let at = (self.active + 1).min(self.tabs.len());
        if activate {
            self.leave_active();
        }
        self.tabs.insert(at, tab);
        if activate {
            self.active = at;
        }
        at
    }

    /// Cierra una pestaña; la última no se cierra. Devuelve si se cerró.
    pub fn close_tab(&mut self, i: usize) -> bool {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return false;
        }
        let tab = self.tabs.remove(i);
        self.closed.push(tab.location.clone());
        if self.closed.len() > CLOSED_MAX {
            self.closed.remove(0);
        }
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        true
    }

    pub fn pop_closed(&mut self) -> Option<Location> {
        self.closed.pop()
    }

    pub fn select_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            if i != self.active {
                self.leave_active();
            }
            self.active = i;
        }
    }

    /// La pestaña activa deja de serlo: su desplegable de filtros se cierra (no vuelve a
    /// aparecer al volver a ella).
    fn leave_active(&mut self) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.list.close_filter_menu();
        }
    }

    pub fn cycle_tab(&mut self, forward: bool) {
        self.leave_active();
        let n = self.tabs.len();
        self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
    }

    /// Mueve la pestaña `from` a `to` (arrastre). La activa sigue siendo la misma.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let active_id = self.tab().id;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
    }

    /// Reemplaza todas las pestañas (al abrir un grupo). Las que había van a las cerradas.
    pub fn replace_tabs(&mut self, tabs: Vec<Tab>, active: usize) {
        if tabs.is_empty() {
            return;
        }
        for old in std::mem::replace(&mut self.tabs, tabs) {
            self.closed.push(old.location);
        }
        let excess = self.closed.len().saturating_sub(CLOSED_MAX);
        self.closed.drain(..excess);
        self.active = active.min(self.tabs.len() - 1);
    }

    /// Saca una pestaña para llevarla a otro panel (no va al historial de cerradas). El panel
    /// puede quedar vacío: quien llama tiene que agregarle una.
    pub fn take_tab(&mut self, i: usize) -> Tab {
        let mut tab = self.tabs.remove(i);
        // El desplegable de filtros quedaría abierto en el otro panel.
        tab.list.close_filter_menu();
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        tab
    }

    /// Inserta una pestaña traída de otro panel en la posición `at` y la activa.
    pub fn insert_tab(&mut self, at: usize, tab: Tab) {
        self.leave_active();
        let at = at.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active = at;
    }

    /// Posición donde entraría una pestaña soltada en `x` sobre la tira: antes de la primera
    /// pestaña cuyo centro queda a la derecha.
    pub fn insert_index_at(&self, x: f32) -> usize {
        self.tab_slots.iter().position(|r| x < (r.left + r.right) / 2.0).unwrap_or(self.tabs.len())
    }

    /// Marca de destino al arrastrar una pestaña desde otro panel: contorno de acento y una
    /// barra en la tira donde caería.
    pub fn draw_tab_drop(&self, p: &dyn Painter, at: usize) {
        let th = p.theme();
        let body = rect(self.rect.left, self.rect.top + TABSTRIP_H, self.rect.right, self.rect.bottom);
        p.round_fill(body, PANE_RADIUS, alpha(th.accent, 0.06));
        p.round_stroke_w(body, PANE_RADIUS, th.accent, 2.0);
        let x = match (self.tab_slots.get(at), self.tab_slots.last()) {
            (Some(r), _) => r.left + TAB_SLANT / 2.0,
            (None, Some(r)) => r.right - TAB_SLANT / 2.0,
            (None, None) => self.rect.left + TAB_FIRST_INSET,
        };
        let (top, bottom) = (self.rect.top + 6.0, self.rect.top + TABSTRIP_H - 4.0);
        p.round_fill(rect(x - 1.5, top, x + 1.5, bottom), 1.5, th.accent);
    }

    /// Pestaña bajo la coordenada x (para arrastrar y soltar dentro de la tira).
    pub fn tab_index_at(&self, x: f32) -> Option<usize> {
        let last = self.tab_slots.len().checked_sub(1)?;
        Some(self.tab_slots.iter().position(|r| x < r.right).unwrap_or(last))
    }

    /// Dónde está dibujada la pestaña `i` (izquierda y ancho), para agarrarla.
    pub fn tab_grab(&self, i: usize, x: f32) -> f32 {
        self.tab_rects.get(i).map_or(0.0, |(r, _)| (x - r.left).clamp(0.0, r.right - r.left))
    }

    /// Ancho de una pestaña.
    pub fn tab_width(&self) -> f32 {
        self.tab_slots.first().map_or(TAB_MIN_W, |r| r.right - r.left)
    }

    /// Dónde se dibujó un botón de navegación en el último cuadro.
    pub fn button_rect(&self, b: Button) -> Option<Rect> {
        self.buttons.iter().find(|(x, _)| *x == b).map(|(_, r)| *r)
    }

    pub fn hit(&self, x: f32, y: f32) -> PaneHit {
        if !contains(&self.rect, x, y) {
            return PaneHit::None;
        }
        for (i, (tab, close)) in self.tab_rects.iter().enumerate() {
            if contains(close, x, y) {
                return PaneHit::TabClose(i);
            }
            if contains(tab, x, y) {
                return PaneHit::Tab(i);
            }
        }
        if contains(&self.new_tab, x, y) {
            return PaneHit::NewTab;
        }
        if contains(&self.maximize, x, y) {
            return PaneHit::Maximize;
        }
        if y < self.rect.top + TABSTRIP_H {
            return PaneHit::TabStrip;
        }
        for (b, r) in &self.buttons {
            if contains(r, x, y) {
                return PaneHit::Button(*b);
            }
        }
        for (i, (r, _)) in self.crumbs.iter().enumerate() {
            if contains(r, x, y) {
                return PaneHit::Crumb(i);
            }
        }
        for (i, (r, _)) in self.chevrons.iter().enumerate() {
            if contains(r, x, y) {
                return PaneHit::Chevron(i);
            }
        }
        if contains(&self.terminal, x, y) {
            return PaneHit::Terminal;
        }
        if self.restore.is_some() && contains(&self.restore_button, x, y) {
            return PaneHit::Restore;
        }
        if contains(&self.search_clear, x, y) {
            return PaneHit::SearchClear;
        }
        if contains(&self.search, x, y) {
            return PaneHit::Search;
        }
        if contains(&self.star, x, y) {
            return PaneHit::Star;
        }
        if contains(&self.address_clear, x, y) {
            return PaneHit::AddressClear;
        }
        if contains(&self.address, x, y) {
            return PaneHit::Address;
        }
        for &(r, i) in &self.favs {
            if contains(&r, x, y) {
                return PaneHit::Fav(i);
            }
        }
        if let Some((r, _)) = self.fav_overflow {
            if contains(&r, x, y) {
                return PaneHit::FavOverflow;
            }
        }
        if contains(&self.favbar, x, y) {
            return PaneHit::FavBar;
        }
        if contains(&self.new_folder_button, x, y) {
            return PaneHit::NewFolderButton;
        }
        if contains(&self.view_button, x, y) {
            return PaneHit::ViewButton;
        }
        if contains(&self.list, x, y) {
            return PaneHit::List;
        }
        PaneHit::None
    }

    /// Dónde se dibujó el botón «Devolver» (solo con la Papelera a la vista).
    pub fn restore_button(&self) -> Option<Rect> {
        self.restore.map(|_| self.restore_button)
    }

    /// Flecha `i`: dónde está y de qué carpeta muestra las subcarpetas.
    pub fn chevron(&self, i: usize) -> Option<(Rect, Location)> {
        self.chevrons.get(i).cloned()
    }

    pub fn crumb(&self, i: usize) -> Option<Location> {
        self.crumbs.get(i).map(|(_, l)| l.clone())
    }

    /// Área de un favorito de la barra (para ubicar menús y el renombrado).
    pub fn fav_rect(&self, i: usize) -> Option<Rect> {
        self.favs.iter().find(|(_, j)| *j == i).map(|(r, _)| *r)
    }

    /// Botón de consola (para ubicar su menú).
    pub fn terminal_rect(&self) -> Rect {
        self.terminal
    }

    pub fn overflow(&self) -> Option<(Rect, usize)> {
        self.fav_overflow
    }

    /// Dónde caería algo soltado en `x` sobre la barra: dentro de una carpeta de favoritos si
    /// se suelta en su centro, si no entre favoritos.
    pub fn fav_drop_at(&self, favs: &Favorites, x: f32) -> FavDrop {
        for &(r, i) in &self.favs {
            if x < r.right {
                let w = r.right - r.left;
                if favs.items[i].is_group() && x > r.left + w * 0.25 && x < r.right - w * 0.25 {
                    return FavDrop::Into(i);
                }
                return if x < r.left + w / 2.0 { FavDrop::Before(i) } else { FavDrop::Before(i + 1) };
            }
        }
        FavDrop::Before(self.favs.last().map(|&(_, i)| i + 1).unwrap_or(favs.items.len()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        p: &dyn Painter,
        area: Rect,
        active_pane: bool,
        hover: PaneHit,
        pressed: PaneHit,
        editing: bool,
        favs: Option<(&Favorites, Option<FavDrop>)>,
        terminal_open: bool,
        maximized: Option<bool>,
        search: SearchView,
    ) {
        self.rect = area;
        let th = p.theme();
        // Cuerpo del panel (debajo de las pestañas) y pestaña activa forman una sola pieza: un
        // contorno continuo que sube por la pestaña y vuelve a bajar, sin uniones.
        let body = rect(area.left, area.top + TABSTRIP_H, area.right, area.bottom);
        let reserved = if maximized.is_some() { MAXIMIZE_W } else { 0.0 };
        let layout = self.layout_tabs(area, reserved);
        let shown = self.animate_tabs(&layout);
        // Con la pestaña activa despegada, el contorno del panel queda liso.
        let docked = shown.get(self.active).copied().filter(|_| self.drag_hidden != Some(self.active));
        let outline = outline(body, docked);
        p.rounded_path(&outline, true, Some(th.surface), None);
        self.draw_tabs(p, area, &layout, &shown, hover);
        self.draw_maximize(p, area, maximized, hover);

        let nav = rect(area.left, area.top + TABSTRIP_H, area.right, area.top + TABSTRIP_H + NAVBAR_H);

        // Botones de navegación.
        self.buttons.clear();
        let top = nav.top + (NAVBAR_H - BTN) / 2.0;
        let mut x = nav.left + 6.0;
        let tab = &self.tabs[self.active];
        for (b, glyph) in BUTTONS {
            let r = rect(x, top, x + BTN, top + BTN);
            let enabled = match b {
                Button::Back => tab.can_back(),
                Button::Forward => tab.can_forward(),
                Button::Up => tab.can_up(),
                Button::Refresh => true,
            };
            if enabled && hover == PaneHit::Button(b) {
                let c = if pressed == hover { alpha(th.text, 0.12) } else { th.hover };
                p.round_fill(r, 6.0, c);
            }
            let color = if enabled { th.text } else { alpha(th.text_dim, 0.45) };
            p.text(FontId::Glyphs, glyph, r, color);
            self.buttons.push((b, r));
            x += BTN + 2.0;
        }

        // Caja de búsqueda, siempre a la vista a la derecha; la barra de dirección ocupa lo demás.
        let search_w = ((nav.right - nav.left) * 0.3).clamp(SEARCH_MIN_W, SEARCH_MAX_W);
        let search_box = rect(nav.right - 8.0 - search_w, top, nav.right - 8.0, top + BTN);
        self.draw_search(p, search_box, &search, hover);

        // «Devolver» en la Papelera, entre la barra de dirección y la búsqueda (la barra se achica).
        let mut addr_right = search_box.left - 6.0;
        self.restore_button = rect(0.0, 0.0, 0.0, 0.0);
        if let Some(enabled) = self.restore {
            let b = rect(addr_right - RESTORE_W, top, addr_right, top + BTN);
            let fill = if enabled && hover == PaneHit::Restore {
                if pressed == hover { alpha(th.text, 0.12) } else { th.hover }
            } else {
                th.surface_alt
            };
            p.round_fill(b, 6.0, fill);
            p.round_stroke(b, 6.0, th.border);
            let color = if enabled { th.text } else { alpha(th.text_dim, 0.45) };
            p.text(FontId::GlyphsSmall, "\u{E7A7}", rect(b.left + 6.0, b.top, b.left + 30.0, b.bottom), color);
            p.text(FontId::Ui, "Devolver", rect(b.left + 32.0, b.top, b.right - 6.0, b.bottom), color);
            self.restore_button = b;
            addr_right = b.left - 6.0;
        }

        // Barra de dirección.
        let addr = rect(x + 6.0, top, addr_right, top + BTN);
        self.address = addr;
        p.round_fill(addr, 6.0, th.surface_alt);
        self.star = rect(0.0, 0.0, 0.0, 0.0);
        self.terminal = rect(0.0, 0.0, 0.0, 0.0);
        self.address_clear = rect(0.0, 0.0, 0.0, 0.0);
        if editing {
            p.round_stroke(addr, 6.0, th.accent);
            self.crumbs.clear();
            // Cruz para borrar lo escrito, como en el Explorador.
            let clear = rect(addr.right - ADDRESS_CLEAR_W - 4.0, addr.top + 4.0, addr.right - 4.0, addr.bottom - 4.0);
            if hover == PaneHit::AddressClear {
                p.round_fill(clear, 4.0, alpha(th.text, 0.12));
            }
            p.text(FontId::GlyphsSmall, "\u{E711}", clear, th.text_dim);
            self.address_clear = clear;
        } else {
            let border = if hover == PaneHit::Address { th.text_dim } else { th.border };
            p.round_stroke(addr, 6.0, border);
            let star_w = if favs.is_some() { STAR_W } else { 0.0 };
            let crumbs_area = rect(addr.left, addr.top, addr.right - star_w - TERM_W - 4.0, addr.bottom);
            self.draw_crumbs(p, crumbs_area, hover);

            // Botón de consola: abre PowerShell o CMD en la carpeta (solo carpetas de disco).
            let term = rect(addr.right - star_w - TERM_W - 2.0, addr.top + 3.0, addr.right - star_w - 2.0, addr.bottom - 3.0);
            let enabled = matches!(self.tabs[self.active].location, Location::Path(_));
            if terminal_open {
                p.round_fill(term, 4.0, alpha(th.accent, 0.22));
                p.round_stroke(term, 4.0, th.accent);
            } else if enabled && hover == PaneHit::Terminal {
                p.round_fill(term, 4.0, th.hover);
            }
            let color = if !enabled {
                alpha(th.text_dim, 0.45)
            } else if terminal_open {
                th.accent
            } else {
                th.text_dim
            };
            p.text(FontId::UiCenter, ">_", term, color);
            self.terminal = if enabled { term } else { rect(0.0, 0.0, 0.0, 0.0) };
            if let Some((f, _)) = favs {
                // Estrella llena si la carpeta ya es favorita.
                let star = rect(addr.right - STAR_W, addr.top + 3.0, addr.right - 3.0, addr.bottom - 3.0);
                if hover == PaneHit::Star {
                    p.round_fill(star, 4.0, th.hover);
                }
                let is_fav = f.find(&self.tabs[self.active].location).is_some();
                let (glyph, color) = if is_fav { ("\u{E735}", th.accent) } else { ("\u{E734}", th.text_dim) };
                p.text(FontId::Glyphs, glyph, star, color);
                self.star = star;
            }
        }

        // Barra de favoritos.
        let mut top = nav.bottom;
        self.favs.clear();
        self.fav_overflow = None;
        self.favbar = rect(0.0, 0.0, 0.0, 0.0);
        if let Some((f, drop)) = favs {
            let bar = rect(area.left, nav.bottom, area.right, nav.bottom + FAVBAR_H);
            self.favbar = bar;
            self.draw_favs(p, bar, f, hover, drop);
            top = bar.bottom;
        }

        // Lista, un poco adentro del borde para no tapar las esquinas redondeadas.
        self.list = rect(area.left + 1.0, top, area.right - 1.0, area.bottom - FOOTER_H);
        // Línea gris que separa el final de la lista del pie con los botones.
        p.fill(rect(area.left + 1.0, area.bottom - FOOTER_H, area.right - 1.0, area.bottom - FOOTER_H + 1.0), th.border);
        // Botón del pie: cambia el tamaño de los íconos de este panel, vuelve a la primera vista al final.
        let vb = rect(area.right - PANE_RADIUS - VIEW_BTN_W, area.bottom - FOOTER_H + 2.0, area.right - PANE_RADIUS, area.bottom - 4.0);
        if hover == PaneHit::ViewButton {
            let c = if pressed == hover { alpha(th.text, 0.12) } else { th.hover };
            p.round_fill(vb, 6.0, c);
        }
        p.text(FontId::GlyphsSmall, "\u{E8A9}", vb, th.text_dim);
        self.view_button = vb;
        let nb = rect(vb.left - VIEW_BTN_W, vb.top, vb.left, vb.bottom);
        if hover == PaneHit::NewFolderButton {
            let c = if pressed == hover { alpha(th.text, 0.12) } else { th.hover };
            p.round_fill(nb, 6.0, c);
        }
        p.text(FontId::GlyphsSmall, "\u{E8F4}", nb, th.text_dim);
        self.new_folder_button = nb;
        // La lista necesita su carpeta para pedir las miniaturas por ruta.
        let tab = &mut self.tabs[self.active];
        tab.list.folder = match &tab.location {
            Location::Path(p) => Some(p.clone()),
            // Dentro de un archivo comprimido, la temporal: se ve lo que ya se extrajo.
            Location::Archive { file, nested, inner } => {
                let dir = crate::archive_nav::temps::global().dir_for(file, nested);
                Some(inner.split('/').filter(|s| !s.is_empty()).fold(dir, |d, s| d.join(s)))
            }
            _ => None,
        };
        tab.list.draw(p, self.list, active_pane);

        // Contorno encima de todo; de acento si es el panel activo.
        let stroke = if active_pane { (th.accent, 1.5) } else { (th.border, 1.0) };
        p.rounded_path(&outline, true, None, Some(stroke));
    }

    /// Posición de cada pestaña visible. La primera arranca un poco adentro del borde izquierdo
    /// del panel, y su lado izquierdo es inclinado como el derecho.
    /// `reserved`: ancho que queda libre a la derecha (botón de maximizar).
    fn layout_tabs(&self, area: Rect, reserved: f32) -> Vec<Rect> {
        let n = self.tabs.len() as f32;
        let right = area.right - 40.0 - reserved;
        let avail = right - area.left - TAB_FIRST_INSET;
        let width = (avail / n).clamp(TAB_MIN_W, TAB_MAX_W);
        let (top, bottom) = (area.top + 3.0, area.top + TABSTRIP_H);
        let mut x = area.left + TAB_FIRST_INSET;
        let mut out = Vec::new();
        for _ in &self.tabs {
            let r = rect(x, top, (x + width).min(right), bottom);
            if r.right - r.left < 30.0 {
                break;
            }
            out.push(r);
            x = r.right + 2.0;
        }
        out
    }

    /// Dónde se dibuja cada pestaña en este cuadro: cada una se desliza hacia su lugar y la que se
    /// arrastra sigue al mouse (sin salirse de la tira).
    fn animate_tabs(&mut self, layout: &[Rect]) -> Vec<Rect> {
        let ids: Vec<usize> = self.tabs.iter().map(|t| t.id).collect();
        self.tab_slots = layout.to_vec();
        self.tab_x.retain(|(id, _)| ids.contains(id));
        let k = 1.0 - (-self.frame_dt.clamp(0.0, 0.05) * 22.0).exp();
        let mut busy = false;
        let mut out = Vec::with_capacity(layout.len());
        for (i, &r) in layout.iter().enumerate() {
            let w = r.right - r.left;
            let id = ids[i];
            let mut x = self.tab_x.iter().find(|(t, _)| *t == id).map_or(r.left, |&(_, x)| x);
            match self.drag_tab.filter(|&(d, _)| d == i) {
                Some((_, left)) => {
                    let (min, max) = (layout[0].left, layout[layout.len() - 1].right - w);
                    x = left.clamp(min, max.max(min));
                }
                None => {
                    let d = r.left - x;
                    if d.abs() < 0.5 {
                        x = r.left;
                    } else {
                        x += d * k;
                        busy = true;
                    }
                }
            }
            match self.tab_x.iter_mut().find(|(t, _)| *t == id) {
                Some(e) => e.1 = x,
                None => self.tab_x.push((id, x)),
            }
            out.push(rect(x, r.top, x + w, r.bottom));
        }
        self.tab_busy = busy;
        self.drag_rect = self.drag_tab.and_then(|(d, _)| out.get(d).copied());
        out
    }

    /// La pestaña que se arrastra, flotando sobre todo: con `detach` en 1 es una pieza suelta con
    /// sombra; en 0 está acoplada a una tira y solo se ve su texto (la forma es la del panel).
    pub fn draw_ghost(p: &dyn Painter, r: Rect, title: &str, detach: f32) {
        let th = p.theme();
        if detach > 0.01 {
            let shadow = |a: f32| Color { r: 0.0, g: 0.0, b: 0.0, a };
            for (grow, a) in [(6.0, 0.05), (3.0, 0.08), (1.0, 0.10)] {
                p.round_fill(rect(r.left - grow, r.top - grow + 4.0, r.right + grow, r.bottom + grow + 4.0), 8.0 + grow, shadow(a * detach));
            }
            let shape = tab_points(r, TAB_SLANT);
            p.rounded_path(&shape, true, Some(alpha(th.surface_alt, detach)), Some((alpha(th.accent, detach), 1.5)));
        }
        p.text(FontId::Ui, title, rect(r.left + TAB_SLANT + 12.0, r.top, r.right - TAB_SLANT - 6.0, r.bottom), th.text);
    }

    /// Dibuja las pestañas inactivas (la activa ya es parte del contorno del panel) y los textos.
    /// `layout`: los lugares fijos; `shown`: dónde se dibuja cada una ahora. La arrastrada va al final.
    fn draw_tabs(&mut self, p: &dyn Painter, area: Rect, layout: &[Rect], shown: &[Rect], hover: PaneHit) {
        let th = p.theme();
        self.tab_rects.clear();
        self.tab_rects.resize(shown.len(), (rect(0.0, 0.0, 0.0, 0.0), rect(0.0, 0.0, 0.0, 0.0)));
        let dragged = self.drag_tab.map(|(d, _)| d).or(self.drag_hidden).filter(|&d| d < shown.len());
        if let Some(d) = self.drag_hidden.filter(|&d| d < shown.len()) {
            // Lugar vacío de la pestaña despegada: solo su silueta.
            p.rounded_path(&tab_points(layout[d], TAB_SLANT), false, None, Some((alpha(th.border, 0.5), 1.0)));
        }
        for i in 0..shown.len() {
            if Some(i) == dragged {
                self.tab_rects[i] = (shown[i], rect(0.0, 0.0, 0.0, 0.0));
                continue;
            }
            let r = shown[i];
            let tab = &self.tabs[i];
            let active = i == self.active;
            let hovered = dragged.is_none() && (hover == PaneHit::Tab(i) || hover == PaneHit::TabClose(i));
            let left_slant = TAB_SLANT;
            if !active {
                let shape = tab_points(r, left_slant);
                if hovered {
                    p.rounded_path(&shape, true, Some(th.hover), None);
                }
                p.rounded_path(&shape, false, None, Some((th.border, 1.0)));
            }

            let close = rect(r.right - TAB_SLANT - 22.0, r.top + 6.0, r.right - TAB_SLANT - 2.0, r.bottom - 6.0);
            let show_close = self.tabs.len() > 1 && (active || hovered);
            let text_right = if show_close { close.left - 2.0 } else { r.right - TAB_SLANT - 6.0 };
            let color = if active { th.text } else { th.text_dim };
            let text = rect(r.left + left_slant + 12.0, r.top, text_right, r.bottom);
            p.text(FontId::Ui, &tab.location.title(), text, color);
            if show_close {
                if hover == PaneHit::TabClose(i) {
                    p.round_fill(close, 4.0, alpha(th.text, 0.12));
                }
                p.text(FontId::GlyphsSmall, "\u{E711}", close, th.text_dim);
            }
            self.tab_rects[i] = (r, if show_close { close } else { rect(0.0, 0.0, 0.0, 0.0) });
        }

        let x = layout.last().map(|r| r.right + 2.0).unwrap_or(area.left);
        let (top, bottom) = (area.top + 3.0, area.top + TABSTRIP_H);
        let nt = rect(x + 4.0, top + 3.0, x + 32.0, bottom - 4.0);
        if hover == PaneHit::NewTab {
            p.round_fill(nt, 6.0, th.hover);
        }
        p.text(FontId::GlyphsSmall, "\u{E710}", nt, th.text_dim);
        self.new_tab = nt;
    }

    /// Área de texto de la caja de búsqueda: ahí va el control de edición (entre la lupa y lo de
    /// la derecha: progreso y cruz).
    pub fn search_text_rect(&self) -> Rect {
        let b = self.search;
        rect(b.left + 28.0, b.top + 5.0, (b.right - SEARCH_PROGRESS_W - 24.0).max(b.left + 60.0), b.bottom - 5.0)
    }

    fn draw_search(&mut self, p: &dyn Painter, field: Rect, s: &SearchView, hover: PaneHit) {
        let th = p.theme();
        self.search = if s.enabled { field } else { rect(0.0, 0.0, 0.0, 0.0) };
        self.search_clear = rect(0.0, 0.0, 0.0, 0.0);
        p.round_fill(field, 6.0, th.surface_alt);
        let border = if s.editing {
            th.accent
        } else if s.enabled && hover == PaneHit::Search {
            th.text_dim
        } else {
            th.border
        };
        p.round_stroke(field, 6.0, border);
        let dim = if s.enabled { th.text_dim } else { alpha(th.text_dim, 0.45) };
        let lens_color = if s.text.is_empty() { dim } else { th.accent };
        p.text(FontId::GlyphsSmall, "\u{E721}", rect(field.left + 4.0, field.top, field.left + 26.0, field.bottom), lens_color);
        let text = rect(field.left + 28.0, field.top, field.right - SEARCH_PROGRESS_W - 24.0, field.bottom);
        if !s.editing {
            if s.text.is_empty() {
                p.text(FontId::Ui, "Buscar", text, dim);
            } else {
                p.text(FontId::Ui, s.text, text, th.text);
            }
        }
        // En curso: indicador girando y porcentaje estimado.
        if let Some(done) = s.running {
            let r = rect(field.right - 24.0 - SEARCH_PROGRESS_W, field.top, field.right - 24.0, field.bottom);
            spinner(p, r.left + 9.0, (r.top + r.bottom) / 2.0, th.accent);
            // Es una estimación (el total no se conoce hasta terminar): no llega a 100 antes de tiempo.
            let pct = format!("{:.0} %", (done * 100.0).floor().min(99.0));
            p.text(FontId::UiRight, &pct, rect(r.left + 18.0, r.top, r.right - 2.0, r.bottom), th.text_dim);
        }
        if !s.text.is_empty() || s.editing {
            let clear = rect(field.right - 24.0, field.top + 4.0, field.right - 4.0, field.bottom - 4.0);
            if hover == PaneHit::SearchClear {
                p.round_fill(clear, 4.0, alpha(th.text, 0.12));
            }
            p.text(FontId::GlyphsSmall, "\u{E711}", clear, th.text_dim);
            self.search_clear = clear;
        }
    }

    /// Botón de maximizar (`Some(false)`) o restaurar (`Some(true)`) en el extremo derecho de la
    /// tira de pestañas; `None` si el layout tiene un solo panel.
    fn draw_maximize(&mut self, p: &dyn Painter, area: Rect, maximized: Option<bool>, hover: PaneHit) {
        let th = p.theme();
        self.maximize = rect(0.0, 0.0, 0.0, 0.0);
        let Some(on) = maximized else { return };
        let (top, bottom) = (area.top + 3.0, area.top + TABSTRIP_H);
        let r = rect(area.right - MAXIMIZE_W, top + 3.0, area.right - 4.0, bottom - 4.0);
        if on {
            p.round_fill(r, 6.0, alpha(th.accent, 0.22));
        } else if hover == PaneHit::Maximize {
            p.round_fill(r, 6.0, th.hover);
        }
        // Segoe: pantalla completa / volver a ventana.
        let (glyph, color) = if on { ("\u{E73F}", th.accent) } else { ("\u{E740}", th.text_dim) };
        p.text(FontId::GlyphsSmall, glyph, r, color);
        self.maximize = r;
    }

    fn draw_favs(&mut self, p: &dyn Painter, bar: Rect, f: &Favorites, hover: PaneHit, drop: Option<FavDrop>) {
        let th = p.theme();
        if f.items.is_empty() {
            let hint = "Arrastrá carpetas acá o tocá la estrella para agregar favoritos";
            p.text(FontId::Ui, hint, rect(bar.left + 12.0, bar.top, bar.right - 8.0, bar.bottom), th.text_dim);
            if drop.is_some() {
                p.fill(rect(bar.left + 6.0, bar.top + 5.0, bar.left + 8.0, bar.bottom - 5.0), th.accent);
            }
            return;
        }
        let folder_icon = crate::sys::file_system().folder_icon();
        let mut x = bar.left + 6.0;
        for (i, fav) in f.items.iter().enumerate() {
            let text_w = p.measure(FontId::Ui, fav.name()).min(FAV_MAX_W);
            let extra = if fav.is_group() { 14.0 } else { 0.0 };
            let w = 8.0 + 16.0 + 6.0 + text_w + extra + 10.0;
            // Si no entra (dejando lugar para "»"), el resto va al desborde.
            let limit = if i + 1 < f.items.len() { bar.right - OVERFLOW_W - 6.0 } else { bar.right - 6.0 };
            if x + w > limit {
                let r = rect(bar.right - OVERFLOW_W - 4.0, bar.top + 3.0, bar.right - 4.0, bar.bottom - 3.0);
                if hover == PaneHit::FavOverflow {
                    p.round_fill(r, 4.0, th.hover);
                }
                p.text(FontId::UiCenter, "»", r, th.text);
                self.fav_overflow = Some((r, i));
                break;
            }
            let r = rect(x, bar.top + 3.0, x + w, bar.bottom - 3.0);
            if drop == Some(FavDrop::Into(i)) {
                p.round_fill(r, 4.0, th.selection);
                p.round_stroke(r, 4.0, th.accent);
            } else if hover == PaneHit::Fav(i) {
                p.round_fill(r, 4.0, th.hover);
            }
            let iy = (r.top + r.bottom) / 2.0 - 8.0;
            match (fav, folder_icon) {
                // Carpeta de favoritos: ícono de carpeta en contorno, para distinguirla.
                (Fav::Group { .. }, _) | (_, None) => {
                    let g = rect(r.left + 6.0, r.top, r.left + 26.0, r.bottom);
                    let color = if fav.is_group() { th.accent } else { th.text_dim };
                    p.text(FontId::Glyphs, "\u{E8B7}", g, color);
                }
                (Fav::Link { .. }, Some(icon)) => p.shell_icon(icon, 0, r.left + 8.0, iy, 16.0),
            }
            let tx = r.left + 8.0 + 16.0 + 6.0;
            p.text(FontId::Ui, fav.name(), rect(tx, r.top, tx + text_w, r.bottom), th.text);
            if fav.is_group() {
                p.text(FontId::GlyphsSmall, "\u{E70D}", rect(tx + text_w, r.top, tx + text_w + 14.0, r.bottom), th.text_dim);
            }
            if drop == Some(FavDrop::Before(i)) {
                p.fill(rect(r.left - 3.0, r.top + 2.0, r.left - 1.0, r.bottom - 2.0), th.accent);
            }
            self.favs.push((r, i));
            x = r.right + 2.0;
        }
        if drop == Some(FavDrop::Before(f.items.len())) {
            p.fill(rect(x - 1.0, bar.top + 5.0, x + 1.0, bar.bottom - 5.0), th.accent);
        }
    }

    fn draw_crumbs(&mut self, p: &dyn Painter, addr: Rect, hover: PaneHit) {
        let th = p.theme();
        self.crumbs.clear();
        self.chevrons.clear();
        let all = self.tabs[self.active].location.crumbs();
        // El primer tramo ("Este equipo") lleva un ícono de monitor adelante.
        let widths: Vec<f32> = all
            .iter()
            .enumerate()
            .map(|(i, (label, _))| p.measure(FontId::Ui, label) + 16.0 + if i == 0 { THIS_PC_ICON_W } else { 0.0 })
            .collect();

        // Si no entran, se descartan los primeros (los más lejanos).
        let avail = addr.right - addr.left - 16.0;
        let mut first = 0;
        while first + 1 < all.len() && widths[first..].iter().sum::<f32>() + CHEVRON_W * (all.len() - first) as f32 > avail {
            first += 1;
        }

        let mut x = addr.left + 6.0;
        if first > 0 {
            p.text(FontId::Ui, "…", rect(x, addr.top, x + 16.0, addr.bottom), th.text_dim);
            x += 16.0;
        }
        for (i, (label, loc)) in all.iter().enumerate().skip(first) {
            if i > first || first > 0 {
                // La flecha que sigue al tramo anterior: sus subcarpetas.
                self.draw_chevron(p, rect(x, addr.top + 4.0, x + CHEVRON_W, addr.bottom - 4.0), all[i - 1].1.clone(), hover);
                x += CHEVRON_W;
            }
            let r = rect(x, addr.top + 4.0, (x + widths[i]).min(addr.right - 4.0), addr.bottom - 4.0);
            let idx = self.crumbs.len();
            let hot = hover == PaneHit::Crumb(idx);
            if hot {
                // Más marcado que el resaltado de las filas: es un botón chico y hay que verlo.
                p.round_fill(r, 4.0, alpha(th.text, 0.16));
            }
            let last = i + 1 == all.len();
            let color = if last || hot { th.text } else { th.text_dim };
            if i == 0 {
                let icon = rect(r.left + 4.0, r.top, r.left + 4.0 + THIS_PC_ICON_W, r.bottom);
                p.text(FontId::GlyphsSmall, "\u{E7F4}", icon, color);
                p.text(FontId::UiCenter, label, rect(r.left + THIS_PC_ICON_W, r.top, r.right, r.bottom), color);
            } else {
                p.text(FontId::UiCenter, label, r, color);
            }
            self.crumbs.push((r, loc.clone()));
            x = r.right;
        }
        // Flecha final: las subcarpetas de la carpeta actual.
        if let Some((_, last)) = all.last() {
            if x + CHEVRON_W <= addr.right - 4.0 {
                self.draw_chevron(p, rect(x, addr.top + 4.0, x + CHEVRON_W, addr.bottom - 4.0), last.clone(), hover);
            }
        }
    }

    /// Flecha entre tramos: abre el menú de subcarpetas de `of`. Abierta, apunta hacia abajo.
    fn draw_chevron(&mut self, p: &dyn Painter, r: Rect, of: Location, hover: PaneHit) {
        let th = p.theme();
        let idx = self.chevrons.len();
        let open = self.open_chevron == Some(idx);
        let hot = open || hover == PaneHit::Chevron(idx);
        if hot {
            p.round_fill(r, 4.0, alpha(th.text, 0.16));
        }
        let glyph = if open { "\u{E70D}" } else { "\u{E76C}" };
        p.text(FontId::GlyphsSmall, glyph, r, if hot { th.text } else { th.text_dim });
        self.chevrons.push((r, of));
    }
}

/// Indicador de "trabajando": puntos en círculo que giran (se anima mientras se siga dibujando).
fn spinner(p: &dyn Painter, cx: f32, cy: f32, c: crate::geom::Color) {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    const DOTS: usize = 8;
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    let head = (t * 10.0) as usize % DOTS;
    for i in 0..DOTS {
        let angle = i as f32 / DOTS as f32 * std::f32::consts::TAU;
        let age = (head + DOTS - i) % DOTS;
        let a = 1.0 - age as f32 / DOTS as f32 * 0.85;
        p.circle(cx + angle.cos() * 5.5, cy + angle.sin() * 5.5, 1.4, alpha(c, a));
    }
}


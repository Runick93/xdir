//! Un panel: tira de pestañas, barra de navegación con dirección y la lista de la pestaña activa.
//! Guarda las áreas del último frame para resolver clics.

use crate::favorites::{Fav, Favorites};
use crate::fs::Location;
use crate::render::{Painter, Rect, contains, rect};
use crate::tab::Tab;
use crate::theme::alpha;

pub const TABSTRIP_H: f32 = 34.0;
pub const NAVBAR_H: f32 = 42.0;
pub const FAVBAR_H: f32 = 30.0;
const PANE_RADIUS: f32 = 8.0;
/// Inclinación de los lados de las pestañas.
const TAB_SLANT: f32 = 10.0;
/// Radio de las puntas de arriba de las pestañas y de su encuentro con el borde.
const TAB_RADIUS: f32 = 6.0;
const TAB_FOOT_RADIUS: f32 = 4.0;

/// Contorno de una pestaña sin base: (abajo izquierda, arriba izquierda, arriba derecha,
/// abajo derecha). `left_slant` 0 = lado izquierdo recto (primera pestaña).
fn tab_points(r: Rect, left_slant: f32) -> Vec<(f32, f32, f32)> {
    let (l, t, rr, b) = (r.left + 0.5, r.top + 0.5, r.right - 0.5, r.bottom);
    vec![(l, b, 0.0), (l + left_slant, t, TAB_RADIUS), (rr - TAB_SLANT, t, TAB_RADIUS), (rr, b, 0.0)]
}

/// Contorno del panel como una sola figura: el cuerpo redondeado con la pestaña activa
/// integrada en el borde de arriba. `first`: la pestaña activa es la primera, y su lado
/// izquierdo es la continuación recta del borde del panel.
fn outline(body: Rect, tab: Option<Rect>, first: bool) -> Vec<(f32, f32, f32)> {
    let h = 0.75; // medio trazo hacia adentro para que el borde quede nítido
    let (l, t, r, b) = (body.left + h, body.top + h, body.right - h, body.bottom - h);
    let Some(tab) = tab else {
        return vec![(l, b, PANE_RADIUS), (l, t, PANE_RADIUS), (r, t, PANE_RADIUS), (r, b, PANE_RADIUS)];
    };
    let (tl, tr, tt) = (tab.left + h, tab.right - h, tab.top + h);
    let mut pts = vec![(l, b, PANE_RADIUS)];
    if first {
        pts.push((l, tt, TAB_RADIUS));
    } else {
        pts.push((l, t, PANE_RADIUS));
        pts.push((tl, t, TAB_FOOT_RADIUS));
        pts.push((tl + TAB_SLANT, tt, TAB_RADIUS));
    }
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
const TAB_MIN_W: f32 = 90.0;
const TAB_MAX_W: f32 = 220.0;
const CHEVRON_W: f32 = 18.0;
const MAXIMIZE_W: f32 = 32.0;
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
    Address,
    /// Estrella de la barra de dirección: agregar o quitar de favoritos.
    Star,
    /// Botón de consola de la barra de dirección.
    Terminal,
    /// Favorito de la barra (índice en la raíz).
    Fav(usize),
    /// Botón "»" con los favoritos que no entran.
    FavOverflow,
    /// Parte vacía de la barra de favoritos.
    FavBar,
    List,
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
    new_tab: Rect,
    maximize: Rect,
    buttons: Vec<(Button, Rect)>,
    pub address: Rect,
    crumbs: Vec<(Rect, Location)>,
    star: Rect,
    terminal: Rect,
    /// Favoritos visibles de la raíz: (área, índice).
    favs: Vec<(Rect, usize)>,
    /// Botón de desborde y el primer favorito que no entró.
    fav_overflow: Option<(Rect, usize)>,
    pub favbar: Rect,
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
            new_tab: zero,
            maximize: zero,
            buttons: Vec::new(),
            address: zero,
            crumbs: Vec::new(),
            star: zero,
            terminal: zero,
            favs: Vec::new(),
            fav_overflow: None,
            favbar: zero,
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
            self.active = i;
        }
    }

    pub fn cycle_tab(&mut self, forward: bool) {
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

    /// Saca una pestaña para llevarla a otro panel (no va al historial de cerradas). El panel
    /// puede quedar vacío: quien llama tiene que agregarle una.
    pub fn take_tab(&mut self, i: usize) -> Tab {
        let tab = self.tabs.remove(i);
        if self.active > i || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        tab
    }

    /// Inserta una pestaña traída de otro panel en la posición `at` y la activa.
    pub fn insert_tab(&mut self, at: usize, tab: Tab) {
        let at = at.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active = at;
    }

    /// Posición donde entraría una pestaña soltada en `x` sobre la tira: antes de la primera
    /// pestaña cuyo centro queda a la derecha.
    pub fn insert_index_at(&self, x: f32) -> usize {
        self.tab_rects
            .iter()
            .position(|(r, _)| x < (r.left + r.right) / 2.0)
            .unwrap_or(self.tabs.len())
    }

    /// Marca de destino al arrastrar una pestaña desde otro panel: contorno de acento y una
    /// barra en la tira donde caería.
    pub fn draw_tab_drop(&self, p: &Painter, at: usize) {
        let th = p.theme;
        let body = rect(self.rect.left, self.rect.top + TABSTRIP_H, self.rect.right, self.rect.bottom);
        p.round_fill(body, PANE_RADIUS, alpha(th.accent, 0.06));
        p.round_stroke_w(body, PANE_RADIUS, th.accent, 2.0);
        let x = match (self.tab_rects.get(at), self.tab_rects.last()) {
            (Some((r, _)), _) => r.left + if at == 0 { 2.0 } else { TAB_SLANT / 2.0 },
            (None, Some((r, _))) => r.right - TAB_SLANT / 2.0,
            (None, None) => self.rect.left + 2.0,
        };
        let (top, bottom) = (self.rect.top + 6.0, self.rect.top + TABSTRIP_H - 4.0);
        p.round_fill(rect(x - 1.5, top, x + 1.5, bottom), 1.5, th.accent);
    }

    /// Pestaña bajo la coordenada x (para arrastrar y soltar dentro de la tira).
    pub fn tab_index_at(&self, x: f32) -> Option<usize> {
        let last = self.tab_rects.len().checked_sub(1)?;
        Some(
            self.tab_rects
                .iter()
                .position(|(r, _)| x < r.right)
                .unwrap_or(last),
        )
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
        if contains(&self.terminal, x, y) {
            return PaneHit::Terminal;
        }
        if contains(&self.star, x, y) {
            return PaneHit::Star;
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
        if contains(&self.list, x, y) {
            return PaneHit::List;
        }
        PaneHit::None
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
        p: &Painter,
        area: Rect,
        active_pane: bool,
        hover: PaneHit,
        pressed: PaneHit,
        editing: bool,
        favs: Option<(&Favorites, Option<FavDrop>)>,
        terminal_open: bool,
        maximized: Option<bool>,
    ) {
        self.rect = area;
        let th = p.theme;
        // Cuerpo del panel (debajo de las pestañas) y pestaña activa forman una sola pieza: un
        // contorno continuo que sube por la pestaña y vuelve a bajar, sin uniones.
        let body = rect(area.left, area.top + TABSTRIP_H, area.right, area.bottom);
        let reserved = if maximized.is_some() { MAXIMIZE_W } else { 0.0 };
        let layout = self.layout_tabs(area, reserved);
        let outline = outline(body, layout.get(self.active).copied(), self.active == 0);
        p.rounded_path(&outline, true, Some(th.surface), None);
        self.draw_tabs(p, area, &layout, hover);
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
            p.text(&p.fonts.glyphs, glyph, r, color);
            self.buttons.push((b, r));
            x += BTN + 2.0;
        }

        // Barra de dirección.
        let addr = rect(x + 6.0, top, nav.right - 8.0, top + BTN);
        self.address = addr;
        p.round_fill(addr, 6.0, th.surface_alt);
        self.star = rect(0.0, 0.0, 0.0, 0.0);
        self.terminal = rect(0.0, 0.0, 0.0, 0.0);
        if editing {
            p.round_stroke(addr, 6.0, th.accent);
            self.crumbs.clear();
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
            p.text(&p.fonts.ui_center, ">_", term, color);
            self.terminal = if enabled { term } else { rect(0.0, 0.0, 0.0, 0.0) };
            if let Some((f, _)) = favs {
                // Estrella llena si la carpeta ya es favorita.
                let star = rect(addr.right - STAR_W, addr.top + 3.0, addr.right - 3.0, addr.bottom - 3.0);
                if hover == PaneHit::Star {
                    p.round_fill(star, 4.0, th.hover);
                }
                let is_fav = f.find(&self.tabs[self.active].location).is_some();
                let (glyph, color) = if is_fav { ("\u{E735}", th.accent) } else { ("\u{E734}", th.text_dim) };
                p.text(&p.fonts.glyphs, glyph, star, color);
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
        self.list = rect(area.left + 1.0, top, area.right - 1.0, area.bottom - PANE_RADIUS * 0.75);
        self.tabs[self.active].list.draw(p, self.list, active_pane);

        // Contorno encima de todo; de acento si es el panel activo.
        let stroke = if active_pane { (th.accent, 1.5) } else { (th.border, 1.0) };
        p.rounded_path(&outline, true, None, Some(stroke));
    }

    /// Posición de cada pestaña visible. La primera arranca justo en el borde izquierdo del
    /// panel (su lado izquierdo es recto, continuación del borde).
    /// `reserved`: ancho que queda libre a la derecha (botón de maximizar).
    fn layout_tabs(&self, area: Rect, reserved: f32) -> Vec<Rect> {
        let n = self.tabs.len() as f32;
        let right = area.right - 40.0 - reserved;
        let avail = right - area.left;
        let width = (avail / n).clamp(TAB_MIN_W, TAB_MAX_W);
        let (top, bottom) = (area.top + 3.0, area.top + TABSTRIP_H);
        let mut x = area.left;
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

    /// Dibuja las pestañas inactivas (la activa ya es parte del contorno del panel) y los textos.
    fn draw_tabs(&mut self, p: &Painter, area: Rect, layout: &[Rect], hover: PaneHit) {
        let th = p.theme;
        self.tab_rects.clear();
        for (i, &r) in layout.iter().enumerate() {
            let tab = &self.tabs[i];
            let active = i == self.active;
            let hovered = hover == PaneHit::Tab(i) || hover == PaneHit::TabClose(i);
            let left_slant = if i == 0 { 0.0 } else { TAB_SLANT };
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
            p.text(&p.fonts.ui, &tab.location.title(), text, color);
            if show_close {
                if hover == PaneHit::TabClose(i) {
                    p.round_fill(close, 4.0, alpha(th.text, 0.12));
                }
                p.text(&p.fonts.glyphs_small, "\u{E711}", close, th.text_dim);
            }
            self.tab_rects.push((r, if show_close { close } else { rect(0.0, 0.0, 0.0, 0.0) }));
        }

        let x = layout.last().map(|r| r.right + 2.0).unwrap_or(area.left);
        let (top, bottom) = (area.top + 3.0, area.top + TABSTRIP_H);
        let nt = rect(x + 4.0, top + 3.0, x + 32.0, bottom - 4.0);
        if hover == PaneHit::NewTab {
            p.round_fill(nt, 6.0, th.hover);
        }
        p.text(&p.fonts.glyphs_small, "\u{E710}", nt, th.text_dim);
        self.new_tab = nt;
    }

    /// Botón de maximizar (`Some(false)`) o restaurar (`Some(true)`) en el extremo derecho de la
    /// tira de pestañas; `None` si el layout tiene un solo panel.
    fn draw_maximize(&mut self, p: &Painter, area: Rect, maximized: Option<bool>, hover: PaneHit) {
        let th = p.theme;
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
        p.text(&p.fonts.glyphs_small, glyph, r, color);
        self.maximize = r;
    }

    fn draw_favs(&mut self, p: &Painter, bar: Rect, f: &Favorites, hover: PaneHit, drop: Option<FavDrop>) {
        let th = p.theme;
        if f.items.is_empty() {
            let hint = "Arrastrá carpetas acá o tocá la estrella para agregar favoritos";
            p.text(&p.fonts.ui, hint, rect(bar.left + 12.0, bar.top, bar.right - 8.0, bar.bottom), th.text_dim);
            if drop.is_some() {
                p.fill(rect(bar.left + 6.0, bar.top + 5.0, bar.left + 8.0, bar.bottom - 5.0), th.accent);
            }
            return;
        }
        let folder_icon = crate::fs::folder_icon();
        let mut x = bar.left + 6.0;
        for (i, fav) in f.items.iter().enumerate() {
            let text_w = p.measure(&p.fonts.ui, fav.name()).min(FAV_MAX_W);
            let extra = if fav.is_group() { 14.0 } else { 0.0 };
            let w = 8.0 + 16.0 + 6.0 + text_w + extra + 10.0;
            // Si no entra (dejando lugar para "»"), el resto va al desborde.
            let limit = if i + 1 < f.items.len() { bar.right - OVERFLOW_W - 6.0 } else { bar.right - 6.0 };
            if x + w > limit {
                let r = rect(bar.right - OVERFLOW_W - 4.0, bar.top + 3.0, bar.right - 4.0, bar.bottom - 3.0);
                if hover == PaneHit::FavOverflow {
                    p.round_fill(r, 4.0, th.hover);
                }
                p.text(&p.fonts.ui_center, "»", r, th.text);
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
                    p.text(&p.fonts.glyphs, "\u{E8B7}", g, color);
                }
                (Fav::Link { .. }, Some(icon)) => p.shell_icon(icon, 0, r.left + 8.0, iy, 16.0),
            }
            let tx = r.left + 8.0 + 16.0 + 6.0;
            p.text(&p.fonts.ui, fav.name(), rect(tx, r.top, tx + text_w, r.bottom), th.text);
            if fav.is_group() {
                p.text(&p.fonts.glyphs_small, "\u{E70D}", rect(tx + text_w, r.top, tx + text_w + 14.0, r.bottom), th.text_dim);
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

    fn draw_crumbs(&mut self, p: &Painter, addr: Rect, hover: PaneHit) {
        let th = p.theme;
        self.crumbs.clear();
        let all = self.tabs[self.active].location.crumbs();
        let widths: Vec<f32> = all.iter().map(|(label, _)| p.measure(&p.fonts.ui, label) + 16.0).collect();

        // Si no entran, se descartan los primeros (los más lejanos).
        let avail = addr.right - addr.left - 16.0;
        let mut first = 0;
        while first + 1 < all.len() && widths[first..].iter().sum::<f32>() + CHEVRON_W * (all.len() - first) as f32 > avail {
            first += 1;
        }

        let mut x = addr.left + 6.0;
        if first > 0 {
            p.text(&p.fonts.ui, "…", rect(x, addr.top, x + 16.0, addr.bottom), th.text_dim);
            x += 16.0;
        }
        for (i, (label, loc)) in all.iter().enumerate().skip(first) {
            if i > first || first > 0 {
                p.text(&p.fonts.glyphs_small, "\u{E76C}", rect(x, addr.top, x + CHEVRON_W, addr.bottom), th.text_dim);
                x += CHEVRON_W;
            }
            let r = rect(x, addr.top + 4.0, (x + widths[i]).min(addr.right - 4.0), addr.bottom - 4.0);
            let idx = self.crumbs.len();
            if hover == PaneHit::Crumb(idx) {
                p.round_fill(r, 4.0, th.hover);
            }
            let last = i + 1 == all.len();
            p.text(&p.fonts.ui_center, label, r, if last { th.text } else { th.text_dim });
            self.crumbs.push((r, loc.clone()));
            x = r.right;
        }
    }
}

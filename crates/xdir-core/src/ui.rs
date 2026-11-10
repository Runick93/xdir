//! Marco de la ventana: distribución de los paneles (layout), divisores arrastrables y barra
//! de estado con los botones para cambiar de layout.

use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};

use crate::console::Console;
use crate::theme::{Accent, PRESETS, alpha};

pub const STATUSBAR_H: f32 = 30.0;
/// Margen exterior y separación entre paneles (también es el área del divisor).
pub const GAP: f32 = 6.0;
const LAYOUT_BTN_W: f32 = 30.0;
const GEAR_W: f32 = 30.0;
const GROUPS_W: f32 = 84.0;
const XTREE_W: f32 = 74.0;
const MIN_SPLIT: f32 = 0.15;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    One,
    /// Dos paneles lado a lado.
    TwoSide,
    /// Dos paneles uno arriba del otro.
    TwoStack,
    /// Uno a la izquierda y dos apilados a la derecha.
    Three,
    Four,
}

impl Layout {
    pub const ALL: [Layout; 5] = [Layout::One, Layout::TwoSide, Layout::TwoStack, Layout::Three, Layout::Four];

    pub fn panes(self) -> usize {
        match self {
            Layout::One => 1,
            Layout::TwoSide | Layout::TwoStack => 2,
            Layout::Three => 3,
            Layout::Four => 4,
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&l| l == self).unwrap()
    }

    pub fn from_index(i: usize) -> Option<Layout> {
        Self::ALL.get(i).copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Splitter {
    /// Línea vertical: reparte el ancho.
    Vertical,
    /// Línea horizontal: reparte el alto.
    Horizontal,
}

/// Proporciones de los divisores (0..1).
#[derive(Clone, Copy, Debug)]
pub struct Splits {
    pub x: f32,
    pub y: f32,
}

impl Default for Splits {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 }
    }
}

impl Splits {
    pub fn set(&mut self, which: Splitter, value: f32) {
        let v = value.clamp(MIN_SPLIT, 1.0 - MIN_SPLIT);
        match which {
            Splitter::Vertical => self.x = v,
            Splitter::Horizontal => self.y = v,
        }
    }
}

/// Área de los paneles (sin márgenes ni barra de estado). `left`: ancho que ocupa algo a la
/// izquierda (el árbol de carpetas, si está).
pub fn panes_area(w: f32, h: f32, left: f32, top: f32) -> Rect {
    rect(GAP + left, top + GAP / 2.0, w - GAP, h - STATUSBAR_H)
}

/// Alto de la barra de pestañas de grupos (arriba de todo, solo si hay grupos guardados).
pub const GROUPBAR_H: f32 = 40.0;
const GROUP_TAB_W: f32 = 150.0;
/// Separación entre pestañas de grupo.
const GROUP_TAB_GAP: f32 = 6.0;
const GROUP_TAB_MIN_W: f32 = 70.0;
const GROUP_ADD_W: f32 = 28.0;

/// Pestañas de los `n` grupos y botón «+» a su derecha. El ancho de las pestañas se achica para
/// que entren todas.
pub fn group_tabs(w: f32, left: f32, n: usize) -> (Vec<Rect>, Rect) {
    let avail = (w - GAP - left - GROUP_ADD_W - 4.0).max(GROUP_TAB_MIN_W);
    let tab_w = (avail / n.max(1) as f32).clamp(GROUP_TAB_MIN_W.min(avail), GROUP_TAB_W);
    let (top, bottom) = (5.0, GROUPBAR_H - 3.0);
    let mut x = left;
    let tabs = (0..n)
        .map(|_| {
            let r = rect(x, top, x + tab_w - GROUP_TAB_GAP, bottom);
            x += tab_w;
            r
        })
        .collect();
    (tabs, rect(x + 2.0, top, x + 2.0 + GROUP_ADD_W, bottom))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupHit {
    Tab(usize),
    /// La × de una pestaña sin guardar.
    Close(usize),
    Add,
}

/// Cuadrado de la × de una pestaña.
fn group_close_rect(tab: Rect) -> Rect {
    rect(tab.right - 24.0, tab.top + 3.0, tab.right - 4.0, tab.bottom - 3.0)
}

/// `temps[i]`: la pestaña i está sin guardar (tiene ×).
pub fn group_bar_at(w: f32, left: f32, temps: &[bool], x: f32, y: f32) -> Option<GroupHit> {
    let (tabs, add) = group_tabs(w, left, temps.len());
    if contains(&add, x, y) {
        return Some(GroupHit::Add);
    }
    let i = tabs.iter().position(|r| contains(r, x, y))?;
    Some(if temps[i] && contains(&group_close_rect(tabs[i]), x, y) { GroupHit::Close(i) } else { GroupHit::Tab(i) })
}

pub fn draw_group_bar(p: &dyn Painter, w: f32, left: f32, names: &[(&str, bool)], active: Option<usize>, hover: Option<GroupHit>) {
    let th = p.theme();
    let (tabs, add) = group_tabs(w, left, names.len());
    for (i, (r, (name, temp))) in tabs.iter().zip(names).enumerate() {
        // Píldora con borde fino; la activa lleva el contorno y un velo del color de acento.
        p.round_fill(*r, 9.0, th.surface);
        if hover == Some(GroupHit::Tab(i)) && active != Some(i) {
            p.round_fill(*r, 9.0, th.hover);
        }
        if active == Some(i) {
            p.round_fill(*r, 9.0, alpha(th.accent, 0.16));
            p.round_stroke_w(*r, 9.0, th.accent, 1.5);
        } else {
            p.round_stroke(*r, 9.0, th.border);
        }
        let color = if active == Some(i) { th.text } else { th.text_dim };
        let right = if *temp { r.right - 28.0 } else { r.right - 12.0 };
        p.text(FontId::Ui, name, rect(r.left + 14.0, r.top, right, r.bottom), color);
        if *temp {
            let x = group_close_rect(*r);
            let hot = hover == Some(GroupHit::Close(i));
            if hot {
                p.round_fill(x, 4.0, th.hover);
            }
            p.text(FontId::GlyphsSmall, "\u{E711}", x, if hot { th.text } else { th.text_dim });
        }
    }
    let hot = hover == Some(GroupHit::Add);
    if hot {
        p.round_fill(add, 6.0, th.hover);
    }
    p.text(FontId::GlyphsSmall, "\u{E710}", add, if hot { th.text } else { th.text_dim });
}

/// Rectángulos de cada panel visible, en orden.
pub fn pane_rects(layout: Layout, area: Rect, s: Splits) -> Vec<Rect> {
    let (l, t, r, b) = (area.left, area.top, area.right, area.bottom);
    let x = l + (r - l) * s.x;
    let y = t + (b - t) * s.y;
    let g = GAP / 2.0;
    match layout {
        Layout::One => vec![area],
        Layout::TwoSide => vec![rect(l, t, x - g, b), rect(x + g, t, r, b)],
        Layout::TwoStack => vec![rect(l, t, r, y - g), rect(l, y + g, r, b)],
        Layout::Three => vec![rect(l, t, x - g, b), rect(x + g, t, r, y - g), rect(x + g, y + g, r, b)],
        Layout::Four => vec![
            rect(l, t, x - g, y - g),
            rect(x + g, t, r, y - g),
            rect(l, y + g, x - g, b),
            rect(x + g, y + g, r, b),
        ],
    }
}

/// Paneles a la vista con su rectángulo: todos los del layout, o solo el maximizado ocupando el área.
pub fn visible_rects(layout: Layout, maximized: Option<usize>, area: Rect, s: Splits) -> Vec<(usize, Rect)> {
    match maximized {
        Some(i) => vec![(i, area)],
        None => pane_rects(layout, area, s).into_iter().enumerate().collect(),
    }
}

/// Áreas sensibles de los divisores del layout.
pub fn splitter_rects(layout: Layout, area: Rect, s: Splits) -> Vec<(Splitter, Rect)> {
    let (l, t, r, b) = (area.left, area.top, area.right, area.bottom);
    let x = l + (r - l) * s.x;
    let y = t + (b - t) * s.y;
    // El área de agarre es más ancha que el hueco entre paneles: 6 DIPs cuesta acertarle.
    let g = GAP;
    let v = (Splitter::Vertical, rect(x - g, t, x + g, b));
    match layout {
        Layout::One => vec![],
        Layout::TwoSide => vec![v],
        Layout::TwoStack => vec![(Splitter::Horizontal, rect(l, y - g, r, y + g))],
        // El divisor horizontal solo existe en la columna derecha.
        Layout::Three => vec![v, (Splitter::Horizontal, rect(x + g, y - g, r, y + g))],
        Layout::Four => vec![v, (Splitter::Horizontal, rect(l, y - g, r, y + g))],
    }
}

/// Fracción del divisor para una posición del mouse.
pub fn split_value(which: Splitter, area: Rect, x: f32, y: f32) -> f32 {
    match which {
        Splitter::Vertical => (x - area.left) / (area.right - area.left),
        Splitter::Horizontal => (y - area.top) / (area.bottom - area.top),
    }
}

/// Rueda dentada al final de la barra de estado: abre el panel de colores.
pub fn gear_rect(w: f32, h: f32) -> Rect {
    rect(w - GAP * 2.0 - GEAR_W, h - STATUSBAR_H + 4.0, w - GAP * 2.0, h - 4.0)
}

/// Botones de layout en la barra de estado (a la izquierda de la rueda dentada).
pub fn layout_buttons(w: f32, h: f32) -> Vec<(Layout, Rect)> {
    let top = h - STATUSBAR_H + 4.0;
    let mut x = w - GAP * 2.0 - GEAR_W - 8.0 - LAYOUT_BTN_W * Layout::ALL.len() as f32;
    Layout::ALL
        .iter()
        .map(|&l| {
            let r = rect(x, top, x + LAYOUT_BTN_W, h - 4.0);
            x += LAYOUT_BTN_W;
            (l, r)
        })
        .collect()
}

/// Botón de grupos guardados, a la izquierda de los de layout.
pub fn groups_rect(w: f32, h: f32) -> Rect {
    let first = layout_buttons(w, h)[0].1;
    rect(first.left - 10.0 - GROUPS_W, first.top, first.left - 10.0, first.bottom)
}

/// Botón que abre xtree (mapa de uso de disco), a la izquierda del de grupos.
pub fn xtree_rect(w: f32, h: f32) -> Rect {
    let g = groups_rect(w, h);
    rect(g.left - 10.0 - XTREE_W, g.top, g.left - 10.0, g.bottom)
}

/// Área de la vista de xtree: todo lo que está arriba de la barra de estado (los paneles, la
/// franja de grupos y el lugar del árbol). La vista pone sus propios márgenes.
pub fn xtree_area(w: f32, h: f32) -> Rect {
    rect(0.0, 0.0, w, h - STATUSBAR_H)
}

pub fn layout_at(w: f32, h: f32, x: f32, y: f32) -> Option<Layout> {
    layout_buttons(w, h).into_iter().find(|(_, r)| contains(r, x, y)).map(|(l, _)| l)
}

/// `xtree_on`: la vista de xtree está abierta (el botón queda encendido).
#[allow(clippy::too_many_arguments)]
pub fn draw_status(p: &dyn Painter, w: f32, h: f32, text: &str, current: Layout, hover: Option<Layout>, gear_hot: bool, groups_hot: bool, xtree_hot: bool, xtree_on: bool) {
    let th = p.theme();
    let buttons = layout_buttons(w, h);
    let groups = groups_rect(w, h);
    let xtree = xtree_rect(w, h);
    let text_r = rect(GAP * 2.0, h - STATUSBAR_H, xtree.left - GAP, h);
    p.text(FontId::Ui, text, text_r, th.text_dim);

    if groups_hot {
        p.round_fill(groups, 4.0, th.hover);
    }
    let color = if groups_hot { th.text } else { th.text_dim };
    p.text(FontId::GlyphsSmall, "\u{E8A9}", rect(groups.left + 6.0, groups.top, groups.left + 26.0, groups.bottom), color);
    p.text(FontId::Ui, "Grupos", rect(groups.left + 30.0, groups.top, groups.right - 4.0, groups.bottom), color);

    if xtree_on {
        p.round_fill(xtree, 4.0, th.selection);
    } else if xtree_hot {
        p.round_fill(xtree, 4.0, th.hover);
    }
    let color = if xtree_on { th.accent } else if xtree_hot { th.text } else { th.text_dim };
    p.text(FontId::GlyphsSmall, "\u{E9D9}", rect(xtree.left + 6.0, xtree.top, xtree.left + 26.0, xtree.bottom), color);
    p.text(FontId::Ui, "xtree", rect(xtree.left + 30.0, xtree.top, xtree.right - 4.0, xtree.bottom), color);

    let gear = gear_rect(w, h);
    if gear_hot {
        p.round_fill(gear, 4.0, th.hover);
    }
    p.text(FontId::Glyphs, "\u{E713}", gear, if gear_hot { th.text } else { th.text_dim });
    for (layout, r) in buttons {
        if layout == current {
            p.round_fill(r, 4.0, alpha(th.accent, 0.25));
        } else if hover == Some(layout) {
            p.round_fill(r, 4.0, th.hover);
        }
        // Miniatura del layout: un rectángulo de 16×12 DIPs partido como los paneles.
        let cx = (r.left + r.right) / 2.0;
        let cy = (r.top + r.bottom) / 2.0;
        let icon = rect(cx - 8.0, cy - 6.0, cx + 8.0, cy + 6.0);
        let color = if layout == current { th.accent } else { th.text_dim };
        for cell in pane_rects(layout, icon, Splits::default()) {
            p.round_stroke(cell, 1.0, color);
        }
    }
}

// ---------- panel de colores ----------

/// Elementos del panel de colores.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerHit {
    Preset(usize),
    /// Usar el acento de Windows.
    System,
    /// Elegir cualquier color con el selector de Windows.
    Custom,
    /// Switch de modo oscuro.
    DarkMode,
    /// Switch de los colores por tipo de archivo.
    FileColors,
    /// Switch «Mostrar extensiones» (nombres de archivo con su extensión).
    Extensions,
    /// Switch «Casillas de selección» de las listas.
    Checkboxes,
    /// Switch «El árbol sigue al panel» (solo con la feature `arbol`).
    TreeFollow,
    /// Switch «Mostrar archivos ocultos» (el mismo estado que el comando `mostrar_ocultos`).
    Hidden,
    /// Switch de la barra de pestañas de grupos.
    GroupBar,
    /// Switch del árbol de carpetas (solo con la feature `arbol`).
    Tree,
    /// Abre el diálogo "Acerca de".
    About,
    /// Dentro del panel pero sobre nada.
    Panel,
}

const PICKER_COLS: usize = 6;
const PICKER_CELL: f32 = 38.0;
const PICKER_PAD: f32 = 14.0;
const PICKER_TITLE_H: f32 = 34.0;
const PICKER_BTN_H: f32 = 32.0;
const PICKER_SEP_H: f32 = 13.0;

/// Estado de los switches del panel de la rueda dentada (el modo oscuro sale del tema).
#[derive(Clone, Copy, Debug, Default)]
pub struct PickerSwitches {
    pub file_colors: bool,
    pub extensions: bool,
    pub checkboxes: bool,
    pub tree_follow: bool,
    pub hidden: bool,
    pub group_bar: bool,
    pub tree: bool,
}

/// Switches del panel, de arriba abajo: modo oscuro, colores por tipo, extensiones, casillas,
/// el árbol sigue al panel (si se compiló el árbol), archivos ocultos, barra de grupos y el árbol.
fn switch_kinds() -> Vec<PickerHit> {
    let arbol = cfg!(feature = "arbol");
    let mut kinds = vec![PickerHit::DarkMode, PickerHit::FileColors, PickerHit::Extensions, PickerHit::Checkboxes];
    if arbol {
        kinds.push(PickerHit::TreeFollow);
    }
    kinds.extend([PickerHit::Hidden, PickerHit::GroupBar]);
    if arbol {
        kinds.push(PickerHit::Tree);
    }
    kinds
}

fn switch_rows() -> usize {
    switch_kinds().len()
}

/// Medidas verticales del panel de la rueda dentada. Se elige la primera que entra en la ventana;
/// si ni la más compacta entra (alturas menores a ~330 DIPs) el panel se pega arriba (top >= 0)
/// y las últimas filas pueden quedar cortadas. La ventana mínima (360) entra con la compacta.
#[derive(Clone, Copy)]
struct PickerLayout {
    title: f32,
    cell: f32,
    btn: f32,
    sep: f32,
    pad: f32,
    gap: f32,
}

const PICKER_LAYOUTS: [PickerLayout; 3] = [
    PickerLayout { title: PICKER_TITLE_H, cell: PICKER_CELL, btn: PICKER_BTN_H, sep: PICKER_SEP_H, pad: PICKER_PAD, gap: 8.0 },
    PickerLayout { title: 28.0, cell: 30.0, btn: 24.0, sep: 8.0, pad: 10.0, gap: 6.0 },
    PickerLayout { title: 24.0, cell: 26.0, btn: 20.0, sep: 6.0, pad: 6.0, gap: 4.0 },
];

impl PickerLayout {
    fn height(&self) -> f32 {
        let rows = (PRESETS.len() / PICKER_COLS) as f32;
        // Colores, dos botones, un separador, las filas de switches, otro separador y "Acerca de".
        self.title + self.cell * rows + self.gap + self.btn * 2.0 + self.sep + self.btn * switch_rows() as f32 + self.sep + self.btn + self.pad
    }
}

/// Margen mínimo entre el panel y el borde superior de la ventana.
const PICKER_TOP_MARGIN: f32 = 4.0;

fn picker_layout(h: f32) -> PickerLayout {
    let avail = h - STATUSBAR_H - 4.0 - PICKER_TOP_MARGIN;
    PICKER_LAYOUTS.into_iter().find(|l| l.height() <= avail).unwrap_or(PICKER_LAYOUTS[PICKER_LAYOUTS.len() - 1])
}

/// Panel flotante sobre la rueda dentada.
pub fn picker_rect(w: f32, h: f32) -> Rect {
    let l = picker_layout(h);
    let width = PICKER_CELL * PICKER_COLS as f32 + PICKER_PAD * 2.0;
    let right = w - GAP * 2.0;
    let bottom = (h - STATUSBAR_H - 4.0).max(l.height());
    rect(right - width, bottom - l.height(), right, bottom)
}

fn picker_items(w: f32, h: f32) -> Vec<(PickerHit, Rect)> {
    let panel = picker_rect(w, h);
    let lay = picker_layout(h);
    let mut out = Vec::new();
    let x0 = panel.left + (panel.right - panel.left - lay.cell * PICKER_COLS as f32) / 2.0;
    for i in 0..PRESETS.len() {
        let (col, row) = ((i % PICKER_COLS) as f32, (i / PICKER_COLS) as f32);
        let x = x0 + col * lay.cell;
        let y = panel.top + lay.title + row * lay.cell;
        out.push((PickerHit::Preset(i), rect(x, y, x + lay.cell, y + lay.cell)));
    }
    let rows = (PRESETS.len() / PICKER_COLS) as f32;
    let y = panel.top + lay.title + lay.cell * rows + lay.gap;
    let (l, r) = (panel.left + PICKER_PAD - 6.0, panel.right - PICKER_PAD + 6.0);
    out.push((PickerHit::System, rect(l, y, r, y + lay.btn)));
    out.push((PickerHit::Custom, rect(l, y + lay.btn, r, y + lay.btn * 2.0)));
    let y = y + lay.btn * 2.0 + lay.sep;
    for (i, kind) in switch_kinds().into_iter().enumerate() {
        let top = y + lay.btn * i as f32;
        out.push((kind, rect(l, top, r, top + lay.btn)));
    }
    let top = y + lay.btn * switch_rows() as f32 + lay.sep;
    out.push((PickerHit::About, rect(l, top, r, top + lay.btn)));
    out
}

pub fn picker_at(w: f32, h: f32, x: f32, y: f32) -> Option<PickerHit> {
    if !contains(&picker_rect(w, h), x, y) {
        return None;
    }
    Some(picker_items(w, h).into_iter().find(|(_, r)| contains(r, x, y)).map(|(k, _)| k).unwrap_or(PickerHit::Panel))
}

pub fn draw_picker(p: &dyn Painter, w: f32, h: f32, current: Accent, hover: Option<PickerHit>, sw: PickerSwitches) {
    let th = p.theme();
    let panel = picker_rect(w, h);
    // Sombra suave y panel.
    p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 12.0, alpha(th.background, 0.5));
    p.round_fill(panel, 10.0, th.surface_alt);
    p.round_stroke(panel, 10.0, th.border);
    let lay = picker_layout(h);
    let title = rect(panel.left + PICKER_PAD, panel.top + 4.0, panel.right - PICKER_PAD, panel.top + lay.title);
    p.text(FontId::UiBold, "Color de acento", title, th.text);

    let is_preset = PRESETS.iter().any(|(_, c)| current == Accent::Rgb(*c));
    for (kind, r) in picker_items(w, h) {
        let hot = hover == Some(kind);
        match kind {
            PickerHit::Preset(i) => {
                let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                let c = hex(PRESETS[i].1);
                if current == Accent::Rgb(PRESETS[i].1) {
                    p.circle_stroke(cx, cy, lay.cell * 15.0 / PICKER_CELL, th.text, 2.0);
                } else if hot {
                    p.circle_stroke(cx, cy, lay.cell * 15.0 / PICKER_CELL, th.text_dim, 1.0);
                }
                p.circle(cx, cy, lay.cell * 11.0 / PICKER_CELL, c);
            }
            PickerHit::System | PickerHit::Custom => {
                if hot {
                    p.round_fill(r, 6.0, th.hover);
                }
                let selected = match kind {
                    PickerHit::System => current == Accent::System,
                    _ => !is_preset && current != Accent::System,
                };
                let label = if kind == PickerHit::System { "Color de Windows" } else { "Personalizado…" };
                p.text(FontId::Ui, label, rect(r.left + 34.0, r.top, r.right - 8.0, r.bottom), th.text);
                let icon = rect(r.left + 6.0, r.top, r.left + 30.0, r.bottom);
                if kind == PickerHit::Custom {
                    // Muestra del color personalizado, o el ícono de paleta si no hay.
                    match (selected, current) {
                        (true, Accent::Rgb(c)) => p.circle((icon.left + icon.right) / 2.0, (icon.top + icon.bottom) / 2.0, 8.0, hex(c)),
                        _ => p.text(FontId::Glyphs, "\u{E790}", icon, th.text_dim),
                    }
                } else {
                    p.text(FontId::Glyphs, "\u{E7F8}", icon, th.text_dim);
                }
                if selected {
                    p.text(FontId::GlyphsSmall, "\u{E73E}", rect(r.right - 26.0, r.top, r.right - 6.0, r.bottom), th.accent);
                }
            }
            PickerHit::DarkMode
            | PickerHit::FileColors
            | PickerHit::Extensions
            | PickerHit::Checkboxes
            | PickerHit::TreeFollow
            | PickerHit::Hidden
            | PickerHit::GroupBar
            | PickerHit::Tree => {
                if hot {
                    p.round_fill(r, 6.0, th.hover);
                }
                let (glyph, label, on) = match kind {
                    PickerHit::DarkMode => ("\u{E708}", "Modo oscuro", th.dark),
                    PickerHit::FileColors => ("\u{E790}", "Colores por tipo", sw.file_colors),
                    PickerHit::Extensions => ("\u{E8EC}", "Mostrar extensiones", sw.extensions),
                    PickerHit::Checkboxes => ("\u{E739}", "Casillas de selección", sw.checkboxes),
                    PickerHit::TreeFollow => ("\u{E71B}", "El árbol sigue al panel", sw.tree_follow),
                    PickerHit::Hidden => ("\u{E7B3}", "Mostrar archivos ocultos", sw.hidden),
                    PickerHit::GroupBar => ("\u{E8A9}", "Barra de grupos", sw.group_bar),
                    _ => ("\u{E8B7}", "Árbol de carpetas", sw.tree),
                };
                if kind == PickerHit::DarkMode {
                    // Separador sobre la primera fila de switches.
                    let sy = r.top - lay.sep / 2.0;
                    p.fill(rect(r.left + 6.0, sy, r.right - 6.0, sy + 1.0), th.border);
                }
                p.text(FontId::Glyphs, glyph, rect(r.left + 6.0, r.top, r.left + 30.0, r.bottom), th.text_dim);
                p.text(FontId::Ui, label, rect(r.left + 34.0, r.top, r.right - 56.0, r.bottom), th.text);
                // Switch: pastilla de acento con la perilla a la derecha cuando está encendido.
                let cy = (r.top + r.bottom) / 2.0;
                let half = 10.0_f32.min(lay.btn / 2.0 - 2.0);
                let track = rect(r.right - 50.0, cy - half, r.right - 10.0, cy + half);
                if on {
                    p.round_fill(track, half, th.accent);
                } else {
                    p.round_stroke_w(track, half, th.text_dim, 1.5);
                }
                let (kx, kr) = if on { (track.right - half, half - 4.0) } else { (track.left + half, half - 5.0) };
                p.circle(kx, cy, kr, if on { th.background } else { th.text_dim });
            }
            PickerHit::About => {
                if hot {
                    p.round_fill(r, 6.0, th.hover);
                }
                let sy = r.top - lay.sep / 2.0;
                p.fill(rect(r.left + 6.0, sy, r.right - 6.0, sy + 1.0), th.border);
                p.text(FontId::Glyphs, "\u{E946}", rect(r.left + 6.0, r.top, r.left + 30.0, r.bottom), th.text_dim);
                p.text(FontId::Ui, "Acerca de xdir", rect(r.left + 34.0, r.top, r.right - 70.0, r.bottom), th.text);
                p.text(FontId::UiRight, crate::about::VERSION, rect(r.right - 70.0, r.top, r.right - 10.0, r.bottom), th.text_dim);
            }
            PickerHit::Panel => {}
        }
    }
}

// ---------- menú de consola ----------

const TERM_ROW_H: f32 = 34.0;
const TERM_MENU_W: f32 = 210.0;

/// Menú desplegable debajo del botón `>_`: una fila por consola.
fn terminal_menu_rows(anchor: Rect, w: f32) -> Vec<(Console, Rect)> {
    let left = (anchor.left - 8.0).min(w - GAP - TERM_MENU_W);
    let top = anchor.bottom + 6.0;
    Console::all()
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let y = top + 6.0 + i as f32 * TERM_ROW_H;
            (c, rect(left + 6.0, y, left + TERM_MENU_W - 6.0, y + TERM_ROW_H))
        })
        .collect()
}

fn terminal_menu_rect(anchor: Rect, w: f32) -> Rect {
    let rows = terminal_menu_rows(anchor, w);
    let (first, last) = (rows[0].1, rows[rows.len() - 1].1);
    rect(first.left - 6.0, first.top - 6.0, first.right + 6.0, last.bottom + 6.0)
}

/// `Some(None)`: dentro del menú pero sobre nada.
pub fn terminal_menu_at(anchor: Rect, w: f32, x: f32, y: f32) -> Option<Option<Console>> {
    if !contains(&terminal_menu_rect(anchor, w), x, y) {
        return None;
    }
    Some(terminal_menu_rows(anchor, w).into_iter().find(|(_, r)| contains(r, x, y)).map(|(c, _)| c))
}

pub fn draw_terminal_menu(p: &dyn Painter, anchor: Rect, w: f32, hover: Option<Console>) {
    let th = p.theme();
    let panel = terminal_menu_rect(anchor, w);
    p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 10.0, alpha(th.background, 0.5));
    p.round_fill(panel, 8.0, th.surface_alt);
    p.round_stroke(panel, 8.0, th.border);
    for (c, r) in terminal_menu_rows(anchor, w) {
        if hover == Some(c) {
            p.round_fill(r, 6.0, th.hover);
        }
        p.text(FontId::UiBold, ">_", rect(r.left + 8.0, r.top, r.left + 34.0, r.bottom), th.accent);
        p.text(FontId::Ui, c.label(), rect(r.left + 40.0, r.top, r.right - 8.0, r.bottom), th.text);
    }
}

fn hex(c: u32) -> Color {
    Color {
        r: (c >> 16) as f32 / 255.0,
        g: ((c >> 8) & 0xFF) as f32 / 255.0,
        b: (c & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

/// Resalta el divisor bajo el mouse o arrastrándose (una línea en el centro de su área).
pub fn draw_splitter(p: &dyn Painter, r: Rect) {
    let th = p.theme();
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let line = if r.right - r.left < r.bottom - r.top {
        rect(cx - 1.0, r.top + 4.0, cx + 1.0, r.bottom - 4.0)
    } else {
        rect(r.left + 4.0, cy - 1.0, r.right - 4.0, cy + 1.0)
    };
    p.round_fill(line, 1.0, alpha(th.accent, 0.8));
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    #[test]
    fn el_panel_entra_en_ventanas_chicas() {
        for h in [360.0, 400.0, 480.0, 544.0, 700.0] {
            let r = picker_rect(900.0, h);
            assert!(r.top >= 0.0, "top {} con h {h}", r.top);
            assert!(r.bottom <= h, "bottom {} con h {h}", r.bottom);
            for (kind, it) in picker_items(900.0, h) {
                assert!(it.top >= r.top && it.bottom <= r.bottom + 0.01, "{kind:?} fuera del panel con h {h}");
            }
        }
    }

    #[test]
    fn el_primero_y_el_ultimo_switch_se_clickean_en_ventana_chica() {
        let (w, h) = (900.0, 360.0);
        let kinds = switch_kinds();
        let items = picker_items(w, h);
        for want in [kinds[0], *kinds.last().unwrap(), PickerHit::About, PickerHit::Preset(0)] {
            let (_, r) = items.iter().find(|(k, _)| *k == want).unwrap();
            let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            assert_eq!(picker_at(w, h, x, y), Some(want));
        }
    }
}

//! Marco de la ventana: distribución de los paneles (layout), divisores arrastrables y barra
//! de estado con los botones para cambiar de layout.

use crate::render::{Painter, Rect, contains, rect};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

use crate::console::Console;
use crate::theme::{Accent, PRESETS, alpha};

pub const STATUSBAR_H: f32 = 30.0;
/// Margen exterior y separación entre paneles (también es el área del divisor).
pub const GAP: f32 = 6.0;
const LAYOUT_BTN_W: f32 = 30.0;
const GEAR_W: f32 = 30.0;
const GROUPS_W: f32 = 84.0;
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
pub fn panes_area(w: f32, h: f32, left: f32) -> Rect {
    rect(GAP + left, GAP / 2.0, w - GAP, h - STATUSBAR_H)
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

pub fn layout_at(w: f32, h: f32, x: f32, y: f32) -> Option<Layout> {
    layout_buttons(w, h).into_iter().find(|(_, r)| contains(r, x, y)).map(|(l, _)| l)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub fn draw_status(p: &Painter, w: f32, h: f32, text: &str, current: Layout, hover: Option<Layout>, gear_hot: bool, groups_hot: bool, update: bool) {
    let th = p.theme;
    let buttons = layout_buttons(w, h);
    let groups = groups_rect(w, h);
    let text_r = rect(GAP * 2.0, h - STATUSBAR_H, groups.left - GAP, h);
    p.text(&p.fonts.ui, text, text_r, th.text_dim);

    if groups_hot {
        p.round_fill(groups, 4.0, th.hover);
    }
    let color = if groups_hot { th.text } else { th.text_dim };
    p.text(&p.fonts.glyphs_small, "\u{E8A9}", rect(groups.left + 6.0, groups.top, groups.left + 26.0, groups.bottom), color);
    p.text(&p.fonts.ui, "Grupos", rect(groups.left + 30.0, groups.top, groups.right - 4.0, groups.bottom), color);

    let gear = gear_rect(w, h);
    if gear_hot {
        p.round_fill(gear, 4.0, th.hover);
    }
    p.text(&p.fonts.glyphs, "\u{E713}", gear, if gear_hot { th.text } else { th.text_dim });
    if update {
        // Punto de acento: hay una versión nueva (el detalle está en el panel del engranaje).
        p.circle(gear.right - 6.0, gear.top + 6.0, 4.0, th.accent);
    }

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

/// Switches del panel: modo oscuro, colores por tipo y, si se compiló, el árbol de carpetas.
fn switch_rows() -> usize {
    2 + usize::from(cfg!(feature = "arbol"))
}

/// Panel flotante sobre la rueda dentada.
pub fn picker_rect(w: f32, h: f32) -> Rect {
    let rows = (PRESETS.len() / PICKER_COLS) as f32;
    let width = PICKER_CELL * PICKER_COLS as f32 + PICKER_PAD * 2.0;
    // Colores, dos botones, un separador y las filas de switches.
    let height = PICKER_TITLE_H + PICKER_CELL * rows + 8.0 + PICKER_BTN_H * 2.0 + PICKER_SEP_H + PICKER_BTN_H * switch_rows() as f32 + PICKER_SEP_H + PICKER_BTN_H + PICKER_PAD;
    let right = w - GAP * 2.0;
    let bottom = h - STATUSBAR_H - 4.0;
    rect(right - width, bottom - height, right, bottom)
}

fn picker_items(w: f32, h: f32) -> Vec<(PickerHit, Rect)> {
    let panel = picker_rect(w, h);
    let mut out = Vec::new();
    for i in 0..PRESETS.len() {
        let (col, row) = ((i % PICKER_COLS) as f32, (i / PICKER_COLS) as f32);
        let x = panel.left + PICKER_PAD + col * PICKER_CELL;
        let y = panel.top + PICKER_TITLE_H + row * PICKER_CELL;
        out.push((PickerHit::Preset(i), rect(x, y, x + PICKER_CELL, y + PICKER_CELL)));
    }
    let rows = (PRESETS.len() / PICKER_COLS) as f32;
    let y = panel.top + PICKER_TITLE_H + PICKER_CELL * rows + 8.0;
    let (l, r) = (panel.left + PICKER_PAD - 6.0, panel.right - PICKER_PAD + 6.0);
    out.push((PickerHit::System, rect(l, y, r, y + PICKER_BTN_H)));
    out.push((PickerHit::Custom, rect(l, y + PICKER_BTN_H, r, y + PICKER_BTN_H * 2.0)));
    let y = y + PICKER_BTN_H * 2.0 + PICKER_SEP_H;
    let mut kinds = vec![PickerHit::DarkMode, PickerHit::FileColors];
    if cfg!(feature = "arbol") {
        kinds.push(PickerHit::Tree);
    }
    for (i, kind) in kinds.into_iter().enumerate() {
        let top = y + PICKER_BTN_H * i as f32;
        out.push((kind, rect(l, top, r, top + PICKER_BTN_H)));
    }
    let top = y + PICKER_BTN_H * switch_rows() as f32 + PICKER_SEP_H;
    out.push((PickerHit::About, rect(l, top, r, top + PICKER_BTN_H)));
    out
}

pub fn picker_at(w: f32, h: f32, x: f32, y: f32) -> Option<PickerHit> {
    if !contains(&picker_rect(w, h), x, y) {
        return None;
    }
    Some(picker_items(w, h).into_iter().find(|(_, r)| contains(r, x, y)).map(|(k, _)| k).unwrap_or(PickerHit::Panel))
}

#[allow(clippy::too_many_arguments)]
pub fn draw_picker(p: &Painter, w: f32, h: f32, current: Accent, hover: Option<PickerHit>, file_colors: bool, tree: bool, update: bool) {
    let th = p.theme;
    let panel = picker_rect(w, h);
    // Sombra suave y panel.
    p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 12.0, alpha(th.background, 0.5));
    p.round_fill(panel, 10.0, th.surface_alt);
    p.round_stroke(panel, 10.0, th.border);
    let title = rect(panel.left + PICKER_PAD, panel.top + 4.0, panel.right - PICKER_PAD, panel.top + PICKER_TITLE_H);
    p.text(&p.fonts.ui_bold, "Color de acento", title, th.text);

    let is_preset = PRESETS.iter().any(|(_, c)| current == Accent::Rgb(*c));
    for (kind, r) in picker_items(w, h) {
        let hot = hover == Some(kind);
        match kind {
            PickerHit::Preset(i) => {
                let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                let c = hex(PRESETS[i].1);
                if current == Accent::Rgb(PRESETS[i].1) {
                    p.circle_stroke(cx, cy, 15.0, th.text, 2.0);
                } else if hot {
                    p.circle_stroke(cx, cy, 15.0, th.text_dim, 1.0);
                }
                p.circle(cx, cy, 11.0, c);
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
                p.text(&p.fonts.ui, label, rect(r.left + 34.0, r.top, r.right - 8.0, r.bottom), th.text);
                let icon = rect(r.left + 6.0, r.top, r.left + 30.0, r.bottom);
                if kind == PickerHit::Custom {
                    // Muestra del color personalizado, o el ícono de paleta si no hay.
                    match (selected, current) {
                        (true, Accent::Rgb(c)) => p.circle((icon.left + icon.right) / 2.0, (icon.top + icon.bottom) / 2.0, 8.0, hex(c)),
                        _ => p.text(&p.fonts.glyphs, "\u{E790}", icon, th.text_dim),
                    }
                } else {
                    p.text(&p.fonts.glyphs, "\u{E7F8}", icon, th.text_dim);
                }
                if selected {
                    p.text(&p.fonts.glyphs_small, "\u{E73E}", rect(r.right - 26.0, r.top, r.right - 6.0, r.bottom), th.accent);
                }
            }
            PickerHit::DarkMode | PickerHit::FileColors | PickerHit::Tree => {
                if hot {
                    p.round_fill(r, 6.0, th.hover);
                }
                let (glyph, label, on) = match kind {
                    PickerHit::DarkMode => ("\u{E708}", "Modo oscuro", th.dark),
                    PickerHit::FileColors => ("\u{E790}", "Colores por tipo", file_colors),
                    _ => ("\u{E8B7}", "Árbol de carpetas", tree),
                };
                if kind == PickerHit::DarkMode {
                    // Separador sobre la primera fila de switches.
                    let sy = r.top - PICKER_SEP_H / 2.0;
                    p.fill(rect(r.left + 6.0, sy, r.right - 6.0, sy + 1.0), th.border);
                }
                p.text(&p.fonts.glyphs, glyph, rect(r.left + 6.0, r.top, r.left + 30.0, r.bottom), th.text_dim);
                p.text(&p.fonts.ui, label, rect(r.left + 34.0, r.top, r.right - 56.0, r.bottom), th.text);
                // Switch: pastilla de acento con la perilla a la derecha cuando está encendido.
                let cy = (r.top + r.bottom) / 2.0;
                let track = rect(r.right - 50.0, cy - 10.0, r.right - 10.0, cy + 10.0);
                if on {
                    p.round_fill(track, 10.0, th.accent);
                } else {
                    p.round_stroke_w(track, 10.0, th.text_dim, 1.5);
                }
                let (kx, kr) = if on { (track.right - 10.0, 6.0) } else { (track.left + 10.0, 5.0) };
                p.circle(kx, cy, kr, if on { th.background } else { th.text_dim });
            }
            PickerHit::About => {
                if hot {
                    p.round_fill(r, 6.0, th.hover);
                }
                let sy = r.top - PICKER_SEP_H / 2.0;
                p.fill(rect(r.left + 6.0, sy, r.right - 6.0, sy + 1.0), th.border);
                p.text(&p.fonts.glyphs, "\u{E946}", rect(r.left + 6.0, r.top, r.left + 30.0, r.bottom), th.text_dim);
                if update {
                    // Sigue el rastro del aviso: punto del engranaje -> este -> botón de "Acerca de".
                    p.circle(r.left + 27.0, r.top + 9.0, 4.0, th.accent);
                }
                p.text(&p.fonts.ui, "Acerca de xdir", rect(r.left + 34.0, r.top, r.right - 70.0, r.bottom), th.text);
                p.text(&p.fonts.ui_right, crate::about::VERSION, rect(r.right - 70.0, r.top, r.right - 10.0, r.bottom), th.text_dim);
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
    Console::ALL
        .iter()
        .enumerate()
        .map(|(i, &c)| {
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

pub fn draw_terminal_menu(p: &Painter, anchor: Rect, w: f32, hover: Option<Console>) {
    let th = p.theme;
    let panel = terminal_menu_rect(anchor, w);
    p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 10.0, alpha(th.background, 0.5));
    p.round_fill(panel, 8.0, th.surface_alt);
    p.round_stroke(panel, 8.0, th.border);
    for (c, r) in terminal_menu_rows(anchor, w) {
        if hover == Some(c) {
            p.round_fill(r, 6.0, th.hover);
        }
        p.text(&p.fonts.ui_bold, ">_", rect(r.left + 8.0, r.top, r.left + 34.0, r.bottom), th.accent);
        p.text(&p.fonts.ui, c.label(), rect(r.left + 40.0, r.top, r.right - 8.0, r.bottom), th.text);
    }
}

fn hex(c: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (c >> 16) as f32 / 255.0,
        g: ((c >> 8) & 0xFF) as f32 / 255.0,
        b: (c & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

/// Resalta el divisor bajo el mouse o arrastrándose (una línea en el centro de su área).
pub fn draw_splitter(p: &Painter, r: Rect) {
    let th = p.theme;
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let line = if r.right - r.left < r.bottom - r.top {
        rect(cx - 1.0, r.top + 4.0, cx + 1.0, r.bottom - 4.0)
    } else {
        rect(r.left + 4.0, cy - 1.0, r.right - 4.0, cy + 1.0)
    };
    p.round_fill(line, 1.0, alpha(th.accent, 0.8));
}

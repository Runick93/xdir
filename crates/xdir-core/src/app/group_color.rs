//! Color de una pestaña de grupo: el clic derecho sobre la pestaña abre un panel chico con los 12
//! colores de acento (los mismos del selector) y una fila para volver al acento de la aplicación.
//! El color se guarda en el grupo (`color` de `grupos.toml`) y se ve como la barra lateral de la
//! pestaña.
//!
//! Como los demás paneles, el diseño se calcula al pintar y se guarda para resolver los clics del
//! cuadro siguiente. Es modal: un clic fuera solo lo cierra.

use std::cell::RefCell;

use super::*;
use crate::geom::{Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::theme::{PRESETS, alpha};

const COLS: usize = 6;
const SWATCH_R: f32 = 11.0;
const STEP: f32 = 34.0;
const PAD: f32 = 12.0;
const ROW_H: f32 = 32.0;
const MARGIN: f32 = 8.0;

/// Qué hay bajo el mouse en el panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupColorHit {
    /// Uno de los 12 colores (su posición en `PRESETS`).
    Swatch(usize),
    /// «Acento de la aplicación»: el grupo sin color propio.
    App,
    /// El panel sin nada debajo.
    Panel,
}

#[derive(Clone)]
struct Layout {
    panel: Rect,
    swatches: [Rect; 12],
    app: Rect,
}

pub struct GroupColorMenu {
    /// El grupo al que se le cambia el color (por nombre: el orden puede cambiar).
    pub group: String,
    /// El color que ya tiene.
    current: Option<u32>,
    /// Dónde se hizo el clic derecho.
    at: (f32, f32),
    /// La opción con el foco del teclado: 0..12 colores, 12 el acento de la aplicación.
    pub focus: Option<usize>,
    layout: RefCell<Option<Layout>>,
}

fn compute(at: (f32, f32), w: f32, h: f32) -> Layout {
    let grid_w = (COLS - 1) as f32 * STEP + 2.0 * SWATCH_R;
    let width = grid_w + 2.0 * PAD;
    let height = PAD + STEP + 2.0 * SWATCH_R + 8.0 + ROW_H + 6.0;
    let left = at.0.min(w - MARGIN - width).max(MARGIN);
    let top = at.1.min(h - MARGIN - height).max(MARGIN);
    let panel = rect(left, top, left + width, top + height);
    let swatches = std::array::from_fn(|i| {
        let (row, col) = ((i / COLS) as f32, (i % COLS) as f32);
        let (cx, cy) = (panel.left + PAD + SWATCH_R + col * STEP, panel.top + PAD + SWATCH_R + row * STEP);
        rect(cx - STEP / 2.0, cy - STEP / 2.0, cx + STEP / 2.0, cy + STEP / 2.0)
    });
    let app_top = panel.top + PAD + STEP + 2.0 * SWATCH_R + 8.0;
    Layout { panel, swatches, app: rect(panel.left + 6.0, app_top, panel.right - 6.0, app_top + ROW_H) }
}

fn center(r: &Rect) -> (f32, f32) {
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

impl GroupColorMenu {
    pub fn new(group: String, current: Option<u32>, at: (f32, f32)) -> Self {
        GroupColorMenu { group, current, at, focus: None, layout: RefCell::new(None) }
    }

    fn laid(&self, w: f32, h: f32) -> Layout {
        self.layout.borrow().clone().unwrap_or_else(|| compute(self.at, w, h))
    }

    /// Lo que hay en (`x`, `y`) según el último cuadro; `None` fuera del panel.
    pub fn hit(&self, w: f32, h: f32, x: f32, y: f32) -> Option<GroupColorHit> {
        let l = self.laid(w, h);
        if !contains(&l.panel, x, y) {
            return None;
        }
        if let Some(i) = l.swatches.iter().position(|r| contains(r, x, y)) {
            return Some(GroupColorHit::Swatch(i));
        }
        Some(if contains(&l.app, x, y) { GroupColorHit::App } else { GroupColorHit::Panel })
    }

    /// Centro de una opción del último cuadro (pruebas de `App`): 0..12 colores, 12 el acento de la app.
    #[cfg(test)]
    pub fn option_center(&self, i: usize) -> (f32, f32) {
        let l = self.layout.borrow().clone().expect("el panel ya se dibujó");
        center(if i < 12 { &l.swatches[i] } else { &l.app })
    }

    /// Mueve el foco: izquierda/derecha de a uno, arriba/abajo de a una fila (la última fila es
    /// el acento de la aplicación).
    pub fn move_focus(&mut self, dx: isize, dy: isize) {
        const APP: usize = 12;
        let f = match self.focus {
            None => 0,
            Some(f) if f == APP => {
                if dy < 0 {
                    COLS + COLS / 2
                } else {
                    APP
                }
            }
            Some(f) => {
                let (row, col) = ((f / COLS) as isize, (f % COLS) as isize);
                if row + dy > 1 {
                    APP
                } else {
                    let row = (row + dy).clamp(0, 1);
                    let col = (col + dx).rem_euclid(COLS as isize);
                    (row * COLS as isize + col) as usize
                }
            }
        };
        self.focus = Some(f);
    }

    /// Pinta el panel sobre la ventana (`w` × `h`) y guarda su diseño para `hit`.
    pub fn draw(&self, p: &dyn Painter, w: f32, h: f32, hover: Option<GroupColorHit>) {
        let th = p.theme();
        let l = compute(self.at, w, h);
        p.round_fill(rect(l.panel.left - 2.0, l.panel.top, l.panel.right + 2.0, l.panel.bottom + 4.0), 10.0, alpha(th.background, 0.5));
        p.round_fill(l.panel, 8.0, th.surface_alt);
        p.round_stroke(l.panel, 8.0, th.border);
        for (i, (_, c)) in PRESETS.iter().enumerate() {
            let (cx, cy) = center(&l.swatches[i]);
            if self.current == Some(*c) {
                p.circle_stroke(cx, cy, SWATCH_R + 3.0, th.text, 2.0);
            } else if hover == Some(GroupColorHit::Swatch(i)) || self.focus == Some(i) {
                p.circle_stroke(cx, cy, SWATCH_R + 3.0, th.text_dim, 1.0);
            }
            p.circle(cx, cy, SWATCH_R, crate::ui::hex(*c));
            p.circle_stroke(cx, cy, SWATCH_R, alpha(th.text, 0.18), 1.0);
        }
        let lit = hover == Some(GroupColorHit::App) || self.focus == Some(12);
        if lit {
            p.round_fill(l.app, 6.0, alpha(th.accent, 0.22));
        }
        if self.current.is_none() {
            p.text(FontId::Glyphs, "\u{E73E}", rect(l.app.left + 6.0, l.app.top, l.app.left + 30.0, l.app.bottom), th.accent);
        }
        p.circle(l.app.left + 42.0, (l.app.top + l.app.bottom) / 2.0, 6.0, th.accent);
        p.text(FontId::Ui, "Acento de la aplicación", rect(l.app.left + 58.0, l.app.top, l.app.right - 6.0, l.app.bottom), if lit { th.accent } else { th.text });
        *self.layout.borrow_mut() = Some(l);
    }
}

impl App {
    /// Clic derecho sobre la pestaña `index` de la barra de grupos: abre el panel de colores.
    pub(super) fn open_group_color(&mut self, index: usize, at: (f32, f32)) {
        let Some(g) = self.groups.items.get(index) else { return };
        self.group_color = Some(GroupColorMenu::new(g.name.clone(), g.color, at));
        self.redraw();
    }

    /// Clic con el panel abierto: un color (o el acento de la aplicación) lo elige y cierra; el
    /// panel no hace nada; fuera del panel, lo cierra.
    pub(super) fn group_color_click(&mut self, hit: Option<GroupColorHit>) {
        match hit {
            Some(GroupColorHit::Swatch(i)) => self.group_color_pick(Some(PRESETS[i].1)),
            Some(GroupColorHit::App) => self.group_color_pick(None),
            Some(GroupColorHit::Panel) => {}
            None => self.group_color = None,
        }
        self.redraw();
    }

    /// Teclas del panel: Esc lo cierra, las flechas mueven el foco y Enter elige. Se queda con
    /// todas las teclas mientras está abierto.
    pub(super) fn group_color_key(&mut self, vk: Key) -> bool {
        if vk == VK_ESCAPE {
            self.group_color = None;
        } else if vk == VK_RETURN {
            match self.group_color.as_ref().and_then(|m| m.focus) {
                Some(i) if i < PRESETS.len() => self.group_color_pick(Some(PRESETS[i].1)),
                Some(_) => self.group_color_pick(None),
                None => {}
            }
        } else if let Some(m) = &mut self.group_color {
            match vk {
                VK_LEFT => m.move_focus(-1, 0),
                VK_RIGHT => m.move_focus(1, 0),
                VK_UP => m.move_focus(0, -1),
                VK_DOWN => m.move_focus(0, 1),
                _ => {}
            }
        }
        self.redraw();
        true
    }

    /// Le pone `color` al grupo del panel (`None`: el acento de la aplicación) y lo cierra.
    fn group_color_pick(&mut self, color: Option<u32>) {
        let Some(m) = self.group_color.take() else { return };
        let Some(i) = self.groups.find(&m.group) else { return };
        if self.groups.items[i].color != color {
            self.groups.items[i].color = color;
            if let Err(e) = self.groups.save() {
                self.notice = Some(format!("No se pudo guardar el grupo: {e}"));
            }
        }
    }
}

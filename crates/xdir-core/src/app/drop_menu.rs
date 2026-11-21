//! Menú desplegable con el estilo de xdir: un panel redondeado con sombra que se abre hacia arriba
//! de un botón del pie, con el tilde de lo actual a la izquierda y la fila bajo el mouse (o con el
//! foco del teclado) resaltada en el color de acento. Reemplaza al menú nativo del sistema, que no
//! tiene el aspecto de la aplicación.
//!
//! Como los demás paneles, el diseño de las partes se calcula al pintar (el ancho sale del texto
//! real) y se guarda para resolver los clics del cuadro siguiente.

use std::cell::RefCell;

use crate::geom::{Rect, contains, rect};
use crate::paint::{FontId, Painter, ellipsize};
use crate::theme::alpha;

const ROW_H: f32 = 32.0;
const PAD: f32 = 6.0;
/// Sangría de la fila resaltada respecto del panel.
const ROW_INSET: f32 = 4.0;
/// Lugar a la izquierda del texto para el tilde.
const GUTTER: f32 = 36.0;
const MIN_W: f32 = 220.0;
const MAX_W: f32 = 460.0;
/// Filas a la vista como mucho: el resto se recorre con la rueda o las flechas.
const MAX_ROWS: usize = 12;
/// Separación entre el botón y el panel.
const GAP: f32 = 6.0;
const MARGIN: f32 = 8.0;

pub struct DropItem {
    pub label: String,
    /// Lleva el tilde (lo que está en uso).
    pub checked: bool,
    /// Se puede elegir. Una fila sin tilde y deshabilitada es solo una nota y se ve tenue.
    pub enabled: bool,
}

impl DropItem {
    pub fn new(label: impl Into<String>, checked: bool, enabled: bool) -> DropItem {
        DropItem { label: label.into(), checked, enabled }
    }
}

/// Qué hay bajo el mouse: una fila (su posición en la lista) o el panel sin nada debajo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropHit {
    Item(usize),
    Panel,
}

#[derive(Clone)]
struct Layout {
    panel: Rect,
    /// Las filas a la vista: (posición en la lista, su rectángulo).
    rows: Vec<(usize, Rect)>,
}

pub struct DropMenu {
    pub items: Vec<DropItem>,
    /// El botón al que se pega el panel.
    anchor: Rect,
    /// Primera fila a la vista.
    first: usize,
    /// La fila con el foco del teclado.
    pub focus: Option<usize>,
    layout: RefCell<Layout>,
}

impl DropMenu {
    pub fn new(items: Vec<DropItem>, anchor: Rect) -> DropMenu {
        DropMenu { items, anchor, first: 0, focus: None, layout: RefCell::new(Layout { panel: rect(0.0, 0.0, 0.0, 0.0), rows: Vec::new() }) }
    }

    /// Lo que hay en (`x`, `y`) según el último cuadro; `None` fuera del panel.
    pub fn hit(&self, x: f32, y: f32) -> Option<DropHit> {
        let l = self.layout.borrow();
        if !contains(&l.panel, x, y) {
            return None;
        }
        Some(l.rows.iter().find(|(_, r)| contains(r, x, y)).map_or(DropHit::Panel, |(i, _)| DropHit::Item(*i)))
    }

    /// El panel del último cuadro (pruebas de `App`).
    #[cfg(test)]
    pub fn panel_rect(&self) -> Rect {
        self.layout.borrow().panel
    }

    /// ¿Se puede elegir la fila `i`?
    pub fn is_enabled(&self, i: usize) -> bool {
        self.items.get(i).is_some_and(|it| it.enabled)
    }

    /// La rueda: `rows` positivo sube (como en la paleta).
    pub fn on_wheel(&mut self, rows: i32) {
        let max = self.items.len().saturating_sub(self.shown());
        self.first = (self.first as i32 - rows).clamp(0, max as i32) as usize;
    }

    /// Mueve el foco a la fila habilitada anterior (`-1`) o siguiente (`1`), dando la vuelta, y la
    /// deja a la vista.
    pub fn move_focus(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        if n == 0 || !self.items.iter().any(|it| it.enabled) {
            return;
        }
        let mut i = self.focus.map_or(if delta > 0 { -1 } else { n }, |f| f as isize);
        loop {
            i = (i + delta).rem_euclid(n);
            if self.items[i as usize].enabled {
                break;
            }
        }
        self.focus = Some(i as usize);
        let shown = self.shown();
        if (i as usize) < self.first {
            self.first = i as usize;
        } else if i as usize >= self.first + shown {
            self.first = i as usize + 1 - shown;
        }
    }

    /// Cuántas filas se ven (según el último cuadro; antes del primero, el tope).
    fn shown(&self) -> usize {
        let n = self.layout.borrow().rows.len();
        if n == 0 { self.items.len().min(MAX_ROWS) } else { n }
    }

    /// Pinta el panel sobre la ventana (`w` × `h`) y guarda su diseño para `hit`.
    pub fn draw(&self, p: &dyn Painter, w: f32, h: f32, hover: Option<DropHit>) {
        let th = p.theme();
        // Alto disponible sobre el botón: de ahí salen las filas que entran.
        let room = (self.anchor.top - GAP - MARGIN - 2.0 * PAD).max(ROW_H);
        let rows = self.items.len().min(MAX_ROWS).min((room / ROW_H) as usize).max(1);
        let first = self.first.min(self.items.len().saturating_sub(rows));
        let overflow = self.items.len() > rows;
        let widest = self.items.iter().map(|it| p.measure(FontId::Ui, &it.label)).fold(0.0_f32, f32::max);
        let width = (widest + GUTTER + 20.0 + if overflow { 10.0 } else { 0.0 }).clamp(MIN_W, MAX_W).min(w - 2.0 * MARGIN);
        let height = rows as f32 * ROW_H + 2.0 * PAD;
        let left = (self.anchor.left - GAP).min(w - MARGIN - width).max(MARGIN);
        let bottom = (self.anchor.top - GAP).min(h - MARGIN);
        let panel = rect(left, bottom - height, left + width, bottom);
        p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 10.0, alpha(th.background, 0.5));
        p.round_fill(panel, 8.0, th.surface_alt);
        p.round_stroke(panel, 8.0, th.border);
        let mut laid = Vec::with_capacity(rows);
        for k in 0..rows {
            let i = first + k;
            let Some(item) = self.items.get(i) else { break };
            let top = panel.top + PAD + k as f32 * ROW_H;
            let row = rect(panel.left + ROW_INSET, top, panel.right - ROW_INSET - if overflow { 8.0 } else { 0.0 }, top + ROW_H);
            let lit = item.enabled && (hover == Some(DropHit::Item(i)) || self.focus == Some(i));
            if lit {
                p.round_fill(row, 6.0, alpha(th.accent, 0.22));
            }
            if item.checked {
                p.text(FontId::Glyphs, "\u{E73E}", rect(row.left + 6.0, row.top, row.left + GUTTER - 6.0, row.bottom), th.accent);
            }
            let color = if lit {
                th.accent
            } else if item.enabled || item.checked {
                th.text
            } else {
                th.text_dim
            };
            let text_r = rect(row.left + GUTTER - ROW_INSET, row.top, row.right - 8.0, row.bottom);
            p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &item.label, text_r.right - text_r.left), text_r, color);
            laid.push((i, row));
        }
        if overflow {
            // Una barra fina a la derecha: dónde está lo que se ve dentro de todo.
            let track = rect(panel.right - 7.0, panel.top + PAD, panel.right - 4.0, panel.bottom - PAD);
            let total = self.items.len() as f32;
            let (t0, t1) = (first as f32 / total, (first + rows) as f32 / total);
            let span = track.bottom - track.top;
            p.round_fill(rect(track.left, track.top + t0 * span, track.right, track.top + t1 * span), 1.5, th.scrollbar);
        }
        *self.layout.borrow_mut() = Layout { panel, rows: laid };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn painter() -> RecordingPainter {
        crate::sys::testing::install_fakes();
        RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(false)))
    }

    fn menu(n: usize) -> DropMenu {
        let items = (0..n).map(|i| DropItem::new(format!("rama-{i}"), i == 0, i != 0)).collect();
        DropMenu::new(items, rect(300.0, 700.0, 480.0, 728.0))
    }

    fn center(r: &Rect) -> (f32, f32) {
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    #[test]
    fn opens_above_the_button_inside_the_window() {
        let (p, m) = (painter(), menu(4));
        m.draw(&p, 1000.0, 800.0, None);
        let r = m.layout.borrow().panel;
        assert!(r.bottom <= 700.0 - GAP + 0.01, "termina sobre el botón: {r:?}");
        assert!(r.left >= MARGIN && r.right <= 1000.0 - MARGIN && r.top >= MARGIN, "{r:?}");
        assert!(r.right - r.left >= MIN_W);
    }

    #[test]
    fn shifts_left_to_fit_in_a_narrow_window() {
        let (p, m) = (painter(), menu(3));
        m.draw(&p, 400.0, 800.0, None);
        let r = m.layout.borrow().panel;
        assert!(r.right <= 400.0 - MARGIN + 0.01 && r.left >= MARGIN, "{r:?}");
    }

    #[test]
    fn draws_check_on_the_current_and_dims_notes() {
        let (p, m) = (painter(), menu(3));
        m.draw(&p, 1000.0, 800.0, None);
        let t = p.text_draws();
        let th = p.theme();
        assert!(t.iter().any(|(f, s, _, c)| *f == FontId::Glyphs && s == "\u{E73E}" && *c == th.accent), "tilde en acento");
        let color_of = |label: &str| t.iter().find(|(_, s, _, _)| s == label).map(|x| x.3).unwrap();
        assert_eq!(color_of("rama-0"), th.text, "la actual (marcada) no se ve tenue");
        assert_eq!(color_of("rama-1"), th.text);
        let notes = DropMenu::new(vec![DropItem::new("(sin ramas)", false, false)], rect(300.0, 700.0, 480.0, 728.0));
        let p2 = painter();
        notes.draw(&p2, 1000.0, 800.0, None);
        assert_eq!(p2.text_draws().iter().find(|x| x.1 == "(sin ramas)").unwrap().3, th.text_dim, "una nota se ve tenue");
    }

    #[test]
    fn hovered_row_is_tinted_and_text_goes_accent() {
        let (p, m) = (painter(), menu(3));
        m.draw(&p, 1000.0, 800.0, None);
        let th = p.theme();
        let n_before = p.round_fills().len();
        let p = painter();
        m.draw(&p, 1000.0, 800.0, Some(DropHit::Item(2)));
        let fills = p.round_fills();
        assert!(fills.iter().any(|(_, c)| *c == alpha(th.accent, 0.22)), "fila resaltada");
        assert!(fills.len() > n_before, "una pieza más que sin hover");
        assert_eq!(p.text_draws().iter().find(|x| x.1 == "rama-2").unwrap().3, th.accent);
        // Una fila deshabilitada no se resalta aunque el mouse esté encima.
        let p = painter();
        m.draw(&p, 1000.0, 800.0, Some(DropHit::Item(0)));
        assert!(!p.round_fills().iter().any(|(_, c)| *c == alpha(th.accent, 0.22)));
    }

    #[test]
    fn hit_finds_rows_panel_and_outside() {
        let (p, m) = (painter(), menu(4));
        assert_eq!(m.hit(400.0, 600.0), None, "antes del primer cuadro no hay nada");
        m.draw(&p, 1000.0, 800.0, None);
        let l = m.layout.borrow().clone();
        for (i, r) in &l.rows {
            let (x, y) = center(r);
            assert_eq!(m.hit(x, y), Some(DropHit::Item(*i)));
        }
        assert_eq!(m.hit(l.panel.left + 1.0, l.panel.top + 1.0), Some(DropHit::Panel));
        assert_eq!(m.hit(1.0, 1.0), None);
    }

    #[test]
    fn long_lists_show_a_window_and_scroll_with_the_wheel() {
        let (p, mut m) = (painter(), menu(40));
        m.draw(&p, 1000.0, 800.0, None);
        assert_eq!(m.layout.borrow().rows.len(), MAX_ROWS);
        assert_eq!(m.layout.borrow().rows[0].0, 0);
        m.on_wheel(-3);
        m.draw(&painter(), 1000.0, 800.0, None);
        assert_eq!(m.layout.borrow().rows[0].0, 3, "bajó tres filas");
        m.on_wheel(-1000);
        m.draw(&painter(), 1000.0, 800.0, None);
        assert_eq!(m.layout.borrow().rows.last().unwrap().0, 39, "llega al final y no pasa");
        m.on_wheel(1000);
        m.draw(&painter(), 1000.0, 800.0, None);
        assert_eq!(m.layout.borrow().rows[0].0, 0, "vuelve al principio");
    }

    #[test]
    fn short_windows_show_fewer_rows() {
        let m = DropMenu::new((0..40).map(|i| DropItem::new(format!("r{i}"), false, true)).collect(), rect(300.0, 150.0, 480.0, 178.0));
        m.draw(&painter(), 1000.0, 400.0, None);
        let l = m.layout.borrow().clone();
        assert!(l.rows.len() < MAX_ROWS && !l.rows.is_empty(), "{}", l.rows.len());
        assert!(l.panel.top >= MARGIN, "{:?}", l.panel);
    }

    #[test]
    fn keyboard_focus_skips_disabled_wraps_and_scrolls_into_view() {
        let mut m = menu(40);
        m.draw(&painter(), 1000.0, 800.0, None);
        m.move_focus(1);
        assert_eq!(m.focus, Some(1), "salta la fila 0 (deshabilitada)");
        m.move_focus(-1);
        assert_eq!(m.focus, Some(39), "da la vuelta hacia atrás");
        m.draw(&painter(), 1000.0, 800.0, None);
        assert!(m.layout.borrow().rows.iter().any(|(i, _)| *i == 39), "el foco quedó a la vista");
        m.move_focus(1);
        assert_eq!(m.focus, Some(1), "da la vuelta hacia adelante");
    }

    #[test]
    fn focus_does_nothing_when_nothing_can_be_chosen() {
        let mut m = DropMenu::new(vec![DropItem::new("(sin ramas)", false, false)], rect(300.0, 700.0, 480.0, 728.0));
        m.move_focus(1);
        assert_eq!(m.focus, None);
        assert!(!m.is_enabled(0) && !m.is_enabled(9));
    }

    #[test]
    fn long_labels_are_cut_with_an_ellipsis() {
        let m = DropMenu::new(vec![DropItem::new("x".repeat(200), false, true)], rect(300.0, 700.0, 480.0, 728.0));
        let p = painter();
        m.draw(&p, 1000.0, 800.0, None);
        let r = m.layout.borrow().panel;
        let shown = p.texts().into_iter().find(|t| t.1.starts_with('x')).unwrap();
        assert!(shown.1.ends_with('…'), "{}", shown.1);
        assert!(shown.2.right <= r.right, "el texto no se sale del panel");
    }
}

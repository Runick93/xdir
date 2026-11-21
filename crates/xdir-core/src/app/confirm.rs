//! Diálogo de confirmación con el estilo de xdir: el mismo marco y velo que los paneles de red,
//! un título, un texto que se parte en renglones y una fila de botones con nombre propio. Reemplaza
//! a los cuadros nativos del sistema (Sí / No / Cancelar), que no tienen el aspecto de la
//! aplicación ni dejan nombrar los botones.
//!
//! El diseño de las partes se calcula al pintar (el texto se parte según su ancho real) y se
//! guarda para resolver los clics del cuadro siguiente, como hacen los paneles.

use std::cell::RefCell;

use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::theme::alpha;

use super::tooltip::wrap_lines;

const WIDTH: f32 = 460.0;
const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Cómo se pinta un botón: el principal (acento), el que destruye algo (rojo) o uno común.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
    Primary,
    Danger,
    Plain,
}

pub struct ConfirmButton {
    pub label: String,
    pub kind: ButtonKind,
}

impl ConfirmButton {
    pub fn new(label: impl Into<String>, kind: ButtonKind) -> ConfirmButton {
        ConfirmButton { label: label.into(), kind }
    }
}

/// Qué hay bajo el mouse: un botón (su posición), la cruz del título o el panel sin nada debajo.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfirmHit {
    Button(usize),
    Close,
    Panel,
}

#[derive(Clone)]
struct Layout {
    panel: Rect,
    close: Rect,
    buttons: Vec<Rect>,
}

fn zero() -> Rect {
    rect(0.0, 0.0, 0.0, 0.0)
}

pub struct Confirm {
    pub title: String,
    pub text: String,
    /// De izquierda a derecha. Por convención el último es «Cancelar».
    pub buttons: Vec<ConfirmButton>,
    /// El botón con el foco del teclado (Enter lo aprieta).
    pub focus: usize,
    layout: RefCell<Layout>,
}

impl Confirm {
    pub fn new(title: impl Into<String>, text: impl Into<String>, buttons: Vec<ConfirmButton>, focus: usize) -> Confirm {
        let focus = focus.min(buttons.len().saturating_sub(1));
        Confirm { title: title.into(), text: text.into(), buttons, focus, layout: RefCell::new(Layout { panel: zero(), close: zero(), buttons: Vec::new() }) }
    }

    /// Mueve el foco `delta` botones, dando la vuelta en los extremos.
    pub fn move_focus(&mut self, delta: isize) {
        let n = self.buttons.len() as isize;
        if n > 0 {
            self.focus = (self.focus as isize + delta).rem_euclid(n) as usize;
        }
    }

    /// Lo que hay en (`x`, `y`) según el último cuadro; `None` fuera del panel. Antes del primer
    /// cuadro no hay nada que acertar.
    pub fn hit(&self, x: f32, y: f32) -> Option<ConfirmHit> {
        let l = self.layout.borrow();
        if !contains(&l.panel, x, y) {
            return None;
        }
        if contains(&l.close, x, y) {
            return Some(ConfirmHit::Close);
        }
        Some(l.buttons.iter().position(|r| contains(r, x, y)).map_or(ConfirmHit::Panel, ConfirmHit::Button))
    }

    /// Pinta el diálogo sobre toda la ventana (`w` × `h`) y guarda su diseño para `hit`.
    pub fn draw(&self, p: &dyn Painter, w: f32, h: f32, hover: Option<ConfirmHit>) {
        let th = p.theme();
        let lines = wrap_lines(&|s| p.measure(FontId::Ui, s), &self.text, WIDTH - 32.0);
        let height = 44.0 + 14.0 + lines.len() as f32 * 20.0 + 20.0 + 58.0;
        let left = ((w - WIDTH) / 2.0).max(8.0);
        // Centrado en la ventana, a lo alto y a lo ancho (si no entra, pegado arriba con un margen).
        let top = ((h - height) / 2.0).max(8.0);
        let panel = rect(left, top, (left + WIDTH).min(w - 8.0), top + height);
        let close = rect(panel.right - 36.0, panel.top + 8.0, panel.right - 8.0, panel.top + 36.0);
        // Los botones van pegados a la derecha, en la franja de abajo.
        let mut buttons = vec![zero(); self.buttons.len()];
        let mut right = panel.right - 16.0;
        for (i, b) in self.buttons.iter().enumerate().rev() {
            let bw = (p.measure(FontId::Ui, &b.label) + 32.0).max(96.0);
            buttons[i] = rect(right - bw, panel.bottom - 44.0, right, panel.bottom - 12.0);
            right = buttons[i].left - 8.0;
        }
        crate::net::drives_dialog::frame(p, w, h, panel, &self.title, close, hover == Some(ConfirmHit::Close));
        for (i, line) in lines.iter().enumerate() {
            let y = panel.top + 58.0 + i as f32 * 20.0;
            p.text(FontId::Ui, line, rect(panel.left + 16.0, y, panel.right - 16.0, y + 20.0), th.text_secondary);
        }
        for (i, (b, r)) in self.buttons.iter().zip(&buttons).enumerate() {
            let hot = hover == Some(ConfirmHit::Button(i));
            let text_r = rect(r.left, r.top + 8.0, r.right, r.bottom - 6.0);
            match b.kind {
                ButtonKind::Primary | ButtonKind::Danger => {
                    let c = if b.kind == ButtonKind::Primary { th.accent } else { th.danger };
                    p.round_fill(*r, 6.0, if hot { alpha(c, 0.85) } else { c });
                    p.text(FontId::UiCenter, &b.label, text_r, WHITE);
                }
                ButtonKind::Plain => {
                    p.round_fill(*r, 6.0, if hot { th.hover } else { th.surface });
                    p.round_stroke(*r, 6.0, th.border);
                    p.text(FontId::UiCenter, &b.label, text_r, th.text);
                }
            }
            if i == self.focus {
                p.round_stroke_w(rect(r.left - 2.0, r.top - 2.0, r.right + 2.0, r.bottom + 2.0), 8.0, alpha(th.text, 0.8), 1.5);
            }
        }
        *self.layout.borrow_mut() = Layout { panel, close, buttons };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn dialog(text: &str) -> Confirm {
        Confirm::new(
            "Título",
            text,
            vec![ConfirmButton::new("Guardar", ButtonKind::Primary), ConfirmButton::new("Descartar", ButtonKind::Danger), ConfirmButton::new("Cancelar", ButtonKind::Plain)],
            0,
        )
    }

    fn painter() -> RecordingPainter {
        crate::sys::testing::install_fakes();
        RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(false)))
    }

    fn center(r: &Rect) -> (f32, f32) {
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    #[test]
    fn draws_title_text_and_labels() {
        let (p, d) = (painter(), dialog("Un texto corto."));
        d.draw(&p, 1000.0, 800.0, None);
        let t: Vec<String> = p.texts().into_iter().map(|x| x.1).collect();
        for s in ["Título", "Un texto corto.", "Guardar", "Descartar", "Cancelar"] {
            assert!(t.iter().any(|x| x == s), "{s}: {t:?}");
        }
    }

    #[test]
    fn long_text_wraps_into_several_lines_and_grows_the_panel() {
        let long = "palabra ".repeat(60);
        let (p1, d1) = (painter(), dialog("corto"));
        d1.draw(&p1, 1000.0, 800.0, None);
        let (p2, d2) = (painter(), dialog(&long));
        d2.draw(&p2, 1000.0, 800.0, None);
        let (a, b) = (d1.layout.borrow().panel, d2.layout.borrow().panel);
        assert!(b.bottom - b.top > a.bottom - a.top, "el panel crece con el texto");
        assert!(p2.texts().iter().filter(|x| x.1.contains("palabra")).count() > 1, "el texto se parte en renglones");
    }

    #[test]
    fn hits_buttons_close_panel_and_outside() {
        let (p, d) = (painter(), dialog("Texto"));
        assert_eq!(d.hit(500.0, 300.0), None, "antes del primer cuadro no hay nada");
        d.draw(&p, 1000.0, 800.0, None);
        let l = d.layout.borrow().clone();
        for (i, r) in l.buttons.iter().enumerate() {
            let (x, y) = center(r);
            assert_eq!(d.hit(x, y), Some(ConfirmHit::Button(i)));
        }
        let (x, y) = center(&l.close);
        assert_eq!(d.hit(x, y), Some(ConfirmHit::Close));
        assert_eq!(d.hit(l.panel.left + 20.0, l.panel.top + 60.0), Some(ConfirmHit::Panel));
        assert_eq!(d.hit(1.0, 1.0), None);
        assert!(l.buttons.windows(2).all(|w| w[0].right < w[1].left), "los botones no se pisan");
        assert!(l.buttons.iter().all(|r| r.right <= l.panel.right - 16.0 + 0.01 && r.left >= l.panel.left));
    }

    #[test]
    fn panel_is_centered_in_the_window() {
        for (w, h) in [(1000.0_f32, 800.0_f32), (1366.0, 700.0), (520.0, 600.0)] {
            let (p, d) = (painter(), dialog("Un texto algo más largo para que ocupe más de un renglón y se vea el panel entero."));
            d.draw(&p, w, h, None);
            let r = d.layout.borrow().panel;
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            assert!((cx - w / 2.0).abs() < 0.5, "centrado a lo ancho en {w}x{h}: {cx}");
            assert!((cy - h / 2.0).abs() < 0.5, "centrado a lo alto en {w}x{h}: {cy}");
        }
    }

    #[test]
    fn panel_taller_than_the_window_sticks_to_the_top() {
        let (p, d) = (painter(), dialog(&"palabra ".repeat(200)));
        d.draw(&p, 1000.0, 200.0, None);
        assert_eq!(d.layout.borrow().panel.top, 8.0);
    }

    #[test]
    fn focus_moves_and_wraps() {
        let mut d = dialog("x");
        assert_eq!(d.focus, 0);
        d.move_focus(-1);
        assert_eq!(d.focus, 2);
        d.move_focus(1);
        assert_eq!(d.focus, 0);
        d.move_focus(2);
        assert_eq!(d.focus, 2);
    }

    #[test]
    fn initial_focus_is_clamped() {
        let d = Confirm::new("t", "x", vec![ConfirmButton::new("Ok", ButtonKind::Plain)], 9);
        assert_eq!(d.focus, 0);
    }
}

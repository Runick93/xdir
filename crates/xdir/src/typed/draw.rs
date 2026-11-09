//! Dibujo del desplegable de rutas bajo la barra de dirección.

use xdir_core::typed::{MAX_ROWS, Suggest};

use crate::render::{Painter, rect};
use crate::theme::alpha;

/// `Suggest::draw`: el desplegable se dibuja con el `Painter` de la ventana.
pub trait SuggestDraw {
    fn draw(&self, p: &Painter);
}

impl SuggestDraw for Suggest {
    fn draw(&self, p: &Painter) {
        if self.items.is_empty() {
            return;
        }
        let th = p.theme;
        let panel = self.rect();
        let scroll = self.scroll();
        p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 10.0, alpha(th.background, 0.6));
        p.round_fill(panel, 8.0, th.surface_alt);
        p.round_stroke(panel, 8.0, th.border);
        for (i, item) in self.items.iter().enumerate().skip(scroll).take(MAX_ROWS) {
            let r = self.row_rect(i - scroll);
            if self.selected == Some(i) {
                p.round_fill(r, 5.0, alpha(th.accent, 0.22));
            } else if self.hover == Some(i) {
                p.round_fill(r, 5.0, th.hover);
            }
            p.text(&p.fonts.ui, item, rect(r.left + 8.0, r.top, r.right - 8.0, r.bottom), th.text);
        }
    }
}

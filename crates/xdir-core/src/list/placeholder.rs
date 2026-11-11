//! Lo que muestra la lista de una carpeta de red que tarda (en lugar de la lista vacía):
//! «Conectando con "nas"…» con un indicador y, a los 20 s sin respuesta, «"nas" no responde» con
//! «Reintentar» y «Cerrar la pestaña».

use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::theme::alpha;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placeholder {
    Connecting(String),
    NotResponding(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceholderButton {
    Retry,
    CloseTab,
}

impl Placeholder {
    pub fn text(&self) -> String {
        match self {
            Placeholder::Connecting(h) => format!("Conectando con \"{h}\"…"),
            Placeholder::NotResponding(h) => format!("\"{h}\" no responde"),
        }
    }
}

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Dibuja el aviso centrado en `body` y devuelve dónde quedaron sus botones.
pub fn draw(p: &dyn Painter, body: Rect, ph: &Placeholder, hover: Option<PlaceholderButton>) -> Vec<(Rect, PlaceholderButton)> {
    let th = p.theme();
    p.fill(body, th.surface);
    let cx = (body.left + body.right) / 2.0;
    let cy = (body.top + body.bottom) / 2.0;
    let line = |top: f32| rect(body.left + 16.0, top, body.right - 16.0, top + 20.0);
    match ph {
        Placeholder::Connecting(_) => {
            p.circle_stroke(cx, cy - 26.0, 8.0, alpha(th.text_dim, 0.4), 2.0);
            p.rounded_path(&[(cx, cy - 34.0, 0.0), (cx + 8.0, cy - 26.0, 0.0)], false, None, Some((th.accent, 2.0)));
            p.text(FontId::UiCenter, &ph.text(), line(cy - 8.0), th.text_dim);
            Vec::new()
        }
        Placeholder::NotResponding(_) => {
            p.text(FontId::Glyphs, "\u{E7BA}", rect(cx - 10.0, cy - 46.0, cx + 10.0, cy - 28.0), th.warning);
            p.text(FontId::UiCenter, &ph.text(), line(cy - 22.0), th.text);
            let retry = rect(cx - 126.0, cy + 6.0, cx - 6.0, cy + 34.0);
            let close = rect(cx + 2.0, cy + 6.0, cx + 142.0, cy + 34.0);
            let hot = |b| hover == Some(b);
            p.round_fill(retry, 6.0, if hot(PlaceholderButton::Retry) { alpha(th.accent, 0.85) } else { th.accent });
            p.text(FontId::UiCenter, "Reintentar", rect(retry.left, retry.top + 6.0, retry.right, retry.bottom - 4.0), WHITE);
            p.round_fill(close, 6.0, if hot(PlaceholderButton::CloseTab) { th.hover } else { th.surface_alt });
            p.round_stroke(close, 6.0, th.border);
            p.text(FontId::UiCenter, "Cerrar la pestaña", rect(close.left, close.top + 6.0, close.right, close.bottom - 4.0), th.text);
            vec![(retry, PlaceholderButton::Retry), (close, PlaceholderButton::CloseTab)]
        }
    }
}

pub fn hit(buttons: &[(Rect, PlaceholderButton)], x: f32, y: f32) -> Option<PlaceholderButton> {
    buttons.iter().find(|(r, _)| contains(r, x, y)).map(|(_, b)| *b)
}

//! Controles de las filas de la configuración: switch, segmentado, desplegable, deslizador y
//! botón. Cada uno se dibuja en el rectángulo que recibe; las medidas salen del diseño (tanda D+).

use crate::geom::{Rect, rect};
use crate::paint::{FontId, Painter};
use crate::theme::alpha;

pub const SWITCH_W: f32 = 40.0;
pub const SWITCH_H: f32 = 20.0;
pub const SEGMENT_W: f32 = 88.0;
pub const CONTROL_H: f32 = 28.0;
pub const DROPDOWN_W: f32 = 220.0;
pub const SLIDER_W: f32 = 200.0;
pub const SLIDER_VALUE_W: f32 = 56.0;
pub const BUTTON_W: f32 = 180.0;

/// Switch: pastilla de acento con la perilla a la derecha cuando está prendido.
pub fn switch(p: &dyn Painter, r: Rect, on: bool, enabled: bool, hot: bool) {
    let th = p.theme();
    let half = (r.bottom - r.top) / 2.0;
    let cy = r.top + half;
    let dim = |c| if enabled { c } else { alpha(c, 0.45) };
    if on {
        p.round_fill(r, half, dim(th.accent));
    } else {
        if hot && enabled {
            p.round_fill(r, half, th.hover);
        }
        p.round_stroke_w(r, half, dim(th.text_dim), 1.5);
    }
    let (kx, kr) = if on { (r.right - half, half - 4.0) } else { (r.left + half, half - 5.0) };
    p.circle(kx, cy, kr, dim(if on { th.background } else { th.text_dim }));
}

/// Segmentado: una opción por tramo de `SEGMENT_W`; la elegida resaltada. Devuelve cada tramo.
pub fn segmented(p: &dyn Painter, r: Rect, options: &[&str], selected: Option<usize>, hot: Option<usize>) -> Vec<Rect> {
    let th = p.theme();
    p.round_fill(r, 6.0, th.background);
    p.round_stroke(r, 6.0, th.border);
    let w = (r.right - r.left) / options.len().max(1) as f32;
    let mut parts = Vec::new();
    for (i, label) in options.iter().enumerate() {
        let s = rect(r.left + w * i as f32, r.top, r.left + w * (i + 1) as f32, r.bottom);
        let inner = rect(s.left + 2.0, s.top + 2.0, s.right - 2.0, s.bottom - 2.0);
        let chosen = selected == Some(i);
        if chosen {
            p.round_fill(inner, 5.0, alpha(th.accent, 0.28));
        } else if hot == Some(i) {
            p.round_fill(inner, 5.0, th.hover);
        }
        if i > 0 && !chosen && selected != Some(i - 1) {
            p.fill(rect(s.left, s.top + 7.0, s.left + 1.0, s.bottom - 7.0), th.border);
        }
        let font = if chosen { FontId::UiBold } else { FontId::Ui };
        // UiBold va alineado a la izquierda: se centra a mano con su ancho.
        let tw = p.measure(font, label);
        let x = ((s.left + s.right) / 2.0 - tw / 2.0).max(s.left + 4.0);
        p.text(font, label, rect(x, s.top, s.right - 2.0, s.bottom), th.text);
        parts.push(s);
    }
    parts
}

/// Desplegable cerrado: lo elegido y la flecha.
pub fn dropdown(p: &dyn Painter, r: Rect, text: &str, hot: bool, enabled: bool) {
    let th = p.theme();
    p.round_fill(r, 6.0, if hot && enabled { th.hover } else { th.background });
    p.round_stroke(r, 6.0, th.border);
    let color = if enabled { th.text } else { alpha(th.text_dim, 0.6) };
    p.text(FontId::Ui, text, rect(r.left + 10.0, r.top, r.right - 30.0, r.bottom), color);
    p.text(FontId::GlyphsSmall, "\u{E70D}", rect(r.right - 26.0, r.top, r.right - 6.0, r.bottom), th.text_dim);
}

/// Deslizador: riel, tramo lleno hasta el valor y perilla. `r` es el riel (sin el número).
pub fn slider(p: &dyn Painter, r: Rect, value: f32, min: f32, max: f32, hot: bool) {
    let th = p.theme();
    let cy = (r.top + r.bottom) / 2.0;
    let t = if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
    let x = r.left + 8.0 + (r.right - r.left - 16.0) * t;
    p.round_fill(rect(r.left + 8.0, cy - 2.0, r.right - 8.0, cy + 2.0), 2.0, th.border);
    p.round_fill(rect(r.left + 8.0, cy - 2.0, x, cy + 2.0), 2.0, th.accent);
    if hot {
        p.circle(x, cy, 10.0, alpha(th.accent, 0.25));
    }
    p.circle(x, cy, 7.0, th.accent);
}

/// Valor del deslizador en la posición `x` del riel `r`, redondeado y dentro del rango.
pub fn slider_value(r: Rect, x: f32, min: f32, max: f32) -> f32 {
    let span = (r.right - r.left - 16.0).max(1.0);
    let t = ((x - r.left - 8.0) / span).clamp(0.0, 1.0);
    (min + (max - min) * t).round().clamp(min, max)
}

/// Botón con borde.
pub fn button(p: &dyn Painter, r: Rect, label: &str, hot: bool, enabled: bool) {
    let th = p.theme();
    p.round_fill(r, 6.0, if hot && enabled { th.hover } else { th.surface });
    p.round_stroke(r, 6.0, th.border);
    let color = if enabled { th.text } else { alpha(th.text_dim, 0.6) };
    p.text(FontId::UiCenter, label, r, color);
}

/// Opción redonda («Del sistema», «Otro…»).
pub fn radio(p: &dyn Painter, r: Rect, label: &str, on: bool, hot: bool) {
    let th = p.theme();
    if hot {
        p.round_fill(r, 6.0, th.hover);
    }
    let (cx, cy) = (r.left + 12.0, (r.top + r.bottom) / 2.0);
    p.circle_stroke(cx, cy, 7.0, if on { th.accent } else { th.text_dim }, 1.5);
    if on {
        p.circle(cx, cy, 4.0, th.accent);
    }
    p.text(FontId::Ui, label, rect(r.left + 26.0, r.top, r.right, r.bottom), th.text);
}

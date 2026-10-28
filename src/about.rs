//! Diálogo "Acerca de": logo, versión y enlaces para reportar un problema. Es un panel dibujado
//! como el resto de la interfaz (sin ventana de Windows aparte); un clic fuera o Esc lo cierra.

use crate::icons::Pixels;
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ISSUES_URL: &str = "https://github.com/Runick93/xdir/issues/new/choose";

/// Logo de Runick: `assets/runick-logo.bgra` (ancho y alto como u32 little-endian y luego BGRA
/// premultiplicado), generado a partir de `assets/runick-logo.png`.
static LOGO: &[u8] = include_bytes!("../assets/runick-logo.bgra");

pub fn logo_pixels() -> Option<Pixels> {
    let width = u32::from_le_bytes(LOGO.get(0..4)?.try_into().ok()?);
    let height = u32::from_le_bytes(LOGO.get(4..8)?.try_into().ok()?);
    let data = LOGO.get(8..)?.to_vec();
    (data.len() == (width * height * 4) as usize).then_some(Pixels { width, height, data })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AboutHit {
    /// Buscar actualizaciones, o instalar la que hay.
    Update,
    Report,
    /// Dentro del diálogo pero sobre nada.
    Panel,
}

const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 272.0;
const BANNER_H: f32 = 108.0;
const BTN_H: f32 = 34.0;
/// El logo es de letras blancas: va sobre una franja oscura aunque el tema sea claro.
const BANNER: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.055, g: 0.059, b: 0.078, a: 1.0 };

fn panel(w: f32, h: f32) -> Rect {
    let (cx, cy) = (w / 2.0, h / 2.0);
    rect(cx - WIDTH / 2.0, cy - HEIGHT / 2.0, cx + WIDTH / 2.0, cy + HEIGHT / 2.0)
}

fn report_button(w: f32, h: f32) -> Rect {
    let p = panel(w, h);
    rect(p.left + 16.0, p.bottom - 16.0 - BTN_H, p.right - 16.0, p.bottom - 16.0)
}

fn update_button(w: f32, h: f32) -> Rect {
    let r = report_button(w, h);
    rect(r.left, r.top - BTN_H - 4.0, r.right, r.bottom - BTN_H - 4.0)
}

pub fn at(w: f32, h: f32, x: f32, y: f32) -> Option<AboutHit> {
    if !contains(&panel(w, h), x, y) {
        return None;
    }
    Some(if contains(&report_button(w, h), x, y) {
        AboutHit::Report
    } else if contains(&update_button(w, h), x, y) {
        AboutHit::Update
    } else {
        AboutHit::Panel
    })
}

/// `update`: el texto del botón de actualizaciones y si hay una versión nueva (se resalta).
pub fn draw(p: &Painter, w: f32, h: f32, hover: Option<AboutHit>, update: (&str, bool)) {
    let th = p.theme;
    // Velo sobre la ventana, para que el diálogo se lea como modal.
    p.fill(rect(0.0, 0.0, w, h), D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.45 });
    let pn = panel(w, h);
    p.round_fill(rect(pn.left - 2.0, pn.top, pn.right + 2.0, pn.bottom + 4.0), 14.0, alpha(th.background, 0.5));
    p.round_fill(pn, 12.0, th.surface_alt);
    // Franja del logo: el panel entero en oscuro, recortado a la parte de arriba.
    p.clip(rect(pn.left, pn.top, pn.right, pn.top + BANNER_H), || p.round_fill(pn, 12.0, BANNER));
    p.logo(rect(pn.left + 28.0, pn.top + 14.0, pn.right - 28.0, pn.top + BANNER_H - 14.0));
    p.round_stroke(pn, 12.0, th.border);

    let line = |top: f32, bottom: f32| rect(pn.left + 20.0, top, pn.right - 20.0, bottom);
    p.text(&p.fonts.ui_bold, &format!("xdir {VERSION}"), line(pn.top + BANNER_H + 8.0, pn.top + BANNER_H + 34.0), th.text);
    p.text(
        &p.fonts.ui,
        "Explorador de archivos multipanel para Windows.",
        line(pn.top + BANNER_H + 34.0, pn.top + BANNER_H + 58.0),
        th.text_dim,
    );

    let r = update_button(w, h);
    if hover == Some(AboutHit::Update) {
        p.round_fill(r, 6.0, th.hover);
    }
    let (label, new) = update;
    p.text(&p.fonts.glyphs, "\u{E895}", rect(r.left + 6.0, r.top, r.left + 32.0, r.bottom), if new { th.accent } else { th.text_dim });
    if new {
        // El último tramo del rastro: engranaje -> "Acerca de xdir" -> este botón.
        p.circle(r.left + 29.0, r.top + 9.0, 4.0, th.accent);
    }
    p.text(if new { &p.fonts.ui_bold } else { &p.fonts.ui }, label, rect(r.left + 38.0, r.top, r.right - 8.0, r.bottom), if new { th.accent } else { th.text });

    let r = report_button(w, h);
    if hover == Some(AboutHit::Report) {
        p.round_fill(r, 6.0, th.hover);
    }
    p.text(&p.fonts.glyphs, "\u{E783}", rect(r.left + 6.0, r.top, r.left + 32.0, r.bottom), th.text_dim);
    p.text(&p.fonts.ui, "Reportar un problema o sugerir algo", rect(r.left + 38.0, r.top, r.right - 8.0, r.bottom), th.text);
}

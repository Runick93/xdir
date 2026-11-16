//! Diálogo "Acerca de": logo, versión, Novedades y el enlace para reportar un problema. Es un panel dibujado
//! como el resto de la interfaz (sin ventana de Windows aparte); un clic fuera o Esc lo cierra.

use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter, Pixels};
use crate::theme::alpha;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ISSUES_URL: &str = "https://github.com/Runick93/xdir/issues/new/choose";

/// Logo de Runick, animado: `assets/runick-logo.bgra` (ancho, alto y cantidad de cuadros como u32
/// little-endian, la duración en ms de cada cuadro y luego los cuadros en BGRA premultiplicado),
/// generado por `scripts/make-logo.py` a partir de `assets/runick-logo.gif`.
static LOGO: &[u8] = include_bytes!("../../../assets/runick-logo.bgra");

fn u32_at(i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(LOGO.get(i * 4..i * 4 + 4)?.try_into().ok()?))
}

/// (ancho, alto, cuadros).
fn logo_header() -> Option<(u32, u32, usize)> {
    let n = u32_at(2)? as usize;
    (n > 0).then_some((u32_at(0)?, u32_at(1)?, n))
}

/// Duración en ms de cada cuadro del logo.
fn logo_durations() -> Vec<u32> {
    let n = logo_header().map_or(0, |h| h.2);
    (0..n).filter_map(|i| u32_at(3 + i)).collect()
}

/// Cantidad de cuadros del logo.
pub fn logo_frame_count() -> usize {
    logo_header().map_or(0, |h| h.2)
}

/// Píxeles del cuadro `i` del logo.
pub fn logo_pixels(i: usize) -> Option<Pixels> {
    let (width, height, n) = logo_header()?;
    let size = (width * height * 4) as usize;
    let start = (3 + n) * 4 + i.checked_mul(size)?;
    let data = LOGO.get(start..start + size)?.to_vec();
    (i < n).then_some(Pixels { width, height, data })
}

/// Cuadro del logo a los `elapsed_ms` de abierto el diálogo (la animación se repite) y los ms que
/// faltan para el siguiente.
pub fn logo_frame_at(elapsed_ms: u64) -> (usize, u64) {
    let durs = logo_durations();
    let total: u64 = durs.iter().map(|&d| d as u64).sum();
    if total == 0 {
        return (0, 1000);
    }
    let mut t = elapsed_ms % total;
    for (i, &d) in durs.iter().enumerate() {
        if t < d as u64 {
            return (i, d as u64 - t);
        }
        t -= d as u64;
    }
    (0, 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AboutHit {
    /// «Novedades»: abre la pestaña y cierra el diálogo.
    Changelog,
    Report,
    /// Dentro del diálogo pero sobre nada.
    Panel,
}

const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 272.0;
const BANNER_H: f32 = 108.0;
const BTN_H: f32 = 34.0;
/// Ancho del cuadro del logo.
const LOGO_W: f32 = 236.0;
/// El logo es de letras blancas: va sobre una franja oscura aunque el tema sea claro.
const BANNER: Color = Color { r: 0.055, g: 0.059, b: 0.078, a: 1.0 };

fn panel(w: f32, h: f32) -> Rect {
    let (cx, cy) = (w / 2.0, h / 2.0);
    rect(cx - WIDTH / 2.0, cy - HEIGHT / 2.0, cx + WIDTH / 2.0, cy + HEIGHT / 2.0)
}

fn report_button(w: f32, h: f32) -> Rect {
    let p = panel(w, h);
    rect(p.left + 16.0, p.bottom - 16.0 - BTN_H, p.right - 16.0, p.bottom - 16.0)
}

/// «Novedades», arriba del de reportar (4 entre los dos).
fn changelog_button(w: f32, h: f32) -> Rect {
    let r = report_button(w, h);
    rect(r.left, r.top - 4.0 - BTN_H, r.right, r.top - 4.0)
}

pub fn at(w: f32, h: f32, x: f32, y: f32) -> Option<AboutHit> {
    if !contains(&panel(w, h), x, y) {
        return None;
    }
    Some(if contains(&report_button(w, h), x, y) {
        AboutHit::Report
    } else if contains(&changelog_button(w, h), x, y) {
        AboutHit::Changelog
    } else {
        AboutHit::Panel
    })
}

pub fn draw(p: &dyn Painter, w: f32, h: f32, hover: Option<AboutHit>, frame: usize) {
    let th = p.theme();
    // Velo sobre la ventana, para que el diálogo se lea como modal.
    p.fill(rect(0.0, 0.0, w, h), Color { r: 0.0, g: 0.0, b: 0.0, a: 0.45 });
    let pn = panel(w, h);
    p.round_fill(rect(pn.left - 2.0, pn.top, pn.right + 2.0, pn.bottom + 4.0), 14.0, alpha(th.background, 0.5));
    p.round_fill(pn, 12.0, th.surface_alt);
    // Franja del logo: el panel entero en oscuro, recortado a la parte de arriba.
    p.clip(rect(pn.left, pn.top, pn.right, pn.top + BANNER_H), &mut || p.round_fill(pn, 12.0, BANNER));
    // Las letras del logo ocupan unos 224 × 31 dips (como el logo anterior); el cuadro trae poco margen.
    let (cx, cy) = ((pn.left + pn.right) / 2.0, pn.top + BANNER_H / 2.0);
    p.logo(rect(cx - LOGO_W / 2.0, cy - LOGO_W * 0.231, cx + LOGO_W / 2.0, cy + LOGO_W * 0.231), frame);
    p.round_stroke(pn, 12.0, th.border);

    let line = |top: f32, bottom: f32| rect(pn.left + 20.0, top, pn.right - 20.0, bottom);
    p.text(FontId::UiBold, &format!("xdir {VERSION}"), line(pn.top + BANNER_H + 8.0, pn.top + BANNER_H + 34.0), th.text);
    p.text(
        FontId::Ui,
        "Explorador de archivos multipanel para Windows.",
        line(pn.top + BANNER_H + 34.0, pn.top + BANNER_H + 58.0),
        th.text_dim,
    );

    let r = changelog_button(w, h);
    if hover == Some(AboutHit::Changelog) {
        p.round_fill(r, 6.0, th.hover);
    }
    p.text(FontId::Glyphs, crate::pane::CHANGELOG_GLYPH, rect(r.left + 6.0, r.top, r.left + 32.0, r.bottom), th.text_dim);
    p.text(FontId::Ui, "Novedades", rect(r.left + 38.0, r.top, r.right - 8.0, r.bottom), th.text);

    let r = report_button(w, h);
    if hover == Some(AboutHit::Report) {
        p.round_fill(r, 6.0, th.hover);
    }
    p.text(FontId::Glyphs, "\u{E783}", rect(r.left + 6.0, r.top, r.left + 32.0, r.bottom), th.text_dim);
    p.text(FontId::Ui, "Reportar un problema o sugerir algo", rect(r.left + 38.0, r.top, r.right - 8.0, r.bottom), th.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;

    /// El nombre «xdir» con la versión va en la fuente de interfaz en seminegrita (como hoy).
    #[test]
    fn about_title_uses_bold_font() {
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        draw(&p, 800.0, 600.0, None, 0);
        let title = format!("xdir {VERSION}");
        let texts = p.texts();
        let found: Vec<_> = texts.iter().filter(|(_, s, _)| *s == title).collect();
        assert_eq!(found.len(), 1, "{texts:?}");
        assert_eq!(found[0].0, FontId::UiBold);
    }

    /// «Novedades» va arriba de «Reportar…» y cada uno responde a su clic.
    #[test]
    fn about_has_changelog_button_above_report() {
        let (w, h) = (800.0, 600.0);
        let (c, r) = (changelog_button(w, h), report_button(w, h));
        assert!(c.bottom < r.top && c.top > panel(w, h).top + BANNER_H + 50.0, "{c:?} {r:?}");
        assert_eq!(at(w, h, (c.left + c.right) / 2.0, (c.top + c.bottom) / 2.0), Some(AboutHit::Changelog));
        assert_eq!(at(w, h, (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0), Some(AboutHit::Report));
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)));
        draw(&p, w, h, Some(AboutHit::Changelog), 0);
        assert!(p.texts().iter().any(|(_, s, _)| s == "Novedades"));
    }

    /// El logo trae cuadros y los pide completos; la animación se repite y avanza con el tiempo.
    #[test]
    fn logo_animates_and_loops() {
        let n = logo_frame_count();
        assert!(n > 1);
        assert!((0..n).all(|i| logo_pixels(i).is_some()) && logo_pixels(n).is_none());
        assert_eq!(logo_frame_at(0).0, 0);
        let total: u64 = logo_durations().iter().map(|&d| d as u64).sum();
        assert_eq!(logo_frame_at(total).0, 0);
        assert_eq!(logo_frame_at(total - 1).0, n - 1);
        let (_, wait) = logo_frame_at(0);
        assert_eq!(logo_frame_at(wait).0, 1);
    }
}

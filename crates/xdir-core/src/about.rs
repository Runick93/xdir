//! Diálogo "Acerca de": logo, versión, Novedades y el enlace para reportar un problema. Es un panel dibujado
//! como el resto de la interfaz (sin ventana de Windows aparte); un clic fuera o Esc lo cierra.

use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter, Pixels};
use crate::theme::alpha;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ISSUES_URL: &str = "https://github.com/Runick93/xdir/issues/new/choose";
pub const REPO_URL: &str = "https://github.com/Runick93/xdir";

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
    /// El ícono de GitHub de la esquina: abre el repositorio.
    Repo,
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

/// El ícono de GitHub, solo, en la esquina de abajo a la derecha.
fn repo_button(w: f32, h: f32) -> Rect {
    let p = panel(w, h);
    rect(p.right - 16.0 - BTN_H, p.bottom - 16.0 - BTN_H, p.right - 16.0, p.bottom - 16.0)
}

/// «Reportar…», abajo, hasta el ícono de GitHub.
fn report_button(w: f32, h: f32) -> Rect {
    let p = panel(w, h);
    let g = repo_button(w, h);
    rect(p.left + 16.0, g.top, g.left - 4.0, g.bottom)
}

/// «Novedades», arriba del de reportar (4 entre los dos).
fn changelog_button(w: f32, h: f32) -> Rect {
    let r = report_button(w, h);
    rect(r.left, r.top - 4.0 - BTN_H, r.right, r.top - 4.0)
}

/// `top` (translúcido) sobre `under`, como color opaco.
fn over(top: Color, under: Color) -> Color {
    let a = top.a;
    Color { r: top.r * a + under.r * (1.0 - a), g: top.g * a + under.g * (1.0 - a), b: top.b * a + under.b * (1.0 - a), a: 1.0 }
}

/// La marca de GitHub (el gato dentro de un círculo) en el cuadrado `r`: un círculo de `fg` con la
/// silueta recortada en `bg`. No hay glifo para ella en las fuentes de íconos del sistema.
fn github_mark(p: &dyn Painter, r: Rect, fg: Color, bg: Color) {
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let s = (r.right - r.left).min(r.bottom - r.top) / 2.0;
    p.circle(cx, cy, s, fg);
    // Silueta en unidades del radio (y hacia abajo): cabeza con orejas, cuello y la cola a la
    // izquierda; el cuerpo baja hasta el borde del círculo, como en la marca.
    const CAT: [(f32, f32, f32); 19] = [
        (-0.20, 0.46, 0.05),
        (-0.47, 0.33, 0.20),
        (-0.60, 0.02, 0.25),
        (-0.55, -0.30, 0.10),
        (-0.50, -0.66, 0.04),
        (-0.24, -0.50, 0.08),
        (0.0, -0.54, 0.10),
        (0.24, -0.50, 0.08),
        (0.50, -0.66, 0.04),
        (0.55, -0.30, 0.10),
        (0.60, 0.02, 0.25),
        (0.47, 0.33, 0.20),
        (0.20, 0.46, 0.05),
        (0.22, 1.05, 0.0),
        (-0.22, 1.05, 0.0),
        (-0.22, 0.78, 0.04),
        (-0.42, 0.74, 0.12),
        (-0.62, 0.52, 0.04),
        (-0.30, 0.64, 0.10),
    ];
    let pts: Vec<(f32, f32, f32)> = CAT.iter().map(|&(x, y, rad)| (cx + x * s, cy + y * s, rad * s)).collect();
    p.clip(rect(cx - s, cy - s, cx + s, cy + s - s * 0.02), &mut || p.rounded_path(&pts, true, Some(bg), None));
}

pub fn at(w: f32, h: f32, x: f32, y: f32) -> Option<AboutHit> {
    if !contains(&panel(w, h), x, y) {
        return None;
    }
    Some(if contains(&report_button(w, h), x, y) {
        AboutHit::Report
    } else if contains(&repo_button(w, h), x, y) {
        AboutHit::Repo
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
        "Explorador multipanel para Windows, macOS y Linux.",
        line(pn.top + BANNER_H + 34.0, pn.top + BANNER_H + 58.0),
        th.text_dim,
    );

    let r = changelog_button(w, h);
    if hover == Some(AboutHit::Changelog) {
        p.round_fill(r, 6.0, th.hover);
    }
    p.text(FontId::Glyphs, crate::pane::CHANGELOG_GLYPH, rect(r.left + 6.0, r.top, r.left + 32.0, r.bottom), th.text_dim);
    p.text(FontId::Ui, "Novedades", rect(r.left + 38.0, r.top, r.right - 8.0, r.bottom), th.text);

    let r = repo_button(w, h);
    // La silueta se pinta encima del círculo: necesita el fondo opaco (el resaltado es translúcido).
    let bg = if hover == Some(AboutHit::Repo) { over(th.hover, th.surface_alt) } else { th.surface_alt };
    if hover == Some(AboutHit::Repo) {
        p.round_fill(r, 6.0, th.hover);
    }
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let c = if hover == Some(AboutHit::Repo) { th.text } else { th.text_dim };
    github_mark(p, rect(cx - 9.0, cy - 9.0, cx + 9.0, cy + 9.0), c, bg);

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
        let g = repo_button(w, h);
        let pn = panel(w, h);
        assert!(g.right <= pn.right && g.bottom <= pn.bottom && g.left > r.right, "GitHub va solo, abajo a la derecha");
        assert!((g.right - g.left - (g.bottom - g.top)).abs() < 0.01, "es un botón cuadrado");
        assert_eq!(at(w, h, (g.left + g.right) / 2.0, (g.top + g.bottom) / 2.0), Some(AboutHit::Repo));
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)));
        draw(&p, w, h, Some(AboutHit::Changelog), 0);
        assert!(p.texts().iter().any(|(_, s, _)| s == "Novedades"));
        assert!(!p.texts().iter().any(|(_, s, _)| s.contains("GitHub")), "sin texto: solo el ícono");
        // Ya no es solo para Windows.
        assert!(p.texts().iter().any(|(_, s, _)| s.contains("Windows, macOS y Linux")));
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

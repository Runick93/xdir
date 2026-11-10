//! El panel de Renombrar varios (diseño `rename_many::draw_panel`, X51): ocupa el lugar de la
//! columna de vista previa. Barra superior de 40 con «Renombrar N elementos» y cerrar; los modos
//! en un control segmentado de 288×28; los campos (cajas nativas del host, sobre `fields`) de 28;
//! las casillas; el resumen («8 de 10 cambian», el choque) y un pie fijo de 52 con Cancelar y
//! Renombrar (apagado con choques o sin cambios). Dibujo y hit-test puros.

use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::rename_many::FormatStyle;
use crate::theme::alpha;

const MARGIN: f32 = 16.0;
const TOP_BAR: f32 = 40.0;
const FOOTER: f32 = 52.0;
const CONTROL_H: f32 = 28.0;
const LABEL_H: f32 = 16.0;
const CHECK: f32 = 16.0;
const START_W: f32 = 96.0;
const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Los tres modos, como en el Finder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModeKind {
    Replace,
    Add,
    Format,
}

impl ModeKind {
    pub const ALL: [ModeKind; 3] = [ModeKind::Replace, ModeKind::Add, ModeKind::Format];

    pub fn label(self) -> &'static str {
        match self {
            ModeKind::Replace => "Reemplazar texto",
            ModeKind::Add => "Agregar texto",
            ModeKind::Format => "Formato",
        }
    }
}

/// Los campos de texto de cada modo.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Field {
    Find,
    With,
    Text,
    Base,
    Start,
}

/// Lo que hay bajo el mouse en el panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelHit {
    Close,
    Mode(ModeKind),
    Style(FormatStyle),
    /// «Antes del nombre» (true) / «Después del nombre» (false).
    Before(bool),
    MatchCase,
    Cancel,
    Apply,
    /// El panel, sin un control debajo.
    Panel,
}

/// Lo que el panel muestra (lo arma la `App`).
#[derive(Clone, Debug, PartialEq)]
pub struct PanelState {
    pub count: usize,
    pub mode: ModeKind,
    pub match_case: bool,
    pub add_before: bool,
    pub style: FormatStyle,
    pub format_before: bool,
    /// Cuántos cambian de nombre.
    pub changing: usize,
    /// El resumen del primer choque («1 choque: «x» ya existe»).
    pub clash: Option<String>,
    pub can_apply: bool,
}

/// Rótulo, caja y campo de cada campo del modo.
fn field_rows(r: Rect, s: &PanelState) -> Vec<(&'static str, Field, Rect)> {
    let (l, rr) = (r.left + MARGIN, r.right - MARGIN);
    let y = r.top + 100.0;
    let at = |top: f32| rect(l, top, rr, top + CONTROL_H);
    match s.mode {
        ModeKind::Replace => vec![("Buscar", Field::Find, at(y + 18.0)), ("Reemplazar con", Field::With, at(y + 76.0))],
        ModeKind::Add => vec![("Texto", Field::Text, at(y + 18.0))],
        ModeKind::Format => {
            let mut v = vec![("Texto base (vacío: el nombre de cada uno)", Field::Base, at(y + 76.0))];
            if s.style != FormatStyle::Date {
                v.push(("Número inicial", Field::Start, rect(l, y + 192.0, l + START_W, y + 192.0 + CONTROL_H)));
            }
            v
        }
    }
}

/// Dónde van las cajas de texto nativas.
pub fn fields(r: Rect, s: &PanelState) -> Vec<(Field, Rect)> {
    field_rows(r, s).into_iter().map(|(_, f, b)| (f, b)).collect()
}

/// Los controles que se cliquean, con su rectángulo; y dónde empieza el resumen.
fn controls(r: Rect, s: &PanelState) -> (Vec<(PanelHit, Rect)>, f32) {
    let (l, rr) = (r.left + MARGIN, r.right - MARGIN);
    let mut v = Vec::new();
    let mid = r.top + TOP_BAR / 2.0;
    v.push((PanelHit::Close, rect(rr - 24.0, mid - 12.0, rr, mid + 12.0)));
    let seg = |top: f32, n: usize, i: usize| {
        let w = (rr - l) / n as f32;
        rect(l + w * i as f32, top, l + w * (i + 1) as f32, top + CONTROL_H)
    };
    // Los modos no van en tercios: «Reemplazar texto» necesita más lugar (112 / 96 / 80 de 288,
    // como en el diseño); con otro ancho de panel, en la misma proporción.
    let mut x = l;
    for (m, part) in ModeKind::ALL.into_iter().zip([112.0, 96.0, 80.0]) {
        let w = (rr - l) * part / 288.0;
        v.push((PanelHit::Mode(m), rect(x, r.top + 56.0, x + w, r.top + 56.0 + CONTROL_H)));
        x += w;
    }
    let y = r.top + 100.0;
    let option = |top: f32| rect(l, top, rr, top + 20.0);
    let summary = match s.mode {
        ModeKind::Replace => {
            v.push((PanelHit::MatchCase, option(y + 116.0)));
            y + 150.0
        }
        ModeKind::Add => {
            v.push((PanelHit::Before(true), option(y + 60.0)));
            v.push((PanelHit::Before(false), option(y + 86.0)));
            y + 120.0
        }
        ModeKind::Format => {
            for (i, st) in [FormatStyle::Number, FormatStyle::Counter, FormatStyle::Date].into_iter().enumerate() {
                v.push((PanelHit::Style(st), seg(y + 18.0, 3, i)));
            }
            v.push((PanelHit::Before(true), option(y + 114.0)));
            v.push((PanelHit::Before(false), option(y + 140.0)));
            if s.style == FormatStyle::Date { y + 176.0 } else { y + 234.0 }
        }
    };
    let by = r.bottom - FOOTER + 12.0;
    let apply = rect(rr - 104.0, by, rr, by + CONTROL_H);
    v.push((PanelHit::Apply, apply));
    v.push((PanelHit::Cancel, rect(apply.left - 8.0 - 96.0, by, apply.left - 8.0, by + CONTROL_H)));
    (v, summary)
}

/// Qué hay en `(x, y)`: un control, el panel (`Panel`) o nada (fuera).
pub fn hit(r: Rect, s: &PanelState, x: f32, y: f32) -> Option<PanelHit> {
    if !contains(&r, x, y) {
        return None;
    }
    let (v, _) = controls(r, s);
    Some(v.into_iter().find(|(_, b)| contains(b, x, y)).map_or(PanelHit::Panel, |(h, _)| h))
}

fn checkbox(p: &dyn Painter, b: Rect, on: bool, label: &str, hot: bool) {
    let th = p.theme();
    let bx = rect(b.left, b.top + 2.0, b.left + CHECK, b.top + 2.0 + CHECK);
    if on {
        p.round_fill(bx, 3.0, th.accent);
        p.text(FontId::GlyphsSmall, "\u{E73E}", rect(bx.left, bx.top - 1.0, bx.right, bx.bottom), WHITE);
    } else {
        p.round_fill(bx, 3.0, th.surface);
        p.round_stroke_w(bx, 3.0, if hot { th.accent } else if th.dark { th.text_secondary } else { th.text_dim }, 1.2);
    }
    p.text(FontId::Ui, label, rect(bx.right + 8.0, b.top, b.right, b.bottom), th.text);
}

fn segmented(p: &dyn Painter, items: &[(Rect, &str, bool, bool)]) {
    let th = p.theme();
    let (Some(first), Some(last)) = (items.first(), items.last()) else { return };
    let all = rect(first.0.left, first.0.top, last.0.right, first.0.bottom);
    p.round_fill(all, 6.0, th.surface_alt);
    p.round_stroke(all, 6.0, th.border);
    for &(b, label, on, hot) in items {
        let inner = rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0);
        if on {
            p.round_fill(inner, 4.0, th.selection);
        } else if hot {
            p.round_fill(inner, 4.0, th.hover);
        }
        // Texto chico centrado a mano (no hay formato chico centrado): «Reemplazar texto» entra
        // en un tercio de 288 como en el diseño.
        let w = p.measure(FontId::Small, label).min(b.right - b.left - 4.0);
        let x = (b.left + b.right - w) / 2.0;
        p.text(FontId::Small, label, rect(x, b.top + 6.0, b.right - 2.0, b.bottom - 6.0), if on { th.text } else { th.text_secondary });
    }
}

/// Dibuja el panel en `r` (el lugar de la columna de vista previa).
pub fn draw(p: &dyn Painter, r: Rect, s: &PanelState, hover: Option<PanelHit>) {
    let th = p.theme();
    p.round_fill(r, 8.0, th.surface);
    p.round_stroke(r, 8.0, th.border);
    if r.right - r.left < 120.0 {
        return;
    }
    let (l, rr) = (r.left + MARGIN, r.right - MARGIN);
    let (v, summary) = controls(r, s);
    let find = |h: PanelHit| v.iter().find(|(x, _)| *x == h).map(|(_, b)| *b);
    let hot = |h: PanelHit| hover == Some(h);
    p.clip(r, &mut || {
        // Barra superior.
        let title = format!("Renombrar {} elemento{}", s.count, if s.count == 1 { "" } else { "s" });
        p.text(FontId::UiBold, &title, rect(l, r.top + 12.0, rr - 32.0, r.top + 28.0), th.text);
        if let Some(b) = find(PanelHit::Close) {
            if hot(PanelHit::Close) {
                p.round_fill(b, 4.0, th.hover);
            }
            p.text(FontId::GlyphsSmall, "\u{E711}", rect(b.left, b.top + 5.0, b.right, b.bottom - 5.0), if hot(PanelHit::Close) { th.text } else { th.text_dim });
        }
        p.fill(rect(r.left, r.top + TOP_BAR, r.right, r.top + TOP_BAR + 1.0), th.border);
        // Modos.
        let modes: Vec<(Rect, &str, bool, bool)> =
            ModeKind::ALL.iter().filter_map(|&m| Some((find(PanelHit::Mode(m))?, m.label(), s.mode == m, hot(PanelHit::Mode(m))))).collect();
        segmented(p, &modes);
        // Rótulos de los campos y el fondo de las cajas (la caja nativa va encima).
        for (label, _, b) in field_rows(r, s) {
            p.text(FontId::Small, label, rect(b.left, b.top - 18.0, rr, b.top - 18.0 + LABEL_H), th.text_dim);
            p.round_fill(b, 4.0, th.surface_alt);
            p.round_stroke(b, 4.0, th.border);
        }
        match s.mode {
            ModeKind::Replace => {
                if let Some(b) = find(PanelHit::MatchCase) {
                    checkbox(p, b, s.match_case, "Distinguir mayúsculas", hot(PanelHit::MatchCase));
                }
            }
            ModeKind::Add | ModeKind::Format => {
                if s.mode == ModeKind::Format {
                    let y = r.top + 100.0;
                    p.text(FontId::Small, "Formato", rect(l, y, rr, y + LABEL_H), th.text_dim);
                    let styles: Vec<(Rect, &str, bool, bool)> = [(FormatStyle::Number, "Número"), (FormatStyle::Counter, "Contador"), (FormatStyle::Date, "Fecha")]
                        .iter()
                        .filter_map(|&(st, label)| Some((find(PanelHit::Style(st))?, label, s.style == st, hot(PanelHit::Style(st)))))
                        .collect();
                    segmented(p, &styles);
                }
                let before = if s.mode == ModeKind::Add { s.add_before } else { s.format_before };
                let what = if s.mode == ModeKind::Add { "" } else { " (el número o la fecha)" };
                for (b, on, label) in [(true, before, "Antes del nombre"), (false, !before, "Después del nombre")] {
                    if let Some(rc) = find(PanelHit::Before(b)) {
                        checkbox(p, rc, on, &format!("{label}{}", if b { what } else { "" }), hot(PanelHit::Before(b)));
                    }
                }
            }
        }
        // Resumen.
        p.fill(rect(l, summary, rr, summary + 1.0), th.border);
        let y = summary + 14.0;
        p.text(FontId::UiBold, &format!("{} de {} cambian", s.changing, s.count), rect(l, y, rr, y + 18.0), th.text);
        let mut y = y + 24.0;
        if let Some(c) = &s.clash {
            p.circle(l + 5.0, y + 8.0, 5.0, th.danger);
            p.text(FontId::Small, "!", rect(l + 2.0, y + 1.0, l + 8.0, y + 15.0), WHITE);
            p.text(FontId::Small, c, rect(l + 16.0, y, rr, y + LABEL_H), th.danger);
            y += 22.0;
        }
        p.text(FontId::Small, "La extensión no cambia.", rect(l, y, rr, y + LABEL_H), th.text_dim);
        // Pie.
        p.fill(rect(r.left, r.bottom - FOOTER, r.right, r.bottom - FOOTER + 1.0), th.border);
        if let Some(b) = find(PanelHit::Cancel) {
            p.round_fill(b, 6.0, if hot(PanelHit::Cancel) { th.hover } else { th.surface_alt });
            p.round_stroke(b, 6.0, th.border);
            p.text(FontId::UiCenter, "Cancelar", rect(b.left, b.top + 6.0, b.right, b.bottom - 6.0), th.text);
        }
        if let Some(b) = find(PanelHit::Apply) {
            let fill = if !s.can_apply { alpha(th.accent, 0.35) } else if hot(PanelHit::Apply) { alpha(th.accent, 0.85) } else { th.accent };
            p.round_fill(b, 6.0, fill);
            p.text(FontId::UiCenter, "Renombrar", rect(b.left, b.top + 6.0, b.right, b.bottom - 6.0), if s.can_apply { WHITE } else { alpha(WHITE, 0.6) });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn state(mode: ModeKind) -> PanelState {
        PanelState {
            count: 10,
            mode,
            match_case: true,
            add_before: true,
            style: FormatStyle::Counter,
            format_before: false,
            changing: 8,
            clash: Some("1 choque: «Bariloche-3090.jpg» ya existe".into()),
            can_apply: false,
        }
    }

    #[test]
    fn panel_draws_title_modes_and_summary() {
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)));
        let r = rect(1074.0, 4.0, 1394.0, 790.0);
        draw(&p, r, &state(ModeKind::Replace), None);
        let texts: Vec<String> = p.texts().into_iter().map(|(_, t, _)| t).collect();
        for want in ["Renombrar 10 elementos", "Reemplazar texto", "Agregar texto", "Formato", "Buscar", "Reemplazar con", "Distinguir mayúsculas", "8 de 10 cambian", "1 choque: «Bariloche-3090.jpg» ya existe", "La extensión no cambia.", "Cancelar", "Renombrar"] {
            assert!(texts.iter().any(|t| t == want), "falta {want}: {texts:?}");
        }
        // El control segmentado mide 288×28 en un panel de 320.
        let (v, _) = controls(r, &state(ModeKind::Replace));
        let modes: Vec<Rect> = v.iter().filter(|(h, _)| matches!(h, PanelHit::Mode(_))).map(|(_, b)| *b).collect();
        assert_eq!(modes.last().unwrap().right - modes[0].left, 288.0);
        assert_eq!(modes[0].bottom - modes[0].top, 28.0);
    }

    #[test]
    fn panel_hit_controls() {
        let r = rect(1074.0, 4.0, 1394.0, 790.0);
        let s = state(ModeKind::Format);
        assert_eq!(hit(r, &s, 1000.0, 100.0), None);
        assert_eq!(hit(r, &s, 1200.0, 400.0), Some(PanelHit::Panel));
        assert_eq!(hit(r, &s, 1366.0, 24.0), Some(PanelHit::Close));
        assert_eq!(hit(r, &s, 1350.0, 790.0 - 30.0), Some(PanelHit::Apply));
        assert_eq!(hit(r, &s, 1240.0, 790.0 - 30.0), Some(PanelHit::Cancel));
        assert_eq!(hit(r, &s, 1100.0, 74.0), Some(PanelHit::Mode(ModeKind::Replace)));
        assert_eq!(hit(r, &s, 1360.0, 74.0), Some(PanelHit::Mode(ModeKind::Format)));
        // Fecha no tiene número inicial.
        assert!(fields(r, &s).iter().any(|(f, _)| *f == Field::Start));
        let date = PanelState { style: FormatStyle::Date, ..s };
        assert!(!fields(r, &date).iter().any(|(f, _)| *f == Field::Start));
    }
}

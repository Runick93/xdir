//! Contenido de la ventana de «archivo en uso»: el título, quién tiene cada archivo y los botones
//! Reintentar y Cerrar, con el mismo estilo que la ventana de copia (`transfer::view`). La ventana
//! es del sistema (`WindowHost::aux_show`); acá solo se mide, se dibuja y se ubica el clic.
//!
//! El alto lo pone el contenido y nunca pasa del de la ventana de copia (`transfer::view::MAX_H`):
//! si con cuatro programas no entran sus cinco archivos, cada uno muestra menos y el resto va en
//! «y N más».

use crate::geom::{Color, Rect, contains, rect};
use crate::in_use::{InUseDialog, MAX_FILES};
use crate::paint::{FontId, Painter, ellipsize};
use crate::theme::alpha;
use crate::transfer::view::MAX_H;

/// Ancho del área de la ventana (el mismo que la de copia).
pub const WIDTH: f32 = 560.0;
const PAD: f32 = 16.0;
/// Sangría de cada programa y de sus archivos, desde el margen.
const GROUP_INDENT: f32 = 12.0;
const FILE_INDENT: f32 = 28.0;
const TITLE_STEP: f32 = 30.0;
const LINE_STEP: f32 = 22.0;
const LABEL_STEP: f32 = 20.0;
const SMALL_STEP: f32 = 18.0;
const GROUP_GAP: f32 = 6.0;
/// Los botones, como «Reintentar» y «Cerrar» de la ventana de copia.
const RETRY_W: f32 = 104.0;
const CLOSE_W: f32 = 96.0;
const BTN_H: f32 = 28.0;

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Lo que hay bajo el mouse en la ventana.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InUseHit {
    Retry,
    Close,
}

/// Una línea de texto: fuente, texto, sangría desde el margen, si va en texto secundario y su
/// arriba (desde el del contenido).
struct Line {
    font: FontId,
    text: String,
    indent: f32,
    dim: bool,
    top: f32,
}

/// «y 1 programa más», «y 3 programas más».
fn more_programs(n: usize) -> String {
    if n == 1 { "y 1 programa más".into() } else { format!("y {n} programas más") }
}

/// Las líneas con hasta `shown` archivos por programa, y el alto del contenido.
fn layout(d: &InUseDialog, shown: usize) -> (Vec<Line>, f32) {
    let mut out = Vec::new();
    let mut y = PAD;
    let push = |out: &mut Vec<Line>, y: &mut f32, font: FontId, text: String, indent: f32, dim: bool, step: f32| {
        out.push(Line { font, text, indent, dim, top: *y });
        *y += step;
    };
    push(&mut out, &mut y, FontId::Title, d.title.clone(), 0.0, false, TITLE_STEP);
    push(&mut out, &mut y, FontId::Ui, d.intro().into(), 0.0, false, LINE_STEP);
    for g in &d.groups {
        y += GROUP_GAP;
        push(&mut out, &mut y, FontId::UiBold, InUseDialog::holder_label(&g.holder), GROUP_INDENT, false, LABEL_STEP);
        push(&mut out, &mut y, FontId::Small, InUseDialog::holder_line(&g.holder), GROUP_INDENT, true, SMALL_STEP);
        for f in g.files.iter().take(shown) {
            push(&mut out, &mut y, FontId::Ui, f.clone(), FILE_INDENT, false, SMALL_STEP);
        }
        let hidden = g.files.len().saturating_sub(shown) + g.more_files;
        if hidden > 0 {
            push(&mut out, &mut y, FontId::Small, format!("y {hidden} más"), FILE_INDENT, true, SMALL_STEP);
        }
    }
    if d.more_groups > 0 {
        push(&mut out, &mut y, FontId::Small, more_programs(d.more_groups), GROUP_INDENT, true, SMALL_STEP);
    }
    y += 8.0;
    push(&mut out, &mut y, FontId::Ui, d.outro().into(), 0.0, false, LINE_STEP);
    (out, y + 12.0 + BTN_H + PAD)
}

/// Cuántos archivos por programa entran sin pasarse de `MAX_H` (a lo sumo `MAX_FILES`).
fn files_shown(d: &InUseDialog) -> usize {
    (0..=MAX_FILES).rev().find(|&k| layout(d, k).1 <= MAX_H).unwrap_or(0)
}

/// Alto del contenido (sin la barra de título del sistema); nunca pasa de `MAX_H`.
pub fn height(d: &InUseDialog) -> f32 {
    layout(d, files_shown(d)).1
}

/// Los botones con su rectángulo, alineados a la derecha al pie del contenido.
fn buttons(r: Rect, d: &InUseDialog) -> [(InUseHit, Rect); 2] {
    let right = r.right - PAD;
    let top = r.top + height(d) - PAD - BTN_H;
    let close = rect(right - CLOSE_W, top, right, top + BTN_H);
    let retry = rect(close.left - 8.0 - RETRY_W, top, close.left - 8.0, top + BTN_H);
    [(InUseHit::Retry, retry), (InUseHit::Close, close)]
}

/// Qué botón hay en (x, y) del área `r`.
pub fn hit(r: Rect, d: &InUseDialog, x: f32, y: f32) -> Option<InUseHit> {
    if !contains(&r, x, y) {
        return None;
    }
    buttons(r, d).into_iter().find(|(_, b)| contains(b, x, y)).map(|(h, _)| h)
}

/// Un botón como los de la ventana de copia; apretado (con el mouse encima) se oscurece un poco más.
fn button(p: &dyn Painter, b: Rect, label: &str, primary: bool, hot: bool, down: bool) {
    let th = p.theme();
    let tr = rect(b.left, b.top + 5.0, b.right, b.bottom - 5.0);
    if primary {
        let c = if down { alpha(th.accent, 0.7) } else if hot { alpha(th.accent, 0.85) } else { th.accent };
        p.round_fill(b, 4.0, c);
        p.text(FontId::UiCenter, label, tr, WHITE);
    } else {
        let c = if down { alpha(th.text, 0.12) } else if hot { th.hover } else { th.surface_alt };
        p.round_fill(b, 4.0, c);
        p.round_stroke(b, 4.0, th.border);
        p.text(FontId::UiCenter, label, tr, th.text);
    }
}

/// Dibuja el contenido en `r` (recortado a él). `pressed` se ve solo mientras el mouse sigue
/// encima del mismo botón.
pub fn draw(p: &dyn Painter, r: Rect, d: &InUseDialog, hover: Option<InUseHit>, pressed: Option<InUseHit>) {
    let th = p.theme();
    p.fill(r, th.surface);
    p.clip(r, &mut || {
        let (l, rr) = (r.left + PAD, r.right - PAD);
        for line in layout(d, files_shown(d)).0 {
            let x = l + line.indent;
            let h = match line.font {
                FontId::Title => 22.0,
                FontId::Small => 16.0,
                _ => 18.0,
            };
            let top = r.top + line.top;
            let c = if line.dim { th.text_dim } else { th.text };
            p.text(line.font, &ellipsize(p, line.font, &line.text, rr - x), rect(x, top, rr, top + h), c);
        }
        for (h, b) in buttons(r, d) {
            let label = if h == InUseHit::Retry { "Reintentar" } else { "Cerrar" };
            let hot = hover == Some(h);
            button(p, b, label, h == InUseHit::Retry, hot, hot && pressed == Some(h));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::rect;
    use crate::in_use::{HolderGroup, InUseRetry, MAX_FILES, MAX_GROUPS};
    use crate::sys::Holder;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn theme() -> Theme {
        Theme::from_system(DEFAULT_ACCENT, Some(true))
    }

    fn excel() -> Holder {
        Holder { name: "Microsoft Excel".into(), exe: Some("EXCEL.EXE".into()), pid: 4812, user: Some(r"PC\Leandro".into()) }
    }

    fn group(pid: u32, files: usize, more_files: usize) -> HolderGroup {
        let holder = Holder { name: format!("Programa {pid}"), exe: None, pid, user: None };
        HolderGroup { holder, files: (0..files).map(|i| format!("archivo{i}.txt")).collect(), more_files }
    }

    fn dialog(groups: Vec<HolderGroup>, more_groups: usize) -> InUseDialog {
        InUseDialog { title: "No se pudo eliminar «informe.xlsx»".into(), groups, more_groups, retry: InUseRetry::Op(crate::ops::Op::NewFolder { parent: std::path::PathBuf::from("/u") }) }
    }

    fn area(d: &InUseDialog) -> Rect {
        rect(0.0, 0.0, WIDTH, height(d))
    }

    fn texts(d: &InUseDialog) -> Vec<(String, Rect)> {
        let p = RecordingPainter::new(theme());
        draw(&p, area(d), d, None, None);
        p.texts().into_iter().map(|(_, s, r)| (s, r)).collect()
    }

    #[test]
    fn height_grows_with_groups_and_files() {
        let one = dialog(vec![group(1, 1, 0)], 0);
        let three = dialog(vec![group(1, MAX_FILES, 0), group(2, MAX_FILES, 0), group(3, MAX_FILES, 0)], 0);
        assert!(height(&one) < height(&three));
        // Un archivo más (o un «y N más») también suma.
        assert!(height(&dialog(vec![group(1, 2, 0)], 0)) > height(&one));
        assert!(height(&dialog(vec![group(1, 1, 0)], 2)) > height(&one));
    }

    #[test]
    fn height_is_capped_by_the_limits() {
        let full = dialog((1..=MAX_GROUPS as u32).map(|p| group(p, MAX_FILES, 1)).collect(), 1);
        let huge = dialog((1..=MAX_GROUPS as u32).map(|p| group(p, MAX_FILES, 200)).collect(), 200);
        // 200 archivos (o programas) de más no cambian el alto: van en «y N más».
        assert_eq!(height(&full), height(&huge));
        // Y lo más alto que puede ser entra en lo que permite la ventana de copia.
        assert!(height(&huge) <= crate::transfer::view::MAX_H);
        // Tres programas con muchos archivos (200 de 3) muestran sus cinco archivos cada uno.
        let many = dialog((1..=3).map(|p| group(p, MAX_FILES, 62)).collect(), 0);
        assert!(height(&many) <= crate::transfer::view::MAX_H);
        let shown = texts(&many).iter().filter(|(s, _)| s.starts_with("archivo")).count();
        assert_eq!(shown, 3 * MAX_FILES);
    }

    #[test]
    fn hidden_files_are_counted_when_they_do_not_fit() {
        // Con cuatro programas de cinco archivos no entran todos: lo que falta va en «y N más».
        let d = dialog((1..=MAX_GROUPS as u32).map(|p| group(p, MAX_FILES, 3)).collect(), 0);
        let t = texts(&d);
        let shown = t.iter().filter(|(s, _)| s.starts_with("archivo")).count();
        let hidden: usize = t.iter().filter_map(|(s, _)| s.strip_prefix("y ")?.strip_suffix(" más")?.parse::<usize>().ok()).sum();
        assert_eq!(shown + hidden, MAX_GROUPS * (MAX_FILES + 3));
    }

    #[test]
    fn hit_finds_both_buttons_and_nothing_elsewhere() {
        let d = dialog(vec![HolderGroup { holder: excel(), files: vec!["informe.xlsx".into()], more_files: 0 }], 0);
        let r = rect(10.0, 20.0, 10.0 + WIDTH, 20.0 + height(&d));
        let bottom = r.bottom - 16.0 - 14.0;
        assert_eq!(hit(r, &d, r.right - 16.0 - 48.0, bottom), Some(InUseHit::Close));
        assert_eq!(hit(r, &d, r.right - 16.0 - 96.0 - 8.0 - 52.0, bottom), Some(InUseHit::Retry));
        // Entre los botones, sobre el texto y fuera del área: nada.
        assert_eq!(hit(r, &d, r.right - 16.0 - 96.0 - 4.0, bottom), None);
        assert_eq!(hit(r, &d, r.left + 40.0, r.top + 30.0), None);
        assert_eq!(hit(r, &d, r.right - 60.0, r.bottom + 5.0), None);
        // Cada botón se dibuja donde está su clic.
        let p = RecordingPainter::new(theme());
        draw(&p, r, &d, None, None);
        for (s, b) in p.texts().into_iter().filter(|(_, s, _)| s == "Reintentar" || s == "Cerrar").map(|(_, s, b)| (s, b)) {
            let want = if s == "Reintentar" { InUseHit::Retry } else { InUseHit::Close };
            assert_eq!(hit(r, &d, (b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0), Some(want));
        }
    }

    #[test]
    fn draw_shows_title_intro_process_and_buttons() {
        let d = dialog(vec![HolderGroup { holder: excel(), files: vec!["informe.xlsx".into()], more_files: 0 }], 0);
        let t: Vec<String> = texts(&d).into_iter().map(|(s, _)| s).collect();
        for want in [
            "No se pudo eliminar «informe.xlsx»",
            "Lo está usando otro programa:",
            "Microsoft Excel (EXCEL.EXE)",
            r"PID 4812 · usuario PC\Leandro",
            "informe.xlsx",
            "Cerralo y reintentá.",
            "Reintentar",
            "Cerrar",
        ] {
            assert!(t.iter().any(|s| s == want), "falta «{want}» en {t:?}");
        }
    }

    #[test]
    fn draw_counts_hidden_files_and_programs() {
        let one = dialog(vec![group(1, 2, 7), group(2, 1, 0)], 1);
        let t: Vec<String> = texts(&one).into_iter().map(|(s, _)| s).collect();
        assert!(t.iter().any(|s| s == "y 7 más"), "{t:?}");
        assert!(t.iter().any(|s| s == "y 1 programa más"), "{t:?}");
        let many = dialog(vec![group(1, 1, 0)], 5);
        assert!(texts(&many).iter().any(|(s, _)| s == "y 5 programas más"));
    }

    #[test]
    fn retry_is_primary_and_reacts_to_hover_and_press() {
        let d = dialog(vec![group(1, 1, 0)], 0);
        let retry_fill = |hover, pressed| {
            let p = RecordingPainter::new(theme());
            draw(&p, area(&d), &d, hover, pressed);
            let b = buttons(area(&d), &d)[0].1;
            p.round_fills().into_iter().find(|(r, _)| *r == b).map(|(_, c)| c).unwrap()
        };
        let th = theme();
        assert_eq!(retry_fill(None, None), th.accent);
        assert_eq!(retry_fill(Some(InUseHit::Retry), None), alpha(th.accent, 0.85));
        assert_eq!(retry_fill(Some(InUseHit::Retry), Some(InUseHit::Retry)), alpha(th.accent, 0.7));
        // Apretado pero con el mouse afuera: se ve normal.
        assert_eq!(retry_fill(Some(InUseHit::Close), Some(InUseHit::Retry)), th.accent);
    }

    #[test]
    fn long_texts_stay_inside_the_area() {
        let long = "x".repeat(300);
        let holder = Holder { name: long.clone(), exe: Some(format!("{long}.EXE")), pid: 1, user: Some(long.clone()) };
        let mut d = dialog(vec![HolderGroup { holder, files: vec![format!("{long}.txt")], more_files: 0 }], 0);
        d.title = long;
        let r = area(&d);
        let p = RecordingPainter::new(theme());
        draw(&p, r, &d, None, None);
        for (_, s, b) in p.texts() {
            assert!(b.left >= r.left && b.right <= r.right && b.top >= r.top && b.bottom <= r.bottom, "{s} fuera: {b:?}");
            // `measure` de prueba: 7 por carácter.
            assert!(7.0 * s.chars().count() as f32 <= b.right - b.left, "{s} no entra en {b:?}");
        }
    }
}

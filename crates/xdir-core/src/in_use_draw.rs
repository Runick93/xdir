//! Contenido de la ventana de «archivo en uso»: el título en rojo, la frase con el programa, una
//! tarjeta por programa (ubicación, proceso, usuario y, si hace falta, sus archivos) y los botones
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
const TITLE_STEP: f32 = 30.0;
const LINE_STEP: f32 = 22.0;
const SMALL_STEP: f32 = 18.0;
/// Las tarjetas: alto de cada fila, separación entre tarjetas y dónde empiezan el texto y el valor.
const ROW_H: f32 = 30.0;
const CARD_GAP: f32 = 8.0;
const CARD_RADIUS: f32 = 6.0;
const ICON_X: f32 = 14.0;
const LABEL_X: f32 = 42.0;
const VALUE_X: f32 = 128.0;
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

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Normal,
    Dim,
    Danger,
}

/// Una línea de texto suelta: fuente, texto, color y su arriba (desde el del contenido).
struct Line {
    font: FontId,
    text: String,
    tone: Tone,
    top: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    None,
    Folder,
    Process,
    User,
    File,
}

/// Una fila de una tarjeta: ícono, rótulo y valor.
struct Row {
    top: f32,
    icon: Icon,
    label: &'static str,
    value: String,
    dim: bool,
}

struct Layout {
    lines: Vec<Line>,
    /// Arriba y alto de cada tarjeta.
    cards: Vec<(f32, f32)>,
    rows: Vec<Row>,
    height: f32,
}

/// «y 1 programa más», «y 3 programas más».
fn more_programs(n: usize) -> String {
    if n == 1 { "y 1 programa más".into() } else { format!("y {n} programas más") }
}

/// La ventana con hasta `shown` archivos por programa.
fn layout(d: &InUseDialog, shown: usize) -> Layout {
    let mut lines = Vec::new();
    let mut y = PAD;
    lines.push(Line { font: FontId::Title, text: d.title.clone(), tone: Tone::Danger, top: y });
    y += TITLE_STEP;
    lines.push(Line { font: FontId::Ui, text: d.intro(), tone: Tone::Normal, top: y });
    y += LINE_STEP + 8.0;
    // Con un solo programa y un solo archivo, el título ya dice cuál es: no se repite.
    let single = d.groups.len() == 1 && d.more_groups == 0 && d.groups[0].files.len() == 1 && d.groups[0].more_files == 0;
    let (mut cards, mut rows) = (Vec::new(), Vec::new());
    for (i, g) in d.groups.iter().enumerate() {
        let mut items: Vec<(Icon, &'static str, String, bool)> = Vec::new();
        if let (0, Some(loc)) = (i, &d.location) {
            items.push((Icon::Folder, "Ubicación", loc.clone(), false));
        }
        items.push((Icon::Process, "Proceso", InUseDialog::process_value(&g.holder), false));
        items.push((Icon::User, "Usuario", InUseDialog::user_value(&g.holder), false));
        if !single {
            for (k, f) in g.files.iter().take(shown).enumerate() {
                items.push((if k == 0 { Icon::File } else { Icon::None }, if k == 0 { "Archivos" } else { "" }, f.clone(), false));
            }
            let hidden = g.files.len().saturating_sub(shown) + g.more_files;
            if hidden > 0 {
                let first = shown == 0;
                items.push((if first { Icon::File } else { Icon::None }, if first { "Archivos" } else { "" }, format!("y {hidden} más"), true));
            }
        }
        let h = items.len() as f32 * ROW_H;
        cards.push((y, h));
        for (k, (icon, label, value, dim)) in items.into_iter().enumerate() {
            rows.push(Row { top: y + k as f32 * ROW_H, icon, label, value, dim });
        }
        y += h + CARD_GAP;
    }
    if d.more_groups > 0 {
        lines.push(Line { font: FontId::Small, text: more_programs(d.more_groups), tone: Tone::Dim, top: y });
        y += SMALL_STEP;
    }
    Layout { lines, cards, rows, height: y + 4.0 + BTN_H + PAD }
}

/// Cuántos archivos por programa entran sin pasarse de `MAX_H` (a lo sumo `MAX_FILES`).
fn files_shown(d: &InUseDialog) -> usize {
    (0..=MAX_FILES).rev().find(|&k| layout(d, k).height <= MAX_H).unwrap_or(0)
}

/// Alto del contenido (sin la barra de título del sistema); nunca pasa de `MAX_H`.
pub fn height(d: &InUseDialog) -> f32 {
    layout(d, files_shown(d)).height
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

/// Un ícono de línea de 16 × 16 con la esquina de arriba a la izquierda en (x, y).
fn icon(p: &dyn Painter, kind: Icon, x: f32, y: f32, c: Color) {
    let s = Some((c, 1.3));
    match kind {
        Icon::None => {}
        Icon::Folder => p.rounded_path(
            &[(x + 1.0, y + 3.0, 1.5), (x + 6.0, y + 3.0, 0.0), (x + 8.0, y + 5.0, 0.0), (x + 15.0, y + 5.0, 1.5), (x + 15.0, y + 13.5, 1.5), (x + 1.0, y + 13.5, 1.5)],
            true,
            None,
            s,
        ),
        Icon::Process => {
            p.round_stroke_w(rect(x + 0.5, y + 2.0, x + 15.5, y + 14.0), 2.5, c, 1.3);
            p.rounded_path(&[(x + 4.0, y + 5.5, 0.0), (x + 7.0, y + 8.0, 0.0), (x + 4.0, y + 10.5, 0.0)], false, None, s);
            p.rounded_path(&[(x + 8.5, y + 10.5, 0.0), (x + 11.5, y + 10.5, 0.0)], false, None, s);
        }
        Icon::User => {
            p.circle_stroke(x + 8.0, y + 5.0, 3.0, c, 1.3);
            p.rounded_path(&[(x + 2.5, y + 15.0, 0.0), (x + 2.5, y + 13.0, 3.0), (x + 13.5, y + 13.0, 3.0), (x + 13.5, y + 15.0, 0.0)], false, None, s);
        }
        Icon::File => p.rounded_path(
            &[(x + 3.0, y + 1.0, 1.0), (x + 9.5, y + 1.0, 0.0), (x + 13.0, y + 4.5, 0.0), (x + 13.0, y + 15.0, 1.0), (x + 3.0, y + 15.0, 1.0)],
            true,
            None,
            s,
        ),
    }
}

/// Dibuja el contenido en `r` (recortado a él). `pressed` se ve solo mientras el mouse sigue
/// encima del mismo botón.
pub fn draw(p: &dyn Painter, r: Rect, d: &InUseDialog, hover: Option<InUseHit>, pressed: Option<InUseHit>) {
    let th = p.theme();
    p.fill(r, th.surface);
    p.clip(r, &mut || {
        let (l, rr) = (r.left + PAD, r.right - PAD);
        let lay = layout(d, files_shown(d));
        for line in &lay.lines {
            let h = if line.font == FontId::Title { 22.0 } else if line.font == FontId::Small { 16.0 } else { 18.0 };
            let top = r.top + line.top;
            let c = match line.tone {
                Tone::Normal => th.text,
                Tone::Dim => th.text_dim,
                Tone::Danger => th.danger,
            };
            p.text(line.font, &ellipsize(p, line.font, &line.text, rr - l), rect(l, top, rr, top + h), c);
        }
        for &(top, h) in &lay.cards {
            let c = rect(l, r.top + top, rr, r.top + top + h);
            p.round_fill(c, CARD_RADIUS, th.surface_alt);
            p.round_stroke(c, CARD_RADIUS, th.border);
        }
        for row in &lay.rows {
            let top = r.top + row.top;
            // Una raya entre filas de la misma tarjeta, no sobre su borde de arriba.
            if lay.cards.iter().all(|&(t, _)| t != row.top) {
                p.fill(rect(l + 1.0, top, rr - 1.0, top + 1.0), th.border);
            }
            icon(p, row.icon, l + ICON_X, top + (ROW_H - 16.0) / 2.0, th.text_secondary);
            let text = |x: f32, s: &str, c: Color| {
                let font = FontId::Ui;
                p.text(font, &ellipsize(p, font, s, rr - ICON_X - (l + x)), rect(l + x, top + 6.0, rr - ICON_X, top + 24.0), c);
            };
            text(LABEL_X, row.label, th.text_dim);
            text(VALUE_X, &row.value, if row.dim { th.text_dim } else { th.text });
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
        InUseDialog {
            title: "No se pudo eliminar «informe.xlsx»".into(),
            groups,
            more_groups,
            location: Some("Temp".into()),
            retry: InUseRetry::Op(crate::ops::Op::NewFolder { parent: std::path::PathBuf::from("/u") }),
        }
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
        // Tres programas con muchos archivos entran y cada uno muestra al menos uno.
        let many = dialog((1..=3).map(|p| group(p, MAX_FILES, 62)).collect(), 0);
        assert!(height(&many) <= crate::transfer::view::MAX_H);
        let shown = texts(&many).iter().filter(|(s, _)| s.starts_with("archivo")).count();
        assert!(shown >= 3, "{shown}");
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
    fn draw_shows_title_intro_card_rows_and_buttons() {
        let d = dialog(vec![HolderGroup { holder: excel(), files: vec!["informe.xlsx".into()], more_files: 0 }], 0);
        let t: Vec<String> = texts(&d).into_iter().map(|(s, _)| s).collect();
        for want in [
            "No se pudo eliminar «informe.xlsx»",
            "Microsoft Excel está usando el archivo.",
            "Ubicación",
            "Temp",
            "Proceso",
            "Microsoft Excel · PID 4812",
            "Usuario",
            r"PC\Leandro",
            "Reintentar",
            "Cerrar",
        ] {
            assert!(t.iter().any(|s| s == want), "falta «{want}» en {t:?}");
        }
        // Con un solo archivo el título ya lo dice: no hay fila de archivos.
        assert!(!t.iter().any(|s| s == "Archivos"), "{t:?}");
    }

    #[test]
    fn title_is_red() {
        let d = dialog(vec![group(1, 1, 0)], 0);
        let p = RecordingPainter::new(theme());
        draw(&p, area(&d), &d, None, None);
        let (.., c) = p.text_draws().into_iter().find(|(_, s, ..)| s.starts_with("No se pudo")).unwrap();
        assert_eq!(c, theme().danger);
    }

    #[test]
    fn several_files_get_their_own_rows() {
        let d = dialog(vec![group(1, 2, 0)], 0);
        let t: Vec<String> = texts(&d).into_iter().map(|(s, _)| s).collect();
        for want in ["Archivos", "archivo0.txt", "archivo1.txt"] {
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
        d.title = long.clone();
        d.location = Some(long);
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

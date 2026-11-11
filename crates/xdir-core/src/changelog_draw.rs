//! Dibujo de la pestaña Novedades (`Location::Changelog`): el CHANGELOG cortado en líneas al
//! ancho del panel, con desplazamiento. Medidas del diseño (X72 y los componentes
//! `changelog::draw_version` / `changelog::draw_item` en `docs/design/xdir.figcraft`).

use std::sync::OnceLock;

use crate::changelog::{self, Block, Span, Style};
use crate::geom::{Rect, rect};
use crate::paint::{FontId, Painter};

/// Del borde de la lista al contenido.
pub const LEFT: f32 = 32.0;
/// Ancho máximo del texto.
pub const MAX_W: f32 = 760.0;
/// Desde arriba de la lista hasta el contenido que se desplaza (el título queda fijo).
const TOP: f32 = 52.0;
pub const LINE_H: f32 = 20.0;
const HEADER_H: f32 = 40.0;
const HEADING_H: f32 = 28.0;
/// Entre una versión y la siguiente.
const VERSION_GAP: f32 = 16.0;
/// Sangría del texto de una viñeta y de una sub-viñeta.
const BULLET_X: f32 = 18.0;
const SUB_X: f32 = 36.0;
/// Margen del código a cada lado, dentro de su recuadro.
pub const CODE_PAD: f32 = 4.0;
/// Una línea de la rueda o de las flechas.
pub const STEP: f32 = 40.0;

/// Un tramo ya ubicado en su línea: `x` desde el comienzo del texto y su ancho (el código con su
/// margen).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub style: Style,
    pub text: String,
    pub x: f32,
    pub w: f32,
}

/// Corta los tramos en líneas de hasta `width`: por palabras, sin espacio al principio de una
/// línea; una palabra más larga que el ancho se corta por caracteres. El código no se parte salvo
/// que solo no entre. Junta lo seguido del mismo estilo en un solo tramo.
pub fn wrap(spans: &[Span], width: f32, measure: &dyn Fn(Style, &str) -> f32) -> Vec<Vec<Run>> {
    let size = |style: Style, s: &str| match style {
        Style::Code => measure(style, s) + CODE_PAD * 2.0,
        _ => measure(style, s),
    };
    let mut lines: Vec<Vec<Run>> = Vec::new();
    let mut line: Vec<Run> = Vec::new();
    let mut x = 0.0;
    for (style, token) in tokens(spans) {
        let mut text: &str = &token;
        if !line.is_empty() {
            let core = if style == Style::Code { text } else { text.trim_end() };
            if x + size(style, core) > width {
                lines.push(std::mem::take(&mut line));
                x = 0.0;
            }
        }
        if line.is_empty() && style != Style::Code {
            text = text.trim_start();
            if text.is_empty() {
                continue;
            }
        }
        // Solo en la línea y todavía no entra: se corta por caracteres.
        loop {
            let core = if style == Style::Code { text } else { text.trim_end() };
            if !line.is_empty() || size(style, core) <= width {
                break;
            }
            let cut = fit_prefix(text, |s| size(style, s) <= width);
            line.push(Run { style, text: text[..cut].to_string(), x: 0.0, w: size(style, &text[..cut]) });
            lines.push(std::mem::take(&mut line));
            x = 0.0;
            text = &text[cut..];
            if style != Style::Code {
                text = text.trim_start();
            }
            if text.is_empty() {
                break;
            }
        }
        if text.is_empty() {
            continue;
        }
        let w = size(style, text);
        line.push(Run { style, text: text.to_string(), x, w });
        x += w;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines.into_iter().map(merge).collect()
}

/// Palabras con su espacio de después (el código entero, como una sola).
fn tokens(spans: &[Span]) -> Vec<(Style, String)> {
    let mut out = Vec::new();
    for s in spans {
        if s.style == Style::Code {
            out.push((s.style, s.text.clone()));
            continue;
        }
        let mut cur = String::new();
        let mut in_space = false;
        for ch in s.text.chars() {
            let space = ch.is_whitespace();
            if !space && in_space && !cur.is_empty() {
                out.push((s.style, std::mem::take(&mut cur)));
            }
            in_space = space;
            cur.push(ch);
        }
        if !cur.is_empty() {
            out.push((s.style, cur));
        }
    }
    out
}

/// Cuántos bytes de `s` entran según `fits` (al menos un carácter).
fn fit_prefix(s: &str, fits: impl Fn(&str) -> bool) -> usize {
    if fits(s) {
        return s.len();
    }
    let mut best = s.chars().next().map_or(0, char::len_utf8);
    for (i, _) in s.char_indices().skip(1) {
        if !fits(&s[..i]) {
            break;
        }
        best = i;
    }
    best
}

fn merge(line: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for r in line {
        if let Some(last) = out.last_mut()
            && last.style == r.style
            && r.style != Style::Code
        {
            last.text.push_str(&r.text);
            last.w += r.w;
            continue;
        }
        out.push(r);
    }
    out
}

/// Una línea ya armada; `y` desde arriba del contenido.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Version { y: f32, title: String, sub: String },
    Heading { y: f32, text: String },
    /// `indent`: dónde empieza el texto; `bullet`: el nivel, en la primera línea de una viñeta.
    Text { y: f32, indent: f32, bullet: Option<u8>, runs: Vec<Run> },
}

impl Line {
    fn top(&self) -> f32 {
        match self {
            Line::Version { y, .. } | Line::Heading { y, .. } | Line::Text { y, .. } => *y,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Line::Version { .. } => HEADER_H,
            Line::Heading { .. } => HEADING_H,
            Line::Text { .. } => LINE_H,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Layout {
    width: f32,
    lines: Vec<Line>,
    height: f32,
    /// Dónde empieza cada versión (para abrir en una).
    anchors: Vec<(String, f32)>,
}

fn layout(blocks: &[Block], width: f32, measure: &dyn Fn(Style, &str) -> f32) -> Layout {
    let mut out = Layout { width, ..Default::default() };
    let mut y = 0.0;
    for b in blocks {
        match b {
            Block::Version { version, date } => {
                if !out.lines.is_empty() {
                    y += VERSION_GAP;
                }
                let (title, sub) = if version.is_empty() {
                    ("En desarrollo".to_string(), "todavía no publicado".to_string())
                } else {
                    (version.clone(), changelog::display_date(date))
                };
                out.anchors.push((version.clone(), y));
                out.lines.push(Line::Version { y, title, sub });
                y += HEADER_H;
            }
            Block::Heading(text) => {
                out.lines.push(Line::Heading { y, text: text.clone() });
                y += HEADING_H;
            }
            Block::Bullet { level, spans } => {
                let indent = if *level == 0 { BULLET_X } else { SUB_X + BULLET_X * (*level as f32 - 1.0) };
                let lines = wrap(spans, (width - indent).max(40.0), measure);
                for (i, runs) in lines.into_iter().enumerate() {
                    out.lines.push(Line::Text { y, indent, bullet: (i == 0).then_some(*level), runs });
                    y += LINE_H;
                }
            }
            Block::Para(spans) => {
                for runs in wrap(spans, width.max(40.0), measure) {
                    out.lines.push(Line::Text { y, indent: 0.0, bullet: None, runs });
                    y += LINE_H;
                }
            }
        }
    }
    out.height = y + 12.0;
    out
}

/// Los bloques del CHANGELOG incluido, leídos una vez.
pub fn bundled() -> &'static [Block] {
    static BLOCKS: OnceLock<Vec<Block>> = OnceLock::new();
    BLOCKS.get_or_init(|| changelog::parse(changelog::TEXT))
}

/// Estado de la vista: va en la pestaña, así se mueve con ella a otro panel.
#[derive(Debug, Default)]
pub struct ChangelogView {
    /// Desplazamiento del contenido (DIPs desde arriba).
    pub scroll: f32,
    /// Alto del contenido y del área visible en el último cuadro.
    pub content_h: f32,
    pub view_h: f32,
    /// Versión que se quiere arriba en el próximo cuadro (abrir desde el aviso de actualización).
    pub target: Option<String>,
    layout: Option<Layout>,
}

impl ChangelogView {
    /// Abierta en la versión `v` (en el próximo cuadro).
    pub fn at(v: Option<&str>) -> Self {
        Self { target: v.map(str::to_string), ..Default::default() }
    }

    /// Mueve el contenido `dy` DIPs (positivo: hacia abajo), sin pasarse.
    pub fn scroll_by(&mut self, dy: f32) -> bool {
        let max = (self.content_h - self.view_h).max(0.0);
        let to = (self.scroll + dy).clamp(0.0, max);
        let changed = (to - self.scroll).abs() > 0.1;
        self.scroll = to;
        changed
    }

    /// Una página (lo que se ve, menos una línea).
    pub fn page(&self) -> f32 {
        (self.view_h - STEP).max(STEP)
    }
}

fn font(style: Style) -> FontId {
    match style {
        Style::Plain => FontId::Ui,
        Style::Bold => FontId::UiBold,
        Style::Code => FontId::Mono,
    }
}

/// Dibuja la pestaña en `r` (el lugar de la lista): título fijo y el CHANGELOG que se desplaza.
pub fn draw(p: &dyn Painter, r: Rect, blocks: &[Block], view: &mut ChangelogView) {
    let th = p.theme();
    let left = r.left + LEFT;
    p.text(FontId::Title, "Novedades", rect(left, r.top + 14.0, r.right - 16.0, r.top + 40.0), th.text);
    let content = rect(left, r.top + TOP, r.right - 4.0, r.bottom - 4.0);
    if content.bottom <= content.top {
        return;
    }
    let width = (content.right - content.left - 24.0).clamp(200.0, MAX_W);
    if view.layout.as_ref().is_none_or(|l| (l.width - width).abs() > 0.5) {
        let measure = |style: Style, s: &str| p.measure(font(style), s);
        view.layout = Some(layout(blocks, width, &measure));
    }
    let Some(lay) = view.layout.as_ref() else { return };
    view.content_h = lay.height;
    view.view_h = content.bottom - content.top;
    if let Some(v) = view.target.take()
        && let Some((_, y)) = lay.anchors.iter().find(|(a, _)| *a == v)
    {
        view.scroll = *y;
    }
    view.scroll = view.scroll.clamp(0.0, (view.content_h - view.view_h).max(0.0));
    let scroll = view.scroll;
    p.clip(content, &mut || {
        for line in &lay.lines {
            let top = content.top + line.top() - scroll;
            if top + line.height() < content.top || top > content.bottom {
                continue;
            }
            draw_line(p, content.left, width, top, line);
        }
    });
    // Barra de desplazamiento fina a la derecha (como la configuración).
    let total = view.content_h;
    if total > view.view_h + 0.5 {
        let track = view.view_h;
        let thumb = (track * track / total).max(24.0);
        let y = content.top + (track - thumb) * (scroll / (total - track));
        p.round_fill(rect(content.right - 6.0, y, content.right - 2.0, y + thumb), 2.0, th.scrollbar);
    }
}

fn draw_line(p: &dyn Painter, x: f32, width: f32, top: f32, line: &Line) {
    let th = p.theme();
    match line {
        Line::Version { title, sub, .. } => {
            let tw = p.measure(FontId::Title, title);
            p.text(FontId::Title, title, rect(x, top + 4.0, x + tw + 8.0, top + 30.0), th.text);
            let sx = x + tw + 12.0;
            p.text(FontId::Ui, sub, rect(sx, top + 6.0, (x + width).max(sx + 1.0), top + 30.0), th.text_dim);
            p.fill(rect(x, top + HEADER_H - 7.0, x + width, top + HEADER_H - 6.0), th.border);
        }
        Line::Heading { text, .. } => {
            p.text(FontId::SmallBold, text, rect(x + 2.0, top + 8.0, x + width, top + HEADING_H), th.text_secondary);
        }
        Line::Text { indent, bullet, runs, .. } => {
            let cy = top + LINE_H / 2.0;
            match bullet {
                Some(0) => p.circle(x + indent - 10.0, cy, 2.5, th.text_dim),
                Some(_) => p.circle_stroke(x + indent - 10.0, cy, 2.5, th.text_dim, 1.0),
                None => {}
            }
            for run in runs {
                let rx = x + indent + run.x;
                if run.style == Style::Code {
                    let chip = rect(rx, top + 1.0, rx + run.w, top + LINE_H - 1.0);
                    p.round_fill(chip, 3.0, th.surface_alt);
                    p.round_stroke(chip, 3.0, th.border);
                    p.text(FontId::Mono, &run.text, rect(rx + CODE_PAD, top, rx + run.w + 4.0, top + LINE_H), th.text);
                } else {
                    let text = run.text.trim_end();
                    // Unos DIPs de más: el texto medido no se corta con «…».
                    p.text(font(run.style), text, rect(rx, top, rx + run.w + 8.0, top + LINE_H), th.text);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;

    /// 7 por carácter, 6 en el código.
    fn m(style: Style, s: &str) -> f32 {
        s.chars().count() as f32 * if style == Style::Code { 6.0 } else { 7.0 }
    }

    fn texts(lines: &[Vec<Run>]) -> Vec<String> {
        lines.iter().map(|l| l.iter().map(|r| r.text.as_str()).collect::<String>()).collect()
    }

    #[test]
    fn short_line_is_one_merged_run() {
        let lines = wrap(&changelog::spans("Uno dos tres."), 700.0, &m);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], vec![Run { style: Style::Plain, text: "Uno dos tres.".into(), x: 0.0, w: 13.0 * 7.0 }]);
    }

    #[test]
    fn wraps_by_words_without_leading_spaces() {
        // 10 caracteres por línea.
        let lines = wrap(&changelog::spans("uno dos tres cuatro cinco"), 70.0, &m);
        assert_eq!(texts(&lines), vec!["uno dos ", "tres ", "cuatro ", "cinco"]);
        for l in &lines {
            let end = l.last().map_or(0.0, |r| r.x + m(r.style, r.text.trim_end()));
            assert!(end <= 70.0, "{l:?}");
            assert!(!l[0].text.starts_with(' '));
        }
    }

    #[test]
    fn long_word_is_cut_by_characters() {
        let lines = wrap(&changelog::spans("x abcdefghijklmnopqrstuvwxyz y"), 70.0, &m);
        assert_eq!(texts(&lines), vec!["x ", "abcdefghij", "klmnopqrst", "uvwxyz y"]);
        // Con caracteres de varios bytes no se corta en el medio de uno.
        let lines = wrap(&changelog::spans("ññññññññññññ"), 35.0, &m);
        assert_eq!(texts(&lines), vec!["ñññññ", "ñññññ", "ññ"]);
        // Más angosto que un carácter: igual avanza.
        assert_eq!(wrap(&changelog::spans("ab"), 1.0, &m).len(), 2);
    }

    #[test]
    fn code_stays_whole_and_styles_keep_their_runs() {
        let sp = changelog::spans("**Duplicar** (`Ctrl+D`) y más");
        let lines = wrap(&sp, 700.0, &m);
        assert_eq!(lines.len(), 1);
        let styles: Vec<Style> = lines[0].iter().map(|r| r.style).collect();
        assert_eq!(styles, vec![Style::Bold, Style::Plain, Style::Code, Style::Plain]);
        let code = &lines[0][2];
        assert_eq!(code.text, "Ctrl+D");
        assert_eq!(code.w, 6.0 * 6.0 + CODE_PAD * 2.0);
        assert_eq!(code.x, lines[0][1].x + lines[0][1].w, "uno detrás del otro");
        // Sin lugar al final: el código pasa entero a la línea siguiente.
        let lines = wrap(&changelog::spans("abcdefgh `Ctrl+Shift+N`"), 120.0, &m);
        assert_eq!(texts(&lines), vec!["abcdefgh ", "Ctrl+Shift+N"]);
    }

    #[test]
    fn empty_spans_give_no_lines() {
        assert!(wrap(&[], 100.0, &m).is_empty());
        assert!(wrap(&changelog::spans("   "), 100.0, &m).is_empty());
    }

    #[test]
    fn layout_places_versions_and_bullets() {
        let blocks = changelog::parse("## 1.0.0 — 2027-01-02\n### Nuevo\n- uno dos tres cuatro\n  - sub\n## 0.9.0 — 2026-12-01\n- x\n");
        let lay = layout(&blocks, 100.0, &m);
        assert_eq!(lay.anchors, vec![("1.0.0".into(), 0.0), ("0.9.0".into(), HEADER_H + HEADING_H + LINE_H * 3.0 + VERSION_GAP)]);
        let bullets: Vec<(f32, Option<u8>)> =
            lay.lines.iter().filter_map(|l| if let Line::Text { indent, bullet, .. } = l { Some((*indent, *bullet)) } else { None }).collect();
        // «uno dos tres cuatro» en 82 de ancho (11 caracteres): dos líneas, la viñeta en la primera.
        assert_eq!(bullets, vec![(BULLET_X, Some(0)), (BULLET_X, None), (SUB_X, Some(1)), (BULLET_X, Some(0))]);
        assert!(matches!(&lay.lines[0], Line::Version { title, sub, .. } if title == "1.0.0" && sub == "02/01/2027"));
    }

    #[test]
    fn unreleased_shows_as_in_development() {
        let lay = layout(&changelog::parse("## Sin publicar\n- algo\n"), 300.0, &m);
        assert!(matches!(&lay.lines[0], Line::Version { title, sub, .. } if title == "En desarrollo" && sub == "todavía no publicado"));
    }

    fn painter() -> RecordingPainter {
        crate::sys::testing::install_fakes();
        RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)))
    }

    #[test]
    fn draw_shows_title_and_versions_and_scrolls_to_target() {
        let p = painter();
        let blocks = changelog::parse("## 1.0.0 — 2027-01-02\n- a\n- b\n- c\n## 0.9.0 — 2026-12-01\n- x\n");
        let r = rect(0.0, 0.0, 800.0, 200.0);
        let mut v = ChangelogView::default();
        draw(&p, r, &blocks, &mut v);
        let t: Vec<String> = p.texts().into_iter().map(|(_, s, _)| s).collect();
        assert!(t.contains(&"Novedades".to_string()) && t.contains(&"1.0.0".to_string()) && t.contains(&"02/01/2027".to_string()), "{t:?}");
        assert_eq!(v.scroll, 0.0);
        assert_eq!(v.view_h, 200.0 - 4.0 - TOP);
        // Abrir en la 0.9.0: queda arriba (o lo más arriba que se pueda).
        let mut v = ChangelogView::at(Some("0.9.0"));
        draw(&p, r, &blocks, &mut v);
        let want = (HEADER_H + LINE_H * 3.0 + VERSION_GAP).min(v.content_h - v.view_h);
        assert_eq!(v.scroll, want);
        assert!(v.target.is_none());
        // Una versión que no está no mueve nada.
        let mut v = ChangelogView::at(Some("7.7.7"));
        draw(&p, r, &blocks, &mut v);
        assert_eq!(v.scroll, 0.0);
    }

    #[test]
    fn scroll_by_stays_in_range() {
        let mut v = ChangelogView { content_h: 500.0, view_h: 200.0, ..Default::default() };
        assert!(v.scroll_by(1000.0));
        assert_eq!(v.scroll, 300.0);
        assert!(!v.scroll_by(10.0));
        assert!(v.scroll_by(-1000.0));
        assert_eq!(v.scroll, 0.0);
        assert_eq!(v.page(), 160.0);
    }

    /// El CHANGELOG del repo se arma a un ancho cualquiera sin líneas que se pasen.
    #[test]
    fn bundled_changelog_lays_out_within_width() {
        let lay = layout(bundled(), 400.0, &m);
        assert!(lay.height > 0.0);
        for l in &lay.lines {
            if let Line::Text { indent, runs, .. } = l {
                let end = runs.last().map_or(0.0, |r| r.x + if r.style == Style::Code { r.w } else { m(r.style, r.text.trim_end()) });
                assert!(indent + end <= 400.0 + 0.01, "{l:?}");
            }
        }
    }
}

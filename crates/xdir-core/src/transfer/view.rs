//! Contenido de la ventana de copia (pantallas X45–X47 de la tanda D): la cola de trabajos, el
//! trabajo expandido con su gráfico de velocidad, los conflictos y los errores al terminar. La
//! ventana es del sistema (`WindowHost::aux_show`); acá solo se mide, se dibuja y se ubica el clic.
//!
//! Medidas del diseño: 560 de ancho; trabajo expandido 290 (gráfico de 528×100), conflicto 316
//! (bloque de 528×222), errores 76 + 44 por error + 61; filas de la cola 64 (56 en cola).

use std::time::{Duration, Instant};

use super::{Choice, JobId, JobState, JobView};
use crate::format::thousands;
use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter, ellipsize};
use crate::sys::{ItemKind, ItemStat, SystemEnv};
use crate::theme::alpha;

/// Ancho del área de la ventana.
pub const WIDTH: f32 = 560.0;
const PAD: f32 = 16.0;
const EXPANDED_H: f32 = 290.0;
const CONFLICT_H: f32 = 316.0;
const ROW_H: f32 = 64.0;
const QUEUED_H: f32 = 56.0;
const ERROR_ROW: f32 = 44.0;
/// Errores que se listan; el resto se resume en una línea.
const MAX_ERRORS: usize = 5;
const MORE_H: f32 = 22.0;
const GRAPH_H: f32 = 100.0;
const BTN_W: f32 = 84.0;
const BTN_H: f32 = 28.0;
const ICON_BTN: f32 = 24.0;

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Lo que hay bajo el mouse en la ventana.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewHit {
    Pause(JobId),
    Resume(JobId),
    Cancel(JobId),
    /// Mostrar u ocultar los detalles (gráfico) de ese trabajo.
    Expand(JobId),
    Choose(JobId, Choice),
    /// La casilla «Hacer esto con todos».
    ApplyAll(JobId),
    Retry(JobId),
    /// «Cerrar» de un trabajo terminado con errores: sale de la cola.
    Dismiss(JobId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    Expanded,
    Conflict,
    Finished,
    Row,
    Queued,
}

fn block_of(j: &JobView, expanded: Option<JobId>) -> Block {
    match j.state {
        JobState::Done => Block::Finished,
        JobState::Conflict if j.conflict.is_some() => Block::Conflict,
        JobState::Waiting => Block::Queued,
        _ if expanded == Some(j.id) => Block::Expanded,
        _ => Block::Row,
    }
}

fn shown_errors(j: &JobView) -> usize {
    j.errors.len().min(MAX_ERRORS)
}

fn finished_h(j: &JobView) -> f32 {
    let more = if j.errors.len() > MAX_ERRORS { MORE_H } else { 0.0 };
    let list = if j.errors.is_empty() { 0.0 } else { ERROR_ROW * shown_errors(j) as f32 + 16.0 + more };
    76.0 + list + 45.0
}

fn block_h(j: &JobView, expanded: Option<JobId>) -> f32 {
    match block_of(j, expanded) {
        Block::Expanded => EXPANDED_H,
        Block::Conflict => CONFLICT_H,
        Block::Finished => finished_h(j),
        Block::Row => ROW_H,
        Block::Queued => QUEUED_H,
    }
}

/// Alto del contenido (sin la barra de título del sistema).
pub fn content_height(jobs: &[JobView], expanded: Option<JobId>) -> f32 {
    jobs.iter().map(|j| block_h(j, expanded)).sum()
}

/// Los botones y controles de cada trabajo, con su rectángulo.
fn controls(r: Rect, jobs: &[JobView], expanded: Option<JobId>) -> Vec<(ViewHit, Rect)> {
    let mut out = Vec::new();
    let mut top = r.top;
    for j in jobs {
        let h = block_h(j, expanded);
        let id = j.id;
        let right = r.right - PAD;
        let header_buttons = |out: &mut Vec<(ViewHit, Rect)>| {
            let cancel = rect(right - BTN_W, top + 31.0, right, top + 31.0 + BTN_H);
            let pause = rect(cancel.left - 8.0 - BTN_W, cancel.top, cancel.left - 8.0, cancel.bottom);
            out.push((if j.paused { ViewHit::Resume(id) } else { ViewHit::Pause(id) }, pause));
            out.push((ViewHit::Cancel(id), cancel));
        };
        match block_of(j, expanded) {
            Block::Expanded => {
                header_buttons(&mut out);
                out.push((ViewHit::Expand(id), rect(r.left + PAD, top + 254.0, r.left + PAD + 120.0, top + 276.0)));
            }
            Block::Conflict => {
                header_buttons(&mut out);
                let (x0, y0) = (r.left + PAD + 12.0, top + 75.0 + 150.0);
                let can_merge = j.conflict.as_ref().is_some_and(|c| c.can_merge);
                let mut choices = vec![(Choice::Replace, 104.0), (Choice::KeepBoth, 128.0), (Choice::Skip, 80.0)];
                if can_merge {
                    choices.insert(0, (Choice::Merge, 96.0));
                }
                let mut x = x0;
                for (c, w) in choices {
                    out.push((ViewHit::Choose(id, c), rect(x, y0, x + w, y0 + BTN_H)));
                    x += w + 8.0;
                }
                out.push((ViewHit::ApplyAll(id), rect(x0, top + 75.0 + 190.0, x0 + 360.0, top + 75.0 + 210.0)));
            }
            Block::Finished => {
                let y = top + h - 45.0;
                let close = rect(right - 96.0, y, right, y + BTN_H);
                out.push((ViewHit::Dismiss(id), close));
                if j.errors.iter().any(|e| e.retry) {
                    out.push((ViewHit::Retry(id), rect(close.left - 8.0 - 104.0, y, close.left - 8.0, y + BTN_H)));
                }
            }
            Block::Row | Block::Queued => {
                let cancel = rect(right - ICON_BTN + 4.0, top + 16.0, right + 4.0, top + 16.0 + ICON_BTN);
                if block_of(j, expanded) == Block::Row {
                    let pause = rect(cancel.left - 4.0 - ICON_BTN, cancel.top, cancel.left - 4.0, cancel.bottom);
                    out.push((if j.paused { ViewHit::Resume(id) } else { ViewHit::Pause(id) }, pause));
                    // El título (y la barra) abren los detalles.
                    out.push((ViewHit::Expand(id), rect(r.left + PAD, top + 8.0, pause.left - 8.0, top + h - 8.0)));
                }
                out.push((ViewHit::Cancel(id), cancel));
            }
        }
        top += h;
    }
    out
}

/// Qué control hay en (x, y).
pub fn hit(r: Rect, jobs: &[JobView], expanded: Option<JobId>, x: f32, y: f32) -> Option<ViewHit> {
    controls(r, jobs, expanded).into_iter().find(|(_, b)| contains(b, x, y)).map(|(h, _)| h)
}

/// «48,2 MB/s».
fn speed_text(env: &dyn SystemEnv, rate: f64) -> String {
    format!("{}/s", env.format_size_short(rate.max(0.0) as u64))
}

/// «quedan 2 min», «quedan 20 s», «quedan 1 h 5 min».
pub fn eta_short(d: Duration) -> String {
    let s = d.as_secs_f64().ceil() as u64;
    if s < 60 {
        format!("quedan {s} s")
    } else if s < 3600 {
        format!("quedan {} min", (s + 30) / 60)
    } else {
        format!("quedan {} h {} min", s / 3600, (s % 3600) / 60)
    }
}

/// «Unos 2 minutos», «Unos 20 segundos», «Alrededor de 1 h 5 min».
pub fn eta_long(d: Duration) -> String {
    let s = d.as_secs_f64().ceil() as u64;
    if s < 60 {
        if s <= 1 { "Un segundo".into() } else { format!("Unos {s} segundos") }
    } else if s < 3600 {
        let m = (s + 30) / 60;
        if m == 1 { "Un minuto".into() } else { format!("Unos {m} minutos") }
    } else {
        format!("Alrededor de {} h {} min", s / 3600, (s % 3600) / 60)
    }
}

/// Avance por bytes (o por elementos si no hay bytes), de 0 a 1.
pub fn fraction(j: &JobView) -> f32 {
    let (done, total) = if j.bytes.1 > 0 { j.bytes } else { j.items };
    if total == 0 { 0.0 } else { (done.min(total) as f64 / total as f64) as f32 }
}

fn percent(j: &JobView) -> u32 {
    (fraction(j) * 100.0).floor() as u32
}

/// La línea de estado de una fila: «58 % · 48,2 MB/s · quedan 2 min», «En pausa», «En cola»…
fn status_line(j: &JobView, env: &dyn SystemEnv, now: Instant) -> String {
    match j.state {
        JobState::Preparing => format!("Preparando… {} elementos", thousands(j.items.1)),
        JobState::Waiting => "En cola".into(),
        JobState::Conflict => format!("{} % · en espera", percent(j)),
        _ if j.paused => format!("{} % · En pausa", percent(j)),
        _ => {
            let mut s = format!("{} %", percent(j));
            if let Some(rate) = j.speed.rate() {
                s.push_str(&format!(" · {}", speed_text(env, rate)));
            }
            match j.speed.eta(now) {
                Some(eta) => s.push_str(&format!(" · {}", eta_short(eta))),
                None => s.push_str(" · Calculando…"),
            }
            s
        }
    }
}

fn button(p: &dyn Painter, b: Rect, label: &str, primary: bool, hot: bool) {
    let th = p.theme();
    if primary {
        p.round_fill(b, 4.0, if hot { alpha(th.accent, 0.85) } else { th.accent });
        p.text(FontId::UiCenter, label, rect(b.left, b.top + 5.0, b.right, b.bottom - 5.0), WHITE);
    } else {
        p.round_fill(b, 4.0, if hot { th.hover } else { th.surface_alt });
        p.round_stroke(b, 4.0, th.border);
        p.text(FontId::UiCenter, label, rect(b.left, b.top + 5.0, b.right, b.bottom - 5.0), th.text);
    }
}

fn icon_button(p: &dyn Painter, b: Rect, glyph: &str, hot: bool) {
    let th = p.theme();
    if hot {
        p.round_fill(b, 4.0, th.hover);
    }
    p.text(FontId::GlyphsSmall, glyph, rect(b.left, b.top + 4.0, b.right, b.bottom - 4.0), if hot { th.text } else { th.text_dim });
}

fn bar(p: &dyn Painter, b: Rect, f: f32, paused: bool) {
    let th = p.theme();
    p.round_fill(b, 2.0, th.border);
    if f > 0.0 {
        let fill = rect(b.left, b.top, b.left + (b.right - b.left) * f.min(1.0), b.bottom);
        p.round_fill(fill, 2.0, if paused { th.warning } else { th.accent });
    }
}

/// El gráfico de velocidad (528×100): el área de MB/s a lo largo del avance (`graph`, una muestra
/// por cada 1 %), la línea del promedio con «Promedio: …» y la velocidad actual («Velocidad: …»).
/// `remaining` (lo que falta) no se escribe en el gráfico: va en los detalles de abajo.
pub fn draw_graph(p: &dyn Painter, r: Rect, graph: &[f32], average: Option<f64>, remaining: Option<Duration>) {
    graph_with(p, r, graph, average, remaining, &|rate| format!("{}/s", crate::format::size_short(rate as u64)));
}

fn graph_with(p: &dyn Painter, r: Rect, graph: &[f32], average: Option<f64>, _remaining: Option<Duration>, speed: &dyn Fn(f64) -> String) {
    let th = p.theme();
    p.round_fill(r, 4.0, th.surface_alt);
    p.round_stroke(r, 4.0, th.border);
    let inner = rect(r.left + 1.0, r.top + 24.0, r.right - 1.0, r.bottom - 1.0);
    for k in 0..3 {
        let y = inner.top + (inner.bottom - inner.top) * k as f32 / 3.0;
        p.fill(rect(inner.left, y, inner.right, y + 1.0), alpha(th.border, 0.6));
    }
    let top = graph.iter().copied().fold(0.0_f32, f32::max).max(average.unwrap_or(0.0) as f32);
    let scale = if top > 0.0 { top * 1.25 } else { 1.0 };
    let h = inner.bottom - inner.top;
    let y_of = |v: f32| inner.bottom - h * (v / scale).clamp(0.0, 1.0);
    let step = (inner.right - inner.left) / 100.0;
    if !graph.is_empty() {
        let mut pts: Vec<(f32, f32, f32)> = vec![(inner.left, inner.bottom, 0.0)];
        pts.extend(graph.iter().enumerate().map(|(i, &v)| (inner.left + step * (i + 1) as f32, y_of(v), 0.0)));
        pts.push((inner.left + step * graph.len() as f32, inner.bottom, 0.0));
        p.rounded_path(&pts, true, Some(alpha(th.accent, 0.3)), None);
        p.rounded_path(&pts[1..pts.len() - 1], false, None, Some((th.accent, 1.5)));
    }
    if let Some(avg) = average {
        // La línea del promedio, tenue; la de la velocidad actual va encima.
        let y = y_of(avg as f32);
        p.fill(rect(inner.left, y, inner.right, y + 1.0), alpha(th.text_dim, 0.7));
        p.text(FontId::Small, &format!("Promedio: {}", speed(avg)), rect(r.left + 8.0, r.top + 5.0, r.right - 8.0, r.top + 21.0), th.text_dim);
    }
    if let Some(&now) = graph.last() {
        let y = y_of(now);
        p.fill(rect(inner.left, y, inner.right, y + 1.0), th.text);
        let label = format!("Velocidad: {}", speed(now as f64));
        let w = p.measure(FontId::Small, &label);
        let ly = (y - 18.0).max(r.top + 5.0);
        p.text(FontId::Small, &label, rect(r.right - 8.0 - w, ly, r.right - 8.0, ly + 16.0), th.text);
    }
}

fn job_glyph(j: &JobView) -> &'static str {
    if j.title.starts_with("Moviendo") || j.title.starts_with("Devolviendo") { "\u{E72A}" } else { "\u{E8C8}" }
}

fn header(p: &dyn Painter, r: Rect, top: f32, j: &JobView, big: &str, big_color: Color) {
    let th = p.theme();
    let (l, rr) = (r.left + PAD, r.right - PAD);
    p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &j.title, rr - l), rect(l, top + 12.0, rr, top + 30.0), th.text);
    p.text(FontId::Title, big, rect(l, top + 34.0, rr - 2.0 * BTN_W - 16.0, top + 56.0), big_color);
}

fn kind_glyph(s: &ItemStat) -> &'static str {
    if s.kind == ItemKind::Dir { "\u{E8B7}" } else { "\u{E8A5}" }
}

fn card(p: &dyn Painter, b: Rect, label: &str, name: &str, s: &ItemStat, newer: bool, env: &dyn SystemEnv) {
    let th = p.theme();
    p.round_fill(b, 6.0, th.surface);
    p.round_stroke(b, 6.0, th.border);
    p.text(FontId::Glyphs, kind_glyph(s), rect(b.left + 16.0, b.top + 28.0, b.left + 52.0, b.top + 68.0), th.text_dim);
    let x = b.left + 70.0;
    p.text(FontId::Small, label, rect(x, b.top + 10.0, b.right - 8.0, b.top + 26.0), th.text_dim);
    if newer {
        let lw = p.measure(FontId::Small, label);
        let tag = rect(x + lw + 8.0, b.top + 9.0, x + lw + 78.0, b.top + 27.0);
        p.round_fill(tag, 9.0, alpha(th.ok, 0.18));
        p.text(FontId::Small, "Más nuevo", rect(tag.left + 7.0, tag.top + 1.0, tag.right, tag.bottom - 1.0), th.ok);
    }
    p.text(FontId::Ui, &ellipsize(p, FontId::Ui, name, b.right - 8.0 - x), rect(x, b.top + 30.0, b.right - 8.0, b.top + 48.0), th.text);
    if s.kind != ItemKind::Dir {
        p.text(FontId::Small, &env.format_size_short(s.size), rect(x, b.top + 52.0, b.right - 8.0, b.top + 68.0), th.text_dim);
    }
    p.text(FontId::Small, &format!("Modificado: {}", env.format_date(s.modified)), rect(x, b.top + 70.0, b.right - 8.0, b.top + 86.0), th.text_dim);
}

fn checkbox(p: &dyn Painter, b: Rect, on: bool, label: &str, hot: bool) {
    let th = p.theme();
    let bx = rect(b.left, b.top + 3.0, b.left + 14.0, b.top + 17.0);
    if on {
        p.round_fill(bx, 3.0, th.accent);
        p.text(FontId::GlyphsSmall, "\u{E73E}", rect(bx.left, bx.top - 1.0, bx.right, bx.bottom), WHITE);
    } else {
        p.round_fill(bx, 3.0, th.surface);
        p.round_stroke_w(bx, 3.0, if hot { th.accent } else { th.text_dim }, 1.2);
    }
    p.text(FontId::Ui, label, rect(bx.right + 8.0, b.top, b.right, b.bottom), th.text);
}

fn choice_label(c: Choice) -> &'static str {
    match c {
        Choice::Replace => "Reemplazar",
        Choice::KeepBoth => "Conservar ambos",
        Choice::Skip => "Omitir",
        Choice::Merge => "Combinar",
    }
}

/// «Hacer esto con todos los archivos (12 conflictos)», «… todas las carpetas …»: vale para todos
/// los conflictos de esa clase que quedan en el trabajo (también adentro de las carpetas que se
/// combinen); el número va solo si se sabe exacto (este y los que quedan).
fn apply_all_label(c: &super::ConflictInfo) -> String {
    let base = if c.is_dir { "Hacer esto con todas las carpetas" } else { "Hacer esto con todos los archivos" };
    match c.remaining {
        Some(r) => format!("{base} ({})", plural(r + 1, "conflicto", "conflictos")),
        None => base.to_string(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Dibuja el contenido de la ventana en `r`.
pub fn draw(p: &dyn Painter, r: Rect, jobs: &[JobView], expanded: Option<JobId>, hover: Option<ViewHit>, env: &dyn SystemEnv) {
    let th = p.theme();
    p.fill(r, th.surface);
    let now = Instant::now();
    let ctl = controls(r, jobs, expanded);
    let find = |h: ViewHit| ctl.iter().find(|(x, _)| *x == h).map(|(_, b)| *b);
    let hot = |h: ViewHit| hover == Some(h);
    let (l, rr) = (r.left + PAD, r.right - PAD);
    let mut top = r.top;
    for (n, j) in jobs.iter().enumerate() {
        let h = block_h(j, expanded);
        let id = j.id;
        if n > 0 {
            p.fill(rect(r.left, top, r.right, top + 1.0), th.border);
        }
        let header_buttons = |paused: bool| {
            let pause_hit = if paused { ViewHit::Resume(id) } else { ViewHit::Pause(id) };
            if let Some(b) = find(pause_hit) {
                button(p, b, if paused { "Reanudar" } else { "Pausa" }, false, hot(pause_hit));
            }
            if let Some(b) = find(ViewHit::Cancel(id)) {
                button(p, b, "Cancelar", false, hot(ViewHit::Cancel(id)));
            }
        };
        match block_of(j, expanded) {
            Block::Expanded => {
                let big = match j.state {
                    JobState::Preparing => format!("Preparando… {} elementos", thousands(j.items.1)),
                    _ if j.paused => format!("{} % · en pausa", percent(j)),
                    _ => format!("{} % completado", percent(j)),
                };
                header(p, r, top, j, &big, th.text);
                header_buttons(j.paused);
                let g = rect(l, top + 71.0, rr, top + 71.0 + GRAPH_H);
                graph_with(p, g, j.speed.graph(), j.speed.average(), j.speed.eta(now), &|rate| speed_text(env, rate));
                let eta = if j.paused { "En pausa".to_string() } else { j.speed.eta(now).map_or("Calculando…".into(), eta_long) };
                let left_items = j.items.1.saturating_sub(j.items.0);
                let left_bytes = j.bytes.1.saturating_sub(j.bytes.0);
                let rows = [
                    ("Nombre:", j.current.clone()),
                    ("Tiempo restante:", eta),
                    ("Elementos restantes:", format!("{} ({})", thousands(left_items), env.format_size_short(left_bytes))),
                ];
                for (k, (label, value)) in rows.iter().enumerate() {
                    let y = top + 182.0 + 22.0 * k as f32;
                    p.text(FontId::Ui, label, rect(l, y, l + 150.0, y + 18.0), th.text_dim);
                    p.text(FontId::Ui, &ellipsize(p, FontId::Ui, value, rr - l - 156.0), rect(l + 156.0, y, rr, y + 18.0), th.text);
                }
                if let Some(b) = find(ViewHit::Expand(id)) {
                    let c = if hot(ViewHit::Expand(id)) { th.text } else { th.text_dim };
                    p.text(FontId::GlyphsSmall, "\u{E70E}", rect(b.left, b.top + 3.0, b.left + 14.0, b.bottom - 3.0), c);
                    p.text(FontId::Ui, "Menos detalles", rect(b.left + 18.0, b.top + 2.0, b.right, b.bottom - 2.0), c);
                }
            }
            Block::Conflict => {
                let Some(c) = &j.conflict else { continue };
                header(p, r, top, j, &format!("{} % · en espera", percent(j)), th.text);
                header_buttons(j.paused);
                let bx = rect(l, top + 75.0, rr, top + 75.0 + 222.0);
                p.round_fill(bx, 6.0, th.surface_alt);
                p.round_stroke(bx, 6.0, th.border);
                let (x0, y0) = (bx.left + 12.0, bx.top);
                p.text(FontId::GlyphsSmall, "\u{E7BA}", rect(x0, y0 + 12.0, x0 + 16.0, y0 + 28.0), th.warning);
                let title = format!("Ya existe «{}» en «{}»", c.name, c.folder);
                p.text(FontId::UiBold, &ellipsize(p, FontId::UiBold, &title, bx.right - x0 - 36.0), rect(x0 + 24.0, y0 + 11.0, bx.right - 12.0, y0 + 29.0), th.text);
                let cw = (bx.right - bx.left - 24.0 - 8.0) / 2.0;
                let incoming = rect(x0, y0 + 40.0, x0 + cw, y0 + 136.0);
                let existing = rect(incoming.right + 8.0, incoming.top, incoming.right + 8.0 + cw, incoming.bottom);
                let newer = c.incoming.modified > c.existing.modified;
                card(p, incoming, "El que se copia", &c.name, &c.incoming, newer, env);
                card(p, existing, "El que ya está", &c.name, &c.existing, !newer && c.existing.modified > c.incoming.modified, env);
                let first = if c.can_merge { Choice::Merge } else { Choice::Replace };
                for ch in [Choice::Merge, Choice::Replace, Choice::KeepBoth, Choice::Skip] {
                    if let Some(b) = find(ViewHit::Choose(id, ch)) {
                        button(p, b, choice_label(ch), ch == first, hot(ViewHit::Choose(id, ch)));
                    }
                }
                if let Some(b) = find(ViewHit::ApplyAll(id)) {
                    let label = apply_all_label(c);
                    checkbox(p, b, j.apply_all, &label, hot(ViewHit::ApplyAll(id)));
                }
            }
            Block::Finished => {
                let (big, color) = if let Some(why) = &j.stopped {
                    (why.clone(), th.danger)
                } else if j.errors.is_empty() {
                    ("Terminó".to_string(), th.text)
                } else {
                    (format!("Terminó con {}", plural(j.errors.len(), "error", "errores")), th.danger)
                };
                header(p, r, top, j, &big, color);
                if !j.errors.is_empty() {
                    let list = rect(l, top + 76.0, rr, top + 76.0 + ERROR_ROW * shown_errors(j) as f32);
                    p.round_fill(list, 6.0, th.surface_alt);
                    p.round_stroke(list, 6.0, th.border);
                    for (k, e) in j.errors.iter().take(MAX_ERRORS).enumerate() {
                        let y = list.top + ERROR_ROW * k as f32;
                        if k > 0 {
                            p.fill(rect(list.left, y, list.right, y + 1.0), th.border);
                        }
                        let name = e.src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| e.src.display().to_string());
                        let folder = e.src.parent().map(|d| d.display().to_string()).unwrap_or_default();
                        p.text(FontId::GlyphsSmall, "\u{E8A5}", rect(list.left + 12.0, y + 12.0, list.left + 28.0, y + 30.0), th.text_dim);
                        let mw = p.measure(FontId::Ui, &e.message).min(220.0);
                        let nx = list.left + 38.0;
                        let nr = list.right - 16.0 - mw - 12.0;
                        p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &name, nr - nx), rect(nx, y + 6.0, nr, y + 24.0), th.text);
                        p.text(FontId::Small, &ellipsize(p, FontId::Small, &folder, nr - nx), rect(nx, y + 24.0, nr, y + 40.0), th.text_dim);
                        p.text(FontId::UiRight, &e.message, rect(list.right - 16.0 - mw, y + 13.0, list.right - 12.0, y + 31.0), th.danger);
                    }
                    if j.errors.len() > MAX_ERRORS {
                        let y = list.bottom + 4.0;
                        p.text(FontId::Small, &format!("y {} más", j.errors.len() - MAX_ERRORS), rect(l, y, rr, y + 16.0), th.text_dim);
                    }
                }
                if let Some(b) = find(ViewHit::Retry(id)) {
                    button(p, b, "Reintentar", true, hot(ViewHit::Retry(id)));
                }
                if let Some(b) = find(ViewHit::Dismiss(id)) {
                    button(p, b, "Cerrar", false, hot(ViewHit::Dismiss(id)));
                }
            }
            Block::Row | Block::Queued => {
                let queued = block_of(j, expanded) == Block::Queued;
                p.text(FontId::GlyphsSmall, job_glyph(j), rect(l, top + 13.0, l + 16.0, top + 31.0), th.text_dim);
                let tr = if queued { rr - ICON_BTN - 8.0 } else { rr - 2.0 * ICON_BTN - 16.0 };
                p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &j.title, tr - l - 24.0), rect(l + 24.0, top + 12.0, tr, top + 30.0), th.text);
                if queued {
                    p.text(FontId::Small, "En cola", rect(l + 24.0, top + 32.0, tr, top + 48.0), th.text_dim);
                } else {
                    bar(p, rect(l + 24.0, top + 36.0, r.left + 480.0, top + 40.0), fraction(j), j.paused);
                    p.text(FontId::Small, &status_line(j, env, now), rect(l + 24.0, top + 44.0, tr, top + 60.0), th.text_dim);
                    let ph = if j.paused { ViewHit::Resume(id) } else { ViewHit::Pause(id) };
                    if let Some(b) = find(ph) {
                        icon_button(p, b, if j.paused { "\u{E768}" } else { "\u{E769}" }, hot(ph));
                    }
                }
                if let Some(b) = find(ViewHit::Cancel(id)) {
                    icon_button(p, b, "\u{E711}", hot(ViewHit::Cancel(id)));
                }
            }
        }
        top += h;
    }
}

/// Título de la ventana al mostrarla: el conflicto, los errores o «Copiando…».
pub fn window_title(jobs: &[JobView]) -> String {
    if jobs.iter().any(|j| j.state == JobState::Conflict && j.conflict.is_some()) {
        return "Copiando: un conflicto".into();
    }
    if let Some(j) = jobs.iter().find(|j| j.state == JobState::Done && !j.errors.is_empty()) {
        return format!("Terminó con {}", plural(j.errors.len(), "error", "errores"));
    }
    "Copiando…".into()
}

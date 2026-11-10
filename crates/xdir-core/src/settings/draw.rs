//! Dibujo de la pestaña Configuración: la lista de secciones a la izquierda, el título de la
//! página y las filas (con desplazamiento). Medidas del diseño (tanda D+, X54–X59): lista de 220
//! (ítems de 204×32 cada 34), contenido desde 252 con filas de hasta 820 y 4 entre filas.

use std::cell::RefCell;

use super::controls::{self, BUTTON_W, CONTROL_H, DROPDOWN_W, SEGMENT_W, SLIDER_VALUE_W, SLIDER_W, SWITCH_H, SWITCH_W};
use super::keys::{RowState, ShortcutRow};
use super::{AccentChoice, Control, Item, PageId, Row, SettingsHit, SettingsView};
use crate::geom::{Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::theme::{PRESETS, alpha};

pub const NAV_W: f32 = 220.0;
const NAV_ITEM_W: f32 = 204.0;
const NAV_ITEM_H: f32 = 32.0;
const NAV_STEP: f32 = 34.0;
/// Del borde izquierdo de la lista al contenido.
pub const CONTENT_X: f32 = 244.0;
pub const ROW_W: f32 = 820.0;
pub const ROW_H: f32 = 56.0;
const ROW_GAP: f32 = 4.0;
const ACCENT_H: f32 = 104.0;
const RULE_H: f32 = 22.0;
pub const SHORTCUT_H: f32 = 36.0;
const CHIP_W: f32 = 104.0;
const CHIP_H: f32 = 24.0;
const CHIPS_X: f32 = 300.0;
const ICON_STEP: f32 = 28.0;
/// Encabezado de la página (título).
const TITLE_H: f32 = 52.0;

/// Una página de la lista de la izquierda; `count` con una búsqueda (cuántos resultados tiene).
#[derive(Clone, Debug)]
pub struct NavItem {
    pub page: PageId,
    pub title: String,
    pub count: Option<usize>,
}

/// La lista de secciones, la línea que la separa y el título de la página. Empieza de cero los
/// lugares de `view.hits` y anota los de la lista. Devuelve el área del contenido.
pub fn frame(p: &dyn Painter, r: Rect, nav: &[NavItem], selected: Option<PageId>, title: &str, hover: Option<SettingsHit>, view: &mut SettingsView) -> Rect {
    let th = p.theme();
    view.hits.clear();
    p.fill(rect(r.left + NAV_W, r.top, r.left + NAV_W + 1.0, r.bottom), th.border);
    for (i, item) in nav.iter().enumerate() {
        let top = r.top + 12.0 + NAV_STEP * i as f32;
        let ir = rect(r.left + 8.0, top, r.left + 8.0 + NAV_ITEM_W, top + NAV_ITEM_H);
        if ir.bottom > r.bottom {
            break;
        }
        let chosen = selected == Some(item.page);
        if chosen {
            p.round_fill(ir, 6.0, alpha(th.accent, 0.22));
            p.round_fill(rect(ir.left, ir.top + 8.0, ir.left + 3.0, ir.bottom - 8.0), 1.5, th.accent);
        } else if hover == Some(SettingsHit::Nav(item.page)) {
            p.round_fill(ir, 6.0, th.hover);
        }
        let dim = item.count == Some(0);
        let color = if dim { alpha(th.text_dim, 0.6) } else { th.text };
        p.text(FontId::Ui, &item.title, rect(ir.left + 12.0, ir.top, ir.right - 40.0, ir.bottom), color);
        if let Some(n) = item.count {
            let c = if dim { alpha(th.text_dim, 0.6) } else { th.text_dim };
            p.text(FontId::UiRight, &n.to_string(), rect(ir.right - 40.0, ir.top, ir.right - 10.0, ir.bottom), c);
        }
        view.hits.push((ir, SettingsHit::Nav(item.page)));
    }
    let left = r.left + CONTENT_X;
    p.text(FontId::Title, title, rect(left, r.top + 14.0, r.right - 16.0, r.top + 40.0), th.text);
    let content = rect(left, r.top + TITLE_H, r.right - 4.0, r.bottom - 4.0);
    view.content = content;
    content
}

/// Ancho de las filas en un área de contenido: 820, o lo que entre.
fn row_width(content: Rect) -> f32 {
    (content.right - content.left - 16.0).clamp(200.0, ROW_W)
}

/// Alto de un elemento.
fn item_height(item: &Item) -> f32 {
    match item {
        Item::Heading(_) => 30.0,
        Item::Note(_) => 26.0,
        Item::Error(..) => 36.0,
        Item::Action(..) => 36.0,
        Item::Row(row) => match &row.control {
            Control::Accent(_) => ACCENT_H,
            Control::ColorRules(rules) => 58.0 + RULE_H * rules.len() as f32,
            Control::Table(rows) => 48.0 + RULE_H * rows.len() as f32,
            _ => ROW_H,
        },
        Item::Shortcut(s) => match s.state {
            RowState::Conflict(_) => 76.0,
            RowState::Rejected(_) => 64.0,
            _ => SHORTCUT_H,
        },
    }
}

/// Las filas, desde arriba del área menos el desplazamiento, recortadas al área. Anota el alto
/// total y los lugares de cada parte en `view`.
pub fn items(p: &dyn Painter, content: Rect, items: &[Item], hover: Option<SettingsHit>, view: &mut SettingsView) {
    let total: f32 = items.iter().map(|i| item_height(i) + ROW_GAP).sum::<f32>() + 12.0;
    view.content_h = total;
    view.view_h = content.bottom - content.top;
    view.scroll = view.scroll.clamp(0.0, (total - view.view_h).max(0.0));
    let w = row_width(content);
    let hits = RefCell::new(Vec::new());
    let mut y = content.top - view.scroll;
    let mut placed = Vec::new();
    for item in items {
        let h = item_height(item);
        placed.push((rect(content.left, y, content.left + w, y + h), item));
        y += h + ROW_GAP;
    }
    p.clip(content, &mut || {
        let mut hits = hits.borrow_mut();
        for (r, item) in &placed {
            if r.bottom < content.top || r.top > content.bottom {
                continue;
            }
            draw_item(p, *r, item, hover, &mut hits);
        }
    });
    // Solo cuenta lo que se ve.
    for (r, h) in hits.into_inner() {
        let top = r.top.max(content.top);
        let bottom = r.bottom.min(content.bottom);
        if bottom > top {
            view.hits.push((rect(r.left, top, r.right, bottom), h));
        }
    }
    view.hits.push((content, SettingsHit::Body));
    // Barra de desplazamiento fina a la derecha.
    if total > view.view_h + 0.5 {
        let th = p.theme();
        let track = view.view_h;
        let thumb = (track * track / total).max(24.0);
        let top = content.top + (track - thumb) * (view.scroll / (total - track));
        p.round_fill(rect(content.right - 6.0, top, content.right - 2.0, top + thumb), 2.0, th.scrollbar);
    }
}

fn hot(hover: Option<SettingsHit>, id: &'static str, part: usize) -> bool {
    hover == Some(SettingsHit::Part(id, part))
}

fn draw_item(p: &dyn Painter, r: Rect, item: &Item, hover: Option<SettingsHit>, hits: &mut Vec<(Rect, SettingsHit)>) {
    let th = p.theme();
    match item {
        Item::Heading(text) => {
            p.text(FontId::SmallBold, text, rect(r.left + 2.0, r.top + 8.0, r.right, r.bottom), th.text_secondary);
        }
        Item::Note(text) => p.text(FontId::Small, text, rect(r.left + 2.0, r.top + 4.0, r.right, r.bottom), th.text_dim),
        Item::Error(text, button) => {
            p.round_fill(r, 6.0, alpha(th.danger, 0.12));
            p.round_stroke(r, 6.0, alpha(th.danger, 0.6));
            let right = if button.is_some() { r.right - BUTTON_W - 24.0 } else { r.right - 12.0 };
            p.text(FontId::Ui, text, rect(r.left + 12.0, r.top, right, r.bottom), th.danger);
            if let Some((id, label)) = button {
                let b = rect(r.right - BUTTON_W - 8.0, r.top + 4.0, r.right - 8.0, r.bottom - 4.0);
                controls::button(p, b, label, hot(hover, id, 0), true);
                hits.push((b, SettingsHit::Part(id, 0)));
            }
        }
        Item::Action(id, label) => {
            let b = rect(r.right - BUTTON_W, r.top + 4.0, r.right, r.top + 4.0 + CONTROL_H);
            controls::button(p, b, label, hot(hover, id, 0), true);
            hits.push((b, SettingsHit::Part(id, 0)));
        }
        Item::Row(row) => draw_row(p, r, row, hover, hits),
        Item::Shortcut(s) => draw_shortcut(p, r, s, hover, hits),
    }
}

/// Tarjeta de una opción: título, descripción y el control a 16 del borde derecho.
fn draw_row(p: &dyn Painter, r: Rect, row: &Row, hover: Option<SettingsHit>, hits: &mut Vec<(Rect, SettingsHit)>) {
    let th = p.theme();
    p.round_fill(r, 6.0, th.surface_alt);
    p.round_stroke(r, 6.0, th.border);
    let title_color = if row.enabled { th.text } else { alpha(th.text_dim, 0.7) };
    let cy = r.top + ROW_H / 2.0;
    let right = r.right - 16.0;
    // Lo que ocupa el control, para que el texto no lo pise.
    let control_w = match &row.control {
        Control::Switch(_) => SWITCH_W,
        Control::Segmented(opts, _) => SEGMENT_W * opts.len() as f32,
        Control::Dropdown(_) => DROPDOWN_W,
        Control::Slider { .. } => SLIDER_W + SLIDER_VALUE_W,
        Control::Buttons(b) => (BUTTON_W + 8.0) * b.len() as f32,
        Control::ColorRules(_) => BUTTON_W,
        _ => 0.0,
    };
    let text_right = right - control_w - 16.0;
    p.text(FontId::Ui, &row.title, rect(r.left + 16.0, r.top + 10.0, text_right, r.top + 28.0), title_color);
    if !row.desc.is_empty() {
        p.text(FontId::Small, &row.desc, rect(r.left + 16.0, r.top + 31.0, text_right, r.top + 47.0), th.text_dim);
    }
    let id = row.id;
    match &row.control {
        Control::None => {}
        Control::Switch(on) => {
            let s = rect(right - SWITCH_W, cy - SWITCH_H / 2.0, right, cy + SWITCH_H / 2.0);
            controls::switch(p, s, *on, row.enabled, hot(hover, id, 0));
            if row.enabled {
                hits.push((rect(s.left - 6.0, r.top + 6.0, s.right + 6.0, r.top + ROW_H - 6.0), SettingsHit::Part(id, 0)));
            }
        }
        Control::Segmented(opts, selected) => {
            let w = SEGMENT_W * opts.len() as f32;
            let s = rect(right - w, cy - CONTROL_H / 2.0, right, cy + CONTROL_H / 2.0);
            let hot_part = match hover {
                Some(SettingsHit::Part(h, i)) if h == id => Some(i),
                _ => None,
            };
            for (i, part) in controls::segmented(p, s, opts, *selected, hot_part).into_iter().enumerate() {
                hits.push((part, SettingsHit::Part(id, i)));
            }
        }
        Control::Dropdown(text) => {
            let d = rect(right - DROPDOWN_W, cy - CONTROL_H / 2.0, right, cy + CONTROL_H / 2.0);
            controls::dropdown(p, d, text, hot(hover, id, 0), row.enabled);
            if row.enabled {
                hits.push((d, SettingsHit::Part(id, 0)));
            }
        }
        Control::Slider { value, min, max } => {
            let s = rect(right - SLIDER_VALUE_W - SLIDER_W, cy - CONTROL_H / 2.0, right - SLIDER_VALUE_W, cy + CONTROL_H / 2.0);
            controls::slider(p, s, *value, *min, *max, hot(hover, id, 0));
            p.text(FontId::UiRight, &format!("{} px", value.round()), rect(s.right, s.top, right, s.bottom), th.text);
            hits.push((s, SettingsHit::Part(id, 0)));
        }
        Control::Buttons(buttons) => {
            let mut x = right;
            for (i, (label, enabled)) in buttons.iter().enumerate().rev() {
                let b = rect(x - BUTTON_W, cy - CONTROL_H / 2.0, x, cy + CONTROL_H / 2.0);
                controls::button(p, b, label, hot(hover, id, i), *enabled);
                if *enabled {
                    hits.push((b, SettingsHit::Part(id, i)));
                }
                x = b.left - 8.0;
            }
        }
        Control::Accent(choice) => {
            let y = r.top + 74.0;
            for (i, (_, c)) in PRESETS.iter().enumerate() {
                let cx = r.left + 16.0 + 15.0 + 34.0 * i as f32;
                let color = crate::ui::hex(*c);
                if *choice == AccentChoice::Preset(i) {
                    p.circle_stroke(cx, y, 15.0, th.text, 2.0);
                } else if hot(hover, id, i) {
                    p.circle_stroke(cx, y, 15.0, th.text_dim, 1.0);
                }
                p.circle(cx, y, 11.0, color);
                hits.push((rect(cx - 17.0, y - 17.0, cx + 17.0, y + 17.0), SettingsHit::Part(id, i)));
            }
            let x = r.left + 16.0 + 34.0 * PRESETS.len() as f32 + 24.0;
            let sys = rect(x, y - 14.0, x + 120.0, y + 14.0);
            controls::radio(p, sys, "Del sistema", *choice == AccentChoice::System, hot(hover, id, PRESETS.len()));
            hits.push((sys, SettingsHit::Part(id, PRESETS.len())));
            let other = rect(sys.right + 8.0, y - 14.0, sys.right + 100.0, y + 14.0);
            let is_other = matches!(choice, AccentChoice::Other(_));
            controls::radio(p, other, "Otro…", is_other, hot(hover, id, PRESETS.len() + 1));
            if let AccentChoice::Other(c) = choice {
                p.circle(other.right + 8.0, y, 8.0, crate::ui::hex(*c));
            }
            hits.push((other, SettingsHit::Part(id, PRESETS.len() + 1)));
        }
        Control::ColorRules(rules) => {
            let b = rect(right - BUTTON_W, r.top + 14.0, right, r.top + 14.0 + CONTROL_H);
            controls::button(p, b, "Editar en config.toml", hot(hover, id, 0), true);
            hits.push((b, SettingsHit::Part(id, 0)));
            for (i, (pattern, color, own)) in rules.iter().enumerate() {
                let top = r.top + 52.0 + RULE_H * i as f32;
                p.round_fill(rect(r.left + 16.0, top + 5.0, r.left + 28.0, top + 17.0), 3.0, *color);
                let label_right = if *own { r.right - 90.0 } else { r.right - 16.0 };
                p.text(FontId::Small, pattern, rect(r.left + 38.0, top, label_right, top + RULE_H), th.text_secondary);
                if *own {
                    p.text(FontId::Small, "propia", rect(r.right - 80.0, top, r.right - 16.0, top + RULE_H), th.accent);
                }
            }
        }
        Control::Table(rows) => {
            for (i, (a, b)) in rows.iter().enumerate() {
                let top = r.top + 40.0 + RULE_H * i as f32;
                p.text(FontId::Small, a, rect(r.left + 16.0, top, r.left + 176.0, top + RULE_H), th.text);
                p.text(FontId::Small, b, rect(r.left + 184.0, top, r.right - 16.0, top + RULE_H), th.text_dim);
            }
        }
    }
}

/// Fila de un comando en Atajos: nombre, chips de sus atajos y los botones ✎ + ↺.
/// Partes: 0 ✎, 1 +, 2 ↺, 10+i el chip i, 20+i la ✕ del chip i, 30 Reasignar, 31 Cancelar.
pub const PART_EDIT: usize = 0;
pub const PART_ADD: usize = 1;
pub const PART_RESET: usize = 2;
pub const PART_CHIP: usize = 10;
pub const PART_CHIP_REMOVE: usize = 20;
pub const PART_REASSIGN: usize = 30;
pub const PART_CANCEL: usize = 31;

fn draw_shortcut(p: &dyn Painter, r: Rect, s: &ShortcutRow, hover: Option<SettingsHit>, hits: &mut Vec<(Rect, SettingsHit)>) {
    let th = p.theme();
    let id = s.id;
    let line = rect(r.left, r.top, r.right, r.top + SHORTCUT_H);
    let row_hot = matches!(hover, Some(SettingsHit::Part(h, _)) if h == id);
    if s.state != RowState::Normal {
        p.round_fill(r, 6.0, th.surface_alt);
        p.round_stroke(r, 6.0, if matches!(s.state, RowState::Recording) { th.accent } else { th.border });
    } else if row_hot {
        p.round_fill(line, 6.0, th.hover);
    }
    p.text(FontId::Ui, &s.title, rect(r.left + 12.0, line.top, r.left + CHIPS_X - 12.0, line.bottom), th.text);
    let cy = (line.top + line.bottom) / 2.0;
    let mut x = r.left + CHIPS_X;
    if s.state == RowState::Recording {
        let box_r = rect(x, cy - CHIP_H / 2.0, x + CHIP_W * 2.0 + 8.0, cy + CHIP_H / 2.0);
        p.round_fill(box_r, 6.0, alpha(th.accent, 0.15));
        p.round_stroke(box_r, 6.0, th.accent);
        p.text(FontId::UiCenter, "Apretá la combinación…", box_r, th.text);
        p.text(FontId::Small, "Esc cancela", rect(box_r.right + 10.0, line.top, box_r.right + 110.0, line.bottom), th.text_dim);
        x = box_r.right + 110.0;
    } else if s.chips.is_empty() {
        p.text(FontId::Ui, "Sin atajo", rect(x, line.top, x + CHIP_W, line.bottom), th.text_dim);
        x += CHIP_W + 8.0;
    } else {
        for (i, chip) in s.chips.iter().enumerate() {
            let c = rect(x, cy - CHIP_H / 2.0, x + CHIP_W, cy + CHIP_H / 2.0);
            p.round_fill(c, 5.0, th.surface);
            p.round_stroke(c, 5.0, th.border);
            let chip_hot = hot(hover, id, PART_CHIP + i) || hot(hover, id, PART_CHIP_REMOVE + i);
            let label_r = if chip_hot { rect(c.left, c.top, c.right - 18.0, c.bottom) } else { c };
            p.text(FontId::UiCenter, chip, label_r, th.text);
            hits.push((c, SettingsHit::Part(id, PART_CHIP + i)));
            if chip_hot {
                let x_r = rect(c.right - 20.0, c.top + 3.0, c.right - 3.0, c.bottom - 3.0);
                if hot(hover, id, PART_CHIP_REMOVE + i) {
                    p.round_fill(x_r, 4.0, alpha(th.text, 0.12));
                }
                p.text(FontId::GlyphsSmall, "\u{E711}", x_r, th.text_dim);
                hits.push((x_r, SettingsHit::Part(id, PART_CHIP_REMOVE + i)));
            }
            x = c.right + 8.0;
        }
    }
    if s.changed && s.state == RowState::Normal {
        p.text(FontId::Small, "(cambiado)", rect(x + 4.0, line.top, x + 90.0, line.bottom), th.text_dim);
    }
    // Botones a la derecha: ✎ + ↺ (↺ solo si cambió).
    let buttons: &[(usize, &str)] = &[(PART_EDIT, "\u{E70F}"), (PART_ADD, "\u{E710}"), (PART_RESET, "\u{E7A7}")];
    let mut bx = r.right - 8.0 - ICON_STEP * 3.0;
    for &(part, glyph) in buttons {
        let b = rect(bx + 2.0, cy - 12.0, bx + 26.0, cy + 12.0);
        bx += ICON_STEP;
        if part == PART_RESET && !s.changed {
            continue;
        }
        if s.state == RowState::Recording && part != PART_EDIT {
            continue;
        }
        if hot(hover, id, part) {
            p.round_fill(b, 4.0, alpha(th.text, 0.12));
        }
        let glyph = if s.state == RowState::Recording { "\u{E711}" } else { glyph };
        p.text(FontId::GlyphsSmall, glyph, b, th.text_dim);
        hits.push((b, SettingsHit::Part(id, part)));
    }
    hits.push((line, SettingsHit::Part(id, usize::MAX)));
    match &s.state {
        RowState::Conflict(msg) => {
            let t = rect(r.left + CHIPS_X, line.bottom, r.right - 12.0, line.bottom + 18.0);
            p.text(FontId::Small, &format!("⚠ {msg}"), t, th.warning);
            let cancel = rect(r.right - 12.0 - 100.0, line.bottom + 14.0, r.right - 12.0, line.bottom + 36.0);
            let reassign = rect(cancel.left - 108.0, cancel.top, cancel.left - 8.0, cancel.bottom);
            controls::button(p, reassign, "Reasignar", hot(hover, id, PART_REASSIGN), true);
            controls::button(p, cancel, "Cancelar", hot(hover, id, PART_CANCEL), true);
            hits.push((reassign, SettingsHit::Part(id, PART_REASSIGN)));
            hits.push((cancel, SettingsHit::Part(id, PART_CANCEL)));
        }
        RowState::Rejected(msg) => {
            let t = rect(r.left + CHIPS_X, line.bottom, r.right - 12.0, line.bottom + 22.0);
            p.text(FontId::Small, &format!("⚠ {msg}"), t, th.danger);
        }
        _ => {}
    }
}

/// ¿El punto está en el área del contenido? (para la rueda).
pub fn in_content(view: &SettingsView, x: f32, y: f32) -> bool {
    contains(&view.content, x, y)
}

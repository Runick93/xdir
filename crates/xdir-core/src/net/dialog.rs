//! El panel «Conectar a un servidor»: campo con la dirección y la estrella de favoritos, el
//! ejemplo (o el error, o «Conectando con…»), favoritos y recientes, y Cancelar / Conectar.
//! Dibujado por xdir encima de los paneles (como «Acerca de»); las cajas de texto son del host.

use crate::geom::{Color, Rect, contains, rect};
use crate::host::TextBox;
use crate::net::{Address, NetPlaces};
use crate::paint::{FontId, Painter, ellipsize};
use crate::sys::KeyStyle;
use crate::theme::alpha;

pub const TITLE: &str = "Conectar a un servidor";

#[derive(Default)]
pub struct ConnectDialog {
    /// La caja de la dirección (del host; se recrea si cambia de lugar).
    pub field: Option<TextBox>,
    /// Error en rojo debajo del campo.
    pub error: Option<String>,
    /// Conectando: la generación del intento (un resultado de otra se descarta).
    pub busy: Option<u64>,
    /// La dirección que se está conectando (para «Conectando con "nas"…»).
    pub busy_host: String,
    /// Fila elegida (favoritos primero, después recientes).
    pub selected: Option<usize>,
    /// Pidiendo el nombre de un favorito nuevo (caja en línea).
    pub naming: Option<TextBox>,
    /// Dónde están las cajas ahora (para recrearlas si el panel se mueve).
    pub boxes_at: Option<(Rect, Option<Rect>)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DialogHit {
    Field,
    Star,
    Favorite(usize),
    Recent(usize),
    RemoveRecent(usize),
    Cancel,
    Connect,
    Close,
    /// Dentro del panel, sobre nada.
    Panel,
}

const WIDTH: f32 = 520.0;
const HEADER: f32 = 44.0;
const ROW: f32 = 32.0;
const SECTION: f32 = 28.0;
const FOOTER: f32 = 58.0;
const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Dónde va cada cosa del panel.
pub struct Layout {
    pub panel: Rect,
    pub close: Rect,
    pub field: Rect,
    pub star: Rect,
    /// Línea del ejemplo / error / «Conectando…».
    pub note: Rect,
    /// Caja del nombre del favorito nuevo (si se está pidiendo).
    pub naming: Option<Rect>,
    pub favorites_title: Option<Rect>,
    pub favorites: Vec<Rect>,
    pub recents_title: Option<Rect>,
    pub recents: Vec<Rect>,
    pub cancel: Rect,
    pub connect: Rect,
}

pub fn layout(w: f32, h: f32, d: &ConnectDialog, places: &NetPlaces) -> Layout {
    let mut height = HEADER + 70.0 + FOOTER;
    if d.naming.is_some() {
        height += 32.0;
    }
    if !places.favorites.is_empty() {
        height += SECTION + ROW * places.favorites.len() as f32 + 6.0;
    }
    if !places.recents.is_empty() {
        height += SECTION + ROW * places.recents.len() as f32 + 6.0;
    }
    let left = ((w - WIDTH) / 2.0).max(8.0);
    let right = (left + WIDTH).min(w - 8.0);
    let top = ((h - height) / 3.0).max(40.0).min((h - height).max(8.0));
    let panel = rect(left, top, right, top + height);
    let close = rect(right - 36.0, top + 8.0, right - 8.0, top + 36.0);
    let field = rect(left + 16.0, top + HEADER + 14.0, right - 60.0, top + HEADER + 42.0);
    let star = rect(right - 52.0, field.top, right - 16.0, field.bottom);
    let note = rect(left + 16.0, field.bottom + 4.0, right - 16.0, field.bottom + 24.0);
    let mut y = note.bottom + 6.0;
    let naming = d.naming.map(|_| {
        let r = rect(left + 140.0, y, right - 60.0, y + 26.0);
        y += 32.0;
        r
    });
    let section = |n: usize, y: &mut f32| -> (Option<Rect>, Vec<Rect>) {
        if n == 0 {
            return (None, Vec::new());
        }
        let t = rect(left + 16.0, *y, right - 16.0, *y + SECTION);
        *y += SECTION;
        let rows = (0..n)
            .map(|i| rect(left + 16.0, *y + i as f32 * ROW, right - 16.0, *y + (i + 1) as f32 * ROW))
            .collect();
        *y += ROW * n as f32 + 6.0;
        (Some(t), rows)
    };
    let (favorites_title, favorites) = section(places.favorites.len(), &mut y);
    let (recents_title, recents) = section(places.recents.len(), &mut y);
    let by = panel.bottom - 44.0;
    let connect = rect(right - 128.0, by, right - 16.0, by + 32.0);
    let cancel = rect(connect.left - 108.0, by, connect.left - 8.0, by + 32.0);
    Layout { panel, close, field, star, note, naming, favorites_title, favorites, recents_title, recents, cancel, connect }
}

/// El ✕ de un reciente (a la derecha de su fila).
fn remove_rect(row: Rect) -> Rect {
    rect(row.right - 30.0, row.top + 4.0, row.right - 6.0, row.bottom - 4.0)
}

pub fn hit(l: &Layout, x: f32, y: f32) -> Option<DialogHit> {
    if !contains(&l.panel, x, y) {
        return None;
    }
    let found = if contains(&l.close, x, y) {
        DialogHit::Close
    } else if contains(&l.star, x, y) {
        DialogHit::Star
    } else if contains(&l.field, x, y) {
        DialogHit::Field
    } else if contains(&l.cancel, x, y) {
        DialogHit::Cancel
    } else if contains(&l.connect, x, y) {
        DialogHit::Connect
    } else if let Some(i) = l.favorites.iter().position(|r| contains(r, x, y)) {
        DialogHit::Favorite(i)
    } else if let Some(i) = l.recents.iter().position(|r| contains(r, x, y)) {
        if contains(&remove_rect(l.recents[i]), x, y) { DialogHit::RemoveRecent(i) } else { DialogHit::Recent(i) }
    } else {
        DialogHit::Panel
    };
    Some(found)
}

/// El ejemplo de direcciones de este sistema: con UNC donde no hay otro esquema que `smb`
/// (Windows); si no, la URL y lo que suma el sistema (`afp://` en macOS, `sftp://` en Linux).
pub fn example(extra: Option<&str>) -> String {
    match extra {
        None => "Por ejemplo: \\\\servidor\\recurso o smb://servidor/recurso".to_string(),
        Some(e) => format!("Por ejemplo: smb://servidor/recurso, {e}://…"),
    }
}

/// Una dirección guardada tal como se muestra en este sistema.
pub fn shown(address: &str, style: KeyStyle) -> String {
    Address::parse(address).map(|a| a.display(style)).unwrap_or_else(|_| address.to_string())
}

pub struct DrawArgs<'a> {
    pub style: KeyStyle,
    pub example: &'a str,
    /// Hace cuánto de cada reciente («hace 2 días»).
    pub recent_dates: &'a [String],
    /// ¿La dirección del campo ya es favorita? (estrella llena).
    pub is_favorite: bool,
    pub hover: Option<DialogHit>,
}

pub fn draw(p: &dyn Painter, w: f32, h: f32, d: &ConnectDialog, places: &NetPlaces, a: &DrawArgs) {
    let th = p.theme();
    let l = layout(w, h, d, places);
    let hot = |x: DialogHit| a.hover == Some(x);
    p.fill(rect(0.0, 0.0, w, h), Color { r: 0.0, g: 0.0, b: 0.0, a: 0.35 });
    let pn = l.panel;
    p.round_fill(rect(pn.left - 2.0, pn.top, pn.right + 2.0, pn.bottom + 4.0), 12.0, alpha(th.background, 0.5));
    p.round_fill(pn, 10.0, th.surface_alt);
    p.round_stroke(pn, 10.0, th.border);
    p.text(FontId::UiBold, TITLE, rect(pn.left + 16.0, pn.top + 12.0, pn.right - 48.0, pn.top + 32.0), th.text);
    if hot(DialogHit::Close) {
        p.round_fill(l.close, 6.0, th.hover);
    }
    p.text(FontId::GlyphsSmall, "\u{E711}", rect(l.close.left, l.close.top + 8.0, l.close.right, l.close.bottom - 8.0), th.text_dim);
    p.fill(rect(pn.left, pn.top + HEADER, pn.right, pn.top + HEADER + 1.0), th.border);

    // Campo (la caja del host va adentro) y estrella.
    let field_border = if d.error.is_some() { th.danger } else { th.accent };
    p.round_fill(l.field, 6.0, th.surface);
    p.round_stroke(l.field, 6.0, field_border);
    p.round_fill(l.star, 6.0, if hot(DialogHit::Star) { th.hover } else { th.surface });
    p.round_stroke(l.star, 6.0, th.border);
    let (star, star_color) = if a.is_favorite { ("\u{E735}", th.accent) } else { ("\u{E734}", th.text_dim) };
    p.text(FontId::Glyphs, star, rect(l.star.left, l.star.top + 6.0, l.star.right, l.star.bottom - 6.0), star_color);

    let n = l.note;
    if d.busy.is_some() {
        p.circle_stroke(n.left + 6.0, n.top + 10.0, 5.0, th.accent, 1.5);
        p.text(FontId::Small, &format!("Conectando con «{}»…", d.busy_host), rect(n.left + 16.0, n.top, n.right, n.bottom), th.text_dim);
    } else if let Some(e) = &d.error {
        p.circle(n.left + 6.0, n.top + 10.0, 6.0, th.danger);
        p.text(FontId::Small, "!", rect(n.left + 4.0, n.top + 3.0, n.left + 9.0, n.top + 17.0), WHITE);
        p.text(FontId::Small, e, rect(n.left + 16.0, n.top, n.right, n.bottom), th.danger);
    } else {
        p.text(FontId::Small, a.example, n, th.text_dim);
    }
    if let Some(r) = l.naming {
        p.text(FontId::Ui, "Nombre del favorito:", rect(pn.left + 16.0, r.top + 4.0, r.left - 6.0, r.bottom), th.text_secondary);
        p.round_fill(r, 6.0, th.surface);
        p.round_stroke(r, 6.0, th.accent);
    }

    let row_bg = |r: Rect, sel: bool, hov: bool| {
        if sel {
            p.round_fill(r, 6.0, th.selection);
        } else if hov {
            p.round_fill(r, 6.0, th.hover);
        }
    };
    if let Some(t) = l.favorites_title {
        p.text(FontId::UiBold, "Favoritos", rect(t.left, t.top + 6.0, t.right, t.bottom), th.text_secondary);
    }
    for (i, (r, f)) in l.favorites.iter().zip(&places.favorites).enumerate() {
        row_bg(*r, d.selected == Some(i), hot(DialogHit::Favorite(i)));
        p.text(FontId::Glyphs, "\u{E735}", rect(r.left + 4.0, r.top + 8.0, r.left + 28.0, r.bottom - 8.0), th.accent);
        let mid = r.left + 190.0;
        p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &f.name, mid - r.left - 44.0), rect(r.left + 34.0, r.top + 7.0, mid - 8.0, r.bottom), th.text);
        p.text(FontId::Ui, &shown(&f.address, a.style), rect(mid, r.top + 7.0, r.right - 8.0, r.bottom), th.text_dim);
    }
    if let Some(t) = l.recents_title {
        p.text(FontId::UiBold, "Recientes", rect(t.left, t.top + 6.0, t.right, t.bottom), th.text_secondary);
    }
    let nf = places.favorites.len();
    for (i, (r, rc)) in l.recents.iter().zip(&places.recents).enumerate() {
        let hov = hot(DialogHit::Recent(i)) || hot(DialogHit::RemoveRecent(i));
        row_bg(*r, d.selected == Some(nf + i), hov);
        p.circle_stroke(r.left + 16.0, r.top + ROW / 2.0, 6.0, th.text_dim, 1.2);
        p.fill(rect(r.left + 15.5, r.top + 12.0, r.left + 16.5, r.top + 16.5), th.text_dim);
        p.fill(rect(r.left + 16.0, r.top + 15.5, r.left + 19.0, r.top + 16.5), th.text_dim);
        let date = a.recent_dates.get(i).map(String::as_str).unwrap_or("");
        let date_right = if hov { r.right - 36.0 } else { r.right - 12.0 };
        p.text(FontId::Ui, &shown(&rc.address, a.style), rect(r.left + 34.0, r.top + 7.0, date_right - 110.0, r.bottom), th.text);
        p.text(FontId::UiRight, date, rect(date_right - 140.0, r.top + 8.0, date_right, r.bottom), th.text_dim);
        if hov {
            let x = remove_rect(*r);
            if hot(DialogHit::RemoveRecent(i)) {
                p.round_fill(x, 4.0, th.hover);
            }
            p.text(FontId::GlyphsSmall, "\u{E711}", rect(x.left, x.top + 6.0, x.right, x.bottom - 6.0), th.text);
        }
    }

    // Pie.
    p.fill(rect(pn.left, pn.bottom - FOOTER, pn.right, pn.bottom - FOOTER + 1.0), th.border);
    p.round_fill(l.cancel, 6.0, if hot(DialogHit::Cancel) { th.hover } else { th.surface });
    p.round_stroke(l.cancel, 6.0, th.border);
    p.text(FontId::UiCenter, "Cancelar", rect(l.cancel.left, l.cancel.top + 8.0, l.cancel.right, l.cancel.bottom - 6.0), th.text);
    let busy = d.busy.is_some();
    let fill = if busy { alpha(th.accent, 0.6) } else if hot(DialogHit::Connect) { alpha(th.accent, 0.85) } else { th.accent };
    p.round_fill(l.connect, 6.0, fill);
    let label = if busy { "Conectando…" } else { "Conectar" };
    p.text(FontId::UiCenter, label, rect(l.connect.left, l.connect.top + 8.0, l.connect.right, l.connect.bottom - 6.0), WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn places() -> NetPlaces {
        let mut p = NetPlaces::default();
        p.toggle_favorite(&Address::parse("smb://nas.local/fotos").unwrap(), Some("NAS — fotos".into()));
        p.add_recent(&Address::parse(r"\\srv01\respaldo").unwrap(), 1);
        p
    }

    #[test]
    fn dialog_layout_and_hits() {
        let (d, pl) = (ConnectDialog::default(), places());
        let l = layout(1000.0, 800.0, &d, &pl);
        let c = |r: Rect| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let at = |r: Rect| {
            let (x, y) = c(r);
            hit(&l, x, y)
        };
        assert_eq!(at(l.field), Some(DialogHit::Field));
        assert_eq!(at(l.star), Some(DialogHit::Star));
        assert_eq!(at(l.favorites[0]), Some(DialogHit::Favorite(0)));
        assert_eq!(at(l.recents[0]), Some(DialogHit::Recent(0)));
        assert_eq!(at(remove_rect(l.recents[0])), Some(DialogHit::RemoveRecent(0)));
        assert_eq!(at(l.connect), Some(DialogHit::Connect));
        assert_eq!(at(l.cancel), Some(DialogHit::Cancel));
        assert_eq!(at(l.close), Some(DialogHit::Close));
        assert_eq!(hit(&l, 2.0, 2.0), None);
        assert!(l.panel.bottom <= 800.0 && l.cancel.bottom < l.panel.bottom);
    }

    #[test]
    fn dialog_draws_sections_and_states() {
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)));
        let mut d = ConnectDialog::default();
        let ex = example(None);
        let dates = vec!["hace 2 días".to_string()];
        let args = DrawArgs { style: KeyStyle::Windows, example: &ex, recent_dates: &dates, is_favorite: false, hover: None };
        draw(&p, 1000.0, 800.0, &d, &places(), &args);
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        for t in [TITLE, "Favoritos", "Recientes", "NAS — fotos", r"\\nas.local\fotos", r"\\srv01\respaldo", "hace 2 días", "Cancelar", "Conectar", ex.as_str()] {
            assert!(texts.iter().any(|x| x == t), "{t}: {texts:?}");
        }
        d.busy = Some(1);
        d.busy_host = "srv01".into();
        let p = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)));
        draw(&p, 1000.0, 800.0, &d, &places(), &args);
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        assert!(texts.iter().any(|x| x == "Conectando con «srv01»…") && texts.iter().any(|x| x == "Conectando…"));
    }

    #[test]
    fn examples_per_system() {
        assert_eq!(example(None), "Por ejemplo: \\\\servidor\\recurso o smb://servidor/recurso");
        assert_eq!(example(Some("afp")), "Por ejemplo: smb://servidor/recurso, afp://…");
        assert_eq!(example(Some("sftp")), "Por ejemplo: smb://servidor/recurso, sftp://…");
    }
}

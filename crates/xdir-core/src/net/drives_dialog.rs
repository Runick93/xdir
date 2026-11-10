//! Los paneles de las unidades de red de Windows: «Conectar unidad de red» (letra, carpeta con
//! autocompletar de favoritos y recientes, «Examinar…», «Conectar de nuevo al iniciar sesión» y
//! «Conectar con otras credenciales») y «Desconectar unidad de red» (las unidades con casillas).
//! Dibujados por xdir encima de los paneles, como «Conectar a un servidor».

use crate::geom::{Color, Rect, contains, rect};
use crate::host::TextBox;
use crate::net::{Address, NetConnection, NetPlaces};
use crate::paint::{FontId, Painter};
use crate::sys::KeyStyle;
use crate::theme::alpha;

pub const MAP_TITLE: &str = "Conectar unidad de red";
pub const UNMAP_TITLE: &str = "Desconectar unidad de red";
pub const NO_DRIVES: &str = "No hay unidades de red";

pub struct MapDialog {
    /// Letras libres (de Z hacia abajo) y la elegida.
    pub letters: Vec<char>,
    pub letter: char,
    pub folder: Option<TextBox>,
    /// «Conectar de nuevo al iniciar sesión» (prendida al abrir, como el Explorador).
    pub reconnect: bool,
    pub other_credentials: bool,
    pub error: Option<String>,
    pub busy: Option<u64>,
    /// Lo que completa lo escrito (de favoritos y recientes).
    pub suggestion: Option<String>,
    pub boxes_at: Option<Rect>,
}

impl MapDialog {
    pub fn new(letters: Vec<char>) -> MapDialog {
        let letter = letters.first().copied().unwrap_or('Z');
        MapDialog { letters, letter, folder: None, reconnect: true, other_credentials: false, error: None, busy: None, suggestion: None, boxes_at: None }
    }
}

pub struct DisconnectDialog {
    /// Las unidades de red (con letra) y si están marcadas.
    pub rows: Vec<(NetConnection, bool)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrivesHit {
    Letter,
    Folder,
    Browse,
    Suggestion,
    Reconnect,
    OtherCredentials,
    Row(usize),
    Cancel,
    Finish,
    Close,
    Panel,
}

const WIDTH: f32 = 520.0;
const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Lo que completa `typed` entre favoritos y recientes (por prefijo, sin mayúsculas), en la
/// forma en que se escribe (`\\srv\rec` o `smb://srv/rec`; si no se sabe, la de este sistema);
/// `None` si no hay o ya está completo.
pub fn suggest(typed: &str, places: &NetPlaces, style: KeyStyle) -> Option<String> {
    let t = typed.trim().to_lowercase();
    if t.len() < 2 {
        return None;
    }
    let unc = t.starts_with('\\') || t.starts_with("//");
    let url = t.starts_with("smb:");
    places
        .favorites
        .iter()
        .map(|f| f.address.as_str())
        .chain(places.recents.iter().map(|r| r.address.as_str()))
        .filter_map(|a| Address::parse(a).ok().filter(|a| a.scheme == "smb"))
        .map(|a| if unc { a.to_unc().unwrap_or_default() } else if url { a.to_url() } else { a.display(style) })
        .find(|s| s.to_lowercase().starts_with(&t) && s.to_lowercase() != t)
}

pub struct MapLayout {
    pub panel: Rect,
    pub close: Rect,
    pub letter: Rect,
    pub folder: Rect,
    pub browse: Rect,
    pub note: Rect,
    pub reconnect: Rect,
    pub other: Rect,
    pub cancel: Rect,
    pub finish: Rect,
}

fn panel_at(w: f32, h: f32, height: f32) -> Rect {
    let left = ((w - WIDTH) / 2.0).max(8.0);
    let top = ((h - height) / 3.0).max(40.0).min((h - height).max(8.0));
    rect(left, top, (left + WIDTH).min(w - 8.0), top + height)
}

pub fn map_layout(w: f32, h: f32) -> MapLayout {
    let p = panel_at(w, h, 300.0);
    let (l, r, t) = (p.left, p.right, p.top);
    let finish = rect(r - 112.0, p.bottom - 44.0, r - 16.0, p.bottom - 12.0);
    MapLayout {
        panel: p,
        close: rect(r - 36.0, t + 8.0, r - 8.0, t + 36.0),
        letter: rect(l + 96.0, t + 92.0, l + 184.0, t + 120.0),
        folder: rect(l + 96.0, t + 132.0, r - 116.0, t + 160.0),
        browse: rect(r - 108.0, t + 132.0, r - 16.0, t + 160.0),
        note: rect(l + 96.0, t + 162.0, r - 16.0, t + 180.0),
        reconnect: rect(l + 96.0, t + 186.0, r - 16.0, t + 208.0),
        other: rect(l + 96.0, t + 212.0, r - 16.0, t + 234.0),
        cancel: rect(finish.left - 108.0, finish.top, finish.left - 8.0, finish.bottom),
        finish,
    }
}

pub fn map_hit(l: &MapLayout, d: &MapDialog, x: f32, y: f32) -> Option<DrivesHit> {
    if !contains(&l.panel, x, y) {
        return None;
    }
    let pairs = [
        (l.close, DrivesHit::Close),
        (l.letter, DrivesHit::Letter),
        (l.browse, DrivesHit::Browse),
        (l.folder, DrivesHit::Folder),
        (l.reconnect, DrivesHit::Reconnect),
        (l.other, DrivesHit::OtherCredentials),
        (l.cancel, DrivesHit::Cancel),
        (l.finish, DrivesHit::Finish),
    ];
    if d.suggestion.is_some() && d.error.is_none() && contains(&l.note, x, y) {
        return Some(DrivesHit::Suggestion);
    }
    Some(pairs.iter().find(|(r, _)| contains(r, x, y)).map_or(DrivesHit::Panel, |(_, h)| *h))
}

fn frame(p: &dyn Painter, w: f32, h: f32, pn: Rect, title: &str, close: Rect, hot_close: bool) {
    let th = p.theme();
    p.fill(rect(0.0, 0.0, w, h), Color { r: 0.0, g: 0.0, b: 0.0, a: 0.35 });
    p.round_fill(rect(pn.left - 2.0, pn.top, pn.right + 2.0, pn.bottom + 4.0), 12.0, alpha(th.background, 0.5));
    p.round_fill(pn, 10.0, th.surface_alt);
    p.round_stroke(pn, 10.0, th.border);
    p.text(FontId::UiBold, title, rect(pn.left + 16.0, pn.top + 12.0, pn.right - 48.0, pn.top + 32.0), th.text);
    if hot_close {
        p.round_fill(close, 6.0, th.hover);
    }
    p.text(FontId::GlyphsSmall, "\u{E711}", rect(close.left, close.top + 8.0, close.right, close.bottom - 8.0), th.text_dim);
    p.fill(rect(pn.left, pn.top + 44.0, pn.right, pn.top + 45.0), th.border);
    p.fill(rect(pn.left, pn.bottom - 58.0, pn.right, pn.bottom - 57.0), th.border);
}

fn checkbox(p: &dyn Painter, r: Rect, on: bool, label: &str) {
    let th = p.theme();
    let b = rect(r.left, r.top + 3.0, r.left + 16.0, r.top + 19.0);
    if on {
        p.round_fill(b, 3.0, th.accent);
        p.text(FontId::GlyphsSmall, "\u{E73E}", rect(b.left, b.top - 1.0, b.right, b.bottom), WHITE);
    } else {
        p.round_stroke_w(b, 3.0, th.text_dim, 1.2);
    }
    p.text(FontId::Ui, label, rect(r.left + 26.0, r.top + 2.0, r.right, r.bottom), th.text);
}

fn buttons(p: &dyn Painter, cancel: Rect, ok: Rect, ok_label: &str, hot_cancel: bool, hot_ok: bool, enabled: bool) {
    let th = p.theme();
    p.round_fill(cancel, 6.0, if hot_cancel { th.hover } else { th.surface });
    p.round_stroke(cancel, 6.0, th.border);
    p.text(FontId::UiCenter, "Cancelar", rect(cancel.left, cancel.top + 8.0, cancel.right, cancel.bottom - 6.0), th.text);
    let fill = if !enabled { alpha(th.accent, 0.4) } else if hot_ok { alpha(th.accent, 0.85) } else { th.accent };
    p.round_fill(ok, 6.0, fill);
    p.text(FontId::UiCenter, ok_label, rect(ok.left, ok.top + 8.0, ok.right, ok.bottom - 6.0), WHITE);
}

pub fn map_draw(p: &dyn Painter, w: f32, h: f32, d: &MapDialog, hover: Option<DrivesHit>) {
    let th = p.theme();
    let l = map_layout(w, h);
    let hot = |x| hover == Some(x);
    frame(p, w, h, l.panel, MAP_TITLE, l.close, hot(DrivesHit::Close));
    let left = l.panel.left + 16.0;
    p.text(FontId::Ui, "Elegí la letra de la unidad y la carpeta a la que te querés conectar.", rect(left, l.panel.top + 58.0, l.panel.right - 16.0, l.panel.top + 80.0), th.text_secondary);
    p.text(FontId::Ui, "Unidad:", rect(left, l.letter.top + 6.0, l.letter.left - 8.0, l.letter.bottom), th.text);
    p.round_fill(l.letter, 6.0, if hot(DrivesHit::Letter) { th.hover } else { th.surface });
    p.round_stroke(l.letter, 6.0, th.border);
    p.text(FontId::Ui, &format!("{}:", d.letter), rect(l.letter.left + 10.0, l.letter.top + 6.0, l.letter.right - 20.0, l.letter.bottom), th.text);
    p.text(FontId::GlyphsSmall, "\u{E70D}", rect(l.letter.right - 22.0, l.letter.top + 9.0, l.letter.right - 6.0, l.letter.bottom - 9.0), th.text_dim);
    p.text(FontId::Ui, "Carpeta:", rect(left, l.folder.top + 6.0, l.folder.left - 8.0, l.folder.bottom), th.text);
    p.round_fill(l.folder, 6.0, th.surface);
    p.round_stroke(l.folder, 6.0, if d.error.is_some() { th.danger } else { th.accent });
    p.round_fill(l.browse, 6.0, if hot(DrivesHit::Browse) { th.hover } else { th.surface });
    p.round_stroke(l.browse, 6.0, th.border);
    p.text(FontId::UiCenter, "Examinar…", rect(l.browse.left, l.browse.top + 6.0, l.browse.right, l.browse.bottom - 4.0), th.text);
    match (&d.error, &d.suggestion, d.busy) {
        (_, _, Some(_)) => p.text(FontId::Small, "Conectando…", l.note, th.text_dim),
        (Some(e), _, _) => p.text(FontId::Small, e, l.note, th.danger),
        (None, Some(s), _) => p.text(FontId::Small, &format!("{s} (clic para completar)"), l.note, if hot(DrivesHit::Suggestion) { th.text } else { th.accent }),
        _ => p.text(FontId::Small, "Por ejemplo: \\\\servidor\\recurso", l.note, th.text_dim),
    }
    checkbox(p, l.reconnect, d.reconnect, "Conectar de nuevo al iniciar sesión");
    checkbox(p, l.other, d.other_credentials, "Conectar con otras credenciales");
    buttons(p, l.cancel, l.finish, if d.busy.is_some() { "Conectando…" } else { "Finalizar" }, hot(DrivesHit::Cancel), hot(DrivesHit::Finish), d.busy.is_none());
}

pub struct UnmapLayout {
    pub panel: Rect,
    pub close: Rect,
    pub rows: Vec<Rect>,
    pub cancel: Rect,
    pub finish: Rect,
}

pub fn unmap_layout(w: f32, h: f32, d: &DisconnectDialog) -> UnmapLayout {
    let n = d.rows.len().max(1) as f32;
    let p = panel_at(w, h, 44.0 + 48.0 + n * 32.0 + 16.0 + 58.0);
    let rows = (0..d.rows.len()).map(|i| rect(p.left + 24.0, p.top + 96.0 + i as f32 * 32.0, p.right - 24.0, p.top + 128.0 + i as f32 * 32.0)).collect();
    let finish = rect(p.right - 128.0, p.bottom - 44.0, p.right - 16.0, p.bottom - 12.0);
    UnmapLayout { close: rect(p.right - 36.0, p.top + 8.0, p.right - 8.0, p.top + 36.0), rows, cancel: rect(finish.left - 108.0, finish.top, finish.left - 8.0, finish.bottom), finish, panel: p }
}

pub fn unmap_hit(l: &UnmapLayout, x: f32, y: f32) -> Option<DrivesHit> {
    if !contains(&l.panel, x, y) {
        return None;
    }
    if contains(&l.close, x, y) {
        return Some(DrivesHit::Close);
    }
    if contains(&l.cancel, x, y) {
        return Some(DrivesHit::Cancel);
    }
    if contains(&l.finish, x, y) {
        return Some(DrivesHit::Finish);
    }
    Some(l.rows.iter().position(|r| contains(r, x, y)).map_or(DrivesHit::Panel, DrivesHit::Row))
}

pub fn unmap_draw(p: &dyn Painter, w: f32, h: f32, d: &DisconnectDialog, style: KeyStyle, hover: Option<DrivesHit>) {
    let th = p.theme();
    let l = unmap_layout(w, h, d);
    let hot = |x| hover == Some(x);
    frame(p, w, h, l.panel, UNMAP_TITLE, l.close, hot(DrivesHit::Close));
    let left = l.panel.left + 16.0;
    if d.rows.is_empty() {
        p.text(FontId::Ui, NO_DRIVES, rect(left, l.panel.top + 58.0, l.panel.right - 16.0, l.panel.top + 80.0), th.text_secondary);
    } else {
        p.text(FontId::Ui, "Elegí las unidades que querés desconectar.", rect(left, l.panel.top + 58.0, l.panel.right - 16.0, l.panel.top + 80.0), th.text_secondary);
        let list = rect(l.panel.left + 16.0, l.panel.top + 90.0, l.panel.right - 16.0, l.panel.top + 102.0 + d.rows.len() as f32 * 32.0);
        p.round_fill(list, 6.0, th.surface);
        p.round_stroke(list, 6.0, th.border);
    }
    for (i, (r, (c, on))) in l.rows.iter().zip(&d.rows).enumerate() {
        if hot(DrivesHit::Row(i)) {
            p.round_fill(*r, 4.0, th.hover);
        }
        checkbox(p, rect(r.left + 4.0, r.top + 5.0, r.left + 30.0, r.bottom), *on, "");
        p.text(FontId::Glyphs, "\u{E968}", rect(r.left + 30.0, r.top + 6.0, r.left + 54.0, r.bottom - 4.0), th.text_dim);
        p.text(FontId::UiBold, &c.letter.map(|l| format!("{l}:")).unwrap_or_default(), rect(r.left + 60.0, r.top + 8.0, r.left + 90.0, r.bottom), th.text);
        let shown = Address::parse(&c.address).map(|a| a.display(style)).unwrap_or_else(|_| c.address.clone());
        p.text(FontId::Ui, &shown, rect(r.left + 92.0, r.top + 8.0, r.right - 110.0, r.bottom), th.text);
        if !c.connected {
            p.text(FontId::UiRight, "desconectada", rect(r.right - 120.0, r.top + 8.0, r.right - 8.0, r.bottom), th.text_dim);
        }
    }
    let any = d.rows.iter().any(|(_, on)| *on);
    buttons(p, l.cancel, l.finish, "Desconectar", hot(DrivesHit::Cancel), hot(DrivesHit::Finish), any);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    #[test]
    fn map_defaults_and_hits() {
        let d = MapDialog::new(vec!['Z', 'Y']);
        assert_eq!((d.letter, d.reconnect, d.other_credentials), ('Z', true, false));
        let l = map_layout(1000.0, 800.0);
        let c = |r: Rect| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        for (r, h) in [(l.letter, DrivesHit::Letter), (l.browse, DrivesHit::Browse), (l.finish, DrivesHit::Finish), (l.reconnect, DrivesHit::Reconnect)] {
            let (x, y) = c(r);
            assert_eq!(map_hit(&l, &d, x, y), Some(h));
        }
        assert_eq!(map_hit(&l, &d, 1.0, 1.0), None);
    }

    #[test]
    fn suggest_by_prefix() {
        let mut p = NetPlaces::default();
        p.toggle_favorite(&Address::parse(r"\\srv01\compartido").unwrap(), None);
        p.add_recent(&Address::parse(r"\\nas\fotos").unwrap(), 1);
        assert_eq!(suggest(r"\\srv", &p, KeyStyle::Windows).as_deref(), Some(r"\\srv01\compartido"));
        assert_eq!(suggest(r"\\NA", &p, KeyStyle::Windows).as_deref(), Some(r"\\nas\fotos"));
        assert_eq!(suggest(r"\\nas\fotos", &p, KeyStyle::Windows), None);
        assert_eq!(suggest(r"\\otro", &p, KeyStyle::Windows), None);
    }

    #[test]
    fn unmap_draws_rows_and_empty() {
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(false)));
        let rows = vec![
            (NetConnection { address: "smb://srv01/compartido".into(), local: "Z:\\".into(), letter: Some('Z'), reconnect: true, connected: true }, true),
            (NetConnection { address: "smb://srv01/respaldo".into(), local: "X:\\".into(), letter: Some('X'), reconnect: true, connected: false }, false),
        ];
        unmap_draw(&p, 1000.0, 800.0, &DisconnectDialog { rows }, KeyStyle::Windows, None);
        let t: Vec<String> = p.texts().into_iter().map(|x| x.1).collect();
        for s in [UNMAP_TITLE, "Z:", r"\\srv01\compartido", "X:", "desconectada", "Desconectar"] {
            assert!(t.iter().any(|x| x == s), "{s}: {t:?}");
        }
        let p = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(false)));
        unmap_draw(&p, 1000.0, 800.0, &DisconnectDialog { rows: Vec::new() }, KeyStyle::Windows, None);
        assert!(p.texts().iter().any(|x| x.1 == NO_DRIVES));
    }
}

//! Sección «Red» de la configuración: favoritos (nombre y dirección editables en línea, ↑/↓, ✕,
//! «Agregar…»), recientes (★ pasa a favoritos, ✕, «Borrar los recientes»), lo conectado ahora con
//! ⏏, el aviso de que xdir no guarda contraseñas y, en macOS sin permiso de «Red local», cómo
//! darlo. Cada cambio guarda `red.toml` en un hilo.
//!
//! Las cuatro funciones tienen la firma de las páginas extra de la configuración (`ExtraPage`, de
//! la corriente S); se enchufan en `settings_pages::PAGES` al juntar las dos.

// Hasta que la pestaña de configuración (corriente S) enchufe la página, solo la usan las pruebas.
#![cfg_attr(not(test), allow(dead_code))]

use crate::geom::{Rect, contains, rect};
use crate::host::{TextBox, TextKind};
use crate::net::Address;
use crate::net::dialog::shown;
use crate::paint::{FontId, Painter, ellipsize};
use crate::sys::{self, KeyStyle};
use crate::theme::alpha;

use super::App;
use super::network::unix_now;

/// Un resultado del buscador de la configuración (el `SearchItem` de S; F1 lo reemplaza).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchItem {
    pub title: String,
    pub desc: String,
    pub keywords: Vec<String>,
}

/// Lo que se puede tocar en la página.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PageHit {
    AddFavorite,
    EditFavorite(usize),
    FavoriteUp(usize),
    FavoriteDown(usize),
    RemoveFavorite(usize),
    ClearRecents,
    StarRecent(usize),
    RemoveRecent(usize),
    Eject(usize),
    OpenCredentials,
    OpenLocalNetwork,
}

/// Estado de la página (dentro de `NetState`).
#[derive(Default)]
pub(super) struct PageState {
    /// Favorito en edición: (índice; `favorites.len()` = uno nuevo, caja del nombre, caja de la
    /// dirección, dónde están).
    pub editing: Option<(usize, TextBox, TextBox, (Rect, Rect))>,
    /// Error de la dirección de una fila (índice, texto).
    pub error: Option<(usize, String)>,
    /// Dónde quedó cada control en el último cuadro.
    pub hits: Vec<(Rect, PageHit)>,
    /// Ya se pidió lo conectado al abrir la página.
    pub asked_connections: bool,
}

const ROW: f32 = 40.0;
const GAP: f32 = 4.0;
const WIDTH: f32 = 820.0;

/// Dónde guarda las contraseñas este sistema.
fn credentials_place(style: KeyStyle, has_place: bool) -> &'static str {
    match (style, has_place) {
        (KeyStyle::Mac, _) => "Llavero",
        (KeyStyle::Windows, true) => "Administrador de credenciales",
        (KeyStyle::Windows, false) => "llavero del escritorio",
    }
}

/// «xdir no guarda contraseñas: las guarda el sistema (Llavero)».
pub(super) fn password_text(style: KeyStyle, has_place: bool) -> String {
    format!("xdir no guarda contraseñas: las guarda el sistema ({})", credentials_place(style, has_place))
}

pub(super) const LOCAL_NETWORK_HINT: &str = "Para ver las máquinas de la red local, permití \"Red local\" en Ajustes del Sistema → Privacidad";

fn item(title: &str, desc: &str, keywords: &[&str]) -> SearchItem {
    SearchItem { title: title.into(), desc: desc.into(), keywords: keywords.iter().map(|k| k.to_string()).collect() }
}

/// Lo que encuentra el buscador en esta página.
pub(super) fn search(_app: &App) -> Vec<SearchItem> {
    const COMMON: [&str; 4] = ["red", "servidor", "smb", "nas"];
    fn with(extra: &[&'static str]) -> Vec<&'static str> {
        COMMON.iter().chain(extra).copied().collect()
    }
    vec![
        item("Favoritos", "Servidores guardados para conectarse con un clic", &with(&["favorito", "estrella"])),
        item("Recientes", "Las últimas direcciones a las que te conectaste", &with(&["reciente", "historial"])),
        item("Conectados ahora", "Lo conectado ahora, para desconectar", &with(&["conectado", "desconectar", "expulsar"])),
        item("Contraseñas", "xdir no guarda contraseñas: las guarda el sistema", &with(&["contraseña", "llavero", "credenciales"])),
    ]
}

fn button(p: &dyn Painter, r: Rect, label: &str, hot: bool) {
    let th = p.theme();
    p.round_fill(r, 6.0, if hot { th.hover } else { th.surface_alt });
    p.round_stroke(r, 6.0, th.border);
    p.text(FontId::UiCenter, label, rect(r.left, r.top + 5.0, r.right, r.bottom - 3.0), th.text);
}

fn icon_button(p: &dyn Painter, r: Rect, glyph: &str, hot: bool) {
    let th = p.theme();
    if hot {
        p.round_fill(r, 4.0, alpha(th.text, 0.12));
    }
    p.text(FontId::GlyphsSmall, glyph, rect(r.left, r.top + 6.0, r.right, r.bottom - 6.0), if hot { th.text } else { th.text_dim });
}

/// Dibuja la página en `r` (arriba a la izquierda del contenido) y devuelve el alto que ocupa.
pub(super) fn draw(app: &mut App, p: &dyn Painter, r: Rect, hover: Option<(f32, f32)>) -> f32 {
    if !app.net.page.asked_connections {
        app.net.page.asked_connections = true;
        app.request_connections();
    }
    let th = p.theme();
    let style = app.net_style();
    let net = sys::network();
    let left = r.left;
    let right = (r.left + WIDTH).min(r.right);
    let mut hits: Vec<(Rect, PageHit)> = Vec::new();
    let hot = |b: Rect| hover.is_some_and(|(x, y)| contains(&b, x, y));
    let mut y = r.top;
    p.text(FontId::Title, "Red", rect(left, y, right, y + 26.0), th.text);
    y += 40.0;

    // Favoritos.
    p.text(FontId::UiBold, "Favoritos", rect(left, y + 6.0, right - 120.0, y + 26.0), th.text_secondary);
    let add = rect(right - 96.0, y, right, y + 26.0);
    button(p, add, "Agregar…", hot(add));
    hits.push((add, PageHit::AddFavorite));
    y += 32.0;
    let nf = app.net.places.favorites.len();
    let editing = app.net.page.editing.as_ref().map(|(i, ..)| *i);
    let rows = nf + usize::from(editing == Some(nf));
    let mut edit_rects = None;
    for i in 0..rows {
        let row = rect(left, y, right, y + ROW);
        let is_edit = editing == Some(i);
        p.round_fill(row, 6.0, th.surface_alt);
        p.round_stroke(row, 6.0, if is_edit { th.accent } else { th.border });
        p.text(FontId::Glyphs, "\u{E735}", rect(left + 8.0, y + 11.0, left + 32.0, y + ROW - 9.0), th.accent);
        let name_r = rect(left + 36.0, y + 6.0, left + 244.0, y + ROW - 6.0);
        let addr_r = rect(left + 252.0, y + 6.0, right - 208.0, y + ROW - 6.0);
        if is_edit {
            for b in [name_r, addr_r] {
                p.round_fill(b, 4.0, th.surface);
                p.round_stroke(b, 4.0, th.border);
            }
            edit_rects = Some((name_r, addr_r));
        } else if let Some(f) = app.net.places.favorites.get(i) {
            p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &f.name, name_r.right - name_r.left), rect(name_r.left, y + 12.0, name_r.right, y + ROW), th.text);
            p.text(FontId::Ui, &shown(&f.address, style), rect(addr_r.left, y + 12.0, addr_r.right, y + ROW), th.text_dim);
        }
        let mut x = right - 36.0;
        let mut slot = |glyph: &str, hit: PageHit, hits: &mut Vec<(Rect, PageHit)>| {
            let b = rect(x, y + 8.0, x + 26.0, y + ROW - 8.0);
            icon_button(p, b, glyph, hot(b));
            hits.push((b, hit));
            x -= 28.0;
        };
        if i < nf {
            slot("\u{E711}", PageHit::RemoveFavorite(i), &mut hits);
            slot("\u{E70D}", PageHit::FavoriteDown(i), &mut hits);
            slot("\u{E70E}", PageHit::FavoriteUp(i), &mut hits);
            if !is_edit {
                slot("\u{E70F}", PageHit::EditFavorite(i), &mut hits);
            }
        }
        y += ROW + GAP;
        if let Some((_, e)) = app.net.page.error.as_ref().filter(|(j, _)| *j == i) {
            p.text(FontId::Small, e, rect(left + 252.0, y - 2.0, right, y + 14.0), th.danger);
            y += 16.0;
        }
    }
    if rows == 0 {
        p.text(FontId::Small, "Todavía no hay favoritos: la estrella de «Conectar a un servidor» los agrega.", rect(left, y, right, y + 18.0), th.text_dim);
        y += 22.0;
    }
    y += 14.0;

    // Recientes.
    p.text(FontId::UiBold, "Recientes", rect(left, y + 6.0, right - 160.0, y + 26.0), th.text_secondary);
    if !app.net.places.recents.is_empty() {
        let clear = rect(right - 150.0, y, right, y + 26.0);
        button(p, clear, "Borrar los recientes", hot(clear));
        hits.push((clear, PageHit::ClearRecents));
    }
    y += 32.0;
    let (now, offset) = (unix_now(), sys::env().local_now().offset);
    for (i, rc) in app.net.places.recents.iter().enumerate() {
        let row = rect(left, y, right, y + ROW);
        p.round_fill(row, 6.0, th.surface_alt);
        p.round_stroke(row, 6.0, th.border);
        p.circle_stroke(left + 18.0, y + ROW / 2.0, 6.0, th.text_dim, 1.2);
        p.text(FontId::Ui, &shown(&rc.address, style), rect(left + 36.0, y + 12.0, right - 240.0, y + ROW), th.text);
        p.text(FontId::UiRight, &crate::net::places::ago(now, rc.date, offset), rect(right - 236.0, y + 12.0, right - 76.0, y + ROW), th.text_dim);
        let star = rect(right - 64.0, y + 8.0, right - 38.0, y + ROW - 8.0);
        let fav = Address::parse(&rc.address).is_ok_and(|a| app.net.places.is_favorite(&a));
        icon_button(p, star, if fav { "\u{E735}" } else { "\u{E734}" }, hot(star));
        hits.push((star, PageHit::StarRecent(i)));
        let x = rect(right - 36.0, y + 8.0, right - 10.0, y + ROW - 8.0);
        icon_button(p, x, "\u{E711}", hot(x));
        hits.push((x, PageHit::RemoveRecent(i)));
        y += ROW + GAP;
    }
    if app.net.places.recents.is_empty() {
        p.text(FontId::Small, "Sin recientes.", rect(left, y, right, y + 18.0), th.text_dim);
        y += 22.0;
    }
    y += 14.0;

    // Conectados ahora.
    p.text(FontId::UiBold, "Conectados ahora", rect(left, y + 6.0, right, y + 26.0), th.text_secondary);
    y += 32.0;
    let connected: Vec<_> = app.net.connections.iter().enumerate().filter(|(_, c)| c.connected).collect();
    for (i, c) in &connected {
        let row = rect(left, y, right, y + ROW);
        p.round_fill(row, 6.0, th.surface_alt);
        p.round_stroke(row, 6.0, th.border);
        p.text(FontId::Glyphs, "\u{E968}", rect(left + 8.0, y + 11.0, left + 32.0, y + ROW - 9.0), th.text_dim);
        p.text(FontId::Ui, &shown(&c.address, style), rect(left + 36.0, y + 12.0, left + 244.0, y + ROW), th.text);
        let where_ = match (c.letter, style) {
            (Some(l), _) => format!("{l}:"),
            (None, KeyStyle::Windows) => "sin letra".into(),
            (None, KeyStyle::Mac) => c.local.to_string_lossy().into_owned(),
        };
        p.text(FontId::Ui, &where_, rect(left + 252.0, y + 12.0, right - 48.0, y + ROW), th.text_dim);
        let e = rect(right - 36.0, y + 8.0, right - 10.0, y + ROW - 8.0);
        crate::list::draw_eject(p, e, hot(e));
        hits.push((e, PageHit::Eject(*i)));
        y += ROW + GAP;
    }
    if connected.is_empty() {
        p.text(FontId::Small, "No hay nada conectado.", rect(left, y, right, y + 18.0), th.text_dim);
        y += 22.0;
    }
    y += 14.0;

    // Contraseñas.
    let has_place = net.has_credentials_place();
    let info = rect(left, y, right, y + 54.0);
    p.round_fill(info, 6.0, th.surface_alt);
    p.round_stroke(info, 6.0, th.border);
    p.circle_stroke(left + 18.0, y + 27.0, 7.0, th.text_dim, 1.2);
    p.text(FontId::Small, "i", rect(left + 16.0, y + 20.0, left + 21.0, y + 34.0), th.text_dim);
    let text_right = if has_place { right - 110.0 } else { right - 12.0 };
    p.text(FontId::Ui, &password_text(style, has_place), rect(left + 36.0, y + 18.0, text_right, y + 40.0), th.text);
    if has_place {
        let b = rect(right - 96.0, y + 13.0, right - 12.0, y + 41.0);
        button(p, b, "Abrir", hot(b));
        hits.push((b, PageHit::OpenCredentials));
    }
    y += 54.0 + 12.0;

    // macOS sin permiso de red local.
    if net.local_network_denied() {
        let r2 = rect(left, y, right, y + 54.0);
        p.round_fill(r2, 6.0, alpha(th.warning, 0.12));
        p.round_stroke(r2, 6.0, alpha(th.warning, 0.5));
        p.text(FontId::Ui, LOCAL_NETWORK_HINT, rect(left + 14.0, y + 18.0, right - 150.0, y + 40.0), th.text);
        let b = rect(right - 136.0, y + 13.0, right - 12.0, y + 41.0);
        button(p, b, "Abrir Ajustes", hot(b));
        hits.push((b, PageHit::OpenLocalNetwork));
        y += 54.0 + 12.0;
    }
    app.net.page.hits = hits;
    if let Some(rects) = edit_rects {
        app.place_page_boxes(rects);
    }
    y - r.top
}

/// Clic en la página: `true` si lo atendió.
pub(super) fn click(app: &mut App, x: f32, y: f32) -> bool {
    let Some(&(_, hit)) = app.net.page.hits.iter().find(|(r, _)| contains(r, x, y)) else { return false };
    let places = &mut app.net.places;
    match hit {
        PageHit::AddFavorite => {
            let n = places.favorites.len();
            app.start_page_edit(n);
            return true;
        }
        PageHit::EditFavorite(i) => {
            app.start_page_edit(i);
            return true;
        }
        PageHit::FavoriteUp(i) => places.move_favorite(i, true),
        PageHit::FavoriteDown(i) => places.move_favorite(i, false),
        PageHit::RemoveFavorite(i) => {
            app.end_page_edit();
            app.net.places.remove_favorite(i);
        }
        PageHit::ClearRecents => places.clear_recents(),
        PageHit::RemoveRecent(i) => places.remove_recent(i),
        PageHit::StarRecent(i) => {
            if let Some(a) = places.recents.get(i).and_then(|r| Address::parse(&r.address).ok()) {
                places.toggle_favorite(&a, None);
            }
        }
        PageHit::Eject(i) => {
            if let Some(local) = app.net.connections.get(i).map(|c| c.local.clone()) {
                app.eject(local);
            }
            app.redraw();
            return true;
        }
        PageHit::OpenCredentials => {
            sys::network().open_credentials_place(app.hwnd);
            return true;
        }
        PageHit::OpenLocalNetwork => {
            sys::network().open_local_network_settings(app.hwnd);
            return true;
        }
    }
    app.save_net_places();
    app.net_changed();
    true
}

/// Enter (`commit`) o Esc en una caja de la página: `true` si era de acá.
pub(super) fn field_done(app: &mut App, b: TextBox, commit: bool) -> bool {
    let Some((i, name_b, addr_b, _)) = app.net.page.editing.as_ref().map(|(i, n, a, r)| (*i, *n, *a, *r)) else { return false };
    if b != name_b && b != addr_b {
        return false;
    }
    if !commit {
        app.end_page_edit();
        app.net.page.error = None;
        app.redraw();
        return true;
    }
    let (name, address) = (app.host.text_get(name_b), app.host.text_get(addr_b));
    let addr = match Address::parse(&address) {
        Ok(a) => a,
        Err(e) => {
            app.net.page.error = Some((i, e));
            app.redraw();
            return true;
        }
    };
    let url = addr.to_url();
    let name = Some(name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| url.clone());
    let favs = &mut app.net.places.favorites;
    match favs.get_mut(i) {
        Some(f) => {
            f.name = name;
            f.address = url;
        }
        None => favs.push(crate::net::NetFavorite { name, address: url }),
    }
    app.net.page.error = None;
    app.end_page_edit();
    app.save_net_places();
    app.net_changed();
    true
}

impl App {
    /// Empieza a editar el favorito `i` (o uno nuevo con `i == len`): las cajas se crean en su
    /// lugar al dibujar.
    pub(super) fn start_page_edit(&mut self, i: usize) {
        self.end_page_edit();
        let (name, address) = self.net.places.favorites.get(i).map_or((String::new(), String::new()), |f| (f.name.clone(), shown(&f.address, self.net_style())));
        let zero = Rect::default();
        let (Some(n), Some(a)) = (
            self.host.text_create(TextKind::Field, zero, &name, &self.theme),
            self.host.text_create(TextKind::Field, zero, &address, &self.theme),
        ) else {
            return;
        };
        self.host.text_focus(if i < self.net.places.favorites.len() { n } else { a });
        self.net.page.editing = Some((i, n, a, (zero, zero)));
        self.redraw();
    }

    pub(super) fn end_page_edit(&mut self) {
        if let Some((_, n, a, _)) = self.net.page.editing.take() {
            self.host.text_destroy(n);
            self.host.text_destroy(a);
            self.host.focus_main();
        }
    }

    /// Las cajas de la fila en edición en su lugar (se recrean si la fila se movió).
    fn place_page_boxes(&mut self, rects: (Rect, Rect)) {
        let Some((i, n, a, at)) = self.net.page.editing.as_ref().map(|(i, n, a, r)| (*i, *n, *a, *r)) else { return };
        if at == rects {
            return;
        }
        let inner = |r: Rect| rect(r.left + 4.0, r.top + 3.0, r.right - 4.0, r.bottom - 3.0);
        let (tn, ta) = (self.host.text_get(n), self.host.text_get(a));
        let (fn_, fa) = (self.host.text_has_focus(n), self.host.text_has_focus(a));
        self.host.text_destroy(n);
        self.host.text_destroy(a);
        let (Some(n2), Some(a2)) = (
            self.host.text_create(TextKind::Field, inner(rects.0), &tn, &self.theme),
            self.host.text_create(TextKind::Field, inner(rects.1), &ta, &self.theme),
        ) else {
            self.net.page.editing = None;
            return;
        };
        if fn_ {
            self.host.text_focus(n2);
        } else if fa {
            self.host.text_focus(a2);
        }
        self.net.page.editing = Some((i, n2, a2, rects));
    }
}

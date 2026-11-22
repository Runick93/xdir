//! Sección «Lugares» de la configuración: los lugares del árbol (carpetas del usuario, Papelera,
//! Acceso rápido de Windows y carpetas propias) con un switch para mostrar u ocultar cada uno,
//! ↑ ↓ para ordenarlos y «Quitar» en las carpetas propias; abajo, «Agregar carpeta…». Cada cambio
//! rearma el árbol (`Tree::set_places`) y guarda `lugares.toml` en un hilo.
//!
//! Es una página extra de la configuración (`settings_pages::PAGES`), sin campos de texto: `draw`
//! solo dibuja y anota dónde quedó cada control (`App::places_hits`).

use crate::geom::{Rect, contains, rect};
use crate::host::TextBox;
use crate::paint::{FontId, Painter, ellipsize};
use crate::places::Place;
use crate::settings::SearchItem;
use crate::settings::controls::{self, SWITCH_H, SWITCH_W};
use crate::sys;
use crate::theme::alpha;

use std::path::{Path, PathBuf};

use super::{App, Hit};
use crate::drop::{DragEvent, DragKind, Effect};

/// Lo que se puede tocar en la página; el índice es la posición en `Places::order`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlaceHit {
    Toggle(usize),
    Up(usize),
    Down(usize),
    Remove(usize),
    Add,
}

const ROW: f32 = 56.0;
const GAP: f32 = 4.0;
const WIDTH: f32 = 820.0;
/// Lo mínimo que se le deja al nombre: a la izquierda de eso no va ningún botón.
pub(super) const MIN_NAME_W: f32 = 90.0;
/// Con menos lugar que esto para el texto, una carpeta propia muestra solo el nombre (sin ruta).
const MIN_PATH_W: f32 = 160.0;
const PICK_TITLE: &str = "Agregar carpeta a Lugares";

fn item(title: &str, desc: &str, keywords: &[&str]) -> SearchItem {
    SearchItem { title: title.into(), desc: desc.into(), keywords: keywords.iter().map(|k| k.to_string()).collect() }
}

/// Lo que encuentra el buscador en esta página.
pub(super) fn search(_app: &App) -> Vec<SearchItem> {
    const COMMON: [&str; 3] = ["lugares", "arbol", "favoritos"];
    fn with(extra: &[&'static str]) -> Vec<&'static str> {
        COMMON.iter().chain(extra).copied().collect()
    }
    vec![
        item(
            "Lugares del árbol",
            "Cuáles se ven y en qué orden: Escritorio, Descargas, Papelera, Acceso rápido…",
            &with(&["escritorio", "descargas", "documentos", "imagenes", "musica", "videos", "papelera", "acceso rapido", "ocultar", "orden"]),
        ),
        item("Carpetas propias", "Sumar al árbol carpetas que usás seguido", &with(&["carpeta", "agregar", "propias"])),
    ]
}

/// ¿Se muestra la fila del Acceso rápido? Solo donde lo hay (Windows): se vigila su carpeta
/// (`App::start_quick_access`) o ya llegaron sus carpetas. Sin tocar el disco.
/// El texto de la entrada de los menús que agrega o quita una carpeta de los lugares.
pub(super) fn place_label(is_place: bool) -> &'static str {
    if is_place { "Quitar de Lugares" } else { "Agregar a Lugares" }
}

pub(super) fn has_quick_access(app: &App) -> bool {
    app.quick_watch.is_some() || app.tree.has_quick_items()
}

/// Índices de `Places::order` que se muestran, en orden.
fn shown_rows(app: &App) -> Vec<usize> {
    let quick = has_quick_access(app);
    app.places.order.iter().enumerate().filter(|(_, p)| quick || **p != Place::QuickAccess).map(|(i, _)| i).collect()
}

fn icon_button(p: &dyn Painter, r: Rect, glyph: &str, hot: bool, enabled: bool) {
    let th = p.theme();
    if hot && enabled {
        p.round_fill(r, 4.0, alpha(th.text, 0.12));
    }
    let color = if !enabled {
        alpha(th.text_dim, 0.4)
    } else if hot {
        th.text
    } else {
        th.text_dim
    };
    p.text(FontId::GlyphsSmall, glyph, rect(r.left, r.top + 6.0, r.right, r.bottom - 6.0), color);
}

/// Dibuja la página en `r` (arriba a la izquierda del contenido) y devuelve el alto que ocupa.
pub(super) fn draw(app: &mut App, p: &dyn Painter, r: Rect, hover: Option<(f32, f32)>) -> f32 {
    let th = p.theme();
    let left = r.left;
    let right = (r.left + WIDTH).min(r.right);
    let mut hits: Vec<(Rect, PlaceHit)> = Vec::new();
    let hot = |b: Rect| hover.is_some_and(|(x, y)| contains(&b, x, y));
    // El título de la página lo dibuja el marco de la configuración.
    let mut y = r.top;
    p.text(FontId::Small, "Lo que se ve en el árbol, en este orden. Las carpetas propias se pueden quitar.", rect(left, y, right, y + 18.0), th.text_dim);
    y += 28.0;
    let rows = shown_rows(app);
    for (k, &i) in rows.iter().enumerate() {
        let place = &app.places.order[i];
        let shown = !app.places.is_hidden(place);
        let (glyph, color, name) = crate::tree::place_look(place, th);
        let row = rect(left, y, right, y + ROW);
        p.round_fill(row, 6.0, th.surface_alt);
        p.round_stroke(row, 6.0, th.border);
        p.text(FontId::Glyphs, glyph, rect(left + 14.0, y + 17.0, left + 40.0, y + ROW - 15.0), if shown { color } else { alpha(color, 0.45) });
        // Los botones van a la derecha: switch, ↑, ↓ y, en las carpetas propias, «Quitar».
        let own = matches!(place, Place::Folder(_));
        let cy = y + ROW / 2.0;
        // Ningún botón empieza a la izquierda de esto (el ícono y lo mínimo del nombre); los que
        // no entran no se muestran (primero «Quitar»).
        let text_left = left + 52.0;
        let floor = text_left + MIN_NAME_W;
        let mut x = right - 16.0;
        let s = rect(x - SWITCH_W, cy - SWITCH_H / 2.0, x, cy + SWITCH_H / 2.0);
        let s_hit = rect(s.left - 6.0, y + 6.0, s.right + 6.0, y + ROW - 6.0);
        controls::switch(p, s, shown, true, hot(s_hit));
        hits.push((s_hit, PlaceHit::Toggle(i)));
        x = s.left - 16.0;
        for (glyph, hit, enabled) in [("\u{E70D}", PlaceHit::Down(i), k + 1 < rows.len()), ("\u{E70E}", PlaceHit::Up(i), k > 0)] {
            let b = rect(x - 26.0, cy - 12.0, x, cy + 12.0);
            if b.left < floor {
                break;
            }
            icon_button(p, b, glyph, hot(b), enabled);
            if enabled {
                hits.push((b, hit));
            }
            x = b.left - 2.0;
        }
        let quit = rect(x - 8.0 - 80.0, cy - 14.0, x - 8.0, cy + 14.0);
        if own && quit.left >= floor {
            let b = quit;
            controls::button(p, b, "Quitar", hot(b), true);
            hits.push((b, PlaceHit::Remove(i)));
            x = b.left;
        }
        let text_right = (x - 16.0).max(floor);
        let title_color = if shown { th.text } else { th.text_dim };
        if let Place::Folder(path) = place
            && text_right - text_left >= MIN_PATH_W
        {
            let full = path.to_string_lossy();
            p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &name, text_right - text_left), rect(text_left, y + 10.0, text_right, y + 28.0), title_color);
            p.text(FontId::Small, &ellipsize(p, FontId::Small, &full, text_right - text_left), rect(text_left, y + 31.0, text_right, y + 47.0), th.text_dim);
        } else {
            p.text(FontId::Ui, &ellipsize(p, FontId::Ui, &name, text_right - text_left), rect(text_left, cy - 9.0, text_right, cy + 9.0), title_color);
        }
        y += ROW + GAP;
    }
    y += 12.0;
    let add = rect(left, y, left + controls::BUTTON_W, y + controls::CONTROL_H);
    controls::button(p, add, "Agregar carpeta…", hot(add), true);
    hits.push((add, PlaceHit::Add));
    y += controls::CONTROL_H + 12.0;
    app.places_hits = hits;
    y - r.top
}

/// Clic en la página: `true` si lo atendió.
pub(super) fn click(app: &mut App, x: f32, y: f32) -> bool {
    let Some(&(_, hit)) = app.places_hits.iter().find(|(r, _)| contains(r, x, y)) else { return false };
    match hit {
        PlaceHit::Toggle(i) => app.places.toggle(i),
        PlaceHit::Up(i) => app.move_place(i, true),
        PlaceHit::Down(i) => app.move_place(i, false),
        PlaceHit::Remove(i) => app.places.remove(i),
        PlaceHit::Add => {
            let Some(path) = sys::shell().pick_folder(app.hwnd, PICK_TITLE) else { return true };
            app.places.add_folder(path);
        }
    }
    app.places_changed();
    true
}

/// La página no tiene campos propios.
pub(super) fn field_done(_app: &mut App, _b: TextBox, _commit: bool) -> bool {
    false
}

pub(super) fn field_tab(_app: &mut App, _b: TextBox, _back: bool) -> bool {
    false
}

impl App {
    /// Sube (o baja) `order[i]` hasta pasar la fila de al lado que se ve: si en el medio hay un
    /// lugar que la página no muestra (el Acceso rápido fuera de Windows), lo saltea.
    fn move_place(&mut self, i: usize, up: bool) {
        let rows = shown_rows(self);
        let Some(k) = rows.iter().position(|&r| r == i) else { return };
        let target = if up { k.checked_sub(1).map(|k| rows[k]) } else { rows.get(k + 1).copied() };
        let Some(target) = target else { return };
        let mut at = i;
        while at != target {
            if up {
                self.places.move_up(at);
                at -= 1;
            } else {
                self.places.move_down(at);
                at += 1;
            }
        }
    }

    /// Lo soltado entre los lugares del árbol (carpetas o la carpeta de una pestaña) queda como
    /// carpeta propia en ese lugar, como en la barra lateral del Finder, en las 3 plataformas.
    pub(super) fn add_dropped_places(&mut self, paths: Vec<PathBuf>, before: Option<Place>) {
        if paths.is_empty() {
            return;
        }
        for p in paths {
            self.places.add_folder_at(p, before.as_ref());
        }
        self.places_changed();
    }

    /// ¿La carpeta ya se ve entre los lugares? (una carpeta propia o una del usuario no oculta)
    pub(super) fn is_place(&self, path: &Path) -> bool {
        self.place_of_path(path).is_some_and(|p| !self.places.is_hidden(&p))
    }

    /// El lugar que corresponde a la carpeta, si es una carpeta propia o una del usuario.
    fn place_of_path(&self, path: &Path) -> Option<Place> {
        let fs = sys::file_system();
        self.places.order.iter().find(|p| match p {
            Place::Folder(q) => fs.same_path(q, path),
            Place::User(i) => fs.user_folder(*i).is_some_and(|u| fs.same_path(&u, path)),
            _ => false,
        }).cloned()
    }

    /// «Agregar a Lugares» o «Quitar de Lugares»: una carpeta propia se quita, una del usuario se
    /// oculta (se vuelve a ver desde Configuración ▸ Lugares) y cualquier otra se agrega al final.
    pub(super) fn toggle_place(&mut self, path: PathBuf) {
        let shown = self.is_place(&path);
        match self.place_of_path(&path) {
            Some(Place::Folder(_)) if shown => {
                if let Some(i) = self.places.order.iter().position(|p| matches!(p, Place::Folder(q) if sys::file_system().same_path(q, &path))) {
                    self.places.remove(i);
                }
            }
            Some(p) if shown => self.places.hidden.push(p),
            _ => self.places.add_folder(path),
        }
        self.notice = Some(if shown { "Se quitó de Lugares" } else { "Se agregó a Lugares" }.into());
        self.places_changed();
    }

    /// El hueco entre lugares bajo el puntero, si el árbol se ve ahí (no tapado por otra cosa).
    pub(super) fn place_gap_at(&self, x: f32, y: f32, whole_row: bool) -> Option<crate::tree::PlaceGap> {
        let over = !self.xtree_open && self.tree.contains(x, y) && matches!(self.hit(x, y), Hit::None | Hit::TreePin | Hit::TreeXtree);
        over.then(|| self.tree.place_gap_at(x, y, whole_row)).flatten()
    }

    /// Arrastrando la pestaña `index` del panel `pane`: el hueco entre lugares bajo el puntero, si
    /// la pestaña muestra una carpeta (lo virtual, como la Papelera o Configuración, no).
    pub(super) fn tab_place_gap(&self, pane: usize, index: usize, x: f32, y: f32) -> Option<crate::tree::PlaceGap> {
        let tab = self.panes.get(pane)?.tabs.get(index)?;
        matches!(tab.location, crate::fs::Location::Path(_)).then(|| self.place_gap_at(x, y, true)).flatten()
    }

    /// Deja la línea de inserción del árbol en `row`; repinta si cambió.
    pub(super) fn set_place_drop(&mut self, row: Option<usize>) {
        if self.tree.place_drop != row {
            self.tree.place_drop = row;
            self.redraw();
        }
    }

    /// Arrastre de carpetas (de la lista, el Finder o el Explorador) entre los lugares del árbol:
    /// la línea de inserción y, al soltar, el lugar nuevo. Devuelve si el evento quedó resuelto
    /// (si no, sigue como soltar en una carpeta).
    pub(super) fn on_drag_places(&mut self, ev: &mut DragEvent, x: f32, y: f32) -> bool {
        let gap = match ev.kind {
            DragKind::Enter | DragKind::Over | DragKind::Drop => self.place_gap_at(x, y, false),
            DragKind::Leave => None,
        };
        let dirs: Vec<PathBuf> = ev.paths.map(|p| p.iter().filter(|p| crate::fs::probe_dir(p) != Some(false)).cloned().collect()).unwrap_or_default();
        let (Some(gap), false) = (gap, dirs.is_empty()) else {
            self.set_place_drop(None);
            return false;
        };
        // Un lugar es un vínculo: no se copia ni se mueve nada.
        ev.effect = if ev.allowed.allows(Effect::LINK) {
            Effect::LINK
        } else if ev.allowed.allows(Effect::COPY) {
            Effect::COPY
        } else {
            Effect::NONE
        };
        if ev.effect == Effect::NONE {
            self.set_place_drop(None);
            return true;
        }
        if ev.kind == DragKind::Drop {
            self.set_place_drop(None);
            self.add_dropped_places(dirs, gap.before);
        } else {
            self.set_place_drop(Some(gap.row));
        }
        true
    }

    /// Cambiaron los lugares: el árbol los rearma (y lee en otro hilo las carpetas propias
    /// nuevas, para atenuar las que no existen) y `lugares.toml` se guarda en un hilo.
    pub(super) fn places_changed(&mut self) {
        self.tree.set_places(&self.places);
        self.tree.probe_places(self.hwnd, self.show_hidden);
        self.places.save_in_background();
        self.redraw();
    }
}

#[cfg(test)]
#[path = "places_page_tests.rs"]
mod tests;

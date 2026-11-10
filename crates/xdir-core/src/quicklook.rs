//! Vista rápida (barra espaciadora): qué elementos recorre y en qué orden, el elemento que se le
//! pasa a la plataforma, y lo que dibuja xdir mismo: la barra de título de la ventana de Windows
//! (`draw_title_bar`, diseño `quicklook::draw_title_bar` de X43/X44) y la tarjeta de respaldo
//! (`draw_fallback`: la imagen grande, «Disponible solo en línea» o «No hay un visor…»).
//!
//! El visor de verdad es de cada sistema (`WindowHost::quick_look_show`): `QLPreviewPanel` en
//! macOS y una ventana con `IPreviewHandler` en Windows. Donde no hay (Linux, o un elemento solo en
//! la nube), la `App` dibuja la tarjeta encima de los paneles (`overlay_rect`).
//!
//! El recorrido se guarda por nombre (no por posición): si la carpeta cambia con la vista rápida
//! abierta, `resync` vuelve a ubicar el actual, o el que quedó en su lugar si se borró.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

use crate::fs::Entry;
use crate::geom::{Color, Rect, contains, rect};
use crate::list::FileList;
use crate::paint::{FontId, Painter};
use crate::tab::Tab;
use crate::theme::alpha;
use crate::thumbs;

pub use crate::preview::draw::CLOUD_TEXT;

/// Alto de la barra de título.
pub const TITLE_H: f32 = 40.0;
/// Tamaño de la ventana al empezar y su mínimo.
pub const WINDOW_W: f32 = 960.0;
pub const WINDOW_H: f32 = 680.0;
pub const MIN_W: f32 = 480.0;
pub const MIN_H: f32 = 360.0;
/// Lo que se deja libre alrededor de la tarjeta dentro de la ventana de xdir.
const OVERLAY_MARGIN: f32 = 24.0;
/// Lado de la imagen grande que se pide (el ancho de la imagen de X44).
pub const IMAGE_DIPS: f32 = 768.0;
/// Ícono grande de la tarjeta (sin imagen, o solo en la nube).
const ICON: f32 = 96.0;

pub const NO_VIEWER_TEXT: &str = "No hay un visor para este tipo de archivo";
pub const OPEN_TEXT: &str = "Abrir";
pub const OPEN_WITH_TEXT: &str = "Abrir con…";

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// El elemento que se muestra, tal como se le pasa a la plataforma.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickLookItem {
    pub path: Option<PathBuf>,
    pub pidl: Option<Arc<[u8]>>,
    pub name: String,
    pub icon: i32,
    /// Posición en el recorrido (desde 0) y largo del recorrido: «k de n».
    pub index: usize,
    pub total: usize,
    /// Solo en la nube: no se lee ni se pide su imagen (macOS: no va a `QLPreviewPanel`). `item`
    /// no lo sabe (no toca el disco): lo completa la `App` con la respuesta de `sys::preview()`.
    pub cloud_only: bool,
    /// Imagen grande (vista sin visor del sistema).
    pub key: thumbs::Key,
}

/// «k de n» (k desde 1).
pub fn position_text(index: usize, total: usize) -> String {
    format!("{} de {}", index + 1, total)
}

/// Cómo se reconoce un elemento entre recargas: su ruta completa si la tiene (resultados de
/// búsqueda, donde el nombre se puede repetir), si no su nombre.
fn ident(e: &Entry) -> &OsStr {
    e.full.as_deref().unwrap_or(&e.name)
}

/// Lo que recorre la vista rápida abierta.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickLook {
    /// Panel de la lista.
    pub pane: usize,
    /// El recorrido, en el orden visible de la lista.
    pub names: Vec<OsString>,
    /// Posición actual en `names`.
    pub cur: usize,
    /// Lo muestra el sistema (`true`) o la tarjeta de xdir (`false`).
    pub native: bool,
    /// Recorre solo los seleccionados (había varios): la lista acompaña con el foco, sin cambiar
    /// la selección. Si no, recorre toda la carpeta y la selección acompaña.
    pub selection: bool,
}

impl QuickLook {
    /// Con varios seleccionados recorre solo esos (en el orden visible); con uno, toda la carpeta
    /// visible. Empieza en el enfocado (o en el primero seleccionado). `None` sin selección.
    pub fn open(list: &FileList, pane: usize) -> Option<QuickLook> {
        let selected = list.selected_positions();
        if selected.is_empty() {
            return None;
        }
        let selection = selected.len() > 1;
        let positions: Vec<usize> = if selection { selected.clone() } else { (0..list.count()).collect() };
        let start = list
            .focus_pos()
            .and_then(|f| positions.iter().position(|&p| p == f))
            .or_else(|| positions.iter().position(|&p| p == selected[0]))
            .unwrap_or(0);
        let names = positions.iter().filter_map(|&p| list.entry_of_pos(p)).map(|i| ident(&list.entries[i]).to_os_string()).collect();
        Some(QuickLook { pane, names, cur: start, native: false, selection })
    }

    /// Pasa al anterior (`delta` < 0) o al siguiente, sin dar la vuelta. Devuelve si se movió.
    pub fn step(&mut self, delta: isize) -> bool {
        let to = self.cur as isize + delta;
        if to < 0 || to >= self.names.len() as isize || delta == 0 {
            return false;
        }
        self.cur = to as usize;
        true
    }

    /// La lista cambió (recarga, filtro, borrado): rearma el recorrido por nombre. Si el actual ya
    /// no está, queda el que ocupó su lugar. `false` si no queda ninguno (hay que cerrar).
    pub fn resync(&mut self, list: &FileList) -> bool {
        let visible = (0..list.count()).filter_map(|p| list.entry_of_pos(p)).map(|i| ident(&list.entries[i]));
        let kept: Vec<&OsStr> = if self.selection {
            let keep: HashSet<&OsStr> = self.names.iter().map(OsString::as_os_str).collect();
            visible.filter(|n| keep.contains(n)).collect()
        } else {
            visible.collect()
        };
        if kept.is_empty() {
            self.names.clear();
            self.cur = 0;
            return false;
        }
        let current = self.names.get(self.cur).map(OsString::as_os_str);
        let cur = current.and_then(|c| kept.iter().position(|n| *n == c)).unwrap_or(self.cur.min(kept.len() - 1));
        // Solo se copian los nombres si el recorrido cambió.
        let same = kept.len() == self.names.len() && kept.iter().zip(&self.names).all(|(a, b)| *a == b.as_os_str());
        if !same {
            self.names = kept.iter().map(|n| n.to_os_string()).collect();
        }
        self.cur = cur;
        true
    }

    /// Posición en la lista del elemento actual.
    pub fn pos_in(&self, list: &FileList) -> Option<usize> {
        let name = self.names.get(self.cur)?.as_os_str();
        (0..list.count()).find(|&p| list.entry_of_pos(p).is_some_and(|i| ident(&list.entries[i]) == name))
    }

    /// El elemento actual para mostrar (solo entre los visibles de la lista). `painter_px`:
    /// píxeles para unos DIPs (el lado de la imagen grande sale del escalón que los cubre). No toca
    /// el disco: `cloud_only` queda en `false` hasta que la `App` lo sepa.
    pub fn item(&self, tab: &Tab, painter_px: &dyn Fn(f32) -> u32) -> Option<QuickLookItem> {
        let list = &tab.list;
        let idx = list.entry_of_pos(self.pos_in(list)?)?;
        let e = &list.entries[idx];
        let item = tab.entry_item(idx);
        let path = Some(item.path.clone()).filter(|p| !p.as_os_str().is_empty());
        let source = match &item.pidl {
            Some(pidl) => thumbs::Source::Pidl(pidl.clone()),
            None => thumbs::Source::Path(item.path.clone()),
        };
        Some(QuickLookItem {
            path,
            pidl: item.pidl,
            name: e.display.clone(),
            icon: e.icon,
            index: self.cur,
            total: self.names.len(),
            cloud_only: false,
            key: (source, thumbs::big_side(painter_px(IMAGE_DIPS))),
        })
    }
}

/// La tarjeta de xdir sobre su ventana de `w`×`h`: 960×680 centrada, achicada para entrar con un
/// margen, nunca menos de 480×360 (salvo que la ventana misma sea más chica).
pub fn overlay_rect(w: f32, h: f32) -> Rect {
    let cw = WINDOW_W.min(w - 2.0 * OVERLAY_MARGIN).max(MIN_W).min(w);
    let ch = WINDOW_H.min(h - 2.0 * OVERLAY_MARGIN).max(MIN_H).min(h);
    let (left, top) = (((w - cw) / 2.0).max(0.0), ((h - ch) / 2.0).max(0.0));
    rect(left, top, left + cw, top + ch)
}

// ---------- barra de título ----------

/// Qué hay bajo el mouse en la barra de título.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitleHit {
    Prev,
    Next,
    Open,
    Close,
    /// El resto de la barra (mover la ventana).
    Drag,
}

/// Rectángulos de la barra (medidas de X43, contadas desde la derecha): ‹, «k de n», ›, «Abrir»
/// y la cruz (el área que responde, más grande que el dibujo).
struct TitleLayout {
    prev: Rect,
    position: Rect,
    next: Rect,
    open: Rect,
    close: Rect,
}

fn title_layout(r: Rect) -> TitleLayout {
    let (right, top) = (r.right, r.top);
    TitleLayout {
        prev: rect(right - 226.0, top + 6.0, right - 204.0, top + 34.0),
        position: rect(right - 204.0, top + 12.0, right - 144.0, top + 28.0),
        next: rect(right - 144.0, top + 6.0, right - 122.0, top + 34.0),
        open: rect(right - 116.0, top + 7.0, right - 44.0, top + 33.0),
        close: rect(right - 36.0, top + 6.0, right - 8.0, top + 34.0),
    }
}

/// Barra de título de 40: ícono, nombre, ‹ «k de n» ›, «Abrir» y la cruz. Los extremos del
/// recorrido apagan su flecha.
pub fn draw_title_bar(p: &dyn Painter, r: Rect, item: &QuickLookItem, hover: Option<TitleHit>) {
    let t = *p.theme();
    let l = title_layout(r);
    // Esquinas de arriba redondeadas como la ventana; abajo, la línea `border`.
    let pts = [(r.left, r.bottom, 0.0), (r.left, r.top, 8.0), (r.right, r.top, 8.0), (r.right, r.bottom, 0.0)];
    p.rounded_path(&pts, true, Some(t.surface_alt), None);
    p.fill(rect(r.left, r.bottom - 1.0, r.right, r.bottom), t.border);
    p.shell_icon(item.icon, 0, r.left + 14.0, r.top + 12.0, 16.0);
    p.text(FontId::UiBold, &item.name, rect(r.left + 38.0, r.top + 12.0, l.prev.left - 8.0, r.top + 28.0), t.text);
    let arrow = |on: bool, hot: bool| if !on { alpha(t.text_dim, 0.4) } else if hot { t.text } else { t.text_dim };
    let (cy, ax) = (r.top + 20.0, l.prev.right - 5.0);
    p.rounded_path(&[(ax, cy - 4.0, 0.0), (ax - 5.0, cy, 0.0), (ax, cy + 4.0, 0.0)], false, None, Some((arrow(item.index > 0, hover == Some(TitleHit::Prev)), 1.5)));
    let nx = l.next.left + 5.0;
    let more = item.index + 1 < item.total;
    p.rounded_path(&[(nx, cy - 4.0, 0.0), (nx + 5.0, cy, 0.0), (nx, cy + 4.0, 0.0)], false, None, Some((arrow(more, hover == Some(TitleHit::Next)), 1.5)));
    p.text(FontId::UiCenter, &position_text(item.index, item.total), l.position, t.text_dim);
    p.round_fill(l.open, 4.0, t.accent);
    if hover == Some(TitleHit::Open) {
        p.round_fill(l.open, 4.0, alpha(WHITE, 0.14));
    }
    bold_centered(p, OPEN_TEXT, rect(l.open.left, l.open.top + 5.0, l.open.right, l.open.bottom - 5.0), WHITE);
    if hover == Some(TitleHit::Close) {
        p.round_fill(rect(l.close.left + 2.0, l.close.top + 2.0, l.close.right - 2.0, l.close.bottom - 2.0), 4.0, t.hover);
    }
    let (x0, y0) = (r.right - 27.0, r.top + 15.0);
    let cross = if hover == Some(TitleHit::Close) { t.text } else { t.text_dim };
    p.rounded_path(&[(x0, y0, 0.0), (x0 + 10.0, y0 + 10.0, 0.0)], false, None, Some((cross, 1.2)));
    p.rounded_path(&[(x0 + 10.0, y0, 0.0), (x0, y0 + 10.0, 0.0)], false, None, Some((cross, 1.2)));
}

/// Qué hay en `(x, y)` de la barra `r`.
pub fn title_hit(r: Rect, x: f32, y: f32) -> Option<TitleHit> {
    if !contains(&r, x, y) {
        return None;
    }
    let l = title_layout(r);
    Some(if contains(&l.prev, x, y) {
        TitleHit::Prev
    } else if contains(&l.next, x, y) {
        TitleHit::Next
    } else if contains(&l.open, x, y) {
        TitleHit::Open
    } else if contains(&l.close, x, y) {
        TitleHit::Close
    } else {
        TitleHit::Drag
    })
}

// ---------- tarjeta de respaldo ----------

/// Botones de la tarjeta sin visor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FallbackHit {
    OpenWith,
    Open,
}

/// Dónde va la imagen grande y los dos botones dentro del cuerpo `r` (debajo de la barra). Con
/// `viewer_missing`, debajo de la imagen va el aviso y los botones (medidas de X44: imagen con 96
/// a los costados y 40 arriba; el aviso 22 más abajo y los botones de 108×30 a 28 del aviso).
fn fallback_layout(r: Rect, viewer_missing: bool) -> (Rect, Rect, Rect) {
    let w = r.right - r.left;
    let mx = (w * 0.1).clamp(16.0, 96.0);
    let block = if viewer_missing { 80.0 } else { 0.0 };
    let bottom = if viewer_missing { 88.0 } else { 40.0 };
    let image = rect(r.left + mx, r.top + 40.0, r.right - mx, (r.bottom - bottom - block).max(r.top + 56.0));
    let cx = (r.left + r.right) / 2.0;
    let by = image.bottom + 50.0;
    (image, rect(cx - 112.0, by, cx - 4.0, by + 30.0), rect(cx + 4.0, by, cx + 112.0, by + 30.0))
}

/// El cuerpo de la vista sin visor del sistema: la imagen grande (o el ícono, si no hay); con
/// `cloud_only`, el ícono y «Disponible solo en línea» (sin pedir la imagen); con
/// `viewer_missing`, además «No hay un visor para este tipo de archivo» y «Abrir con…» y «Abrir».
pub fn draw_fallback(p: &dyn Painter, r: Rect, item: &QuickLookItem, viewer_missing: bool, hover: Option<FallbackHit>) {
    let t = *p.theme();
    let viewer_missing = viewer_missing && !item.cloud_only;
    let (image, open_with, open) = fallback_layout(r, viewer_missing);
    let (cx, cy) = ((image.left + image.right) / 2.0, (image.top + image.bottom) / 2.0);
    if item.cloud_only {
        p.shell_icon(item.icon, 0, cx - ICON / 2.0, cy - ICON / 2.0 - 16.0, ICON);
        p.text(FontId::UiCenter, CLOUD_TEXT, rect(r.left, cy + ICON / 2.0, r.right, cy + ICON / 2.0 + 16.0), t.text_dim);
        return;
    }
    if !p.big_image(&item.key, image) {
        p.shell_icon(item.icon, 0, cx - ICON / 2.0, cy - ICON / 2.0, ICON);
    }
    if !viewer_missing {
        return;
    }
    p.text(FontId::UiCenter, NO_VIEWER_TEXT, rect(r.left, image.bottom + 22.0, r.right, image.bottom + 38.0), t.text_dim);
    p.round_fill(open_with, 4.0, if hover == Some(FallbackHit::OpenWith) { t.hover } else { t.surface_alt });
    p.round_stroke(open_with, 4.0, t.border);
    p.text(FontId::UiCenter, OPEN_WITH_TEXT, rect(open_with.left, open_with.top + 7.0, open_with.right, open_with.bottom - 7.0), t.text);
    p.round_fill(open, 4.0, t.accent);
    if hover == Some(FallbackHit::Open) {
        p.round_fill(open, 4.0, alpha(WHITE, 0.14));
    }
    bold_centered(p, OPEN_TEXT, rect(open.left, open.top + 7.0, open.right, open.bottom - 7.0), WHITE);
}

/// Texto en negrita centrado a lo ancho de `r` (no hay un formato de negrita centrada).
fn bold_centered(p: &dyn Painter, s: &str, r: Rect, c: Color) {
    let w = p.measure(FontId::UiBold, s).min(r.right - r.left);
    let left = (r.left + r.right - w) / 2.0;
    p.text(FontId::UiBold, s, rect(left, r.top, left + w + 1.0, r.bottom), c);
}

/// Qué botón de la tarjeta hay en `(x, y)` (solo sin visor tiene botones).
pub fn fallback_hit(r: Rect, viewer_missing: bool, x: f32, y: f32) -> Option<FallbackHit> {
    if !viewer_missing {
        return None;
    }
    let (_, open_with, open) = fallback_layout(r, true);
    if contains(&open_with, x, y) {
        Some(FallbackHit::OpenWith)
    } else if contains(&open, x, y) {
        Some(FallbackHit::Open)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn item(name: &str, index: usize, total: usize) -> QuickLookItem {
        QuickLookItem {
            path: Some(PathBuf::from(format!("/tmp/{name}"))),
            pidl: None,
            name: name.to_string(),
            icon: 3,
            index,
            total,
            cloud_only: false,
            key: (thumbs::Source::Path(PathBuf::from(format!("/tmp/{name}"))), 1024),
        }
    }

    fn painter() -> RecordingPainter {
        RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)))
    }

    #[test]
    fn title_bar_layout() {
        let p = painter();
        let r = rect(220.0, 50.0, 1180.0, 90.0);
        draw_title_bar(&p, r, &item("balance-anual-2026.pdf", 1, 12), None);
        let texts = p.texts();
        let find = |s: &str| texts.iter().find(|t| t.1 == s).unwrap_or_else(|| panic!("falta «{s}»: {texts:?}")).2;
        let name = find("balance-anual-2026.pdf");
        assert_eq!((name.left, name.top), (r.left + 38.0, r.top + 12.0));
        let pos = find("2 de 12");
        assert_eq!((pos.left, pos.right), (r.left + 756.0, r.left + 816.0));
        // «Abrir» centrado en su botón de 844 a 916 (el pintor de prueba mide 7 por carácter).
        let open = find("Abrir");
        assert_eq!((open.left, open.right), (r.left + 880.0 - 17.5, r.left + 880.0 + 18.5));
        assert!(name.right <= r.left + 744.0, "el nombre no pisa las flechas");
        assert_eq!(p.icons(), [(r.left + 14.0, r.top + 12.0, 16.0)]);
        // Lo que responde a los clics.
        assert_eq!(title_hit(r, r.left + 746.0, r.top + 20.0), Some(TitleHit::Prev));
        assert_eq!(title_hit(r, r.left + 823.0, r.top + 20.0), Some(TitleHit::Next));
        assert_eq!(title_hit(r, r.left + 880.0, r.top + 20.0), Some(TitleHit::Open));
        assert_eq!(title_hit(r, r.left + 938.0, r.top + 20.0), Some(TitleHit::Close));
        assert_eq!(title_hit(r, r.left + 300.0, r.top + 20.0), Some(TitleHit::Drag));
        assert_eq!(title_hit(r, r.left + 300.0, r.bottom + 2.0), None);
    }

    #[test]
    fn fallback_texts() {
        let p = painter();
        let r = rect(220.0, 90.0, 1180.0, 730.0);
        draw_fallback(&p, r, &item("VID_3102.mp4", 11, 12), true, None);
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        assert_eq!(texts, [NO_VIEWER_TEXT, "Abrir con…", "Abrir"]);
        assert_eq!(p.big_requests().len(), 1, "la imagen grande");
        // Medidas de X44 (cuerpo de 960×640 desde y = 40 de la ventana).
        let (img, with, open) = fallback_layout(r, true);
        assert_eq!((img.left, img.top, img.right, img.bottom), (r.left + 96.0, r.top + 40.0, r.right - 96.0, r.top + 472.0));
        assert_eq!((with.left, with.top, with.right), (r.left + 368.0, r.top + 522.0, r.left + 476.0));
        assert_eq!((open.left, open.right), (r.left + 484.0, r.left + 592.0));
        assert_eq!(fallback_hit(r, true, with.left + 4.0, with.top + 4.0), Some(FallbackHit::OpenWith));
        assert_eq!(fallback_hit(r, true, open.left + 4.0, open.top + 4.0), Some(FallbackHit::Open));
        assert_eq!(fallback_hit(r, false, open.left + 4.0, open.top + 4.0), None);
        // Con visor (o en Linux): solo la imagen.
        let p = painter();
        draw_fallback(&p, r, &item("foto.jpg", 0, 1), false, None);
        assert!(p.texts().is_empty());
        // Solo en la nube: el ícono y el aviso, sin pedir la imagen.
        let p = painter();
        let mut cloud = item("nube.jpg", 0, 1);
        cloud.cloud_only = true;
        draw_fallback(&p, r, &cloud, true, None);
        assert_eq!(p.texts().into_iter().map(|t| t.1).collect::<Vec<_>>(), [CLOUD_TEXT]);
        assert!(p.big_requests().is_empty());
        assert_eq!(p.icons().len(), 1);
    }

    #[test]
    fn overlay_rect_clamps() {
        let r = overlay_rect(1400.0, 820.0);
        assert_eq!((r.left, r.top, r.right, r.bottom), (220.0, 70.0, 1180.0, 750.0));
        let r = overlay_rect(800.0, 500.0);
        assert!(r.right - r.left >= MIN_W && r.bottom - r.top >= MIN_H, "{r:?}");
        assert!(r.left >= 0.0 && r.top >= 0.0 && r.right <= 800.0 && r.bottom <= 500.0, "{r:?}");
        assert_eq!(((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0), (400.0, 250.0));
        // Una ventana todavía más chica que el mínimo: la tarjeta no se sale.
        let r = overlay_rect(500.0, 380.0);
        assert_eq!((r.left, r.top, r.right, r.bottom), (10.0, 10.0, 490.0, 370.0));
        let r = overlay_rect(400.0, 300.0);
        assert_eq!((r.left, r.top, r.right, r.bottom), (0.0, 0.0, 400.0, 300.0));
    }

    #[test]
    fn quick_look_step_no_wrap() {
        let mut q = QuickLook { pane: 0, names: vec!["a".into(), "b".into()], cur: 0, native: false, selection: false };
        assert!(!q.step(-1));
        assert!(q.step(1));
        assert!(!q.step(1));
        assert_eq!(q.cur, 1);
    }
}

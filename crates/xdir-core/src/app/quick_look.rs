//! Vista rápida en la `App`: abrirla con Espacio (o el menú y la paleta), recorrer con las flechas
//! (la lista acompaña), seguir los cambios de la carpeta y cerrarla. Lo muestra el visor del
//! sistema (`WindowHost::quick_look_show`); si no hay, o el elemento está solo en la nube, la
//! `App` dibuja su tarjeta encima de los paneles (`paint_quick_look`).
//!
//! Si un elemento está solo en la nube se pregunta fuera del hilo de interfaz
//! (`sys::preview()` con `PreviewTarget::Cloud`): un volumen de red muerto no traba la ventana.
//! Hasta que llega la respuesta se sigue viendo lo anterior (al abrir, la tarjeta con el ícono), y
//! al host recién se le pasa el elemento cuando se sabe que no está solo en la nube.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::keys::keys::{VK_LEFT, VK_RIGHT, VK_SPACE};
use crate::paint::Painter;
use crate::preview::{PreviewBody, PreviewReply, PreviewRequest, PreviewTarget};
use crate::quicklook::{self, FallbackHit, QuickLook, QuickLookItem, TitleHit};
use crate::thumbs;

/// La vista rápida abierta: el recorrido, de qué pestaña y carpeta es, lo que se ve y lo que
/// espera saber si está en la nube.
pub(super) struct QlState {
    pub ql: QuickLook,
    tab: usize,
    location: Location,
    /// Lo que se ve, con su estado de nube ya sabido.
    pub shown: Option<QuickLookItem>,
    /// El elemento al que se pasó, esperando la respuesta de nube (no va al host todavía).
    pub pending: Option<QuickLookItem>,
    /// Estado de nube ya leído en esta vista rápida, por ruta.
    cloud: HashMap<PathBuf, bool>,
    generation: u64,
    current: Arc<AtomicU64>,
    tx: Sender<PreviewReply>,
    rx: Receiver<PreviewReply>,
}

impl QlState {
    fn new(ql: QuickLook, tab: usize, location: Location) -> QlState {
        let (tx, rx) = channel();
        QlState { ql, tab, location, shown: None, pending: None, cloud: HashMap::new(), generation: 0, current: Arc::new(AtomicU64::new(0)), tx, rx }
    }

    /// Una generación nueva: la respuesta que estaba en camino queda vieja.
    fn bump(&mut self) -> u64 {
        self.generation += 1;
        self.current.store(self.generation, Ordering::Relaxed);
        self.generation
    }
}

impl App {
    /// Píxeles para unos DIPs según el DPI de la ventana (el escalón de la imagen grande del
    /// elemento que se le pasa al host).
    fn ql_px(&self) -> impl Fn(f32) -> u32 + use<> {
        let dpi = self.dpi.max(96) as f32;
        move |d| (d * dpi / 96.0).round() as u32
    }

    /// Abre la vista rápida sobre el panel activo, o la cierra si estaba abierta. Sin nada
    /// seleccionado no hace nada.
    pub(super) fn toggle_quick_look(&mut self) {
        if self.quick_look.is_some() {
            self.close_quick_look();
            return;
        }
        let pane = self.active;
        let tab = self.panes[pane].tab();
        let Some(ql) = QuickLook::open(&tab.list, pane) else { return };
        self.quick_look = Some(QlState::new(ql, tab.id, tab.location.clone()));
        self.show_quick_look_item();
    }

    /// Lo que ya contestó la columna de vista previa sobre esta ruta (si está en la nube).
    fn preview_cloud_of(&self, path: &std::path::Path) -> Option<bool> {
        self.preview.subject.as_ref().filter(|w| w.target.path() == path)?;
        self.preview.info.as_ref().map(|i| i.body == PreviewBody::CloudOnly)
    }

    /// Pasa al elemento actual del recorrido. Si ya se sabe si está en la nube, lo muestra; si no,
    /// lo pregunta fuera del hilo de interfaz y lo muestra al llegar la respuesta.
    fn show_quick_look_item(&mut self) {
        let px = self.ql_px();
        let Some(st) = &self.quick_look else { return };
        let Some(mut item) = st.ql.item(self.panes[st.ql.pane].tab(), &px) else {
            self.close_quick_look();
            return;
        };
        let known = match item.path.as_deref() {
            None => Some(false),
            Some(p) => st.cloud.get(p).copied().or_else(|| self.preview_cloud_of(p)),
        };
        if let Some(cloud) = known {
            item.cloud_only = cloud;
            self.present_quick_look(item);
            return;
        }
        if st.pending.as_ref() == Some(&item) {
            return;
        }
        let (hwnd, path) = (self.hwnd, item.path.clone().unwrap_or_default());
        let Some(st) = &mut self.quick_look else { return };
        let generation = st.bump();
        st.pending = Some(item);
        sys::preview().request(PreviewRequest { hwnd, tx: st.tx.clone(), generation, current: st.current.clone(), target: PreviewTarget::Cloud(path) });
        self.redraw();
    }

    /// Llegó la respuesta de nube (`WM_APP_PREVIEW`): el elemento que esperaba se muestra.
    pub(super) fn on_quick_look_reply(&mut self) {
        let Some(st) = &mut self.quick_look else { return };
        let mut body = None;
        while let Ok(r) = st.rx.try_recv() {
            if r.generation == st.generation {
                body = Some(r.info.body);
            }
        }
        let (Some(body), Some(mut item)) = (body, st.pending.clone()) else { return };
        let cloud = body == PreviewBody::CloudOnly;
        if let Some(p) = &item.path {
            st.cloud.insert(p.clone(), cloud);
        }
        item.cloud_only = cloud;
        self.present_quick_look(item);
    }

    /// Muestra el elemento (con su estado de nube ya sabido): lo solo en la nube no va al visor
    /// del sistema, lo dibuja la `App` (y si el visor estaba abierto, se cierra), salvo que la
    /// ventana del host tenga su propia tarjeta (`quick_look_shows_cloud`, Windows).
    fn present_quick_look(&mut self, item: QuickLookItem) {
        let Some(st) = &mut self.quick_look else { return };
        if st.pending.take().is_some() {
            st.bump();
        }
        if st.shown.as_ref() == Some(&item) {
            return;
        }
        let host = self.host.clone();
        let was_native = st.ql.native;
        let native = (!item.cloud_only || host.quick_look_shows_cloud()) && host.quick_look_show(&item);
        if was_native && !native {
            host.quick_look_close();
        }
        if let Some(st) = &mut self.quick_look {
            st.ql.native = native;
            st.shown = Some(item);
        }
        self.redraw();
    }

    /// Cierra la vista rápida (y el visor del host, si lo estaba mostrando).
    pub(super) fn close_quick_look(&mut self) {
        let Some(st) = self.quick_look.take() else { return };
        if st.ql.native {
            self.host.quick_look_close();
        }
        if matches!(self.hover, Hit::QuickLookTitle(_) | Hit::QuickLookBody(_)) {
            self.hover = Hit::None;
        }
        self.redraw();
    }

    /// El usuario cerró el visor del sistema por su cuenta (botón de cerrar del panel): la
    /// vista rápida termina sin volver a cerrarlo.
    pub fn on_quick_look_closed(&mut self) {
        if self.quick_look.take().is_some() {
            self.redraw();
        }
    }

    /// Anterior o siguiente del recorrido; la lista acompaña (la selección si recorre la carpeta,
    /// solo el foco si recorre los seleccionados).
    fn quick_look_step(&mut self, delta: isize) {
        // La lista pudo cambiar (orden, filtro, grupos plegados) sin recargarse.
        self.sync_quick_look();
        let Some(st) = &mut self.quick_look else { return };
        if !st.ql.step(delta) {
            return;
        }
        let (pane, selection) = (st.ql.pane, st.ql.selection);
        let pos = st.ql.pos_in(&self.panes[pane].tab().list);
        if let Some(pos) = pos {
            let list = &mut self.panes[pane].tab_mut().list;
            if selection {
                list.focus_only(pos);
            } else {
                list.select_pos(pos);
            }
        }
        self.show_quick_look_item();
        self.redraw();
    }

    /// La carpeta de la vista rápida cambió (se cargó de nuevo, se filtró): se vuelve a ubicar el
    /// elemento por nombre. Si la pestaña pasó a otra carpeta, o no queda ninguno, se cierra.
    pub(super) fn sync_quick_look(&mut self) {
        let Some(st) = &mut self.quick_look else { return };
        let tab = self.panes[st.ql.pane].tab();
        if tab.id != st.tab || tab.location != st.location || !st.ql.resync(&tab.list) {
            self.close_quick_look();
            return;
        }
        self.show_quick_look_item();
    }

    /// Recorriendo la carpeta, si el usuario movió el foco de la lista por otro camino (un clic,
    /// la búsqueda por tipeo), la vista rápida pasa a ese elemento.
    pub(super) fn quick_look_follow_list(&mut self) {
        let Some(st) = &mut self.quick_look else { return };
        let tab = self.panes[st.ql.pane].tab();
        if tab.id != st.tab || tab.location != st.location {
            self.close_quick_look();
            return;
        }
        if st.ql.selection {
            return;
        }
        let Some(pos) = tab.list.focus_pos() else { return };
        if st.ql.pos_in(&tab.list) == Some(pos) {
            return;
        }
        if !st.ql.resync(&tab.list) {
            self.close_quick_look();
            return;
        }
        let Some(st) = &mut self.quick_look else { return };
        if pos < st.ql.names.len() {
            st.ql.cur = pos;
        }
        self.show_quick_look_item();
    }

    /// Teclas de la vista rápida, antes de los atajos. `Some(usada)` si la tecla es de ella;
    /// `None` sigue el camino de siempre.
    pub(super) fn quick_look_key(&mut self, vk: Key, ctrl: bool, shift: bool, alt: bool) -> Option<bool> {
        let plain = !ctrl && !shift && !alt;
        if self.quick_look.is_some() {
            return match vk {
                VK_ESCAPE => {
                    self.close_quick_look();
                    Some(true)
                }
                VK_SPACE if plain => {
                    self.close_quick_look();
                    self.swallow_space = true;
                    Some(true)
                }
                VK_LEFT | VK_UP if plain => {
                    self.quick_look_step(-1);
                    Some(true)
                }
                VK_RIGHT | VK_DOWN if plain => {
                    self.quick_look_step(1);
                    Some(true)
                }
                _ => None,
            };
        }
        if vk != VK_SPACE || !plain || self.xtree_open || self.about_open {
            return None;
        }
        let active = self.active;
        let list = &self.panes[active].tab().list;
        // Búsqueda por tipeo en curso: el espacio es parte del nombre (llega por `on_char`).
        if list.typeahead_active() {
            return Some(false);
        }
        // Sobre el encabezado de un grupo, el Espacio lo pliega (como siempre).
        if list.header_focused() {
            if let Some(action) = self.tab_mut().list.on_key(VK_SPACE.0, false, false) {
                self.apply(active, action);
            }
            self.swallow_space = true;
            return Some(true);
        }
        if list.selected_indices().next().is_none() {
            return Some(false);
        }
        self.run_command(Command::QuickLook);
        self.swallow_space = true;
        Some(true)
    }

    /// La tarjeta de xdir (si la vista rápida está abierta sin visor del sistema).
    pub(super) fn quick_look_card(&self) -> Option<geom::Rect> {
        self.quick_look.as_ref().filter(|s| !s.ql.native)?;
        let (w, h) = self.client_dips();
        Some(quicklook::overlay_rect(w, h))
    }

    /// Qué hay de la tarjeta en `(x, y)` (None: fuera de ella).
    pub(super) fn quick_look_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let card = self.quick_look_card()?;
        if !contains(&card, x, y) {
            return None;
        }
        let (bar, body) = card_parts(card);
        if let Some(t) = quicklook::title_hit(bar, x, y).filter(|t| *t != TitleHit::Drag) {
            return Some(Hit::QuickLookTitle(t));
        }
        Some(Hit::QuickLookBody(quicklook::fallback_hit(body, false, x, y)))
    }

    /// Clic en la tarjeta.
    pub(super) fn quick_look_click(&mut self, hit: Hit) {
        match hit {
            Hit::QuickLookTitle(TitleHit::Prev) => self.quick_look_step(-1),
            Hit::QuickLookTitle(TitleHit::Next) => self.quick_look_step(1),
            Hit::QuickLookTitle(TitleHit::Close) => self.close_quick_look(),
            Hit::QuickLookTitle(TitleHit::Open) | Hit::QuickLookBody(Some(FallbackHit::Open)) => self.quick_look_open_item(false),
            Hit::QuickLookBody(Some(FallbackHit::OpenWith)) => self.quick_look_open_item(true),
            _ => {}
        }
        self.redraw();
    }

    /// «Abrir» (con su programa) o «Abrir con…» (el diálogo del sistema) del elemento actual.
    fn quick_look_open_item(&mut self, with: bool) {
        let Some(item) = self.quick_look.as_ref().and_then(|s| s.pending.clone().or_else(|| s.shown.clone())) else { return };
        let Some(path) = item.path.clone() else { return };
        let target = shell::Item { path, pidl: item.pidl.clone() };
        let pane = self.quick_look.as_ref().map_or(self.active, |s| s.ql.pane);
        if with {
            let location = self.panes[pane].tab().location.clone();
            let _ = sys::shell().invoke_verb(self.hwnd, &location, &[target], "openas");
        } else {
            sys::shell().open_item(self.hwnd, &target);
        }
    }
}

/// Barra de título y cuerpo de la tarjeta.
fn card_parts(card: geom::Rect) -> (geom::Rect, geom::Rect) {
    let bar = geom::rect(card.left, card.top, card.right, card.top + quicklook::TITLE_H);
    (bar, geom::rect(card.left, bar.bottom, card.right, card.bottom))
}

/// Dibuja la tarjeta de la vista rápida (sombra, fondo, barra y cuerpo) sobre la ventana, con
/// lo que ya se sabe del elemento (`QlState::shown`): dibujar no toca el disco. Al abrir, mientras
/// llega la respuesta de nube, la barra y el ícono del elemento.
pub(super) fn paint_quick_look(p: &dyn Painter, w: f32, h: f32, st: &QlState, hover: Hit) {
    let (item, waiting) = match (&st.shown, &st.pending) {
        (Some(item), _) => (item.clone(), false),
        (None, Some(item)) => (item.clone(), true),
        (None, None) => return,
    };
    let mut item = item;
    // El escalón de la imagen con la escala de este pintor.
    item.key.1 = thumbs::big_side(p.thumb_px(quicklook::IMAGE_DIPS));
    let t = *p.theme();
    let card = quicklook::overlay_rect(w, h);
    // Sombra (diseño: negro al 45 %, 12 abajo, desenfoque 36): capas finas que se suman hacia
    // el borde de la tarjeta.
    let black = geom::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    let alpha = if t.dark { 0.05 } else { 0.022 };
    for i in (1..=8).rev() {
        let grow = i as f32 * 3.0;
        let r = geom::rect(card.left - grow, card.top - grow + 12.0, card.right + grow, card.bottom + grow + 12.0);
        p.round_fill(r, 8.0 + grow, theme::alpha(black, alpha));
    }
    p.round_fill(card, 8.0, t.surface);
    let (bar, body) = card_parts(card);
    let title_hover = match hover {
        Hit::QuickLookTitle(h) => Some(h),
        _ => None,
    };
    let body_hover = match hover {
        Hit::QuickLookBody(h) => h,
        _ => None,
    };
    quicklook::draw_title_bar(p, bar, &item, title_hover);
    if waiting {
        const ICON: f32 = 96.0;
        let (cx, cy) = ((body.left + body.right) / 2.0, (body.top + body.bottom) / 2.0);
        p.shell_icon(item.icon, 0, cx - ICON / 2.0, cy - ICON / 2.0, ICON);
    } else {
        p.clip(body, &mut || quicklook::draw_fallback(p, body, &item, false, body_hover));
    }
    p.round_stroke(card, 8.0, t.border);
}

//! Carteles de ayuda de los botones sin texto: qué hacen y su atajo, al dejar el mouse quieto un
//! momento encima. Los dibuja xdir (igual en los tres sistemas), encima de todo lo demás.

use super::*;
use crate::commands::{self, Keymap};
use crate::geom::Rect;
use crate::paint::{FontId, Painter};

/// Pasó la espera del cartel. `lparam` = turno del cartel pendiente (los viejos se descartan).
pub(super) const WM_APP_TOOLTIP: u32 = WM_APP + 23;
/// Cuánto tiene que quedarse el mouse sobre un botón para que aparezca su cartel.
const TOOLTIP_DELAY_MS: u64 = 500;
/// Margen entre el texto y el borde del cartel.
const PAD: f32 = 6.0;
/// Alto de la línea de texto chico.
const LINE_H: f32 = 14.0;
/// Cuánto más abajo del punto del mouse va el cartel (para no tapar la flecha).
const BELOW: f32 = 20.0;
/// Distancia mínima a los bordes de la ventana.
const EDGE: f32 = 4.0;

/// En qué está el cartel del botón bajo el mouse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TipState {
    /// Esperando el aviso de la espera.
    Pending,
    Visible,
    /// Se fue con un clic o una tecla: no vuelve hasta que el mouse pase a otra cosa.
    Dismissed,
}

/// El cartel del botón bajo el mouse.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tip {
    pub hit: Hit,
    /// Punto del mouse donde se lo pidió (el cartel va debajo, o arriba si no entra).
    pub x: f32,
    pub y: f32,
    pub state: TipState,
    /// Turno: el aviso de la espera trae el suyo.
    pub token: u64,
}

/// Texto del cartel de lo que hay bajo el mouse; `None` si no lleva (los botones con texto, la
/// lista…). `tab_path(panel, pestaña)` da la ruta completa de la carpeta de una pestaña;
/// `tree_pinned`: el árbol está fijo (su chincheta lo suelta).
pub(super) fn tooltip_for(hit: Hit, keymap: &Keymap, tab_path: &dyn Fn(usize, usize) -> Option<String>, tree_pinned: bool) -> Option<String> {
    // Los de comandos: el título que muestra la paleta y el atajo en uso.
    let command = |c: Command| {
        let title = commands::ALL.iter().find(|s| s.command == c).map_or("", |s| s.title());
        match keymap.label(c) {
            keys if keys.is_empty() => title.to_string(),
            keys => format!("{title} ({keys})"),
        }
    };
    let text = match hit {
        Hit::Layout(l) => command(Command::Layout(l.index() as u8)),
        Hit::Gear => "Apariencia y opciones".to_string(),
        Hit::GroupBar(ui::GroupHit::Add) => "Guardar los paneles como grupo".to_string(),
        Hit::TreePin if tree_pinned => "Soltar el árbol".to_string(),
        Hit::TreePin => "Fijar el árbol".to_string(),
        Hit::Pane(p, ph) => match ph {
            PaneHit::Tab(t) => return tab_path(p, t),
            PaneHit::NewTab => command(Command::NewTab),
            PaneHit::TabClose(_) => command(Command::CloseTab),
            PaneHit::Maximize => command(Command::Maximize),
            PaneHit::Button(Button::Back) => command(Command::Back),
            PaneHit::Button(Button::Forward) => command(Command::Forward),
            PaneHit::Button(Button::Up) => command(Command::Up),
            PaneHit::Button(Button::Refresh) => command(Command::Refresh),
            PaneHit::Star => command(Command::ToggleFavorite),
            PaneHit::Terminal => "Abrir una consola en la carpeta".to_string(),
            PaneHit::AddressClear => "Borrar".to_string(),
            PaneHit::SearchClear => "Terminar la búsqueda".to_string(),
            PaneHit::FavOverflow => "Más favoritos".to_string(),
            PaneHit::NewFolderButton => command(Command::NewFolder),
            PaneHit::ViewButton => "Cambiar la vista".to_string(),
            _ => return None,
        },
        _ => return None,
    };
    Some(text)
}

/// Rectángulo del cartel para un texto de `text_w` DIPs con el mouse en `(x, y)`, en una ventana
/// de `w`×`h`: debajo del cursor, o arriba si no entra; siempre dentro de la ventana.
pub(super) fn tooltip_rect(w: f32, h: f32, x: f32, y: f32, text_w: f32) -> Rect {
    let bw = (text_w + PAD * 2.0).min(w - EDGE * 2.0).max(0.0);
    let bh = LINE_H + PAD * 2.0;
    let left = x.min(w - EDGE - bw).max(EDGE.min(w - bw).max(0.0));
    let mut top = y + BELOW;
    if top + bh > h - EDGE {
        top = y - bh - EDGE;
    }
    let top = top.min(h - bh).max(0.0);
    geom::rect(left, top, left + bw, top + bh)
}

/// Dibuja el cartel con el mouse en `(x, y)`.
pub(super) fn draw_tooltip(p: &dyn Painter, w: f32, h: f32, x: f32, y: f32, text: &str) {
    let th = p.theme();
    let r = tooltip_rect(w, h, x, y, p.measure(FontId::Small, text));
    p.round_fill(r, 4.0, th.surface_alt);
    p.round_stroke(r, 4.0, th.border);
    p.text(FontId::Small, text, geom::rect(r.left + PAD, r.top + PAD, r.right - PAD, r.bottom - PAD), th.text);
}

impl App {
    /// Texto del cartel de `hit` en esta ventana. Sin cartel para el botón cuyo menú está abierto.
    pub(super) fn tooltip_text(&self, hit: Hit) -> Option<String> {
        match hit {
            Hit::Gear if self.picker_open => return None,
            Hit::Pane(i, PaneHit::Terminal) if self.term_menu == Some(i) => return None,
            // El botón «xtree» dice qué hace; el cartel lleva el estado del escaneo, si lo hay.
            Hit::Xtree => return self.xtree.as_ref().and_then(|x| x.badge_tip()),
            _ => {}
        }
        let tab_path = |p: usize, t: usize| {
            let location = &self.panes.get(p)?.tabs.get(t)?.location;
            Some(match location.target() {
                Some(path) => path.display().to_string(),
                None => location.title(),
            })
        };
        tooltip_for(hit, &self.config.keymap, &tab_path, self.tree_pinned())
    }

    /// El mouse se movió a `(x, y)`, sobre `hover`: arranca la espera del cartel de un botón, lo
    /// cambia enseguida si ya había uno a la vista, o lo saca. Devuelve si hay que redibujar.
    pub(super) fn update_tooltip(&mut self, hover: Hit, x: f32, y: f32) -> bool {
        let was_visible = self.tip.is_some_and(|t| t.state == TipState::Visible);
        // Botón apretado (Atrás, Adelante…): es un clic, no un arrastre; el cartel sigue descartado
        // hasta que el mouse salga del botón.
        if matches!(self.drag, Drag::Button(..)) {
            return false;
        }
        if self.drag != Drag::None {
            self.tip = None;
            return was_visible;
        }
        if self.tip.is_some_and(|t| t.hit == hover) {
            // Sigue sobre el mismo botón: mientras espera, el cartel va donde está el mouse.
            if let Some(t) = self.tip.as_mut().filter(|t| t.state == TipState::Pending) {
                (t.x, t.y) = (x, y);
            }
            return false;
        }
        if self.tooltip_text(hover).is_none() {
            self.tip = None;
            return was_visible;
        }
        self.tip_token += 1;
        let token = self.tip_token;
        let state = if was_visible { TipState::Visible } else { TipState::Pending };
        self.tip = Some(Tip { hit: hover, x, y, state, token });
        if state == TipState::Pending {
            let hwnd = self.hwnd;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(TOOLTIP_DELAY_MS));
                hwnd.post(WM_APP_TOOLTIP, 0, token as isize);
            });
        }
        was_visible
    }

    /// Pasó la espera: si el mouse sigue sobre el mismo botón, aparece su cartel.
    pub(super) fn on_tooltip_due(&mut self, token: u64) {
        if let Some(t) = self.tip.as_mut().filter(|t| t.token == token && t.state == TipState::Pending) {
            t.state = TipState::Visible;
            self.redraw();
        }
    }

    /// Un clic, una tecla, la rueda o un menú: el cartel se va y no vuelve hasta pasar a otra cosa.
    pub(super) fn dismiss_tooltip(&mut self) {
        if let Some(t) = &mut self.tip {
            if t.state == TipState::Visible {
                self.host.invalidate();
            }
            t.state = TipState::Dismissed;
        }
    }

    /// El mouse salió de la ventana: el cartel se va.
    pub(super) fn hide_tooltip(&mut self) {
        self.tip = None;
    }

    /// El cartel a la vista: (texto, punto del mouse).
    pub(super) fn visible_tooltip(&self) -> Option<(String, f32, f32)> {
        let t = self.tip.filter(|t| t.state == TipState::Visible)?;
        Some((self.tooltip_text(t.hit)?, t.x, t.y))
    }
}

//! La pestaña Novedades desde la `App`: abrirla (una sola, como la configuración), dibujarla,
//! desplazarla con la rueda y el teclado, y el aviso de la barra de estado al actualizar.

use super::*;
use crate::changelog_draw::{self, ChangelogView};
use crate::keys::keys::{VK_DOWN, VK_END, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP};
use crate::paint::Painter;

impl App {
    /// Dónde está la pestaña Novedades: (panel, pestaña).
    pub(super) fn changelog_tab(&self) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(i, p)| p.tabs.iter().position(|t| t.is_changelog()).map(|j| (i, j)))
    }

    /// La vista de Novedades del panel, si es la pestaña que está a la vista.
    pub(super) fn changelog_view_mut(&mut self, pane: usize) -> Option<&mut ChangelogView> {
        self.panes.get_mut(pane)?.tab_mut().changelog.as_mut()
    }

    /// Abre Novedades: si ya hay una pestaña (en cualquier panel), la activa con su panel; si no,
    /// abre una nueva en el panel activo (no reemplaza la carpeta). `version`: la que se quiere
    /// arriba (desde el aviso de actualización). Abrirla quita el aviso.
    pub(super) fn open_changelog(&mut self, version: Option<&str>) {
        self.picker_open = false;
        self.about_open = false;
        self.term_menu = None;
        self.update_notice = None;
        if self.xtree_open {
            self.close_xtree();
        }
        match self.changelog_tab() {
            // En un panel que el layout no muestra: se trae al activo.
            Some((pane, index)) if !self.shown_panes().contains(&pane) => {
                let at = self.panes[self.active].tabs.len();
                self.move_tab_to_pane(pane, index, self.active, at);
            }
            Some((pane, index)) => {
                self.set_active(pane);
                self.select_tab(pane, index);
            }
            None => self.new_tab(self.active, Location::Changelog, true),
        }
        if let Some(v) = version
            && let Some(view) = self.changelog_view_mut(self.active)
        {
            view.target = Some(v.to_string());
        }
        self.redraw();
    }

    /// Dibuja la pestaña Novedades del panel en el lugar de su lista (después de `Pane::draw`).
    pub(super) fn draw_changelog(&mut self, p: &dyn Painter, pane: usize) {
        let r = self.panes[pane].list;
        if let Some(view) = self.changelog_view_mut(pane) {
            changelog_draw::draw(p, r, changelog_draw::bundled(), view);
        }
    }

    /// La rueda sobre la pestaña Novedades: mueve el contenido.
    pub(super) fn changelog_wheel(&mut self, pane: usize, delta: f32) {
        if let Some(v) = self.changelog_view_mut(pane)
            && v.scroll_by(-delta * changelog_draw::STEP / 2.0)
        {
            self.redraw();
        }
    }

    /// Las teclas de desplazamiento con Novedades a la vista en el panel activo: flechas,
    /// RePág / AvPág, Inicio y Fin. `None` si no es para ella.
    pub(super) fn changelog_key(&mut self, vk: Key) -> Option<bool> {
        let active = self.active;
        let v = self.changelog_view_mut(active)?;
        let dy = match vk {
            VK_UP => -changelog_draw::STEP,
            VK_DOWN => changelog_draw::STEP,
            VK_PRIOR => -v.page(),
            VK_NEXT => v.page(),
            VK_HOME => -f32::MAX / 4.0,
            VK_END => f32::MAX / 4.0,
            _ => return None,
        };
        if v.scroll_by(dy) {
            self.redraw();
        }
        Some(true)
    }

    /// Clic en «Ver novedades»: abre la pestaña en la versión nueva.
    pub(super) fn open_update_notice(&mut self) {
        let version = self.update_notice.clone();
        self.open_changelog(version.as_deref());
    }
}

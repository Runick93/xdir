//! La pestaña Configuración desde la `App`: abrirla (una sola), su página, el dibujo de cada
//! cuadro, los clics y la rueda.

use super::settings_pages::PAGES;
use super::*;
use crate::paint::Painter;
use crate::settings::draw::NavItem;
use crate::settings::{Item, PageId, Section, SettingsHit};

impl App {
    /// Las páginas de la lista de la izquierda: las secciones propias con las extra (Red) después
    /// de la que dicen.
    pub(super) fn settings_nav(&self) -> Vec<PageId> {
        let mut out = Vec::new();
        for s in Section::ALL {
            out.push(PageId::Builtin(s));
            out.extend(PAGES.iter().filter(|p| p.after == s).map(|p| PageId::Extra(p.id)));
        }
        out
    }

    /// Nombre de una página.
    pub(super) fn page_title(page: PageId) -> &'static str {
        match page {
            PageId::Builtin(s) => s.title(),
            PageId::Extra(id) => PAGES.iter().find(|p| p.id == id).map_or("", |p| p.title),
        }
    }

    /// La página de un ancla de la sesión (`atajos`, `red`); desconocida → Apariencia.
    pub(super) fn page_from_id(id: &str) -> PageId {
        if let Some(s) = Section::from_id(id) {
            return PageId::Builtin(s);
        }
        PAGES.iter().find(|p| p.id == id).map_or(PageId::default(), |p| PageId::Extra(p.id))
    }

    /// El ancla de la sesión de una página.
    pub(super) fn page_id(page: PageId) -> &'static str {
        match page {
            PageId::Builtin(s) => s.id(),
            PageId::Extra(id) => id,
        }
    }

    /// Dónde está la pestaña Configuración: (panel, pestaña).
    pub(super) fn settings_tab(&self) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(i, p)| p.tabs.iter().position(|t| t.is_settings()).map(|j| (i, j)))
    }

    /// La vista de la pestaña Configuración del panel, si es la que está a la vista.
    pub(super) fn settings_view(&self, pane: usize) -> Option<&crate::settings::SettingsView> {
        self.panes.get(pane)?.tab().settings.as_ref()
    }

    pub(super) fn settings_view_mut(&mut self, pane: usize) -> Option<&mut crate::settings::SettingsView> {
        self.panes.get_mut(pane)?.tab_mut().settings.as_mut()
    }

    /// Abre la configuración: si ya hay una pestaña (en cualquier panel), la activa con su panel;
    /// si no, abre una nueva en el panel activo (no reemplaza la carpeta). `page`: la página.
    pub(super) fn open_settings(&mut self, page: Option<PageId>) {
        self.picker_open = false;
        self.term_menu = None;
        if self.xtree_open {
            self.close_xtree();
        }
        match self.settings_tab() {
            // En un panel que el layout no muestra: se trae al activo.
            Some((pane, index)) if !self.shown_panes().contains(&pane) => {
                let at = self.panes[self.active].tabs.len();
                self.move_tab_to_pane(pane, index, self.active, at);
            }
            Some((pane, index)) => {
                self.set_active(pane);
                self.select_tab(pane, index);
            }
            None => self.new_tab(self.active, Location::Settings, true),
        }
        if let Some(page) = page
            && let Some(v) = self.settings_view_mut(self.active)
        {
            v.set_page(page);
        }
        self.redraw();
    }

    /// Lo que va en el contenido de una página propia (las opciones con sus valores de ahora).
    pub(super) fn settings_items(&self, _section: Section) -> Vec<Item> {
        Vec::new()
    }

    /// Dibuja la pestaña Configuración del panel en el lugar de su lista (después de `Pane::draw`).
    pub(super) fn draw_settings(&mut self, p: &dyn Painter, pane: usize) {
        let r = self.panes[pane].list;
        let hover = match self.hover {
            Hit::Settings(j, h) if j == pane => Some(h),
            _ => None,
        };
        let nav: Vec<NavItem> = self.settings_nav().into_iter().map(|page| NavItem { page, title: Self::page_title(page).to_string(), count: None }).collect();
        let Some(view) = self.settings_view_mut(pane) else { return };
        let page = view.page;
        let title = Self::page_title(page);
        let content = crate::settings::draw::frame(p, r, &nav, Some(page), title, hover, view);
        match page {
            PageId::Builtin(section) => {
                let items = self.settings_items(section);
                if let Some(view) = self.settings_view_mut(pane) {
                    crate::settings::draw::items(p, content, &items, hover, view);
                }
            }
            PageId::Extra(id) => {
                let Some(extra) = PAGES.iter().find(|e| e.id == id) else { return };
                let scroll = self.settings_view(pane).map_or(0.0, |v| v.scroll);
                let (mx, my) = self.cursor_pos();
                let mouse = geom::contains(&content, mx, my).then_some((mx, my));
                let shifted = geom::rect(content.left, content.top - scroll, content.right, content.bottom);
                let mut h = 0.0;
                p.clip(content, &mut || h = (extra.draw)(self, p, shifted, mouse));
                if let Some(view) = self.settings_view_mut(pane) {
                    view.content_h = h;
                    view.view_h = content.bottom - content.top;
                    view.hits.push((content, SettingsHit::Extra));
                }
            }
        }
    }

    /// Lo que hay bajo `(x, y)` en la pestaña Configuración del panel (según el último cuadro).
    pub(super) fn settings_hit(&self, pane: usize, x: f32, y: f32) -> Option<SettingsHit> {
        let v = self.settings_view(pane)?;
        Some(v.hit_at(x, y).unwrap_or(SettingsHit::Body))
    }

    /// Un clic en la pestaña Configuración.
    pub(super) fn settings_click(&mut self, pane: usize, hit: SettingsHit, x: f32, y: f32) {
        match hit {
            SettingsHit::Nav(page) => {
                // Con una búsqueda, elegir una sección la termina y va a esa sección.
                if let Some(v) = self.settings_view_mut(pane) {
                    v.set_page(page);
                }
                if !self.panes[pane].tab().search_query().is_empty() {
                    self.clear_filter(pane);
                }
            }
            SettingsHit::Extra => {
                if let Some(PageId::Extra(id)) = self.settings_view(pane).map(|v| v.page)
                    && let Some(extra) = PAGES.iter().find(|e| e.id == id)
                {
                    (extra.click)(self, x, y);
                }
            }
            SettingsHit::Part(id, part) => self.settings_part(pane, id, part, x),
            SettingsHit::Body => {}
        }
        self.redraw();
    }

    /// Un clic en una parte de una fila.
    pub(super) fn settings_part(&mut self, _pane: usize, _id: &'static str, _part: usize, _x: f32) {}

    /// La rueda sobre la pestaña Configuración: mueve el contenido.
    pub(super) fn settings_wheel(&mut self, pane: usize, delta: f32) {
        if let Some(v) = self.settings_view_mut(pane)
            && v.scroll_by(-delta * 20.0)
        {
            self.redraw();
        }
    }

    /// Terminó un campo (`WM_APP_FIELD_DONE`): si es de una página extra, lo atiende ella.
    pub(super) fn settings_field_done(&mut self, commit: bool) -> bool {
        let Some(b) = self.settings_field else { return false };
        PAGES.iter().any(|p| (p.field_done)(self, b, commit))
    }

    /// La página de la pestaña Configuración para la sesión.
    pub(super) fn settings_session_page(&self) -> Option<String> {
        let (pane, index) = self.settings_tab()?;
        let page = self.panes[pane].tabs[index].settings.as_ref()?.page;
        Some(Self::page_id(page).to_string())
    }
}

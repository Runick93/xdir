//! Dibujo de la ventana.

use super::*;

impl App {
    // ---------- dibujo ----------

    pub(super) fn all_listed(&self) -> bool {
        self.panes[..self.layout.panes()].iter().all(|p| p.tab().is_loaded() && !p.tab().loading)
    }

    pub fn paint(&mut self) {
        if self.drag == Drag::None {
            self.autosave_group();
        }
        self.sync_folder_views();
        let title = match &self.group_name {
            Some(name) => format!("xdir - {name}"),
            None => "xdir".to_string(),
        };
        if title != self.title {
            self.host.set_title(&title);
            self.title = title;
        }

        let first_frame = self.first_frame_ms.is_none();
        if first_frame {
            self.marks.push(("WM_PAINT", ms_since(self.started)));
        }
        let listed = self.all_listed();
        let mark_list_frame = listed && !self.list_frame_done;

        // Lo elegido cambió: se corta el cálculo del tamaño anterior y arranca el nuevo.
        self.sync_sel_size();
        let mut status = match self.xtree.as_ref().filter(|_| self.xtree_open) {
            Some(v) => v.status_text(),
            None => self.status_text(),
        };
        // Carpetas de red elegidas: el texto termina en el enlace «calcular tamaño».
        let size_link = !self.xtree_open && self.sel_size.total() == crate::tab::SelTotal::NeedsClick;
        if let Some(n) = &self.notice {
            status = format!("{n}   ·   {status}");
        }
        // Expulsando o desconectando algo: delante de todo, mientras dure.
        if let Some(e) = self.eject_status() {
            status = format!("{e}   ·   {status}");
        }
        // El aviso de actualización va delante (con su enlace); otro aviso lo tapa mientras está.
        let update_notice = self.update_notice.clone().filter(|_| self.notice.is_none());
        if update_notice.is_none() {
            self.update_link = None;
        }
        let xtree_badge = self.xtree_badge();
        let transfers = self.transfers_progress();
        let transfers_failed = self.transfers_failed();
        // El ancho del árbol se anima al colapsar o expandir: se avanza antes de calcular el layout.
        let now = Instant::now();
        let dt = self.last_frame.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        #[cfg(feature = "arbol")]
        self.follow_tree();
        #[cfg(feature = "arbol")]
        let tree_animating = self.tree.tick(dt);
        #[cfg(not(feature = "arbol"))]
        let tree_animating = false;
        let tree_w = self.tree_width();
        let groups_h = self.groups_bar_h();
        // La columna de vista previa: lo que muestra (y su pedido, después de la pausa).
        self.sync_preview();
        self.sync_gallery_stamps();
        let preview_w = self.preview_width();
        let preview_col = self.preview_column();
        // El lugar de la derecha es de la vista previa, de Renombrar varios o del panel de
        // información (armado acá: el dibujo y los clics usan lo mismo).
        let rename_panel = self.rename_panel_state();
        let rename_hover = match self.hover {
            Hit::RenamePanel(h) => Some(h),
            _ => None,
        };
        self.sync_info();
        let info_shown = self.info_open();
        if let Some(r) = preview_col.filter(|_| info_shown) {
            self.layout_info(r);
        }
        let preview_shown =
            preview_col.filter(|_| !info_shown && rename_panel.is_none()).map(|_| self.preview_shown());
        let info_hover = match self.hover {
            Hit::Info(h) => Some(h),
            _ => None,
        };
        let preview_forget = std::mem::take(&mut self.preview.forget);
        let preview_on = self.preview.on;
        #[cfg(feature = "arbol")]
        let current_folder = self.panes[self.active].tab().location.target();
        // Animaciones: rectángulos de este frame (transición de layout) y scroll suave de las
        // listas a la vista. Mientras algo siga en curso se pide otro frame más abajo.
        let (cw, ch) = self.client_dips();
        let target = ui::visible_rects(self.layout, self.maximized, ui::panes_area(cw, ch, tree_w, groups_h, preview_w), self.splits);
        let (rects, mut animating) = self.animated_rects(target);
        animating |= tree_animating;
        let ejecting = self.ejecting_roots();
        #[cfg(feature = "arbol")]
        {
            self.tree.ejecting = ejecting.iter().cloned().collect();
        }
        for i in self.shown_panes() {
            animating |= self.panes[i].tab_mut().list.tick(dt);
            // Botón «Devolver» con la Papelera a la vista (apagado sin selección).
            let tab = self.panes[i].tab();
            let restore = sys::file_system().is_trash(&tab.location).then(|| tab.list.selected_indices().next().is_some());
            self.panes[i].restore = restore;
            // ⏏ en los discos expulsables de «Este equipo».
            let tab = self.panes[i].tab();
            let ejects: Vec<usize> = match tab.location {
                // Los discos y las conexiones de red (`Network::connections`).
                Location::Computer => (0..tab.list.entries.len())
                    .filter(|&k| {
                        let p = tab.entry_path(k);
                        self.ejectable.contains(&p) || self.net.connections.iter().any(|c| c.connected && crate::sys::same_path_folded(&c.local, &p))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let busy = ejects.iter().copied().filter(|&k| ejecting.iter().any(|r| crate::sys::same_path_folded(r, &tab.entry_path(k)))).collect();
            let list = &mut self.panes[i].tab_mut().list;
            list.eject_rows = ejects;
            list.eject_busy = busy;
        }
        self.last_frame = animating.then_some(now);
        for p in &mut self.panes {
            p.frame_dt = dt;
        }
        if let Some(g) = &mut self.ghost {
            animating |= g.step(dt);
        }
        let ghost = self.ghost.as_ref().map(|g| (g.title.clone(), geom::rect(g.x, g.y, g.x + g.w, g.y + pane::TABSTRIP_H - 3.0), g.detach));
        // El texto del cartel de ayuda se arma antes de prestar los campos de `App` por separado.
        let tooltip = self.visible_tooltip();
        let switches = self.picker_switches();
        // «Conectar a un servidor»: las cajas en su lugar y lo que se dibuja, antes de prestar
        // los campos por separado.
        self.sync_connect_boxes();
        self.sync_map_boxes();
        let drives_style = self.net_style();
        self.update_placeholders();
        self.sync_browse();
        self.sync_red_page();
        let connect_extra = self.net.dialog.as_ref().map(|_| {
            let style = self.net_style();
            let (now, offset) = (super::network::unix_now(), sys::env().local_now().offset);
            let dates: Vec<String> = self.net.places.recents.iter().map(|r| crate::net::places::ago(now, r.date, offset)).collect();
            let fav = crate::net::Address::parse(&self.connect_field_text()).is_ok_and(|a| self.net.places.is_favorite(&a));
            (style, self.net_example(), dates, fav)
        });
        // Las migas de la configuración dicen la página.
        for i in self.shown_panes() {
            let page = self.panes[i].tab().settings.as_ref().map(|v| v.page);
            if let Some(page) = page
                && let Some(v) = self.settings_view_mut(i)
            {
                v.page_title = Self::page_title(page).to_string();
            }
        }
        let (layout, splits, active, maximized) = (self.layout, self.splits, self.active, self.maximized);
        let (hover, drag, edit_pane) = (self.hover, self.drag, self.edit.map(|(_, p)| p));
        let (show_favorites, fav_drop) = (self.show_favorites, self.fav_drop);
        let (picker_open, accent) = (self.picker_open, self.accent);
        let about_open = self.about_open;
        let term_menu = self.term_menu;
        let tab_drop = self.tab_drop;
        let filter_pane = self.filter_edit.map(|(_, p)| p);
        // Vista de xtree abierta: se dibuja en lugar de los paneles, la franja de grupos y el árbol.
        let xtree_open = self.xtree_open;
        let started = self.started;
        // El host dibuja con una copia del tema y de las reglas de color: así la clausura puede
        // tomar la `App` entera (la configuración dibuja con ella).
        let host = self.host.clone();
        let theme = self.theme;
        let colors = if self.file_colors { self.config.colors.clone() } else { Vec::new() };
        let result = host.draw(&theme, &colors, &mut |p, w, h| {
            if first_frame {
                self.marks.push(("render target", ms_since(started)));
            }
            if mark_list_frame {
                self.marks.push(("inicio frame con la carpeta", ms_since(started)));
            }
            let area = ui::panes_area(w, h, tree_w, groups_h, preview_w);
            for src in &preview_forget {
                p.forget_big(src);
            }
            let xtree_shown = xtree_open && self.xtree.is_some();
            let rects = if xtree_shown { &[][..] } else { &rects[..] };
            if let Some(v) = self.xtree.as_mut().filter(|_| xtree_open) {
                v.paint(p, ui::xtree_area(w, h));
            } else if groups_h > 0.0 {
                let names: Vec<(&str, bool)> = self.groups.items.iter().map(|g| (g.name.as_str(), g.temp)).collect();
                let active_group = self.group_name.as_deref().and_then(|n| self.groups.find(n));
                let group_hover = match hover {
                    Hit::GroupBar(g) => Some(g),
                    _ => None,
                };
                ui::draw_group_bar(p, w - preview_w, area.left, &names, active_group, group_hover);
            }
            // Botón de maximizar solo si hay más de un panel; encendido en el maximizado.
            let max_button = |i: usize| (layout.panes() > 1).then_some(maximized == Some(i));
            for &(i, r) in rects {
                let ph = match hover {
                    Hit::Pane(j, ph) if j == i => ph,
                    _ => PaneHit::None,
                };
                let pressed = match drag {
                    Drag::Button(j, b) if j == i => PaneHit::Button(b),
                    _ => PaneHit::None,
                };
                let favs = show_favorites.then(|| (&self.favorites, fav_drop.filter(|(j, _)| *j == i).map(|(_, d)| d)));
                let tab = self.panes[i].tab();
                let query = tab.search_query().to_string();
                let is_settings = tab.is_settings();
                let is_changelog = tab.is_changelog();
                let search = SearchView {
                    text: &query,
                    editing: filter_pane == Some(i),
                    running: tab.search.as_ref().filter(|s| !s.done).map(|s| s.progress),
                    // Las novedades no tienen buscador.
                    enabled: !is_changelog,
                    placeholder: if is_settings { "Buscar en la configuración" } else { "Buscar" },
                };
                self.panes[i].draw(p, r, i == active, ph, pressed, edit_pane == Some(i), favs, term_menu == Some(i), max_button(i), search);
                if is_settings {
                    self.draw_settings(p, i);
                }
                if is_changelog {
                    self.draw_changelog(p, i);
                }
                if let Some((_, at)) = tab_drop.filter(|(j, _)| *j == i) {
                    self.panes[i].draw_tab_drop(p, at);
                }
            }
            // El árbol ocupa su lugar a la izquierda de los paneles: fijo, con su ancho; suelto, una
            // franja de íconos que al asomar el mouse se expande encima de los paneles (con sombra).
            #[cfg(feature = "arbol")]
            if self.tree.visible && !xtree_open {
                let tree = &mut self.tree;
                let r = geom::rect(ui::GAP, area.top - groups_h, ui::GAP + tree.width_now(), area.bottom);
                if !tree.pinned && tree.width_now() > tree::COLLAPSED_W + 1.0 {
                    p.round_fill(geom::rect(r.left + 2.0, r.top + 2.0, r.right + 6.0, r.bottom + 4.0), 12.0, theme::alpha(p.theme().background, 0.45));
                }
                tree.draw(p, r, current_folder.as_deref(), xtree_badge);
            }
            if let (Some(r), Some(shown)) = (preview_col, &preview_shown) {
                crate::preview::draw::draw(p, r, shown);
            }
            if let (Some(r), Some(st)) = (preview_col, &rename_panel) {
                crate::rename_many_draw::draw(p, r, st, rename_hover);
            }
            if info_shown && preview_col.is_some() {
                crate::info::draw::draw(p, &self.info.layout, info_hover);
            }
            if let Some((title, r, detach)) = &ghost {
                pane::Pane::draw_ghost(p, *r, title, *detach);
            }
            let hot = match (drag, hover) {
                (Drag::Splitter(s), _) | (_, Hit::Splitter(s)) => Some(s),
                _ => None,
            };
            // El borde de la columna resaltado bajo el mouse o mientras se arrastra.
            if let Some(r) = preview_col.filter(|_| matches!(drag, Drag::PreviewEdge(_)) || hover == Hit::PreviewEdge) {
                ui::draw_splitter(p, geom::rect(r.left - ui::GAP, r.top, r.left, r.bottom));
            }
            if let Some(s) = hot.filter(|_| maximized.is_none() && !xtree_open) {
                for (which, r) in ui::splitter_rects(layout, area, splits) {
                    if which == s {
                        ui::draw_splitter(p, r);
                    }
                }
            }
            let layout_hover = match hover {
                Hit::Layout(l) => Some(l),
                _ => None,
            };
            let buttons = ui::StatusButtons {
                gear_hot: picker_open || hover == Hit::Gear,
                groups_hot: hover == Hit::Groups,
                xtree_hot: hover == Hit::Xtree,
                xtree_on: xtree_open,
                xtree: xtree_badge,
                preview_hot: hover == Hit::PreviewToggle,
                preview_on: preview_on && !xtree_open,
                transfers,
                transfers_failed,
                transfers_hot: hover == Hit::Transfers,
                lead: 0.0,
                size_link,
                size_link_hot: hover == Hit::SizeLink,
            };
            let mut buttons = buttons;
            if let Some(v) = &update_notice {
                let (link, lead) = ui::draw_update_notice(p, h, v, hover == Hit::UpdateNotice);
                self.update_link = Some(link);
                buttons.lead = lead;
            }
            self.size_link = ui::draw_status(p, w, h, &status, layout, layout_hover, buttons);
            // Desplegable de filtros de una lista: encima de los paneles y de la barra de estado
            // (entra en el área de su lista; en un panel muy bajo puede pasarse).
            for &(i, _) in rects {
                self.panes[i].tab().list.draw_filter_menu(p);
            }
            if let Some(pane) = term_menu {
                let term_hover = match hover {
                    Hit::TermMenu(c) => c,
                    _ => None,
                };
                ui::draw_terminal_menu(p, self.panes[pane].terminal_rect(), w, term_hover);
            }
            if picker_open {
                let picker_hover = match hover {
                    Hit::Picker(k) => Some(k),
                    _ => None,
                };
                ui::draw_picker(p, w, h, accent, picker_hover, switches);
            }
            // Vista rápida sin visor del sistema: la tarjeta de xdir encima de los paneles.
            if let Some(st) = self.quick_look.as_ref().filter(|s| !s.ql.native) {
                quick_look::paint_quick_look(p, w, h, st, hover);
            }
            if let Some(sg) = &self.suggest {
                sg.draw(p);
            }
            if let Some((pal, _)) = &self.palette {
                pal.draw(p, w);
            }
            if about_open {
                let about_hover = match hover {
                    Hit::About(k) => Some(k),
                    _ => None,
                };
                about::draw(p, w, h, about_hover);
            }
            if let (Some(d), Some((style, example, dates, fav))) = (&self.net.dialog, &connect_extra) {
                let places = &self.net.places;
                let hover = match hover {
                    Hit::Connect(k) => Some(k),
                    _ => None,
                };
                let args = crate::net::dialog::DrawArgs { style: *style, example, recent_dates: dates, is_favorite: *fav, hover };
                crate::net::dialog::draw(p, w, h, d, places, &args);
            }
            let drives_hover = match hover {
                Hit::Drives(k) => Some(k),
                _ => None,
            };
            if let Some(m) = &self.net.map {
                crate::net::drives_dialog::map_draw(p, w, h, m, drives_hover);
            }
            if let Some(u) = &self.net.unmap {
                crate::net::drives_dialog::unmap_draw(p, w, h, u, drives_style, drives_hover);
            }
            // El cartel de ayuda va último: encima de todo.
            if let Some((text, x, y)) = &tooltip {
                tooltip::draw_tooltip(p, w, h, *x, *y, text);
            }
        });
        if result.is_err() {
            return;
        }
        // Pestañas deslizándose hacia su lugar, o una búsqueda en curso (su indicador gira): otro cuadro.
        let searching = self.shown_panes().into_iter().any(|i| {
            let tab = self.panes[i].tab();
            // También mientras se lee un archivo comprimido (su «Cargando…» gira).
            tab.search.as_ref().is_some_and(|s| !s.done) || (tab.loading && tab.location.is_archive())
        });
        // Un ⏏ girando (expulsando o desconectando).
        let searching = searching || !ejecting.is_empty();
        if self.panes.iter().any(|p| p.tab_busy) || searching {
            animating = true;
            self.last_frame = Some(now);
        }
        if animating {
            self.redraw();
        }
        self.notify_a11y_focus();
        // Superposiciones y metadatos de las filas recién dibujadas.
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for i in self.shown_panes() {
            self.panes[i].tab_mut().request_overlays(hwnd, &tx);
            self.panes[i].tab_mut().request_metadata(hwnd, &tx);
        }

        if self.first_frame_ms.is_none() {
            let ms = ms_since(self.started);
            self.first_frame_ms = Some(ms);
            self.marks.push(("primer frame", ms));
        }
        if listed && !self.list_frame_done {
            self.list_frame_done = true;
            self.marks.push(("primer frame con la carpeta", ms_since(self.started)));
            if let Some(path) = &self.probe {
                let total = self.marks.last().unwrap().1;
                let mut out = format!(
                    "first_frame_ms={:.2}\nlisted_frame_ms={total:.2}\nitems={}\nui_load_ms={:.2}\ndpi={}\n",
                    self.first_frame_ms.unwrap(),
                    self.panes[self.active].tab().list.count(),
                    self.ui_load_ms,
                    self.dpi
                );
                for (name, t) in &self.marks {
                    out += &format!("  {t:7.2}  {name}\n");
                }
                let _ = std::fs::write(path, out);
                self.host.request_close();
            }
        }
    }
}

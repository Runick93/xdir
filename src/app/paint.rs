//! Dibujo de la ventana.

use super::*;

impl App {
    // ---------- dibujo ----------

    pub(super) fn all_listed(&self) -> bool {
        self.panes[..self.layout.panes()].iter().all(|p| p.tab().is_loaded() && !p.tab().loading)
    }

    pub(super) fn paint(&mut self) {
        if self.drag == Drag::None {
            self.autosave_group();
        }
        let title = match &self.group_name {
            Some(name) => format!("xdir - {name}"),
            None => "xdir".to_string(),
        };
        if title != self.title {
            let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let _ = SetWindowTextW(self.hwnd.hwnd(), PCWSTR(wide.as_ptr()));
            }
            self.title = title;
        }

        if self.first_frame_ms.is_none() {
            self.marks.push(("WM_PAINT", ms_since(self.started)));
            if self.renderer.ensure_target(self.hwnd.hwnd(), self.dpi).is_ok() {
                self.marks.push(("render target", ms_since(self.started)));
            }
        }
        let listed = self.all_listed();
        if listed && !self.list_frame_done {
            self.marks.push(("inicio frame con la carpeta", ms_since(self.started)));
        }

        let mut status = self.panes[self.active].tab().status();
        if let Some(n) = &self.notice {
            status = format!("{n}   ·   {status}");
        }
        // El ancho del árbol se anima al colapsar o expandir: se avanza antes de calcular el layout.
        let now = Instant::now();
        let dt = self.last_frame.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        #[cfg(feature = "arbol")]
        let tree_animating = self.tree.tick(dt);
        #[cfg(not(feature = "arbol"))]
        let tree_animating = false;
        let tree_w = self.tree_width();
        let groups_h = self.groups_bar_h();
        let tree_on = self.tree_visible();
        #[cfg(feature = "arbol")]
        let current_folder = self.panes[self.active].tab().location.target();
        // Animaciones: rectángulos de este frame (transición de layout) y scroll suave de las
        // listas a la vista. Mientras algo siga en curso se pide otro frame más abajo.
        let (cw, ch) = self.client_dips();
        let target = ui::visible_rects(self.layout, self.maximized, ui::panes_area(cw, ch, tree_w, groups_h), self.splits);
        let (rects, mut animating) = self.animated_rects(target);
        animating |= tree_animating;
        for i in self.shown_panes() {
            animating |= self.panes[i].tab_mut().list.tick(dt);
        }
        self.last_frame = animating.then_some(now);
        for p in &mut self.panes {
            p.frame_dt = dt;
        }
        if let Some(g) = &mut self.ghost {
            animating |= g.step(dt);
        }
        let ghost = self.ghost.as_ref().map(|g| (g.title.clone(), render::rect(g.x, g.y, g.x + g.w, g.y + pane::TABSTRIP_H - 3.0), g.detach));
        #[cfg(feature = "arbol")]
        let tree = &mut self.tree;
        let (panes, layout, splits, active, maximized) = (&mut self.panes, self.layout, self.splits, self.active, self.maximized);
        let (hover, drag, edit_pane) = (self.hover, self.drag, self.edit.map(|(_, p)| p));
        let (favorites, show_favorites, fav_drop) = (&self.favorites, self.show_favorites, self.fav_drop);
        let (picker_open, accent) = (self.picker_open, self.accent);
        let about_open = self.about_open;
        let file_colors = self.file_colors;
        let (groups, group_name, show_group_bar) = (&self.groups, &self.group_name, self.show_group_bar);
        let term_menu = self.term_menu;
        let tab_drop = self.tab_drop;
        let filter_pane = self.filter_edit.map(|(_, p)| p);
        let palette = self.palette.as_ref().map(|(p, _)| p);
        let suggest = self.suggest.as_ref();
        let result = self.renderer.draw(self.hwnd.hwnd(), self.dpi, &self.theme, if self.file_colors { &self.config.colors } else { &[] }, |p, w, h| {
            let area = ui::panes_area(w, h, tree_w, groups_h);
            if groups_h > 0.0 {
                let names: Vec<(&str, bool)> = groups.items.iter().map(|g| (g.name.as_str(), g.temp)).collect();
                let active_group = group_name.as_deref().and_then(|n| groups.find(n));
                let group_hover = match hover {
                    Hit::GroupBar(g) => Some(g),
                    _ => None,
                };
                ui::draw_group_bar(p, w, &names, active_group, group_hover);
            }
            // Botón de maximizar solo si hay más de un panel; encendido en el maximizado.
            let max_button = |i: usize| (layout.panes() > 1).then_some(maximized == Some(i));
            for &(i, r) in &rects {
                let ph = match hover {
                    Hit::Pane(j, ph) if j == i => ph,
                    _ => PaneHit::None,
                };
                let pressed = match drag {
                    Drag::Button(j, b) if j == i => PaneHit::Button(b),
                    _ => PaneHit::None,
                };
                let favs = show_favorites.then(|| (favorites, fav_drop.filter(|(j, _)| *j == i).map(|(_, d)| d)));
                // La fila del filtro se ve mientras se edita o si hay uno aplicado.
                let list = &panes[i].tab().list;
                let editing_filter = filter_pane == Some(i);
                let filter_text = list.filter().to_string();
                let filter = (editing_filter || !filter_text.is_empty()).then(|| FilterView {
                    text: &filter_text,
                    shown: list.count(),
                    total: list.unfiltered_count(),
                    editing: editing_filter,
                });
                panes[i].draw(p, r, i == active, ph, pressed, edit_pane == Some(i), favs, term_menu == Some(i), max_button(i), filter);
                if let Some((_, at)) = tab_drop.filter(|(j, _)| *j == i) {
                    panes[i].draw_tab_drop(p, at);
                }
            }
            // El árbol ocupa su lugar a la izquierda de los paneles: fijo, con su ancho; suelto, una
            // franja de íconos que al asomar el mouse se expande encima de los paneles (con sombra).
            #[cfg(feature = "arbol")]
            if tree.visible {
                let r = render::rect(ui::GAP, area.top, ui::GAP + tree.width_now(), area.bottom);
                if !tree.pinned && tree.width_now() > tree::COLLAPSED_W + 1.0 {
                    p.round_fill(render::rect(r.left + 2.0, r.top + 2.0, r.right + 6.0, r.bottom + 4.0), 12.0, theme::alpha(p.theme.background, 0.45));
                }
                tree.draw(p, r, current_folder.as_deref());
            }
            if let Some((title, r, detach)) = &ghost {
                pane::Pane::draw_ghost(p, *r, title, *detach);
            }
            let hot = match (drag, hover) {
                (Drag::Splitter(s), _) | (_, Hit::Splitter(s)) => Some(s),
                _ => None,
            };
            if let Some(s) = hot.filter(|_| maximized.is_none()) {
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
            ui::draw_status(p, w, h, &status, layout, layout_hover, picker_open || hover == Hit::Gear, hover == Hit::Groups, hover == Hit::Xtree);
            if let Some(pane) = term_menu {
                let term_hover = match hover {
                    Hit::TermMenu(c) => c,
                    _ => None,
                };
                ui::draw_terminal_menu(p, panes[pane].terminal_rect(), w, term_hover);
            }
            if picker_open {
                let picker_hover = match hover {
                    Hit::Picker(k) => Some(k),
                    _ => None,
                };
                ui::draw_picker(p, w, h, accent, picker_hover, file_colors, show_group_bar, tree_on);
            }
            if let Some(sg) = suggest {
                sg.draw(p);
            }
            if let Some(pal) = palette {
                pal.draw(p, w);
            }
            if about_open {
                let about_hover = match hover {
                    Hit::About(k) => Some(k),
                    _ => None,
                };
                about::draw(p, w, h, about_hover);
            }
        });
        if result.is_err() {
            return;
        }
        // Pestañas deslizándose hacia su lugar: otro cuadro.
        if self.panes.iter().any(|p| p.tab_busy) {
            animating = true;
            self.last_frame = Some(now);
        }
        if animating {
            self.redraw();
        }
        uia::notify_focus(self);
        // Superposiciones de las filas recién dibujadas.
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for i in self.shown_panes() {
            self.panes[i].tab_mut().request_overlays(hwnd, &tx);
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
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
}

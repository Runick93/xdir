//! Entrada de mouse: qué hay bajo el cursor, clics y arrastres.

use super::*;

impl App {
    // ---------- entrada ----------

    pub(super) fn client_dips(&self) -> (f32, f32) {
        self.host.client_size()
    }

    pub(super) fn hit(&self, x: f32, y: f32) -> Hit {
        let (w, h) = self.client_dips();
        // El diálogo "Acerca de" es modal: tapa todo lo demás.
        if self.about_open {
            return about::at(w, h, x, y).map_or(Hit::None, Hit::About);
        }
        // Los paneles de unidades de red y «Conectar a un servidor» también son modales.
        if let Some(m) = &self.net.map {
            let l = crate::net::drives_dialog::map_layout(w, h);
            return crate::net::drives_dialog::map_hit(&l, m, x, y).map_or(Hit::None, Hit::Drives);
        }
        if let Some(u) = &self.net.unmap {
            let l = crate::net::drives_dialog::unmap_layout(w, h, u);
            return crate::net::drives_dialog::unmap_hit(&l, x, y).map_or(Hit::None, Hit::Drives);
        }
        if let Some(d) = &self.net.dialog {
            let l = crate::net::dialog::layout(w, h, d, &self.net.places);
            return crate::net::dialog::hit(&l, x, y).map_or(Hit::None, Hit::Connect);
        }
        // La paleta va encima de todo, y después los menús flotantes.
        if let Some((pal, _)) = &self.palette {
            if let Some(i) = pal.item_at(w, x, y) {
                return Hit::Palette(i);
            }
        }
        if let Some(sg) = &self.suggest {
            if let Some(i) = sg.item_at(x, y) {
                return Hit::Suggest(i);
            }
        }
        // La tarjeta de la vista rápida (sin visor del sistema) va encima de los paneles.
        if let Some(h) = self.quick_look_hit(x, y) {
            return h;
        }
        if let Some(pane) = self.term_menu {
            if let Some(c) = ui::terminal_menu_at(self.panes[pane].terminal_rect(), w, x, y) {
                return Hit::TermMenu(c);
            }
        }
        if self.picker_open {
            if let Some(k) = ui::picker_at(w, h, x, y) {
                return Hit::Picker(k);
            }
        }
        if let Some(i) = self.filter_menu_pane() {
            if let Some(h) = self.panes[i].tab().list.filter_menu_at(x, y) {
                return Hit::FilterMenu(i, h);
            }
        }
        if contains(&ui::gear_rect(w, h), x, y) {
            return Hit::Gear;
        }
        if contains(&ui::groups_rect(w, h), x, y) {
            return Hit::Groups;
        }
        if contains(&self.xtree_button_rect(), x, y) {
            return Hit::Xtree;
        }
        if contains(&ui::preview_rect(w, h), x, y) {
            return Hit::PreviewToggle;
        }
        if self.transfers_progress().is_some() && contains(&self.transfers_button_rect(), x, y) {
            return Hit::Transfers;
        }
        // El indicador de actividad (donde quedó en el último cuadro).
        if let Some(r) = self.activity_rect
            && contains(&r, x, y)
        {
            return Hit::Activity;
        }
        // «calcular tamaño» de la barra de estado (donde quedó en el último cuadro).
        if let Some(r) = self.size_link
            && contains(&r, x, y)
            && !self.xtree_open
        {
            return Hit::SizeLink;
        }
        // «Ver novedades» del aviso de actualización (donde quedó en el último cuadro).
        if self.update_notice.is_some()
            && let Some(r) = self.update_link
            && contains(&r, x, y)
        {
            return Hit::UpdateNotice;
        }
        // La vista de xtree tapa los paneles, la franja de grupos y el árbol.
        if self.xtree_open {
            if let Some(l) = ui::layout_at(w, h, x, y) {
                return Hit::Layout(l);
            }
            if contains(&crate::xtree::view::back_rect(ui::xtree_area(w, h)), x, y) {
                return Hit::XtreeBack;
            }
            return if contains(&ui::xtree_area(w, h), x, y) { Hit::XtreeView } else { Hit::None };
        }
        if self.preview_edge_at(x, y) {
            return Hit::PreviewEdge;
        }
        if let Some(h) = self.rename_panel_hit(x, y) {
            return Hit::RenamePanel(h);
        }
        if let Some(h) = self.info_hit(x, y) {
            return Hit::Info(h);
        }
        #[cfg(feature = "arbol")]
        if self.tree.contains(x, y) {
            return if self.tree.on_pin_button(x, y) {
                Hit::TreePin
            } else if self.tree.xtree_at(x, y) {
                Hit::TreeXtree
            } else {
                Hit::None
            };
        }
        let temps: Vec<bool> = self.groups.items.iter().map(|g| g.temp).collect();
        if let Some(g) = ui::group_bar_at(w - self.preview_width(), ui::GAP + self.tree_width(), &temps, x, y).filter(|_| self.groups_bar_h() > 0.0) {
            return Hit::GroupBar(g);
        }
        let area = ui::panes_area(w, h, self.tree_width(), self.groups_bar_h(), self.preview_width());
        if self.maximized.is_none() {
            for (s, r) in ui::splitter_rects(self.layout, area, self.splits) {
                if contains(&r, x, y) {
                    return Hit::Splitter(s);
                }
            }
        }
        if let Some(l) = ui::layout_at(w, h, x, y) {
            return Hit::Layout(l);
        }
        for i in self.shown_panes() {
            match self.panes[i].hit(x, y) {
                PaneHit::None => {}
                // La configuración ocupa el lugar de la lista.
                PaneHit::List if self.panes[i].tab().is_settings() => {
                    return Hit::Settings(i, self.settings_hit(i, x, y).unwrap_or(crate::settings::SettingsHit::Body));
                }
                // Las novedades también.
                PaneHit::List if self.panes[i].tab().is_changelog() => return Hit::Changelog(i),
                h => return Hit::Pane(i, h),
            }
        }
        Hit::None
    }

    pub fn on_left_down(&mut self, x: f32, y: f32, double: bool) {
        self.dismiss_tooltip();
        let hit = self.hit(x, y);
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        // Grabando un atajo, un clic fuera de su fila deja de grabar (y sigue su curso).
        self.cancel_recording_unless(hit);
        self.info.focused = matches!(hit, Hit::Info(_));
        if self.about_open {
            match hit {
                Hit::About(about::AboutHit::Report) => self.host.open_url(about::ISSUES_URL),
                Hit::About(about::AboutHit::Changelog) => self.open_changelog(None),
                Hit::About(about::AboutHit::Panel) => {}
                _ => self.about_open = false,
            }
            self.redraw();
            return;
        }
        if self.net.map.is_some() || self.net.unmap.is_some() {
            let h = match hit {
                Hit::Drives(h) => Some(h),
                _ => None,
            };
            if self.net.map.is_some() { self.map_click(h) } else { self.unmap_click(h) }
            return;
        }
        if self.net.dialog.is_some() {
            let h = match hit {
                Hit::Connect(h) => Some(h),
                _ => None,
            };
            self.connect_dialog_click(h, double);
            return;
        }
        if matches!(hit, Hit::QuickLookTitle(_) | Hit::QuickLookBody(_)) {
            self.quick_look_click(hit);
            return;
        }
        // Desplegable de la barra de dirección: una ruta se abre; el resto del panel no hace nada.
        if let Hit::Suggest(i) = hit {
            if let Some(i) = i {
                self.pick_suggestion(i);
            }
            return;
        }
        // Con la paleta abierta: un resultado se abre, el resto del panel no hace nada y fuera
        // de ella se cierra (sin que el clic llegue a lo de abajo).
        if self.palette.is_some() {
            match hit {
                Hit::Palette(Some(i)) => self.activate_palette(i, key_down(VK_CONTROL)),
                Hit::Palette(None) => {
                    if let Some((_, edit)) = &self.palette {
                        self.host.text_focus(*edit);
                    }
                }
                _ => self.close_palette(),
            }
            return;
        }
        // Desplegable de filtros: una opción se tilda o destilda (sigue abierto); un clic afuera
        // lo cierra sin llegar a lo de abajo, como un menú.
        if let Hit::FilterMenu(i, h) = hit {
            if let Some(h) = h {
                self.panes[i].tab_mut().list.click_filter_menu(h);
                self.sync_folder_views();
                self.sync_quick_look();
            }
            self.redraw();
            return;
        }
        if self.close_filter_menus() {
            self.redraw();
            return;
        }
        #[cfg(feature = "arbol")]
        if !self.xtree_open {
            if self.tree.edge_at(x, y) {
                self.host.capture_mouse();
                self.drag = Drag::TreeEdge;
                return;
            }
            if let Some((bar, grab)) = self.tree.bar_at(x, y) {
                self.host.capture_mouse();
                self.drag = Drag::TreeBar(bar, grab);
                self.tree.drag_bar(bar, grab, x, y);
                self.redraw();
                return;
            }
        }
        #[cfg(feature = "arbol")]
        if !self.xtree_open && self.on_tree_click(x, y) {
            return;
        }
        // Un clic fuera del panel de colores lo cierra (y sigue su curso).
        if self.picker_open && !matches!(hit, Hit::Picker(_) | Hit::Gear) {
            self.picker_open = false;
            self.redraw();
        }
        if self.term_menu.is_some() && !matches!(hit, Hit::TermMenu(_) | Hit::Pane(_, PaneHit::Terminal)) {
            self.term_menu = None;
            self.redraw();
        }
        // ⏏ de un disco en «Este equipo».
        if let Hit::Pane(i, PaneHit::List) = hit
            && let Some(idx) = self.panes[i].tab().list.eject_hit(x, y)
        {
            self.set_active(i);
            let root = self.panes[i].tab().entry_path(idx);
            self.eject(root);
            self.redraw();
            return;
        }
        match hit {
            Hit::TermMenu(Some(c)) => {
                if let Some(pane) = self.term_menu.take() {
                    self.open_console(pane, c);
                }
            }
            Hit::TermMenu(None) | Hit::Palette(_) | Hit::Suggest(_) => {}
            // Un clic en el botón con el menú abierto lo cierra (el menú ya se descartó al llegar el
            // clic): no hay que volver a abrirlo.
            Hit::Groups if self.groups_menu_closed.is_some_and(|t| t.elapsed().as_millis() < 300) => self.groups_menu_closed = None,
            Hit::Groups => self.show_groups_menu(),
            Hit::Xtree => self.toggle_xtree(),
            Hit::PreviewToggle => self.run_command(Command::TogglePreview),
            Hit::Transfers => self.show_transfers(),
            Hit::Activity => self.activity_click(),
            Hit::RenamePanel(h) => {
                self.rename_panel_click(h);
                return;
            }
            Hit::Info(h) => self.info_click(h),
            Hit::PreviewEdge => {
                self.host.capture_mouse();
                let grab = self.preview_column().map_or(0.0, |r| r.left - x);
                self.drag = Drag::PreviewEdge(grab);
            }
            Hit::XtreeBack => {
                self.close_xtree();
                self.redraw();
            }
            Hit::XtreeView => {
                let area = self.xtree_area();
                let action = self.xtree.as_mut().map_or(xtree_view::Action::None, |v| v.on_click(area, x, y, double));
                self.xtree_action(action);
            }
            Hit::GroupBar(ui::GroupHit::Tab(i)) if double => {
                if let Some(name) = self.groups.items.get(i).map(|g| g.name.clone()) {
                    self.open_palette(PaletteMode::GroupName);
                    self.group_rename = Some(name);
                }
            }
            Hit::GroupBar(ui::GroupHit::Tab(i)) => self.switch_group(i),
            Hit::GroupBar(ui::GroupHit::Close(i)) => self.close_group_tab(i),
            Hit::GroupBar(ui::GroupHit::Add) => self.new_group(),
            Hit::Gear => {
                self.picker_open = !self.picker_open;
                self.redraw();
            }
            Hit::Picker(ui::PickerHit::Preset(i)) => self.set_accent(Accent::Rgb(theme::PRESETS[i].1)),
            Hit::Picker(ui::PickerHit::System) => self.set_accent(Accent::System),
            Hit::Picker(ui::PickerHit::Custom) => self.choose_custom_color(),
            Hit::Picker(ui::PickerHit::DarkMode) => self.set_dark_mode(!self.theme.dark),
            Hit::Picker(ui::PickerHit::FileColors) => {
                self.file_colors = !self.file_colors;
                self.redraw();
            }
            Hit::Picker(ui::PickerHit::Extensions) => self.set_show_extensions(!self.show_extensions),
            Hit::Picker(ui::PickerHit::Checkboxes) => self.set_checkboxes(!self.checkboxes),
            #[cfg(feature = "arbol")]
            Hit::Picker(ui::PickerHit::TreeFollow) => self.set_tree_follow(!self.tree_follows()),
            #[cfg(not(feature = "arbol"))]
            Hit::Picker(ui::PickerHit::TreeFollow) => {}
            // El mismo estado que Ctrl+H: correr el comando.
            Hit::Picker(ui::PickerHit::Hidden) => self.run_command(Command::ToggleHidden),
            Hit::Picker(ui::PickerHit::GroupBar) => self.run_command(Command::ToggleGroupBar),
            Hit::Picker(ui::PickerHit::Tree) => self.run_command(Command::ToggleTree),
            Hit::Picker(ui::PickerHit::About) => self.run_command(Command::About),
            Hit::Picker(ui::PickerHit::Changelog) => self.run_command(Command::Changelog),
            Hit::UpdateNotice => self.open_update_notice(),
            Hit::SizeLink => self.request_sel_size(),
            Hit::Changelog(i) => {
                self.set_active(i);
                self.redraw();
            }
            Hit::Picker(ui::PickerHit::MoreOptions) => self.run_command(Command::Settings),
            Hit::Settings(i, h) => {
                self.set_active(i);
                self.settings_click(i, h, x, y);
            }
            Hit::Picker(ui::PickerHit::Panel) => {}
            Hit::Splitter(s) if double => {
                self.begin_layout_anim();
                self.splits.set(s, 0.5);
                self.redraw();
            }
            Hit::Splitter(s) => {
                self.host.capture_mouse();
                self.drag = Drag::Splitter(s);
            }
            Hit::Layout(l) => {
                // Con la vista de xtree abierta el cambio no se vería: se cierra.
                self.close_xtree();
                self.set_layout(l);
            }
            Hit::Pane(i, ph) => {
                self.set_active(i);
                match ph {
                    PaneHit::Tab(j) => {
                        self.select_tab(i, j);
                        self.host.capture_mouse();
                        let grab = self.panes[i].tab_grab(j, x);
                        self.drag = Drag::Tab { pane: i, index: j, start_x: x, start_y: y, moved: false, grab };
                    }
                    PaneHit::TabClose(j) => self.close_tab(i, j),
                    PaneHit::Placeholder(crate::list::placeholder::PlaceholderButton::Retry) => {
                        self.panes[i].tab_mut().reload(hwnd, &tx, false);
                        self.watch_slow_load(i);
                    }
                    PaneHit::Placeholder(crate::list::placeholder::PlaceholderButton::CloseTab) => {
                        let j = self.panes[i].active;
                        self.close_tab(i, j);
                    }
                    PaneHit::NewTab => {
                        let loc = new_tab_location(&self.panes[i].tab().location);
                        self.new_tab(i, loc, true);
                    }
                    PaneHit::Maximize => self.toggle_maximize(i),
                    PaneHit::ViewButton => {
                        let next = (self.panes[i].tab().list.view.cycle_index() + 1) % list::CYCLE.len();
                        self.set_view(i, list::CYCLE[next]);
                    }
                    PaneHit::NewFolderButton => self.run_command(Command::NewFolder),
                    PaneHit::Restore => self.restore_selection(),
                    PaneHit::Search => self.begin_filter(i),
                    PaneHit::SearchClear => self.clear_filter(i),
                    PaneHit::TabStrip if double => self.toggle_maximize(i),
                    PaneHit::TabStrip => {}
                    PaneHit::Button(b) => {
                        self.host.capture_mouse();
                        self.drag = Drag::Button(i, b);
                    }
                    // Las migas de la configuración llevan a la sección (con una búsqueda, la
                    // terminan).
                    PaneHit::Crumb(_) if self.panes[i].tab().is_settings() => {
                        if !self.panes[i].tab().search_query().is_empty() {
                            self.clear_filter(i);
                        }
                    }
                    // Novedades tiene un solo tramo: es donde se está.
                    PaneHit::Crumb(_) if self.panes[i].tab().is_changelog() => {}
                    PaneHit::Crumb(k) => {
                        if let Some(loc) = self.panes[i].crumb(k) {
                            self.panes[i].tab_mut().navigate(hwnd, &tx, loc);
                        }
                    }
                    PaneHit::Address => self.begin_edit(i),
                    PaneHit::AddressClear => self.clear_edit(),
                    PaneHit::Chevron(k) => self.show_subfolder_menu(i, k),
                    PaneHit::Star => self.toggle_favorite(i),
                    PaneHit::Terminal => {
                        self.term_menu = if self.term_menu == Some(i) { None } else { Some(i) };
                    }
                    PaneHit::Fav(j) => {
                        self.host.capture_mouse();
                        self.drag = Drag::Fav { pane: i, index: j, start_x: x, moved: false };
                    }
                    PaneHit::FavOverflow => {
                        if let Some((r, first)) = self.panes[i].overflow() {
                            self.show_fav_menu(i, FavMenuSource::Overflow(first), r);
                        }
                    }
                    PaneHit::FavBar => {}
                    PaneHit::List if double => {
                        let host = &self.host;
                        let fit = self.panes[i].tab_mut().list.fit_column_at(x, y, &|s, bold| host.text_width(s, bold));
                        let action = match fit {
                            Some(action) => action,
                            None => self.panes[i].tab_mut().list.on_double_click(x, y),
                        };
                        self.apply(i, action);
                    }
                    PaneHit::List => {
                        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
                        let (action, capture) = self.panes[i].tab_mut().list.on_mouse_down(x, y, ctrl, shift);
                        if capture {
                            self.host.capture_mouse();
                            self.drag = Drag::List(i);
                        }
                        self.apply(i, action);
                    }
                    PaneHit::None => {}
                }
            }
            Hit::None | Hit::About(_) | Hit::Connect(_) | Hit::Drives(_) | Hit::FilterMenu(..) | Hit::TreePin | Hit::TreeXtree | Hit::QuickLookTitle(_) | Hit::QuickLookBody(_) => {}
        }
        // Un clic en la lista con la vista rápida abierta: pasa al elemento cliqueado.
        self.quick_look_follow_list();
        self.redraw();
    }

    pub fn on_left_up(&mut self, x: f32, y: f32) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        self.host.release_mouse();
        let tab_drop = self.tab_drop.take();
        for p in &mut self.panes {
            p.drag_tab = None;
            p.drag_hidden = None;
        }
        self.ghost = None;
        match drag {
            Drag::Button(i, b) if self.hit(x, y) == Hit::Pane(i, PaneHit::Button(b)) => self.on_button(i, b),
            Drag::Tab { pane, index, .. } => {
                if let Some((to, at)) = tab_drop {
                    self.move_tab_to_pane(pane, index, to, at);
                }
            }
            Drag::Fav { pane, index, moved: false, .. } => self.activate_fav(pane, &[index], key_down(VK_CONTROL)),
            Drag::Fav { index, .. } => {
                // Soltado sobre otra carpeta de favoritos: va adentro; si no, se reordena.
                match self.fav_drop.take().map(|(_, d)| d) {
                    Some(FavDrop::Into(g)) if g != index => {
                        if let Some(f) = self.favorites.remove(&[index]) {
                            let g = if g > index { g - 1 } else { g };
                            self.favorites.push_into(&[g], f);
                        }
                    }
                    Some(FavDrop::Before(to)) => self.favorites.move_root(index, to),
                    _ => {}
                }
                self.save_favorites();
            }
            Drag::List(i) => {
                let action = self.panes[i].tab_mut().list.on_mouse_up();
                self.apply(i, action);
            }
            Drag::ArchiveOut(from) => self.drop_archive_drag(from, x, y),
            _ => {}
        }
        self.redraw();
    }

    pub fn on_middle_click(&mut self, x: f32, y: f32) {
        match self.hit(x, y) {
            Hit::Pane(i, PaneHit::Tab(j) | PaneHit::TabClose(j)) => self.close_tab(i, j),
            Hit::Pane(i, PaneHit::Fav(j)) => self.activate_fav(i, &[j], true),
            // Clic medio en una carpeta: la abre en una pestaña nueva, sin cambiar de pestaña.
            Hit::Pane(i, PaneHit::List) => {
                let tab = self.panes[i].tab();
                if let Some(idx) = tab.list.entry_at(x, y) {
                    let e = &tab.list.entries[idx];
                    if e.is_dir {
                        // Dentro de un archivo comprimido, la subcarpeta de adentro.
                        let to = match tab.location.archive_child(&e.name.to_string_lossy()) {
                            Some(child) => child,
                            None => Location::Path(tab.entry_path(idx)),
                        };
                        self.new_tab(i, to, false);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn on_mouse_move(&mut self, x: f32, y: f32) {
        self.host.track_mouse_leave();
        let mut redraw = false;
        match self.drag {
            Drag::Splitter(s) => {
                let (w, h) = self.client_dips();
                self.splits.set(s, ui::split_value(s, ui::panes_area(w, h, self.tree_width(), self.groups_bar_h(), self.preview_width()), x, y));
                redraw = true;
            }
            Drag::Tab { pane, index, start_x, start_y, moved, grab } => {
                if moved || (x - start_x).abs() > 6.0 || (y - start_y).abs() > 6.0 {
                    let (mut pane, mut index) = (pane, index);
                    let mut hit = self.hit(x, y);
                    let in_strip = |h: Hit| match h {
                        Hit::Pane(j, PaneHit::Tab(_) | PaneHit::TabClose(_) | PaneHit::NewTab | PaneHit::TabStrip | PaneHit::Maximize) => Some(j),
                        _ => None,
                    };
                    // Sobre la tira de otro panel la pestaña se integra ahí.
                    if let Some(j) = in_strip(hit).filter(|&j| j != pane) {
                        let at = self.panes[j].insert_index_at(x).min(self.panes[j].tabs.len());
                        self.move_tab_to_pane(pane, index, j, at);
                        (pane, index) = (j, at);
                        hit = self.hit(x, y);
                    }
                    // Sobre el cuerpo de otro panel se marca dónde caería al soltar; sobre un
                    // divisor o la barra de estado, nada.
                    let target = match hit {
                        Hit::Pane(j, _) if j != pane => Some((j, self.panes[j].tabs.len())),
                        _ => None,
                    };
                    self.tab_drop = target;
                    // Acoplada mientras el mouse está sobre la tira del panel de la pestaña; fuera de
                    // las tiras se despega y flota.
                    let docked = in_strip(hit) == Some(pane);
                    let left = x - grab;
                    for p in &mut self.panes {
                        p.drag_tab = None;
                        p.drag_hidden = None;
                    }
                    if docked {
                        self.panes[pane].drag_tab = Some((index, left));
                        let center = left + self.panes[pane].tab_width() / 2.0;
                        if let Some(to) = self.panes[pane].tab_index_at(center) {
                            self.panes[pane].move_tab(index, to);
                            index = to;
                            self.panes[pane].drag_tab = Some((index, left));
                        }
                    } else {
                        self.panes[pane].drag_hidden = Some(index);
                    }
                    let (tx, ty) = match self.panes[pane].drag_rect.filter(|_| docked) {
                        Some(r) => (r.left, r.top),
                        None => (left, if docked { self.panes[pane].rect.top + 3.0 } else { y - 17.0 }),
                    };
                    let w = self.panes[pane].tab_width();
                    match &mut self.ghost {
                        Some(g) => (g.tx, g.ty, g.docked, g.w) = (tx, ty, docked, w),
                        None => {
                            let title = self.panes[pane].tabs[index].location.title();
                            self.ghost = Some(Ghost { title, w, x: left, y: y - 17.0, tx, ty, detach: 1.0, docked });
                        }
                    }
                    self.drag = Drag::Tab { pane, index, start_x, start_y, moved: true, grab };
                    redraw = true;
                }
            }
            #[cfg(feature = "arbol")]
            Drag::TreeEdge => {
                self.tree.set_width(x - ui::GAP);
                redraw = true;
            }
            #[cfg(feature = "arbol")]
            Drag::TreeBar(bar, grab) => {
                self.tree.drag_bar(bar, grab, x, y);
                redraw = true;
            }
            Drag::SettingsSlider(pane, id) => {
                self.settings_slider(pane, id, x);
                redraw = true;
            }
            Drag::PreviewEdge(grab) => {
                self.drag_preview_edge(x + grab);
                self.sync_rename_boxes();
                redraw = true;
            }
            Drag::List(i) => {
                let action = self.panes[i].tab_mut().list.on_mouse_move(x, y);
                // Unos píxeles con el botón apretado sobre un elemento: arrastrar los seleccionados
                // (a otro panel, al Explorador o a otra aplicación).
                if action == Action::BeginDrag {
                    self.apply(i, action);
                    return;
                }
                redraw |= action != Action::None;
            }
            Drag::Fav { pane, index, start_x, moved } => {
                if moved || (x - start_x).abs() > 5.0 {
                    self.drag = Drag::Fav { pane, index, start_x, moved: true };
                    let target = self.panes[pane].fav_drop_at(&self.favorites, x);
                    self.fav_drop = Some((pane, target));
                    redraw = true;
                }
            }
            _ => {}
        }

        let hover = self.hit(x, y);
        redraw |= hover != self.hover;
        // Una página extra de la configuración dibuja su hover con la posición del mouse.
        redraw |= matches!(hover, Hit::Settings(_, crate::settings::SettingsHit::Extra));
        redraw |= self.update_tooltip(hover, x, y);
        // Con xtree abierto el mouse es de la vista: los paneles y el árbol de abajo no se resaltan.
        if self.xtree_open {
            let area = self.xtree_area();
            if let Some(v) = &mut self.xtree {
                redraw |= v.on_mouse_move(area, x, y);
            }
            self.hover = hover;
            if redraw {
                self.redraw();
            }
            return;
        }
        #[cfg(feature = "arbol")]
        {
            redraw |= self.tree.on_mouse_move(x, y, !matches!(self.drag, Drag::None));
        }
        if let Some(sg) = &mut self.suggest {
            let h = match hover {
                Hit::Suggest(Some(i)) => Some(i),
                _ => None,
            };
            if sg.hover != h {
                sg.hover = h;
                redraw = true;
            }
        }
        if let Some((pal, _)) = &mut self.palette {
            let h = match hover {
                Hit::Palette(Some(i)) => Some(i),
                _ => None,
            };
            if pal.hover != h {
                pal.hover = h;
                redraw = true;
            }
        }
        if let Some(i) = self.filter_menu_pane() {
            let h = match hover {
                Hit::FilterMenu(_, h) => h,
                _ => None,
            };
            redraw |= self.panes[i].tab_mut().list.set_filter_hover(h);
        }
        // La fila resaltada solo en el panel bajo el mouse.
        for (i, pane) in self.panes.iter_mut().enumerate() {
            if self.drag == Drag::List(i) {
                continue;
            }
            let list = &mut pane.tab_mut().list;
            redraw |= if hover == Hit::Pane(i, PaneHit::List) {
                list.on_mouse_move(x, y) != Action::None
            } else {
                list.on_mouse_leave() != Action::None
            };
        }
        self.hover = hover;
        if redraw {
            self.redraw();
        }
    }
}

/// Dónde abre una pestaña nueva desde la pestaña `from`: la misma ubicación, salvo la
/// configuración y las novedades (hay una sola de cada una): «Este equipo».
pub(super) fn new_tab_location(from: &Location) -> Location {
    match from {
        Location::Settings | Location::Changelog => Location::Computer,
        other => other.clone(),
    }
}

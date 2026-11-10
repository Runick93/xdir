//! Paneles, pestañas, vistas, árbol lateral, sesión y layout.

use super::*;

impl App {
    // ---------- paneles y pestañas ----------

    pub(super) fn tab_mut(&mut self) -> &mut Tab {
        self.panes[self.active].tab_mut()
    }

    pub(super) fn tab_by_id(&mut self, id: usize) -> Option<&mut Tab> {
        self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()).find(|t| t.id == id)
    }

    pub(super) fn make_tab(&mut self, location: Location) -> Tab {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        // La vista la toma de su carpeta en la próxima `sync_folder_views`.
        let mut tab = Tab::new(id, location, self.show_hidden);
        tab.list.show_extensions = self.show_extensions;
        tab.list.checkboxes = self.checkboxes;
        tab
    }

    pub(super) fn tree_visible(&self) -> bool {
        #[cfg(feature = "arbol")]
        return self.tree.visible;
        #[cfg(not(feature = "arbol"))]
        false
    }

    pub(super) fn tree_quick_open(&self) -> bool {
        #[cfg(feature = "arbol")]
        return self.tree.quick_open();
        #[cfg(not(feature = "arbol"))]
        true
    }

    pub(super) fn tree_pinned(&self) -> bool {
        #[cfg(feature = "arbol")]
        return self.tree.pinned;
        #[cfg(not(feature = "arbol"))]
        true
    }

    /// Lee el Acceso rápido de Windows para el árbol y empieza a vigilar sus cambios.
    #[cfg(feature = "arbol")]
    pub fn start_quick_access(&mut self) {
        self.tree.request_quick(self.hwnd);
        self.quick_watch = sys::file_system().quick_access_watch_dir().and_then(|dir| sys::file_system().watch(self.hwnd, tree::WM_APP_QUICK, 0, 0, &dir));
    }

    /// Menú contextual nativo de una carpeta del árbol (incluye "Anclar a Acceso rápido" /
    /// "Desanclar de Acceso rápido"); al cerrarse se relee el Acceso rápido.
    #[cfg(feature = "arbol")]
    pub(super) fn tree_context_menu(&mut self, path: PathBuf, at: (f32, f32)) {
        let items = [shell::Item::from(path.clone())];
        let pinned = self.tree.is_pinned(&path);
        let chosen = sys::shell().context_menu(self.hwnd, &Location::Path(path.clone()), &items, at, key_down(VK_SHIFT), false, &[], pinned);
        if chosen.as_deref() == Some(shell::UNPIN_QUICK) {
            self.toggle_quick_pin(path);
        } else if chosen.is_some() {
            self.tree.request_quick(self.hwnd);
            self.tree.on_mouse_leave();
        }
        self.redraw();
    }

    /// Ancla la carpeta al Acceso rápido de Windows, o la desancla si ya lo está (verbos nativos
    /// `pintohome` y `unpinfromhome`), y relee el Acceso rápido.
    #[cfg(feature = "arbol")]
    pub(super) fn toggle_quick_pin(&mut self, path: PathBuf) {
        // El estado se pregunta de nuevo al Shell: "pintohome" sobre una carpeta ya anclada la desancla.
        let done = if sys::file_system().is_quick_pinned(&path) { sys::shell().unpin_quick_access(self.hwnd, &path) } else { sys::shell().pin_quick_access(self.hwnd, &path) };
        if done.is_err() {
            self.notice = Some("No se pudo cambiar el Acceso rápido".into());
        }
        self.tree.request_quick(self.hwnd);
        self.redraw();
    }

    /// Ancho que ocupa el árbol de carpetas a la izquierda (0 si no está o no se compiló).
    pub(super) fn tree_width(&self) -> f32 {
        #[cfg(feature = "arbol")]
        if self.tree.visible {
            return self.tree.layout_width() + ui::GAP;
        }
        0.0
    }

    /// El árbol sigue al panel activo (`arbol_sigue_panel`): cuando cambia la carpeta que muestra
    /// (navegar, cambiar de pestaña o de panel, mostrar el árbol), despliega sus carpetas padre y
    /// la deja a la vista. Se llama antes de cada cuadro: todo cambio de carpeta pide redibujar.
    #[cfg(feature = "arbol")]
    pub(super) fn follow_tree(&mut self) {
        if !self.tree_follows() || !self.tree.visible || self.xtree_open {
            return;
        }
        let current = &self.panes[self.active].tab().location;
        if self.tree_followed.as_ref() == Some(current) {
            return;
        }
        self.tree_followed = Some(current.clone());
        // Solo carpetas de disco: "Este equipo" y las ubicaciones del Shell no están en el árbol.
        if let Location::Path(p) = current {
            let p = p.clone();
            self.tree.reveal(self.hwnd, &p, self.show_hidden);
        }
    }

    /// Clic en el árbol: expandir/contraer o abrir la carpeta en el panel activo. Devuelve si
    /// el clic era del árbol.
    #[cfg(feature = "arbol")]
    pub(super) fn on_tree_click(&mut self, x: f32, y: f32) -> bool {
        if !self.tree.contains(x, y) {
            return false;
        }
        if let Some(id) = self.tree.eject_hit(x, y) {
            let root = self.tree.path_of(id);
            self.eject(root);
            self.redraw();
            return true;
        }
        match self.tree.hit(x, y) {
            tree::TreeHit::Toggle(id) => self.tree.toggle(self.hwnd, id, self.show_hidden),
            tree::TreeHit::Open(loc) => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                self.tab_mut().navigate(hwnd, &tx, loc);
            }
            tree::TreeHit::Pin => self.run_command(Command::ToggleTreePin),
            // La chincheta de una carpeta anclada la desancla, como en el Explorador.
            tree::TreeHit::Unpin(path) => self.toggle_quick_pin(path),
            // Como el botón «xtree» de la barra: abre la vista sobre la carpeta del panel activo
            // o la cierra si está abierta. No navega el panel.
            tree::TreeHit::Xtree => self.toggle_xtree(),
            tree::TreeHit::Nothing => {}
        }
        self.redraw();
        true
    }

    /// Vista de la pestaña activa del panel (queda guardada para su carpeta).
    pub(super) fn set_view(&mut self, pane: usize, view: list::View) {
        self.panes[pane].tab_mut().list.set_view(view);
        self.sync_folder_views();
        self.notice = Some(format!("Vista: {}", view.name().to_lowercase()));
        self.view_notice = Some((pane, self.panes[pane].tab().view_key(), view));
        self.redraw();
    }

    /// Ctrl+rueda: vista siguiente (más grande) o anterior sobre el panel, en el orden de
    /// `list::CYCLE`.
    pub(super) fn cycle_view(&mut self, pane: usize, bigger: bool) {
        let i = self.panes[pane].tab().list.view.cycle_index();
        let j = if bigger { (i + 1).min(list::CYCLE.len() - 1) } else { i.saturating_sub(1) };
        if i != j {
            self.set_view(pane, list::CYCLE[j]);
        }
    }

    // ---------- vista por carpeta ----------

    /// Cada lista toma la vista de su carpeta al cambiar de ubicación, y lo que el usuario cambia
    /// (vista, orden, columnas, anchos) se guarda para la carpeta que muestra (`Tab::sync_view`).
    /// Se llama antes de cada cuadro, antes de entregar los lotes de carga y después de cada
    /// cambio de vista; si cambió algo, `carpetas.toml` se guarda un segundo después.
    pub(super) fn sync_folder_views(&mut self) {
        let views = &mut self.folder_views;
        let mut changed = false;
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            changed |= tab.sync_view(views);
        }
        if changed {
            self.schedule_views_save();
        }
        self.expire_view_notice();
    }

    /// Borra el aviso «Vista: …» si quedó viejo: se navegó a otra carpeta, cambió la vista o la
    /// lista activa es otra. Si otro aviso lo reemplazó, ese no se toca.
    fn expire_view_notice(&mut self) {
        let Some((pane, key, view)) = &self.view_notice else { return };
        let text = format!("Vista: {}", view.name().to_lowercase());
        if self.notice.as_deref() != Some(text.as_str()) {
            self.view_notice = None;
            return;
        }
        let still = *pane == self.active
            && self.panes.get(*pane).is_some_and(|p| p.tab().view_key() == *key && p.tab().list.view == *view);
        if !still {
            self.notice = None;
            self.view_notice = None;
        }
    }

    /// Panel a la vista con el desplegable de filtros abierto en su lista.
    pub(super) fn filter_menu_pane(&self) -> Option<usize> {
        if self.xtree_open {
            return None;
        }
        self.shown_panes().into_iter().find(|&i| self.panes[i].tab().list.filter_menu().is_some())
    }

    /// Cierra los desplegables de filtros (de todas las pestañas). Devuelve si había alguno.
    pub(super) fn close_filter_menus(&mut self) -> bool {
        let mut closed = false;
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            closed |= tab.list.close_filter_menu();
        }
        closed
    }

    /// Guardado diferido de `carpetas.toml`: un segundo después del último cambio (un solo aviso
    /// en camino, con el mismo patrón que la pausa del buscador) y al cerrar.
    pub(super) fn schedule_views_save(&mut self) {
        self.views_changed_at = Some(Instant::now());
        if !self.views_save_posted {
            self.views_save_posted = true;
            post_after(self.hwnd, VIEWS_SAVE_DELAY_MS);
        }
    }

    /// Llegó el aviso de guardado: si hubo cambios después de pedirlo, se espera lo que falta.
    pub(super) fn on_views_save_due(&mut self) {
        self.views_save_posted = false;
        let Some(at) = self.views_changed_at else { return };
        let waited = at.elapsed().as_millis() as u64;
        if waited < VIEWS_SAVE_DELAY_MS {
            self.views_save_posted = true;
            post_after(self.hwnd, VIEWS_SAVE_DELAY_MS - waited);
            return;
        }
        self.views_changed_at = None;
        self.folder_views.save_if_dirty();
    }

    /// «Usar esta vista en todas las carpetas»: con confirmación, la vista de la carpeta del panel
    /// activo pasa a ser la por defecto y se borran las de las demás; las listas abiertas la toman.
    pub(super) fn apply_view_to_all(&mut self, pane: usize) {
        let question = "¿Usar esta vista en todas las carpetas? Se pierden las vistas guardadas de cada carpeta.";
        if self.host.ask_yes_no_cancel("Usar esta vista en todas las carpetas", question) != Some(true) {
            return;
        }
        let mut v = self.panes[pane].tab().list.view_settings();
        // La columna de carpeta es solo de las búsquedas.
        v.columns.retain(|c| c.col != list::Column::Folder);
        self.folder_views.apply_to_all(v);
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.reload_view();
        }
        self.sync_folder_views();
        self.schedule_views_save();
    }

    /// «Restablecer la vista de esta carpeta»: vuelve a la por defecto (también en las otras
    /// pestañas que la muestran).
    pub(super) fn reset_folder_view(&mut self, pane: usize) {
        let key = self.panes[pane].tab().view_key();
        self.folder_views.reset(&key);
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()).filter(|t| t.view_key() == key) {
            tab.reload_view();
        }
        self.sync_folder_views();
        self.schedule_views_save();
    }

    /// Menú del clic derecho en el encabezado: mostrar u ocultar columnas.
    /// `at`: punto del clic derecho, en DIPs del área de la ventana.
    pub(super) fn show_columns_menu(&mut self, pane: usize, at: (f32, f32)) {
        const RESET: usize = 100;
        // Ids de «Ver», «Ordenar por» y «Agrupar por»: después de las columnas y de RESET.
        const SUB_BASE: u32 = 200;
        let current = self.panes[pane].tab().list.columns.clone();
        let mut entries: Vec<shell::Entry> = list::COLUMNS
            .iter()
            .enumerate()
            .filter(|(_, c)| c.0 != list::Column::Folder)
            .map(|(i, &(col, title, _, _))| shell::Entry::Item {
                id: i as u32 + 1,
                label: title.to_string(),
                // El nombre no se puede ocultar.
                enabled: col != list::Column::Name,
                checked: current.iter().any(|c| c.col == col),
            })
            .collect();
        entries.push(shell::Entry::Sep);
        let mut cmds = Vec::new();
        let mut item = |label: &str, command: Command, enabled: bool, checked: bool| {
            cmds.push(command);
            shell::Entry::Item { id: SUB_BASE + cmds.len() as u32 - 1, label: label.to_string(), enabled, checked }
        };
        let views = self.views_submenu(pane, &mut item);
        let (sorts, groupings) = self.sort_group_submenus(pane, &mut item);
        entries.push(views);
        entries.push(sorts);
        entries.push(groupings);
        entries.push(shell::Entry::Sep);
        entries.push(shell::Entry::Item { id: RESET as u32, label: "Columnas de fábrica".into(), enabled: true, checked: false });
        let cmd = self.host.popup_menu(&entries, MenuPos::At(at.0, at.1), None, None).map_or(0, |c| c as usize);
        match cmd {
            c if c >= SUB_BASE as usize => {
                // Lo mismo que en el menú del fondo, pero sobre el panel del encabezado.
                let list = &mut self.panes[pane].tab_mut().list;
                match cmds.get(c - SUB_BASE as usize) {
                    Some(&Command::View(i)) => match list::VIEWS.get(i as usize) {
                        Some(&(view, ..)) => {
                            self.set_view(pane, view);
                            self.redraw();
                            return;
                        }
                        None => return,
                    },
                    Some(&Command::ApplyViewToAll) => {
                        self.apply_view_to_all(pane);
                        self.redraw();
                        return;
                    }
                    Some(&Command::ResetFolderView) => {
                        self.reset_folder_view(pane);
                        self.redraw();
                        return;
                    }
                    Some(&Command::SortBy(i)) => match list::COLUMNS.get(i as usize) {
                        Some(&(col, ..)) => list.set_sort(col),
                        None => return,
                    },
                    Some(&Command::GroupBy(i)) => match list::GROUPINGS.get(i as usize) {
                        Some(&col) => list.set_group(col),
                        None => return,
                    },
                    _ => return,
                }
            }
            RESET => {
                let list = &mut self.panes[pane].tab_mut().list;
                // La columna de carpeta de una búsqueda se queda donde está.
                let folder = list.columns.iter().position(|c| c.col == list::Column::Folder).map(|i| (i, list.columns[i]));
                list.reset_name_width();
                list.columns = list::default_columns();
                if let Some((at, spec)) = folder {
                    list.columns.insert(at.min(list.columns.len()), spec);
                }
            }
            i if (1..=list::COLUMNS.len()).contains(&i) => self.panes[pane].tab_mut().list.toggle_column(list::COLUMNS[i - 1].0),
            _ => return,
        }
        self.sync_folder_views();
        self.redraw();
    }

    pub(super) fn restore_panes(&mut self, s: &Session, start_path: Option<PathBuf>) {
        let fallback = default_session();
        let mut seen_settings = false;
        for i in 0..PANES {
            let state = s.panes.get(i).or(fallback.panes.get(i)).cloned().unwrap_or_default();
            let locations = if state.tabs.is_empty() { vec![Location::Computer] } else { state.tabs };
            let mut tabs: Vec<Tab> = locations.into_iter().map(|l| self.make_tab(l)).collect();
            // Hay una sola configuración: si la sesión trae más de una, quedan carpetas.
            for tab in &mut tabs {
                if let Some(v) = &mut tab.settings {
                    if seen_settings {
                        *tab = Tab::new(tab.id, Location::Computer, self.show_hidden);
                    } else {
                        v.set_page(s.settings_page.as_deref().map_or(crate::settings::PageId::default(), Self::page_from_id));
                    }
                    seen_settings = true;
                }
            }
            self.panes.push(Pane::new(tabs, state.active));
        }
        self.active = s.active_pane.min(self.layout.panes() - 1);
        // Una carpeta pasada por línea de comandos se abre en una pestaña nueva del panel activo.
        if let Some(path) = start_path {
            let tab = self.make_tab(Location::Path(path));
            self.panes[self.active].add_tab(tab, true);
        }
    }

    pub(super) fn session(&self) -> Session {
        let mut s = Session {
            layout: self.layout.index(),
            split_x: self.splits.x,
            split_y: self.splits.y,
            active_pane: self.active,
            show_hidden: self.show_hidden,
            show_favorites: self.show_favorites,
            group_bar: self.show_group_bar,
            file_colors: self.file_colors,
            show_extensions: self.show_extensions,
            checkboxes: self.checkboxes,
            tree_follow: self.tree_follow,
            show_tree: self.tree_visible(),
            tree_pinned: self.tree_pinned(),
            quick_open: self.tree_quick_open(),
            accent: self.accent,
            dark_mode: self.dark_mode,
            group: self.group_name.clone(),
            preview_on: self.preview.on,
            preview_width: self.preview.width,
            info_sections: self.info.sections,
            settings_page: self.settings_session_page(),
            xtree_last: self.xtree.as_ref().map(|x| x.root().to_path_buf()).or_else(|| self.xtree_last.clone()),
            // Ya no se usan (las vistas son por carpeta), pero una versión anterior las lee.
            columns: self.folder_views.default_view().columns.clone(),
            views: self.panes.iter().map(|p| p.tab().list.view).collect(),
            panes: self
                .panes
                .iter()
                .map(|p| PaneState { tabs: p.tabs.iter().map(|t| t.location.clone()).collect(), active: p.active })
                .collect(),
            ..Default::default()
        };
        if let Some((window, maximized)) = self.host.placement() {
            s.window = Some(window);
            s.maximized = maximized;
        }
        s
    }

    /// Solo las pestañas a la vista cargan y vigilan su carpeta.
    pub fn sync_visibility(&mut self) {
        let (hwnd, tx, visible_panes) = (self.hwnd, self.tx.clone(), self.layout.panes());
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let active = pane.active;
            for (j, tab) in pane.tabs.iter_mut().enumerate() {
                tab.set_visible(hwnd, &tx, i < visible_panes && j == active);
            }
        }
        // Cambió la pestaña a la vista (o el layout): Renombrar varios se recalcula o se cierra,
        // y una grabación de atajo que ya no se ve termina.
        self.end_hidden_recording();
        self.release_hidden_settings_field();
        self.refresh_rename_many();
        self.sync_rename_boxes();
    }

    pub(super) fn set_active(&mut self, pane: usize) {
        if pane < self.layout.panes() && pane != self.active {
            self.end_edit(false);
            // Ir a otro panel restaura el layout si había uno maximizado.
            self.maximized = None;
            self.active = pane;
            // Una grabación de atajo en el panel que se deja termina.
            self.end_hidden_recording();
            self.redraw();
        }
    }

    /// Maximiza el panel (ocupa toda el área) o, si ya lo está, vuelve al layout.
    pub(super) fn toggle_maximize(&mut self, pane: usize) {
        if self.layout.panes() < 2 || pane >= self.layout.panes() {
            return;
        }
        self.end_edit(false);
        self.term_menu = None;
        self.begin_layout_anim();
        self.maximized = if self.maximized == Some(pane) { None } else { Some(pane) };
        self.active = pane;
        self.redraw();
    }

    /// Índices de los paneles dibujados: todos los del layout o solo el maximizado.
    pub(super) fn shown_panes(&self) -> Vec<usize> {
        match self.maximized {
            Some(i) => vec![i],
            None => (0..self.layout.panes()).collect(),
        }
    }

    /// Comandos de prueba (feature `pruebas`): `cmd <id>` ejecuta una acción de `commands::ALL`;
    /// `picker`, `term <panel>`, `tabdrop <panel> <pos>`, `droptarget <panel> <pos>`,
    /// `select <panel> <pos>`, `addsel <panel> <pos>`, `hover <panel> <x> <y>`, `casillas`,
    /// `active <panel>`, `info <qué>` (un clic en el panel de información) y `notice <texto>` dejan la interfaz en un estado que se quiere mostrar.
    #[cfg(feature = "pruebas")]
    pub fn test_hook(&mut self, text: &str) {
        let mut it = text.splitn(3, ' ');
        let (verb, a, b) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or(""));
        let (na, nb) = (a.parse::<usize>().unwrap_or(0), b.parse::<usize>().unwrap_or(0));
        match verb {
            "cmd" => {
                if let Some(s) = crate::commands::ALL.iter().find(|s| s.id == a) {
                    self.run_command(s.command);
                }
            }
            "picker" => self.picker_open = true,
            "go" => self.go_to_text(na, b),
            // band <panel> <x0> <y0> <x1> <y1>: arrastre con rectángulo (en DIPs de la ventana).
            "band" => {
                let n: Vec<f32> = text.split_whitespace().skip(2).filter_map(|v| v.parse().ok()).collect();
                if let [x0, y0, x1, y1] = n[..] {
                    let list = &mut self.panes[na].tab_mut().list;
                    let down = list.on_mouse_down(x0, y0, false, false);
                    let moved = list.on_mouse_move(x1, y1);
                    self.notice = Some(format!("down={down:?} move={moved:?}"));
                }
            }
            "term" => self.term_menu = Some(na),
            "tabdrop" => self.tab_drop = Some((na, nb)),
            "droptarget" => self.panes[na].tab_mut().list.drop_target = Some(nb),
            "select" => self.panes[na].tab_mut().list.select_pos(nb),
            "addsel" => self.panes[na].tab_mut().list.set_selected_pos(nb, true),
            "casillas" => self.set_checkboxes(true),
            "hover" => {
                let n: Vec<f32> = text.split_whitespace().skip(2).filter_map(|v| v.parse().ok()).collect();
                if let [x, y] = n[..] {
                    self.panes[na].tab_mut().list.on_mouse_move(x, y);
                }
            }
            "active" => self.set_active(na),
            "notice" => self.notice = Some(format!("{a} {b}").trim().to_string()),
            "close" => self.close_palette(),
            "info" => self.info_test(a, nb),
            // Configuración: `config_pagina <id>` va a esa página; `config_parte <id> <parte>` es
            // un clic en esa parte de una fila (como la vio el último cuadro).
            "config_pagina" => {
                let page = Self::page_from_id(a);
                if let Some(v) = self.settings_view_mut(self.active) {
                    v.set_page(page);
                }
            }
            // `buscar <texto>`: lo escrito en la caja de búsqueda del panel activo.
            "buscar" => self.search_typed(self.active, text.strip_prefix("buscar").unwrap_or("").trim()),
            // `grabar <tecla> <ctrl,shift,alt,meta>`: una tecla mientras se graba un atajo.
            "grabar" => {
                let has = |m: &str| b.split(',').any(|x| x == m);
                self.settings_capture(crate::keys::Key(na as u16), has("ctrl"), has("shift"), has("alt"), has("meta"));
            }
            "config_parte" => {
                let id = crate::settings::options::OPTIONS
                    .iter()
                    .map(|o| o.id)
                    .chain(["reglas_color", "error_config", "restablecer_atajos"])
                    .chain(crate::commands::ALL.iter().map(|s| s.id))
                    .find(|id| *id == a);
                if let Some(r) = id.and_then(|id| self.part_rect(self.active, id, nb)) {
                    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    self.on_left_down(x, y, false);
                    self.on_left_up(x, y);
                }
            }
            "copiar" | "mover" | "ventana" => self.transfer_test(verb, &format!("{a} {b}")),
            _ => {}
        }
        self.redraw();
    }

    /// Animaciones: la opción de `config.toml` y la preferencia de Windows tienen que permitirlas.
    pub fn apply_animation_setting(&mut self) {
        anim::set_enabled(self.config.animations.unwrap_or_else(anim::system_allows));
        anim::set_off_by_config(self.config.animations == Some(false));
        if !anim::enabled() {
            self.layout_anim = None;
        }
    }

    /// Empieza una transición desde lo que se está viendo. Se llama ANTES de cambiar layout,
    /// divisores o maximizado: el estado cambia al instante y solo se dibuja de a poco.
    pub(super) fn begin_layout_anim(&mut self) {
        if anim::enabled() && self.displayed.iter().any(Option::is_some) {
            self.layout_anim = Some(LayoutAnim { from: self.displayed, start: Instant::now() });
        }
    }

    /// Rectángulos de los paneles para este frame: los de destino o, en plena transición, los
    /// intermedios. Devuelve también si la transición sigue en curso.
    pub(super) fn animated_rects(&mut self, target: Vec<(usize, geom::Rect)>) -> (Vec<(usize, geom::Rect)>, bool) {
        let progress = self.layout_anim.as_ref().map(|a| a.start.elapsed().as_secs_f32() * 1000.0 / anim::LAYOUT_MS);
        let (rects, running) = match (&self.layout_anim, progress) {
            (Some(a), Some(t)) if t < 1.0 => {
                let e = anim::ease_out(t);
                // Un panel que no estaba "aparece" desde un poco más chico.
                let rects = target.iter().map(|&(i, r)| (i, anim::lerp_rect(a.from[i].unwrap_or_else(|| anim::grow_from(r)), r, e))).collect();
                (rects, true)
            }
            _ => {
                self.layout_anim = None;
                (target, false)
            }
        };
        self.displayed = [None; PANES];
        for &(i, r) in &rects {
            self.displayed[i] = Some(r);
        }
        (rects, running)
    }

    pub(super) fn set_layout(&mut self, layout: Layout) {
        self.end_edit(false);
        if layout != self.layout || self.maximized.is_some() {
            self.begin_layout_anim();
        }
        self.maximized = None;
        self.layout = layout;
        self.active = self.active.min(layout.panes() - 1);
        self.sync_visibility();
        self.redraw();
    }

    pub(super) fn new_tab(&mut self, pane: usize, location: Location, activate: bool) {
        let tab = self.make_tab(location);
        self.panes[pane].add_tab(tab, activate);
        self.sync_visibility();
        self.sync_folder_views();
        self.redraw();
    }

    pub(super) fn close_tab(&mut self, pane: usize, index: usize) {
        if self.edit.is_some_and(|(_, p)| p == pane) {
            self.end_edit(false);
        }
        if self.panes[pane].close_tab(index) {
            self.sync_visibility();
            self.redraw();
        }
    }

    /// Lleva la pestaña `index` del panel `from` a la posición `at` del panel `to`, con su
    /// historial y su selección. Si era la única del panel de origen, ahí se abre "Este equipo".
    pub(super) fn move_tab_to_pane(&mut self, from: usize, index: usize, to: usize, at: usize) {
        if from == to || index >= self.panes[from].tabs.len() {
            return;
        }
        self.end_edit(false);
        self.term_menu = None;
        let tab = self.panes[from].take_tab(index);
        if self.panes[from].tabs.is_empty() {
            let fresh = self.make_tab(Location::Computer);
            self.panes[from].insert_tab(0, fresh);
        }
        self.panes[to].insert_tab(at, tab);
        self.active = to;
        self.sync_visibility();
        self.redraw();
    }

    pub(super) fn select_tab(&mut self, pane: usize, index: usize) {
        self.panes[pane].select_tab(index);
        self.sync_visibility();
        self.redraw();
    }
}

/// Manda `WM_APP_VIEWS_SAVE` a la ventana dentro de `ms` milisegundos.
fn post_after(hwnd: Handle, ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        hwnd.post(WM_APP_VIEWS_SAVE, 0, 0);
    });
}

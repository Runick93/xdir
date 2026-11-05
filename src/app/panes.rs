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
        let mut tab = Tab::new(id, location, self.show_hidden);
        tab.list.columns = self.columns.clone();
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
    pub(super) fn start_quick_access(&mut self) {
        self.tree.request_quick(self.hwnd);
        self.quick_watch = fs::quick_access_watch_dir().and_then(|dir| fs::Watcher::with_message(self.hwnd, tree::WM_APP_QUICK, 0, 0, &dir));
    }

    /// Menú contextual nativo de una carpeta del árbol (incluye "Anclar a Acceso rápido" /
    /// "Desanclar de Acceso rápido"); al cerrarse se relee el Acceso rápido.
    #[cfg(feature = "arbol")]
    pub(super) fn tree_context_menu(&mut self, path: PathBuf, lparam: LPARAM) {
        let screen = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
        let items = [shell::Item::from(path.clone())];
        let pinned = self.tree.is_pinned(&path);
        let chosen = shell::context_menu(self.hwnd, &Location::Path(path.clone()), &items, screen, key_down(VK_SHIFT), false, &[], pinned);
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
        let done = if fs::is_quick_pinned(&path) { shell::unpin_quick_access(self.hwnd, &path) } else { shell::pin_quick_access(self.hwnd, &path) };
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

    /// Clic en el árbol: expandir/contraer o abrir la carpeta en el panel activo. Devuelve si
    /// el clic era del árbol.
    #[cfg(feature = "arbol")]
    pub(super) fn on_tree_click(&mut self, x: f32, y: f32) -> bool {
        if !self.tree.contains(x, y) {
            return false;
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
            tree::TreeHit::Nothing => {}
        }
        self.redraw();
        true
    }

    /// Vista de un panel: se aplica a todas sus pestañas (y la toman las nuevas).
    pub(super) fn set_view(&mut self, pane: usize, view: list::View) {
        for tab in &mut self.panes[pane].tabs {
            tab.list.set_view(view);
        }
        self.notice = Some(format!("Vista: {}", view.name().to_lowercase()));
        self.redraw();
    }

    /// Ctrl+rueda: vista siguiente (más grande) o anterior sobre el panel.
    pub(super) fn cycle_view(&mut self, pane: usize, bigger: bool) {
        let i = self.panes[pane].tab().list.view.index();
        let j = if bigger { (i + 1).min(list::VIEWS.len() - 1) } else { i.saturating_sub(1) };
        if i != j {
            self.set_view(pane, list::VIEWS[j].0);
        }
    }

    /// Las columnas son las mismas en todas las listas (y se guardan en la sesión).
    pub(super) fn set_columns(&mut self, columns: Vec<ColumnSpec>) {
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.list.columns = columns.clone();
        }
        self.columns = columns;
        self.redraw();
    }

    /// Menú del clic derecho en el encabezado: mostrar u ocultar columnas.
    pub(super) fn show_columns_menu(&mut self, pane: usize, screen: POINT) {
        const RESET: usize = 100;
        let current = self.panes[pane].tab().list.columns.clone();
        let cmd = unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            for (i, &(col, title, _, _)) in list::COLUMNS.iter().enumerate() {
                let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
                let mut flags = MF_STRING;
                if current.iter().any(|c| c.col == col) {
                    flags |= MF_CHECKED;
                }
                // El nombre no se puede ocultar.
                if col == list::Column::Name {
                    flags |= MF_GRAYED;
                }
                let _ = AppendMenuW(hmenu, flags, i + 1, PCWSTR(wide.as_ptr()));
            }
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            let wide: Vec<u16> = "Columnas de fábrica".encode_utf16().chain(Some(0)).collect();
            let _ = AppendMenuW(hmenu, MF_STRING, RESET, PCWSTR(wide.as_ptr()));
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN).0, screen.x, screen.y, self.hwnd.hwnd(), None).0 as usize;
            let _ = DestroyMenu(hmenu);
            cmd
        };
        match cmd {
            RESET => {
                for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
                    tab.list.reset_name_width();
                }
                self.set_columns(list::default_columns());
            }
            i if (1..=list::COLUMNS.len()).contains(&i) => {
                let list = &mut self.panes[pane].tab_mut().list;
                list.toggle_column(list::COLUMNS[i - 1].0);
                let cols = list.columns.clone();
                self.set_columns(cols);
            }
            _ => {}
        }
    }

    pub(super) fn restore_panes(&mut self, s: &Session, start_path: Option<PathBuf>) {
        let fallback = default_session();
        for i in 0..PANES {
            let state = s.panes.get(i).or(fallback.panes.get(i)).cloned().unwrap_or_default();
            let locations = if state.tabs.is_empty() { vec![Location::Computer] } else { state.tabs };
            let view = s.views.get(i).copied().unwrap_or(list::View::Details);
            let tabs: Vec<Tab> = locations
                .into_iter()
                .map(|l| {
                    let mut t = self.make_tab(l);
                    t.list.view = view;
                    t
                })
                .collect();
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
            show_tree: self.tree_visible(),
            tree_pinned: self.tree_pinned(),
            quick_open: self.tree_quick_open(),
            accent: self.accent,
            dark_mode: self.dark_mode,
            group: self.group_name.clone(),
            columns: self.columns.clone(),
            views: self.panes.iter().map(|p| p.tab().list.view).collect(),
            panes: self
                .panes
                .iter()
                .map(|p| PaneState { tabs: p.tabs.iter().map(|t| t.location.clone()).collect(), active: p.active })
                .collect(),
            ..Default::default()
        };
        let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        if unsafe { GetWindowPlacement(self.hwnd.hwnd(), &mut wp) }.is_ok() {
            let r = wp.rcNormalPosition;
            s.window = Some([r.left, r.top, r.right - r.left, r.bottom - r.top]);
            s.maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32;
        }
        s
    }

    pub(super) fn show(&mut self) {
        unsafe {
            match self.window_state.take() {
                Some(([x, y, w, h], maximized)) => {
                    let wp = WINDOWPLACEMENT {
                        length: size_of::<WINDOWPLACEMENT>() as u32,
                        showCmd: if maximized { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                        rcNormalPosition: RECT { left: x, top: y, right: x + w, bottom: y + h },
                        ..Default::default()
                    };
                    // Ajusta solo si la posición guardada quedó fuera de los monitores actuales.
                    let _ = SetWindowPlacement(self.hwnd.hwnd(), &wp);
                }
                None => {
                    let dpi = GetDpiForWindow(self.hwnd.hwnd());
                    let _ = SetWindowPos(
                        self.hwnd.hwnd(),
                        None,
                        0,
                        0,
                        scale(INITIAL_SIZE.0, dpi),
                        scale(INITIAL_SIZE.1, dpi),
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    let _ = ShowWindow(self.hwnd.hwnd(), SW_SHOW);
                }
            }
        }
    }

    /// Solo las pestañas a la vista cargan y vigilan su carpeta.
    pub(super) fn sync_visibility(&mut self) {
        let (hwnd, tx, visible_panes) = (self.hwnd, self.tx.clone(), self.layout.panes());
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let active = pane.active;
            for (j, tab) in pane.tabs.iter_mut().enumerate() {
                tab.set_visible(hwnd, &tx, i < visible_panes && j == active);
            }
        }
    }

    pub(super) fn set_active(&mut self, pane: usize) {
        if pane < self.layout.panes() && pane != self.active {
            self.end_edit(false);
            // Ir a otro panel restaura el layout si había uno maximizado.
            self.maximized = None;
            self.active = pane;
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
    /// `select <panel> <pos>`, `active <panel>` y `notice <texto>` dejan la interfaz en un estado
    /// que se quiere mostrar.
    #[cfg(feature = "pruebas")]
    pub(super) fn test_hook(&mut self, text: &str) {
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
            "active" => self.set_active(na),
            "notice" => self.notice = Some(format!("{a} {b}").trim().to_string()),
            "close" => self.close_palette(),
            _ => {}
        }
        self.redraw();
    }

    /// Animaciones: la opción de `config.toml` y la preferencia de Windows tienen que permitirlas.
    pub(super) fn apply_animation_setting(&mut self) {
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
    pub(super) fn animated_rects(&mut self, target: Vec<(usize, render::Rect)>) -> (Vec<(usize, render::Rect)>, bool) {
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
        let mut tab = self.make_tab(location);
        tab.list.view = self.panes[pane].tab().list.view;
        self.panes[pane].add_tab(tab, activate);
        self.sync_visibility();
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

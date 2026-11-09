//! Teclado, comandos y acciones de los paneles.

use super::*;

impl App {
    /// Atajos de la ventana. Devuelve si la tecla se usó.
    pub fn on_key(&mut self, vk: Key) -> bool {
        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
        // Ctrl+Alt (y AltGr) en Windows y ⌥⌘ en macOS llegan acá, no a `on_alt_key`: sin mirar
        // Alt, Ctrl+Alt+N se buscaba como Ctrl+N.
        let alt = key_down(VK_MENU);
        let active = self.active;
        match vk {
            VK_ESCAPE if self.about_open => self.about_open = false,
            VK_ESCAPE if self.picker_open => self.picker_open = false,
            VK_ESCAPE if self.term_menu.is_some() => self.term_menu = None,
            // Con xtree abierto, sus teclas van primero; de las de xdir quedan solo las globales.
            _ if self.xtree_open => return self.on_xtree_key(vk, ctrl, shift),
            VK_ESCAPE if self.panes[active].tab().search.is_some() => self.clear_filter(active),
            _ => match self.config.keymap.lookup(vk, ctrl, shift, alt) {
                Some(c) => self.run_command(c),
                None => match self.tab_mut().list.on_key(vk.0, ctrl, shift) {
                    Some(action) => {
                        self.apply(active, action);
                        return true;
                    }
                    None => return false,
                },
            },
        }
        self.redraw();
        true
    }

    /// Ejecuta una acción sobre el panel activo (atajo de teclado o paleta de comandos).
    pub fn run_command(&mut self, c: Command) {
        // La vista de xtree tapa los paneles: lo que los cambia la cierra para que se vea el efecto.
        if self.xtree_open && !keeps_xtree(c) {
            self.close_xtree();
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let active = self.active;
        match c {
            Command::JumpTo => self.open_palette(PaletteMode::Folders),
            Command::Palette => self.open_palette(PaletteMode::All),
            Command::NewTab => {
                let loc = self.panes[active].tab().location.clone();
                self.new_tab(active, loc, true);
            }
            Command::CloseTab => {
                let i = self.panes[active].active;
                self.close_tab(active, i);
            }
            Command::ReopenTab => {
                if let Some(loc) = self.panes[active].pop_closed() {
                    self.new_tab(active, loc, true);
                }
            }
            Command::NextTab | Command::PrevTab => {
                self.panes[active].cycle_tab(c == Command::NextTab);
                self.sync_visibility();
            }
            Command::NextGroup | Command::PrevGroup => {
                let n = self.groups.items.len();
                if n == 0 {
                    // Sin grupos, el atajo sigue siendo útil: pasa de pestaña.
                    self.panes[active].cycle_tab(c == Command::NextGroup);
                    self.sync_visibility();
                } else {
                    let cur = self.group_name.as_deref().and_then(|g| self.groups.find(g));
                    let next = match (cur, c == Command::NextGroup) {
                        (Some(i), true) => (i + 1) % n,
                        (Some(i), false) => (i + n - 1) % n,
                        (None, true) => 0,
                        (None, false) => n - 1,
                    };
                    self.switch_group(next);
                }
            }
            Command::NextPane | Command::PrevPane => {
                let n = self.layout.panes();
                self.set_active(if c == Command::PrevPane { (active + n - 1) % n } else { (active + 1) % n });
            }
            Command::FocusPane(i) => self.set_active(i as usize),
            Command::Layout(i) => {
                if let Some(l) = Layout::from_index(i as usize) {
                    self.set_layout(l);
                }
            }
            Command::Maximize => self.toggle_maximize(active),
            Command::Back => self.tab_mut().go_back(hwnd, &tx),
            Command::Forward => self.tab_mut().go_forward(hwnd, &tx),
            Command::Up => self.tab_mut().go_up(hwnd, &tx),
            Command::Refresh => self.tab_mut().reload(hwnd, &tx, false),
            Command::EditAddress => self.begin_edit(active),
            Command::Filter => self.begin_filter(active),
            Command::GoComputer => self.tab_mut().navigate(hwnd, &tx, Location::Computer),
            Command::ToggleHidden => {
                // En todas las pestañas a la vez.
                self.show_hidden = !self.show_hidden;
                for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
                    if tab.list.show_hidden != self.show_hidden {
                        tab.list.toggle_hidden();
                    }
                }
            }
            Command::SelectAll => self.tab_mut().list.select_all(),
            Command::InvertSelection => self.tab_mut().list.invert_selection(),
            Command::SelectPattern => self.open_palette(PaletteMode::Pattern { select: true }),
            Command::DeselectPattern => self.open_palette(PaletteMode::Pattern { select: false }),
            Command::Copy => self.copy_selection(false),
            Command::Cut => self.copy_selection(true),
            Command::Paste => self.paste(),
            Command::CopyPaths => {
                let paths = self.selection(active);
                if !paths.is_empty() {
                    let _ = sys::shell().copy_text(hwnd, &fileops::quoted_paths(&paths));
                }
            }
            Command::Undo => {
                if let Some(undo) = self.undo.pop() {
                    sys::file_ops().spawn(hwnd, self.ops_tx.clone(), Op::Undo(undo));
                }
            }
            Command::NewFolder => {
                if let Some(parent) = self.panes[active].tab().location.target() {
                    sys::file_ops().spawn(hwnd, self.ops_tx.clone(), Op::NewFolder { parent });
                }
            }
            Command::NewTextFile => {
                if let Some(parent) = self.panes[active].tab().location.target() {
                    sys::file_ops().spawn(hwnd, self.ops_tx.clone(), Op::NewFile { parent });
                }
            }
            Command::Recycle => self.delete_selection(true),
            Command::Delete => self.delete_selection(false),
            Command::Rename => self.begin_rename(active),
            Command::Properties => {
                // De la selección, o de la carpeta si no hay nada seleccionado.
                let items = self.selection_items(active);
                match (&items[..], &self.panes[active].tab().location) {
                    ([], Location::Path(p)) => sys::shell().properties(hwnd, &[p.clone().into()]),
                    _ => sys::shell().properties(hwnd, &items),
                }
            }
            Command::ToggleFavorite => self.toggle_favorite(active),
            Command::ToggleFavBar => self.show_favorites = !self.show_favorites,
            Command::ToggleGroupBar => {
                self.show_group_bar = !self.show_group_bar;
                self.ensure_group();
            }
            Command::PowerShell => self.open_console(active, Console::nth(0)),
            Command::Cmd => self.open_console(active, Console::nth(1)),
            Command::ToggleDarkMode => self.set_dark_mode(!self.theme.dark),
            Command::SaveGroup => self.open_palette(PaletteMode::GroupName),
            Command::View(i) => {
                if let Some(&(view, ..)) = list::VIEWS.get(i as usize) {
                    self.set_view(active, view);
                }
            }
            Command::SortBy(i) => {
                if let Some(&(col, ..)) = list::COLUMNS.get(i as usize) {
                    self.tab_mut().list.set_sort(col);
                }
            }
            Command::About => {
                self.picker_open = false;
                self.term_menu = None;
                self.about_open = true;
            }
            Command::ToggleTree => {
                #[cfg(feature = "arbol")]
                {
                    self.begin_layout_anim();
                    self.tree.visible = !self.tree.visible;
                }
            }
            Command::ToggleTreePin => {
                #[cfg(feature = "arbol")]
                {
                    // El ancho del árbol y el lugar que se le reserva se animan cuadro a cuadro
                    // (ver `tree.tick` al dibujar); los paneles siguen ese ancho.
                    self.tree.set_pinned(!self.tree.pinned, true);
                }
            }
            Command::ToggleQuickPin => {
                #[cfg(feature = "arbol")]
                if let Location::Path(p) = self.panes[self.active].tab().location.clone() {
                    self.toggle_quick_pin(p);
                }
            }
            Command::Xtree => self.toggle_xtree(),
            Command::OpenConfig => {
                if let Some(path) = config::file_path() {
                    if !path.exists() {
                        self.config = Config::load();
                    }
                    self.host.open_in_editor(&path);
                }
            }
        }
        self.redraw();
    }


    /// Tecla con la vista de xtree abierta: primero la vista (T, F5, Retroceso, Esc, A); después
    /// solo los atajos de la ventana que no tocan los paneles tapados.
    fn on_xtree_key(&mut self, vk: Key, ctrl: bool, shift: bool) -> bool {
        if !ctrl {
            let action = self.xtree.as_mut().map_or(xtree_view::Action::None, |x| x.on_key(vk));
            if action != xtree_view::Action::None {
                self.xtree_action(action);
                return true;
            }
        }
        match self.config.keymap.lookup(vk, ctrl, shift, false) {
            Some(c @ (Command::Palette | Command::Xtree | Command::ToggleDarkMode | Command::About | Command::OpenConfig)) => {
                self.run_command(c);
                true
            }
            _ => false,
        }
    }

    /// Abre la vista de xtree (o la cierra si estaba abierta). La primera vez analiza la carpeta
    /// del panel activo; después se reabre con lo que ya tenía, sin volver a analizar.
    pub(super) fn toggle_xtree(&mut self) {
        if self.xtree_open {
            self.close_xtree();
        } else {
            if self.xtree.is_none() {
                let root = match &self.panes[self.active].tab().location {
                    Location::Path(p) => p.clone(),
                    _ => crate::sys::disk_scan().default_root(),
                };
                self.xtree = Some(Xtree::new(self.hwnd, root));
            }
            // Lo que flota sobre los paneles se cierra: quedaría encima de la vista.
            self.picker_open = false;
            self.term_menu = None;
            self.end_edit(false);
            self.end_filter(true);
            self.xtree_open = true;
        }
        self.hover = Hit::None;
        self.redraw();
    }

    /// Cierra la vista de xtree (el escaneo sigue) y su menú de discos.
    pub(super) fn close_xtree(&mut self) {
        self.xtree_open = false;
        if let Some(x) = &mut self.xtree {
            x.close_menu();
        }
        self.hover = Hit::None;
    }

    /// Lo que pide la vista de xtree después de un evento.
    pub(super) fn xtree_action(&mut self, action: xtree_view::Action) {
        match action {
            xtree_view::Action::None => return,
            xtree_view::Action::Redraw => {}
            xtree_view::Action::Close => self.close_xtree(),
            xtree_view::Action::OpenInPanel { folder, select } => {
                self.close_xtree();
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                let tab = self.tab_mut();
                tab.navigate(hwnd, &tx, Location::Path(folder));
                if let Some(name) = select {
                    tab.focus_after_load(name);
                }
            }
        }
        self.redraw();
    }

    /// Área de la vista de xtree en la ventana.
    pub(super) fn xtree_area(&self) -> geom::Rect {
        let (w, h) = self.client_dips();
        ui::xtree_area(w, h)
    }

    pub(super) fn on_button(&mut self, pane: usize, b: Button) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab_mut();
        match b {
            Button::Back => tab.go_back(hwnd, &tx),
            Button::Forward => tab.go_forward(hwnd, &tx),
            Button::Up => tab.go_up(hwnd, &tx),
            Button::Refresh => tab.reload(hwnd, &tx, false),
        }
    }

    pub(super) fn apply(&mut self, pane: usize, action: Action) {
        match action {
            Action::None => return,
            Action::Redraw => {}
            Action::Open(idx) => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                self.panes[pane].tab_mut().open(hwnd, &tx, idx);
            }
            Action::BeginDrag => {
                self.start_drag(pane);
                return;
            }
            Action::ColumnsChanged => {
                let cols = self.panes[pane].tab().list.columns.clone();
                self.set_columns(cols);
            }
        }
        self.redraw();
    }

    pub fn redraw(&self) {
        self.host.invalidate();
    }

    /// Posición del mouse en DIPs del área de la ventana.
    pub fn cursor_pos(&self) -> (f32, f32) {
        self.host.cursor_dips()
    }
}

/// Comandos que se pueden usar con la vista de xtree abierta sin cerrarla: no tocan los paneles
/// (o solo abren la paleta, y lo que se elija ahí decide).
fn keeps_xtree(c: Command) -> bool {
    matches!(
        c,
        Command::Palette
            | Command::JumpTo
            | Command::SaveGroup
            | Command::Xtree
            | Command::ToggleDarkMode
            | Command::About
            | Command::OpenConfig
    )
}

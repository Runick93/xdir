//! Teclado, comandos y acciones de los paneles.

use super::*;

impl App {
    /// Atajos de la ventana. Devuelve si la tecla se usó.
    pub fn on_key(&mut self, vk: Key) -> bool {
        self.dismiss_tooltip();
        let (ctrl, shift) = (key_down(VK_CONTROL), key_down(VK_SHIFT));
        // Ctrl+Alt (y AltGr) en Windows y ⌥⌘ en macOS llegan acá, no a `on_alt_key`: sin mirar
        // Alt, Ctrl+Alt+N se buscaba como Ctrl+N.
        let alt = key_down(VK_MENU);
        // Meta: la tecla Windows / Super (⌃ en macOS), para atajos como ⌃⌘N.
        let meta = key_down(VK_LWIN);
        // El Espacio solo se traga si es el carácter que sigue a esta tecla.
        self.swallow_space = false;
        // Grabando un atajo en la configuración, la tecla es para eso (ni ⌘W ni Tab llegan).
        if let Some(used) = self.settings_capture_key(vk, alt) {
            return used;
        }
        // La vista rápida va antes que los atajos: Espacio la abre o la cierra, Esc la cierra y
        // las flechas la recorren.
        if let Some(used) = self.quick_look_key(vk, ctrl, shift, alt) {
            return used;
        }
        let used = self.on_window_key(vk, ctrl, shift, alt, meta);
        // Con la vista rápida recorriendo la carpeta, sigue al foco de la lista.
        self.quick_look_follow_list();
        used
    }

    /// `on_key` después de la vista rápida: Escape de lo que flota, xtree, los atajos y la lista.
    fn on_window_key(&mut self, vk: Key, ctrl: bool, shift: bool, alt: bool, meta: bool) -> bool {
        let active = self.active;
        match vk {
            VK_ESCAPE if self.about_open => self.about_open = false,
            VK_ESCAPE if self.net.dialog.is_some() => self.close_connect_dialog(),
            VK_ESCAPE if self.net.map.is_some() => self.close_map_dialog(),
            VK_ESCAPE if self.net.unmap.is_some() => self.net.unmap = None,
            VK_ESCAPE if self.picker_open => self.picker_open = false,
            VK_ESCAPE if self.term_menu.is_some() => self.term_menu = None,
            VK_ESCAPE if self.filter_menu_pane().is_some() => {
                self.close_filter_menus();
            }
            VK_ESCAPE if self.rename_many.is_some() => self.close_rename_many(),
            VK_ESCAPE if self.info_open() && self.info.focused => self.close_info(),
            // Con xtree abierto, sus teclas van primero; de las de xdir quedan solo las globales.
            _ if self.xtree_open => return self.on_xtree_key(vk, ctrl, shift),
            VK_ESCAPE if self.panes[active].tab().search.is_some() => self.clear_filter(active),
            // Lo último: si nada más tomó el Escape, corta la apertura desde un archivo comprimido.
            VK_ESCAPE if self.archive.opening.as_ref().is_some_and(|s| !s.load(std::sync::atomic::Ordering::Relaxed)) => {
                self.cancel_archive_open();
            }
            _ => match self.config.keymap.lookup(vk, ctrl, shift, alt, meta) {
                Some(c) => self.run_command(c),
                // Las flechas, RePág/AvPág, Inicio y Fin desplazan las novedades.
                None if !ctrl && !alt && !meta && self.changelog_key(vk).is_some() => {}
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
        let from_xtree = self.xtree_open;
        if self.xtree_open && !keeps_xtree(c) {
            self.close_xtree();
        }
        // Un comando (del menú, de la paleta, de un botón) termina la grabación de un atajo.
        self.end_recording();
        // Dentro de un archivo comprimido, lo que escribe no se hace (aviso).
        if self.refuse_in_archive(c) {
            return;
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let active = self.active;
        // Con la configuración o las novedades a la vista, lo que necesita una carpeta no hace nada.
        if self.panes[active].tab().is_page() && needs_folder(c) {
            self.redraw();
            return;
        }
        match c {
            Command::JumpTo => self.open_palette(PaletteMode::Folders),
            Command::Palette => self.open_palette(PaletteMode::All),
            Command::NewTab => {
                let loc = input::new_tab_location(&self.panes[active].tab().location);
                self.new_tab(active, loc, true);
            }
            Command::CloseTab => {
                let i = self.panes[active].active;
                self.close_tab(active, i);
            }
            Command::ReopenTab => {
                match self.panes[active].pop_closed() {
                    // Hay una sola configuración: si ya está abierta, se enfoca.
                    Some(Location::Settings) => self.open_settings(None),
                    Some(Location::Changelog) => self.open_changelog(None),
                    Some(loc) => self.new_tab(active, loc, true),
                    None => {}
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
            Command::Refresh => {
                if let Location::Path(dir) = self.panes[active].tab().location.clone() {
                    self.sel_size.invalidate(&dir);
                }
                self.tab_mut().reload(hwnd, &tx, false)
            }
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
            Command::ShowTransfers => self.show_transfers(),
            // Dentro de un archivo comprimido, la ruta de adentro (la del menú).
            Command::CopyPaths if self.in_archive() => self.copy_archive_paths(active),
            Command::CopyPaths => {
                let paths = self.selection(active);
                if !paths.is_empty() {
                    let _ = sys::shell().copy_text(hwnd, &fileops::quoted_paths(&paths));
                }
            }
            Command::Undo => match self.undo.pop() {
                // Lo que armó el motor de copia lo deshace el motor (un paso, con su progreso).
                Some(undo @ (Undo::Batch(_) | Undo::CreatedFolder(_) | Undo::Moved(_) | Undo::CreatedIds(_))) => {
                    self.start_transfer(crate::transfer::JobSpec::Undo(undo));
                }
                // Renombrar varios se deshace en el núcleo, con el mismo cruce seguro.
                Some(Undo::Renames(renames)) => self.undo_renames(renames),
                Some(undo) => sys::file_ops().spawn(hwnd, self.ops_tx.clone(), Op::Undo(undo)),
                None => {}
            },
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
            Command::NewFolderWithSelection => self.new_folder_with_selection(),
            // En la Papelera de la Mac, `Recycle` devuelve (como ⌘⌫ en el Finder). Es el mismo
            // comando con sus dos atajos, ⌘⌫ y Supr (fn+⌫): los dos devuelven, a propósito (borrar
            // definitivamente desde la Papelera sigue siendo `Delete`: ⌥⌘⌫ o ⇧Supr). En Windows y
            // Linux, Supr en la Papelera sigue siendo borrar.
            Command::Recycle if sys::env().key_style() == sys::KeyStyle::Mac && sys::file_system().is_trash(&self.panes[active].tab().location) => {
                self.restore_selection()
            }
            Command::Recycle => self.delete_selection(true),
            Command::Restore => self.restore_selection(),
            Command::CreateShortcut => self.create_shortcuts(),
            Command::ShowOriginal => self.show_original(),
            Command::Compress => self.compress_selection(),
            Command::ExtractHere => self.extract_selection(false),
            Command::ExtractInto => self.extract_selection(true),
            Command::Eject => {
                if let Some(root) = self.eject_target() {
                    self.eject(root);
                }
            }
            Command::ConnectServer => self.open_connect_dialog(),
            Command::MapNetworkDrive => self.open_map_dialog(),
            Command::DisconnectNetworkDrive => self.open_unmap_dialog(),
            Command::Delete => self.delete_selection(false),
            Command::Rename => self.rename_command(active),
            Command::RenameMany => self.open_rename_many(active),
            // El panel de información (lo de la selección, la carpeta o el disco enfocado).
            Command::Properties => self.toggle_info(),
            Command::Duplicate => self.duplicate_selection(),
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
                    self.sync_folder_views();
                }
            }
            Command::GroupBy(i) => {
                if let Some(&col) = list::GROUPINGS.get(i as usize) {
                    self.tab_mut().list.set_group(col);
                    self.sync_folder_views();
                }
            }
            Command::ApplyViewToAll => self.apply_view_to_all(self.active),
            Command::ResetFolderView => self.reset_folder_view(self.active),
            Command::ClearFilters => {
                self.tab_mut().list.clear_filters();
                self.sync_folder_views();
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
            // Con xtree abierto la columna no se ve: el botón o el atajo cierran xtree y la muestran.
            Command::TogglePreview if from_xtree => self.show_preview(),
            Command::TogglePreview => self.toggle_preview(),
            Command::QuickLook => self.toggle_quick_look(),
            Command::Settings => self.open_settings(None),
            Command::Changelog => self.open_changelog(None),
            Command::SettingsKeys => self.open_settings(Some(crate::settings::PageId::Builtin(crate::settings::Section::Keys))),
            Command::OpenConfig => {
                if let Some(path) = config::file_path() {
                    if !path.exists() {
                        self.config = Config::load();
                    }
                    self.host.open_in_editor(&path);
                }
            }
        }
        // Ordenar, agrupar, filtrar u ocultar cambian el recorrido de la vista rápida.
        if c != Command::QuickLook {
            self.sync_quick_look();
        }
        // El orden de la lista (y el lugar del panel de la derecha) pudo cambiar.
        self.refresh_rename_many();
        self.sync_rename_boxes();
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
        match self.config.keymap.lookup(vk, ctrl, shift, false, key_down(VK_LWIN)) {
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
            // Una vista que no analizó nada se arma de nuevo con la raíz de ahora (`xtree_abrir`)
            // y la opción de ahora; solo se conserva lo que se eligió a mano en el desplegable.
            if self.xtree.as_ref().is_none_or(|x| !x.scanned()) {
                let picked = self.xtree.as_ref().and_then(|x| x.user_pick());
                let root = picked.unwrap_or_else(|| self.xtree_root());
                // De fábrica no se analiza solo: la vista vacía con la raíz en el desplegable.
                self.xtree = Some(if self.config.xtree_scan_on_open {
                    self.xtree_last = Some(root.clone());
                    Xtree::new(self.hwnd, root, self.config.xtree_by_type)
                } else {
                    Xtree::unscanned(self.hwnd, root, self.config.xtree_by_type)
                });
            }
            // Lo que flota sobre los paneles se cierra: quedaría encima de la vista.
            self.close_quick_look();
            self.close_rename_many();
            self.picker_open = false;
            self.term_menu = None;
            self.close_filter_menus();
            self.end_edit(false);
            self.end_filter(true);
            self.xtree_open = true;
            // El resultado terminado ya se ve: el indicador de la barra vuelve al botón de siempre.
            if let Some(x) = &mut self.xtree {
                x.mark_seen();
            }
        }
        self.hover = Hit::None;
        self.redraw();
    }

    /// Qué analiza xtree al abrirse (`[opciones] xtree_abrir`): la carpeta del panel (si no es una
    /// carpeta, el disco del sistema), el disco del sistema o la última analizada (si ya no existe,
    /// la del panel).
    pub(super) fn xtree_root(&self) -> PathBuf {
        let pane = || match &self.panes[self.active].tab().location {
            Location::Path(p) => p.clone(),
            _ => crate::sys::disk_scan().default_root(),
        };
        match self.config.xtree_open {
            config::XtreeOpen::Pane => pane(),
            config::XtreeOpen::System => crate::sys::disk_scan().default_root(),
            // El último analizado, si sigue (sin mirar el disco si es de red).
            config::XtreeOpen::Last => self.xtree_last.clone().filter(|p| crate::fs::probe_dir(p) != Some(false)).unwrap_or_else(pane),
        }
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

    /// Estado del escaneo de xtree para el indicador de la barra de estado.
    pub(super) fn xtree_badge(&self) -> XtreeBadge {
        self.xtree.as_ref().map_or(XtreeBadge::Idle, |x| x.badge())
    }

    /// Botón «xtree» de la barra de estado (más ancho mientras muestra el porcentaje).
    pub(super) fn xtree_button_rect(&self) -> geom::Rect {
        let (w, h) = self.client_dips();
        ui::xtree_rect(w, h, self.xtree_badge())
    }

    /// Indicador de las operaciones de copia, a la izquierda del botón «xtree».
    pub(super) fn transfers_button_rect(&self) -> geom::Rect {
        let (w, h) = self.client_dips();
        ui::transfers_rect(w, h, self.xtree_badge())
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

    /// Menú del historial de Atrás (o Adelante) de la pestaña activa del panel: hasta
    /// `HISTORY_MENU_MAX` carpetas, la más cercana primero. Elegir la i-ésima equivale a apretar
    /// el botón i veces.
    pub(super) fn history_menu(&mut self, pane: usize, forward: bool) {
        let tab = self.panes[pane].tab();
        let list = if forward { tab.forward_list(HISTORY_MENU_MAX) } else { tab.back_list(HISTORY_MENU_MAX) };
        if list.is_empty() {
            return;
        }
        let entries: Vec<shell::Entry> = list
            .iter()
            .enumerate()
            .map(|(i, loc)| shell::Entry::Item { id: i as u32 + 1, label: loc.title(), enabled: true, checked: false })
            .collect();
        let Some(id) = self.host.popup_menu(&entries, MenuPos::Cursor, None, None) else { return };
        let n = id as usize;
        if n == 0 || n > list.len() {
            return;
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab_mut();
        if forward {
            tab.go_forward_n(n, hwnd, &tx);
        } else {
            tab.go_back_n(n, hwnd, &tx);
        }
        self.redraw();
    }

    pub(super) fn apply(&mut self, pane: usize, action: Action) {
        match action {
            Action::None => return,
            Action::Redraw => {}
            Action::Open(idx) => self.open_entry(pane, idx),
            Action::BeginDrag => {
                self.start_drag(pane);
                return;
            }
            // Ordenar con un clic en el encabezado, anchos y columnas: quedan para la carpeta.
            Action::ColumnsChanged => {}
        }
        self.sync_folder_views();
        // Orden, filtros o grupos plegados: la vista rápida rearma su recorrido.
        self.sync_quick_look();
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

/// Máximo de carpetas en el menú del historial de Atrás y Adelante.
const HISTORY_MENU_MAX: usize = 15;

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

/// Comandos que trabajan sobre la carpeta o la lista del panel: con la pestaña Configuración a
/// la vista no hacen nada.
fn needs_folder(c: Command) -> bool {
    matches!(
        c,
        Command::Back
            | Command::Forward
            | Command::Up
            | Command::Refresh
            | Command::SelectAll
            | Command::InvertSelection
            | Command::SelectPattern
            | Command::DeselectPattern
            | Command::Copy
            | Command::Cut
            | Command::Paste
            | Command::CopyPaths
            | Command::NewFolder
            | Command::NewTextFile
            | Command::NewFolderWithSelection
            | Command::Recycle
            | Command::Delete
            | Command::Restore
            | Command::CreateShortcut
            | Command::ShowOriginal
            | Command::Rename
            | Command::RenameMany
            | Command::Properties
            | Command::Duplicate
            | Command::Compress
            | Command::ExtractHere
            | Command::ExtractInto
            | Command::ToggleFavorite
            | Command::PowerShell
            | Command::Cmd
            | Command::View(_)
            | Command::SortBy(_)
            | Command::GroupBy(_)
            | Command::ApplyViewToAll
            | Command::ResetFolderView
            | Command::ClearFilters
            | Command::QuickLook
            | Command::ToggleQuickPin
    )
}

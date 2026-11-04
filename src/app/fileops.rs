//! Operaciones de archivos: portapapeles, borrar, arrastrar y soltar, menú contextual.

use super::*;

impl App {
    // ---------- operaciones de archivos ----------

    /// Rutas seleccionadas en la pestaña activa de un panel.
    pub(super) fn selection(&self, pane: usize) -> Vec<PathBuf> {
        let tab = self.panes[pane].tab();
        tab.list.selected_indices().map(|i| tab.entry_path(i)).collect()
    }

    /// Selección como elementos del Shell (con PIDL si son virtuales).
    pub(super) fn selection_items(&self, pane: usize) -> Vec<shell::Item> {
        let tab = self.panes[pane].tab();
        tab.list.selected_indices().map(|i| tab.entry_item(i)).collect()
    }

    pub(super) fn copy_selection(&mut self, cut: bool) {
        let items = self.selection_items(self.active);
        if !items.is_empty() {
            if let Err(e) = shell::copy_files(&items, cut) {
                self.notice = Some(format!("No se pudo copiar: {}", e.message()));
            }
        }
    }

    pub(super) fn paste(&mut self) {
        let Some(dest) = self.panes[self.active].tab().location.target() else { return };
        let Some((sources, cut)) = shell::clipboard_files() else {
            // Sin rutas de disco (copiado por RDP, desde un zip...): pega la carpeta destino.
            match shell::paste_virtual(self.hwnd, &self.panes[self.active].tab().location.clone()) {
                Ok(true) => self.reload_shell_tabs(),
                Ok(false) => {}
                Err(e) => self.notice = Some(format!("No se pudo pegar: {}", e.message())),
            }
            return;
        };
        let op = if cut { Op::Move { sources, dest } } else { Op::Copy { sources, dest } };
        ops::spawn(self.hwnd, self.ops_tx.clone(), op);
        // Lo cortado se pega una sola vez, como en el Explorador.
        if cut {
            shell::clear_clipboard();
        }
    }

    pub(super) fn delete_selection(&mut self, recycle: bool) {
        // En carpetas virtuales borra el propio Shell (en la Papelera: eliminar definitivamente).
        let location = self.panes[self.active].tab().location.clone();
        if let Location::Shell { .. } = location {
            let items = self.selection_items(self.active);
            if !items.is_empty() {
                let _ = shell::invoke_verb(self.hwnd, &location, &items, "delete");
                self.reload_shell_tabs();
            }
            return;
        }
        let paths = self.selection(self.active);
        if !paths.is_empty() && self.panes[self.active].tab().location.target().is_some() {
            ops::spawn(self.hwnd, self.ops_tx.clone(), Op::Delete { paths, recycle });
        }
    }

    pub(super) fn on_op_done(&mut self, result: OpResult) {
        self.notice = result.error.map(|e| format!("Error: {e}"));
        if let Some(undo) = result.undo {
            self.undo.push(undo);
            if self.undo.len() > UNDO_MAX {
                self.undo.remove(0);
            }
        }
        self.reload_shell_tabs();
        let Some(path) = result.focus else { return };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        // Las pestañas que muestran esa carpeta enfocan el elemento al recargar.
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if tab.location.is_path(parent) && tab.is_loaded() {
                tab.focus_after_load(name.to_os_string());
                tab.reload(hwnd, &tx, true);
            }
        }
        if result.rename_after {
            self.pending_rename = Some(path);
        }
    }

    /// Las carpetas virtuales no tienen vigilancia: se recargan después de cada operación.
    pub(super) fn reload_shell_tabs(&mut self) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let visible = self.layout.panes();
        for pane in &mut self.panes[..visible] {
            let tab = pane.tab_mut();
            if matches!(tab.location, Location::Shell { .. }) && !tab.loading {
                tab.reload(hwnd, &tx, true);
            }
        }
    }

    /// Tras elegir algo del menú del fondo: si apareció un elemento nuevo en la carpeta, lo selecciona
    /// y abre su nombre para escribirlo ("Nuevo > Carpeta" lo crea el Shell con el nombre por defecto).
    pub(super) fn try_new_watch(&mut self) {
        let Some(w) = &self.new_watch else { return };
        if w.since.elapsed().as_secs() > 5 {
            self.new_watch = None;
            return;
        }
        let pane = w.pane;
        let tab = self.panes[pane].tab_mut();
        if tab.loading {
            return;
        }
        let Some(idx) = tab.list.entries.iter().position(|e| !w.before.contains(&e.display.to_lowercase())) else { return };
        tab.list.select_index(idx);
        self.new_watch = None;
        self.begin_rename(pane);
    }

    /// Abre el renombrado sobre la carpeta recién creada cuando ya aparece en la lista.
    pub(super) fn try_pending_rename(&mut self) {
        let Some(path) = self.pending_rename.clone() else { return };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        let pane = self.active;
        let tab = self.panes[pane].tab_mut();
        if !tab.location.is_path(parent) || tab.loading {
            return;
        }
        if let Some(idx) = tab.list.entries.iter().position(|e| e.name.eq_ignore_ascii_case(name)) {
            tab.list.select_index(idx);
            self.pending_rename = None;
            self.begin_rename(pane);
        }
    }

    pub(super) fn start_drag(&mut self, pane: usize) {
        let items = self.selection_items(pane);
        if items.is_empty() {
            return;
        }
        let obj = match shell::data_object(&items) {
            Ok(o) => o,
            Err(e) => {
                self.notice = Some(format!("No se pudo arrastrar: {e}"));
                self.redraw();
                return;
            }
        };
        let result = unsafe {
            let _ = ReleaseCapture();
            self.drag = Drag::None;
            // Bucle modal: mientras dura, los eventos de soltar llegan por `WM_APP_DRAG`.
            SHDoDragDrop(Some(self.hwnd), &obj, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)
        };
        // Cancelar con Esc no es un error; que Windows no pueda arrastrar, sí: se avisa.
        if let Err(e) = result {
            self.notice = Some(format!("No se pudo arrastrar: {e}"));
        }
        self.redraw();
    }

    /// Carpeta destino bajo un punto: una carpeta de la lista o la carpeta del panel.
    pub(super) fn drop_folder(&self, x: f32, y: f32) -> Option<(usize, Option<usize>, PathBuf)> {
        let Hit::Pane(i, ph) = self.hit(x, y) else { return None };
        let tab = self.panes[i].tab();
        if ph == PaneHit::List {
            if let Some(idx) = tab.list.entry_at(x, y) {
                if tab.list.entries[idx].is_dir {
                    return Some((i, Some(idx), tab.entry_path(idx)));
                }
            }
        }
        tab.location.target().map(|t| (i, None, t))
    }

    pub(super) fn on_drag(&mut self, ev: &mut DragEvent) {
        let mut pt = ev.point;
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let k = 96.0 / self.dpi as f32;
        if self.on_drag_favbar(ev, pt.x as f32 * k, pt.y as f32 * k) {
            return;
        }
        let target = match ev.kind {
            DragKind::Leave => None,
            _ => self.drop_folder(pt.x as f32 * k, pt.y as f32 * k),
        };
        ev.effect = match (&target, ev.paths) {
            (Some((_, _, dest)), Some(paths)) => drop_effect(paths, dest, ev.keys, ev.allowed),
            _ => DROPEFFECT_NONE,
        };

        // Resaltado de la carpeta destino.
        let mut changed = false;
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let want = match &target {
                Some((p, Some(idx), _)) if *p == i && ev.effect != DROPEFFECT_NONE && ev.kind != DragKind::Drop => {
                    Some(*idx)
                }
                _ => None,
            };
            let list = &mut pane.tab_mut().list;
            if list.drop_target != want {
                list.drop_target = want;
                changed = true;
            }
        }
        if changed {
            self.redraw();
        }

        if ev.kind == DragKind::Drop && ev.effect != DROPEFFECT_NONE {
            if let (Some((pane, _, dest)), Some(paths)) = (target, ev.paths) {
                let sources = paths.to_vec();
                let op = if ev.effect == DROPEFFECT_MOVE { Op::Move { sources, dest } } else { Op::Copy { sources, dest } };
                ops::spawn(self.hwnd, self.ops_tx.clone(), op);
                self.set_active(pane);
            }
        }
    }

    /// Entradas propias del menú del fondo de una carpeta; `cmds` recibe el comando de cada id.
    pub(super) fn background_entries(&self, pane: usize, cmds: &mut Vec<Command>) -> Vec<shell::Entry> {
        use shell::Entry;
        let list = &self.panes[pane].tab().list;
        let mut item = |label: &str, command: Command, enabled: bool, checked: bool| {
            cmds.push(command);
            Entry::Item { id: cmds.len() as u32 - 1, label: label.to_string(), enabled, checked }
        };
        let views = list::VIEWS
            .iter()
            .enumerate()
            .map(|(i, &(view, name, ..))| item(name, Command::View(i as u8), true, view == list.view))
            .collect();
        let sorts = list::COLUMNS
            .iter()
            .enumerate()
            .map(|(i, &(col, title, ..))| item(title, Command::SortBy(i as u8), true, col == list.sort))
            .collect();
        let undo = item("Deshacer", Command::Undo, !self.undo.is_empty(), false);
        let mut entries = vec![
            Entry::Sub("Ver".into(), views),
            Entry::Sub("Ordenar por".into(), sorts),
            item("Actualizar", Command::Refresh, true, false),
            Entry::Sep,
            item("Pegar", Command::Paste, true, false),
            undo,
        ];
        // Anclar la carpeta que se está viendo al Acceso rápido de Windows (el menú del fondo del
        // Shell no trae ese verbo).
        #[cfg(feature = "arbol")]
        if let Location::Path(p) = &self.panes[pane].tab().location {
            if pane == self.active && p.parent().is_some() {
                let label = if self.tree.is_pinned(p) { "Desanclar de Acceso rápido" } else { "Anclar a Acceso rápido" };
                entries.push(Entry::Sep);
                entries.push(item(label, Command::ToggleQuickPin, true, false));
            }
        }
        entries
    }

    pub(super) fn on_context_menu(&mut self, lparam: LPARAM) {
        let pane = self.active;
        // lparam -1: tecla de menú o Shift+F10; se abre sobre la fila con foco.
        let screen = if lparam.0 == -1 {
            let (x, y) = self.panes[pane].tab().list.focus_point().unwrap_or((40.0, 80.0));
            let k = self.dpi as f32 / 96.0;
            let mut pt = POINT { x: (x * k) as i32, y: (y * k) as i32 };
            unsafe {
                let _ = ClientToScreen(self.hwnd, &mut pt);
            }
            pt
        } else {
            POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 }
        };
        let location = self.panes[pane].tab().location.clone();
        let items = self.selection_items(pane);
        // En carpetas virtuales el Shell resuelve todo: conoce el contexto (p. ej. la Papelera).
        let intercept = !matches!(location, Location::Shell { .. });
        // El menú del fondo del Shell solo trae las extensiones: Ver, Ordenar, Actualizar, Pegar y
        // Deshacer los pone la vista del Explorador, así que se agregan acá.
        let mut cmds = Vec::new();
        let own = if items.is_empty() && intercept { self.background_entries(pane, &mut cmds) } else { Vec::new() };
        let before: Option<std::collections::HashSet<String>> = (items.is_empty() && matches!(location, Location::Path(_)))
            .then(|| self.panes[pane].tab().list.entries.iter().map(|e| e.display.to_lowercase()).collect());
        // Una sola carpeta anclada: su menú dice "Desanclar de Acceso rápido".
        #[cfg(feature = "arbol")]
        let pinned_one = match items.as_slice() {
            [one] if one.pidl.is_none() && one.path.is_dir() => self.tree.is_pinned(&one.path).then(|| one.path.clone()),
            _ => None,
        };
        #[cfg(not(feature = "arbol"))]
        let pinned_one: Option<PathBuf> = None;
        let chosen = shell::context_menu(self.hwnd, &location, &items, screen, key_down(VK_SHIFT), intercept, &own, pinned_one.is_some());
        #[cfg(feature = "arbol")]
        if let (Some(shell::UNPIN_QUICK), Some(path)) = (chosen.as_deref(), pinned_one) {
            self.toggle_quick_pin(path);
            self.redraw();
            return;
        }
        // El menú del Shell puede haber anclado o desanclado una carpeta del Acceso rápido.
        #[cfg(feature = "arbol")]
        if chosen.as_deref() == Some("shell") {
            self.tree.request_quick(self.hwnd);
        }
        if let (Some("shell"), Some(before)) = (chosen.as_deref(), before) {
            self.new_watch = Some(NewWatch { pane, before, since: Instant::now() });
        }
        if let Some(cmd) = chosen.as_deref().and_then(|c| c.strip_prefix("own:")).and_then(|n| n.parse::<usize>().ok()) {
            if let Some(&command) = cmds.get(cmd) {
                self.run_command(command);
            }
            self.redraw();
            return;
        }
        match chosen.as_deref() {
            Some("rename") => self.begin_rename(pane),
            Some("delete") => self.delete_selection(!key_down(VK_SHIFT)),
            Some("cut") => self.copy_selection(true),
            Some("copy") => self.copy_selection(false),
            Some("paste") => self.paste(),
            _ => {}
        }
        self.redraw();
    }
}

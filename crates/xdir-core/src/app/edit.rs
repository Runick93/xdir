//! Renombrado en línea, barra de dirección editable y filtro rápido.

use super::*;

impl App {
    // ---------- renombrado en línea ----------

    pub(super) fn begin_rename(&mut self, pane: usize) {
        if self.rename.is_some() || self.edit.is_some() {
            return;
        }
        let tab = self.panes[pane].tab_mut();
        if tab.location.target().is_none() {
            return;
        }
        let Some(idx) = tab.list.focused_index() else { return };
        let Some(r) = tab.list.name_rect(idx) else { return };
        let name = tab.list.entries[idx].display.clone();
        let is_dir = tab.list.entries[idx].is_dir;
        let path = tab.entry_path(idx);
        // Se selecciona el nombre sin la extensión, como en el Explorador.
        let stem_len = match name.rfind('.') {
            Some(dot) if !is_dir && dot > 0 => name[..dot].encode_utf16().count(),
            _ => name.encode_utf16().count(),
        };
        self.redraw();

        let Some(edit) = self.host.text_create(TextKind::Rename, r, &name, &self.theme) else { return };
        self.host.text_select(edit, 0, stem_len as isize);
        self.host.text_focus(edit);
        self.rename = Some(Rename { edit, path });
    }

    pub(super) fn end_rename(&mut self, commit: bool) {
        let Some(Rename { edit, path }) = self.rename.take() else { return };
        let text = self.host.text_get(edit);
        self.host.text_destroy(edit);
        self.host.focus_main();
        if commit {
            self.commit_rename(path, text.trim().to_string());
        }
        self.redraw();
    }

    /// Renombra `path` a `new_name` si el nombre cambió y no está vacío.
    pub(super) fn commit_rename(&mut self, path: PathBuf, new_name: String) {
        let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !new_name.is_empty() && new_name != old_name {
            sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), Op::Rename { path, new_name });
        }
    }

    // ---------- barra de dirección editable ----------

    pub(super) fn begin_edit(&mut self, pane: usize) {
        if self.edit.is_some() {
            return;
        }
        self.set_active(pane);
        // En la configuración no se escribe una ruta: Ctrl+L va al buscador.
        if self.panes[pane].tab().is_settings() {
            self.begin_filter(pane);
            return;
        }
        let text = match &self.panes[pane].tab().location {
            Location::Computer => "Este equipo".to_string(),
            Location::Settings => return,
            Location::Path(p) => p.to_string_lossy().into_owned(),
            Location::Shell { parsing, .. } => parsing.clone(),
        };
        let r = self.panes[pane].address;
        let (x, w) = (r.left + 10.0, r.right - r.left - 20.0 - pane::ADDRESS_CLEAR_W);
        let y = (r.top + r.bottom) / 2.0 - 10.0;
        let Some(edit) = self.host.text_create(TextKind::Address, geom::rect(x, y, x + w, y + 20.0), &text, &self.theme) else { return };
        self.host.text_select(edit, 0, -1);
        self.host.text_focus(edit);
        self.edit = Some((edit, pane));
        // Como el Explorador: al abrir, el historial de rutas escritas.
        self.suggest = Some(typed::Suggest::new(r, ""));
        self.redraw();
    }

    /// Flecha de la barra de dirección: menú con las subcarpetas del tramo anterior (o las unidades,
    /// si es "Este equipo"); elegir una navega a ella.
    pub(super) fn show_subfolder_menu(&mut self, pane: usize, k: usize) {
        let Some((r, loc)) = self.panes[pane].chevron(k) else { return };
        let items = sys::file_system().subfolders(&loc);
        self.set_active(pane);
        self.panes[pane].open_chevron = Some(k);
        self.redraw();
        let mut entries: Vec<shell::Entry> = items
            .iter()
            .enumerate()
            .map(|(i, (name, _))| shell::Entry::Item { id: i as u32 + 1, label: name.clone(), enabled: true, checked: false })
            .collect();
        if items.is_empty() {
            entries.push(shell::Entry::Item { id: 0, label: "Sin subcarpetas".into(), enabled: false, checked: false });
        }
        let cmd = self.host.popup_menu(&entries, MenuPos::At(r.left, r.bottom + 2.0), None, None);
        self.panes[pane].open_chevron = None;
        if let Some((_, target)) = cmd.and_then(|c| (c as usize).checked_sub(1)).and_then(|i| items.get(i)) {
            let (hwnd, tx) = (self.hwnd, self.tx.clone());
            self.panes[pane].tab_mut().navigate(hwnd, &tx, target.clone());
        }
        self.redraw();
    }

    /// Cruz de la barra de dirección: borra lo escrito y sigue editando (con el historial a la vista).
    pub(super) fn clear_edit(&mut self) {
        let Some((edit, _)) = self.edit else { return };
        self.host.text_set(edit, "");
        self.host.text_focus(edit);
        if let Some(sg) = &mut self.suggest {
            sg.set_query("");
        }
        self.redraw();
    }

    /// Flechas y Re Pág / Av Pág sobre el desplegable: la barra muestra la ruta elegida.
    pub(super) fn on_suggest_key(&mut self, vk: Key) {
        let (Some((edit, _)), Some(sg)) = (self.edit, self.suggest.as_mut()) else { return };
        let delta = match vk {
            VK_UP => -1,
            VK_DOWN => 1,
            VK_PRIOR => -sg.page(),
            _ => sg.page(),
        };
        if let Some(text) = sg.move_by(delta) {
            // Cambiar el texto dispara EN_CHANGE: que no vuelva a buscar y pierda la elección.
            sg.quiet = true;
            self.host.text_set(edit, &text);
            self.host.text_select(edit, text.encode_utf16().count(), -1);
            sg.quiet = false;
            self.redraw();
        }
    }

    /// Clic en una ruta del desplegable: se navega a ella.
    pub(super) fn pick_suggestion(&mut self, index: usize) {
        let (Some((edit, _)), Some(path)) = (self.edit, self.suggest.as_ref().and_then(|s| s.items.get(index).cloned())) else { return };
        self.host.text_set(edit, &path);
        self.end_edit(true);
    }

    pub(super) fn end_edit(&mut self, commit: bool) {
        let Some((edit, pane)) = self.edit.take() else { return };
        self.suggest = None;
        let text = self.host.text_get(edit);
        self.host.text_destroy(edit);
        self.host.focus_main();
        if commit {
            self.go_to_text(pane, &text);
            let typed = text.trim().trim_matches('"');
            if std::path::Path::new(typed).exists() {
                typed::push(typed);
            }
        }
        self.redraw();
    }


    /// Pone el foco en la caja de búsqueda del panel (o se lo devuelve si ya está abierta).
    pub(super) fn begin_filter(&mut self, pane: usize) {
        self.end_recording();
        if let Some((edit, p)) = self.filter_edit {
            if p == pane {
                self.host.text_focus(edit);
                return;
            }
            self.end_filter(true);
        }
        self.end_edit(false);
        self.set_active(pane);
        let r = self.panes[pane].search_text_rect();
        let y = (r.top + r.bottom) / 2.0 - 10.0;
        let text = self.panes[pane].tab().search_query().to_string();
        let Some(edit) = self.host.text_create(TextKind::Filter, geom::rect(r.left, y, r.right, y + 20.0), &text, &self.theme) else { return };
        self.host.text_select(edit, 0, -1);
        self.host.text_focus(edit);
        self.filter_edit = Some((edit, pane));
        self.redraw();
    }

    /// Cierra la caja de búsqueda. `keep`: Enter o perder el foco dejan la búsqueda (y la lanzan
    /// ya si estaba esperando que se deje de escribir); Escape la termina y vuelve a la carpeta.
    pub(super) fn end_filter(&mut self, keep: bool) {
        let Some((edit, pane)) = self.filter_edit.take() else { return };
        let text = self.host.text_get(edit);
        self.host.text_destroy(edit);
        self.host.focus_main_if_lost();
        self.pending_search = None;
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab_mut();
        if !keep {
            tab.end_search(hwnd, &tx);
        } else if text.trim() != tab.search_query() {
            tab.search(hwnd, &tx, text.trim());
        }
        self.redraw();
    }

    /// Termina la búsqueda del panel (cruz de la caja o Escape sobre la lista).
    pub(super) fn clear_filter(&mut self, pane: usize) {
        if self.filter_edit.is_some_and(|(_, p)| p == pane) {
            self.end_filter(false);
        } else {
            let (hwnd, tx) = (self.hwnd, self.tx.clone());
            self.panes[pane].tab_mut().end_search(hwnd, &tx);
            self.redraw();
        }
    }

    /// Cambió lo escrito en la caja de búsqueda: se busca cuando se deja de escribir un momento
    /// (para no recorrer el disco con cada letra). Vacía, vuelve a la carpeta enseguida.
    pub(super) fn search_typed(&mut self, pane: usize, text: &str) {
        let text = text.trim().to_string();
        // En la configuración se busca en memoria: al instante (y escribir ahí deja de grabar).
        if self.panes[pane].tab().is_settings() {
            self.end_recording();
            let (hwnd, tx) = (self.hwnd, self.tx.clone());
            self.panes[pane].tab_mut().search(hwnd, &tx, &text);
            self.redraw();
            return;
        }
        if text.is_empty() {
            self.pending_search = None;
            let (hwnd, tx) = (self.hwnd, self.tx.clone());
            self.panes[pane].tab_mut().end_search(hwnd, &tx);
            self.redraw();
            return;
        }
        self.search_token += 1;
        let token = self.search_token;
        self.pending_search = Some((pane, text, token));
        let hwnd = self.hwnd;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(SEARCH_DELAY_MS));
            hwnd.post(WM_APP_SEARCH, 0, token as isize);
        });
    }

    /// Pasó la pausa después de escribir: si no se siguió escribiendo, se busca.
    pub(super) fn on_search_due(&mut self, token: u64) {
        let Some((pane, text, t)) = self.pending_search.take() else { return };
        if t != token {
            self.pending_search = Some((pane, text, t));
            return;
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        if self.panes[pane].tab().search_query() != text {
            self.panes[pane].tab_mut().search(hwnd, &tx, &text);
        }
        self.redraw();
    }

    /// Navega a lo que se escribió en la barra de dirección.
    pub(super) fn go_to_text(&mut self, pane: usize, text: &str) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab_mut();
        let text = text.trim().trim_matches('"');
        if text.is_empty() || text.eq_ignore_ascii_case("este equipo") {
            tab.navigate(hwnd, &tx, Location::Computer);
            return;
        }
        // "shell:RecycleBinFolder", "::{GUID}": lugares del Shell.
        if text.starts_with("shell:") || text.starts_with("::") {
            match sys::file_system().shell_location(text) {
                Some(loc) => tab.navigate(hwnd, &tx, loc),
                None => self.host.beep(),
            }
            return;
        }
        let mut path = PathBuf::from(self.host.expand_env(text));
        if path.is_relative() {
            if let Location::Path(base) = &tab.location {
                path = base.join(path);
            }
        }
        // "C:" solo es "la carpeta actual de C:"; se interpreta como la raíz.
        if path.as_os_str().len() == 2 && path.to_string_lossy().ends_with(':') {
            path.push("\\");
        }
        // "..", "." y "a\..\b": se resuelven acá (Windows los acepta, pero la pestaña mostraría la ruta sucia).
        path = fs::normalize_lexical(&path);
        // Un recurso de red que pide usuario y contraseña (o que no se verifica): se muestra el
        // diálogo de Windows. Si igual no se puede ver, se navega y la barra de estado dice por qué.
        if fs::is_unc(&path) && !path.exists() {
            sys::file_system().connect_share(hwnd, &path);
            if !path.exists() {
                tab.navigate(hwnd, &tx, Location::Path(path));
                return;
            }
        }
        if path.is_dir() {
            // Con las mayúsculas del disco, no como se escribió (se ve en pestañas e historial).
            tab.navigate(hwnd, &tx, Location::Path(sys::file_system().true_case(&path)));
        } else if fs::is_zip(&path) && path.is_file() {
            // Un .zip escrito en la barra: se entra como carpeta.
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let parent = path.parent().map(|p| Box::new(Location::Path(p.to_path_buf())));
            tab.navigate(hwnd, &tx, Location::Shell { parsing: path.to_string_lossy().into_owned(), name, parent });
        } else if path.is_file() {
            // Un archivo: se abre su carpeta con el archivo enfocado.
            if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
                tab.navigate(hwnd, &tx, Location::Path(parent.to_path_buf()));
                tab.focus_after_load(name.to_os_string());
            }
        } else {
            self.host.beep();
        }
    }
}

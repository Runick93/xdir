//! Grupos de paneles guardados, config y unidades.

use super::*;

impl App {
    /// Abre un grupo guardado: su layout, divisores y las pestañas de cada panel. Las pestañas
    /// que había quedan entre las cerradas (Ctrl+Shift+T las recupera).
    pub(super) fn apply_group(&mut self, index: usize) {
        let Some(g) = self.groups.items.get(index).cloned() else { return };
        // Los paneles cambian: la vista de xtree no debe taparlos.
        self.close_xtree();
        // Lo que se cambió en el grupo que se deja queda guardado en él.
        self.autosave_group();
        self.end_edit(false);
        self.end_filter(true);
        self.begin_layout_anim();
        self.maximized = None;
        self.layout = Layout::from_index(g.layout).unwrap_or(Layout::Four);
        self.splits = Splits { x: g.split_x, y: g.split_y };
        for (i, state) in g.panes.iter().enumerate().take(PANES) {
            if state.tabs.is_empty() {
                continue;
            }
            let tabs: Vec<Tab> = state.tabs.iter().cloned().map(|l| self.make_tab(l)).collect();
            self.panes[i].replace_tabs(tabs, state.active);
        }
        self.active = 0;
        self.notice = Some(format!("Grupo «{}»", g.name));
        self.group_name = Some(g.name);
        self.sync_visibility();
        self.redraw();
    }

    /// Pestaña de grupo nueva, ya guardada como «Grupo N» y vacía (cada panel en Este equipo);
    /// se le cambia el nombre con doble clic.
    pub(super) fn new_group(&mut self) {
        let mut n = self.groups.items.len() + 1;
        while self.groups.find(&format!("Grupo {n}")).is_some() {
            n += 1;
        }
        let name = format!("Grupo {n}");
        let panes = (0..PANES).map(|_| PaneState { tabs: vec![Location::Computer], active: 0 }).collect();
        self.groups.put(Group { name, layout: self.layout.index(), split_x: self.splits.x, split_y: self.splits.y, panes, temp: true });
        let _ = self.groups.save();
        self.apply_group(self.groups.items.len() - 1);
    }

    /// Con la barra de grupos activa y ningún grupo, crea una pestaña temporal con los paneles de
    /// ahora, hasta que se decida si guardarla (o se descarte al cerrarla).
    pub(super) fn ensure_group(&mut self) {
        if !self.show_group_bar || !self.groups.items.is_empty() {
            return;
        }
        let mut group = self.group_snapshot("Grupo 1");
        group.temp = true;
        self.groups.put(group);
        self.group_name = Some("Grupo 1".to_string());
        let _ = self.groups.save();
    }

    pub(super) fn rename_group(&mut self, old: &str, name: &str) {
        let Some(i) = self.groups.find(old) else { return };
        if self.groups.find(name).is_some_and(|j| j != i) {
            self.notice = Some(format!("Ya hay un grupo «{name}»"));
        } else {
            self.groups.items[i].name = name.to_string();
            if self.group_name.as_deref() == Some(old) {
                self.group_name = Some(name.to_string());
            }
            let _ = self.groups.save();
        }
        self.redraw();
    }

    /// × de una pestaña sin guardar: pregunta si se conserva como grupo o se descarta.
    pub(super) fn close_group_tab(&mut self, index: usize) {
        let Some(name) = self.groups.items.get(index).map(|g| g.name.clone()) else { return };
        let text = format!("¿Guardar «{name}» como grupo?\n\nSí: queda guardado. No: se descarta.");
        let answer = self.host.ask_yes_no_cancel("Cerrar pestaña de grupo", &text);
        if answer == Some(true) {
            self.groups.items[index].temp = false;
            let _ = self.groups.save();
        } else if answer == Some(false) {
            let was_active = self.group_name.as_deref() == Some(name.as_str());
            self.groups.remove(index);
            let _ = self.groups.save();
            if was_active {
                self.group_name = None;
                if !self.groups.items.is_empty() {
                    self.apply_group(index.saturating_sub(1).min(self.groups.items.len() - 1));
                }
            }
        }
        self.ensure_group();
        self.redraw();
    }

    pub(super) fn switch_group(&mut self, index: usize) {
        if self.group_name.as_deref().and_then(|n| self.groups.find(n)) != Some(index) {
            self.apply_group(index);
        }
    }

    /// Cambió el conjunto de unidades: se actualiza "Este equipo" en los paneles y el árbol, y los
    /// paneles que estaban en una unidad que ya no existe vuelven a "Este equipo".
    pub fn drives_changed(&mut self) {
        sys::file_system().forget_drive_labels();
        #[cfg(feature = "arbol")]
        self.tree.refresh_drives();
        #[cfg(feature = "arbol")]
        self.request_remote_drive_labels();
        self.request_ejectable();
        self.request_connections();
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            match &tab.location {
                Location::Computer => tab.reload(hwnd, &tx, true),
                Location::Path(p) if !sys::network().is_remote(p) && sys::file_system().drive_missing(p) => tab.navigate(hwnd, &tx, Location::Computer),
                _ => {}
            }
        }
        self.redraw();
    }

    /// Alto de la barra de pestañas de grupos: solo aparece cuando hay grupos guardados.
    pub(super) fn groups_bar_h(&self) -> f32 {
        if self.groups.items.is_empty() || !self.show_group_bar { 0.0 } else { ui::GROUPBAR_H }
    }

    /// Los paneles y carpetas de ahora, con el nombre de un grupo.
    pub(super) fn group_snapshot(&self, name: &str) -> Group {
        // Guardar con un nombre (o reemplazar un grupo) lo vuelve permanente.
        Group {
            temp: false,
            name: name.to_string(),
            layout: self.layout.index(),
            split_x: self.splits.x,
            split_y: self.splits.y,
            panes: self.panes.iter().map(|p| PaneState { tabs: p.tabs.iter().map(|t| t.location.clone()).collect(), active: p.active }).collect(),
        }
    }

    /// Mientras hay un grupo activo, cada cambio de paneles, pestañas o carpetas se guarda en él.
    pub(super) fn autosave_group(&mut self) {
        let Some(name) = self.group_name.clone() else { return };
        let Some(i) = self.groups.find(&name) else { return };
        let snap = Group { temp: self.groups.items[i].temp, ..self.group_snapshot(&self.groups.items[i].name) };
        if snap.same_as(&self.groups.items[i]) {
            return;
        }
        self.groups.items[i] = snap;
        if let Err(e) = self.groups.save() {
            self.notice = Some(format!("No se pudo guardar el grupo: {e}"));
        }
    }

    /// Guarda los paneles y carpetas actuales como grupo (reemplaza uno con el mismo nombre).
    pub(super) fn save_group(&mut self, name: &str) {
        if let Some(old) = self.group_rename.take() {
            self.rename_group(&old, name);
            return;
        }
        let group = self.group_snapshot(name);
        self.groups.put(group);
        self.group_name = Some(name.to_string());
        self.notice = Some(match self.groups.save() {
            Ok(()) => format!("Grupo «{name}» guardado"),
            Err(e) => format!("No se pudo guardar el grupo: {e}"),
        });
        self.redraw();
    }

    /// Menú del botón de grupos: abrir uno, guardar el actual, eliminar o editar el archivo.
    pub(super) fn show_groups_menu(&mut self) {
        const SAVE: usize = 1;
        const EDIT: usize = 2;
        const OPEN: usize = 1000;
        const DELETE: usize = 2000;
        let (w, h) = self.client_dips();
        let r = ui::groups_rect(w, h);
        let item = |id: usize, text: &str| shell::Entry::Item { id: id as u32, label: text.to_string(), enabled: true, checked: false };
        let mut entries: Vec<shell::Entry> = self.groups.items.iter().enumerate().map(|(i, g)| item(OPEN + i, &g.name)).collect();
        if self.groups.items.is_empty() {
            entries.push(shell::Entry::Item { id: 0, label: "Todavía no hay grupos".into(), enabled: false, checked: false });
        }
        entries.push(shell::Entry::Sep);
        entries.push(item(SAVE, "Guardar paneles y carpetas como grupo…"));
        if !self.groups.items.is_empty() {
            let delete = self.groups.items.iter().enumerate().map(|(i, g)| item(DELETE + i, &g.name)).collect();
            entries.push(shell::Entry::Sub("Eliminar".into(), delete));
            entries.push(item(EDIT, "Editar grupos.toml"));
        }
        // El grupo activo lleva un punto del color de acento.
        let dot = self.group_name.as_deref().and_then(|n| self.groups.find(n)).map(|i| ((OPEN + i) as u32, self.theme.accent));
        // Hacia arriba desde el botón: la barra de estado está abajo de todo.
        let cmd = self.host.popup_menu(&entries, MenuPos::Above(r.left, r.top), dot, None).map_or(0, |c| c as usize);
        self.groups_menu_closed = (cmd == 0).then(Instant::now);
        match cmd {
            SAVE => self.open_palette(PaletteMode::GroupName),
            EDIT => {
                if let Some(dir) = session::config_dir() {
                    self.host.open_in_editor(&dir.join("grupos.toml"));
                }
            }
            c if c >= DELETE => {
                let i = c - DELETE;
                if let Some(g) = self.groups.items.get(i) {
                    self.notice = Some(format!("Grupo «{}» eliminado", g.name));
                    if self.group_name.as_deref() == Some(g.name.as_str()) {
                        self.group_name = None;
                    }
                    self.groups.remove(i);
                    let _ = self.groups.save();
                }
            }
            c if c >= OPEN => self.apply_group(c - OPEN),
            _ => {}
        }
        self.redraw();
    }

    /// Vuelve a leer `config.toml` y `grupos.toml` si cambiaron (se editan con la app abierta).
    pub(super) fn reload_config(&mut self) {
        if self.config.changed_on_disk() {
            self.config = Config::load();
            self.apply_animation_setting();
            self.notice = Some(match self.config.errors.first() {
                Some(e) => format!("config.toml, {e}"),
                None => "Configuración aplicada".to_string(),
            });
            self.redraw();
        }
        let groups = Groups::load();
        if groups != self.groups {
            self.groups = groups;
        }
    }
}

//! Favoritos: barra, menús, edición y arrastre.

use super::*;

impl App {
    // ---------- favoritos ----------

    pub(super) fn save_favorites(&mut self) {
        if let Err(e) = self.favorites.save() {
            self.notice = Some(format!("No se pudieron guardar los favoritos: {e}"));
        }
        self.redraw();
    }

    /// Estrella / Ctrl+D: agrega la carpeta del panel a favoritos, o la quita si ya está.
    pub(super) fn toggle_favorite(&mut self, pane: usize) {
        let location = self.panes[pane].tab().location.clone();
        let title = location.title();
        match self.favorites.find(&location) {
            Some(path) => {
                self.favorites.remove(&path);
                self.notice = Some(format!("Quitado de favoritos: {title}"));
            }
            None => {
                self.favorites.items.push(Fav::Link { name: title.clone(), location });
                self.notice = Some(format!("Agregado a favoritos: {title}"));
            }
        }
        self.save_favorites();
    }

    /// Abre un favorito: navega el panel (o abre una pestaña nueva). Una carpeta de favoritos
    /// despliega su menú; con `new_tab`, abre todo su contenido en pestañas nuevas.
    pub(super) fn activate_fav(&mut self, pane: usize, path: &[usize], new_tab: bool) {
        match self.favorites.get(path).cloned() {
            Some(Fav::Link { location, .. }) if new_tab => self.new_tab(pane, location, false),
            Some(Fav::Link { location, .. }) => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                self.panes[pane].tab_mut().navigate(hwnd, &tx, location);
            }
            Some(Fav::Group { items, .. }) if new_tab => {
                for f in items {
                    if let Fav::Link { location, .. } = f {
                        self.new_tab(pane, location, false);
                    }
                }
            }
            Some(Fav::Group { .. }) => {
                let anchor = path.first().and_then(|&i| self.panes[pane].fav_rect(i)).unwrap_or(self.panes[pane].favbar);
                self.show_fav_menu(pane, FavMenuSource::Group(path.to_vec()), anchor);
            }
            None => {}
        }
        self.redraw();
    }

    /// Favoritos de un menú desplegable: el contenido de una carpeta de favoritos, o los de la
    /// raíz que no entraron en la barra. None si la carpeta ya no existe.
    pub(super) fn fav_menu_entries(&self, source: &FavMenuSource) -> Option<Vec<(FavPath, Fav)>> {
        Some(match source {
            FavMenuSource::Group(path) => match self.favorites.get(path) {
                Some(Fav::Group { items, .. }) => items
                    .iter()
                    .enumerate()
                    .map(|(j, f)| ([path.as_slice(), &[j]].concat(), f.clone()))
                    .collect(),
                _ => return None,
            },
            FavMenuSource::Overflow(first) => {
                self.favorites.items.iter().enumerate().skip(*first).map(|(i, f)| (vec![i], f.clone())).collect()
            }
        })
    }

    /// Clic derecho en PowerShell o CMD del menú de terminales: abrirla como administrador o con
    /// las credenciales de otro usuario.
    pub(super) fn console_context_menu(&mut self, pane: usize, console: Console) {
        // Las variantes que ofrece el sistema (en Windows: administrador y otro usuario).
        let verbs = Console::alt_verbs();
        if verbs.is_empty() {
            return;
        }
        let entries: Vec<shell::Entry> = verbs
            .iter()
            .enumerate()
            .map(|(i, (_, label))| shell::Entry::Item { id: i as u32 + 1, label: label.to_string(), enabled: true, checked: false })
            .collect();
        let Some(cmd) = popup_menu(self, &entries, MenuPos::Cursor, None) else { return };
        let Some(&(verb, _)) = verbs.get(cmd as usize - 1) else { return };
        self.term_menu = None;
        if let Location::Path(dir) = self.panes[pane].tab().location.clone() {
            if !console.open_as(self.hwnd, &dir, verb) {
                self.notice = Some(format!("No se pudo abrir {}", console.label()));
            }
        }
        self.redraw();
    }

    /// Menú contextual de la barra de favoritos (o de un favorito). Devuelve la acción elegida.
    /// `recurse`: se abre desde un menú de favoritos todavía abierto.
    pub(super) fn fav_context_menu(&self, path: Option<&FavPath>, recurse: bool) -> Option<FavAct> {
        let fav = path.and_then(|p| self.favorites.get(p));
        let mut entries = Vec::new();
        let item = |id: FavAct, text: &str| shell::Entry::Item { id: id as u32, label: text.to_string(), enabled: true, checked: false };
        match fav {
            Some(Fav::Link { .. }) => {
                entries.push(item(FavAct::Open, "Abrir"));
                entries.push(item(FavAct::OpenNewTab, "Abrir en pestaña nueva"));
                entries.push(shell::Entry::Sep);
            }
            Some(Fav::Group { .. }) => {
                entries.push(item(FavAct::OpenNewTab, "Abrir todo en pestañas nuevas"));
                entries.push(shell::Entry::Sep);
            }
            None => {}
        }
        if fav.is_some() {
            entries.push(item(FavAct::Rename, "Renombrar"));
            entries.push(item(FavAct::Delete, "Eliminar"));
            if path.is_some_and(|p| p.len() > 1) {
                entries.push(item(FavAct::MoveOut, "Sacar de la carpeta"));
            }
            entries.push(shell::Entry::Sep);
        }
        if !recurse {
            entries.push(item(FavAct::NewGroup, "Nueva carpeta de favoritos"));
            entries.push(item(FavAct::AddCurrent, "Agregar la carpeta actual"));
            entries.push(shell::Entry::Sep);
            entries.push(item(FavAct::EditFile, "Editar el archivo de favoritos"));
            entries.push(item(FavAct::HideBar, &format!("Ocultar la barra de favoritos ({})", crate::commands::shortcut_label("Ctrl+Shift+B"))));
        }
        let pos = if recurse { MenuPos::CursorNested } else { MenuPos::Cursor };
        popup_menu(self, &entries, pos, None).and_then(|id| FavAct::from_id(id as i32))
    }

    pub(super) fn fav_action(&mut self, pane: usize, path: Option<FavPath>, act: FavAct) {
        match (act, path) {
            (FavAct::Open, Some(p)) => self.activate_fav(pane, &p, false),
            (FavAct::OpenNewTab, Some(p)) => self.activate_fav(pane, &p, true),
            (FavAct::Rename, Some(p)) => self.begin_fav_edit(pane, p),
            (FavAct::Delete, Some(p)) => {
                self.favorites.remove(&p);
                self.save_favorites();
            }
            (FavAct::MoveOut, Some(p)) => {
                if let Some(f) = self.favorites.remove(&p) {
                    self.favorites.items.push(f);
                    self.save_favorites();
                }
            }
            (FavAct::NewGroup, p) => {
                // Al lado del favorito sobre el que se hizo clic, o al final.
                let at = p.and_then(|p| p.first().copied()).map(|i| i + 1).unwrap_or(self.favorites.items.len());
                let group = Fav::Group { name: "Nueva carpeta".into(), items: Vec::new() };
                self.favorites.insert_root(at, group);
                self.save_favorites();
                self.begin_fav_edit(pane, vec![at.min(self.favorites.items.len() - 1)]);
            }
            (FavAct::AddCurrent, _) => {
                if self.favorites.find(&self.panes[pane].tab().location).is_none() {
                    self.toggle_favorite(pane);
                }
            }
            (FavAct::EditFile, _) => {
                // Se guarda antes para que exista; al volver a la ventana se relee.
                self.save_favorites();
                if let Some(file) = Favorites::file_path() {
                    shell::open_file(self.hwnd, &file, None);
                }
            }
            (FavAct::HideBar, _) => {
                self.show_favorites = false;
                self.redraw();
            }
            _ => {}
        }
    }

    /// Renombrado de un favorito en el lugar (o sobre la barra si está dentro de una carpeta).
    pub(super) fn begin_fav_edit(&mut self, pane: usize, path: FavPath) {
        if self.fav_edit.is_some() || self.edit.is_some() || self.rename.is_some() {
            return;
        }
        let Some(name) = self.favorites.get(&path).map(|f| f.name().to_string()) else { return };
        // Dibujar ya, para ubicar el favorito donde quedó.
        self.paint();
        let pane_ref = &self.panes[pane];
        let bar = pane_ref.favbar;
        let r = match path.as_slice() {
            [i] => pane_ref.fav_rect(*i).unwrap_or(bar),
            _ => bar,
        };
        let r = render::rect(r.left, r.top, r.right.max(r.left + 200.0).min(bar.right), r.bottom);
        let Some(edit) = TextBox::create(self, TextKind::FavRename, r, &name) else { return };
        edit.select(0, -1);
        edit.focus();
        self.fav_edit = Some((edit, path));
    }

    pub(super) fn end_fav_edit(&mut self, commit: bool) {
        let Some((edit, path)) = self.fav_edit.take() else { return };
        let text = edit.text();
        edit.destroy();
        focus_main(self.hwnd);
        let name = text.trim().to_string();
        if commit && !name.is_empty() {
            if let Some(f) = self.favorites.get_mut(&path) {
                f.set_name(name);
            }
            self.save_favorites();
        }
        self.redraw();
    }

    /// Arrastre de carpetas (desde la lista o el Explorador) sobre la barra de favoritos.
    /// Devuelve si el punto está sobre la barra (y el evento quedó resuelto).
    pub(super) fn on_drag_favbar(&mut self, ev: &mut DragEvent, x: f32, y: f32) -> bool {
        let over = match (self.show_favorites, ev.kind, self.hit(x, y)) {
            (true, DragKind::Enter | DragKind::Over | DragKind::Drop, Hit::Pane(i, PaneHit::Fav(_) | PaneHit::FavBar | PaneHit::FavOverflow)) => Some(i),
            _ => None,
        };
        let Some(pane) = over else {
            if self.fav_drop.take().is_some() {
                self.redraw();
            }
            return false;
        };
        let dirs: Vec<PathBuf> = ev.paths.map(|p| p.iter().filter(|p| p.is_dir()).cloned().collect()).unwrap_or_default();
        // Un favorito es un vínculo: no se copia ni se mueve nada.
        ev.effect = if dirs.is_empty() {
            Effect::NONE
        } else if ev.allowed.allows(Effect::LINK) {
            Effect::LINK
        } else if ev.allowed.allows(Effect::COPY) {
            Effect::COPY
        } else {
            Effect::NONE
        };
        let target = self.panes[pane].fav_drop_at(&self.favorites, x);
        for p in &mut self.panes {
            p.tab_mut().list.drop_target = None;
        }
        if ev.kind == DragKind::Drop {
            self.fav_drop = None;
            if ev.effect != Effect::NONE {
                let new: Vec<Fav> = dirs
                    .into_iter()
                    .map(|d| Fav::Link { name: Location::Path(d.clone()).title(), location: Location::Path(d) })
                    .collect();
                match target {
                    FavDrop::Into(g) => {
                        for f in new {
                            self.favorites.push_into(&[g], f);
                        }
                    }
                    FavDrop::Before(at) => {
                        for (n, f) in new.into_iter().enumerate() {
                            self.favorites.insert_root(at + n, f);
                        }
                    }
                }
                self.save_favorites();
            }
        } else {
            let want = (ev.effect != Effect::NONE).then_some((pane, target));
            if want != self.fav_drop {
                self.fav_drop = want;
            }
        }
        self.redraw();
        true
    }
}

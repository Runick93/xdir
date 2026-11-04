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

    /// Menú desplegable de una carpeta de favoritos o del desborde "»".
    pub(super) fn show_fav_menu(&mut self, pane: usize, source: FavMenuSource, anchor: render::Rect) {
        let entries: Vec<(FavPath, Fav)> = match &source {
            FavMenuSource::Group(path) => match self.favorites.get(path) {
                Some(Fav::Group { items, .. }) => items
                    .iter()
                    .enumerate()
                    .map(|(j, f)| ([path.as_slice(), &[j]].concat(), f.clone()))
                    .collect(),
                _ => return,
            },
            FavMenuSource::Overflow(first) => {
                self.favorites.items.iter().enumerate().skip(*first).map(|(i, f)| (vec![i], f.clone())).collect()
            }
        };
        let mut menu = FavMenu { ids: Vec::new(), positions: HashMap::new(), action: None };
        let k = self.dpi as f32 / 96.0;
        let mut pt = POINT { x: (anchor.left * k) as i32, y: (anchor.bottom * k) as i32 };
        unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            build_fav_menu(&mut menu, hmenu, &entries);
            let _ = ClientToScreen(self.hwnd, &mut pt);
            self.fav_menu = Some(menu);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN).0, pt.x, pt.y, self.hwnd, None).0;
            let menu = self.fav_menu.take();
            let _ = DestroyMenu(hmenu);
            let Some(menu) = menu else { return };
            // Si dentro del menú se eligió algo con clic derecho, eso manda.
            if let Some((path, act)) = menu.action {
                self.fav_action(pane, Some(path), act);
            } else if cmd > 0 {
                if let Some(path) = menu.ids.get(cmd as usize - 1).cloned() {
                    self.activate_fav(pane, &path, key_down(VK_CONTROL));
                }
            }
        }
    }

    /// Clic derecho sobre un elemento de un menú de favoritos (`WM_MENURBUTTONUP`).
    pub(super) fn on_fav_menu_right_click(&mut self, hmenu: isize, pos: u32) {
        let Some(path) = self.fav_menu.as_ref().and_then(|m| m.positions.get(&(hmenu, pos)).cloned()) else { return };
        if let Some(act) = self.fav_context_menu(Some(&path), true) {
            if let Some(m) = self.fav_menu.as_mut() {
                m.action = Some((path, act));
            }
            unsafe {
                let _ = EndMenu();
            }
        }
    }

    /// Clic derecho en PowerShell o CMD del menú de terminales: abrirla como administrador o con
    /// las credenciales de otro usuario.
    pub(super) fn console_context_menu(&mut self, pane: usize, console: Console) {
        const ADMIN: usize = 1;
        const OTHER: usize = 2;
        let cmd = unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            let _ = AppendMenuW(hmenu, MF_STRING, ADMIN, w!("Ejecutar como administrador"));
            let _ = AppendMenuW(hmenu, MF_STRING, OTHER, w!("Ejecutar como otro usuario"));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, self.hwnd, None).0;
            let _ = DestroyMenu(hmenu);
            cmd as usize
        };
        let verb = match cmd {
            ADMIN => "runas",
            OTHER => "runasuser",
            _ => return,
        };
        self.term_menu = None;
        if let Location::Path(dir) = self.panes[pane].tab().location.clone() {
            if !console.open_as(self.hwnd, &dir, verb) {
                self.notice = Some(format!("No se pudo abrir {}", console.label()));
            }
        }
        self.redraw();
    }

    /// Menú contextual de la barra de favoritos (o de un favorito). Devuelve la acción elegida.
    pub(super) fn fav_context_menu(&self, path: Option<&FavPath>, recurse: bool) -> Option<FavAct> {
        let fav = path.and_then(|p| self.favorites.get(p));
        unsafe {
            let hmenu = CreatePopupMenu().ok()?;
            let add = |id: FavAct, text: &str| {
                let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(hmenu, MF_STRING, id as usize, PCWSTR(wide.as_ptr()));
            };
            let sep = || {
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            };
            match fav {
                Some(Fav::Link { .. }) => {
                    add(FavAct::Open, "Abrir");
                    add(FavAct::OpenNewTab, "Abrir en pestaña nueva");
                    sep();
                }
                Some(Fav::Group { .. }) => {
                    add(FavAct::OpenNewTab, "Abrir todo en pestañas nuevas");
                    sep();
                }
                None => {}
            }
            if fav.is_some() {
                add(FavAct::Rename, "Renombrar");
                add(FavAct::Delete, "Eliminar");
                if path.is_some_and(|p| p.len() > 1) {
                    add(FavAct::MoveOut, "Sacar de la carpeta");
                }
                sep();
            }
            if !recurse {
                add(FavAct::NewGroup, "Nueva carpeta de favoritos");
                add(FavAct::AddCurrent, "Agregar la carpeta actual");
                sep();
                add(FavAct::EditFile, "Editar el archivo de favoritos");
                add(FavAct::HideBar, "Ocultar la barra de favoritos (Ctrl+Shift+B)");
            }
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let mut flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
            if recurse {
                flags |= TPM_RECURSE;
            }
            let cmd = TrackPopupMenuEx(hmenu, flags.0, pt.x, pt.y, self.hwnd, None).0;
            let _ = DestroyMenu(hmenu);
            FavAct::from_id(cmd)
        }
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
                    let wide = fs::wide_path(&file);
                    unsafe {
                        ShellExecuteW(Some(self.hwnd), w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL);
                    }
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
        let k = self.dpi as f32 / 96.0;
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                (r.left * k) as i32,
                (r.top * k) as i32,
                ((r.right - r.left) * k) as i32,
                ((r.bottom - r.top) * k) as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            // Opaco: sin capa, el GDI del control deja ver el Mica de la ventana (fondo gris).
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_FAVEDIT_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetFocus(Some(edit));
            self.fav_edit = Some((edit, path));
        }
    }

    pub(super) fn end_fav_edit(&mut self, commit: bool) {
        let Some((edit, path)) = self.fav_edit.take() else { return };
        let mut buf = vec![0u16; 512];
        let text = unsafe {
            let len = GetWindowTextW(edit, &mut buf);
            let _ = DestroyWindow(edit);
            let _ = SetFocus(Some(self.hwnd));
            String::from_utf16_lossy(&buf[..len.max(0) as usize])
        };
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
            DROPEFFECT_NONE
        } else if ev.allowed.0 & DROPEFFECT_LINK.0 != 0 {
            DROPEFFECT_LINK
        } else if ev.allowed.0 & DROPEFFECT_COPY.0 != 0 {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        };
        let target = self.panes[pane].fav_drop_at(&self.favorites, x);
        for p in &mut self.panes {
            p.tab_mut().list.drop_target = None;
        }
        if ev.kind == DragKind::Drop {
            self.fav_drop = None;
            if ev.effect != DROPEFFECT_NONE {
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
            let want = (ev.effect != DROPEFFECT_NONE).then_some((pane, target));
            if want != self.fav_drop {
                self.fav_drop = want;
            }
        }
        self.redraw();
        true
    }
}

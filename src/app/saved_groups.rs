//! Grupos de paneles guardados, config y unidades.

use super::*;

impl App {
    /// Abre un grupo guardado: su layout, divisores y las pestañas de cada panel. Las pestañas
    /// que había quedan entre las cerradas (Ctrl+Shift+T las recupera).
    pub(super) fn apply_group(&mut self, index: usize) {
        let Some(g) = self.groups.items.get(index).cloned() else { return };
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
        let text: Vec<u16> = format!("¿Guardar «{name}» como grupo?\n\nSí: queda guardado. No: se descarta.\0").encode_utf16().collect();
        let title: Vec<u16> = "Cerrar pestaña de grupo\0".encode_utf16().collect();
        let answer = unsafe { MessageBoxW(Some(self.hwnd.hwnd()), PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_YESNOCANCEL | MB_ICONQUESTION) };
        if answer == IDYES {
            self.groups.items[index].temp = false;
            let _ = self.groups.save();
        } else if answer == IDNO {
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

    /// Abre xtree (junto a xdir.exe) en la carpeta del panel activo, o en la unidad del sistema.
    pub(super) fn open_xtree(&mut self) {
        let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("xtree.exe"))) else { return };
        let mut cmd = std::process::Command::new(exe);
        if let Location::Path(p) = &self.panes[self.active].tab().location {
            cmd.arg(p);
        }
        if cmd.spawn().is_err() {
            self.notice = Some("No se pudo abrir xtree.exe (tiene que estar junto a xdir.exe)".into());
        }
    }

    /// Cambió el conjunto de unidades: se actualiza "Este equipo" en los paneles y el árbol, y los
    /// paneles que estaban en una unidad que ya no existe vuelven a "Este equipo".
    pub(super) fn drives_changed(&mut self) {
        fs::forget_drive_labels();
        #[cfg(feature = "arbol")]
        self.tree.refresh_drives();
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            match &tab.location {
                Location::Computer => tab.reload(hwnd, &tx, true),
                Location::Path(p) if fs::drive_missing(p) => tab.navigate(hwnd, &tx, Location::Computer),
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

    /// Punto del color de acento para marcar un ítem de menú (bitmap de 32 bits con alfa).
    pub(super) fn accent_dot(&self) -> Option<windows::Win32::Graphics::Gdi::HBITMAP> {
        use windows::Win32::Graphics::Gdi::{BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateDIBSection, DIB_RGB_COLORS};
        let size = (16.0 * self.dpi as f32 / 96.0).round().max(8.0) as i32;
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
        if bits.is_null() {
            return None;
        }
        let px = unsafe { std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize) };
        let c = self.theme.accent;
        let (c_r, c_g, c_b) = ((c.r * 255.0) as u32, (c.g * 255.0) as u32, (c.b * 255.0) as u32);
        let (center, radius) = (size as f32 / 2.0, size as f32 * 0.25);
        for y in 0..size {
            for x in 0..size {
                let (dx, dy) = (x as f32 + 0.5 - center, y as f32 + 0.5 - center);
                // Borde suave de un píxel; el color va premultiplicado por el alfa.
                let cover = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                let a = (cover * 255.0) as u32;
                px[(y * size + x) as usize] = (a << 24) | ((c_r * a / 255) << 16) | ((c_g * a / 255) << 8) | (c_b * a / 255);
            }
        }
        Some(bitmap)
    }

    /// Menú del botón de grupos: abrir uno, guardar el actual, eliminar o editar el archivo.
    pub(super) fn show_groups_menu(&mut self) {
        const SAVE: usize = 1;
        const EDIT: usize = 2;
        const OPEN: usize = 1000;
        const DELETE: usize = 2000;
        let (w, h) = self.client_dips();
        let r = ui::groups_rect(w, h);
        let k = self.dpi as f32 / 96.0;
        let mut pt = POINT { x: (r.left * k) as i32, y: (r.top * k) as i32 };
        let cmd = unsafe {
            let Ok(hmenu) = CreatePopupMenu() else { return };
            let add = |menu: HMENU, id: usize, text: &str| {
                let wide: Vec<u16> = text.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
            };
            for (i, g) in self.groups.items.iter().enumerate() {
                add(hmenu, OPEN + i, &g.name);
            }
            if self.groups.items.is_empty() {
                let wide: Vec<u16> = "Todavía no hay grupos".encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(hmenu, MF_STRING | MF_GRAYED, 0, PCWSTR(wide.as_ptr()));
            }
            // El grupo activo lleva un punto del color de acento.
            let dot = self.group_name.as_deref().and_then(|n| self.groups.find(n)).and_then(|i| Some((i, self.accent_dot()?)));
            if let Some((i, bitmap)) = dot {
                let mii = MENUITEMINFOW { cbSize: size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, hbmpItem: bitmap, ..Default::default() };
                let _ = SetMenuItemInfoW(hmenu, (OPEN + i) as u32, false, &mii);
            }
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            add(hmenu, SAVE, "Guardar paneles y carpetas como grupo…");
            if !self.groups.items.is_empty() {
                if let Ok(sub) = CreatePopupMenu() {
                    for (i, g) in self.groups.items.iter().enumerate() {
                        add(sub, DELETE + i, &g.name);
                    }
                    let wide: Vec<u16> = "Eliminar".encode_utf16().chain(Some(0)).collect();
                    let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, PCWSTR(wide.as_ptr()));
                }
                add(hmenu, EDIT, "Editar grupos.toml");
            }
            let _ = ClientToScreen(self.hwnd.hwnd(), &mut pt);
            // Hacia arriba desde el botón: la barra de estado está abajo de todo.
            let flags = TPM_RETURNCMD | TPM_LEFTALIGN | TPM_BOTTOMALIGN;
            let cmd = TrackPopupMenuEx(hmenu, flags.0, pt.x, pt.y, self.hwnd.hwnd(), None).0 as usize;
            let _ = DestroyMenu(hmenu);
            self.groups_menu_closed = (cmd == 0).then(Instant::now);
            if let Some((_, bitmap)) = dot {
                let _ = DeleteObject(bitmap.into());
            }
            cmd
        };
        match cmd {
            SAVE => self.open_palette(PaletteMode::GroupName),
            EDIT => {
                if let Some(dir) = session::config_dir() {
                    open_in_editor(self.hwnd.hwnd(), &dir.join("grupos.toml"));
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

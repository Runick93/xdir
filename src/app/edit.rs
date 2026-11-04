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
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_RENAME_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem_len as isize)));
            let _ = SetFocus(Some(edit));
            self.rename = Some(Rename { edit, path });
        }
    }

    pub(super) fn end_rename(&mut self, commit: bool) {
        let Some(Rename { edit, path }) = self.rename.take() else { return };
        let mut buf = vec![0u16; 1024];
        let text = unsafe {
            let len = GetWindowTextW(edit, &mut buf);
            let _ = DestroyWindow(edit);
            let _ = SetFocus(Some(self.hwnd));
            String::from_utf16_lossy(&buf[..len.max(0) as usize])
        };
        let new_name = text.trim().to_string();
        let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if commit && !new_name.is_empty() && new_name != old_name {
            ops::spawn(self.hwnd, self.ops_tx.clone(), Op::Rename { path, new_name });
        }
        self.redraw();
    }

    // ---------- barra de dirección editable ----------

    pub(super) fn begin_edit(&mut self, pane: usize) {
        if self.edit.is_some() {
            return;
        }
        self.set_active(pane);
        let text = match &self.panes[pane].tab().location {
            Location::Computer => "Este equipo".to_string(),
            Location::Path(p) => p.to_string_lossy().into_owned(),
            Location::Shell { parsing, .. } => parsing.clone(),
        };
        let r = self.panes[pane].address;
        let k = self.dpi as f32 / 96.0;
        let (x, w) = ((r.left + 10.0) * k, (r.right - r.left - 20.0 - pane::ADDRESS_CLEAR_W) * k);
        let h = 20.0 * k;
        let y = (r.top + r.bottom) / 2.0 * k - h / 2.0;
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                x as i32,
                y as i32,
                w as i32,
                h as i32,
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
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_EDIT_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetFocus(Some(edit));
            self.edit = Some((edit, pane));
        }
        // Como el Explorador: al abrir, el historial de rutas escritas.
        self.suggest = Some(typed::Suggest::new(r, ""));
        self.redraw();
    }

    /// Flecha de la barra de dirección: menú con las subcarpetas del tramo anterior (o las unidades,
    /// si es "Este equipo"); elegir una navega a ella.
    pub(super) fn show_subfolder_menu(&mut self, pane: usize, k: usize) {
        let Some((r, loc)) = self.panes[pane].chevron(k) else { return };
        let items = fs::subfolders(&loc);
        self.set_active(pane);
        self.panes[pane].open_chevron = Some(k);
        self.redraw();
        let kk = self.dpi as f32 / 96.0;
        let mut pt = POINT { x: (r.left * kk) as i32, y: (r.bottom * kk) as i32 + 2 };
        let cmd = unsafe {
            let _ = windows::Win32::Graphics::Gdi::UpdateWindow(self.hwnd);
            let Ok(hmenu) = CreatePopupMenu() else { return };
            for (i, (name, _)) in items.iter().enumerate() {
                let wide: Vec<u16> = name.replace('&', "&&").encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(hmenu, MF_STRING, i + 1, PCWSTR(wide.as_ptr()));
            }
            if items.is_empty() {
                let wide: Vec<u16> = "Sin subcarpetas".encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(hmenu, MF_STRING | MF_GRAYED, 0, PCWSTR(wide.as_ptr()));
            }
            let _ = ClientToScreen(self.hwnd, &mut pt);
            let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN).0, pt.x, pt.y, self.hwnd, None).0 as usize;
            let _ = DestroyMenu(hmenu);
            cmd
        };
        self.panes[pane].open_chevron = None;
        if let Some((_, target)) = cmd.checked_sub(1).and_then(|i| items.get(i)) {
            let (hwnd, tx) = (self.hwnd, self.tx.clone());
            self.panes[pane].tab_mut().navigate(hwnd, &tx, target.clone());
        }
        self.redraw();
    }

    /// Cruz de la barra de dirección: borra lo escrito y sigue editando (con el historial a la vista).
    pub(super) fn clear_edit(&mut self) {
        let Some((edit, _)) = self.edit else { return };
        unsafe {
            let _ = SetWindowTextW(edit, w!(""));
            let _ = SetFocus(Some(edit));
        }
        if let Some(sg) = &mut self.suggest {
            sg.set_query("");
        }
        self.redraw();
    }

    /// Flechas y Re Pág / Av Pág sobre el desplegable: la barra muestra la ruta elegida.
    pub(super) fn on_suggest_key(&mut self, vk: VIRTUAL_KEY) {
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
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
                SendMessageW(edit, EM_SETSEL, Some(WPARAM(text.encode_utf16().count())), Some(LPARAM(-1)));
            }
            sg.quiet = false;
            self.redraw();
        }
    }

    /// Clic en una ruta del desplegable: se navega a ella.
    pub(super) fn pick_suggestion(&mut self, index: usize) {
        let (Some((edit, _)), Some(path)) = (self.edit, self.suggest.as_ref().and_then(|s| s.items.get(index).cloned())) else { return };
        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
        }
        self.end_edit(true);
    }

    pub(super) fn end_edit(&mut self, commit: bool) {
        let Some((edit, pane)) = self.edit.take() else { return };
        self.suggest = None;
        let mut buf = vec![0u16; 32 * 1024];
        let text = unsafe {
            let len = GetWindowTextW(edit, &mut buf);
            let _ = DestroyWindow(edit);
            let _ = SetFocus(Some(self.hwnd));
            String::from_utf16_lossy(&buf[..len.max(0) as usize])
        };
        if commit {
            self.go_to_text(pane, &text);
            let typed = text.trim().trim_matches('"');
            if std::path::Path::new(typed).exists() {
                typed::push(typed);
            }
        }
        self.redraw();
    }


    /// Abre el filtro rápido del panel (o le devuelve el foco si ya está abierto).
    pub(super) fn begin_filter(&mut self, pane: usize) {
        if let Some((edit, p)) = self.filter_edit {
            if p == pane {
                unsafe {
                    let _ = SetFocus(Some(edit));
                }
                return;
            }
            self.end_filter(true);
        }
        self.end_edit(false);
        self.set_active(pane);
        let r = self.panes[pane].filter_text_rect();
        let k = self.dpi as f32 / 96.0;
        let h = 20.0 * k;
        let (x, w) = (r.left * k, (r.right - r.left) * k);
        let y = (r.top + r.bottom) / 2.0 * k - h / 2.0;
        let wide: Vec<u16> = self.panes[pane].tab().list.filter().encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                x as i32,
                y as i32,
                w as i32,
                h as i32,
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
            let _ = SetWindowSubclass(edit, Some(edit_proc), 1, WM_APP_FILTER_DONE as usize);
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SetFocus(Some(edit));
            self.filter_edit = Some((edit, pane));
        }
        self.redraw();
    }

    /// Cierra el control del filtro. `keep`: Enter o perder el foco lo dejan aplicado; Escape lo quita.
    pub(super) fn end_filter(&mut self, keep: bool) {
        let Some((edit, pane)) = self.filter_edit.take() else { return };
        unsafe {
            let _ = DestroyWindow(edit);
            if GetFocus().is_invalid() {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        if !keep {
            self.panes[pane].tab_mut().list.set_filter("");
        }
        self.redraw();
    }

    /// Quita el filtro del panel (cruz de la fila o Escape sobre la lista).
    pub(super) fn clear_filter(&mut self, pane: usize) {
        if self.filter_edit.is_some_and(|(_, p)| p == pane) {
            self.end_filter(false);
        } else {
            self.panes[pane].tab_mut().list.set_filter("");
            self.redraw();
        }
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
            match fs::shell_location(text) {
                Some(loc) => tab.navigate(hwnd, &tx, loc),
                None => unsafe {
                    let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONWARNING);
                },
            }
            return;
        }
        let mut path = PathBuf::from(expand_env(text));
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
            fs::connect_share(hwnd, &path);
            if !path.exists() {
                tab.navigate(hwnd, &tx, Location::Path(path));
                return;
            }
        }
        if path.is_dir() {
            // Con las mayúsculas del disco, no como se escribió (se ve en pestañas e historial).
            tab.navigate(hwnd, &tx, Location::Path(fs::true_case(&path)));
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
            unsafe {
                let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONWARNING);
            }
        }
    }

    pub(super) fn update_edit_resources(&mut self) {
        unsafe {
            if !self.edit_font.is_invalid() {
                let _ = DeleteObject(self.edit_font.into());
            }
            if !self.edit_brush.is_invalid() {
                let _ = DeleteObject(self.edit_brush.into());
            }
            self.edit_font = CreateFontW(
                -(13 * self.dpi as i32 / 96),
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            self.edit_brush = CreateSolidBrush(colorref(self.theme.surface_alt));
        }
    }
}

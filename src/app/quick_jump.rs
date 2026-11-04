//! Salto rápido y paleta de comandos.

use super::*;

impl App {
    /// Abre el salto rápido o la paleta. Si ya está abierta, cambia de modo conservando lo escrito.
    pub(super) fn open_palette(&mut self, mode: PaletteMode) {
        self.group_rename = None;
        if let Some((pal, edit)) = &mut self.palette {
            pal.mode = mode;
            let edit = *edit;
            self.refresh_palette();
            unsafe {
                let _ = SetFocus(Some(edit));
            }
            return;
        }
        self.end_edit(false);
        self.end_filter(true);
        self.picker_open = false;
        self.term_menu = None;
        let (w, _) = self.client_dips();
        let items = palette::search(mode, "", &Sources { history: &self.history, favorites: &self.favorites, groups: &self.groups, keymap: &self.config.keymap }, expand_env);
        let pal = Palette::new(mode, items);
        let r = pal.input_rect(w);
        let k = self.dpi as f32 / 96.0;
        let h = 20.0 * k;
        let (x, width) = (r.left * k, (r.right - r.left) * k);
        let y = (r.top + r.bottom) / 2.0 * k - h / 2.0;
        unsafe {
            let Ok(edit) = CreateWindowExW(
                WS_EX_LAYERED,
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                x as i32,
                y as i32,
                width as i32,
                h as i32,
                Some(self.hwnd),
                None,
                None,
                None,
            ) else {
                return;
            };
            let _ = SetLayeredWindowAttributes(edit, COLORREF(0), 255, LWA_ALPHA);
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(self.edit_font.0 as usize)), Some(LPARAM(1)));
            let _ = SetWindowSubclass(edit, Some(palette_proc), 2, 0);
            let _ = SetFocus(Some(edit));
            self.palette = Some((pal, edit));
        }
        self.redraw();
    }

    /// Vuelve a buscar con lo escrito (cambió el texto o el modo).
    pub(super) fn refresh_palette(&mut self) {
        let Some((pal, edit)) = &mut self.palette else { return };
        let query = window_text(*edit);
        pal.set_items(palette::search(pal.mode, &query, &Sources { history: &self.history, favorites: &self.favorites, groups: &self.groups, keymap: &self.config.keymap }, expand_env));
        self.redraw();
    }

    pub(super) fn on_palette_key(&mut self, vk: VIRTUAL_KEY, ctrl: bool) {
        let Some((pal, edit)) = &mut self.palette else { return };
        match vk {
            VK_UP => pal.move_by(-1),
            VK_DOWN => pal.move_by(1),
            VK_PRIOR => pal.move_by(-(palette::MAX_ROWS as i32)),
            VK_NEXT => pal.move_by(palette::MAX_ROWS as i32),
            VK_RETURN => {
                let i = pal.selected;
                self.activate_palette(i, ctrl);
                return;
            }
            // Ctrl+P / Ctrl+Shift+P dentro de la paleta: cambiar de modo.
            VIRTUAL_KEY(0x50) => {
                let mode = if key_down(VK_SHIFT) { PaletteMode::All } else { PaletteMode::Folders };
                self.open_palette(mode);
                return;
            }
            VK_ESCAPE => self.close_palette(),
            // Perdió el foco: se cierra, salvo que el foco haya vuelto (clic dentro del panel).
            _ => {
                let edit = *edit;
                if unsafe { GetFocus() } != edit {
                    self.close_palette();
                }
            }
        }
        self.redraw();
    }

    pub(super) fn close_palette(&mut self) {
        let Some((_, edit)) = self.palette.take() else { return };
        unsafe {
            let _ = DestroyWindow(edit);
            if GetFocus().is_invalid() {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        self.redraw();
    }

    /// Abre un resultado: ejecuta la acción o va a la carpeta (con Ctrl, en una pestaña nueva).
    pub(super) fn activate_palette(&mut self, index: usize, ctrl: bool) {
        let Some(item) = self.palette.as_ref().and_then(|(p, _)| p.items.get(index).cloned()) else { return };
        self.close_palette();
        match item.target {
            Target::Command(c) => self.run_command(c),
            Target::Group(i) => self.apply_group(i),
            Target::SaveGroup(name) => self.save_group(&name),
            Target::Folder(loc) => {
                if let Location::Path(p) = &loc {
                    if !p.is_dir() {
                        // Ya no existe: sale del historial.
                        self.history.forget(p);
                        self.notice = Some(format!("No existe: {}", p.display()));
                        self.redraw();
                        return;
                    }
                }
                let active = self.active;
                if ctrl {
                    self.new_tab(active, loc, true);
                } else {
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    self.tab_mut().navigate(hwnd, &tx, loc);
                }
            }
        }
        self.redraw();
    }
}

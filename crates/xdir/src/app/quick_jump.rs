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
            edit.focus();
            return;
        }
        self.end_edit(false);
        self.end_filter(true);
        self.picker_open = false;
        self.term_menu = None;
        let (w, _) = self.client_dips();
        let items = palette::search(mode, "", &Sources { history: &self.history, favorites: &self.favorites, groups: &self.groups, keymap: &self.config.keymap }, |s| self.host.expand_env(s));
        let pal = Palette::new(mode, items);
        let r = pal.input_rect(w);
        let y = (r.top + r.bottom) / 2.0 - 10.0;
        let Some(edit) = TextBox::create(self, TextKind::Palette, render::rect(r.left, y, r.right, y + 20.0), "") else { return };
        edit.focus();
        self.palette = Some((pal, edit));
        self.redraw();
    }

    /// Vuelve a buscar con lo escrito (cambió el texto o el modo).
    pub(super) fn refresh_palette(&mut self) {
        let host = self.host.clone();
        let Some((pal, edit)) = &mut self.palette else { return };
        let query = edit.text();
        pal.set_items(palette::search(pal.mode, &query, &Sources { history: &self.history, favorites: &self.favorites, groups: &self.groups, keymap: &self.config.keymap }, |s| host.expand_env(s)));
        self.redraw();
    }

    pub(super) fn on_palette_key(&mut self, vk: Key, ctrl: bool) {
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
            Key(0x50) => {
                let mode = if key_down(VK_SHIFT) { PaletteMode::All } else { PaletteMode::Folders };
                self.open_palette(mode);
                return;
            }
            VK_ESCAPE => self.close_palette(),
            // Perdió el foco: se cierra, salvo que el foco haya vuelto (clic dentro del panel).
            _ => {
                if !edit.has_focus() {
                    self.close_palette();
                }
            }
        }
        self.redraw();
    }

    pub(super) fn close_palette(&mut self) {
        let Some((_, edit)) = self.palette.take() else { return };
        edit.destroy();
        self.host.focus_main_if_lost();
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
                // La carpeta se abre en un panel: la vista de xtree no debe taparlo.
                self.close_xtree();
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

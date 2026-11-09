//! Consola y color de acento / modo oscuro.

use super::*;

impl App {
    // ---------- consola ----------

    /// Abre la consola en la carpeta del panel.
    pub(super) fn open_console(&mut self, pane: usize, console: Console) {
        if let Location::Path(dir) = self.panes[pane].tab().location.clone() {
            if !console.open(self.hwnd, &dir) {
                self.notice = Some(format!("No se pudo abrir {}", console.label()));
            }
        }
        self.redraw();
    }

    // ---------- color de acento ----------

    pub(super) fn set_accent(&mut self, accent: Accent) {
        self.accent = accent;
        self.theme = Theme::from_system(accent, self.dark_mode);
        self.update_edit_resources();
        self.redraw();
    }

    /// Switch del panel de la rueda dentada: fija oscuro o claro (deja de seguir a Windows).
    pub(super) fn set_dark_mode(&mut self, dark: bool) {
        self.dark_mode = Some(dark);
        self.theme = Theme::from_system(self.accent, self.dark_mode);
        // Barra de título y menús nativos al mismo modo.
        self.apply_window_theme();
        shell::set_menu_mode(self.dark_mode);
        self.redraw();
    }
}

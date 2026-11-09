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

    pub fn set_accent(&mut self, accent: Accent) {
        self.accent = accent;
        self.theme = Theme::from_system(accent, self.dark_mode);
        self.host.text_restyle(&self.theme);
        self.redraw();
    }

    /// "Personalizado…": el selector de color del sistema. En Windows es modal y el color se aplica
    /// al volver; en macOS el panel queda abierto y cada color llega después (`MSG_ACCENT`).
    pub(super) fn choose_custom_color(&mut self) {
        let current = match self.accent {
            Accent::Rgb(c) => c,
            Accent::System => theme::PRESETS[0].1,
        };
        if let Some(c) = self.host.choose_color(current) {
            self.set_accent(Accent::Rgb(c));
        }
    }

    /// Ajusta la ventana al tema (barra de título, fondo Mica en Windows) y los controles de edición.
    pub fn apply_window_theme(&mut self) {
        // El host también pasa el renderizador a fondo translúcido si quedó Mica.
        let mica = self.host.apply_theme(&self.theme, self.dark_mode.is_none());
        self.transparent = mica;
        self.host.text_restyle(&self.theme);
    }

    /// Switch «Mostrar extensiones» de la rueda dentada: en todas las pestañas a la vez (las nuevas
    /// lo toman en `make_tab`).
    pub(super) fn set_show_extensions(&mut self, on: bool) {
        self.show_extensions = on;
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.list.show_extensions = on;
        }
        self.redraw();
    }

    /// Switch del panel de la rueda dentada: fija oscuro o claro (deja de seguir a Windows).
    pub(super) fn set_dark_mode(&mut self, dark: bool) {
        self.dark_mode = Some(dark);
        self.theme = Theme::from_system(self.accent, self.dark_mode);
        // Barra de título y menús nativos al mismo modo.
        self.apply_window_theme();
        sys::shell().set_menu_mode(self.dark_mode);
        self.redraw();
    }
}

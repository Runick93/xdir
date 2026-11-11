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

    /// Switch «Casillas de selección»: en todas las pestañas a la vez (las nuevas lo toman en
    /// `make_tab`).
    pub(super) fn set_checkboxes(&mut self, on: bool) {
        self.checkboxes = on;
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.list.checkboxes = on;
        }
        self.redraw();
    }

    /// El árbol sigue al panel activo: lo que eligió el usuario en la rueda dentada o, si nunca lo
    /// tocó, `arbol_sigue_panel` de `config.toml`.
    pub(super) fn tree_follows(&self) -> bool {
        self.tree_follow.unwrap_or(self.config.tree_follows_pane)
    }

    /// Switch «El árbol sigue al panel»: desde ahora manda lo elegido (se guarda en la sesión).
    #[cfg(feature = "arbol")]
    pub(super) fn set_tree_follow(&mut self, on: bool) {
        self.tree_follow = Some(on);
        // Al prenderlo, el árbol va enseguida a la carpeta del panel.
        self.tree_followed = None;
        self.redraw();
    }

    /// Estado de los switches de la rueda dentada.
    pub(super) fn picker_switches(&self) -> ui::PickerSwitches {
        ui::PickerSwitches {
            file_colors: self.file_colors,
            extensions: self.show_extensions,
            checkboxes: self.checkboxes,
            tree_follow: self.tree_follows(),
            hidden: self.show_hidden,
            group_bar: self.show_group_bar,
            tree: self.tree_visible(),
        }
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

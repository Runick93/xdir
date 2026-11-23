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

    /// Las consolas del menú `>_`: las del sistema y la propia de `config.toml`.
    pub(super) fn consoles(&self) -> Vec<Console> {
        Console::all(self.config.custom_terminal.as_deref())
    }

    /// La terminal que abre «Abrir la terminal en la carpeta»: la de `[opciones] terminal` si
    /// está, si no la primera del sistema.
    pub(super) fn default_console(&self) -> Console {
        Console::preferred(self.config.terminal.as_deref(), self.config.custom_terminal.as_deref())
    }

    /// Elige la terminal por defecto y la guarda en `config.toml`. La primera del sistema es la
    /// de fábrica: elegirla quita la línea.
    pub(super) fn set_default_terminal(&mut self, console: Console) {
        let first = Console::all(None).first().copied();
        let key = "terminal".to_string();
        let edit = if first == Some(console) {
            self.config.terminal = None;
            config::ConfigEdit::Remove { section: "opciones", key }
        } else {
            self.config.terminal = Some(console.key().to_string());
            config::ConfigEdit::Set { section: "opciones", key, value: crate::toml::quote(console.key()) }
        };
        self.edit_config(vec![edit]);
        self.redraw();
    }

    // ---------- color de acento ----------

    /// El tema con el acento, el modo y el fondo de ahora.
    pub(super) fn make_theme(&self) -> Theme {
        Theme::with_background(self.accent, self.dark_mode, self.background)
    }

    pub fn set_accent(&mut self, accent: Accent) {
        self.accent = accent;
        self.theme = self.make_theme();
        self.host.text_restyle(&self.theme);
        self.redraw();
    }

    /// «Otro…»: abre el selector de color propio de xdir (no el del sistema). Al aceptar, el color
    /// es el nuevo acento; Esc cancela sin cambiar nada.
    pub(super) fn choose_custom_color(&mut self) {
        let current = match self.accent {
            Accent::Rgb(c) => c,
            Accent::System => theme::PRESETS[0].1,
        };
        self.open_color_picker(color_picker::PickTarget::Accent, current);
    }

    /// «Elegir color…» del fondo: el mismo selector, partiendo del fondo de ahora.
    pub(super) fn choose_background(&mut self) {
        let current = self.background.unwrap_or_else(|| {
            let c = self.theme.background;
            let ch = |v: f32| (v * 255.0).round() as u32;
            ch(c.r) << 16 | ch(c.g) << 8 | ch(c.b)
        });
        self.open_color_picker(color_picker::PickTarget::Background, current);
    }

    fn open_color_picker(&mut self, target: color_picker::PickTarget, current: u32) {
        self.color_picker = Some(color_picker::ColorPicker::new(target, current));
        self.redraw();
    }

    /// Fondo propio (`None`: vuelve al del modo claro u oscuro). Las superficies, los bordes y los
    /// textos se derivan de él (ver `Theme::with_background`).
    pub fn set_background(&mut self, background: Option<u32>) {
        self.background = background;
        self.theme = self.make_theme();
        // La barra de título y los menús siguen al modo que resulte del fondo.
        self.apply_window_theme();
        sys::shell().set_menu_mode(Some(self.theme.dark).filter(|_| background.is_some()).or(self.dark_mode));
        self.redraw();
    }

    /// Se resolvió el selector: lo elegido se aplica a lo que se estaba eligiendo.
    fn color_picker_done(&mut self, target: color_picker::PickTarget, outcome: color_picker::Outcome) {
        match outcome {
            color_picker::Outcome::Open => return,
            color_picker::Outcome::Cancel => {}
            color_picker::Outcome::Accept(c) => match target {
                color_picker::PickTarget::Accent => self.set_accent(Accent::Rgb(c)),
                color_picker::PickTarget::Background => self.set_background(Some(c)),
            },
        }
        self.color_picker = None;
        self.redraw();
    }

    /// Clic en el selector (el cuadrado y la franja empiezan un arrastre).
    pub(super) fn color_picker_click(&mut self, x: f32, y: f32) {
        let (w, h) = self.client_dips();
        let Some(pk) = &mut self.color_picker else { return };
        let target = pk.target;
        let outcome = pk.press(w, h, x, y);
        if pk.is_dragging() {
            self.host.capture_mouse();
        }
        self.color_picker_done(target, outcome);
        self.redraw();
    }

    /// El mouse se mueve con el botón apretado sobre el selector. `true` si lo usó.
    pub(super) fn color_picker_drag(&mut self, x: f32, y: f32) -> bool {
        let (w, h) = self.client_dips();
        let moved = self.color_picker.as_mut().is_some_and(|pk| pk.drag(w, h, x, y));
        if moved {
            self.redraw();
        }
        moved
    }

    /// Una tecla con el selector abierto: se queda con todas.
    pub(super) fn color_picker_key(&mut self, vk: Key, shift: bool, ctrl: bool) -> bool {
        let Some(pk) = &mut self.color_picker else { return false };
        let target = pk.target;
        let outcome = pk.key(vk, shift, ctrl);
        self.color_picker_done(target, outcome);
        self.redraw();
        true
    }

    /// Ajusta la ventana al tema (barra de título, fondo Mica en Windows) y los controles de edición.
    pub fn apply_window_theme(&mut self) {
        // El host también pasa el renderizador a fondo translúcido si quedó Mica.
        // Con un fondo propio no hay Mica: se vería el del sistema en vez del elegido.
        let mica = self.host.apply_theme(&self.theme, self.dark_mode.is_none() && self.background.is_none());
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
        self.theme = self.make_theme();
        // Barra de título y menús nativos al mismo modo.
        self.apply_window_theme();
        sys::shell().set_menu_mode(self.dark_mode);
        self.redraw();
    }
}

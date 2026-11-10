//! La pestaña Configuración desde la `App`: abrirla (una sola), su página, el dibujo de cada
//! cuadro, los clics y la rueda.

use super::settings_pages::PAGES;
use super::*;
use crate::config::ConfigEdit;
use crate::paint::Painter;
use crate::settings::draw::NavItem;
use crate::settings::options::{self, OptionSpec};
use crate::settings::{AccentChoice, Control, Item, PageId, Row, Section, SettingsHit};

impl App {
    /// Las páginas de la lista de la izquierda: las secciones propias con las extra (Red) después
    /// de la que dicen.
    pub(super) fn settings_nav(&self) -> Vec<PageId> {
        let mut out = Vec::new();
        for s in Section::ALL {
            out.push(PageId::Builtin(s));
            out.extend(PAGES.iter().filter(|p| p.after == s).map(|p| PageId::Extra(p.id)));
        }
        out
    }

    /// Nombre de una página.
    pub(super) fn page_title(page: PageId) -> &'static str {
        match page {
            PageId::Builtin(s) => s.title(),
            PageId::Extra(id) => PAGES.iter().find(|p| p.id == id).map_or("", |p| p.title),
        }
    }

    /// La página de un ancla de la sesión (`atajos`, `red`); desconocida → Apariencia.
    pub(super) fn page_from_id(id: &str) -> PageId {
        if let Some(s) = Section::from_id(id) {
            return PageId::Builtin(s);
        }
        PAGES.iter().find(|p| p.id == id).map_or(PageId::default(), |p| PageId::Extra(p.id))
    }

    /// El ancla de la sesión de una página.
    pub(super) fn page_id(page: PageId) -> &'static str {
        match page {
            PageId::Builtin(s) => s.id(),
            PageId::Extra(id) => id,
        }
    }

    /// Dónde está la pestaña Configuración: (panel, pestaña).
    pub(super) fn settings_tab(&self) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(i, p)| p.tabs.iter().position(|t| t.is_settings()).map(|j| (i, j)))
    }

    /// La vista de la pestaña Configuración del panel, si es la que está a la vista.
    pub(super) fn settings_view(&self, pane: usize) -> Option<&crate::settings::SettingsView> {
        self.panes.get(pane)?.tab().settings.as_ref()
    }

    pub(super) fn settings_view_mut(&mut self, pane: usize) -> Option<&mut crate::settings::SettingsView> {
        self.panes.get_mut(pane)?.tab_mut().settings.as_mut()
    }

    /// Abre la configuración: si ya hay una pestaña (en cualquier panel), la activa con su panel;
    /// si no, abre una nueva en el panel activo (no reemplaza la carpeta). `page`: la página.
    pub(super) fn open_settings(&mut self, page: Option<PageId>) {
        self.picker_open = false;
        self.term_menu = None;
        if self.xtree_open {
            self.close_xtree();
        }
        match self.settings_tab() {
            // En un panel que el layout no muestra: se trae al activo.
            Some((pane, index)) if !self.shown_panes().contains(&pane) => {
                let at = self.panes[self.active].tabs.len();
                self.move_tab_to_pane(pane, index, self.active, at);
            }
            Some((pane, index)) => {
                self.set_active(pane);
                self.select_tab(pane, index);
            }
            None => {
                let from = self.panes[self.active].tab().id;
                self.new_tab(self.active, Location::Settings, true);
                if let Some(v) = self.settings_view_mut(self.active) {
                    v.from_tab = Some(from);
                }
            }
        }
        if let Some(page) = page
            && let Some(v) = self.settings_view_mut(self.active)
        {
            v.set_page(page);
        }
        self.redraw();
    }

    /// Lo que va en el contenido de una página propia (las opciones con sus valores de ahora),
    /// en el orden de la tabla de opciones.
    pub(super) fn settings_items(&self, section: Section) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some(e) = &self.config_error {
            items.push(Item::Error(format!("No se pudo guardar config.toml: {e}"), Some(("error_config", "Abrir config.toml".to_string()))));
        }
        for spec in options::of(section) {
            items.extend(self.option_items(spec));
        }
        if section == Section::Appearance {
            items.push(Item::Note("La rueda dentada de la barra de estado tiene los ajustes rápidos; lo que se cambia en un lugar se ve en el otro.".into()));
        }
        items
    }

    /// Las filas de una opción con sus valores de ahora (ninguna si no aplica en esta compilación).
    pub(super) fn option_items(&self, spec: &'static OptionSpec) -> Vec<Item> {
        let row = |control: Control| Row::new(spec.id, spec.title, spec.desc, control);
        let one = |r: Row| vec![Item::Row(r)];
        let arbol = cfg!(feature = "arbol");
        match spec.id {
            "tema" => {
                let sel = match self.dark_mode {
                    Some(false) => 0,
                    Some(true) => 1,
                    None => 2,
                };
                one(row(Control::Segmented(vec!["Claro", "Oscuro", "Sistema"], Some(sel))))
            }
            "acento" => {
                let choice = match self.accent {
                    Accent::System => AccentChoice::System,
                    Accent::Rgb(c) => theme::PRESETS.iter().position(|(_, p)| *p == c).map_or(AccentChoice::Other(c), AccentChoice::Preset),
                };
                one(row(Control::Accent(choice)))
            }
            "animaciones" => {
                let sel = match self.config.animations {
                    None => 0,
                    Some(true) => 1,
                    Some(false) => 2,
                };
                one(row(Control::Segmented(vec!["Sistema", "Sí", "No"], Some(sel))))
            }
            "extensiones" => one(row(Control::Switch(self.show_extensions))),
            "ocultos" => one(row(Control::Switch(self.show_hidden))),
            "casillas" => one(row(Control::Switch(self.checkboxes))),
            "barra_grupos" => one(row(Control::Switch(self.show_group_bar))),
            "colores_tipo" => {
                let rules = self
                    .config
                    .colors
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (config::rule_label(r), r.color, i < self.config.own_colors))
                    .collect();
                vec![
                    Item::Row(row(Control::Switch(self.file_colors))),
                    Item::Row(Row::new("reglas_color", "Reglas activas", "Propias primero, después las de fábrica; se cambian en config.toml", Control::ColorRules(rules))),
                ]
            }
            "vista_defecto" => {
                let v = self.folder_views.default_view().view;
                let name = list::VIEWS.iter().find(|x| x.0 == v).map_or("", |x| x.1);
                one(row(Control::Dropdown(name.to_string())))
            }
            "vistas_carpetas" => {
                let from = self.settings_from_tab();
                let label = match &from {
                    Some(t) => format!("Usar la vista de \"{}\" en todas", t.location.title()),
                    None => "Usar la vista de la carpeta en todas".to_string(),
                };
                one(row(Control::Buttons(vec![(label, from.is_some()), ("Restablecer las vistas…".to_string(), true)])))
            }
            "arbol" if !arbol => Vec::new(),
            "arbol" => one(row(Control::Switch(self.tree_visible()))),
            "arbol_fijo" => one(Row { enabled: arbol, ..row(Control::Switch(self.tree_pinned())) }),
            "arbol_sigue" => one(row(Control::Switch(self.tree_follows()))),
            "arbol_config" => {
                let now = if self.config.tree_follows_pane { "sí" } else { "no" };
                let desc = format!("Ahora dice que {now}; una vez tocado el switch, manda el switch");
                one(Row { desc, ..row(Control::Buttons(vec![("Volver a lo de config.toml".to_string(), self.tree_follow.is_some())])) })
            }
            "vista_previa" => one(row(Control::Switch(self.preview.on))),
            "vista_previa_ancho" => one(row(Control::Slider { value: self.preview.width, min: ui::PREVIEW_MIN_W, max: ui::PREVIEW_MAX_W })),
            "xtree_por_tipo" => one(row(Control::Switch(self.config.xtree_by_type))),
            "xtree_abrir" => one(row(Control::Dropdown(self.config.xtree_open.title().to_string()))),
            "xtree_teclas" => vec![
                Item::Row(row(Control::Table(
                    [
                        ("T", "Colorear el mapa por tipo de archivo o por carpeta"),
                        ("F5", "Volver a analizar"),
                        ("⌫", "Subir a la carpeta de arriba"),
                        ("A", "Analizar de nuevo con permisos de administrador"),
                        ("Esc", "Cerrar xtree"),
                    ]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                ))),
                Item::Action("ver_atajos", "Ver los atajos".to_string()),
            ],
            "abrir_config" => {
                let path = config::file_path().map(|p| p.display().to_string()).unwrap_or_default();
                one(Row { desc: path, ..row(Control::Buttons(vec![("Abrir config.toml".to_string(), true)])) })
            }
            "carpeta_config" => {
                let dir = session::config_dir().map(|p| p.display().to_string()).unwrap_or_else(|| "(no hay)".into());
                let label = format!("Mostrar en {}", sys::env().file_manager_name());
                one(Row { desc: dir, ..row(Control::Buttons(vec![(label, true)])) })
            }
            "archivos" => {
                let mac = sys::env().key_style() == sys::KeyStyle::Mac;
                let rows = session::files(mac).into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
                one(row(Control::Table(rows)))
            }
            "portable" => {
                let on = session::portable_active();
                let mut desc = if on { "Los archivos se guardan junto a xdir" } else { "Los archivos se guardan en la carpeta del usuario" }.to_string();
                if let Some(m) = &self.portable_msg {
                    desc = format!("{desc} · {m}");
                }
                let control = if sys::env().portable_supported() {
                    Control::Buttons(vec![if on { ("Desactivar".to_string(), true) } else { ("Activar el modo portable".to_string(), true) }])
                } else {
                    Control::None
                };
                one(Row { desc, ..row(control) })
            }
            _ => Vec::new(),
        }
    }

    /// La pestaña de carpeta desde la que se abrió la configuración, si sigue abierta.
    pub(super) fn settings_from_tab(&self) -> Option<&Tab> {
        let (pane, index) = self.settings_tab()?;
        let id = self.panes[pane].tabs[index].settings.as_ref()?.from_tab?;
        self.panes.iter().flat_map(|p| p.tabs.iter()).find(|t| t.id == id && !t.is_settings())
    }

    /// Dónde se dibujó una parte de una fila en el último cuadro.
    pub(super) fn part_rect(&self, pane: usize, id: &'static str, part: usize) -> Option<geom::Rect> {
        self.settings_view(pane)?.hits.iter().find(|(_, h)| *h == SettingsHit::Part(id, part)).map(|(r, _)| *r)
    }

    // ---------- escribir config.toml ----------

    /// Guarda cambios en `config.toml` (lo de memoria ya cambió). Una sola escritura a la vez: si
    /// hay una en camino, estos esperan y se escriben juntos cuando termine.
    pub(super) fn edit_config(&mut self, edits: Vec<ConfigEdit>) {
        self.config_pending.extend(edits);
        if !self.config_saving {
            self.flush_config();
        }
    }

    fn flush_config(&mut self) {
        if self.config_pending.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut self.config_pending);
        self.config_saving = true;
        // La ruta se resuelve acá (la carpeta de configuración es una sola para toda la ventana).
        let path = config::file_path();
        self.spawn_task(move || {
            let result = match path {
                Some(p) => config::write_edits_at(&p, &edits),
                None => Err("no hay carpeta de configuración".to_string()),
            };
            tasks::TaskDone::ConfigSaved { result }
        });
    }

    /// Terminó de escribir `config.toml`.
    pub(super) fn on_config_saved(&mut self, result: Result<Config, String>) {
        self.config_saving = false;
        match result {
            // Lo escrito (con lo que se haya cambiado a mano en el medio) pasa a ser la
            // configuración; su fecha evita que `reload_config` lo tome como un cambio de afuera.
            // Si hay más cambios esperando, la memoria ya los tiene: solo se toma la fecha.
            Ok(c) if self.config_pending.is_empty() => {
                let animations_changed = c.animations != self.config.animations;
                self.config = c;
                self.config_error = None;
                if animations_changed {
                    self.apply_settings_animations();
                }
            }
            Ok(c) => {
                self.config.take_modified(&c);
                self.config_error = None;
            }
            Err(e) => self.config_error = Some(e),
        }
        self.flush_config();
        self.redraw();
    }

    // ---------- clics en las filas ----------

    /// Tema: claro, oscuro o el del sistema.
    pub(super) fn set_theme_mode(&mut self, mode: Option<bool>) {
        self.dark_mode = mode;
        self.theme = Theme::from_system(self.accent, self.dark_mode);
        self.apply_window_theme();
        sys::shell().set_menu_mode(self.dark_mode);
        self.redraw();
    }

    /// Aplica la opción de animaciones. En las pruebas no: es un estado de todo el proceso y
    /// otras pruebas que corren a la vez miran el scroll suave.
    fn apply_settings_animations(&mut self) {
        if !cfg!(test) {
            self.apply_animation_setting();
        }
    }

    /// Animaciones: `None` sigue al sistema (se quita la línea de `config.toml`).
    pub(super) fn set_animations(&mut self, on: Option<bool>) {
        self.config.animations = on;
        self.apply_settings_animations();
        let key = "animaciones".to_string();
        self.edit_config(vec![match on {
            None => ConfigEdit::Remove { section: "opciones", key },
            Some(v) => ConfigEdit::Set { section: "opciones", key, value: v.to_string() },
        }]);
    }

    /// El árbol sigue al panel (`None`: lo que diga `config.toml`).
    pub(super) fn set_tree_follow_option(&mut self, on: Option<bool>) {
        match on {
            #[cfg(feature = "arbol")]
            Some(v) => self.set_tree_follow(v),
            _ => self.tree_follow = on,
        }
        self.redraw();
    }

    /// La vista por defecto de las carpetas sin vista propia.
    pub(super) fn set_default_view(&mut self, v: list::View) {
        self.folder_views.set_default_view(v);
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.reload_view();
        }
        self.sync_folder_views();
        self.schedule_views_save();
    }

    /// Un clic en una parte de una fila.
    pub(super) fn settings_part(&mut self, pane: usize, id: &'static str, part: usize, x: f32) {
        match (id, part) {
            ("tema", 0) => self.set_theme_mode(Some(false)),
            ("tema", 1) => self.set_theme_mode(Some(true)),
            ("tema", _) => self.set_theme_mode(None),
            ("acento", i) if i < theme::PRESETS.len() => self.set_accent(Accent::Rgb(theme::PRESETS[i].1)),
            ("acento", i) if i == theme::PRESETS.len() => self.set_accent(Accent::System),
            ("acento", _) => self.choose_custom_color(),
            ("animaciones", 0) => self.set_animations(None),
            ("animaciones", 1) => self.set_animations(Some(true)),
            ("animaciones", _) => self.set_animations(Some(false)),
            ("extensiones", _) => self.set_show_extensions(!self.show_extensions),
            ("ocultos", _) => self.run_command(Command::ToggleHidden),
            ("casillas", _) => self.set_checkboxes(!self.checkboxes),
            ("barra_grupos", _) => self.run_command(Command::ToggleGroupBar),
            ("colores_tipo", _) => self.file_colors = !self.file_colors,
            ("reglas_color", _) | ("error_config", _) => self.run_command(Command::OpenConfig),
            ("vista_defecto", _) => self.default_view_menu(pane),
            ("vistas_carpetas", 0) => {
                if let Some(id) = self.settings_from_tab().map(|t| t.id) {
                    self.apply_view_of_tab_to_all(id);
                }
            }
            ("vistas_carpetas", _) => {
                if self.host.ask_yes_no("Vistas de las carpetas", "¿Volver todas las carpetas a la vista de fábrica?") {
                    self.folder_views.apply_to_all(crate::folder_views::ViewSettings::default());
                    for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
                        tab.reload_view();
                    }
                    self.sync_folder_views();
                    self.schedule_views_save();
                }
            }
            ("arbol", _) => self.run_command(Command::ToggleTree),
            ("arbol_fijo", _) => self.run_command(Command::ToggleTreePin),
            ("arbol_sigue", _) => self.set_tree_follow_option(Some(!self.tree_follows())),
            ("arbol_config", _) => self.set_tree_follow_option(None),
            ("vista_previa", _) => self.run_command(Command::TogglePreview),
            ("xtree_por_tipo", _) => {
                let on = !self.config.xtree_by_type;
                self.config.xtree_by_type = on;
                let key = "xtree_por_tipo".to_string();
                self.edit_config(vec![if on {
                    ConfigEdit::Set { section: "opciones", key, value: "true".into() }
                } else {
                    ConfigEdit::Remove { section: "opciones", key }
                }]);
            }
            ("xtree_abrir", _) => self.xtree_open_menu(pane),
            ("ver_atajos", _) => self.open_settings(Some(PageId::Builtin(Section::Keys))),
            ("abrir_config", _) => self.run_command(Command::OpenConfig),
            ("carpeta_config", _) => {
                if let Some(dir) = session::config_dir() {
                    sys::shell().open_file(self.hwnd, &dir, None);
                }
            }
            ("portable", _) => self.toggle_portable(),
            ("vista_previa_ancho", _) => {
                self.host.capture_mouse();
                self.drag = Drag::SettingsSlider(pane, id);
                self.settings_slider(pane, id, x);
            }
            _ => {}
        }
        self.redraw();
    }

    /// Arrastre del deslizador `id` del panel a la posición `x`.
    pub(super) fn settings_slider(&mut self, pane: usize, id: &'static str, x: f32) {
        let Some(r) = self.part_rect(pane, id, 0) else { return };
        if id == "vista_previa_ancho" {
            self.preview.width = crate::settings::controls::slider_value(r, x, ui::PREVIEW_MIN_W, ui::PREVIEW_MAX_W);
        }
        self.redraw();
    }

    /// Qué analiza xtree al abrirse.
    pub(super) fn set_xtree_open(&mut self, open: config::XtreeOpen) {
        self.config.xtree_open = open;
        let key = "xtree_abrir".to_string();
        self.edit_config(vec![match open {
            config::XtreeOpen::Pane => ConfigEdit::Remove { section: "opciones", key },
            o => ConfigEdit::Set { section: "opciones", key, value: crate::toml::quote(o.id()) },
        }]);
    }

    /// El desplegable de «Disco que se analiza al abrir».
    fn xtree_open_menu(&mut self, pane: usize) {
        let current = self.config.xtree_open;
        let entries: Vec<shell::Entry> = config::XtreeOpen::ALL
            .iter()
            .enumerate()
            .map(|(i, o)| shell::Entry::Item { id: i as u32 + 1, label: o.title().to_string(), enabled: true, checked: *o == current })
            .collect();
        let at = self.part_rect(pane, "xtree_abrir", 0).map_or(MenuPos::Cursor, |r| MenuPos::At(r.left, r.bottom + 2.0));
        if let Some(i) = self.host.popup_menu(&entries, at, None, None).and_then(|c| (c as usize).checked_sub(1))
            && let Some(o) = config::XtreeOpen::ALL.get(i)
        {
            self.set_xtree_open(*o);
        }
    }

    /// Modo portable: activarlo (pregunta, copia los archivos y crea la marca, en un hilo) o
    /// desactivarlo (borra la marca). Se aplica al volver a abrir xdir.
    fn toggle_portable(&mut self) {
        if !sys::env().portable_supported() {
            return;
        }
        let (Some(exe), Some(from)) = (session::exe_dir(), session::config_dir()) else { return };
        let on = !session::portable_active();
        if on {
            let question = format!("¿Activar el modo portable? Se copian los archivos de configuración a {} y se usan los de ahí.", exe.display());
            if !self.host.ask_yes_no("Modo portable", &question) {
                return;
            }
        }
        self.spawn_task(move || tasks::TaskDone::Portable { on, result: if on { session::portable_files(&from, &exe) } else { session::portable_off(&exe) } });
    }

    /// Terminó de activar o desactivar el modo portable.
    pub(super) fn on_portable_done(&mut self, on: bool, result: Result<(), String>) {
        self.portable_msg = Some(match result {
            Ok(()) if on => "Activado: se aplica al volver a abrir xdir".to_string(),
            Ok(()) => "Desactivado: se aplica al volver a abrir xdir".to_string(),
            Err(e) => e,
        });
        self.redraw();
    }

    /// El desplegable de la vista por defecto.
    fn default_view_menu(&mut self, pane: usize) {
        let current = self.folder_views.default_view().view;
        let entries: Vec<shell::Entry> = list::VIEWS
            .iter()
            .enumerate()
            .map(|(i, v)| shell::Entry::Item { id: i as u32 + 1, label: v.1.to_string(), enabled: true, checked: v.0 == current })
            .collect();
        let at = self.part_rect(pane, "vista_defecto", 0).map_or(MenuPos::Cursor, |r| MenuPos::At(r.left, r.bottom + 2.0));
        if let Some(i) = self.host.popup_menu(&entries, at, None, None).and_then(|c| (c as usize).checked_sub(1))
            && let Some(v) = list::VIEWS.get(i)
        {
            self.set_default_view(v.0);
        }
    }

    /// «Usar la vista de "<carpeta>" en todas» con la pestaña `id` (como «Usar esta vista en todas
    /// las carpetas», con la misma pregunta).
    fn apply_view_of_tab_to_all(&mut self, id: usize) {
        let Some(tab) = self.panes.iter().flat_map(|p| p.tabs.iter()).find(|t| t.id == id) else { return };
        let mut v = tab.list.view_settings();
        let question = "¿Usar esta vista en todas las carpetas? Se pierden las vistas guardadas de cada carpeta.";
        if self.host.ask_yes_no_cancel("Usar esta vista en todas las carpetas", question) != Some(true) {
            return;
        }
        v.columns.retain(|c| c.col != list::Column::Folder);
        self.folder_views.apply_to_all(v);
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            tab.reload_view();
        }
        self.sync_folder_views();
        self.schedule_views_save();
    }

    /// Dibuja la pestaña Configuración del panel en el lugar de su lista (después de `Pane::draw`).
    pub(super) fn draw_settings(&mut self, p: &dyn Painter, pane: usize) {
        let r = self.panes[pane].list;
        let hover = match self.hover {
            Hit::Settings(j, h) if j == pane => Some(h),
            _ => None,
        };
        let nav: Vec<NavItem> = self.settings_nav().into_iter().map(|page| NavItem { page, title: Self::page_title(page).to_string(), count: None }).collect();
        let Some(view) = self.settings_view_mut(pane) else { return };
        let page = view.page;
        let title = Self::page_title(page);
        let content = crate::settings::draw::frame(p, r, &nav, Some(page), title, hover, view);
        match page {
            PageId::Builtin(section) => {
                let items = self.settings_items(section);
                if let Some(view) = self.settings_view_mut(pane) {
                    crate::settings::draw::items(p, content, &items, hover, view);
                }
            }
            PageId::Extra(id) => {
                let Some(extra) = PAGES.iter().find(|e| e.id == id) else { return };
                let scroll = self.settings_view(pane).map_or(0.0, |v| v.scroll);
                let (mx, my) = self.cursor_pos();
                let mouse = geom::contains(&content, mx, my).then_some((mx, my));
                let shifted = geom::rect(content.left, content.top - scroll, content.right, content.bottom);
                let mut h = 0.0;
                p.clip(content, &mut || h = (extra.draw)(self, p, shifted, mouse));
                if let Some(view) = self.settings_view_mut(pane) {
                    view.content_h = h;
                    view.view_h = content.bottom - content.top;
                    view.hits.push((content, SettingsHit::Extra));
                }
            }
        }
    }

    /// Lo que hay bajo `(x, y)` en la pestaña Configuración del panel (según el último cuadro).
    pub(super) fn settings_hit(&self, pane: usize, x: f32, y: f32) -> Option<SettingsHit> {
        let v = self.settings_view(pane)?;
        Some(v.hit_at(x, y).unwrap_or(SettingsHit::Body))
    }

    /// Un clic en la pestaña Configuración.
    pub(super) fn settings_click(&mut self, pane: usize, hit: SettingsHit, x: f32, y: f32) {
        match hit {
            SettingsHit::Nav(page) => {
                // Con una búsqueda, elegir una sección la termina y va a esa sección.
                if let Some(v) = self.settings_view_mut(pane) {
                    v.set_page(page);
                }
                if !self.panes[pane].tab().search_query().is_empty() {
                    self.clear_filter(pane);
                }
            }
            SettingsHit::Extra => {
                if let Some(PageId::Extra(id)) = self.settings_view(pane).map(|v| v.page)
                    && let Some(extra) = PAGES.iter().find(|e| e.id == id)
                {
                    (extra.click)(self, x, y);
                }
            }
            SettingsHit::Part(id, part) => self.settings_part(pane, id, part, x),
            SettingsHit::Body => {}
        }
        self.redraw();
    }


    /// La rueda sobre la pestaña Configuración: mueve el contenido.
    pub(super) fn settings_wheel(&mut self, pane: usize, delta: f32) {
        if let Some(v) = self.settings_view_mut(pane)
            && v.scroll_by(-delta * 20.0)
        {
            self.redraw();
        }
    }

    /// Terminó un campo (`WM_APP_FIELD_DONE`): si es de una página extra, lo atiende ella.
    pub(super) fn settings_field_done(&mut self, commit: bool) -> bool {
        let Some(b) = self.settings_field else { return false };
        PAGES.iter().any(|p| (p.field_done)(self, b, commit))
    }

    /// La página de la pestaña Configuración para la sesión.
    pub(super) fn settings_session_page(&self) -> Option<String> {
        let (pane, index) = self.settings_tab()?;
        let page = self.panes[pane].tabs[index].settings.as_ref()?.page;
        Some(Self::page_id(page).to_string())
    }
}

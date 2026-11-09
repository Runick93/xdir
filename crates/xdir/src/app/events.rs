//! Eventos de la ventana ya traducidos por la plataforma (coordenadas en DIPs del área de la
//! ventana, teclas con la numeración de `platform::Key`). Cada sistema traduce sus mensajes
//! (`messages.rs` en Windows, la vista de AppKit en macOS) y llama a estos métodos.

use super::*;

/// Cursor del mouse que corresponde a un punto.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CursorKind {
    Arrow,
    /// Redimensionar a lo ancho (separador vertical, borde de columna o del árbol).
    SizeWE,
    /// Redimensionar a lo alto (separador horizontal).
    SizeNS,
}

impl App {
    /// Mensajes de la aplicación (`WM_APP + n`) que mandan los hilos de trabajo y los controles.
    /// Devuelve false si no es uno de ellos.
    pub(super) fn on_app_message(&mut self, msg: u32, wparam: usize, lparam: isize) -> bool {
        match msg {
            WM_APP_LOAD => {
                let t0 = Instant::now();
                let mut changed = false;
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                while let Ok(event) = self.rx.try_recv() {
                    if let Some(tab) = self.tab_by_id(event.panel) {
                        changed |= tab.on_load(event);
                        tab.retry_with_credentials(hwnd, &tx);
                        if let Some(path) = tab.visited.take() {
                            self.history.record(&path, jump::now());
                        }
                    }
                }
                self.ui_load_ms += ms_since(t0);
                if changed {
                    self.try_pending_rename();
                    self.try_new_watch();
                    self.redraw();
                }
            }
            icons::WM_APP_ICONS => self.redraw(),
            WM_APP_CHANGED => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                if let Some(tab) = self.tab_by_id(wparam) {
                    tab.on_changed(hwnd, &tx, lparam as u64);
                }
            }
            WM_APP_EDIT_DONE => self.end_edit(wparam == 1),
            WM_APP_FAVEDIT_DONE => self.end_fav_edit(wparam == 1),
            WM_APP_FILTER_DONE => self.end_filter(wparam == 1),
            WM_APP_RENAME_DONE => self.end_rename(wparam == 1),
            WM_APP_SUGGEST_KEY => self.on_suggest_key(Key(wparam as u16)),
            WM_APP_PALETTE_KEY => self.on_palette_key(Key(wparam as u16), lparam == 1),
            WM_APP_TEXT_CHANGED => self.on_text_changed(lparam),
            WM_APP_SEARCH => self.on_search_due(lparam as u64),
            #[cfg(feature = "arbol")]
            tree::WM_APP_TREE => {
                self.tree.on_loaded();
                self.redraw();
            }
            // Terminó de leerse el Acceso rápido (wparam 1), o el vigilante avisó que cambió
            // (wparam 0): se relee.
            #[cfg(feature = "arbol")]
            tree::WM_APP_QUICK => {
                if wparam == 1 {
                    if self.tree.on_quick_loaded() {
                        self.tree.request_quick(self.hwnd);
                    }
                } else {
                    self.tree.request_quick(self.hwnd);
                }
                self.redraw();
            }
            crate::xtree::scan::WM_APP_XTREE => {
                if self.xtree.as_mut().is_some_and(|v| v.on_message(wparam, lparam)) && self.xtree_open {
                    self.redraw();
                }
            }
            WM_APP_OP_DONE => {
                while let Ok(result) = self.ops_rx.try_recv() {
                    self.on_op_done(result);
                }
                self.redraw();
            }
            _ => return false,
        }
        true
    }

    /// Cambió el texto de una caja de edición: el filtro y la paleta se actualizan con cada tecla,
    /// y la barra de dirección actualiza el desplegable de rutas.
    pub(super) fn on_text_changed(&mut self, id: isize) {
        if let Some((edit, pane)) = self.filter_edit {
            if edit.0 as isize == id {
                let text = self.host.text_get(edit);
                self.search_typed(pane, &text);
            }
        }
        if self.palette.as_ref().is_some_and(|(_, e)| e.0 as isize == id) {
            self.refresh_palette();
        }
        if let Some((edit, _)) = self.edit {
            if edit.0 as isize == id {
                let text = self.host.text_get(edit);
                if let Some(sg) = self.suggest.as_mut().filter(|sg| !sg.quiet) {
                    sg.set_query(&text);
                    self.redraw();
                }
            }
        }
    }

    /// Cambió el tamaño (o la escala) del área de la ventana: los controles flotantes se cierran.
    pub(super) fn on_resized(&mut self) {
        self.end_edit(false);
        self.end_filter(true);
        self.close_palette();
    }

    /// Cambió el modo claro/oscuro del sistema.
    pub(super) fn on_system_theme_changed(&mut self) {
        self.theme = Theme::from_system(self.accent, self.dark_mode);
        self.apply_window_theme();
        self.redraw();
    }

    /// Qué cursor mostrar sobre un punto.
    pub(super) fn cursor_at(&self, x: f32, y: f32) -> CursorKind {
        let splitter = match self.drag {
            Drag::Splitter(s) => Some(s),
            _ => match self.hit(x, y) {
                Hit::Splitter(s) => Some(s),
                _ => None,
            },
        };
        // Borde de un encabezado de columna: cambiar el ancho.
        let column_edge = match (self.drag, self.hit(x, y)) {
            (Drag::List(i), _) | (_, Hit::Pane(i, PaneHit::List)) => self.panes[i].tab().list.wants_resize_cursor(x, y),
            _ => false,
        };
        #[cfg(feature = "arbol")]
        let tree_edge = !self.xtree_open && (matches!(self.drag, Drag::TreeEdge) || (matches!(self.drag, Drag::None) && self.tree.edge_at(x, y)));
        #[cfg(not(feature = "arbol"))]
        let tree_edge = false;
        match splitter {
            Some(Splitter::Vertical) => CursorKind::SizeWE,
            Some(Splitter::Horizontal) => CursorKind::SizeNS,
            None if column_edge || tree_edge => CursorKind::SizeWE,
            None => CursorKind::Arrow,
        }
    }

    /// Botón derecho apretado: prepara el menú contextual que llega al soltarlo.
    pub(super) fn on_right_down(&mut self, x: f32, y: f32) {
        self.host.focus_main();
        self.fav_ctx = None;
        self.header_ctx = None;
        self.term_ctx = None;
        // Clic derecho en la vista de xtree: sube a la carpeta superior (no hay menú).
        if self.xtree_open {
            if self.hit(x, y) == Hit::XtreeView {
                let action = self.xtree.as_mut().map_or(xtree_view::Action::None, |v| v.on_right_click());
                self.xtree_action(action);
            }
            return;
        }
        #[cfg(feature = "arbol")]
        {
            self.tree_ctx = self.tree.contains(x, y).then(|| self.tree.path_at(x, y));
        }
        match self.hit(x, y) {
            Hit::Pane(i, PaneHit::List) if self.panes[i].tab().list.in_header(x, y) => {
                self.set_active(i);
                self.header_ctx = Some(i);
            }
            Hit::Pane(i, PaneHit::List) => {
                self.set_active(i);
                self.panes[i].tab_mut().list.on_right_down(x, y);
                self.redraw();
            }
            Hit::TermMenu(Some(c)) => {
                if let Some(pane) = self.term_menu {
                    self.term_ctx = Some((pane, c));
                }
            }
            Hit::Pane(i, PaneHit::Fav(j)) => self.fav_ctx = Some((i, Some(vec![j]))),
            Hit::Pane(i, PaneHit::FavBar | PaneHit::FavOverflow) => self.fav_ctx = Some((i, None)),
            _ => {}
        }
    }

    /// Pedido de menú contextual: `at` es el punto del clic derecho (None: tecla de menú o
    /// Shift+F10, se abre sobre la fila con foco).
    pub(super) fn on_context_menu_request(&mut self, at: Option<(f32, f32)>) {
        // Con xtree abierto no hay menú contextual: los paneles están tapados.
        if self.xtree_open {
            return;
        }
        if let Some(pane) = self.header_ctx.take().filter(|_| at.is_some()) {
            self.show_columns_menu(pane, at.unwrap());
            return;
        }
        if let Some((pane, console)) = self.term_ctx.take().filter(|_| at.is_some()) {
            self.console_context_menu(pane, console);
            return;
        }
        #[cfg(feature = "arbol")]
        if let Some(target) = self.tree_ctx.take().filter(|_| at.is_some()) {
            if let Some(path) = target {
                self.tree_context_menu(path, at.unwrap());
            }
            return;
        }
        match self.fav_ctx.take() {
            Some((pane, path)) if at.is_some() => {
                if let Some(act) = self.fav_context_menu(path.as_ref(), false) {
                    self.fav_action(pane, path, act);
                }
            }
            _ => self.on_context_menu(at),
        }
    }

    /// Tecla con Alt (Windows) u Opción (macOS): solo los atajos que la usan; el resto sigue de
    /// largo (Alt+F4, Alt+Espacio). Devuelve si se usó.
    pub(super) fn on_alt_key(&mut self, vk: Key) -> bool {
        match self.config.keymap.lookup(vk, key_down(VK_CONTROL), key_down(VK_SHIFT), true) {
            Some(c) => {
                self.run_command(c);
                true
            }
            None => false,
        }
    }

    /// Un carácter escrito sobre la lista: búsqueda por las primeras letras.
    pub(super) fn on_char(&mut self, c: char) {
        if !c.is_control() && !key_down(VK_CONTROL) && !self.xtree_open {
            let action = self.tab_mut().list.on_char(c);
            self.apply(self.active, action);
        }
    }

    /// El mouse salió de la ventana.
    pub(super) fn on_mouse_left(&mut self) {
        self.hover = Hit::None;
        #[cfg(feature = "arbol")]
        if matches!(self.drag, Drag::None) {
            self.tree.on_mouse_leave();
        }
        for p in &mut self.panes {
            p.tab_mut().list.on_mouse_leave();
        }
        self.redraw();
    }

    /// Rueda del mouse (o desplazamiento del touchpad) en `x, y`. `delta` en "clics" (positivo:
    /// hacia arriba); `lines` es cuántas filas mueve cada clic. Va al panel bajo el mouse, aunque
    /// no sea el activo.
    pub(super) fn on_wheel(&mut self, x: f32, y: f32, delta: f32, lines: f32) {
        // Con xtree abierto la rueda mueve su tabla; los paneles tapados no se mueven.
        if self.xtree_open {
            if self.hit(x, y) == Hit::XtreeView {
                let area = self.xtree_area();
                if self.xtree.as_mut().is_some_and(|v| v.on_wheel(area, delta * lines)) {
                    self.redraw();
                }
            }
            return;
        }
        let pane = match self.hit(x, y) {
            Hit::Palette(_) => {
                if let Some((pal, _)) = &mut self.palette {
                    pal.on_wheel(delta.round() as i32 * 3);
                    self.redraw();
                }
                return;
            }
            Hit::Suggest(_) => {
                if let Some(sg) = &mut self.suggest {
                    sg.on_wheel(delta.round() as i32 * 3);
                    self.redraw();
                }
                return;
            }
            Hit::Pane(i, _) => i,
            _ => self.active,
        };
        #[cfg(feature = "arbol")]
        if self.tree.contains(x, y) {
            if key_down(VK_SHIFT) {
                self.tree.on_wheel_h(delta * lines);
            } else {
                self.tree.on_wheel(delta * lines);
            }
            self.redraw();
            return;
        }
        // Ctrl+rueda: tamaño de la vista, como en el Explorador.
        if key_down(VK_CONTROL) {
            self.cycle_view(pane, delta > 0.0);
            return;
        }
        let action = self.panes[pane].tab_mut().list.on_wheel(delta * lines);
        self.apply(pane, action);
    }

    /// Rueda horizontal (touchpad, mouse con inclinación): el árbol se mueve a los costados.
    /// `delta` positivo: hacia la derecha.
    pub(super) fn on_wheel_h(&mut self, x: f32, y: f32, delta: f32, lines: f32) {
        #[cfg(feature = "arbol")]
        if !self.xtree_open && self.tree.contains(x, y) {
            self.tree.on_wheel_h(-delta * lines);
            self.redraw();
        }
        let _ = (x, y, delta, lines);
    }

    /// Botones laterales del mouse (o deslizar con dos dedos en macOS): atrás y adelante en el
    /// panel bajo el mouse.
    pub(super) fn on_back_forward(&mut self, x: f32, y: f32, forward: bool) {
        if let Hit::Pane(i, _) = self.hit(x, y) {
            self.set_active(i);
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        if forward {
            self.tab_mut().go_forward(hwnd, &tx);
        } else {
            self.tab_mut().go_back(hwnd, &tx);
        }
        self.redraw();
    }

    /// La ventana volvió a primer plano: se releen los favoritos y la configuración (por si se
    /// editaron los archivos) y el lector de pantalla vuelve a leer el elemento con foco.
    pub(super) fn on_app_activated(&mut self) {
        self.a11y_focus = None;
        #[cfg(feature = "arbol")]
        self.tree.request_quick(self.hwnd);
        let fresh = Favorites::load();
        if fresh != self.favorites {
            self.favorites = fresh;
            self.redraw();
        }
        if self.probe.is_none() {
            self.reload_config();
        }
    }

    /// La ventana se va a cerrar: se guardan la sesión y el historial.
    pub(super) fn on_closing(&mut self) {
        // El escaneo de xtree (abierto o en segundo plano) no sigue después de cerrar xdir.
        if let Some(v) = &self.xtree {
            v.cancel();
        }
        if self.probe.is_none() {
            self.autosave_group();
            let _ = session::save(&self.session());
            let _ = self.history.save();
        }
    }
}

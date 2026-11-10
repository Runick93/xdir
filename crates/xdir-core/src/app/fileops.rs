//! Operaciones de archivos: portapapeles, borrar, arrastrar y soltar, menú contextual.

use super::*;
use crate::archive::{ArchiveFormat, extract_folder_name, format_of};
use crate::transfer::JobSpec;

/// Las entradas «Extraer aquí» y «Extraer en "nombre"» para un elemento (con el nombre real de la
/// carpeta que se crearía), si es un archivo de un formato que este sistema sabe extraer.
pub fn archive_menu(name: &str, is_dir: bool, formats: &[ArchiveFormat]) -> Vec<(String, Command)> {
    if is_dir || !format_of(name).is_some_and(|f| formats.contains(&f)) {
        return Vec::new();
    }
    vec![
        ("Extraer aquí".to_string(), Command::ExtractHere),
        (format!("Extraer en \"{}\"", extract_folder_name(name)), Command::ExtractInto),
    ]
}

impl App {
    // ---------- operaciones de archivos ----------

    /// Rutas seleccionadas en la pestaña activa de un panel.
    pub(super) fn selection(&self, pane: usize) -> Vec<PathBuf> {
        let tab = self.panes[pane].tab();
        tab.list.selected_indices().map(|i| tab.entry_path(i)).collect()
    }

    /// Selección como elementos del Shell (con PIDL si son virtuales).
    pub(super) fn selection_items(&self, pane: usize) -> Vec<shell::Item> {
        let tab = self.panes[pane].tab();
        tab.list.selected_indices().map(|i| tab.entry_item(i)).collect()
    }

    /// Arrastra la selección hacia otras ventanas (o dentro de la misma). Si el sistema no puede
    /// arrastrar, se avisa en la barra de estado.
    pub(super) fn start_drag(&mut self, pane: usize) {
        let items = self.selection_items(pane);
        if items.is_empty() {
            return;
        }
        // En Windows el arrastre es un bucle modal: mientras dura llegan los eventos de soltar.
        self.drag = Drag::None;
        let host = self.host.clone();
        if let Err(e) = host.start_drag(&items) {
            self.notice = Some(format!("No se pudo arrastrar: {e}"));
        }
        self.redraw();
    }

    pub(super) fn copy_selection(&mut self, cut: bool) {
        let items = self.selection_items(self.active);
        if !items.is_empty() {
            if let Err(e) = sys::shell().copy_files(&items, cut) {
                self.notice = Some(format!("No se pudo copiar: {e}"));
            }
        }
    }

    pub(super) fn paste(&mut self) {
        let Some(dest) = self.panes[self.active].tab().location.target() else { return };
        let Some((sources, cut)) = sys::shell().clipboard_files() else {
            // Sin rutas de disco (copiado por RDP, desde un zip...): pega la carpeta destino.
            match sys::shell().paste_virtual(self.hwnd, &self.panes[self.active].tab().location.clone()) {
                Ok(true) => self.reload_shell_tabs(),
                Ok(false) => {}
                Err(e) => self.notice = Some(format!("No se pudo pegar: {e}")),
            }
            return;
        };
        // Copiar y mover entre rutas de disco van por el motor (con la ventana de copia).
        self.start_transfer(if cut { JobSpec::Move { sources, dest } } else { JobSpec::Copy { sources, dest } });
        // Lo cortado se pega una sola vez, como en el Explorador.
        if cut {
            sys::shell().clear_clipboard();
        }
    }

    /// ¿El panel activo muestra la Papelera?
    pub(super) fn in_trash(&self) -> bool {
        sys::file_system().is_trash(&self.panes[self.active].tab().location)
    }

    /// Duplicar: una copia de cada seleccionado a su lado (solo carpetas de disco, no la Papelera).
    pub(super) fn duplicate_selection(&mut self) {
        if !matches!(self.panes[self.active].tab().location, Location::Path(_)) || self.in_trash() {
            return;
        }
        let sources = self.selection(self.active);
        if !sources.is_empty() {
            self.start_transfer(JobSpec::Duplicate { sources });
        }
    }

    /// El único seleccionado, si es un archivo que se sabe extraer: (ruta, nombre).
    fn selected_archive(&self) -> Option<(PathBuf, String)> {
        let tab = self.panes[self.active].tab();
        if !matches!(tab.location, Location::Path(_)) || self.in_trash() {
            return None;
        }
        let mut sel = tab.list.selected_indices();
        let (Some(i), None) = (sel.next(), sel.next()) else { return None };
        let e = &tab.list.entries[i];
        let name = e.name.to_string_lossy().into_owned();
        (!archive_menu(&name, e.is_dir, sys::archives().formats()).is_empty()).then(|| (tab.entry_path(i), name))
    }

    /// Comprimir: un zip con la selección en su carpeta (solo carpetas de disco, no la Papelera).
    pub(super) fn compress_selection(&mut self) {
        if !matches!(self.panes[self.active].tab().location, Location::Path(_)) || self.in_trash() {
            return;
        }
        let sources = self.selection(self.active);
        if !sources.is_empty() {
            self.start_transfer(JobSpec::Compress { sources });
        }
    }

    /// Extraer el archivo seleccionado: en su carpeta o en una nueva con su nombre.
    pub(super) fn extract_selection(&mut self, into_folder: bool) {
        if let Some((archive, _)) = self.selected_archive() {
            self.start_transfer(JobSpec::Extract { archive, into_folder });
        }
    }

    /// Carpeta nueva con la selección: en la carpeta del panel (en una búsqueda, la carpeta
    /// buscada), solo en carpetas de disco.
    pub(super) fn new_folder_with_selection(&mut self) {
        let location = &self.panes[self.active].tab().location;
        let (Location::Path(_), Some(parent)) = (location, location.target()) else { return };
        if self.in_trash() {
            return;
        }
        let items = self.selection(self.active);
        if !items.is_empty() {
            self.start_transfer(JobSpec::NewFolderWith { parent, items });
        }
    }

    /// Devolver: cada seleccionado de la Papelera vuelve a su lugar. En la Papelera virtual de
    /// Windows lo hace el Shell (verbo `undelete`); en las demás, el motor de copia
    /// (`JobSpec::Restore`, con el lugar original que lee cada sistema y su carpeta preparada sin
    /// seguir enlaces).
    pub(super) fn restore_selection(&mut self) {
        let location = self.panes[self.active].tab().location.clone();
        if !sys::file_system().is_trash(&location) {
            return;
        }
        if let Location::Shell { .. } = location {
            let items = self.selection_items(self.active);
            if !items.is_empty() {
                if let Err(e) = sys::shell().invoke_verb(self.hwnd, &location, &items, "undelete") {
                    self.notice = Some(format!("No se pudo devolver: {e}"));
                }
                self.reload_shell_tabs();
            }
            return;
        }
        let items = self.selection(self.active);
        if !items.is_empty() {
            self.start_transfer(JobSpec::Restore { items });
        }
    }

    pub(super) fn delete_selection(&mut self, recycle: bool) {
        // En carpetas virtuales borra el propio Shell (en la Papelera: eliminar definitivamente).
        let location = self.panes[self.active].tab().location.clone();
        if let Location::Shell { .. } = location {
            let items = self.selection_items(self.active);
            if !items.is_empty() {
                let _ = sys::shell().invoke_verb(self.hwnd, &location, &items, "delete");
                self.reload_shell_tabs();
            }
            return;
        }
        let paths = self.selection(self.active);
        if !paths.is_empty() && self.panes[self.active].tab().location.target().is_some() {
            sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), Op::Delete { paths, recycle });
        }
    }

    pub(super) fn on_op_done(&mut self, result: OpResult) {
        self.notice = result.error.map(|e| format!("Error: {e}"));
        if let Some(undo) = result.undo {
            self.undo.push(undo);
            if self.undo.len() > UNDO_MAX {
                self.undo.remove(0);
            }
        }
        self.reload_shell_tabs();
        // Lo que tiene que quedar seleccionado (las copias de Duplicar), por carpeta.
        let mut by_folder: Vec<(PathBuf, Vec<std::ffi::OsString>)> = Vec::new();
        for p in &result.select {
            let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else { continue };
            match by_folder.iter_mut().find(|(f, _)| f == parent) {
                Some((_, names)) => names.push(name.to_os_string()),
                None => by_folder.push((parent.to_path_buf(), vec![name.to_os_string()])),
            }
        }
        if !by_folder.is_empty() {
            let (hwnd, tx) = (self.hwnd, self.tx.clone());
            for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
                let Some((_, names)) = by_folder.iter().find(|(f, _)| tab.location.is_path(f)) else { continue };
                if tab.is_loaded() {
                    tab.select_after_load(names.clone());
                    tab.reload(hwnd, &tx, true);
                }
            }
        }
        let Some(path) = result.focus else { return };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        // Las pestañas que muestran esa carpeta enfocan el elemento al recargar.
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if tab.location.is_path(parent) && tab.is_loaded() {
                tab.focus_after_load(name.to_os_string());
                tab.reload(hwnd, &tx, true);
            }
        }
        if result.rename_after {
            self.pending_rename = Some(path);
        }
    }

    /// Las carpetas virtuales no tienen vigilancia: se recargan después de cada operación.
    pub(super) fn reload_shell_tabs(&mut self) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let visible = self.layout.panes();
        for pane in &mut self.panes[..visible] {
            let tab = pane.tab_mut();
            if matches!(tab.location, Location::Shell { .. }) && !tab.loading {
                tab.reload(hwnd, &tx, true);
            }
        }
    }

    /// Tras elegir algo del menú del fondo: si apareció un elemento nuevo en la carpeta, lo selecciona
    /// y abre su nombre para escribirlo ("Nuevo > Carpeta" lo crea el Shell con el nombre por defecto).
    pub(super) fn try_new_watch(&mut self) {
        let Some(w) = &self.new_watch else { return };
        if w.since.elapsed().as_secs() > 5 {
            self.new_watch = None;
            return;
        }
        let pane = w.pane;
        let tab = self.panes[pane].tab_mut();
        if tab.loading {
            return;
        }
        let Some(idx) = tab.list.entries.iter().position(|e| !w.before.contains(&e.display.to_lowercase())) else { return };
        tab.list.select_index(idx);
        self.new_watch = None;
        self.begin_rename(pane);
    }

    /// Abre el renombrado sobre la carpeta recién creada cuando ya aparece en la lista.
    pub(super) fn try_pending_rename(&mut self) {
        let Some(path) = self.pending_rename.clone() else { return };
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return };
        let pane = self.active;
        let tab = self.panes[pane].tab_mut();
        if !tab.location.is_path(parent) || tab.loading {
            return;
        }
        if let Some(idx) = tab.list.entries.iter().position(|e| e.name.eq_ignore_ascii_case(name)) {
            tab.list.select_index(idx);
            self.pending_rename = None;
            self.begin_rename(pane);
        }
    }

    /// Carpeta destino bajo un punto: una carpeta de la lista o la carpeta del panel.
    pub(super) fn drop_folder(&self, x: f32, y: f32) -> Option<(usize, Option<usize>, PathBuf)> {
        let Hit::Pane(i, ph) = self.hit(x, y) else { return None };
        let tab = self.panes[i].tab();
        if ph == PaneHit::List {
            if let Some(idx) = tab.list.entry_at(x, y) {
                if tab.list.entries[idx].is_dir {
                    return Some((i, Some(idx), tab.entry_path(idx)));
                }
            }
        }
        tab.location.target().map(|t| (i, None, t))
    }

    pub fn on_drag(&mut self, ev: &mut DragEvent) {
        let (x, y) = ev.point;
        if self.on_drag_favbar(ev, x, y) {
            return;
        }
        let target = match ev.kind {
            DragKind::Leave => None,
            _ => self.drop_folder(x, y),
        };
        ev.effect = match (&target, ev.paths) {
            (Some((_, _, dest)), Some(paths)) => {
                let roots: Vec<PathBuf> = self.net.connections.iter().map(|c| c.local.clone()).collect();
                drop_effect(paths, dest, ev.ctrl, ev.shift, ev.allowed, &roots)
            }
            _ => Effect::NONE,
        };

        // Resaltado de la carpeta destino.
        let mut changed = false;
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let want = match &target {
                Some((p, Some(idx), _)) if *p == i && ev.effect != Effect::NONE && ev.kind != DragKind::Drop => {
                    Some(*idx)
                }
                _ => None,
            };
            let list = &mut pane.tab_mut().list;
            if list.drop_target != want {
                list.drop_target = want;
                changed = true;
            }
        }
        if changed {
            self.redraw();
        }

        if ev.kind == DragKind::Drop && ev.effect != Effect::NONE {
            if let (Some((pane, _, dest)), Some(paths)) = (target, ev.paths) {
                let sources = paths.to_vec();
                self.start_transfer(if ev.effect == Effect::MOVE { JobSpec::Move { sources, dest } } else { JobSpec::Copy { sources, dest } });
                self.set_active(pane);
            }
        }
    }

    /// El submenú «Ver» de la carpeta del panel (lo comparten el menú del fondo y el del encabezado
    /// de columnas); `item` arma cada ítem.
    pub(super) fn views_submenu(&self, pane: usize, item: &mut dyn FnMut(&str, Command, bool, bool) -> shell::Entry) -> shell::Entry {
        use shell::Entry;
        let list = &self.panes[pane].tab().list;
        // En el orden de `CYCLE` (de lo más compacto a lo más grande); el comando lleva el índice
        // de `VIEWS`.
        let mut views: Vec<Entry> =
            list::CYCLE.iter().map(|&view| item(view.name(), Command::View(view.index() as u8), true, view == list.view)).collect();
        // La vista es de cada carpeta: pasarla a todas o volver a la por defecto.
        views.push(Entry::Sep);
        views.push(item("Usar esta vista en todas las carpetas", Command::ApplyViewToAll, true, false));
        views.push(item("Restablecer la vista de esta carpeta", Command::ResetFolderView, true, false));
        Entry::Sub("Ver".into(), views)
    }

    /// Los submenús «Ordenar por» y «Agrupar por» de la carpeta del panel (los comparten el menú
    /// del fondo y el del encabezado de columnas); `item` arma cada ítem.
    pub(super) fn sort_group_submenus(
        &self,
        pane: usize,
        item: &mut dyn FnMut(&str, Command, bool, bool) -> shell::Entry,
    ) -> (shell::Entry, shell::Entry) {
        use shell::Entry;
        let list = &self.panes[pane].tab().list;
        // Las columnas de metadatos, solo si están a la vista (son muchas y casi siempre vacías) o
        // si la lista está ordenada por ella.
        let sorts = list::COLUMNS
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.0.is_meta() || c.0 == list.sort || list.columns.iter().any(|s| s.col == c.0))
            .map(|(i, &(col, title, ..))| item(title, Command::SortBy(i as u8), true, col == list.sort))
            .collect();
        // «Agrupar por»: los criterios con el título de su columna y «(Ninguno)».
        let groupings = list::GROUPINGS
            .iter()
            .enumerate()
            .map(|(i, &col)| item(col.map_or("(Ninguno)", |c| c.title()), Command::GroupBy(i as u8), true, col == list.group_by))
            .collect();
        (Entry::Sub("Ordenar por".into(), sorts), Entry::Sub("Agrupar por".into(), groupings))
    }

    /// Entradas propias del menú del fondo de una carpeta; `cmds` recibe el comando de cada id.
    pub(super) fn background_entries(&self, pane: usize, cmds: &mut Vec<Command>) -> Vec<shell::Entry> {
        use shell::Entry;
        let mut item = |label: &str, command: Command, enabled: bool, checked: bool| {
            cmds.push(command);
            Entry::Item { id: cmds.len() as u32 - 1, label: label.to_string(), enabled, checked }
        };
        let views = self.views_submenu(pane, &mut item);
        let (sorts, groupings) = self.sort_group_submenus(pane, &mut item);
        let undo = item("Deshacer", Command::Undo, !self.undo.is_empty(), false);
        // Sin el árbol no se agrega nada después.
        #[cfg_attr(not(feature = "arbol"), allow(unused_mut))]
        let mut entries = vec![
            views,
            sorts,
            groupings,
            item("Actualizar", Command::Refresh, true, false),
            Entry::Sep,
            item("Pegar", Command::Paste, true, false),
            undo,
        ];
        // Anclar la carpeta que se está viendo al Acceso rápido de Windows (el menú del fondo del
        // Shell no trae ese verbo).
        #[cfg(feature = "arbol")]
        if let Location::Path(p) = &self.panes[pane].tab().location {
            if pane == self.active && p.parent().is_some() {
                let label = if self.tree.is_pinned(p) { "Desanclar de Acceso rápido" } else { "Anclar a Acceso rápido" };
                entries.push(Entry::Sep);
                entries.push(item(label, Command::ToggleQuickPin, true, false));
            }
        }
        entries
    }

    /// Entradas propias del menú de los elementos seleccionados; `cmds` recibe el comando de cada id.
    pub(super) fn item_entries(&self, cmds: &mut Vec<Command>) -> Vec<shell::Entry> {
        let mut item = |label: &str, c: Command| {
            cmds.push(c);
            shell::Entry::Item { id: cmds.len() as u32 - 1, label: label.into(), enabled: true, checked: false }
        };
        // «Vista rápida» primero. «Copiar como ruta de acceso» usa las mismas comillas y saltos de
        // línea que «Copiar las rutas como texto» (`quoted_paths`).
        let many = self.panes[self.active].tab().list.selected_indices().nth(1).is_some();
        let link = self.selected_link().is_some();
        // Solo en una carpeta del disco se crean cosas nuevas (duplicados, accesos, carpetas): ni en
        // la Papelera ni en «Este equipo» ni en carpetas virtuales.
        let creates = matches!(self.panes[self.active].tab().location, Location::Path(_)) && !self.in_trash();
        let mut entries = vec![item("Vista rápida", Command::QuickLook)];
        if creates {
            entries.push(item("Duplicar", Command::Duplicate));
            entries.push(item("Crear acceso directo", Command::CreateShortcut));
        }
        if link {
            entries.push(item("Mostrar el original", Command::ShowOriginal));
        }
        // Con dos o más: juntarlos en una carpeta nueva.
        if many && creates {
            entries.push(item("Carpeta nueva con la selección", Command::NewFolderWithSelection));
        }
        // Comprimir lo elegido; con un solo archivo comprimido, extraerlo (con el nombre real).
        if creates {
            entries.push(item("Comprimir", Command::Compress));
            if let Some((_, name)) = self.selected_archive() {
                for (label, c) in archive_menu(&name, false, sys::archives().formats()) {
                    entries.push(item(&label, c));
                }
            }
        }
        entries.push(item("Copiar como ruta de acceso", Command::CopyPaths));
        // Con varios seleccionados en una carpeta del disco: Renombrar varios.
        let n = self.panes[self.active].tab().list.selected_positions().len();
        if n > 1 && self.can_rename_many(self.active) {
            entries.push(item(&format!("Renombrar {n} elementos…"), Command::RenameMany));
        }
        entries
    }

    /// Menú contextual de la lista en `at` (DIPs del área de la ventana). Sin punto (tecla de
    /// menú o Shift+F10) se abre sobre la fila con foco.
    pub(super) fn on_context_menu(&mut self, at: Option<(f32, f32)>) {
        self.close_filter_menus();
        let pane = self.active;
        let at = at.unwrap_or_else(|| self.panes[pane].tab().list.focus_point().unwrap_or((40.0, 80.0)));
        let location = self.panes[pane].tab().location.clone();
        let items = self.selection_items(pane);
        // En carpetas virtuales el Shell resuelve todo: conoce el contexto (p. ej. la Papelera).
        let intercept = !matches!(location, Location::Shell { .. });
        // El menú del fondo del Shell solo trae las extensiones: Ver, Ordenar, Actualizar, Pegar y
        // Deshacer los pone la vista del Explorador, así que se agregan acá.
        let mut cmds = Vec::new();
        let mut own = Vec::new();
        // En la Papelera, «Devolver» primero (también en la del Shell de Windows).
        if !items.is_empty() && sys::file_system().is_trash(&location) {
            cmds.push(Command::Restore);
            own.push(shell::Entry::Item { id: 0, label: "Devolver".into(), enabled: true, checked: false });
        }
        // Un disco expulsable de «Este equipo»: «Expulsar».
        if matches!(location, Location::Computer) && items.len() == 1 && self.eject_target().is_some() {
            cmds.push(Command::Eject);
            own.push(shell::Entry::Item { id: cmds.len() as u32 - 1, label: "Expulsar".into(), enabled: true, checked: false });
        }
        // El fondo de «Este equipo»: conectar a un servidor (y las unidades de red de Windows).
        if matches!(location, Location::Computer) && items.is_empty() {
            for (c, label) in [
                (Command::ConnectServer, "Conectar a un servidor…"),
                (Command::MapNetworkDrive, "Conectar unidad de red…"),
                (Command::DisconnectNetworkDrive, "Desconectar unidad de red…"),
            ] {
                if crate::commands::available(c) {
                    cmds.push(c);
                    own.push(shell::Entry::Item { id: cmds.len() as u32 - 1, label: label.into(), enabled: true, checked: false });
                }
            }
        }
        own.extend(match (items.is_empty(), intercept) {
            (true, true) => self.background_entries(pane, &mut cmds),
            (false, true) => self.item_entries(&mut cmds),
            _ => Vec::new(),
        });
        let before: Option<std::collections::HashSet<String>> = (items.is_empty() && matches!(location, Location::Path(_)))
            .then(|| self.panes[pane].tab().list.entries.iter().map(|e| e.display.to_lowercase()).collect());
        // Una sola carpeta anclada: su menú dice "Desanclar de Acceso rápido".
        #[cfg(feature = "arbol")]
        let pinned_one = match items.as_slice() {
            [one] if one.pidl.is_none() && crate::fs::probe_dir(&one.path) == Some(true) => self.tree.is_pinned(&one.path).then(|| one.path.clone()),
            _ => None,
        };
        #[cfg(not(feature = "arbol"))]
        let pinned_one: Option<PathBuf> = None;
        let chosen = sys::shell().context_menu(self.hwnd, &location, &items, at, key_down(VK_SHIFT), intercept, &own, pinned_one.is_some());
        #[cfg(feature = "arbol")]
        if let (Some(shell::UNPIN_QUICK), Some(path)) = (chosen.as_deref(), pinned_one) {
            self.toggle_quick_pin(path);
            self.redraw();
            return;
        }
        // El menú del Shell puede haber anclado o desanclado una carpeta del Acceso rápido.
        #[cfg(feature = "arbol")]
        if chosen.as_deref() == Some("shell") {
            self.tree.request_quick(self.hwnd);
        }
        if let (Some("shell"), Some(before)) = (chosen.as_deref(), before) {
            self.new_watch = Some(NewWatch { pane, before, since: Instant::now() });
        }
        if let Some(cmd) = chosen.as_deref().and_then(|c| c.strip_prefix("own:")).and_then(|n| n.parse::<usize>().ok()) {
            if let Some(&command) = cmds.get(cmd) {
                self.run_command(command);
            }
            self.redraw();
            return;
        }
        match chosen.as_deref() {
            Some("rename") => self.begin_rename(pane),
            Some("delete") => self.delete_selection(!key_down(VK_SHIFT)),
            Some("cut") => self.copy_selection(true),
            Some("copy") => self.copy_selection(false),
            Some("paste") => self.paste(),
            // «Propiedades» / «Obtener información» del menú: el panel de información.
            Some("properties") => self.open_info(),
            _ => {}
        }
        self.redraw();
    }
}

/// Rutas entre comillas dobles, una por línea, como «Copiar como ruta de acceso» del Explorador
/// (y «Copiar las rutas como texto», que usa lo mismo).
pub fn quoted_paths(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join("\r\n")
}

/// Efecto al soltar como en el Explorador: Ctrl copia, Shift mueve; sin teclas, mueve dentro
/// de la misma unidad y copia entre unidades. Soltar en la misma carpeta (o dentro de sí
/// misma) no hace nada.
/// El volumen de una ruta para decidir mover o copiar. Una ruta de red no se pregunta al sistema
/// (iría al servidor con cada movimiento del mouse): su volumen es la conexión que la contiene
/// (`net_roots`) o, en UNC, `\\servidor\recurso`.
fn volume_id(p: &std::path::Path, net_roots: &[PathBuf]) -> Option<std::ffi::OsString> {
    if !sys::network().is_remote(p) {
        return sys::shell().volume_of(p);
    }
    let root = net_roots.iter().filter(|r| !r.as_os_str().is_empty() && p.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned();
    let root = root.unwrap_or_else(|| p.components().take(if crate::fs::is_unc(p) { 1 } else { 3 }).collect());
    Some(std::ffi::OsString::from(format!("red:{}", root.to_string_lossy().to_lowercase())))
}

pub(super) fn drop_effect(paths: &[PathBuf], dest: &std::path::Path, ctrl: bool, shift: bool, allowed: Effect, net_roots: &[PathBuf]) -> Effect {
    if paths.iter().any(|p| p.parent() == Some(dest) || dest.starts_with(p)) {
        return Effect::NONE;
    }
    let same_volume = paths.first().and_then(|p| volume_id(p, net_roots)) == volume_id(dest, net_roots);
    let want = if ctrl {
        Effect::COPY
    } else if shift || same_volume {
        Effect::MOVE
    } else {
        Effect::COPY
    };
    if allowed.allows(want) {
        want
    } else if allowed.allows(Effect::COPY) {
        Effect::COPY
    } else {
        Effect::NONE
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn effect(sources: &[&str], dest: &str, ctrl: bool, shift: bool, allowed: Effect) -> Effect {
        // `volume_of` pasa por el `Shell` del registro (el falso compara el primer tramo, como Windows).
        crate::sys::testing::install_fakes();
        let paths: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        drop_effect(&paths, std::path::Path::new(dest), ctrl, shift, allowed, &[])
    }

    #[test]
    fn drop_rules() {
        let all = Effect::ALL;
        // Misma unidad: mueve; otra unidad: copia.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", false, false, all), Effect::MOVE);
        assert_eq!(effect(&[r"D:\a\x.txt"], r"C:\b", false, false, all), Effect::COPY);
        // Ctrl copia, Shift mueve.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", true, false, all), Effect::COPY);
        assert_eq!(effect(&[r"D:\a\x.txt"], r"C:\b", false, true, all), Effect::MOVE);
        // A la misma carpeta, o una carpeta dentro de sí misma: nada.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\a", false, false, all), Effect::NONE);
        assert_eq!(effect(&[r"C:\a"], r"C:\a\sub", false, false, all), Effect::NONE);
        // Si el origen solo permite copiar, se copia.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", false, false, Effect::COPY), Effect::COPY);
    }
}

//! Trabajos sueltos en otro hilo (resolver un acceso directo, saber qué discos se pueden expulsar,
//! expulsar, renombrar varios, saber si un disco distingue mayúsculas): el hilo de interfaz nunca
//! espera al disco. Cada trabajo vuelve por el canal `task_tx` con un `TaskDone` y avisa con
//! `WM_APP_TASK`.

use super::*;

/// Terminó un trabajo de `spawn_task`; el resultado está en el canal.
pub const WM_APP_TASK: u32 = crate::WM_APP + 26;

/// Lo que devuelve un trabajo suelto.
pub(super) enum TaskDone {
    /// El original de un acceso directo (`None`: no se resolvió o ya no existe).
    Original { pane: usize, link: PathBuf, target: Option<PathBuf> },
    /// Los discos que se pueden expulsar.
    Ejectable(Vec<PathBuf>, Vec<(PathBuf, std::ffi::OsString)>),
    /// Terminó de expulsar `root` (`label`: su nombre para los avisos).
    Ejected { root: PathBuf, label: String, result: Result<(), crate::shell::EjectError> },
    /// Terminó Renombrar varios en la carpeta `dir`; si era un deshacer, `undoing` trae los pares
    /// para volver a apilarlos si falló.
    Renamed { dir: PathBuf, result: Result<Undo, String>, undoing: Option<Vec<(PathBuf, String)>> },
    /// Si el disco de `dir` distingue mayúsculas (para los choques de Renombrar varios).
    CaseInfo { dir: PathBuf, case_insensitive: bool },
    /// Se leyó `red.toml`.
    NetPlaces(crate::net::NetPlaces),
    /// Lo conectado (`Network::connections`).
    NetConnections(Result<Vec<crate::net::NetConnection>, crate::net::NetError>),
    /// Terminó de conectar `addr` (`tab`: la pestaña que lo pidió al fallar por credenciales;
    /// `None`: abrir el resultado en una pestaña nueva del panel `pane`).
    /// `generation`: la del intento (los cancelados se descartan).
    Connected { pane: usize, tab: Option<usize>, generation: u64, addr: crate::net::Address, result: Result<PathBuf, crate::net::NetError> },
    /// Nombres de unidades de red, pedidos en otro hilo (para el árbol).
    #[cfg_attr(not(feature = "arbol"), allow(dead_code))]
    DriveLabels(Vec<(PathBuf, String)>),
}

impl App {
    /// Corre `f` en un hilo; su resultado vuelve por el canal y se procesa con `WM_APP_TASK`.
    pub(super) fn spawn_task(&self, f: impl FnOnce() -> TaskDone + Send + 'static) {
        let (tx, hwnd) = (self.task_tx.clone(), self.hwnd);
        std::thread::spawn(move || {
            if tx.send(f()).is_ok() {
                hwnd.post(WM_APP_TASK, 0, 0);
            }
        });
    }

    /// `WM_APP_TASK`: procesa lo que llegó.
    pub(super) fn on_tasks(&mut self) {
        while let Ok(t) = self.task_rx.try_recv() {
            self.on_task_done(t);
        }
        self.redraw();
    }

    pub(super) fn on_task_done(&mut self, t: TaskDone) {
        match t {
            TaskDone::Renamed { dir, result, undoing } => self.on_renamed(dir, result, undoing),
            TaskDone::CaseInfo { dir, case_insensitive } => self.on_case_info(&dir, case_insensitive),
            TaskDone::Original { pane, link, target } => {
                let name = link.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match target.as_ref().and_then(|t| Some((t.parent()?, t.file_name()?))) {
                    Some((parent, file)) => {
                        let (hwnd, tx) = (self.hwnd, self.tx.clone());
                        // En el panel desde el que se pidió (aunque ahora el activo sea otro).
                        let pane = pane.min(self.panes.len() - 1);
                        let tab = self.panes[pane].tab_mut();
                        tab.navigate(hwnd, &tx, Location::Path(parent.to_path_buf()));
                        tab.focus_after_load(file.to_os_string());
                    }
                    None => self.notice = Some(format!("No se encuentra el original de \"{name}\"")),
                }
            }
            TaskDone::Ejectable(roots, ids) => {
                self.ejectable = roots.into_iter().collect();
                self.remember_volume_ids(ids);
                #[cfg(feature = "arbol")]
                {
                    self.tree.ejectable = self.ejectable.clone();
                }
            }
            TaskDone::Ejected { root, label, result } => {
                self.eject_done(&root);
                self.on_ejected(root, label, result);
            }
            TaskDone::NetPlaces(places) => self.on_net_places(places),
            TaskDone::NetConnections(result) => self.on_net_connections(result),
            TaskDone::Connected { pane, tab, generation, addr, result } => self.on_connected(pane, tab, generation, addr, result),
            TaskDone::DriveLabels(labels) => self.on_drive_labels(labels),
        }
    }

    /// Terminó de expulsar: el aviso, o la pregunta de reintentar si alguien lo está usando.
    fn on_ejected(&mut self, root: PathBuf, label: String, result: Result<(), crate::shell::EjectError>) {
        match result {
            Ok(()) => {
                self.notice = Some(format!("Se puede quitar \"{label}\" con seguridad"));
                self.ejectable.remove(&root);
            }
            Err(e) => {
                let who = e.busy_by.clone().unwrap_or_else(|| "otro programa".to_string());
                let text = if e.busy || e.busy_by.is_some() {
                    format!("No se pudo expulsar \"{label}\": lo está usando {who}. ¿Reintentar?")
                } else {
                    format!("No se pudo expulsar \"{label}\": {}. ¿Reintentar?", e.message)
                };
                if self.host.ask_yes_no_cancel("Expulsar", &text) == Some(true) {
                    self.eject(root);
                }
            }
        }
    }

    /// Pregunta en otro hilo qué discos se pueden expulsar (al cargar «Este equipo» y al cambiar
    /// las unidades).
    pub(super) fn request_ejectable(&self) {
        let roots = sys::file_system().drives();
        self.spawn_task(move || {
            let roots = sys::shell().ejectable(&roots);
            // La identidad de cada uno (para reconocer sus trabajos por otro camino al expulsar).
            let ids = roots.iter().filter_map(|r| sys::copy_prims().volume(r).map(|v| (r.clone(), v.id))).collect();
            TaskDone::Ejectable(roots, ids)
        });
    }

    /// El disco expulsable del que se habla: el enfocado (o seleccionado) en «Este equipo», o el
    /// de la carpeta del panel activo.
    pub(super) fn eject_target(&self) -> Option<PathBuf> {
        let tab = self.panes[self.active].tab();
        match &tab.location {
            Location::Computer => {
                let idx = tab.list.selected_indices().next().or_else(|| tab.list.focused_index())?;
                Some(tab.entry_path(idx)).filter(|p| self.ejectable.contains(p))
            }
            Location::Path(p) => {
                self.ejectable.iter().filter(|r| p.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned()
            }
            _ => None,
        }
    }

    /// Suelta lo que xdir tiene abierto en el disco de `root`: las pestañas en ese disco pasan a
    /// «Este equipo» (y sueltan su guardián), se cierra xtree si su raíz está ahí y se vacía la
    /// vista previa.
    pub(super) fn release_volume(&mut self, root: &std::path::Path) {
        // La vista rápida de un archivo de ese disco se cierra primero (suelta el archivo). Las
        // miniaturas no guardan nada abierto: con las pestañas fuera del disco no se piden más.
        if self.quick_look_under(root) {
            self.close_quick_look();
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if matches!(&tab.location, Location::Path(p) if p.starts_with(root)) {
                tab.navigate(hwnd, &tx, Location::Computer);
            }
        }
        if self.xtree.as_ref().is_some_and(|x| x.root().starts_with(root)) {
            self.close_xtree();
            self.xtree = None;
        }
        self.clear_preview_subject();
    }

    /// Expulsa el disco de `root`. Con operaciones de copia sobre ese disco pregunta si se
    /// cancelan y espera a que limpien lo a medio copiar (`eject_after_jobs`); después suelta lo
    /// de xdir y expulsa en otro hilo.
    pub(super) fn eject(&mut self, root: PathBuf) {
        let fs = sys::file_system();
        let label = fs.drive_label(&fs.drive_name(&root));
        let at = root.clone();
        self.eject_after_jobs(&at, &label, Box::new(move |app: &mut App| app.eject_now(root)));
    }

    /// Suelta lo de xdir en el disco y lo expulsa en otro hilo (sin trabajos de copia encima).
    fn eject_now(&mut self, root: PathBuf) {
        self.release_volume(&root);
        let fs = sys::file_system();
        let label = fs.drive_label(&fs.drive_name(&root));
        self.spawn_task(move || {
            let result = sys::shell().eject(&root);
            TaskDone::Ejected { root, label, result }
        });
    }

    /// «Mostrar el original» del elemento seleccionado (si es un enlace).
    pub(super) fn show_original(&mut self) {
        let Some(link) = self.selected_link() else { return };
        let pane = self.active;
        self.spawn_task(move || {
            // Solo vale un original que exista (sin leer su contenido).
            let target = sys::shell().link_target(&link).filter(|t| std::fs::symlink_metadata(t).is_ok());
            TaskDone::Original { pane, link, target }
        });
    }

    /// El único seleccionado, si es un acceso directo, alias o enlace simbólico.
    pub(super) fn selected_link(&self) -> Option<PathBuf> {
        let tab = self.panes[self.active].tab();
        if !matches!(tab.location, Location::Path(_)) {
            return None;
        }
        let mut sel = tab.list.selected_indices();
        let (Some(i), None) = (sel.next(), sel.next()) else { return None };
        let e = &tab.list.entries[i];
        let lnk = sys::shell().link_style() == crate::names::LinkStyle::Lnk
            && std::path::Path::new(&e.name).extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk"));
        (e.attrs & crate::fs::ATTR_LINK != 0 || lnk).then(|| tab.entry_path(i))
    }

    /// «Crear acceso directo»: uno junto a cada seleccionado (solo carpetas de disco).
    pub(super) fn create_shortcuts(&mut self) {
        if !matches!(self.panes[self.active].tab().location, Location::Path(_)) || self.in_trash() {
            return;
        }
        let targets = self.selection(self.active);
        if !targets.is_empty() {
            sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), Op::Shortcuts { targets });
        }
    }
}

//! Trabajos sueltos en otro hilo (resolver un acceso directo, saber qué discos se pueden expulsar,
//! expulsar, renombrar varios, saber si un disco distingue mayúsculas): el hilo de interfaz nunca
//! espera al disco. Cada trabajo vuelve por el canal `task_tx` con un `TaskDone` y avisa con
//! `WM_APP_TASK`.

use super::*;

/// Terminó un trabajo de `spawn_task`; el resultado está en el canal.
pub const WM_APP_TASK: u32 = crate::WM_APP + 26;

/// Cuánto se espera al sistema al expulsar un disco (`App::eject_limit`).
pub(super) const EJECT_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);
/// El motivo cuando el sistema no contesta a tiempo (o el hilo de expulsar falla).
const NO_ANSWER: &str = "el sistema no respondió";

fn no_answer() -> crate::shell::EjectError {
    crate::shell::EjectError { busy_by: None, busy: false, message: NO_ANSWER.into() }
}

/// Expulsa `root` esperando como mucho `limit` (y sin caerse si el sistema entra en pánico): si
/// no contesta, el error `NO_ANSWER`. El pedido sigue en su hilo: si después expulsa bien, `late`
/// manda `TaskDone::LateEjected` (corrige el aviso); un error tardío no se avisa.
fn eject_within(root: &std::path::Path, limit: std::time::Duration, late: impl FnOnce() + Send + 'static) -> Result<(), crate::shell::EjectError> {
    let (tx, rx) = std::sync::mpsc::channel();
    // Quién recibe: el que espera (mientras espera) o, si ya se fue, el aviso tardío. Con el
    // candado, el resultado no se pierde entre los dos.
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    let theirs = slot.clone();
    let r = root.to_path_buf();
    std::thread::spawn(move || {
        let result = sys::shell().eject(&r);
        let waiting = theirs.lock().unwrap_or_else(|e| e.into_inner()).take();
        match waiting {
            Some(tx) => {
                let _ = tx.send(result);
            }
            None if result.is_ok() => late(),
            None => {}
        }
    });
    match rx.recv_timeout(limit) {
        Ok(r) => r,
        Err(_) => {
            // Si llegó justo ahora, se toma; si no, el que espera se va.
            let gone = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            match gone {
                Some(_) => Err(no_answer()),
                None => rx.recv().unwrap_or_else(|_| Err(no_answer())),
            }
        }
    }
}

/// Lo que devuelve un trabajo suelto.
pub(super) enum TaskDone {
    /// Terminó de escribir `config.toml` (la pestaña Configuración): lo que quedó, o el error.
    ConfigSaved { result: Result<Config, String> },
    /// Terminó de activar (`on`) o desactivar el modo portable.
    Portable { on: bool, result: Result<(), String> },
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
    /// Se leyó `red.toml` (con su fecha); `None`: no cambió.
    NetPlaces(Option<(crate::net::NetPlaces, Option<std::time::SystemTime>)>),
    /// Lo conectado (`Network::connections`).
    NetConnections(Result<Vec<crate::net::NetConnection>, crate::net::NetError>),
    /// La identidad del volumen de cada conexión (pedida aparte).
    NetVolumeIds(Vec<(PathBuf, std::ffi::OsString)>),
    /// Terminó de conectar `addr` (`tab`: la pestaña que lo pidió al fallar por credenciales;
    /// `None`: abrir el resultado en una pestaña nueva del panel `pane`).
    /// `generation`: la del intento (los cancelados se descartan).
    Connected { pane: usize, tab: Option<usize>, generation: u64, addr: crate::net::Address, result: Result<PathBuf, crate::net::NetError> },
    /// Nombres de unidades de red, pedidos en otro hilo (para el árbol).
    #[cfg_attr(not(feature = "arbol"), allow(dead_code))]
    DriveLabels(Vec<(PathBuf, String)>),
    /// Las letras libres para una unidad de red.
    FreeLetters(Vec<char>),
    /// Una conexión que había vencido terminó bien después.
    LateConnected { generation: u64, addr: crate::net::Address, path: PathBuf },
    /// Se expulsó `root` después de que venciera la espera: el aviso se corrige.
    LateEjected { root: PathBuf, label: String },
    /// Las subcarpetas de la flecha `chevron` del panel `pane` (pedidas en otro hilo).
    Subfolders { pane: usize, chevron: usize, loc: Location, items: Vec<(String, Location)> },
    /// Terminó de desconectar `root` (`label`: el último tramo de la dirección; `force`: si fue
    /// a la fuerza).
    Disconnected { root: PathBuf, label: String, force: bool, result: Result<(), crate::net::NetError> },
    /// Se extrajo a la temporal un archivo de adentro de un archivo comprimido para abrirlo.
    ArchiveOpened { name: String, with: bool, token: std::sync::Arc<std::sync::atomic::AtomicBool>, result: Result<PathBuf, String> },
    /// Se extrajo (o no) a la temporal un elemento para la vista previa.
    /// `cancelled`: se cortó porque el foco se fue (si volvió, se pide de nuevo).
    ArchivePrepared { path: PathBuf, ok: bool, cancelled: bool },
}

impl App {
    /// Corre `f` en un hilo; su resultado vuelve por el canal y se procesa con `WM_APP_TASK`.
    pub(super) fn spawn_task(&self, f: impl FnOnce() -> TaskDone + Send + 'static) {
        let (tx, hwnd) = (self.task_tx.clone(), self.hwnd);
        std::thread::spawn(move || {
            // Si entra en pánico no vuelve nada (como antes); los que dejan estado puesto usan
            // `spawn_task_or`.
            if let Ok(t) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                && tx.send(t).is_ok()
            {
                hwnd.post(WM_APP_TASK, 0, 0);
            }
        });
    }

    /// Como `spawn_task`, pero si `f` entra en pánico vuelve `fallback()` (el resultado de error):
    /// expulsar, desconectar o extraer nunca quedan «en curso» para siempre.
    pub(super) fn spawn_task_or(&self, f: impl FnOnce() -> TaskDone + Send + 'static, fallback: impl FnOnce() -> TaskDone + Send + 'static) {
        let (tx, hwnd) = (self.task_tx.clone(), self.hwnd);
        std::thread::spawn(move || {
            let t = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| fallback());
            if tx.send(t).is_ok() {
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
            TaskDone::ConfigSaved { result } => self.on_config_saved(result),
            TaskDone::Portable { on, result } => self.on_portable_done(on, result),
            TaskDone::Renamed { dir, result, undoing } => self.on_renamed(dir, result, undoing),
            TaskDone::CaseInfo { dir, case_insensitive } => self.on_case_info(&dir, case_insensitive),
            TaskDone::ArchiveOpened { name, with, token, result } => self.on_archive_opened(name, with, token, result),
            TaskDone::ArchivePrepared { path, ok, cancelled } => self.on_archive_prepared(path, ok, cancelled),
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
            TaskDone::NetVolumeIds(ids) => self.remember_net_volume_ids(ids),
            TaskDone::Connected { pane, tab, generation, addr, result } => {
                self.activity.end(&super::activity::ActivityKey::Connect(generation));
                self.on_connected(pane, tab, generation, addr, result)
            }
            TaskDone::DriveLabels(labels) => self.on_drive_labels(labels),
            TaskDone::Subfolders { pane, chevron, loc, items } => self.on_subfolders(pane, chevron, loc, items),
            TaskDone::LateConnected { generation, addr, path } => self.on_late_connected(generation, addr, path),
            TaskDone::LateEjected { root, label } => self.on_ejected(root, label, Ok(())),
            TaskDone::FreeLetters(letters) => self.on_free_letters(letters),
            TaskDone::Disconnected { root, label, force, result } => self.on_disconnected(root, label, force, result),
        }
    }

    /// Terminó de expulsar: el aviso, o la pregunta de reintentar si alguien lo está usando.
    fn on_ejected(&mut self, root: PathBuf, label: String, result: Result<(), crate::shell::EjectError>) {
        match result {
            // El sistema no contestó (o el hilo falló): un aviso; el ⏏ ya se puede volver a tocar.
            Err(e) if e.message == NO_ANSWER => self.notice = Some(format!("No se pudo expulsar \"{label}\": {NO_ANSWER}")),
            Ok(()) => {
                self.notice = Some(format!("Se expulsó \"{label}\": ya lo podés quitar"));
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
                Some(tab.entry_path(idx)).filter(|p| self.ejectable.contains(p) || self.network_root(p).is_some_and(|c| crate::sys::same_path_folded(&c.local, p)))
            }
            Location::Path(p) => {
                let disk = self.ejectable.iter().filter(|r| p.starts_with(r)).max_by_key(|r| r.as_os_str().len()).cloned();
                // Una carpeta de red: su conexión (la más adentro gana).
                let net = self.network_root(p).map(|c| c.local.clone());
                match (disk, net) {
                    (Some(d), Some(n)) => Some(if n.as_os_str().len() >= d.as_os_str().len() { n } else { d }),
                    (d, n) => n.or(d),
                }
            }
            _ => None,
        }
    }

    /// Suelta lo que xdir tiene abierto en el disco de `root`: las pestañas en ese disco pasan a
    /// «Este equipo» (y sueltan su guardián), se cierra xtree si su raíz está ahí y se vacía la
    /// vista previa. El panel de información suelta lo de ese disco y corta la suma de carpeta:
    /// devuelve su hilo para esperar a que cierre la carpeta que tenía abierta (sale enseguida:
    /// mira el corte en cada elemento y antes de abrir cada carpeta). También corta las
    /// comparaciones y sumas de verificación que leen de ahí y devuelve sus hilos (salen en la
    /// próxima lectura).
    pub(super) fn release_volume(&mut self, root: &std::path::Path) -> Vec<std::thread::JoinHandle<()>> {
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
        let mut waits = self.release_compare_under(root);
        waits.extend(self.info.sum.release_under(root));
        waits.extend(self.release_info_under(root));
        waits
    }

    /// Expulsa el disco de `root`. Con operaciones de copia sobre ese disco pregunta si se
    /// cancelan y espera a que limpien lo a medio copiar (`eject_after_jobs`); después suelta lo
    /// de xdir y expulsa en otro hilo.
    pub(super) fn eject(&mut self, root: PathBuf) {
        // Ya se está expulsando (su ⏏ gira): un segundo pedido no hace nada.
        if self.is_ejecting(&root) {
            return;
        }
        // Una conexión de red se desconecta (mismo flujo: trabajos, soltar, en otro hilo).
        if let Some(c) = self.network_root(&root) {
            let local = c.local.clone();
            self.disconnect(local);
            return;
        }
        let fs = sys::file_system();
        let label = fs.drive_label(&fs.drive_name(&root));
        let at = root.clone();
        self.eject_after_jobs(&at, &label, super::transfers::Verb::Eject, Box::new(move |app: &mut App| app.eject_now(root)));
    }

    /// Suelta lo de xdir en el disco y lo expulsa en otro hilo (sin trabajos de copia encima).
    fn eject_now(&mut self, root: PathBuf) {
        let reading = self.release_volume(&root);
        let fs = sys::file_system();
        let label = fs.drive_label(&fs.drive_name(&root));
        let (limit, eject_limit) = (self.jobs.limit, self.eject_limit);
        let left = self.ejecting_readers(&root, reading.len());
        let (r, l) = (root.clone(), label.clone());
        let (late_root, late_label, late_tx, hwnd) = (root.clone(), label.clone(), self.task_tx.clone(), self.hwnd);
        let late = move || {
            if late_tx.send(TaskDone::LateEjected { root: late_root, label: late_label }).is_ok() {
                hwnd.post(WM_APP_TASK, 0, 0);
            }
        };
        self.spawn_task_or(
            move || {
                // Como mucho el límite: un lector trabado no deja el disco sin expulsar (si sigue
                // abierto, el sistema lo dice al expulsar).
                super::compare::wait_bounded(reading, limit, &left);
                let result = eject_within(&root, eject_limit, late);
                TaskDone::Ejected { root, label, result }
            },
            move || TaskDone::Ejected { root: r, label: l, result: Err(no_answer()) },
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ronda 1: un trabajo que entra en pánico manda su resultado de error (no deja el estado puesto).
    #[test]
    fn panicking_task_sends_fallback() {
        crate::sys::testing::install_fakes();
        let a = App::new(Init::blank(), crate::sys::testing::FakeHost::new());
        a.spawn_task_or(|| panic!("prueba"), || TaskDone::FreeLetters(vec!['Z']));
        let t = a.task_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("llega el de error");
        assert!(matches!(t, TaskDone::FreeLetters(l) if l == ['Z']));
    }
}

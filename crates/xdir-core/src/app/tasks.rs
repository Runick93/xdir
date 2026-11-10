//! Trabajos sueltos en otro hilo (resolver un acceso directo, saber qué discos se pueden expulsar,
//! expulsar): el hilo de interfaz nunca espera al disco. Cada trabajo devuelve un `TaskDone` por
//! canal y avisa con `WM_APP_TASK`.

use super::*;

/// Terminó un trabajo suelto; el resultado está en el canal.
pub const WM_APP_TASK: u32 = crate::WM_APP + 26;

/// Lo que devuelve un trabajo suelto.
pub(super) enum TaskDone {
    /// El original de un acceso directo (`None`: no se resolvió o ya no existe).
    Original { link: PathBuf, target: Option<PathBuf> },
}

impl App {
    /// Corre `f` en otro hilo; su resultado llega a `on_task_done`.
    pub(super) fn spawn_task(&self, f: impl FnOnce() -> TaskDone + Send + 'static) {
        let (tx, hwnd) = (self.task_tx.clone(), self.hwnd);
        std::thread::spawn(move || {
            if tx.send(f()).is_ok() {
                hwnd.post(WM_APP_TASK, 0, 0);
            }
        });
    }

    pub(super) fn on_task_done(&mut self, t: TaskDone) {
        match t {
            TaskDone::Original { link, target } => {
                let name = link.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match target.as_ref().and_then(|t| Some((t.parent()?, t.file_name()?))) {
                    Some((parent, file)) => {
                        let (hwnd, tx) = (self.hwnd, self.tx.clone());
                        let tab = self.panes[self.active].tab_mut();
                        tab.navigate(hwnd, &tx, Location::Path(parent.to_path_buf()));
                        tab.focus_after_load(file.to_os_string());
                    }
                    None => self.notice = Some(format!("No se encuentra el original de \"{name}\"")),
                }
            }
        }
    }

    /// «Mostrar el original» del elemento seleccionado (si es un enlace).
    pub(super) fn show_original(&mut self) {
        let Some(link) = self.selected_link() else { return };
        self.spawn_task(move || {
            // Solo vale un original que exista (sin leer su contenido).
            let target = sys::shell().link_target(&link).filter(|t| std::fs::symlink_metadata(t).is_ok());
            TaskDone::Original { link, target }
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
        if !matches!(self.panes[self.active].tab().location, Location::Path(_)) {
            return;
        }
        let targets = self.selection(self.active);
        if !targets.is_empty() {
            sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), Op::Shortcuts { targets });
        }
    }
}

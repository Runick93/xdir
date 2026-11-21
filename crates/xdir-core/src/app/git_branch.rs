//! El menú de ramas del pie del panel y el cambio de rama. Todo lo que toca git corre en otro
//! hilo (`spawn_task`): la interfaz nunca espera al proceso.

use super::*;
use crate::git::{self, Head, SwitchOutcome};
use crate::host::MenuPos;
use crate::shell::Entry;
use super::tasks::TaskDone;

/// Cuántas ramas muestra el menú; el resto se resume en una línea.
const MENU_MAX: usize = 40;

/// Cómo se cambia de rama.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum GitHow {
    /// Sin tocar nada más.
    Plain,
    /// Guardando antes los cambios en un stash.
    Stash,
    /// Descartando los cambios.
    Discard,
}

impl App {
    /// Ya se sabe si hay git: las pestañas listadas antes releen su repo y su rama (unas pocas
    /// lecturas de archivos chicos, solo en carpetas locales: `tab::repo_head_of`).
    pub(super) fn on_git_ready(&mut self) {
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.git = crate::tab::repo_head_of(&tab.location);
            }
        }
    }

    /// Clic en el botón de rama del panel `pane`: relee la rama y pide la lista de ramas.
    pub(super) fn open_branch_menu(&mut self, pane: usize) {
        if self.git_busy || pane >= self.panes.len() {
            return;
        }
        let tab = self.panes[pane].tab_mut();
        let Some(old_root) = tab.git.as_ref().map(|g| g.root.clone()) else { return };
        // La rama pudo cambiar por fuera desde el último listado.
        tab.git = git::read_head(&old_root);
        let Some(root) = tab.git.as_ref().map(|g| g.root.clone()) else { return };
        self.git_busy = true;
        let r = root.clone();
        self.spawn_task_or(
            move || TaskDone::GitBranches { pane, result: git::branches(&r), root: r },
            move || TaskDone::GitBranches { pane, root, result: Err("git no respondió".into()) },
        );
    }

    /// Llegaron las ramas: arma el menú, lo muestra y, si se elige una, cambia a ella.
    pub(super) fn on_git_branches(&mut self, pane: usize, root: PathBuf, result: Result<Vec<String>, String>) {
        self.git_busy = false;
        let list = match result {
            Ok(l) => l,
            Err(t) => {
                self.notice = Some(t);
                return;
            }
        };
        // El panel pudo cerrarse (o dejar el repo) mientras tanto.
        let Some(p) = self.panes.get(pane) else { return };
        let current = match p.tab().git.as_ref() {
            Some(g) if g.root == root => match &g.head {
                Head::Branch(b) => Some(b.clone()),
                Head::Detached(_) => None,
            },
            _ => return,
        };
        let r = p.branch_button_rect();
        let item = |id: u32, label: &str, enabled: bool, checked: bool| Entry::Item { id, label: label.to_string(), enabled, checked };
        let mut entries: Vec<Entry> = Vec::new();
        if list.is_empty() {
            entries.push(item(0, "(sin ramas)", false, false));
        }
        for (i, name) in list.iter().take(MENU_MAX).enumerate() {
            let is_current = current.as_deref() == Some(name.as_str());
            entries.push(item(i as u32 + 1, name, !is_current, is_current));
        }
        if list.len() > MENU_MAX {
            entries.push(item(0, &format!("…y {} más", list.len() - MENU_MAX), false, false));
        }
        let choice = self.host.popup_menu(&entries, MenuPos::Above(r.left, r.top), None, None);
        let Some(id) = choice else { return };
        let Some(name) = (id as usize).checked_sub(1).filter(|i| *i < MENU_MAX).and_then(|i| list.get(i)) else { return };
        if current.as_deref() == Some(name.as_str()) {
            return;
        }
        self.start_switch(root, name.clone(), GitHow::Plain);
    }

    /// Cambia el repo `root` a `dest` en otro hilo.
    pub(super) fn start_switch(&mut self, root: PathBuf, dest: String, how: GitHow) {
        self.git_busy = true;
        let (r, d) = (root.clone(), dest.clone());
        self.spawn_task_or(
            move || {
                let result = match how {
                    GitHow::Plain => git::switch(&r, &d),
                    GitHow::Stash => git::stash_and_switch(&r, &d),
                    GitHow::Discard => git::discard_and_switch(&r, &d),
                };
                TaskDone::GitSwitched { root: r, dest: d, how, result }
            },
            move || TaskDone::GitSwitched { root, dest, how, result: SwitchOutcome::Failed("git no respondió".into()) },
        );
    }

    /// Terminó el cambio de rama: recarga todo lo que esté en ese repo o avisa qué pasó.
    pub(super) fn on_git_switched(&mut self, root: PathBuf, dest: String, how: GitHow, result: SwitchOutcome) {
        self.git_busy = false;
        match result {
            SwitchOutcome::Switched => {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                for pane in &mut self.panes {
                    for tab in &mut pane.tabs {
                        if tab.git.as_ref().is_some_and(|g| g.root == root) {
                            tab.reload(hwnd, &tx, true);
                        }
                    }
                }
                if how == GitHow::Stash {
                    self.notice = Some(format!("Tus cambios están en el stash ({})", git::stash_message(&dest)));
                }
            }
            SwitchOutcome::Failed(t) => self.notice = Some(t),
            // Solo el cambio simple abre la ventana; si ya se probó guardar o descartar, no se repite.
            SwitchOutcome::Dirty { files } if how == GitHow::Plain && files > 0 => self.ask_dirty(root, dest, files),
            // Sin archivos con seguimiento modificados descartar no arregla nada: son archivos nuevos.
            SwitchOutcome::Dirty { files: 0 } => {
                self.notice = Some("No se pudo cambiar de rama: hay archivos sin seguimiento que se sobrescribirían".into())
            }
            SwitchOutcome::Dirty { .. } => self.notice = Some("No se pudo cambiar de rama: hay cambios sin guardar que chocan".into()),
        }
    }
    /// Git se negó a cambiar por cambios sin guardar: ofrece guardarlos en el stash o descartarlos.
    /// Las ventanas son modales en el hilo de la interfaz; `git_busy` ya está apagado y
    /// `start_switch` lo vuelve a encender si se sigue adelante.
    fn ask_dirty(&mut self, root: PathBuf, dest: String, files: usize) {
        let title = format!("No se puede pasar a «{dest}»");
        let text = "Tienes cambios sin guardar que chocan con esa rama. ¿Guardarlos en el stash y cambiar? Sí: guardar en el stash y cambiar. No: descartar los cambios. Cancelar: quedarse aquí.";
        match self.host.ask_yes_no_cancel(&title, text) {
            Some(true) => self.start_switch(root, dest, GitHow::Stash),
            Some(false) => {
                let text = if files == 1 {
                    "Se perderá 1 archivo modificado. Esto no se puede deshacer. ¿Descartar y cambiar?".to_string()
                } else {
                    format!("Se perderán {files} archivos modificados. Esto no se puede deshacer. ¿Descartar y cambiar?")
                };
                if self.host.ask_yes_no("Descartar cambios", &text) {
                    self.start_switch(root, dest, GitHow::Discard);
                }
            }
            None => {}
        }
    }
}

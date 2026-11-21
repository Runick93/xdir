//! El menú de ramas del pie del panel y el cambio de rama. Todo lo que toca git corre en otro
//! hilo (`spawn_task`): la interfaz nunca espera al proceso.

use super::*;
use super::confirm::{ButtonKind, Confirm, ConfirmButton, ConfirmHit};
use crate::keys::keys::{VK_LEFT, VK_RIGHT, VK_TAB};
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
        self.git_task = "Leyendo las ramas".into();
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
    /// Mientras tanto, el indicador de actividad lo dice; no hay límite de tiempo ni se corta
    /// git (cortarlo puede dejar `index.lock` puesto).
    pub(super) fn start_switch(&mut self, root: PathBuf, dest: String, how: GitHow) {
        self.git_busy = true;
        self.git_task = format!("Cambiando a \"{dest}\"");
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
            // Solo el cambio simple abre una ventana; si ya se probó guardar o descartar, no se repite.
            SwitchOutcome::UntrackedInTheWay if how == GitHow::Plain => self.ask_untracked(root, dest),
            SwitchOutcome::UntrackedInTheWay => {
                self.notice = Some("No se pudo cambiar de rama: hay archivos sin seguimiento que se sobrescribirían".into())
            }
            SwitchOutcome::Dirty { files } if how == GitHow::Plain && files > 0 => self.ask_dirty(root, dest, files),
            SwitchOutcome::Dirty { .. } => self.notice = Some("No se pudo cambiar de rama: hay cambios sin guardar que chocan".into()),
        }
    }

    /// Git se negó a cambiar porque hay archivos nuevos que la rama destino sobrescribiría: solo
    /// se ofrece guardarlos en el stash (`-u` se los lleva). Descartar no sirve: no toca los
    /// archivos sin seguimiento.
    fn ask_untracked(&mut self, root: PathBuf, dest: String) {
        let dialog = Confirm::new(
            format!("No se puede pasar a «{dest}»"),
            "Hay archivos sin seguimiento que se sobrescribirían al cambiar. ¿Querés guardarlos en el stash y cambiar?",
            vec![ConfirmButton::new("Guardar en el stash y cambiar", ButtonKind::Primary), ConfirmButton::new("Cancelar", ButtonKind::Plain)],
            0,
        );
        self.confirm = Some(ConfirmState { dialog, ask: GitAsk::Untracked { root, dest } });
    }

    /// Git se negó a cambiar por cambios sin guardar: ofrece guardarlos en el stash o descartarlos
    /// (esto último pide una segunda confirmación). El diálogo es de la propia ventana: `git_busy`
    /// ya está apagado y `start_switch` lo vuelve a encender si se sigue adelante.
    fn ask_dirty(&mut self, root: PathBuf, dest: String, files: usize) {
        let dialog = Confirm::new(
            format!("No se puede pasar a «{dest}»"),
            "Tenés cambios sin guardar que chocan con esa rama. Podés guardarlos en el stash para recuperarlos después, o descartarlos.",
            vec![
                ConfirmButton::new("Guardar en el stash y cambiar", ButtonKind::Primary),
                ConfirmButton::new("Descartar…", ButtonKind::Plain),
                ConfirmButton::new("Cancelar", ButtonKind::Plain),
            ],
            0,
        );
        self.confirm = Some(ConfirmState { dialog, ask: GitAsk::Dirty { root, dest, files } });
    }

    /// La segunda confirmación de descartar: dice cuántos archivos se pierden y deja el foco en
    /// «Cancelar», para que un Enter distraído no destruya nada.
    fn ask_discard(&mut self, root: PathBuf, dest: String, files: usize) {
        let text = if files == 1 {
            "Se perderá 1 archivo modificado. Esto no se puede deshacer.".to_string()
        } else {
            format!("Se perderán {files} archivos modificados. Esto no se puede deshacer.")
        };
        let dialog = Confirm::new(
            "Descartar cambios",
            text,
            vec![ConfirmButton::new("Descartar y cambiar", ButtonKind::Danger), ConfirmButton::new("Cancelar", ButtonKind::Plain)],
            1,
        );
        self.confirm = Some(ConfirmState { dialog, ask: GitAsk::Discard { root, dest } });
    }

    /// Clic en el diálogo: un botón lo resuelve; la cruz lo cancela; el resto no hace nada.
    pub(super) fn confirm_click(&mut self, hit: Option<ConfirmHit>) {
        match hit {
            Some(ConfirmHit::Button(i)) => self.confirm_choose(Some(i)),
            Some(ConfirmHit::Close) => self.confirm_choose(None),
            Some(ConfirmHit::Panel) | None => {}
        }
        self.redraw();
    }

    /// Teclas del diálogo: Esc cancela, Enter aprieta el botón con el foco, y las flechas y Tab
    /// mueven el foco. Se queda con todas las teclas mientras está abierto.
    pub(super) fn confirm_key(&mut self, vk: Key, shift: bool) -> bool {
        let step = match vk {
            VK_LEFT | VK_UP => -1,
            VK_RIGHT | VK_DOWN => 1,
            VK_TAB if shift => -1,
            VK_TAB => 1,
            _ => 0,
        };
        if vk == VK_ESCAPE {
            self.confirm_choose(None);
        } else if vk == VK_RETURN {
            let focus = self.confirm.as_ref().map(|c| c.dialog.focus);
            self.confirm_choose(focus);
        } else if step != 0
            && let Some(c) = &mut self.confirm
        {
            c.dialog.move_focus(step);
        }
        self.redraw();
        true
    }

    /// Cierra el diálogo y hace lo que corresponde al botón elegido (`None`: se canceló). El
    /// último botón de cada diálogo es siempre «Cancelar».
    pub(super) fn confirm_choose(&mut self, choice: Option<usize>) {
        let Some(state) = self.confirm.take() else { return };
        let Some(i) = choice else { return };
        match (state.ask, i) {
            (GitAsk::Dirty { root, dest, .. } | GitAsk::Untracked { root, dest }, 0) => self.start_switch(root, dest, GitHow::Stash),
            (GitAsk::Dirty { root, dest, files }, 1) => self.ask_discard(root, dest, files),
            (GitAsk::Discard { root, dest }, 0) => self.start_switch(root, dest, GitHow::Discard),
            _ => {}
        }
    }
}

/// Qué pregunta hay abierta y con qué datos seguir según la respuesta.
pub(super) enum GitAsk {
    /// Cambios con seguimiento que chocan: guardar en el stash, descartar (con otra confirmación) o cancelar.
    Dirty { root: PathBuf, dest: String, files: usize },
    /// La segunda confirmación de descartar; se abre desde `Dirty`.
    Discard { root: PathBuf, dest: String },
    /// Archivos sin seguimiento que estorban: solo guardar en el stash o cancelar.
    Untracked { root: PathBuf, dest: String },
}

/// El diálogo de confirmación abierto y la pregunta a la que responde.
pub(super) struct ConfirmState {
    pub(super) dialog: Confirm,
    pub(super) ask: GitAsk,
}
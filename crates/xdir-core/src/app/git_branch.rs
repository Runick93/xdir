//! El menú de ramas del pie del panel y el cambio de rama. Todo lo que toca git corre en otro
//! hilo (`spawn_task`): la interfaz nunca espera al proceso.

use super::*;
use super::confirm::{ButtonKind, Confirm, ConfirmButton, ConfirmHit};
use crate::keys::keys::{VK_LEFT, VK_RIGHT, VK_TAB};
use crate::git::{self, Head, SwitchOutcome};
use super::drop_menu::{DropHit, DropItem, DropMenu};
use super::tasks::TaskDone;

/// Cuántas ramas muestra el menú; el resto se resume en una línea.
const MENU_MAX: usize = 40;

/// Cada cuánto, como mucho, se consulta la red de un repositorio (`git fetch`).
const FETCH_EVERY: std::time::Duration = std::time::Duration::from_secs(300);

/// Qué se sabe del remoto de un repositorio.
#[derive(Default)]
pub(super) struct SyncInfo {
    /// Lo último que se leyó; `None` si la rama no sigue a ninguna remota (o aún no se leyó).
    pub(super) state: Option<git::Upstream>,
    /// Ya se hizo la primera lectura (local).
    pub(super) checked: bool,
    /// Cuándo se intentó el último `fetch`.
    pub(super) last_fetch: Option<std::time::Instant>,
    /// Hay una consulta en curso.
    pub(super) job: Option<SyncJob>,
}

/// Quién pidió una consulta del remoto.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SyncJob {
    /// La pidió la propia app: no se nota.
    Background,
    /// La pidió la persona con un clic en el botón verde: el ícono gira hasta que termina.
    Refresh,
}

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
        self.git_sync_tick();
    }

    /// Recarga, en cualquier panel, todas las pestañas que estén en el repositorio `root`.
    fn reload_repo_tabs(&mut self, root: &std::path::Path) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                if tab.git.as_ref().is_some_and(|g| g.root == root) {
                    tab.reload(hwnd, &tx, true);
                }
            }
        }
    }

    // ---- sincronizar con el remoto (botón verde / amarillo del pie) ----

    /// Los repositorios con alguna pestaña, sin repetir.
    fn git_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        for tab in self.panes.iter().flat_map(|p| p.tabs.iter()) {
            if let Some(g) = &tab.git
                && !roots.contains(&g.root)
            {
                roots.push(g.root.clone());
            }
        }
        roots
    }

    /// Después de cada listado: pide lo que falte saber del remoto de los repositorios a la vista y
    /// pone al día el botón. `git` solo corre la primera vez que se ve un repositorio (una lectura
    /// local) y, a lo sumo cada `FETCH_EVERY`, un `fetch` que sí usa la red.
    pub(super) fn git_sync_tick(&mut self) {
        for root in self.git_roots() {
            let todo = {
                let info = self.git_sync.entry(root.clone()).or_default();
                let fetch_due = info.state.is_some() && info.last_fetch.is_none_or(|t| t.elapsed() >= FETCH_EVERY);
                if info.job.is_some() {
                    None
                } else if !info.checked {
                    Some(false)
                } else if fetch_due {
                    Some(true)
                } else {
                    None
                }
            };
            if let Some(fetch) = todo {
                self.request_sync(root, fetch, SyncJob::Background);
            }
        }
        self.apply_sync_views();
    }

    /// Pide en otro hilo cuánto adelanta o atrasa la rama de `root`; con `fetch`, trae antes lo
    /// nuevo del remoto. Un `Refresh` (lo pidió la persona) no baja a `Background` si ya había uno.
    pub(super) fn request_sync(&mut self, root: PathBuf, fetch: bool, job: SyncJob) {
        let info = self.git_sync.entry(root.clone()).or_default();
        info.job = Some(if info.job == Some(SyncJob::Refresh) { SyncJob::Refresh } else { job });
        if fetch {
            // Se anota al intentar, no al lograr: sin red no se insiste en cada listado.
            info.last_fetch = Some(std::time::Instant::now());
        }
        let r = root.clone();
        self.spawn_task_or(
            move || {
                let fetch_error = if fetch { git::fetch(&r).err() } else { None };
                TaskDone::GitSync { state: git::upstream_state(&r), root: r, fetched: fetch, fetch_error }
            },
            move || TaskDone::GitSync { root, state: None, fetched: fetch, fetch_error: Some("git no respondió".into()) },
        );
        self.apply_sync_views();
    }

    /// Llegó lo que se sabe del remoto de `root`. Solo avisa de un `fetch` fallido si lo pidió la
    /// persona; el de fondo falla en silencio (sin red, sin permisos…).
    pub(super) fn on_git_sync(&mut self, root: PathBuf, state: Option<git::Upstream>, _fetched: bool, fetch_error: Option<String>) {
        let Some(info) = self.git_sync.get_mut(&root) else { return };
        let job = info.job.take();
        info.state = state;
        info.checked = true;
        if job == Some(SyncJob::Refresh)
            && let Some(e) = fetch_error
        {
            self.notice = Some(format!("No se pudo consultar el remoto: {e}"));
        }
        // La primera lectura local puede pedir el `fetch` que corresponde.
        self.git_sync_tick();
    }

    /// Pone en cada pestaña lo que dice su botón de sincronizar, según lo que se sabe de su repo.
    pub(super) fn apply_sync_views(&mut self) {
        let pulling = self.git_pulling.clone();
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                tab.sync = tab.git.as_ref().and_then(|g| {
                    let info = self.git_sync.get(&g.root)?;
                    let up = info.state?;
                    Some(git::SyncView { behind: up.behind, spinning: info.job == Some(SyncJob::Refresh) || pulling.as_ref() == Some(&g.root) })
                });
            }
        }
    }

    /// Clic en el botón de sincronizar del panel `pane`: amarillo, baja lo nuevo (`pull`); verde,
    /// vuelve a consultar el remoto. En los dos casos el ícono gira hasta terminar.
    pub(super) fn sync_click(&mut self, pane: usize) {
        if self.git_busy {
            return;
        }
        let Some(tab) = self.panes.get(pane).map(|p| p.tab()) else { return };
        let (Some(g), Some(view)) = (tab.git.as_ref(), tab.sync) else { return };
        if view.spinning {
            return;
        }
        let (root, branch) = (g.root.clone(), g.head.label().to_string());
        if view.behind > 0 {
            self.start_pull(root, branch);
        } else {
            match self.git_sync.get_mut(&root) {
                // Ya hay una consulta de fondo en curso: pasa a verse (gira) en vez de repetirla.
                Some(info) if info.job.is_some() => info.job = Some(SyncJob::Refresh),
                _ => self.request_sync(root, true, SyncJob::Refresh),
            }
            self.apply_sync_views();
        }
        self.redraw();
    }

    /// Baja lo nuevo del remoto en otro hilo. Mientras tanto `git_busy` impide cambiar de rama, y el
    /// ícono gira. Solo avanza en línea recta: si las ramas se separaron, git lo dice y no toca nada.
    fn start_pull(&mut self, root: PathBuf, branch: String) {
        self.git_busy = true;
        self.git_pulling = Some(root.clone());
        self.git_task = format!("Actualizando \"{branch}\"");
        self.apply_sync_views();
        let r = root.clone();
        self.spawn_task_or(
            move || TaskDone::GitPulled { result: git::pull(&r), root: r },
            move || TaskDone::GitPulled { root, result: Err("git no respondió".into()) },
        );
    }

    /// Terminó el `pull`: recarga las pestañas de ese repo o avisa por qué no se pudo.
    pub(super) fn on_git_pulled(&mut self, root: PathBuf, result: Result<(), String>) {
        self.git_busy = false;
        self.git_pulling = None;
        match result {
            Ok(()) => {
                // Ya no falta nada por bajar: se ve enseguida; la lectura de abajo lo confirma.
                if let Some(info) = self.git_sync.get_mut(&root) {
                    info.state = info.state.map(|u| git::Upstream { behind: 0, ..u });
                }
                self.reload_repo_tabs(&root);
            }
            Err(t) => self.notice = Some(t),
        }
        self.request_sync(root, false, SyncJob::Background);
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

    /// Llegaron las ramas: abre el menú desplegable sobre el botón (de xdir, no del sistema); si se elige una, cambia a ella.
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
        // Una fila por rama (las más recientes primero), la actual con el tilde y sin poder
        // elegirse; `picks[i]` es la rama de la fila `i` (o nada si es una nota).
        let anchor = p.branch_button_rect();
        let mut items: Vec<DropItem> = Vec::new();
        let mut picks: Vec<Option<String>> = Vec::new();
        if list.is_empty() {
            items.push(DropItem::new("(sin ramas)", false, false));
            picks.push(None);
        }
        for name in list.iter().take(MENU_MAX) {
            let is_current = current.as_deref() == Some(name.as_str());
            items.push(DropItem::new(name.clone(), is_current, !is_current));
            picks.push(Some(name.clone()));
        }
        if list.len() > MENU_MAX {
            items.push(DropItem::new(format!("…y {} más", list.len() - MENU_MAX), false, false));
            picks.push(None);
        }
        self.drop_menu = Some(BranchMenu { menu: DropMenu::new(items, anchor), root, picks });
        self.redraw();
    }

    /// Clic con el menú de ramas abierto: una fila habilitada cambia de rama; el panel no hace
    /// nada; fuera del panel, lo cierra.
    pub(super) fn drop_click(&mut self, hit: Option<DropHit>) {
        match hit {
            Some(DropHit::Item(i)) => self.drop_pick(i),
            Some(DropHit::Panel) => {}
            None => self.drop_menu = None,
        }
        self.redraw();
    }

    /// Teclas del menú de ramas: Esc lo cierra, arriba y abajo mueven el foco, Enter elige la fila
    /// con foco. Se queda con todas las teclas mientras está abierto.
    pub(super) fn drop_key(&mut self, vk: Key) -> bool {
        if vk == VK_ESCAPE {
            self.drop_menu = None;
        } else if vk == VK_RETURN {
            if let Some(i) = self.drop_menu.as_ref().and_then(|m| m.menu.focus) {
                self.drop_pick(i);
            }
        } else if let Some(m) = &mut self.drop_menu {
            if vk == VK_UP {
                m.menu.move_focus(-1);
            } else if vk == VK_DOWN {
                m.menu.move_focus(1);
            }
        }
        self.redraw();
        true
    }

    /// Elige la fila `i` del menú de ramas: lo cierra y cambia a esa rama.
    fn drop_pick(&mut self, i: usize) {
        let Some(m) = self.drop_menu.take() else { return };
        if !m.menu.is_enabled(i) {
            // Una nota o la rama actual: no se puede elegir, el menú sigue abierto.
            self.drop_menu = Some(m);
            return;
        }
        if let Some(Some(name)) = m.picks.get(i) {
            self.start_switch(m.root, name.clone(), GitHow::Plain);
        }
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
                self.reload_repo_tabs(&root);
                // La rama nueva puede seguir a otra remota: se vuelve a leer cuánto falta bajar.
                self.request_sync(root.clone(), false, SyncJob::Background);
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

/// El menú de ramas abierto: el desplegable, el repositorio y la rama de cada fila.
pub(super) struct BranchMenu {
    pub(super) menu: DropMenu,
    root: PathBuf,
    picks: Vec<Option<String>>,
}

/// El diálogo de confirmación abierto y la pregunta a la que responde.
pub(super) struct ConfirmState {
    pub(super) dialog: Confirm,
    pub(super) ask: GitAsk,
}
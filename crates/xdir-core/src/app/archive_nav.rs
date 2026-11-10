//! Dentro de un archivo comprimido, en la `App`: abrir lo de adentro (extracción temporal en un
//! hilo), Alt/⌥ + doble clic para abrirlo con el sistema, contraseña o daño («¿Abrir con el
//! sistema?»), solo lectura (aviso y menú propio con lo que escribe deshabilitado), copiar y
//! arrastrar hacia afuera con el motor, y la extracción para la vista previa, la Galería y la
//! vista rápida.

use std::collections::HashSet;
use std::path::Path;

use super::tasks::TaskDone;
use super::*;
use crate::archive_nav::{self as nav, temps};
use crate::fs::{ATTR_ARCHIVE_BAD, ATTR_LINK};
use crate::transfer::JobSpec;

/// Lo copiado dentro de un archivo comprimido (no tiene rutas de disco: no va al portapapeles del
/// sistema): el archivo de verdad y las rutas interiores.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ArchiveClip {
    pub archive: PathBuf,
    pub entries: Vec<String>,
}

/// Estado de la `App` para los archivos comprimidos.
#[derive(Default)]
pub(super) struct ArchiveState {
    pub clip: Option<ArchiveClip>,
    /// Extracciones temporales listas (para la vista previa) y en camino.
    pub ready: HashSet<PathBuf>,
    pub pending: HashSet<PathBuf>,
}

/// ¿El comando escribe en la carpeta del panel o en lo elegido? (dentro de un archivo comprimido
/// no se puede).
pub(super) fn writes(c: Command) -> bool {
    matches!(
        c,
        Command::Cut
            | Command::Paste
            | Command::NewFolder
            | Command::NewTextFile
            | Command::NewFolderWithSelection
            | Command::Recycle
            | Command::Delete
            | Command::Rename
            | Command::RenameMany
            | Command::Duplicate
            | Command::CreateShortcut
            | Command::Compress
            | Command::ExtractHere
            | Command::ExtractInto
            | Command::Restore
    )
}

/// El archivo comprimido de verdad de una ubicación: el del disco o la extracción temporal del
/// último anidado (que ya está, porque se listó).
fn real_archive(file: &Path, nested: &[String]) -> PathBuf {
    match nested.split_last() {
        None => file.to_path_buf(),
        Some((last, outer)) => temps::global().entry_path(file, outer, last).unwrap_or_else(|| file.to_path_buf()),
    }
}

impl App {
    /// ¿El panel activo está dentro de un archivo comprimido?
    pub(super) fn in_archive(&self) -> bool {
        self.panes[self.active].tab().location.is_archive()
    }

    /// Antes de un comando: dentro de un archivo comprimido lo que escribe no se hace y se avisa.
    /// Devuelve `true` si se frenó.
    pub(super) fn refuse_in_archive(&mut self, c: Command) -> bool {
        if self.in_archive() && writes(c) {
            self.notice = Some(nav::READ_ONLY.into());
            return true;
        }
        false
    }

    /// Abrir el elemento `idx` del panel (doble clic, Enter): con Alt/⌥, un archivo comprimido del
    /// disco se abre con el sistema; dentro de un archivo comprimido, las carpetas y los
    /// anidados se navegan y un archivo se extrae a la temporal y se abre con su programa.
    pub(super) fn open_entry(&mut self, pane: usize, idx: usize) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let tab = self.panes[pane].tab();
        let Some(e) = tab.list.entries.get(idx) else { return };
        let name = e.name.to_string_lossy().into_owned();
        if key_down(VK_MENU) && !e.is_dir && matches!(tab.location, Location::Path(_)) && nav::is_navigable_archive(&name) {
            let path = tab.entry_path(idx);
            sys::shell().open_file(hwnd, &path, path.parent());
            return;
        }
        if !tab.location.is_archive() {
            self.panes[pane].tab_mut().open(hwnd, &tx, idx);
            return;
        }
        if e.attrs & ATTR_ARCHIVE_BAD != 0 {
            self.notice = Some(nav::BAD_NAME.into());
            return;
        }
        if e.attrs & ATTR_LINK != 0 {
            self.notice = Some(nav::LINK.into());
            return;
        }
        if self.panes[pane].tab_mut().open(hwnd, &tx, idx) {
            return;
        }
        let tab = self.panes[pane].tab();
        let (Location::Archive { file, nested, .. }, Some(rel)) = (&tab.location, tab.archive_rel(idx)) else { return };
        let (file, nested) = (file.clone(), nested.clone());
        self.notice = Some(format!("Abriendo «{name}»…"));
        self.spawn_task(move || TaskDone::ArchiveOpened { name, result: temps::global().ensure(&file, &nested, &rel).map_err(|e| e.message()) });
    }

    /// Terminó la extracción para abrir: se abre con su programa (o se avisa por qué no).
    pub(super) fn on_archive_opened(&mut self, name: String, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                self.notice = None;
                sys::shell().open_file(self.hwnd, &path, path.parent());
            }
            Err(m) => self.notice = Some(format!("No se pudo abrir «{name}»: {m}")),
        }
    }

    /// Terminó de cargarse una pestaña dentro de un archivo comprimido con un error que se
    /// arregla con el sistema (contraseña, formato, dañado): la pestaña vuelve a donde estaba y
    /// se pregunta si abrirlo con el sistema.
    pub(super) fn after_archive_load(&mut self, tab_id: usize) {
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let Some(tab) = self.tab_by_id(tab_id) else { return };
        let (Location::Archive { file, nested, .. }, Some(err)) = (&tab.location, &tab.error) else { return };
        if !nav::offers_system(err) {
            return;
        }
        let (archive, err) = (real_archive(file, nested), err.clone());
        let shown = tab.location.parent().filter(|_| !tab.can_back());
        if tab.can_back() {
            tab.go_back(hwnd, &tx);
        } else if let Some(up) = shown {
            tab.navigate(hwnd, &tx, up);
        }
        let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let q = format!("«{name}»: {err}. ¿Abrir con el sistema?");
        if self.host.clone().ask_yes_no("Abrir con el sistema", &q) {
            sys::shell().open_file(hwnd, &archive, archive.parent());
        }
    }

    /// Copiar dentro de un archivo comprimido: lo elegido queda en el portapapeles propio (el del
    /// sistema se vacía: no hay rutas que poner). Devuelve `false` si el panel no está en uno.
    pub(super) fn copy_from_archive(&mut self) -> bool {
        let tab = self.panes[self.active].tab();
        let Location::Archive { file, nested, .. } = &tab.location else { return false };
        let mut entries = Vec::new();
        for i in tab.list.selected_indices() {
            if tab.list.entries[i].attrs & ATTR_ARCHIVE_BAD != 0 {
                self.notice = Some(nav::BAD_NAME.into());
                continue;
            }
            entries.extend(tab.archive_rel(i));
        }
        if !entries.is_empty() {
            self.archive.clip = Some(ArchiveClip { archive: real_archive(file, nested), entries });
            sys::shell().clear_clipboard();
        }
        true
    }

    /// Pegar lo copiado de un archivo comprimido en `dest` (extrae con el motor). `false` si no
    /// hay nada copiado de uno o el portapapeles del sistema tiene archivos (lo último que se
    /// copió afuera manda).
    pub(super) fn paste_from_archive(&mut self, dest: &Path) -> bool {
        let Some(clip) = self.archive.clip.clone() else { return false };
        if sys::shell().clipboard_files().is_some() {
            self.archive.clip = None;
            return false;
        }
        self.start_transfer(JobSpec::ExtractEntries { archive: clip.archive, entries: clip.entries, dest: dest.to_path_buf() });
        true
    }

    /// Empezó un arrastre de lo elegido dentro de un archivo comprimido: no hay rutas de disco
    /// para el sistema, así que se arrastra dentro de xdir (soltar en otro panel lo copia ahí).
    pub(super) fn start_archive_drag(&mut self, pane: usize) -> bool {
        if !self.panes[pane].tab().location.is_archive() {
            return false;
        }
        self.drag = Drag::ArchiveOut(pane);
        self.notice = Some("Soltá en un panel para copiar ahí".into());
        true
    }

    /// Se soltó el arrastre desde un archivo comprimido en `(x, y)`: si es una carpeta de disco de
    /// otro panel (o una carpeta de su lista), lo elegido se copia ahí con el motor.
    pub(super) fn drop_archive_drag(&mut self, from: usize, x: f32, y: f32) {
        self.notice = None;
        let Some((pane, _, dest)) = self.drop_folder(x, y) else { return };
        let tab = self.panes[from].tab();
        let Location::Archive { file, nested, .. } = &tab.location else { return };
        let entries: Vec<String> =
            tab.list.selected_indices().filter(|&i| tab.list.entries[i].attrs & ATTR_ARCHIVE_BAD == 0).filter_map(|i| tab.archive_rel(i)).collect();
        if entries.is_empty() {
            return;
        }
        let archive = real_archive(file, nested);
        self.start_transfer(JobSpec::ExtractEntries { archive, entries, dest });
        self.set_active(pane);
    }

    /// Menú contextual dentro de un archivo comprimido (no hay menú del sistema para lo de
    /// adentro): lo que se puede hacer y, deshabilitado, lo que escribe.
    pub(super) fn archive_context_menu(&mut self, pane: usize) {
        let has_items = self.panes[pane].tab().list.selected_indices().next().is_some();
        let mut cmds: Vec<Command> = Vec::new();
        let mut item = |label: &str, c: Command, enabled: bool| {
            cmds.push(c);
            shell::Entry::Item { id: cmds.len() as u32, label: label.into(), enabled, checked: false }
        };
        let entries = if has_items {
            vec![
                item("Abrir", Command::Refresh, true),
                item("Vista rápida", Command::QuickLook, true),
                shell::Entry::Sep,
                item("Copiar", Command::Copy, true),
                item("Cortar", Command::Cut, false),
                item("Pegar", Command::Paste, false),
                shell::Entry::Sep,
                item("Renombrar", Command::Rename, false),
                item("Duplicar", Command::Duplicate, false),
                item("Comprimir", Command::Compress, false),
                item("Eliminar", Command::Recycle, false),
            ]
        } else {
            vec![
                item("Actualizar", Command::Refresh, true),
                shell::Entry::Sep,
                item("Pegar", Command::Paste, false),
                item("Carpeta nueva", Command::NewFolder, false),
            ]
        };
        let Some(id) = self.host.popup_menu(&entries, MenuPos::Cursor, None, None) else { return };
        let Some(&c) = cmds.get((id as usize).wrapping_sub(1)) else { return };
        // «Abrir» del menú de un elemento: el enfocado (o el primero elegido).
        if has_items && id == 1 {
            let first = self.panes[pane].tab().list.selected_indices().next();
            if let Some(idx) = first {
                self.open_entry(pane, idx);
            }
            return;
        }
        self.run_command(c);
    }

    /// El elemento de un archivo comprimido que mostrarían la vista previa, la Galería o la vista
    /// rápida: se extrae a la temporal (si no pasa del tamaño que acepta la vista previa) y, al
    /// llegar, se vuelve a pedir lo que se ve.
    pub(super) fn archive_prefetch(&mut self) {
        let pane = self.active;
        let tab = self.panes[pane].tab();
        let Location::Archive { file, nested, .. } = &tab.location else { return };
        let gallery = tab.list.view == list::View::Gallery;
        if !(self.preview_visible() || gallery || self.quick_look.is_some()) {
            return;
        }
        let list = &tab.list;
        let mut sel = list.selected_indices();
        let idx = match (sel.next(), sel.next()) {
            (Some(i), None) => i,
            (None, _) => match list.focused_index() {
                Some(i) => i,
                None => return,
            },
            _ => return,
        };
        let Some(e) = list.entries.get(idx) else { return };
        if e.is_dir || e.attrs & (ATTR_ARCHIVE_BAD | ATTR_LINK) != 0 || e.size > crate::thumbs::BIG_CACHE_BYTES as u64 {
            return;
        }
        let path = tab.entry_path(idx);
        if self.archive.ready.contains(&path) || self.archive.pending.contains(&path) {
            return;
        }
        let Some(rel) = tab.archive_rel(idx) else { return };
        let (file, nested) = (file.clone(), nested.clone());
        self.archive.pending.insert(path.clone());
        self.spawn_task(move || {
            let ok = temps::global().ensure(&file, &nested, &rel).is_ok();
            TaskDone::ArchivePrepared { path, ok }
        });
    }

    /// Llegó una extracción para la vista previa: lo que se ve de ese elemento se vuelve a pedir.
    pub(super) fn on_archive_prepared(&mut self, path: PathBuf, ok: bool) {
        self.archive.pending.remove(&path);
        if !ok {
            return;
        }
        self.archive.ready.insert(path.clone());
        self.preview.forget.push(crate::thumbs::Source::Path(path));
        self.preview.refresh();
        self.sync_preview();
        if let Some(st) = &mut self.quick_look {
            st.shown = None;
            st.pending = None;
        }
        if self.quick_look.is_some() {
            self.sync_quick_look();
        }
    }

    /// ¿La vista previa ya puede leer este elemento de un archivo comprimido?
    pub(super) fn archive_ready(&self, path: &Path) -> bool {
        self.archive.ready.contains(path)
    }
}

//! Dentro de un archivo comprimido, en la `App`: abrir lo de adentro (extracción temporal en un
//! hilo), Alt/⌥ + doble clic para abrirlo con el sistema, contraseña o daño («¿Abrir con el
//! sistema?»), solo lectura (aviso y menú propio con lo que escribe deshabilitado), copiar y
//! arrastrar hacia afuera con el motor, y la extracción para la vista previa, la Galería y la
//! vista rápida.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Turno de la extracción para la vista previa: al pasar a otro elemento la anterior se corta.
    pub prefetch_turn: Arc<AtomicU64>,
    /// Lo último que se pidió para la vista previa.
    pub prefetch_last: Option<PathBuf>,
    /// La extracción para abrir en curso (Escape la cancela).
    pub opening: Option<Arc<AtomicBool>>,
}

/// El aviso de una apertura cancelada.
const CANCELLED: &str = "Se canceló la apertura";

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
            | Command::Restore
    )
}

/// El archivo comprimido de verdad de una ubicación: el del disco o la extracción temporal del
/// último anidado (que ya está, porque se listó).
/// `None` si la cadena tiene un nombre no permitido (una sesión editada a mano): nunca otro
/// archivo en su lugar.
fn real_archive(file: &Path, nested: &[String]) -> Option<PathBuf> {
    match nested.split_last() {
        None => Some(file.to_path_buf()),
        Some((last, outer)) => temps::global().entry_path(file, outer, last),
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
        // Un archivo comprimido de adentro con Alt/⌥: se extrae y lo abre el sistema.
        if key_down(VK_MENU) && !e.is_dir && e.attrs & (ATTR_ARCHIVE_BAD | ATTR_LINK) == 0 && nav::is_navigable_archive(&name) {
            self.open_from_archive(pane, idx, false);
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
        self.open_from_archive(pane, idx, false);
    }

    /// Extrae el archivo `idx` de adentro a la temporal (en un hilo) y lo abre con su programa
    /// (o, con `with`, con el «Abrir con…» del sistema).
    pub(super) fn open_from_archive(&mut self, pane: usize, idx: usize, with: bool) {
        let tab = self.panes[pane].tab();
        let (Location::Archive { file, nested, .. }, Some(rel)) = (&tab.location, tab.archive_rel(idx)) else { return };
        let e = &tab.list.entries[idx];
        if e.is_dir || e.attrs & (ATTR_ARCHIVE_BAD | ATTR_LINK) != 0 {
            return;
        }
        let name = e.name.to_string_lossy().into_owned();
        let (file, nested) = (file.clone(), nested.clone());
        self.notice = Some(format!("Abriendo «{name}»…   Esc cancela"));
        // Una apertura nueva corta la anterior.
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.archive.opening.replace(stop.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        self.spawn_task(move || {
            let cancel = || stop.load(Ordering::Relaxed);
            let result = temps::global().ensure(&file, &nested, &rel, &temps::Limits { max: temps::OPEN_MAX, cancel: &cancel });
            TaskDone::ArchiveOpened { name, with, token: stop.clone(), result: result.map_err(|e| if e.kind == sys::ErrKind::Cancelled { CANCELLED.to_string() } else { e.message() }) }
        });
    }

    /// Escape con una apertura desde un archivo comprimido en curso: se corta. `false` si no había.
    pub(super) fn cancel_archive_open(&mut self) -> bool {
        // Queda anotada (marcada) hasta que llegue: si ya había terminado, tampoco se abre.
        let Some(stop) = self.archive.opening.as_ref().filter(|s| !s.load(Ordering::Relaxed)) else { return false };
        stop.store(true, Ordering::Relaxed);
        self.notice = Some(CANCELLED.into());
        true
    }

    /// Terminó la extracción para abrir: se abre con su programa (o se avisa por qué no).
    /// `token`: el de esa apertura; solo la última (la de `opening`) abre algo.
    pub(super) fn on_archive_opened(&mut self, name: String, with: bool, token: Arc<AtomicBool>, result: Result<PathBuf, String>) {
        // Reemplazada por otra apertura: no se abre nada ni se toca la de ahora.
        if !self.archive.opening.as_ref().is_some_and(|cur| Arc::ptr_eq(cur, &token)) {
            return;
        }
        self.archive.opening = None;
        // Cancelada con Escape.
        if token.load(Ordering::Relaxed) || matches!(&result, Err(m) if m == CANCELLED) {
            return;
        }
        match result {
            Ok(path) if with => {
                self.notice = None;
                let folder = Location::Path(path.parent().map(Path::to_path_buf).unwrap_or_default());
                let _ = sys::shell().invoke_verb(self.hwnd, &folder, &[shell::Item::from(path)], "openas");
            }
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
        let (Location::Archive { file, nested, inner }, Some(err)) = (&tab.location, &tab.error) else { return };
        // Lo que se iba a ver ya no está: a lo más cercano que sigue (sin dejarlo en el historial).
        let fallback = match err.as_str() {
            nav::GONE => Some(file.parent().map_or(Location::Computer, |p| Location::Path(p.to_path_buf()))),
            nav::NO_FOLDER if !inner.is_empty() => Some(Location::Archive { file: file.clone(), nested: nested.clone(), inner: String::new() }),
            nav::NESTED_GONE => Location::Archive { file: file.clone(), nested: nested.clone(), inner: String::new() }.parent(),
            _ => None,
        };
        if let Some(to) = fallback {
            tab.replace_location(hwnd, &tx, to);
            return;
        }
        if !nav::offers_system(err) {
            return;
        }
        let Some(archive) = real_archive(file, nested) else { return };
        let err = err.clone();
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
        let Some(archive) = real_archive(file, nested) else { return true };
        if !entries.is_empty() {
            self.archive.clip = Some(ArchiveClip { archive, entries });
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
        let Some(archive) = real_archive(file, nested) else { return };
        self.start_transfer(JobSpec::ExtractEntries { archive, entries, dest });
        self.set_active(pane);
    }
    /// Menú contextual dentro de un archivo comprimido (no hay menú del sistema para lo de
    /// adentro, pantalla X69): abrir, extraer y copiar; deshabilitado lo que escribe.
    pub(super) fn archive_context_menu(&mut self, pane: usize) {
        #[derive(Clone, Copy)]
        enum Do {
            Open,
            OpenWith,
            ExtractHere,
            ExtractTo,
            Copy,
            CopyPath,
            Refresh,
            None,
        }
        let has_items = self.panes[pane].tab().list.selected_indices().next().is_some();
        let mut acts: Vec<Do> = Vec::new();
        let mut item = |label: &str, a: Do, enabled: bool| {
            acts.push(a);
            shell::Entry::Item { id: acts.len() as u32, label: label.into(), enabled, checked: false }
        };
        let sep = || shell::Entry::Sep;
        let entries = if has_items {
            vec![
                item("Abrir", Do::Open, true),
                item("Abrir con…", Do::OpenWith, true),
                sep(),
                item("Extraer aquí", Do::ExtractHere, true),
                item("Extraer en…", Do::ExtractTo, true),
                sep(),
                item("Cortar", Do::None, false),
                item("Copiar", Do::Copy, true),
                item("Copiar como ruta de acceso", Do::CopyPath, true),
                sep(),
                item("Eliminar", Do::None, false),
                item("Renombrar", Do::None, false),
                sep(),
                item("Nueva carpeta", Do::None, false),
                item("Comprimir", Do::None, false),
            ]
        } else {
            vec![item("Actualizar", Do::Refresh, true), sep(), item("Pegar", Do::None, false), item("Nueva carpeta", Do::None, false)]
        };
        let Some(id) = self.host.popup_menu(&entries, MenuPos::Cursor, None, None) else { return };
        let Some(&chosen) = acts.get((id as usize).wrapping_sub(1)) else { return };
        let first = self.panes[pane].tab().list.selected_indices().next();
        match chosen {
            Do::Open => {
                if let Some(idx) = first {
                    self.open_entry(pane, idx);
                }
            }
            Do::OpenWith => {
                if let Some(idx) = first {
                    self.open_from_archive(pane, idx, true);
                }
            }
            Do::ExtractHere => self.extract_from_archive(pane, false),
            Do::ExtractTo => self.extract_from_archive(pane, true),
            Do::Copy => self.run_command(Command::Copy),
            Do::CopyPath => self.copy_archive_paths(pane),
            Do::Refresh => self.run_command(Command::Refresh),
            Do::None => {}
        }
    }

    /// «Copiar como ruta de acceso» dentro de un archivo comprimido: la ruta de adentro de cada uno
    /// («/x/fotos.zip/2024/a.jpg»), sin los de nombre no permitido.
    pub(super) fn copy_archive_paths(&mut self, pane: usize) {
        let tab = self.panes[pane].tab();
        let text: Vec<String> = tab
            .list
            .selected_indices()
            .filter(|&i| tab.list.entries[i].attrs & ATTR_ARCHIVE_BAD == 0)
            .filter_map(|i| tab.location.archive_child(&tab.list.entries[i].name.to_string_lossy()))
            .map(|l| format!("\"{}\"", l.address_text()))
            .collect();
        if !text.is_empty() {
            let _ = sys::shell().copy_text(self.hwnd, &text.join("\r\n"));
        }
    }

    /// «Extraer aquí» / «Extraer en…» (atajo, paleta o menú) dentro de un archivo comprimido: lo
    /// elegido va a la carpeta del archivo, o a la que se elija; siempre afuera.
    pub(super) fn extract_from_archive(&mut self, pane: usize, choose: bool) {
        let dir = if choose {
            sys::shell().pick_folder(self.hwnd, "Extraer en")
        } else {
            match &self.panes[pane].tab().location {
                Location::Archive { file, .. } => file.parent().map(Path::to_path_buf),
                _ => None,
            }
        };
        if let Some(dir) = dir {
            self.extract_selection_to(pane, dir);
        }
    }

    /// Lo elegido en el panel `pane` (dentro de un archivo comprimido) se extrae en `dir` con el
    /// motor («Extraer aquí» del menú: la carpeta del archivo; «Extraer en…»: la que se elija).
    fn extract_selection_to(&mut self, pane: usize, dir: PathBuf) {
        let tab = self.panes[pane].tab();
        let Location::Archive { file, nested, .. } = &tab.location else { return };
        let entries: Vec<String> =
            tab.list.selected_indices().filter(|&i| tab.list.entries[i].attrs & ATTR_ARCHIVE_BAD == 0).filter_map(|i| tab.archive_rel(i)).collect();
        if let (false, Some(archive)) = (entries.is_empty(), real_archive(file, nested)) {
            self.start_transfer(JobSpec::ExtractEntries { archive, entries, dest: dir });
        }
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
            (Some(i), None) => Some(i),
            (None, _) => list.focused_index(),
            _ => None,
        };
        let limit = crate::thumbs::BIG_CACHE_BYTES as u64;
        let wanted = idx.filter(|&i| list.entries.get(i).is_some_and(|e| !e.is_dir && e.attrs & (ATTR_ARCHIVE_BAD | ATTR_LINK) == 0 && e.size <= limit));
        let path = wanted.map(|i| tab.entry_path(i));
        // Pasó a otro elemento (o a ninguno): la extracción anterior se corta.
        if path != self.archive.prefetch_last {
            self.archive.prefetch_turn.fetch_add(1, Ordering::Relaxed);
            self.archive.prefetch_last = path.clone();
        }
        let (Some(idx), Some(path)) = (wanted, path) else { return };
        if self.archive.ready.contains(&path) || self.archive.pending.contains(&path) {
            return;
        }
        let Some(rel) = tab.archive_rel(idx) else { return };
        let (file, nested) = (file.clone(), nested.clone());
        self.archive.pending.insert(path.clone());
        let turn = self.archive.prefetch_turn.clone();
        let mine = turn.load(Ordering::Relaxed);
        self.spawn_task(move || {
            let cancel = || turn.load(Ordering::Relaxed) != mine;
            // Un gemelo (mismo nombre en otro uso de mayúsculas) queda en otra carpeta: la vista
            // previa no lo muestra en lugar del de la fila.
            let res = temps::global().ensure(&file, &nested, &rel, &temps::Limits { max: limit, cancel: &cancel });
            let cancelled = matches!(&res, Err(e) if e.kind == sys::ErrKind::Cancelled);
            let ok = res.is_ok_and(|p| p == path);
            TaskDone::ArchivePrepared { path, ok, cancelled }
        });
    }

    /// Llegó una extracción para la vista previa: lo que se ve de ese elemento se vuelve a pedir.
    pub(super) fn on_archive_prepared(&mut self, path: PathBuf, ok: bool, cancelled: bool) {
        self.archive.pending.remove(&path);
        if !ok {
            // Cortada porque el foco se fue: si volvió a este elemento, se pide otra vez (un error
            // de verdad no se reintenta).
            if cancelled {
                self.archive_prefetch();
            }
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

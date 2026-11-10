//! Panel de información (Alt+Enter / ⌘I): ocupa el lugar de la columna de vista previa
//! (`RightPanel::Info`) y muestra lo del panel activo, como ella: la selección; sin selección, la
//! carpeta; en «Este equipo», el disco enfocado.
//!
//! Como la vista previa, el sujeto se calcula antes de cada cuadro (`sync_info`) y se pide después
//! de una pausa corta, con una generación nueva (las respuestas viejas se descartan). El tamaño de
//! una carpeta sale al instante de xtree si ya la escaneó; si no, se suma en un hilo
//! (`info::size::folder_size`) que avisa cada 500 elementos. Editar el nombre manda `Op::Rename`
//! (con deshacer); los demás cambios van por `sys::info().apply` y su error se muestra en la
//! sección.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::preview::RightPanel;
use super::*;
use crate::info::draw::{self as idraw, Body, Header, InfoHit, Panel, Row, SectionView};
use crate::info::{Access, InfoChange, InfoReply, InfoRequest, ItemInfo, Section, Tri, WM_APP_INFO};
use crate::thumbs;

/// Título del panel (el nombre va en el encabezado).
const TITLE: &str = "Información";

/// Cuánto se espera después del último cambio de selección para pedir la información.
const INFO_DELAY_MS: u64 = 50;

/// Qué se está editando en el panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum EditKind {
    Name,
    Comment,
    Tag,
}

/// Avance de la suma de una carpeta (`count` = None: xtree da el tamaño, no la cantidad).
#[derive(Clone, Debug, PartialEq)]
struct FolderSize {
    path: PathBuf,
    bytes: u64,
    count: Option<u64>,
    /// Carpetas que no se pudieron leer.
    skipped: u64,
    done: bool,
}

/// Estado del panel en la `App`.
pub(super) struct InfoState {
    /// Secciones abiertas (bits de `Section`), se guardan en la sesión.
    pub sections: u32,
    pub(super) generation: u64,
    /// Generación con que empezó el sujeto actual: las respuestas de los cambios desde ahí valen
    /// (aunque después se haya vuelto a leer).
    since: u64,
    current: Arc<AtomicU64>,
    pub(super) tx: Sender<InfoReply>,
    rx: Receiver<InfoReply>,
    /// Rutas que se muestran y su versión (fecha y tamaño de la lista), para volver a pedir si cambian.
    subject: Vec<PathBuf>,
    stamps: Vec<(u64, u64)>,
    /// Lo que llegó del servicio para `subject` (None: todavía no).
    items: Option<Vec<ItemInfo>>,
    folder: Option<FolderSize>,
    pending: bool,
    changed_at: Instant,
    posted: bool,
    scroll: f32,
    errors: Vec<(Section, String)>,
    /// Caja de texto abierta sobre el panel.
    pub(super) edit: Option<(TextBox, EditKind)>,
    /// Cómo quedó armado el panel en el último cuadro (dibujo y clics).
    pub(super) layout: idraw::Layout,
    /// El último clic fue en el panel: Esc lo cierra.
    pub(super) focused: bool,
    /// Sumar el tamaño de las carpetas en un hilo (las pruebas lo apagan y mandan el avance a mano).
    pub(super) size_thread: bool,
    /// Generación de la suma de carpeta: cambia solo con el sujeto (releer después de un cambio
    /// no corta una suma larga).
    pub(super) size_generation: u64,
    /// Señal de corte de la suma en curso, compartida con su hilo: cambiar de sujeto (o soltar el
    /// disco) la pone en `true` y el hilo deja de abrir carpetas enseguida.
    pub(super) size_cancel: Arc<AtomicBool>,
    /// El hilo de la suma y su carpeta (expulsar espera a que suelte el disco).
    pub(super) size_worker: Option<(PathBuf, std::thread::JoinHandle<()>)>,
}

impl InfoState {
    pub(super) fn new(sections: u32) -> InfoState {
        let (tx, rx) = channel();
        InfoState {
            sections,
            generation: 0,
            since: 0,
            current: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
            subject: Vec::new(),
            stamps: Vec::new(),
            items: None,
            folder: None,
            pending: false,
            changed_at: Instant::now(),
            posted: false,
            scroll: 0.0,
            errors: Vec::new(),
            edit: None,
            layout: idraw::Layout::default(),
            focused: false,
            size_thread: true,
            size_generation: 0,
            size_cancel: Arc::new(AtomicBool::new(false)),
            size_worker: None,
        }
    }

    /// Corta la suma en curso (cambió el sujeto o se cerró el panel).
    fn bump_size(&mut self) {
        self.size_generation += 1;
        self.size_cancel.store(true, Ordering::Relaxed);
        self.size_cancel = Arc::new(AtomicBool::new(false));
    }

    fn bump(&mut self) -> u64 {
        self.generation += 1;
        self.current.store(self.generation, Ordering::Relaxed);
        self.generation
    }

    fn is_open(&self, s: Section) -> bool {
        self.sections & s.bit() != 0
    }

    fn error(&self, s: Section) -> Option<String> {
        self.errors.iter().find(|(x, _)| *x == s).map(|(_, e)| e.clone())
    }
}

/// Posición en la lista de la pestaña del elemento con esa ruta (por nombre primero, que es barato).
fn index_of(tab: &Tab, p: &Path) -> Option<usize> {
    let name = p.file_name()?;
    (0..tab.list.entries.len()).find(|&i| {
        let e = &tab.list.entries[i];
        (e.name == name || e.full.is_some()) && tab.entry_path(i) == p
    })
}

/// «…/Imágenes/Viaje»: los dos últimos tramos de una carpeta (la ruta entera si es corta).
fn short_location(dir: &Path) -> String {
    let parts: Vec<String> = dir.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.len() <= 3 {
        return dir.display().to_string();
    }
    let sep = std::path::MAIN_SEPARATOR;
    format!("…{sep}{}{sep}{}", parts[parts.len() - 2], parts[parts.len() - 1])
}

/// «12 elementos», con las carpetas que no se pudieron leer si hubo.
fn content_text(n: u64, skipped: u64) -> String {
    let base = crate::preview::draw::items_text(n);
    match skipped {
        0 => base,
        1 => format!("{base} (sin contar 1 carpeta sin permiso)"),
        k => format!("{base} (sin contar {} carpetas sin permiso)", crate::format::thousands(k)),
    }
}

/// «3,8 MB (3.963.904 bytes)».
fn size_with_bytes(env: &dyn sys::SystemEnv, bytes: u64) -> String {
    format!("{} ({} bytes)", env.format_size_short(bytes), crate::format::thousands(bytes))
}

fn tri(on: bool) -> Tri {
    if on { Tri::On } else { Tri::Off }
}

impl App {
    /// ¿El panel está abierto (en el lugar de la columna)?
    pub(super) fn info_open(&self) -> bool {
        self.preview.owner == RightPanel::Info
    }

    /// Alt+Enter / ⌘I: lo abre o, si estaba abierto, lo cierra.
    pub(super) fn toggle_info(&mut self) {
        if self.info_open() {
            self.close_info();
        } else {
            self.open_info();
        }
    }

    /// Abre el panel (o lo deja abierto) en el lugar de la columna.
    pub(super) fn open_info(&mut self) {
        if self.info_open() {
            return;
        }
        // Hay un solo panel a la vez: Renombrar varios se cierra. El dueño pasa a ser la
        // información antes de cerrarlo, así la vista previa no llega a pedir nada en el medio.
        let shown = self.preview_visible();
        self.preview.owner = RightPanel::Info;
        self.close_rename_many();
        if !shown {
            self.begin_layout_anim();
        }
        self.info.scroll = 0.0;
        self.sync_info();
        self.redraw();
    }

    /// Cierra el panel: vuelve la vista previa si estaba prendida.
    pub(super) fn close_info(&mut self) {
        if !self.info_open() {
            return;
        }
        self.end_info_edit(false);
        if !self.preview.on {
            self.begin_layout_anim();
        }
        self.preview.owner = RightPanel::Preview;
        let st = &mut self.info;
        st.bump();
        st.subject.clear();
        st.stamps.clear();
        st.items = None;
        st.folder = None;
        st.bump_size();
        st.errors.clear();
        st.pending = false;
        st.focused = false;
        st.layout = idraw::Layout::default();
        self.sync_preview();
        self.redraw();
    }

    /// Expulsar: si el panel muestra algo del disco de `root` (o suma una carpeta de ahí), lo suelta
    /// y corta la suma. Devuelve el hilo de la suma de ese disco para esperarlo antes de expulsar.
    pub(super) fn release_info_under(&mut self, root: &Path) -> Option<std::thread::JoinHandle<()>> {
        let st = &self.info;
        let under = st.subject.iter().any(|p| p.starts_with(root)) || st.folder.as_ref().is_some_and(|f| f.path.starts_with(root));
        if under {
            self.end_info_edit(false);
            let st = &mut self.info;
            st.bump();
            st.subject.clear();
            st.stamps.clear();
            st.items = None;
            st.folder = None;
            st.bump_size();
            st.errors.clear();
            st.pending = false;
            self.redraw();
        }
        self.info.size_worker.take_if(|(p, _)| p.starts_with(root)).map(|(_, h)| h)
    }

    /// Pruebas: la carpeta que se está sumando (o ya sumada) y si terminó.
    #[cfg(test)]
    pub(super) fn info_folder(&self) -> Option<(PathBuf, bool)> {
        self.info.folder.as_ref().map(|f| (f.path.clone(), f.done))
    }

    /// Pruebas: lo que muestra el panel.
    #[cfg(test)]
    pub(super) fn info_subject(&self) -> &[PathBuf] {
        &self.info.subject
    }

    /// Qué se muestra: rutas y su versión. La selección del panel activo; sin selección, la carpeta
    /// (o en «Este equipo», el disco enfocado). Las carpetas virtuales no tienen de dónde leer.
    fn info_wanted(&self) -> (Vec<PathBuf>, Vec<(u64, u64)>) {
        let tab = self.panes[self.active].tab();
        let list = &tab.list;
        if matches!(tab.location, Location::Shell { .. }) {
            return (Vec::new(), Vec::new());
        }
        let selected: Vec<usize> = list.selected_indices().collect();
        let idx = if selected.is_empty() {
            match &tab.location {
                Location::Path(p) => return (vec![p.clone()], vec![(list.entries.len() as u64, 0)]),
                _ => list.focused_index().into_iter().collect(),
            }
        } else {
            selected
        };
        let idx: Vec<usize> = idx.into_iter().filter(|&i| i < list.entries.len()).collect();
        let paths = idx.iter().map(|&i| tab.entry_path(i)).collect();
        let stamps = idx.iter().map(|&i| (list.entries[i].modified, list.entries[i].size)).collect();
        (paths, stamps)
    }

    /// Antes de cada cuadro: si cambió lo que se muestra, se vacía y se programa el pedido; si
    /// cambió solo la versión, se vuelve a pedir sin vaciar.
    pub(super) fn sync_info(&mut self) {
        if !self.info_open() || !self.preview_visible() {
            return;
        }
        let (paths, stamps) = self.info_wanted();
        if paths == self.info.subject && stamps == self.info.stamps {
            return;
        }
        if paths != self.info.subject {
            self.end_info_edit(true);
            let st = &mut self.info;
            st.items = None;
            st.folder = None;
            st.bump_size();
            st.errors.clear();
            st.scroll = 0.0;
        }
        let st = &mut self.info;
        let same = paths == st.subject;
        st.subject = paths;
        st.stamps = stamps;
        st.bump();
        if !same {
            st.since = st.generation;
        }
        if st.subject.is_empty() {
            st.pending = false;
            return;
        }
        st.pending = true;
        st.changed_at = Instant::now();
        if !st.posted {
            st.posted = true;
            post_info_due(self.hwnd, INFO_DELAY_MS);
        }
    }

    /// Pasó la pausa: si hubo otro cambio después, se espera lo que falta; si no, se pide.
    fn on_info_due(&mut self) {
        self.info.posted = false;
        if !self.info.pending {
            return;
        }
        let waited = self.info.changed_at.elapsed().as_millis() as u64;
        if waited < INFO_DELAY_MS {
            self.info.posted = true;
            post_info_due(self.hwnd, INFO_DELAY_MS - waited);
            return;
        }
        self.request_info();
    }

    fn request_info(&mut self) {
        self.info.pending = false;
        if self.info.subject.is_empty() {
            return;
        }
        let generation = self.info.bump();
        sys::info().request(InfoRequest {
            hwnd: self.hwnd,
            tx: self.info.tx.clone(),
            generation,
            current: self.info.current.clone(),
            paths: self.info.subject.clone(),
        });
    }

    /// Pruebas: pide ya lo pendiente, sin esperar la pausa.
    #[cfg(test)]
    pub(super) fn flush_info(&mut self) {
        if self.info.pending {
            self.request_info();
        }
    }

    /// `WM_APP_INFO`: la pausa (`wparam` 1) o respuestas del servicio y de la suma de carpetas.
    pub(super) fn on_info_message(&mut self, wparam: usize) {
        if wparam == 1 {
            self.on_info_due();
            return;
        }
        let mut got = false;
        while let Ok(r) = self.info.rx.try_recv() {
            match r {
                InfoReply::Items { generation, items } if generation == self.info.generation && !self.info.subject.is_empty() => {
                    self.info.items = Some(items);
                    self.start_folder_size();
                    got = true;
                }
                InfoReply::FolderSize { generation, path, bytes, files, folders, skipped, done } if generation == self.info.size_generation => {
                    if let Some(f) = self.info.folder.as_mut().filter(|f| f.path == path && !f.done) {
                        f.bytes = bytes;
                        f.count = Some(files + folders);
                        f.skipped = skipped;
                        f.done = done;
                        got = true;
                    }
                }
                InfoReply::Applied { generation, section, error } if generation >= self.info.since && !self.info.subject.is_empty() => {
                    self.info.errors.retain(|(s, _)| *s != section);
                    match error {
                        Some(e) => self.info.errors.push((section, e)),
                        // Anduvo: se vuelve a leer (sin vaciar lo que se ve).
                        None => self.request_info(),
                    }
                    got = true;
                }
                _ => {}
            }
        }
        if got {
            self.redraw();
        }
    }

    /// Una sola carpeta (no un disco ni algo solo en la nube): su tamaño de xtree o sumado en un hilo.
    fn start_folder_size(&mut self) {
        let Some([item]) = self.info.items.as_deref() else { return };
        if !item.is_dir || item.volume.is_some() || item.cloud_only {
            return;
        }
        let path = item.path.clone();
        // Ya sumada o sumándose (releer después de un cambio no la corta).
        if self.info.folder.as_ref().is_some_and(|f| f.path == path) {
            return;
        }
        if let Some(bytes) = self.xtree.as_ref().and_then(|x| x.size_of(&path)) {
            self.info.folder = Some(FolderSize { path, bytes, count: None, skipped: 0, done: true });
            return;
        }
        self.info.folder = Some(FolderSize { path: path.clone(), bytes: 0, count: Some(0), skipped: 0, done: false });
        if !self.info.size_thread {
            return;
        }
        let (hwnd, tx, generation, ctl) = (self.hwnd, self.info.tx.clone(), self.info.size_generation, self.info.size_cancel.clone());
        let at = path.clone();
        let spawned = std::thread::Builder::new().name("xdir-tamano-carpeta".into()).spawn(move || {
            let stale = || ctl.load(Ordering::Relaxed);
            let mut last = (0, 0, 0);
            let skipped = crate::info::size::folder_size(&path, &|p| sys::file_system().cloud_only(p), &ctl, &mut |b, f, fo| {
                last = (b, f, fo);
                if !stale() && tx.send(InfoReply::FolderSize { generation, path: path.clone(), bytes: b, files: f, folders: fo, skipped: 0, done: false }).is_ok() {
                    hwnd.post(WM_APP_INFO, 0, 0);
                }
            });
            if !stale() && tx.send(InfoReply::FolderSize { generation, path: path.clone(), bytes: last.0, files: last.1, folders: last.2, skipped, done: true }).is_ok() {
                hwnd.post(WM_APP_INFO, 0, 0);
            }
        });
        debug_assert!(spawned.is_ok(), "hilo del tamaño de la carpeta");
        self.info.size_worker = spawned.ok().map(|h| (at, h));
    }

    /// Lo que se ve de un elemento antes de que llegue la respuesta (o lo que falta en ella): de la
    /// lista, sin leer nada.
    fn info_items(&self) -> Option<Vec<ItemInfo>> {
        let tab = self.panes[self.active].tab();
        let list = &tab.list;
        let entry_of = |p: &Path| index_of(tab, p);
        let mut items = match &self.info.items {
            Some(items) => items.clone(),
            None if self.info.subject.len() == 1 => {
                let p = &self.info.subject[0];
                let mut it = ItemInfo { path: p.clone(), is_dir: true, ..ItemInfo::default() };
                if let Some(e) = entry_of(p).map(|i| &list.entries[i]) {
                    it.is_dir = e.is_dir;
                    it.size = if e.is_dir { 0 } else { e.size };
                    it.created = e.created;
                    it.modified = e.modified;
                }
                vec![it]
            }
            None => return None,
        };
        // El tipo de la lista si el servicio no lo dio (la lectura mínima no lo sabe).
        if items.len() == 1 && items[0].kind_name.is_empty() {
            items[0].kind_name = match entry_of(&items[0].path) {
                Some(i) => list.entries[i].type_name.to_string(),
                None if items[0].is_dir => sys::file_system().folder_type().to_string(),
                None => String::new(),
            };
        }
        Some(items)
    }

    /// ¿Se muestra la carpeta abierta (nada seleccionado)?
    fn info_is_location(&self) -> bool {
        let tab = self.panes[self.active].tab();
        tab.list.selected_indices().next().is_none() && matches!(&tab.location, Location::Path(p) if self.info.subject.first() == Some(p))
    }

    /// Lo que dibuja el panel ahora.
    pub(super) fn info_panel(&self) -> Panel {
        let env = sys::env();
        let st = &self.info;
        let tab = self.panes[self.active].tab();
        let list = &tab.list;
        let more_button = sys::info().has_more_in_system() && !st.subject.is_empty();
        let Some(items) = self.info_items() else {
            let n = st.subject.len();
            let name = if n == 0 { tab.location.title() } else { format!("{n} elementos") };
            let title = TITLE.to_string();
            let line = if n == 0 { "Sin información para este lugar" } else { "Cargando…" };
            let icon = sys::file_system().folder_icon().unwrap_or(0);
            return Panel { title, header: Header { key: None, icon, image: false, name, lines: vec![(line.to_string(), false)] }, sections: Vec::new(), more_button, eject_button: false };
        };
        let section = |s: Section, body: Body| SectionView { section: s, open: st.is_open(s), error: st.error(s), body };
        if items.len() > 1 {
            let sum = crate::info::merge(&items);
            let first_icon = list.selected_indices().next().map_or(0, |i| list.entries[i].icon);
            let mut rows = Vec::new();
            if let Some(k) = &sum.kind {
                rows.push(Row::new("Tipo", k.clone()));
            }
            rows.push(Row::new("Tamaño", size_with_bytes(env, sum.size)));
            if sum.on_disk > 0 {
                rows.push(Row::new("En disco", size_with_bytes(env, sum.on_disk)));
            }
            let all_tags = items.iter().all(|i| i.tags.is_some());
            let lines = vec![(sum.kind.clone().unwrap_or_else(|| "Varios tipos".into()), false), (env.format_size_short(sum.size), false)];
            return Panel {
                title: TITLE.to_string(),
                header: Header { key: None, icon: first_icon, image: false, name: format!("{} elementos", crate::format::thousands(sum.count as u64)), lines },
                sections: vec![
                    section(Section::Tags, Body::Tags(all_tags.then(|| sum.tags.clone()))),
                    section(Section::General, Body::General { capacity: None, rows, flags: sum.flags.clone() }),
                ],
                more_button,
                eject_button: false,
            };
        }
        let item = &items[0];
        // El elemento de la lista (nombre a la vista, ícono, metadatos de la tanda B).
        let idx = index_of(tab, &item.path);
        let entry = idx.map(|i| &list.entries[i]);
        let name = match entry {
            Some(e) => e.display.clone(),
            None if matches!(&tab.location, Location::Path(p) if *p == item.path) => tab.location.title(),
            None => item.path.file_name().map_or_else(|| item.path.display().to_string(), |n| n.to_string_lossy().into_owned()),
        };
        let icon = entry.map(|e| e.icon).or_else(|| sys::file_system().folder_icon()).unwrap_or(0);
        let key = Some((thumbs::Source::Path(item.path.clone()), 0));
        let image = !item.is_dir && !item.cloud_only && item.volume.is_none();
        let kind = item.kind_name.clone();
        let title = TITLE.to_string();
        // Disco.
        if let Some(v) = &item.volume {
            let used = v.total.saturating_sub(v.free);
            let (free_t, total_t, used_t) = (env.format_size_short(v.free), env.format_size_short(v.total), env.format_size_short(used));
            let mut lines = vec![(kind.clone(), false), (format!("{free_t} disponibles de {total_t}"), false)];
            if !v.format.is_empty() {
                lines.push((v.format.clone(), false));
            }
            let mut rows = vec![Row::new("Tipo", kind.clone())];
            if !v.format.is_empty() {
                rows.push(Row::new("Formato", v.format.clone()));
            }
            rows.push(Row::new("Capacidad", total_t.clone()));
            rows.push(Row::new("Disponible", free_t.clone()));
            rows.push(Row::new("Usado", used_t.clone()));
            let capacity = Some((used, v.free, format!("Usado: {used_t}"), format!("Disponible: {free_t}")));
            let mut sections = vec![section(Section::General, Body::General { capacity, rows, flags: Vec::new() })];
            if let Some(body) = self.perms_body(item) {
                sections.push(section(Section::Perms, body));
            }
            // ⏏ si el disco se puede expulsar (como en la fila de «Este equipo»).
            let eject_button = self.ejectable.contains(&item.path);
            return Panel { title, header: Header { key: None, icon, image: false, name, lines }, sections, more_button, eject_button };
        }
        // Encabezado.
        let mut lines = vec![(kind.clone(), false)];
        let folder = st.folder.as_ref().filter(|f| f.path == item.path);
        if item.is_dir {
            match folder {
                Some(f) if !f.done => {
                    lines.push((format!("Calculando… {}", env.format_size_short(f.bytes)), true));
                    lines.push((crate::preview::draw::items_text(f.count.unwrap_or(0)), false));
                }
                Some(f) => {
                    lines.push((env.format_size_short(f.bytes), false));
                    if let Some(n) = f.count {
                        lines.push((crate::preview::draw::items_text(n), false));
                    }
                }
                None if item.cloud_only => lines.push((idraw::CLOUD_TEXT.to_string(), false)),
                None => {}
            }
        } else {
            lines.push((env.format_size_short(item.size), false));
            if item.modified != 0 {
                lines.push((format!("Modificado: {}", env.format_date(item.modified)), false));
            }
        }
        // General.
        let mut rows = vec![Row::new("Tipo", kind.clone())];
        if item.is_dir {
            match folder {
                Some(f) if !f.done => {
                    rows.push(Row { spinner: true, ..Row::new("Tamaño", format!("Calculando… {}", env.format_size_short(f.bytes))) });
                    rows.push(Row::new("Contenido", content_text(f.count.unwrap_or(0), f.skipped)));
                }
                Some(f) => {
                    rows.push(Row::new("Tamaño", size_with_bytes(env, f.bytes)));
                    if let Some(n) = f.count {
                        rows.push(Row::new("Contenido", content_text(n, f.skipped)));
                    }
                }
                None => {}
            }
        } else {
            rows.push(Row::new("Tamaño", size_with_bytes(env, item.size)));
            if item.on_disk > 0 {
                rows.push(Row::new("En disco", size_with_bytes(env, item.on_disk)));
            }
        }
        if let Some(parent) = item.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            rows.push(Row { link: true, ..Row::new("Ubicación", short_location(parent)) });
        }
        for (label, ft) in [("Creado", item.created), ("Modificado", item.modified)] {
            if ft != 0 {
                rows.push(Row::new(label, env.format_date(ft)));
            }
        }
        if item.cloud_only {
            rows.push(Row::new("Estado", idraw::CLOUD_TEXT));
        }
        let flags = item.flags.iter().map(|&(f, on, editable)| (f, tri(on), editable)).collect();
        // Más información: los metadatos de la lista, los propios del sistema y la última apertura.
        let mut more = Vec::new();
        if let Some(i) = idx {
            let meta = list.meta_of(i);
            if let Some(e) = entry {
                more.extend(crate::preview::detail_rows(e, meta, 0, env).into_iter().filter(|(l, _)| !matches!(*l, "Tipo" | "Tamaño" | "Creado" | "Modificado")).map(|(l, v)| Row::new(l, v)));
            }
        }
        more.extend(item.more.iter().map(|(l, v)| Row::new(*l, v.clone())));
        let mut note = None;
        if item.last_opened != 0 {
            let label = if item.last_opened_is_access { "Último acceso" } else { "Abierto" };
            more.push(Row::new(label, env.format_date(item.last_opened)));
            if item.last_opened_is_access {
                note = Some("El sistema puede no actualizar el último acceso.".to_string());
            }
        }
        let file_name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut sections = vec![
            section(Section::Tags, Body::Tags(item.tags.clone())),
            section(Section::General, Body::General { capacity: None, rows, flags }),
            section(Section::More, Body::Rows { rows: more, note }),
        ];
        // La carpeta abierta (sin selección) no se renombra desde acá.
        if !self.info_is_location() {
            sections.push(section(Section::Name, Body::Name { name: file_name, hide_ext: item.hidden_ext }));
        }
        sections.push(section(Section::Comments, Body::Comment(item.comment.clone())));
        if let Some(ow) = item.open_with.as_ref().filter(|_| !item.is_dir) {
            let note = if ow.per_file {
                "Abrir todos los documentos como este con esta app."
            } else {
                "Elegir una app lo abre con ella; «Cambiar todos…» cambia la predeterminada."
            };
            let app = ow.current.as_ref().map(|a| format!("{} (predeterminada)", a.name));
            let icon = ow.current.as_ref().map_or(0, |a| a.icon);
            sections.push(section(Section::OpenWith, Body::OpenWith { app, icon, note: note.to_string() }));
        }
        if let Some(body) = self.perms_body(item) {
            sections.push(section(Section::Perms, body));
        }
        Panel { title, header: Header { key, icon, image, name, lines }, sections, more_button, eject_button: false }
    }

    fn perms_body(&self, item: &ItemInfo) -> Option<Body> {
        let p = item.perms.as_ref()?;
        Some(if p.editable {
            let rows = vec![
                (p.owner.clone(), crate::info::access_of(p.mode, 0).label().to_string()),
                (p.group.clone(), crate::info::access_of(p.mode, 1).label().to_string()),
                ("Todos".to_string(), crate::info::access_of(p.mode, 2).label().to_string()),
            ];
            Body::Perms { rows, editable: true, note: None }
        } else {
            // Sin grupo ni modo es Windows (ACL): el dueño y lo que puede el usuario actual. En
            // Unix, la tabla de siempre, solo para ver (otro dueño, un disco o un enlace).
            if p.group.is_empty() && p.mode == 0 {
                let rows = vec![(p.owner.clone(), "Dueño".to_string()), ("Tu usuario".to_string(), crate::info::rw_text(p.can_read, p.can_write).to_string())];
                let note = sys::info().has_more_in_system().then(|| "Los permisos detallados están en «Más en el sistema…».".to_string());
                Body::Perms { rows, editable: false, note }
            } else {
                let rows = vec![
                    (p.owner.clone(), crate::info::access_of(p.mode, 0).label().to_string()),
                    (p.group.clone(), crate::info::access_of(p.mode, 1).label().to_string()),
                    ("Todos".to_string(), crate::info::access_of(p.mode, 2).label().to_string()),
                ];
                Body::Perms { rows, editable: false, note: Some("Solo para ver: no se pueden cambiar acá.".to_string()) }
            }
        })
    }

    /// Arma el panel para este cuadro en `r` (lo dibuja `paint` y lo usan los clics).
    pub(super) fn layout_info(&mut self, r: geom::Rect) {
        let panel = self.info_panel();
        let host = self.host.clone();
        let l = idraw::layout(r, &panel, self.info.scroll, &|s| host.text_width(s, false));
        self.info.scroll = l.scroll;
        self.info.layout = l;
    }

    /// Qué hay del panel en `(x, y)`.
    pub(super) fn info_hit(&self, x: f32, y: f32) -> Option<InfoHit> {
        if !self.info_open() || !self.preview_visible() {
            return None;
        }
        idraw::hit(&self.info.layout, x, y)
    }

    /// La rueda sobre el panel lo desplaza.
    pub(super) fn info_wheel(&mut self, delta: f32) {
        self.end_info_edit(true);
        let max = self.info.layout.max_scroll();
        self.info.scroll = (self.info.scroll - delta * idraw::ROW_H).clamp(0.0, max);
        self.redraw();
    }

    fn apply_info(&mut self, change: InfoChange) {
        let section = change.section();
        self.info.errors.retain(|(s, _)| *s != section);
        sys::info().apply(self.hwnd, self.info.tx.clone(), self.info.generation, change);
    }

    /// Un clic en el panel.
    pub(super) fn info_click(&mut self, h: InfoHit) {
        let items = self.info_items().unwrap_or_default();
        let paths = self.info.subject.clone();
        let single = (items.len() == 1).then(|| items[0].clone());
        match h {
            InfoHit::Close => self.close_info(),
            InfoHit::Toggle(s) => {
                self.end_info_edit(true);
                self.info.sections ^= s.bit();
            }
            InfoHit::TagRemove(i) => {
                let tags = if items.len() > 1 { crate::info::merge(&items).tags } else { single.as_ref().and_then(|s| s.tags.clone()).unwrap_or_default() };
                if let Some(t) = tags.get(i) {
                    self.apply_info(InfoChange::Tags { paths, add: Vec::new(), remove: vec![t.name.clone()] });
                }
            }
            InfoHit::TagAdd => self.begin_info_edit(EditKind::Tag, InfoHit::TagAdd, ""),
            InfoHit::Flag(f) => {
                let state = if items.len() > 1 {
                    crate::info::merge(&items).flags.iter().find(|x| x.0 == f).map(|x| x.1)
                } else {
                    single.as_ref().and_then(|s| s.flags.iter().find(|x| x.0 == f)).map(|x| tri(x.1))
                };
                if let Some(state) = state {
                    self.apply_info(InfoChange::Flag { paths, flag: f, on: state != Tri::On });
                }
            }
            InfoHit::HideExt => {
                if let Some(s) = single.filter(|s| s.hidden_ext.is_some()) {
                    self.apply_info(InfoChange::HideExt { path: s.path.clone(), on: s.hidden_ext != Some(true) });
                }
            }
            InfoHit::Location => {
                if let Some(s) = single
                    && let (Some(parent), Some(name)) = (s.path.parent(), s.path.file_name())
                {
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    let tab = self.panes[self.active].tab_mut();
                    tab.focus_after_load(name.to_os_string());
                    tab.navigate(hwnd, &tx, Location::Path(parent.to_path_buf()));
                }
            }
            InfoHit::NameField => {
                if let Some(s) = single {
                    let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.begin_info_edit(EditKind::Name, InfoHit::NameField, &name);
                    // Seleccionado el nombre sin la extensión, como al renombrar en la lista.
                    if let Some((b, _)) = self.info.edit {
                        let stem = match name.rfind('.') {
                            Some(i) if i > 0 && !s.is_dir => name[..i].encode_utf16().count() as isize,
                            _ => -1,
                        };
                        self.host.text_select(b, 0, stem);
                    }
                }
            }
            InfoHit::CommentField => {
                if let Some(c) = single.and_then(|s| s.comment) {
                    self.begin_info_edit(EditKind::Comment, InfoHit::CommentField, &c);
                }
            }
            InfoHit::OpenWithMenu => {
                let Some(ow) = single.and_then(|s| s.open_with) else { return };
                let entries: Vec<shell::Entry> = ow
                    .apps
                    .iter()
                    .enumerate()
                    .map(|(i, a)| shell::Entry::Item { id: i as u32, label: a.name.clone(), enabled: true, checked: ow.current.as_ref().is_some_and(|c| c.id == a.id) })
                    .collect();
                if entries.is_empty() {
                    return;
                }
                let at = self.info.layout.hits.iter().find(|(x, _)| *x == InfoHit::OpenWithMenu).map_or(MenuPos::Cursor, |(_, r)| MenuPos::At(r.left, r.bottom));
                if let Some(app) = self.host.popup_menu(&entries, at, None, None).and_then(|i| ow.apps.get(i as usize)) {
                    self.apply_info(InfoChange::OpenWith { paths, app: app.id.clone() });
                }
            }
            InfoHit::ChangeAll => {
                let Some(s) = single else { return };
                let Some(ow) = s.open_with.clone() else { return };
                // Windows: su diálogo elige la app (y ya pregunta); acá no se pregunta nada.
                if ow.system_dialog {
                    self.apply_info(InfoChange::ChangeAll { path: s.path.clone(), app: String::new() });
                    self.redraw();
                    return;
                }
                // macOS: la app elegida para este archivo pasa a ser la de todos. Linux (no hay app
                // por archivo): se elige en un menú.
                let app = if ow.per_file {
                    ow.current.clone()
                } else {
                    let entries: Vec<shell::Entry> = ow
                        .apps
                        .iter()
                        .enumerate()
                        .map(|(i, a)| shell::Entry::Item { id: i as u32, label: a.name.clone(), enabled: true, checked: ow.current.as_ref().is_some_and(|c| c.id == a.id) })
                        .collect();
                    let at = self.info.layout.hits.iter().find(|(x, _)| *x == InfoHit::ChangeAll).map_or(MenuPos::Cursor, |(_, r)| MenuPos::At(r.left, r.bottom));
                    self.host.popup_menu(&entries, at, None, None).and_then(|i| ow.apps.get(i as usize).cloned())
                };
                let Some(app) = app else { return };
                let text = format!("¿Usar «{}» para abrir todos los documentos como este?", app.name);
                if self.host.ask_yes_no("Cambiar todos", &text) {
                    self.apply_info(InfoChange::ChangeAll { path: s.path.clone(), app: app.id });
                }
            }
            InfoHit::Perm(who) => {
                let Some(s) = single else { return };
                let Some(p) = s.perms.as_ref().filter(|p| p.editable) else { return };
                let now = crate::info::access_of(p.mode, who);
                let entries: Vec<shell::Entry> = Access::CHOICES
                    .iter()
                    .enumerate()
                    .map(|(i, a)| shell::Entry::Item { id: i as u32, label: a.label().to_string(), enabled: true, checked: *a == now })
                    .collect();
                let at = self.info.layout.hits.iter().find(|(x, _)| *x == InfoHit::Perm(who)).map_or(MenuPos::Cursor, |(_, r)| MenuPos::At(r.left, r.bottom));
                let Some(access) = self.host.popup_menu(&entries, at, None, None).and_then(|i| Access::CHOICES.get(i as usize).copied()) else { return };
                if access == now {
                    return;
                }
                let mode = crate::info::with_access(p.mode, who, access);
                let name = s.path.file_name().map_or_else(|| s.path.display().to_string(), |n| n.to_string_lossy().into_owned());
                // Cambiar permisos se confirma; en una carpeta, después, si va para lo de adentro.
                if !self.host.ask_yes_no("Compartir y permisos", &format!("¿Cambiar los permisos de «{name}» a «{}»?", access.label())) {
                    return;
                }
                let recursive = s.is_dir && self.host.ask_yes_no("Compartir y permisos", &format!("¿Aplicar también a los elementos incluidos en «{name}»?"));
                self.apply_info(InfoChange::Perms { paths, mode, recursive });
            }
            InfoHit::MoreInSystem => {
                let items: Vec<shell::Item> = paths.into_iter().map(shell::Item::from).collect();
                sys::info().more_in_system(self.hwnd, &items);
            }
            InfoHit::Eject => {
                if let [root] = &paths[..]
                    && self.ejectable.contains(root)
                {
                    self.eject(root.clone());
                }
            }
            InfoHit::Body => {}
        }
        self.redraw();
    }

    /// Monta una caja de texto del host sobre el campo `at` del panel.
    fn begin_info_edit(&mut self, kind: EditKind, at: InfoHit, text: &str) {
        self.end_info_edit(true);
        let Some(&(_, r)) = self.info.layout.fields.iter().find(|(h, _)| *h == at) else { return };
        let view = self.info.layout.view;
        if r.top < view.top || r.bottom > view.bottom {
            return;
        }
        if let Some(b) = self.host.text_create(TextKind::Info, r, text, &self.theme) {
            self.host.text_focus(b);
            self.host.text_select(b, 0, -1);
            self.info.edit = Some((b, kind));
        }
    }

    /// Terminó la edición (`WM_APP_INFO_EDIT_DONE`, otro clic, desplazar o cerrar): con `commit`,
    /// se aplica lo escrito.
    pub(super) fn end_info_edit(&mut self, commit: bool) {
        let Some((b, kind)) = self.info.edit.take() else { return };
        let text = self.host.text_get(b);
        self.host.text_destroy(b);
        self.host.focus_main();
        if !commit {
            self.redraw();
            return;
        }
        let items = self.info_items().unwrap_or_default();
        let paths = self.info.subject.clone();
        match kind {
            EditKind::Name => {
                let [item] = items.as_slice() else { return };
                let new_name = text.trim().to_string();
                let old = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !new_name.is_empty() && new_name != old {
                    sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), Op::Rename { path: item.path.clone(), new_name });
                }
            }
            EditKind::Comment => {
                let [item] = items.as_slice() else { return };
                if item.comment.as_deref() != Some(text.as_str()) {
                    self.apply_info(InfoChange::Comment { path: item.path.clone(), text });
                }
            }
            EditKind::Tag => {
                let tag = text.trim().to_string();
                if !tag.is_empty() {
                    self.apply_info(InfoChange::Tags { paths, add: vec![tag], remove: Vec::new() });
                }
            }
        }
        self.redraw();
    }
}

impl App {
    /// Pruebas (`info <qué> [n]`): un clic en el panel de información sin buscar coordenadas:
    /// `oculto`, `bloqueado`, `comentario`, `nombre`, `etiqueta`, `cerrar`, `mas`, `expulsar`,
    /// `seccion <n>` (plegar o abrir la sección n) o `bajar <px>` (desplazar).
    #[cfg(feature = "pruebas")]
    pub(super) fn info_test(&mut self, what: &str, n: usize) {
        let h = match what {
            "oculto" => InfoHit::Flag(crate::info::Flag::Hidden),
            "bloqueado" => InfoHit::Flag(crate::info::Flag::Locked),
            "comentario" => InfoHit::CommentField,
            "nombre" => InfoHit::NameField,
            "etiqueta" => InfoHit::TagAdd,
            "cerrar" => InfoHit::Close,
            "mas" => InfoHit::MoreInSystem,
            "expulsar" => InfoHit::Eject,
            "seccion" => match Section::ALL.get(n) {
                Some(&s) => InfoHit::Toggle(s),
                None => return,
            },
            "bajar" => {
                self.info.scroll += n as f32;
                self.redraw();
                return;
            }
            _ => return,
        };
        self.info_click(h);
    }
}

/// Manda `WM_APP_INFO` (pausa cumplida) a la ventana dentro de `ms` milisegundos.
fn post_info_due(hwnd: Handle, ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        hwnd.post(WM_APP_INFO, 1, 0);
    });
}

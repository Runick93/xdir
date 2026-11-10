//! Una pestaña: ubicación, historial atrás/adelante, la lista de archivos, su carga y su vigilancia.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::fs::{self, Entry, LoadEvent, LoadMsg, Location, WM_APP_CHANGED};
use crate::folder_views::{self, FolderViews, ViewSettings};
use crate::list::{self, Column, ColumnSpec, FileList};
use crate::sys::{self, Handle, Watch};

/// Búsqueda en la carpeta de la pestaña y todas sus subcarpetas (en curso o terminada).
pub struct Search {
    /// Lo que se escribió en el buscador.
    pub query: String,
    /// Elementos revisados hasta ahora.
    pub scanned: u64,
    /// Fracción estimada del recorrido (0 a 1).
    pub progress: f32,
    /// Terminó de recorrer todo.
    pub done: bool,
    /// Cuándo empezó y cuándo terminó (para mostrar cuánto tardó).
    pub started: Instant,
    pub finished: Option<Instant>,
    /// Carpeta virtual (Papelera, Acceso rápido...): no se recorre; se filtra por nombre lo que
    /// la carpeta muestra (`FileList::set_name_filter`), sin columna «Carpeta» ni vista propia.
    pub in_place: bool,
}

pub struct Tab {
    pub id: usize,
    pub location: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
    pub list: FileList,
    pub loading: bool,
    pub error: Option<String>,
    /// Generación de la carga en curso; las cargas viejas se descartan.
    generation: Arc<AtomicU64>,
    /// Identifica la navegación actual para descartar avisos de vigilancia viejos.
    nav_id: u64,
    /// Recarga silenciosa: se junta todo y se reemplaza al final, sin parpadeo.
    reload_buf: Option<Vec<Entry>>,
    /// Elemento a enfocar cuando termine la carga (al subir, la carpeta de la que se vino).
    focus_after_load: Option<OsString>,
    /// Elementos a seleccionar al terminar la carga en curso (el foco va al último).
    select_after_load: Vec<OsString>,
    watcher: Option<Box<dyn Watch>>,
    /// Solo las pestañas a la vista vigilan su carpeta (un hilo por pestaña no escala).
    visible: bool,
    /// La carga en curso es una visita (navegar, atrás, adelante, subir), no una recarga.
    counts_as_visit: bool,
    /// Navegación en la que ya se pidieron credenciales de red (para no insistir).
    auth_nav: u64,
    /// Carpeta que se terminó de abrir como visita: la app la anota en el historial.
    pub visited: Option<PathBuf>,
    /// Mientras hay una búsqueda, la lista muestra sus resultados en vez de la carpeta.
    pub search: Option<Search>,
    /// Aviso de la última carga (un archivo comprimido con más entradas de las que se listan).
    pub load_notice: Option<String>,
    /// Vista de la carpeta que tiene la lista: su clave y cómo era al aplicarla o al guardarla
    /// por última vez (`None`: falta aplicar la de la ubicación actual, ver `sync_view`).
    view_applied: Option<(String, ViewSettings)>,
    /// Lo que el usuario había cambiado en la vista de la ubicación que se acaba de dejar,
    /// pendiente de guardar en `sync_view`.
    left_view: Option<LeftView>,
}

/// Vista cambiada de una ubicación que se dejó: su clave, cómo quedó, los filtros con los que se
/// había aplicado y, si era una búsqueda, la clave de la carpeta buscada (dueña de los filtros).
struct LeftView {
    key: String,
    view: ViewSettings,
    applied_filters: Vec<(Column, Vec<String>)>,
    folder: Option<String>,
}

/// Guarda la vista `v` de la ubicación `key`. En una búsqueda (`folder` con la clave de la
/// carpeta buscada) los filtros por columna son de la carpeta: si cambiaron respecto de
/// `applied_filters` se guardan en ella, y la entrada de la búsqueda se queda sin filtros.
fn store_view(views: &mut FolderViews, key: &str, v: ViewSettings, applied_filters: &[(Column, Vec<String>)], folder: Option<&str>) {
    let Some(folder) = folder else {
        views.set(key, v);
        return;
    };
    if v.filters != applied_filters {
        let mut fv = views.get(folder);
        fv.filters = v.filters.clone();
        views.set(folder, fv);
    }
    views.set(key, ViewSettings { filters: Vec::new(), ..v });
}

impl Tab {
    /// Si la pestaña está a la vista (carga y vigila su carpeta). Lo usan las pruebas de `App`
    /// (app/tests.rs).
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Muestra u oculta la pestaña. Al mostrarse por primera vez carga su carpeta; al volver
    /// a mostrarse, recarga en silencio por si cambió mientras no se veía.
    pub fn set_visible(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if !visible {
            self.watcher = None;
            return;
        }
        if self.nav_id == 0 {
            self.start(hwnd, tx);
            return;
        }
        if let Location::Path(p) = &self.location {
            self.watcher = sys::file_system().watch(hwnd, WM_APP_CHANGED, self.id, self.nav_id as isize, p);
        }
        if !self.loading {
            self.reload(hwnd, tx, true);
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.nav_id != 0
    }

    pub fn new(id: usize, location: Location, show_hidden: bool) -> Self {
        let location = match location {
            Location::Path(p) => Location::Path(sys::file_system().long_path(&p)),
            other => other,
        };
        let mut list = FileList::default();
        list.show_hidden = show_hidden;
        Self {
            id,
            location,
            back: Vec::new(),
            forward: Vec::new(),
            list,
            loading: false,
            error: None,
            generation: Arc::new(AtomicU64::new(0)),
            nav_id: 0,
            reload_buf: None,
            focus_after_load: None,
            select_after_load: Vec::new(),
            watcher: None,
            visible: false,
            counts_as_visit: false,
            auth_nav: 0,
            visited: None,
            search: None,
            load_notice: None,
            view_applied: None,
            left_view: None,
        }
    }

    /// Clave de la vista de lo que muestra la pestaña: la carpeta, la ubicación especial o, con
    /// una búsqueda, la carpeta buscada (ver `folder_views::folder_key`).
    pub fn view_key(&self) -> String {
        match (self.recursive_search(), self.list.search_root.as_deref()) {
            (Some(_), Some(root)) => folder_views::folder_key(&self.location, Some(root)),
            (Some(_), None) => folder_views::computer_search_key(),
            (None, _) => folder_views::folder_key(&self.location, None),
        }
    }

    /// La vista guardada, como se ve en esta ubicación: con una búsqueda, con la columna de la
    /// carpeta de cada resultado (después del nombre si no la tenía) y los filtros por columna de
    /// la carpeta buscada (el filtro rápido se aplica sobre lo que dejan pasar); sin búsqueda, sin
    /// esa columna.
    fn view_here(&self, mut v: ViewSettings) -> ViewSettings {
        let searching = self.recursive_search().is_some();
        if searching {
            v.filters = self.list.filters.clone();
        }
        let folder = v.columns.iter().position(|c| c.col == Column::Folder);
        match (searching, folder) {
            (true, None) => {
                let at = v.columns.iter().position(|c| c.col == Column::Name).map_or(0, |i| i + 1);
                v.columns.insert(at, ColumnSpec { col: Column::Folder, width: list::COLUMNS[7].3 });
            }
            (false, Some(i)) => {
                v.columns.remove(i);
            }
            _ => {}
        }
        v
    }

    /// La vista de la lista y la de su carpeta van juntas: al llegar a otra ubicación la lista
    /// toma la vista guardada de esa carpeta (o la por defecto); si el usuario cambió algo
    /// (vista, orden, columnas, anchos), se guarda para la carpeta actual. Lo llama la `App`
    /// antes de cada cuadro y antes de entregar cada lote de la carga, así la lista ya está con
    /// su vista cuando llegan los elementos. Devuelve si cambió algo de `views`.
    pub fn sync_view(&mut self, views: &mut FolderViews) -> bool {
        let mut changed = false;
        if let Some(left) = self.left_view.take() {
            store_view(views, &left.key, left.view, &left.applied_filters, left.folder.as_deref());
            changed = true;
        }
        let key = self.view_key();
        let current = self.list.view_settings();
        match &self.view_applied {
            Some((k, applied)) if *k == key => {
                if current != *applied {
                    let folder = self.search_folder_key();
                    store_view(views, &key, current.clone(), &applied.filters, folder.as_deref());
                    self.view_applied = Some((key, current));
                    changed = true;
                }
            }
            _ => {
                let v = self.view_here(views.get(&key));
                self.list.apply_view_settings(&v);
                views.touch(&key);
                self.view_applied = Some((key, self.list.view_settings()));
            }
        }
        changed
    }

    /// La vista guardada de esta ubicación cambió por otro lado («Usar esta vista en todas las
    /// carpetas», «Restablecer la vista»): la próxima `sync_view` la vuelve a aplicar.
    pub fn reload_view(&mut self) {
        self.view_applied = None;
        self.left_view = None;
    }

    /// Se deja la ubicación actual: si el usuario le cambió la vista, queda para guardar.
    fn leave_view(&mut self) {
        if let Some((key, applied)) = self.view_applied.take() {
            let current = self.list.view_settings();
            if current != applied {
                self.left_view = Some(LeftView { key, view: current, applied_filters: applied.filters, folder: self.search_folder_key() });
            }
        }
    }

    /// Con una búsqueda, la clave de la carpeta buscada: los filtros por columna son de ella.
    fn search_folder_key(&self) -> Option<String> {
        self.recursive_search().map(|_| folder_views::folder_key(&self.location, None))
    }

    /// La búsqueda en curso si recorre subcarpetas (con sus resultados en lugar de la carpeta);
    /// la de una carpeta virtual solo filtra lo que la carpeta muestra.
    fn recursive_search(&self) -> Option<&Search> {
        self.search.as_ref().filter(|s| !s.in_place)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn can_up(&self) -> bool {
        self.location.parent().is_some()
    }

    pub fn navigate(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, to: Location) {
        // Rutas cortas 8.3 (LEANDR~1) a su forma larga, para mostrarlas y compararlas bien.
        let to = match to {
            Location::Path(p) => Location::Path(sys::file_system().long_path(&p)),
            other => other,
        };
        if to == self.location && self.nav_id != 0 {
            self.reload(hwnd, tx, false);
            return;
        }
        let from = std::mem::replace(&mut self.location, to);
        if self.nav_id != 0 {
            self.back.push(from);
        }
        self.forward.clear();
        self.start(hwnd, tx);
        self.counts_as_visit = true;
    }

    pub fn go_back(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        self.go_back_n(1, hwnd, tx);
    }

    pub fn go_forward(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        self.go_forward_n(1, hwnd, tx);
    }

    /// Carpetas a las que lleva Atrás, la más cercana primero (como mucho `max`).
    pub fn back_list(&self, max: usize) -> Vec<Location> {
        self.back.iter().rev().take(max).cloned().collect()
    }

    /// Carpetas a las que lleva Adelante, la más cercana primero (como mucho `max`).
    pub fn forward_list(&self, max: usize) -> Vec<Location> {
        self.forward.iter().rev().take(max).cloned().collect()
    }

    /// `n` veces Atrás (las salteadas quedan en Adelante), leyendo la carpeta una sola vez.
    pub fn go_back_n(&mut self, n: usize, hwnd: Handle, tx: &Sender<LoadEvent>) {
        let n = n.min(self.back.len());
        if n == 0 {
            return;
        }
        for _ in 0..n {
            let prev = self.back.pop().expect("hay n lugares atrás");
            let from = std::mem::replace(&mut self.location, prev);
            self.forward.push(from);
        }
        // Al volver a una carpeta superior se enfoca la subcarpeta por la que se había entrado.
        self.focus_after_load = self.forward.last().and_then(|next| child_name(&self.location, next));
        self.start(hwnd, tx);
        self.counts_as_visit = true;
    }

    /// `n` veces Adelante (las salteadas quedan en Atrás), leyendo la carpeta una sola vez.
    pub fn go_forward_n(&mut self, n: usize, hwnd: Handle, tx: &Sender<LoadEvent>) {
        let n = n.min(self.forward.len());
        if n == 0 {
            return;
        }
        for _ in 0..n {
            let next = self.forward.pop().expect("hay n lugares adelante");
            let from = std::mem::replace(&mut self.location, next);
            self.back.push(from);
        }
        self.start(hwnd, tx);
        self.counts_as_visit = true;
    }

    pub fn go_up(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        if let Some(parent) = self.location.parent() {
            let child = child_name(&parent, &self.location);
            self.navigate(hwnd, tx, parent);
            self.focus_after_load = child;
        }
    }

    /// Vuelve a leer la carpeta. `silent`: sin vaciar la lista mientras tanto.
    pub fn reload(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, silent: bool) {
        // Con una búsqueda a la vista: recargar es repetirla; las recargas silenciosas (cambios
        // en la carpeta) no la interrumpen.
        if let Some(query) = self.recursive_search().map(|s| s.query.clone()) {
            if !silent {
                self.search(hwnd, tx, &query);
            }
            return;
        }
        // En una carpeta virtual se recarga la carpeta y el filtro sigue (en la recarga silenciosa
        // la lista lo conserva; la otra la vacía y se vuelve a poner).
        let in_place = self.search.as_ref().map(|s| s.query.clone());
        if silent {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            // Los lotes de metadatos en camino traen la generación vieja y se descartan: lo que no
            // llegó se vuelve a pedir (si la recarga falla, la lista sigue igual y sin «Leyendo…» colgado).
            self.list.forget_unanswered_meta();
            self.reload_buf = Some(Vec::new());
            self.spawn_load(hwnd, tx, generation);
        } else {
            self.focus_after_load = self.list.focused_index().map(|i| self.list.entries[i].name.clone());
            self.start(hwnd, tx);
            if let Some(query) = in_place {
                self.search(hwnd, tx, &query);
            }
        }
    }

    /// Al terminar la carga en curso deja seleccionados `names` y el foco en el último.
    pub fn select_after_load(&mut self, names: Vec<OsString>) {
        self.select_after_load = names;
    }

    /// ¿Hay una recarga silenciosa en curso (la lista sigue mostrando lo de antes)?
    pub fn reload_pending(&self) -> bool {
        self.reload_buf.is_some()
    }

    /// Enfoca `name` cuando termine la carga en curso.
    pub fn focus_after_load(&mut self, name: OsString) {
        self.focus_after_load = Some(name);
    }

    /// Busca `query` en la carpeta de la pestaña y todas sus subcarpetas ("Este equipo": en
    /// todas las unidades). Los resultados van apareciendo en la lista. En una carpeta virtual
    /// (Papelera...) no se recorre nada: se filtra por nombre lo que muestra. Con el texto vacío,
    /// termina la búsqueda y vuelve a la carpeta.
    pub fn search(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, query: &str) {
        if query.trim().is_empty() {
            self.end_search(hwnd, tx);
            return;
        }
        let (roots, root) = match &self.location {
            Location::Path(p) => (vec![p.clone()], Some(p.clone())),
            Location::Computer => (sys::file_system().drives(), None),
            // Las carpetas virtuales y las de un archivo comprimido no se recorren: se filtra lo
            // que muestran.
            Location::Shell { .. } | Location::Archive { .. } => {
                let now = Instant::now();
                self.search = Some(Search { query: query.to_string(), scanned: 0, progress: 1.0, done: true, started: now, finished: Some(now), in_place: true });
                self.list.set_name_filter(Some(query));
                return;
            }
        };
        self.leave_view();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.list.clear();
        // Los resultados llevan su ruta completa: de ahí se leen los metadatos.
        self.list.meta_enabled = true;
        self.list.search_root = root;
        if !self.list.columns.iter().any(|c| c.col == Column::Folder) {
            let at = self.list.columns.iter().position(|c| c.col == Column::Name).map_or(0, |i| i + 1);
            self.list.columns.insert(at, ColumnSpec { col: Column::Folder, width: list::COLUMNS[7].3 });
        }
        self.loading = true;
        self.error = None;
        self.reload_buf = None;
        self.focus_after_load = None;
        self.counts_as_visit = false;
        self.search = Some(Search { query: query.to_string(), scanned: 0, progress: 0.0, done: false, started: Instant::now(), finished: None, in_place: false });
        fs::spawn_search(hwnd, tx.clone(), self.id, generation, self.generation.clone(), roots, self.list.show_hidden, list::name_matcher(query));
    }

    /// Termina la búsqueda (si había) y vuelve a mostrar la carpeta.
    pub fn end_search(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        match &self.search {
            // La carpeta virtual ya está en la lista: alcanza con quitar el filtro.
            Some(s) if s.in_place => self.drop_search(),
            Some(_) => self.start(hwnd, tx),
            None => {}
        }
    }

    /// Lo escrito en el buscador, si hay una búsqueda.
    pub fn search_query(&self) -> &str {
        self.search.as_ref().map_or("", |s| s.query.as_str())
    }

    /// Saca de la lista lo propio de la búsqueda (la columna de carpeta).
    fn drop_search(&mut self) {
        self.search = None;
        self.list.set_name_filter(None);
        self.list.search_root = None;
        self.list.columns.retain(|c| c.col != Column::Folder);
    }

    fn start(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        self.leave_view();
        self.drop_search();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.nav_id += 1;
        // Quien navega la vuelve a marcar como visita después; una recarga no cuenta.
        self.counts_as_visit = false;
        self.list.clear();
        self.load_notice = None;
        // Solo las carpetas de disco tienen de dónde leer metadatos.
        self.list.meta_enabled = matches!(self.location, Location::Path(_));
        self.loading = true;
        self.error = None;
        self.reload_buf = None;
        self.watcher = match &self.location {
            Location::Path(p) if self.visible => sys::file_system().watch(hwnd, WM_APP_CHANGED, self.id, self.nav_id as isize, p),
            _ => None,
        };
        self.spawn_load(hwnd, tx, generation);
    }

    /// Lanza la lectura de la ubicación: la del sistema, o la del núcleo dentro de un archivo
    /// comprimido (en los dos casos en otro hilo).
    fn spawn_load(&self, hwnd: Handle, tx: &Sender<LoadEvent>, generation: u64) {
        let (id, current, location) = (self.id, self.generation.clone(), self.location.clone());
        if location.is_archive() {
            crate::archive_nav::spawn_load(hwnd, tx.clone(), id, generation, current, location);
        } else {
            sys::file_system().spawn_load(hwnd, tx.clone(), id, generation, current, location);
        }
    }

    /// Aviso del vigilante: recarga silenciosa si sigue siendo la misma navegación.
    pub fn on_changed(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, nav_id: u64) {
        if nav_id == self.nav_id && !self.loading && self.recursive_search().is_none() {
            self.reload(hwnd, tx, true);
        }
    }

    /// Aplica un mensaje de carga. Devuelve si hay que redibujar.
    pub fn on_load(&mut self, event: LoadEvent) -> bool {
        if event.generation != self.generation.load(Ordering::Relaxed) {
            return false;
        }
        match event.msg {
            LoadMsg::Entries(batch) => match &mut self.reload_buf {
                Some(buf) => {
                    buf.extend(batch);
                    return false;
                }
                None => self.list.append(batch),
            },
            LoadMsg::Notice(text) => self.load_notice = Some(text),
            LoadMsg::Progress { scanned, done } => {
                if let Some(s) = &mut self.search {
                    s.scanned = scanned;
                    s.progress = done.clamp(0.0, 1.0);
                }
            }
            LoadMsg::Listed(result) => {
                self.loading = false;
                if let Some(s) = &mut self.search {
                    s.done = true;
                    s.progress = 1.0;
                    s.finished = Some(Instant::now());
                }
                if let Some(buf) = self.reload_buf.take() {
                    if result.is_ok() {
                        self.list.replace(buf);
                    }
                } else if let Err(e) = result {
                    self.error = Some(e);
                } else if std::mem::take(&mut self.counts_as_visit) {
                    if let Location::Path(p) = &self.location {
                        self.visited = Some(p.clone());
                    }
                }
                if let Some(name) = self.focus_after_load.take() {
                    self.list.focus_name(&name);
                }
                let select = std::mem::take(&mut self.select_after_load);
                if let Some(last) = select.last() {
                    self.list.focus_name(last);
                    for name in &select {
                        if let Some(idx) = self.list.entries.iter().position(|e| &e.name == name)
                            && let Some(pos) = (0..self.list.count()).find(|&p| self.list.entry_of_pos(p) == Some(idx))
                        {
                            self.list.set_selected_pos(pos, true);
                        }
                    }
                }
            }
            // Las superposiciones se piden sobre la lista visible; en una recarga se vuelven a pedir.
            LoadMsg::Overlays(overlays) => {
                if self.reload_buf.is_some() {
                    return false;
                }
                self.list.set_overlays(&overlays);
            }
            // Los índices son de la lista actual; los de una recarga sin aplicar se vuelven a pedir.
            LoadMsg::Metadata(batch) => {
                if self.reload_buf.is_some() {
                    return false;
                }
                self.list.set_meta(batch);
            }
            LoadMsg::Icons(icons) => match &mut self.reload_buf {
                // Íconos de una recarga que todavía no se aplicó.
                Some(buf) => {
                    for (pos, icon) in icons {
                        if let Some(e) = buf.get_mut(pos as usize) {
                            e.icon = icon;
                        }
                    }
                    return false;
                }
                None => self.list.set_icons(&icons),
            },
        }
        true
    }

    /// Ruta del elemento, o su nombre de análisis del Shell si es virtual.
    pub fn entry_path(&self, idx: usize) -> PathBuf {
        let e = &self.list.entries[idx];
        if let Some(full) = &e.full {
            return PathBuf::from(full);
        }
        match &self.location {
            Location::Path(p) => p.join(&e.name),
            // Dentro de un archivo comprimido, la ruta temporal llega en `full`; sin ella (nombre no
            // permitido) no hay ninguna ruta del disco: nunca se arma una con el nombre crudo.
            Location::Archive { .. } => PathBuf::new(),
            _ => PathBuf::from(&e.name),
        }
    }

    /// La ruta interior (con `/`) del elemento `idx` de un archivo comprimido.
    pub fn archive_rel(&self, idx: usize) -> Option<String> {
        let Location::Archive { inner, .. } = &self.location else { return None };
        Some(fs::inner_join(inner, &self.list.entries.get(idx)?.name.to_string_lossy()))
    }

    /// Pide las superposiciones de las filas que se acaban de ver (solo carpetas de disco).
    pub fn request_overlays(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        if !matches!(self.location, Location::Path(_)) {
            return;
        }
        let idxs = self.list.take_unchecked_visible();
        if idxs.is_empty() {
            return;
        }
        let items = idxs.into_iter().map(|i| (i as u32, self.entry_path(i))).collect();
        sys::overlays().request(crate::fs::OverlayRequest {
            hwnd,
            tx: tx.clone(),
            panel: self.id,
            generation: self.generation.load(Ordering::Relaxed),
            items,
        });
    }

    /// Pide los metadatos que la lista necesita (columnas de metadatos a la vista, o la carpeta
    /// entera si se agrupa, ordena o filtra por una), en pedidos de hasta `fs::META_BATCH`.
    pub fn request_metadata(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        if self.reload_buf.is_some() {
            return;
        }
        let idxs = self.list.take_meta_request();
        if idxs.is_empty() {
            return;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        for chunk in idxs.chunks(fs::META_BATCH) {
            sys::metadata().request(fs::MetadataRequest {
                hwnd,
                tx: tx.clone(),
                panel: self.id,
                generation,
                current: self.generation.clone(),
                items: chunk.iter().map(|&i| (i, self.entry_path(i))).collect(),
                fields: fs::MetaField::ALL.to_vec(),
            });
        }
    }

    pub fn entry_item(&self, idx: usize) -> crate::shell::Item {
        crate::shell::Item { path: self.entry_path(idx), pidl: self.list.entries[idx].pidl.clone() }
    }

    /// Abre un elemento: las carpetas se navegan, los archivos se abren con su programa. Un
    /// archivo comprimido que se sabe leer se abre como carpeta. Dentro de un archivo comprimido
    /// solo navega (carpetas y archivos anidados): abrir un archivo de adentro lo hace la `App`
    /// con la extracción temporal; devuelve `false` si no navegó.
    pub fn open(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, idx: usize) -> bool {
        if self.location.is_archive() {
            let e = &self.list.entries[idx];
            if e.attrs & (fs::ATTR_ARCHIVE_BAD | fs::ATTR_LINK) != 0 {
                return false;
            }
            let name = e.name.to_string_lossy().into_owned();
            let to = if e.is_dir {
                self.location.archive_child(&name)
            } else if crate::archive_nav::is_navigable_archive(&name) {
                self.location.archive_nested(&name)
            } else {
                None
            };
            let Some(to) = to else { return false };
            self.navigate(hwnd, tx, to);
            return true;
        }
        let path = self.entry_path(idx);
        let e = &self.list.entries[idx];
        if e.is_dir {
            // Una carpeta de disco se navega por ruta; una virtual, por el Shell.
            let to = if path.is_dir() {
                Location::Path(path)
            } else {
                Location::Shell {
                    parsing: path.to_string_lossy().into_owned(),
                    name: e.display.clone(),
                    parent: Some(Box::new(self.location.clone())),
                }
            };
            self.navigate(hwnd, tx, to);
            return true;
        }
        // Un archivo comprimido de disco (de un formato que se sabe leer) se abre como carpeta
        // dentro del mismo panel; lo lista el núcleo.
        if crate::archive_nav::is_navigable_archive(&e.name.to_string_lossy()) && matches!(self.location, Location::Path(_)) && path.is_file() {
            self.navigate(hwnd, tx, Location::Archive { file: path, nested: Vec::new(), inner: String::new() });
            return true;
        }
        if !path.exists() {
            // Archivo virtual (en un teléfono, dentro de un ZIP del Shell): lo abre el Shell.
            sys::shell().open_item(hwnd, &self.entry_item(idx));
            return false;
        }
        let dir = match &self.location {
            Location::Path(p) => Some(p.as_path()),
            _ => None,
        };
        sys::shell().open_file(hwnd, &path, dir);
        false
    }

    /// Si la carpeta de red falló por credenciales (usuario o contraseña incorrectos, acceso
    /// denegado), muestra el diálogo de Windows y vuelve a cargar. Una vez por navegación.
    pub fn retry_with_credentials(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        let Location::Path(path) = &self.location else { return };
        let Some(e) = &self.error else { return };
        let auth = ["os error 1326", "os error 5)", "os error 86", "os error 1219"].iter().any(|c| e.contains(c));
        if !auth || !fs::is_unc(path) || self.auth_nav == self.nav_id {
            return;
        }
        self.auth_nav = self.nav_id;
        if sys::file_system().connect_share(hwnd, &path.clone()) {
            self.reload(hwnd, tx, false);
            self.auth_nav = self.nav_id;
        }
    }

    pub fn status(&self) -> String {
        if let Some(s) = &self.search {
            let found = self.list.total();
            let found = if found == 1 { "1 encontrado".to_string() } else { format!("{found} encontrados") };
            let scanned = crate::format::thousands(s.scanned);
            // En una carpeta virtual no se revisa nada: solo cuántos coinciden.
            let mut text = if s.in_place {
                format!("Búsqueda «{}»: {found}", s.query)
            } else if s.done {
                let secs = s.finished.unwrap_or(s.started).duration_since(s.started).as_secs_f32();
                format!("Búsqueda «{}»: {found} · {scanned} elementos revisados en {secs:.1} s", s.query)
            } else {
                format!("Buscando «{}»…   {:.0} %   ·   {found} · {scanned} elementos revisados", s.query, (s.progress * 100.0).floor().min(99.0))
            };
            if self.list.is_filtered() {
                text.push_str(" (filtrado)");
            }
            return text;
        }
        if let Some(e) = &self.error {
            return format!("No se pudo abrir: {e}");
        }
        // Con selección, como el Explorador: cuántos y cuánto pesan los archivos (las carpetas no
        // suman; si solo hay carpetas, no hay tamaño).
        let (count, bytes, files) = self
            .list
            .selected_entries()
            .fold((0usize, 0u64, 0usize), |(c, b, f), e| (c + 1, b + if e.is_dir { 0 } else { e.size }, f + !e.is_dir as usize));
        // Todos los elementos, también los de los grupos plegados.
        let mut s = match self.list.total() {
            1 => "1 elemento".to_string(),
            n => format!("{n} elementos"),
        };
        if self.loading {
            s.push_str("   ·   cargando…");
        }
        if let Some(n) = &self.load_notice {
            s = format!("{n}   ·   {s}");
        }
        if count > 0 {
            s.push_str("   ·   ");
            s.push_str(&selection_summary(count, (files > 0).then_some(bytes)));
        }
        // Solo los filtros por columna (no el filtro rápido, que ya se ve en el buscador).
        if self.list.is_filtered() {
            s.push_str(" (filtrado)");
        }
        s
    }
}

/// Texto de la barra de estado con elementos seleccionados: «3 elementos seleccionados · 4,2 MB».
/// `file_bytes` es lo que suman los archivos (`None` si solo hay carpetas: no se muestra tamaño).
pub fn selection_summary(count: usize, file_bytes: Option<u64>) -> String {
    let mut s = if count == 1 { "1 elemento seleccionado".to_string() } else { format!("{count} elementos seleccionados") };
    if let Some(b) = file_bytes {
        s.push_str(" · ");
        s.push_str(&crate::format::size_short(b));
    }
    s
}

/// Nombre del hijo directo de `parent` en el camino hacia `descendant`.
fn child_name(parent: &Location, descendant: &Location) -> Option<OsString> {
    // Los elementos virtuales se listan con su nombre visible.
    if let Location::Shell { name, .. } = descendant {
        return Some(OsString::from(name));
    }
    if let Location::Archive { file, nested, inner } = descendant {
        // Lo que sigue a la ruta interior del padre, hasta la próxima barra.
        let rest = match parent {
            Location::Path(p) => return (file.parent() == Some(p.as_path())).then(|| file.file_name().map(|n| n.to_os_string())).flatten(),
            Location::Archive { file: f, nested: n, inner: i } if f == file && nested.starts_with(n) => {
                let tail = if nested.len() > n.len() { &nested[n.len()] } else if n.len() == nested.len() { inner } else { return None };
                if i.is_empty() { tail.as_str() } else { tail.strip_prefix(i.as_str())?.strip_prefix('/')? }
            }
            _ => return None,
        };
        return rest.split('/').next().filter(|s| !s.is_empty()).map(OsString::from);
    }
    let Location::Path(d) = descendant else { return None };
    match parent {
        Location::Shell { .. } | Location::Archive { .. } => None,
        Location::Computer => d.components().next().map(|c| {
            // La unidad se lista como "C:\".
            let mut s = c.as_os_str().to_os_string();
            s.push("\\");
            s
        }),
        Location::Path(p) => d.strip_prefix(p).ok()?.components().next().map(|c| c.as_os_str().to_os_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use crate::sys::testing::{FakeFileSystem, install_fakes};
    use std::path::Path;

    /// Carpeta temporal propia de una prueba, con el nombre que le da el sistema de archivos
    /// instalado (en Windows el runner da nombres cortos `RUNNER~1`).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-tab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sys::file_system().true_case(&dir)
    }

    /// Texto de la selección en la barra de estado: singular o plural y, si hay archivos, su tamaño.
    #[test]
    fn selection_summary_texts() {
        install_fakes();
        assert_eq!(selection_summary(1, Some(2048)), format!("1 elemento seleccionado · {}", crate::format::size_short(2048)));
        assert_eq!(selection_summary(3, None), "3 elementos seleccionados");
    }

    /// Una pestaña que se muestra en una carpeta la lee con el `FileSystem` instalado y recibe
    /// sus elementos por el canal de carga.
    #[test]
    fn tab_loads_through_file_system() {
        install_fakes();
        let dir = temp_dir("carga");
        std::fs::write(dir.join("uno.txt"), "1").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        let (tx, rx) = channel();
        let mut tab = Tab::new(3, Location::Path(dir.clone()), false);
        tab.set_visible(Handle(0), &tx, true);
        while tab.loading {
            let ev = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("la carga termina");
            assert_eq!(ev.panel, 3);
            tab.on_load(ev);
        }
        assert_eq!(FakeFileSystem::loads(&dir), 1);
        assert_eq!(tab.error, None);
        let mut names: Vec<String> = tab.list.entries.iter().map(|e| e.name.to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["sub", "uno.txt"]);
        assert!(tab.list.entries.iter().any(|e| e.is_dir && e.name == "sub"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn pump(tab: &mut Tab, rx: &std::sync::mpsc::Receiver<LoadEvent>) {
        while tab.loading {
            let ev = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("la carga termina");
            tab.on_load(ev);
        }
    }

    fn shown(tab: &Tab) -> Vec<String> {
        let mut v: Vec<String> = (0..tab.list.count()).map(|p| tab.list.entries[tab.list.entry_of_pos(p).unwrap()].display.clone()).collect();
        v.sort();
        v
    }

    /// Dentro de un archivo comprimido la pestaña lista en un hilo del núcleo (no el sistema de
    /// archivos), entra a una carpeta (también implícita), filtra en el lugar al buscar y al subir
    /// desde la raíz vuelve a la carpeta del archivo con el archivo enfocado.
    #[test]
    fn tab_lists_archive_and_navigates() {
        install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-tab-zip-{}", std::process::id()));
        let zip = dir.join("fotos.zip");
        crate::sys::testing::FakeArchives::shared().add_archive(&zip, &[("2024/viaje/a.jpg", b"jpg"), ("2024/b.jpg", b"b"), ("nota.txt", b"n")], 9);
        let (tx, rx) = channel();
        let loc = Location::Archive { file: zip.clone(), nested: vec![], inner: String::new() };
        let mut tab = Tab::new(4, loc.clone(), false);
        tab.set_visible(Handle(0), &tx, true);
        pump(&mut tab, &rx);
        assert_eq!(tab.error, None);
        assert_eq!(FakeFileSystem::loads(&zip), 0, "no lo lista el sistema");
        assert_eq!(shown(&tab), ["2024", "nota.txt"]);
        assert!(!tab.list.meta_enabled);
        let i = tab.list.entries.iter().position(|e| e.name == "2024").unwrap();
        assert!(tab.open(Handle(0), &tx, i));
        pump(&mut tab, &rx);
        assert_eq!(tab.location, Location::Archive { file: zip.clone(), nested: vec![], inner: "2024".into() });
        assert_eq!(shown(&tab), ["b.jpg", "viaje"]);
        // Un archivo de adentro no lo abre la pestaña (lo hace la App con la temporal).
        let b = tab.list.entries.iter().position(|e| e.name == "b.jpg").unwrap();
        assert!(!tab.open(Handle(0), &tx, b));
        assert_eq!(tab.archive_rel(b).as_deref(), Some("2024/b.jpg"));
        assert!(tab.entry_path(b).ends_with(Path::new("2024").join("b.jpg")));
        // Buscar: filtra lo que muestra, sin recorrer.
        tab.search(Handle(0), &tx, "via");
        assert_eq!(shown(&tab), ["viaje"]);
        tab.end_search(Handle(0), &tx);
        tab.go_up(Handle(0), &tx);
        pump(&mut tab, &rx);
        tab.go_up(Handle(0), &tx);
        assert_eq!(tab.location, Location::Path(dir.clone()));
        assert_eq!(tab.focus_after_load, Some("fotos.zip".into()));
    }

    /// Más entradas que el tope: se listan las primeras y la barra de estado lo dice.
    #[test]
    fn archive_cap_shows_notice() {
        install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-tab-zip-tope-{}", std::process::id()));
        let zip = dir.join("muchos.zip");
        let names: Vec<String> = (0..crate::archive_nav::MAX_ENTRIES + 3).map(|i| format!("f{i}")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        crate::sys::testing::FakeArchives::shared().add_archive(&zip, &entries, 9);
        let (tx, rx) = channel();
        let mut tab = Tab::new(6, Location::Archive { file: zip, nested: vec![], inner: String::new() }, false);
        tab.set_visible(Handle(0), &tx, true);
        pump(&mut tab, &rx);
        assert_eq!(tab.list.entries.len(), crate::archive_nav::MAX_ENTRIES);
        assert!(tab.status().starts_with("Se muestran los primeros 100.000 elementos"), "{}", tab.status());
    }

    /// Una carpeta interior que ya no está, o un zip con contraseña: error de la carga.
    #[test]
    fn archive_errors_reach_the_tab() {
        install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-tab-zip-err-{}", std::process::id()));
        let zip = dir.join("clave.zip");
        crate::sys::testing::FakeArchives::shared().add_archive(&zip, &[("a.txt", b"a")], 9);
        let (tx, rx) = channel();
        let mut tab = Tab::new(5, Location::Archive { file: zip.clone(), nested: vec![], inner: "no".into() }, false);
        tab.set_visible(Handle(0), &tx, true);
        pump(&mut tab, &rx);
        assert_eq!(tab.error.as_deref(), Some(crate::archive_nav::NO_FOLDER));
        let locked = dir.join("otra.zip");
        crate::sys::testing::FakeArchives::shared().add_archive(&locked, &[("a.txt", b"a")], 9);
        crate::sys::testing::FakeArchives::shared().set_password(&locked);
        tab.navigate(Handle(0), &tx, Location::Archive { file: locked, nested: vec![], inner: String::new() });
        pump(&mut tab, &rx);
        assert_eq!(tab.error.as_deref(), Some(sys::PASSWORD));
    }

    /// Pestaña a la vista que ya pasó por `paths` en orden (cada una, una navegación).
    fn tab_through(paths: &[&str]) -> (Tab, Sender<LoadEvent>) {
        let (tx, _rx) = channel();
        let mut tab = Tab::new(7, Location::Path(PathBuf::from(paths[0])), false);
        tab.set_visible(Handle(0), &tx, true);
        for p in &paths[1..] {
            tab.navigate(Handle(0), &tx, Location::Path(PathBuf::from(p)));
        }
        (tab, tx)
    }

    /// Con 20 navegaciones, la lista de Atrás tiene como máximo lo pedido y empieza por la
    /// carpeta anterior.
    #[test]
    fn back_list_is_capped_and_nearest_first() {
        install_fakes();
        let names: Vec<String> = (0..21).map(|i| format!("/historial/c{i}")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let (tab, _tx) = tab_through(&refs);
        let back = tab.back_list(15);
        assert_eq!(back.len(), 15);
        assert_eq!(back[0], Location::Path(PathBuf::from("/historial/c19")));
        assert_eq!(back[14], Location::Path(PathBuf::from("/historial/c5")));
        assert!(tab.forward_list(15).is_empty());
    }

    /// Ir dos lugares atrás de una vez equivale a dos «Atrás»: Adelante queda con las salteadas,
    /// la más cercana primero, y se lee la carpeta una sola vez.
    #[test]
    fn go_back_n_keeps_forward() {
        install_fakes();
        let (mut tab, tx) = tab_through(&["/hist-n/A", "/hist-n/B", "/hist-n/C", "/hist-n/D"]);
        let before = FakeFileSystem::loads(Path::new("/hist-n/B"));
        tab.go_back_n(2, Handle(0), &tx);
        assert_eq!(tab.location, Location::Path(PathBuf::from("/hist-n/B")));
        assert_eq!(FakeFileSystem::loads(Path::new("/hist-n/B")) - before, 1);
        assert_eq!(tab.forward_list(15), [Location::Path(PathBuf::from("/hist-n/C")), Location::Path(PathBuf::from("/hist-n/D"))]);
        assert_eq!(tab.back_list(15), [Location::Path(PathBuf::from("/hist-n/A"))]);
        // Adelante dos lugares vuelve a D; pedir más de los que hay se queda en el último.
        tab.go_forward_n(5, Handle(0), &tx);
        assert_eq!(tab.location, Location::Path(PathBuf::from("/hist-n/D")));
        assert_eq!(tab.back_list(15).len(), 3);
        assert!(!tab.can_forward());
    }

    /// Cada guardián de carpeta se suelta: al ocultar la pestaña, al navegar a otra carpeta y al
    /// cerrarla. 50 pestañas que vigilan y se cierran → 50 bajas en el falso.
    #[test]
    fn watchers_are_released() {
        install_fakes();
        let dir = temp_dir("guardianes");
        let other = temp_dir("guardianes-otra");
        let (tx, _rx) = channel();
        let tabs: Vec<Tab> = (0..50)
            .map(|i| {
                let mut t = Tab::new(100 + i, Location::Path(dir.clone()), false);
                t.set_visible(Handle(0), &tx, true);
                t
            })
            .collect();
        assert_eq!(FakeFileSystem::watches(&dir), (50, 0));
        drop(tabs);
        assert_eq!(FakeFileSystem::watches(&dir), (50, 50));

        // Navegar suelta el guardián de la carpeta anterior; ocultar, el de la actual.
        let mut t = Tab::new(200, Location::Path(dir.clone()), false);
        t.set_visible(Handle(0), &tx, true);
        t.navigate(Handle(0), &tx, Location::Path(other.clone()));
        assert_eq!(FakeFileSystem::watches(&dir), (51, 51));
        assert_eq!(FakeFileSystem::watches(&other), (1, 0));
        t.set_visible(Handle(0), &tx, false);
        assert_eq!(FakeFileSystem::watches(&other), (1, 1));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    /// La barra de estado termina en « (filtrado)» solo con filtros de columna.
    #[test]
    fn status_says_filtered() {
        install_fakes();
        let mut tab = Tab::new(9, Location::Path(PathBuf::from("/filtrado")), false);
        let entry = |name: &str| Entry {
            name: name.into(),
            full: None,
            group: 0,
            pidl: None,
            display: name.into(),
            sort_key: name.to_lowercase(),
            is_dir: false,
            size: 0,
            modified: 0,
            created: 0,
            attrs: 0,
            type_name: Arc::from("Archivo"),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
            packed: 0,
        };
        tab.list.append(vec![entry("a.jpg"), entry("b.png"), entry("c.png")]);
        assert_eq!(tab.status(), "3 elementos");
        tab.list.set_filter(Column::Extension, vec!["PNG".into()]);
        assert_eq!(tab.status(), "2 elementos (filtrado)");
        tab.list.select_all();
        assert!(tab.status().ends_with(" (filtrado)"), "{}", tab.status());
        tab.list.set_filter(Column::Extension, Vec::new());
        assert!(!tab.status().contains("filtrado"));
    }
}

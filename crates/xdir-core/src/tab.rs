//! Una pestaña: ubicación, historial atrás/adelante, la lista de archivos, su carga y su vigilancia.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::fs::{self, Entry, LoadEvent, LoadMsg, Location, WM_APP_CHANGED};
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
            watcher: None,
            visible: false,
            counts_as_visit: false,
            auth_nav: 0,
            visited: None,
            search: None,
        }
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
        if let Some(prev) = self.back.pop() {
            let from = std::mem::replace(&mut self.location, prev);
            self.focus_after_load = child_name(&self.location, &from);
            self.forward.push(from);
            self.start(hwnd, tx);
            self.counts_as_visit = true;
        }
    }

    pub fn go_forward(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        if let Some(next) = self.forward.pop() {
            let from = std::mem::replace(&mut self.location, next);
            self.back.push(from);
            self.start(hwnd, tx);
            self.counts_as_visit = true;
        }
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
        if let Some(query) = self.search.as_ref().map(|s| s.query.clone()) {
            if !silent {
                self.search(hwnd, tx, &query);
            }
            return;
        }
        if silent {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            self.reload_buf = Some(Vec::new());
            sys::file_system().spawn_load(hwnd, tx.clone(), self.id, generation, self.generation.clone(), self.location.clone());
        } else {
            self.focus_after_load = self.list.focused_index().map(|i| self.list.entries[i].name.clone());
            self.start(hwnd, tx);
        }
    }

    /// Enfoca `name` cuando termine la carga en curso.
    pub fn focus_after_load(&mut self, name: OsString) {
        self.focus_after_load = Some(name);
    }

    /// Busca `query` en la carpeta de la pestaña y todas sus subcarpetas ("Este equipo": en
    /// todas las unidades). Los resultados van apareciendo en la lista. Con el texto vacío,
    /// termina la búsqueda y vuelve a la carpeta.
    pub fn search(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, query: &str) {
        if query.trim().is_empty() {
            self.end_search(hwnd, tx);
            return;
        }
        let (roots, root) = match &self.location {
            Location::Path(p) => (vec![p.clone()], Some(p.clone())),
            Location::Computer => (sys::file_system().drives(), None),
            // Las carpetas virtuales no se recorren.
            Location::Shell { .. } => return,
        };
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.list.clear();
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
        self.search = Some(Search { query: query.to_string(), scanned: 0, progress: 0.0, done: false, started: Instant::now(), finished: None });
        fs::spawn_search(hwnd, tx.clone(), self.id, generation, self.generation.clone(), roots, self.list.show_hidden, list::name_matcher(query));
    }

    /// Termina la búsqueda (si había) y vuelve a mostrar la carpeta.
    pub fn end_search(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        if self.search.is_some() {
            self.start(hwnd, tx);
        }
    }

    /// Lo escrito en el buscador, si hay una búsqueda.
    pub fn search_query(&self) -> &str {
        self.search.as_ref().map_or("", |s| s.query.as_str())
    }

    /// Saca de la lista lo propio de la búsqueda (la columna de carpeta).
    fn drop_search(&mut self) {
        self.search = None;
        self.list.search_root = None;
        self.list.columns.retain(|c| c.col != Column::Folder);
    }

    fn start(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>) {
        self.drop_search();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.nav_id += 1;
        // Quien navega la vuelve a marcar como visita después; una recarga no cuenta.
        self.counts_as_visit = false;
        self.list.clear();
        self.loading = true;
        self.error = None;
        self.reload_buf = None;
        self.watcher = match &self.location {
            Location::Path(p) if self.visible => sys::file_system().watch(hwnd, WM_APP_CHANGED, self.id, self.nav_id as isize, p),
            _ => None,
        };
        sys::file_system().spawn_load(hwnd, tx.clone(), self.id, generation, self.generation.clone(), self.location.clone());
    }

    /// Aviso del vigilante: recarga silenciosa si sigue siendo la misma navegación.
    pub fn on_changed(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, nav_id: u64) {
        if nav_id == self.nav_id && !self.loading && self.search.is_none() {
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
            }
            // Las superposiciones se piden sobre la lista visible; en una recarga se vuelven a pedir.
            LoadMsg::Overlays(overlays) => {
                if self.reload_buf.is_some() {
                    return false;
                }
                self.list.set_overlays(&overlays);
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
            _ => PathBuf::from(&e.name),
        }
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

    pub fn entry_item(&self, idx: usize) -> crate::shell::Item {
        crate::shell::Item { path: self.entry_path(idx), pidl: self.list.entries[idx].pidl.clone() }
    }

    /// Abre un elemento: las carpetas se navegan, los archivos se abren con su programa.
    pub fn open(&mut self, hwnd: Handle, tx: &Sender<LoadEvent>, idx: usize) {
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
            return;
        }
        // Un .zip de disco se abre como carpeta dentro del mismo panel (lo lista el Shell).
        if fs::is_zip(&path) && path.is_file() {
            let to = Location::Shell {
                parsing: path.to_string_lossy().into_owned(),
                name: e.display.clone(),
                parent: Some(Box::new(self.location.clone())),
            };
            self.navigate(hwnd, tx, to);
            return;
        }
        if !path.exists() {
            // Archivo virtual (en un teléfono, dentro de un ZIP): lo abre el Shell.
            sys::shell().open_item(hwnd, &self.entry_item(idx));
            return;
        }
        let dir = match &self.location {
            Location::Path(p) => Some(p.as_path()),
            _ => None,
        };
        sys::shell().open_file(hwnd, &path, dir);
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
            let found = self.list.count();
            let found = if found == 1 { "1 encontrado".to_string() } else { format!("{found} encontrados") };
            let scanned = crate::format::thousands(s.scanned);
            return if s.done {
                let secs = s.finished.unwrap_or(s.started).duration_since(s.started).as_secs_f32();
                format!("Búsqueda «{}»: {found} · {scanned} elementos revisados en {secs:.1} s", s.query)
            } else {
                format!("Buscando «{}»…   {:.0} %   ·   {found} · {scanned} elementos revisados", s.query, (s.progress * 100.0).floor().min(99.0))
            };
        }
        if let Some(e) = &self.error {
            return format!("No se pudo abrir: {e}");
        }
        let n = self.list.count();
        let mut s = match n {
            1 => "1 elemento".to_string(),
            _ => format!("{n} elementos"),
        };
        if self.loading {
            s.push_str("   ·   cargando…");
        }
        let (count, bytes, files) = self
            .list
            .selected_entries()
            .fold((0usize, 0u64, 0usize), |(c, b, f), e| (c + 1, b + if e.is_dir { 0 } else { e.size }, f + !e.is_dir as usize));
        if count > 0 {
            s.push_str(&format!("   ·   {count} seleccionado{}", if count == 1 { "" } else { "s" }));
            if files > 0 {
                s.push_str(&format!("   ·   {}", crate::format::size_short(bytes)));
            }
        }
        s
    }
}

/// Nombre del hijo directo de `parent` en el camino hacia `descendant`.
fn child_name(parent: &Location, descendant: &Location) -> Option<OsString> {
    // Los elementos virtuales se listan con su nombre visible.
    if let Location::Shell { name, .. } = descendant {
        return Some(OsString::from(name));
    }
    let Location::Path(d) = descendant else { return None };
    match parent {
        Location::Shell { .. } => None,
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

    /// Carpeta temporal propia de una prueba, con el nombre que le da el sistema de archivos
    /// instalado (en Windows el runner da nombres cortos `RUNNER~1`).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-tab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sys::file_system().true_case(&dir)
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
}

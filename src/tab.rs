//! Una pestaña: ubicación, historial atrás/adelante, la lista de archivos, su carga y su vigilancia.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::fs::{self, Entry, LoadEvent, LoadMsg, Location, Watcher};
use crate::list::FileList;

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
    watcher: Option<Watcher>,
    /// Solo las pestañas a la vista vigilan su carpeta (un hilo por pestaña no escala).
    visible: bool,
}

impl Tab {
    /// Muestra u oculta la pestaña. Al mostrarse por primera vez carga su carpeta; al volver
    /// a mostrarse, recarga en silencio por si cambió mientras no se veía.
    pub fn set_visible(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>, visible: bool) {
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
            self.watcher = Watcher::new(hwnd, self.id, self.nav_id, p);
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
            Location::Path(p) => Location::Path(fs::long_path(&p)),
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

    pub fn navigate(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>, to: Location) {
        // Rutas cortas 8.3 (LEANDR~1) a su forma larga, para mostrarlas y compararlas bien.
        let to = match to {
            Location::Path(p) => Location::Path(fs::long_path(&p)),
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
    }

    pub fn go_back(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>) {
        if let Some(prev) = self.back.pop() {
            let from = std::mem::replace(&mut self.location, prev);
            self.focus_after_load = child_name(&self.location, &from);
            self.forward.push(from);
            self.start(hwnd, tx);
        }
    }

    pub fn go_forward(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>) {
        if let Some(next) = self.forward.pop() {
            let from = std::mem::replace(&mut self.location, next);
            self.back.push(from);
            self.start(hwnd, tx);
        }
    }

    pub fn go_up(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>) {
        if let Some(parent) = self.location.parent() {
            let child = child_name(&parent, &self.location);
            self.navigate(hwnd, tx, parent);
            self.focus_after_load = child;
        }
    }

    /// Vuelve a leer la carpeta. `silent`: sin vaciar la lista mientras tanto.
    pub fn reload(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>, silent: bool) {
        if silent {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            self.reload_buf = Some(Vec::new());
            fs::spawn_load(hwnd, tx.clone(), self.id, generation, self.generation.clone(), self.location.clone());
        } else {
            self.focus_after_load = self.list.focused_index().map(|i| self.list.entries[i].name.clone());
            self.start(hwnd, tx);
        }
    }

    /// Enfoca `name` cuando termine la carga en curso.
    pub fn focus_after_load(&mut self, name: OsString) {
        self.focus_after_load = Some(name);
    }

    fn start(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.nav_id += 1;
        self.list.clear();
        self.loading = true;
        self.error = None;
        self.reload_buf = None;
        self.watcher = match &self.location {
            Location::Path(p) if self.visible => Watcher::new(hwnd, self.id, self.nav_id, p),
            _ => None,
        };
        fs::spawn_load(hwnd, tx.clone(), self.id, generation, self.generation.clone(), self.location.clone());
    }

    /// Aviso del vigilante: recarga silenciosa si sigue siendo la misma navegación.
    pub fn on_changed(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>, nav_id: u64) {
        if nav_id == self.nav_id && !self.loading {
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
            LoadMsg::Listed(result) => {
                self.loading = false;
                if let Some(buf) = self.reload_buf.take() {
                    if result.is_ok() {
                        self.list.replace(buf);
                    }
                } else if let Err(e) = result {
                    self.error = Some(e);
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
    pub fn request_overlays(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>) {
        if !matches!(self.location, Location::Path(_)) {
            return;
        }
        let idxs = self.list.take_unchecked_visible();
        if idxs.is_empty() {
            return;
        }
        let items = idxs.into_iter().map(|i| (i as u32, self.entry_path(i))).collect();
        crate::icons::request_overlays(crate::icons::OverlayRequest {
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
    pub fn open(&mut self, hwnd: HWND, tx: &Sender<LoadEvent>, idx: usize) {
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
        if !path.exists() {
            // Archivo virtual (en un teléfono, dentro de un ZIP): lo abre el Shell.
            crate::shell::open_item(hwnd, &self.entry_item(idx));
            return;
        }
        let file = fs::wide_path(&path);
        let dir = match &self.location {
            Location::Path(p) => fs::wide_path(p),
            _ => vec![0],
        };
        unsafe {
            ShellExecuteW(Some(hwnd), w!("open"), PCWSTR(file.as_ptr()), None, PCWSTR(dir.as_ptr()), SW_SHOWNORMAL);
        }
    }

    pub fn status(&self) -> String {
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

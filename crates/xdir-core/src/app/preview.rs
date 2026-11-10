//! Columna de vista previa: qué muestra (lo del último panel activo), cuándo se le pide al
//! servicio (`sys::Preview`) y el borde que cambia su ancho.
//!
//! Mover la selección nunca espera: el sujeto se calcula antes de cada cuadro (`sync_preview`) y,
//! si cambió, se pide recién después de una pausa corta (`PREVIEW_DELAY_MS`, con un solo aviso en
//! camino, como el guardado de `carpetas.toml`), así las flechas rápidas no lanzan un hilo de
//! lectura por tecla. Cada pedido lleva una generación nueva y las respuestas viejas se descartan.

use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::fs::Entry;
use crate::preview::draw::{CLOUD_TEXT, UNREADABLE_TEXT, items_text};
use crate::preview::{PreviewBody, PreviewInfo, PreviewReply, PreviewRequest, PreviewTarget, Shown, detail_rows};
use crate::thumbs;

/// Pasó la pausa después del último cambio de sujeto: se pide la vista previa.
pub(super) const WM_APP_PREVIEW_DUE: u32 = WM_APP + 25;
/// Cuánto se espera después del último cambio de selección para pedir la vista previa.
const PREVIEW_DELAY_MS: u64 = 50;
/// Lo que se agarra del borde izquierdo de la columna para cambiar su ancho.
const EDGE_GRAB: f32 = 4.0;

/// Lo que se le pide al servicio: qué y su versión (fecha de modificación y tamaño), para volver a
/// pedirlo si cambia.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Wanted {
    pub target: PreviewTarget,
    pub stamp: (u64, u64),
}

/// De dónde sale lo que se muestra: el panel, la pestaña, la carpeta, cuántos hay seleccionados y
/// el elemento (con su versión). Si no cambia, el sujeto tampoco.
#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    pane: usize,
    tab: usize,
    location: Location,
    selected: usize,
    entry: Option<(usize, OsString, u64, u64)>,
    len: usize,
}

/// De quién es el lugar de la derecha (el de la columna de vista previa): de la vista previa, del
/// panel de información o del de Renombrar varios. Hay uno solo a la vez: abrir uno cierra el otro y
/// al cerrar información o Renombrar varios vuelve la vista previa si estaba prendida.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RightPanel {
    Preview,
    Info,
    RenameMany,
}

/// Estado de la columna en la `App`.
pub(super) struct PreviewState {
    /// Prendida (se ve salvo con xtree abierto).
    pub on: bool,
    /// Quién ocupa el lugar (con otro dueño que la vista previa, se ve aunque esté apagada).
    pub owner: RightPanel,
    /// Ancho elegido (240–640); el que se usa puede ser menor para no achicar los paneles.
    pub width: f32,
    pub(super) generation: u64,
    current: Arc<AtomicU64>,
    pub(super) tx: Sender<PreviewReply>,
    rx: Receiver<PreviewReply>,
    fingerprint: Option<Fingerprint>,
    /// Lo que se muestra ahora (`None`: varios, nada o sin ruta) y lo que llegó para eso.
    pub(super) subject: Option<Wanted>,
    pub(super) info: Option<PreviewInfo>,
    /// El sujeto cambió y falta pedirlo (cuando pase la pausa).
    pending: bool,
    changed_at: Instant,
    /// Hay un aviso de la pausa en camino (`WM_APP_PREVIEW_DUE`).
    posted: bool,
    /// Imágenes grandes que hay que olvidar en el próximo cuadro (el archivo cambió).
    pub(super) forget: Vec<thumbs::Source>,
    /// Galería: el enfocado de cada panel con su fecha y tamaño, para notar que cambió en el
    /// disco aunque la columna esté apagada.
    gallery_stamps: Vec<Option<(thumbs::Source, u64, u64)>>,
}

impl PreviewState {
    pub(super) fn new(on: bool, width: f32) -> PreviewState {
        let (tx, rx) = channel();
        PreviewState {
            on,
            owner: RightPanel::Preview,
            width: width.clamp(ui::PREVIEW_MIN_W, ui::PREVIEW_MAX_W),
            generation: 0,
            current: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
            fingerprint: None,
            subject: None,
            info: None,
            pending: false,
            changed_at: Instant::now(),
            posted: false,
            forget: Vec::new(),
            gallery_stamps: Vec::new(),
        }
    }

    /// Una generación nueva: lo que estaba en camino queda viejo.
    fn bump(&mut self) -> u64 {
        self.generation += 1;
        self.current.store(self.generation, Ordering::Relaxed);
        self.generation
    }
}

/// De dónde sale la imagen de un elemento (por PIDL o por ruta).
fn source_of(tab: &Tab, idx: usize) -> thumbs::Source {
    let item = tab.entry_item(idx);
    match item.pidl {
        Some(pidl) => thumbs::Source::Pidl(pidl),
        None => thumbs::Source::Path(item.path),
    }
}

impl App {
    /// ¿Se ve la columna? (prendida o con otro panel en su lugar, y sin la vista de xtree encima).
    pub(super) fn preview_visible(&self) -> bool {
        if !(self.preview.on || self.preview.owner != RightPanel::Preview) || self.xtree_open {
            return false;
        }
        // Si ni el mínimo entra sin achicar los paneles, no se ve en este cuadro (sigue prendida).
        let (w, _) = self.client_dips();
        ui::preview_fits(w, self.tree_width(), self.panes_across())
    }

    /// Paneles lado a lado (para no dejar ninguno por debajo de su mínimo).
    fn panes_across(&self) -> usize {
        if self.maximized.is_some() {
            return 1;
        }
        match self.layout {
            Layout::One | Layout::TwoStack => 1,
            _ => 2,
        }
    }

    /// Ancho que ocupa la columna a la derecha, con su separación (0 si no se ve). Es el `right`
    /// de `ui::panes_area`.
    pub(super) fn preview_width(&self) -> f32 {
        if !self.preview_visible() {
            return 0.0;
        }
        let (w, _) = self.client_dips();
        ui::clamp_preview_width(self.preview.width, w, self.tree_width(), self.panes_across()) + ui::GAP
    }

    /// Rectángulo de la columna (si se ve).
    pub(super) fn preview_column(&self) -> Option<geom::Rect> {
        let width = self.preview_width();
        if width <= 0.0 {
            return None;
        }
        let (w, h) = self.client_dips();
        Some(ui::preview_column_rect(w, h, ui::GAP / 2.0, width - ui::GAP))
    }

    /// ¿`(x, y)` está sobre el borde izquierdo de la columna?
    pub(super) fn preview_edge_at(&self, x: f32, y: f32) -> bool {
        self.preview_column().is_some_and(|r| y >= r.top && y <= r.bottom && x >= r.left - ui::GAP && x <= r.left + EDGE_GRAB)
    }

    /// Arrastrando el borde: el borde va a `x` (el mouse menos lo que había de él al agarrarlo).
    pub(super) fn drag_preview_edge(&mut self, x: f32) {
        let (w, _) = self.client_dips();
        let want = (w - ui::GAP) - x;
        let clamped = ui::clamp_preview_width(want, w, self.tree_width(), self.panes_across());
        self.preview.width = clamped.clamp(ui::PREVIEW_MIN_W, ui::PREVIEW_MAX_W);
    }

    /// Prende la columna (venía de xtree, donde no se veía).
    pub(super) fn show_preview(&mut self) {
        if self.preview.owner != RightPanel::Preview {
            self.close_info();
        }
        self.preview.on = true;
        self.sync_preview();
        self.redraw();
    }

    /// Prende o apaga la columna. Con otro panel en su lugar, lo cierra y muestra la vista previa.
    pub(super) fn toggle_preview(&mut self) {
        if self.preview.owner != RightPanel::Preview {
            self.close_info();
            self.preview.on = true;
            self.sync_preview();
            self.redraw();
            return;
        }
        self.begin_layout_anim();
        self.preview.on = !self.preview.on;
        self.sync_preview();
        self.redraw();
    }

    /// Qué se le pediría al servicio para lo que muestra la columna, y de qué elemento sale (índice
    /// en la lista). `None`: varios seleccionados, nada o un lugar sin ruta de disco.
    fn preview_wanted(&self) -> (Option<Wanted>, Option<usize>) {
        let tab = self.panes[self.active].tab();
        let list = &tab.list;
        let mut selected = list.selected_indices();
        let (first, second) = (selected.next(), selected.next());
        let idx = match (first, second) {
            (Some(_), Some(_)) => return (None, None),
            (Some(i), None) => Some(i),
            // Nada seleccionado: en «Este equipo», el disco enfocado; en otra carpeta, ella misma.
            (None, _) => match &tab.location {
                // Su versión es la cantidad de elementos: si cambia, se vuelve a contar.
                Location::Path(p) => return (Some(Wanted { target: PreviewTarget::Folder(p.clone()), stamp: (list.entries.len() as u64, 0) }), None),
                Location::Computer => list.focused_index(),
                Location::Shell { .. } => None,
            },
        };
        let Some(idx) = idx.filter(|&i| i < list.entries.len()) else { return (None, None) };
        let e = &list.entries[idx];
        let target = match &tab.location {
            // Las carpetas virtuales no tienen de dónde leer.
            Location::Shell { .. } => return (None, Some(idx)),
            Location::Computer if e.is_dir => {
                let path = tab.entry_path(idx);
                if sys::file_system().drives().iter().any(|d| d == &path) {
                    PreviewTarget::Drive(path)
                } else {
                    PreviewTarget::Folder(path)
                }
            }
            _ if e.is_dir => PreviewTarget::Folder(tab.entry_path(idx)),
            _ => PreviewTarget::File(tab.entry_path(idx)),
        };
        (Some(Wanted { target, stamp: (e.modified, e.size) }), Some(idx))
    }

    /// Antes de cada cuadro (y al prender la columna): si cambió lo que muestra, lo deja cargando y
    /// programa el pedido después de la pausa. Si cambió solo la versión del mismo archivo (lo
    /// avisó el vigilante de la carpeta), olvida su imagen grande y lo vuelve a pedir sin pasar
    /// por «cargando».
    /// Galería: si el enfocado de un panel es el mismo archivo que antes pero con otra fecha o
    /// tamaño (se editó), su imagen grande se olvida y se vuelve a pedir (como hace la columna).
    pub(super) fn sync_gallery_stamps(&mut self) {
        let stamps: Vec<_> = self
            .panes
            .iter()
            .map(|pane| {
                let list = &pane.tab().list;
                if list.view != list::View::Gallery {
                    return None;
                }
                let e = list.entries.get(list.focused_index()?)?;
                Some((list.thumb_source(e)?, e.modified, e.size))
            })
            .collect();
        for (i, now) in stamps.iter().enumerate() {
            if let (Some((src, m, s)), Some(Some((old, om, os)))) = (now, self.preview.gallery_stamps.get(i))
                && src == old
                && (m, s) != (om, os)
                && let thumbs::Source::Path(p) = src
            {
                self.preview.forget.push(thumbs::Source::Path(p.clone()));
            }
        }
        self.preview.gallery_stamps = stamps;
    }

    pub(super) fn sync_preview(&mut self) {
        // Con otro panel en su lugar, la vista previa no muestra ni pide nada.
        if !self.preview_visible() || self.preview.owner != RightPanel::Preview {
            if self.preview.subject.is_some() || self.preview.fingerprint.is_some() {
                self.preview.subject = None;
                self.preview.info = None;
                self.preview.fingerprint = None;
                self.preview.pending = false;
                self.preview.bump();
            }
            return;
        }
        let tab = self.panes[self.active].tab();
        let list = &tab.list;
        let focus = list.focused_index();
        let selected = list.selected_indices().take(2).count();
        let entry = match selected {
            1 => list.selected_indices().next(),
            0 => focus,
            _ => None,
        };
        let fp = Fingerprint {
            pane: self.active,
            tab: tab.id,
            location: tab.location.clone(),
            selected,
            entry: entry.and_then(|i| list.entries.get(i)).map(|e| (entry.unwrap_or(0), e.name.clone(), e.modified, e.size)),
            len: list.entries.len(),
        };
        if self.preview.fingerprint.as_ref() == Some(&fp) {
            return;
        }
        self.preview.fingerprint = Some(fp);
        let (wanted, idx) = self.preview_wanted();
        if wanted == self.preview.subject {
            return;
        }
        let same_target = matches!((&wanted, &self.preview.subject), (Some(a), Some(b)) if a.target == b.target);
        if same_target {
            // El mismo archivo con otra fecha o tamaño: la imagen vieja ya no vale.
            if let Some(w) = &wanted {
                self.preview.forget.push(thumbs::Source::Path(w.target.path().to_path_buf()));
            }
        } else {
            self.preview.info = None;
        }
        self.preview.subject = wanted;
        self.preview.bump();
        if let Some(i) = idx.filter(|_| self.preview.subject.is_some()) {
            self.panes[self.active].tab_mut().list.want_meta(i);
        }
        if self.preview.subject.is_none() {
            self.preview.pending = false;
            return;
        }
        self.preview.pending = true;
        self.preview.changed_at = Instant::now();
        if !self.preview.posted {
            self.preview.posted = true;
            post_preview_due(self.hwnd, PREVIEW_DELAY_MS);
        }
    }

    /// Pasó la pausa: si hubo otro cambio después, se espera lo que falta; si no, se pide.
    pub(super) fn on_preview_due(&mut self) {
        self.preview.posted = false;
        if !self.preview.pending {
            return;
        }
        let waited = self.preview.changed_at.elapsed().as_millis() as u64;
        if waited < PREVIEW_DELAY_MS {
            self.preview.posted = true;
            post_preview_due(self.hwnd, PREVIEW_DELAY_MS - waited);
            return;
        }
        self.request_preview();
    }

    /// Pide la vista previa del sujeto actual al servicio, con una generación nueva.
    fn request_preview(&mut self) {
        self.preview.pending = false;
        let Some(w) = &self.preview.subject else { return };
        let target = w.target.clone();
        let generation = self.preview.bump();
        sys::preview().request(PreviewRequest {
            hwnd: self.hwnd,
            tx: self.preview.tx.clone(),
            generation,
            current: self.preview.current.clone(),
            target,
        });
    }

    /// Pruebas: pide ya lo pendiente, sin esperar la pausa.
    #[cfg(test)]
    pub(super) fn flush_preview(&mut self) {
        if self.preview.pending {
            self.request_preview();
        }
    }

    /// Llegaron respuestas (`WM_APP_PREVIEW`): se queda la del último pedido.
    pub(super) fn on_preview_reply(&mut self) {
        let mut got = false;
        while let Ok(r) = self.preview.rx.try_recv() {
            if r.generation == self.preview.generation && self.preview.subject.is_some() {
                self.preview.info = Some(r.info);
                got = true;
            }
        }
        if got {
            self.redraw();
        }
    }

    /// Lo que dibuja la columna ahora.
    pub(super) fn preview_shown(&self) -> Shown {
        let env = sys::env();
        let tab = self.panes[self.active].tab();
        let list = &tab.list;
        let info = self.preview.info.clone();
        let selected: Vec<usize> = list.selected_indices().collect();
        if selected.len() > 1 {
            let entries: Vec<&Entry> = selected.iter().map(|&i| &list.entries[i]).collect();
            let kind = entries.first().map(|e| e.type_name.clone()).filter(|k| entries.iter().all(|e| e.type_name == *k));
            let bytes = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
            let keys = selected.iter().take(3).map(|&i| (source_of(tab, i), 0)).collect();
            let icons = selected.iter().take(3).map(|&i| list.entries[i].icon).collect();
            return Shown::Many { count: selected.len(), keys, icons, bytes, kind };
        }
        let idx = match selected.first() {
            Some(&i) => Some(i),
            None if matches!(tab.location, Location::Computer) => list.focused_index(),
            None => None,
        };
        let accessed = info.as_ref().map_or(0, |i| i.accessed);
        let (name, key, icon, mut rows, tags, is_dir) = match idx.and_then(|i| list.entries.get(i).map(|e| (i, e))) {
            Some((i, e)) => {
                let meta = list.meta_of(i);
                let rows = detail_rows(e, meta, accessed, env);
                let tags = meta.map(|m| m.tags.clone()).unwrap_or_default();
                (e.display.clone(), (source_of(tab, i), 0), e.icon, rows, tags, e.is_dir)
            }
            None => {
                let Location::Path(p) = &tab.location else { return Shown::Empty };
                // La carpeta abierta: no es un elemento de la lista; sus fechas llegan con la vista previa.
                let mut rows = vec![("Tipo", sys::file_system().folder_type().to_string())];
                if let Some(i) = &info {
                    for (label, ft) in [("Creado", i.created), ("Modificado", i.modified), ("Último acceso", i.accessed)] {
                        if ft != 0 {
                            rows.push((label, env.format_date(ft)));
                        }
                    }
                }
                let icon = sys::file_system().folder_icon().unwrap_or(0);
                (tab.location.title(), (thumbs::Source::Path(p.clone()), 0), icon, rows, Vec::new(), true)
            }
        };
        // Lo que sale de la lectura: cantidad y tamaño de una carpeta, espacio de un disco, estado.
        let mut extra: Vec<(&'static str, String)> = Vec::new();
        match info.as_ref().map(|i| &i.body) {
            Some(PreviewBody::Folder { items }) => extra.push(("Contenido", items_text(*items))),
            Some(PreviewBody::Drive { total, free }) => {
                extra.push(("Espacio libre", env.format_size_short(*free)));
                extra.push(("Capacidad", env.format_size_short(*total)));
            }
            _ => {}
        }
        if is_dir && !matches!(info.as_ref().map(|i| &i.body), Some(PreviewBody::Drive { .. })) {
            let path = match idx {
                Some(i) => tab.entry_path(i),
                None => tab.location.target().unwrap_or_default(),
            };
            if let Some(size) = self.xtree.as_ref().and_then(|x| x.size_of(&path)) {
                extra.push(("Tamaño", env.format_size_short(size)));
            }
        }
        let at = rows.iter().position(|(l, _)| *l == "Tipo").map_or(0, |i| i + 1);
        for (k, row) in extra.into_iter().enumerate() {
            rows.insert(at + k, row);
        }
        match info.as_ref().map(|i| &i.body) {
            Some(PreviewBody::CloudOnly) => rows.push(("Estado", CLOUD_TEXT.to_string())),
            Some(PreviewBody::Unreadable) => rows.push(("Estado", UNREADABLE_TEXT.to_string())),
            _ => {}
        }
        // Un lugar sin ruta de disco no se pide: no hay vista previa (el ícono del tipo).
        let info = if self.preview.subject.is_none() { Some(PreviewInfo::new(PreviewBody::Nothing)) } else { info };
        Shown::Item { name, key, icon, info, rows, tags }
    }
}

/// Manda `WM_APP_PREVIEW_DUE` a la ventana dentro de `ms` milisegundos.
fn post_preview_due(hwnd: Handle, ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        hwnd.post(WM_APP_PREVIEW_DUE, 0, 0);
    });
}

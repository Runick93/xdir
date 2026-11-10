//! Columnas de metadatos (Dimensiones, Duración, Autor, Etiquetas, Artista y Álbum): sus textos,
//! sus grupos y qué se le pide al sistema (`sys::Metadata`). Los datos llegan en lotes en otro
//! momento; mientras tanto la celda queda vacía y, al agrupar o filtrar, el elemento está en
//! «Leyendo…». Se guardan por ruta y fecha de modificación mientras la carpeta está abierta.

use std::ffi::OsString;
use std::sync::Arc;

use super::groups::{GroupKey, PINNED_LAST};
use super::{Column, FileList};
use crate::fs::{Entry, Meta, MetaField};

/// Grupo de los elementos cuyos metadatos todavía no llegaron: siempre al final.
pub const READING: &str = "Leyendo…";
/// Nota del desplegable de filtros mientras faltan metadatos.
pub const READING_NOTE: &str = "Leyendo datos…";

/// Metadatos vacíos: los de los elementos que no tienen de dónde leerlos (fuera de una carpeta de
/// disco).
static EMPTY: Meta = Meta { width: None, height: None, duration_ms: None, authors: Vec::new(), tags: Vec::new(), artist: None, album: None };

impl Column {
    /// Campo de metadatos de la columna, si es una de las columnas nuevas.
    pub fn meta_field(self) -> Option<MetaField> {
        match self {
            Column::Dimensions => Some(MetaField::Dimensions),
            Column::Duration => Some(MetaField::Duration),
            Column::Authors => Some(MetaField::Authors),
            Column::Tags => Some(MetaField::Tags),
            Column::Artist => Some(MetaField::Artist),
            Column::Album => Some(MetaField::Album),
            _ => None,
        }
    }

    /// ¿Sus valores salen de `sys::Metadata`?
    pub fn is_meta(self) -> bool {
        self.meta_field().is_some()
    }
}

/// Duración como h:mm:ss («0:03:21»).
pub fn duration_text(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Texto de la celda de una columna de metadatos (vacío sin datos).
pub fn meta_text(col: Column, m: Option<&Meta>) -> String {
    let Some(m) = m else { return String::new() };
    match col {
        Column::Dimensions => match (m.width, m.height) {
            (Some(w), Some(h)) => format!("{w} × {h}"),
            _ => String::new(),
        },
        Column::Duration => m.duration_ms.map(duration_text).unwrap_or_default(),
        Column::Authors => m.authors.join("; "),
        Column::Tags => m.tags.join("; "),
        Column::Artist => m.artist.clone().unwrap_or_default(),
        Column::Album => m.album.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Píxeles de una imagen (para ordenar por dimensiones).
fn pixels(m: Option<&Meta>) -> Option<u64> {
    let m = m?;
    Some(u64::from(m.width?) * u64::from(m.height?))
}

/// Orden por una columna de metadatos (sin datos, primero).
pub(super) fn meta_cmp(col: Column, a: Option<&Meta>, b: Option<&Meta>) -> std::cmp::Ordering {
    match col {
        Column::Dimensions => pixels(a).cmp(&pixels(b)),
        Column::Duration => a.and_then(|m| m.duration_ms).cmp(&b.and_then(|m| m.duration_ms)),
        _ => crate::format::caseless_cmp(&meta_text(col, a), &meta_text(col, b)),
    }
}

fn key(order: i64, label: &str) -> GroupKey {
    GroupKey { order, label: label.to_string() }
}

/// Un grupo por valor; sin valor, «Sin valor» al final.
fn value_group(v: &str) -> GroupKey {
    if v.is_empty() { key(PINNED_LAST, "Sin valor") } else { key(1, v) }
}

/// Grupo de una columna de metadatos. Sin datos todavía, «Leyendo…» (después de «Sin valor»).
pub(super) fn meta_group(col: Column, m: Option<&Meta>) -> GroupKey {
    let Some(m) = m else { return key(PINNED_LAST + 1, READING) };
    match col {
        // Por el lado más largo de la imagen.
        Column::Dimensions => match (m.width, m.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => match w.max(h) {
                0..=640 => key(0, "Pequeña"),
                641..=1920 => key(1, "Mediana"),
                1921..=4096 => key(2, "Grande"),
                _ => key(3, "Muy grande"),
            },
            _ => value_group(""),
        },
        Column::Duration => match m.duration_ms {
            None => value_group(""),
            Some(ms) if ms < 60_000 => key(0, "Muy corta (< 1 min)"),
            Some(ms) if ms <= 5 * 60_000 => key(1, "Corta (1–5 min)"),
            Some(ms) if ms <= 30 * 60_000 => key(2, "Media (5–30 min)"),
            Some(_) => key(3, "Larga (> 30 min)"),
        },
        // Varias etiquetas: el grupo de la primera en orden alfabético.
        Column::Tags => value_group(m.tags.iter().min_by(|a, b| crate::format::caseless_cmp(a, b)).map_or("", |t| t.as_str())),
        _ => value_group(&meta_text(col, Some(m))),
    }
}

/// ¿Pasa `m` el filtro de la columna con esos grupos tildados? Las etiquetas: si cualquiera de
/// ellas está tildada.
pub(super) fn meta_passes(col: Column, m: Option<&Meta>, labels: &[String]) -> bool {
    match (col, m) {
        (Column::Tags, Some(m)) if !m.tags.is_empty() => m.tags.iter().any(|t| labels.contains(t)),
        _ => {
            let k = meta_group(col, m);
            labels.contains(&k.label)
        }
    }
}

/// Grupos de un elemento para las opciones del filtro: uno por etiqueta en Etiquetas; en las
/// demás, el de `meta_group`.
pub(super) fn meta_option_keys(col: Column, m: Option<&Meta>) -> Vec<GroupKey> {
    match (col, m) {
        (Column::Tags, Some(m)) if !m.tags.is_empty() => {
            let mut tags: Vec<&String> = m.tags.iter().collect();
            tags.sort_unstable();
            tags.dedup();
            tags.into_iter().map(|t| key(1, t)).collect()
        }
        _ => vec![meta_group(col, m)],
    }
}

/// Metadatos de la entrada `i`: vacíos si la lista no tiene de dónde leerlos.
pub(super) fn meta_at(metas: &[Option<Arc<Meta>>], enabled: bool, i: usize) -> Option<&Meta> {
    if !enabled {
        return Some(&EMPTY);
    }
    metas.get(i).and_then(|m| m.as_deref())
}

/// Clave de la caché: ruta (o nombre dentro de la carpeta) y fecha de modificación.
fn cache_key(e: &Entry) -> (OsString, u64) {
    (e.full.clone().unwrap_or_else(|| e.name.clone()), e.modified)
}

impl FileList {
    /// Metadatos leídos del elemento (índice en `entries`), si ya llegaron.
    pub fn meta_of(&self, idx: usize) -> Option<&Meta> {
        meta_at(&self.meta, self.meta_enabled, idx)
    }

    /// ¿Hace falta la carpeta entera? (se agrupa, se ordena o se filtra por una columna de
    /// metadatos, o su desplegable de filtros está abierto).
    fn meta_whole(&self) -> bool {
        self.group_by.is_some_and(Column::is_meta)
            || self.sort.is_meta()
            || self.filters.iter().any(|(c, _)| c.is_meta())
            || self.filter_menu().is_some_and(Column::is_meta)
    }

    /// Elementos (índices en `entries`) cuyos metadatos hay que pedir ahora: los que se ven si hay
    /// una columna de metadatos a la vista, o todos si se agrupa, ordena o filtra por una. Quedan
    /// marcados como pedidos: no se vuelven a pedir.
    pub fn take_meta_request(&mut self) -> Vec<usize> {
        if !self.meta_enabled {
            return Vec::new();
        }
        let whole = self.meta_whole();
        let shown = self.view.has_header() && self.columns.iter().any(|c| c.col.is_meta());
        if !whole && !shown {
            return Vec::new();
        }
        let n = self.entries.len();
        self.meta.resize(n, None);
        self.meta_asked.resize(n, false);
        let candidates: Vec<usize> = if whole {
            (0..n).filter(|&i| self.shows(&self.entries[i])).collect()
        } else if self.view_len() > 0.0 && !self.visible.is_empty() {
            let (first, last) = self.visible_range();
            self.visible[first..last].iter().map(|&v| v as usize).collect()
        } else {
            Vec::new()
        };
        candidates.into_iter().filter(|&i| !std::mem::replace(&mut self.meta_asked[i], true)).collect()
    }

    /// Llegó un lote de metadatos: se guardan y, si se agrupa, ordena o filtra por una columna de
    /// metadatos, se rearma lo que se ve. El foco se queda en su elemento y el desplazamiento no
    /// se mueve (salvo que ya no entre).
    pub fn set_meta(&mut self, batch: Vec<(usize, Meta)>) {
        let n = self.entries.len();
        self.meta.resize(n, None);
        self.meta_asked.resize(n, false);
        let mut changed = Vec::new();
        for (i, m) in batch {
            let Some(e) = self.entries.get(i) else { continue };
            let m = Arc::new(m);
            self.meta_cache.insert(cache_key(e), m.clone());
            self.meta[i] = Some(m);
            self.meta_asked[i] = true;
            changed.push(i);
        }
        if changed.is_empty() {
            return;
        }
        let group = self.group_by.filter(|c| c.is_meta());
        if let Some(col) = group
            && self.keys_for == Some(col)
            && self.entry_keys.len() == n
        {
            for &i in &changed {
                let k = meta_group(col, self.meta_of(i));
                self.entry_keys[i] = self.intern_key(k);
            }
        }
        let focus = self.focused_index();
        if self.sort.is_meta() || self.filters.iter().any(|(c, _)| c.is_meta()) {
            self.rebuild(focus);
            self.clamp_scroll();
        } else if group.is_some() {
            self.regroup();
            self.restore_focus(focus);
            self.clamp_scroll();
        } else if self.filter_menu().is_some_and(Column::is_meta) {
            self.refresh_filter_menu();
        }
    }

    /// Metadatos de los elementos nuevos de una recarga: los que ya se tenían (misma ruta y fecha)
    /// se toman de la caché; el resto se vuelve a pedir.
    pub(super) fn meta_from_cache(&mut self) {
        self.meta = self.entries.iter().map(|e| self.meta_cache.get(&cache_key(e)).cloned()).collect();
        self.meta_asked = self.meta.iter().map(Option::is_some).collect();
    }

    /// ¿Faltan metadatos de elementos que cuentan en el desplegable de filtros de la columna?
    pub(super) fn filter_reading(&self, col: Column) -> bool {
        col.is_meta() && (0..self.entries.len()).any(|i| self.meta_of(i).is_none() && self.shows(&self.entries[i]))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, channel};

    use super::*;
    use crate::fs::{LoadEvent, LoadMsg, Location};
    use crate::geom::rect;
    use crate::list::{Row, View};
    use crate::sys::Handle;
    use crate::sys::testing::{FakeMetadata, RecordingPainter, install_fakes};
    use crate::tab::Tab;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.into(),
            full: None,
            group: 0,
            pidl: None,
            display: name.into(),
            sort_key: name.to_lowercase(),
            is_dir: false,
            size: 1,
            modified: 0,
            created: 0,
            attrs: 0,
            type_name: Arc::from("Archivo"),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    /// Pestaña en `dir` (sin cargarla) con esos archivos, y el canal de carga.
    fn tab_with(dir: &str, names: &[String]) -> (Tab, std::sync::mpsc::Sender<LoadEvent>, Receiver<LoadEvent>) {
        install_fakes();
        FakeMetadata::set_delay(None);
        FakeMetadata::take_requests();
        let (tx, rx) = channel();
        let mut tab = Tab::new(11, Location::Path(PathBuf::from(dir)), false);
        tab.list.append(names.iter().map(|n| entry(n)).collect());
        (tab, tx, rx)
    }

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("f{i:03}.jpg")).collect()
    }

    fn painter() -> RecordingPainter {
        RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)))
    }

    fn draw(tab: &mut Tab) -> Vec<String> {
        let p = painter();
        tab.list.draw(&p, rect(0.0, 0.0, 1400.0, 400.0), true);
        p.texts().into_iter().map(|t| t.1).collect()
    }

    /// Entrega lo que haya llegado por el canal.
    fn deliver(tab: &mut Tab, rx: &Receiver<LoadEvent>) {
        while let Ok(ev) = rx.try_recv() {
            tab.on_load(ev);
        }
    }

    fn headers(tab: &Tab) -> Vec<String> {
        tab.list.rows().iter().filter_map(|r| if let Row::Header { key, count, .. } = r { Some(format!("{} ({count})", key.label)) } else { None }).collect()
    }

    fn requested() -> Vec<usize> {
        FakeMetadata::take_requests().iter().map(|r| r.0.len()).collect()
    }

    fn dims(w: u32, h: u32) -> Meta {
        Meta { width: Some(w), height: Some(h), ..Meta::default() }
    }

    fn duration(ms: u64) -> Meta {
        Meta { duration_ms: Some(ms), ..Meta::default() }
    }

    #[test]
    fn duration_format() {
        assert_eq!(duration_text(201_000), "0:03:21");
        assert_eq!(duration_text(3_723_000), "1:02:03");
        assert_eq!(duration_text(0), "0:00:00");
    }

    /// Sin columnas de metadatos (ni agrupar ni filtrar por una) no se pide nada; con una a la
    /// vista, se piden los elementos que se ven, todos los campos.
    #[test]
    fn metadata_only_requested_when_column_visible() {
        let (mut tab, tx, _rx) = tab_with("/meta-visible", &names(100));
        draw(&mut tab);
        tab.request_metadata(Handle(0), &tx);
        assert!(FakeMetadata::take_requests().is_empty());
        tab.list.set_view(View::Content);
        draw(&mut tab);
        tab.request_metadata(Handle(0), &tx);
        assert!(FakeMetadata::take_requests().is_empty());

        tab.list.set_view(View::Details);
        tab.list.toggle_column(Column::Dimensions);
        draw(&mut tab);
        tab.request_metadata(Handle(0), &tx);
        let reqs = FakeMetadata::take_requests();
        assert_eq!(reqs.len(), 1);
        let (items, fields) = &reqs[0];
        // Solo lo que entra en 400 de alto, no los 100.
        assert!(!items.is_empty() && items.len() < 30, "{}", items.len());
        assert_eq!(items[0], (0, PathBuf::from("/meta-visible").join("f000.jpg")));
        assert!(fields.contains(&MetaField::Dimensions));
    }

    /// Con «Dimensiones» a la vista se piden los visibles; al llegar, la celda dice «4032 × 3024».
    /// Lo pedido no se vuelve a pedir.
    #[test]
    fn metadata_cells_fill_when_arriving() {
        let (mut tab, tx, rx) = tab_with("/meta-celdas", &names(3));
        FakeMetadata::serve(&PathBuf::from("/meta-celdas/f001.jpg"), dims(4032, 3024));
        tab.list.toggle_column(Column::Dimensions);
        assert!(!draw(&mut tab).iter().any(|t| t == "4032 × 3024"));
        tab.request_metadata(Handle(0), &tx);
        assert_eq!(requested(), [3]);
        // Hasta que se entrega, la celda sigue vacía.
        assert!(!draw(&mut tab).iter().any(|t| t == "4032 × 3024"));
        deliver(&mut tab, &rx);
        assert!(draw(&mut tab).iter().any(|t| t == "4032 × 3024"));
        tab.request_metadata(Handle(0), &tx);
        assert!(requested().is_empty());
    }

    /// Agrupar por duración pide la carpeta entera (en lotes de 200), aunque la columna no se
    /// vea; antes de llegar todo está en «Leyendo…», después en sus grupos.
    #[test]
    fn group_by_duration_requests_whole_folder_and_shows_reading() {
        let (mut tab, tx, rx) = tab_with("/meta-duracion", &names(300));
        FakeMetadata::serve(&PathBuf::from("/meta-duracion/f000.jpg"), duration(30_000));
        FakeMetadata::serve(&PathBuf::from("/meta-duracion/f001.jpg"), duration(201_000));
        FakeMetadata::serve(&PathBuf::from("/meta-duracion/f002.jpg"), duration(3_723_000));
        tab.list.set_group(Some(Column::Duration));
        assert_eq!(headers(&tab), ["Leyendo… (300)"]);
        draw(&mut tab);
        tab.request_metadata(Handle(0), &tx);
        assert_eq!(requested(), [200, 100]);
        assert_eq!(headers(&tab), ["Leyendo… (300)"]);
        deliver(&mut tab, &rx);
        assert_eq!(headers(&tab), ["Muy corta (< 1 min) (1)", "Corta (1–5 min) (1)", "Larga (> 30 min) (1)", "Sin valor (297)"]);
    }

    /// Un lote de una carga anterior (otra generación) no cambia nada.
    #[test]
    fn stale_metadata_batch_ignored() {
        let (mut tab, _tx, _rx) = tab_with("/meta-vieja", &names(3));
        tab.list.toggle_column(Column::Dimensions);
        let stale = LoadEvent { panel: 11, generation: 41, msg: LoadMsg::Metadata(vec![(0, dims(10, 10))]) };
        assert!(!tab.on_load(stale));
        assert_eq!(tab.list.meta_of(0), None);
        assert!(!draw(&mut tab).iter().any(|t| t == "10 × 10"));
    }

    /// Al llegar los metadatos el foco se queda en su elemento y la lista no se desplaza, también
    /// agrupando por una columna de metadatos.
    #[test]
    fn metadata_arrival_keeps_focus_and_scroll() {
        let (mut tab, tx, rx) = tab_with("/meta-foco", &names(100));
        for i in 0..100 {
            FakeMetadata::serve(&PathBuf::from(format!("/meta-foco/f{i:03}.jpg")), dims(100 + i, 100));
        }
        tab.list.toggle_column(Column::Dimensions);
        draw(&mut tab);
        assert!(tab.list.focus_name(&"f060.jpg".into()));
        draw(&mut tab);
        let (focus, scroll) = (tab.list.focused_index(), tab.list.scroll);
        assert!(scroll > 0.0);
        tab.request_metadata(Handle(0), &tx);
        deliver(&mut tab, &rx);
        draw(&mut tab);
        assert_eq!((tab.list.focused_index(), tab.list.scroll), (focus, scroll));

        // Agrupando por duración (todos quedan en «Sin valor» al llegar): el foco no se pierde.
        tab.list.set_group(Some(Column::Duration));
        draw(&mut tab);
        let scroll = tab.list.scroll;
        tab.request_metadata(Handle(0), &tx);
        deliver(&mut tab, &rx);
        assert_eq!(tab.list.focused_index(), focus);
        assert_eq!(tab.list.scroll, scroll);
    }

    /// Etiquetas: para agrupar, la primera en orden alfabético; para filtrar, cualquiera de ellas.
    #[test]
    fn tags_group_first_tag_and_filter_any() {
        let files: Vec<String> = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"].iter().map(|s| s.to_string()).collect();
        let (mut tab, tx, rx) = tab_with("/meta-etiquetas", &files);
        let tags = |t: &[&str]| Meta { tags: t.iter().map(|s| s.to_string()).collect(), ..Meta::default() };
        FakeMetadata::serve(&PathBuf::from("/meta-etiquetas/a.jpg"), tags(&["viaje", "Bariloche"]));
        FakeMetadata::serve(&PathBuf::from("/meta-etiquetas/b.jpg"), tags(&["viaje"]));
        FakeMetadata::serve(&PathBuf::from("/meta-etiquetas/c.jpg"), tags(&["lago", "viaje"]));
        tab.list.set_group(Some(Column::Tags));
        draw(&mut tab);
        tab.request_metadata(Handle(0), &tx);
        deliver(&mut tab, &rx);
        assert_eq!(headers(&tab), ["Bariloche (1)", "lago (1)", "viaje (1)", "Sin valor (1)"]);

        let options: Vec<(String, usize)> = tab.list.filter_options(Column::Tags).into_iter().map(|o| (o.0.label, o.1)).collect();
        assert_eq!(options, [("Bariloche".to_string(), 1), ("lago".to_string(), 1), ("viaje".to_string(), 3), ("Sin valor".to_string(), 1)]);
        tab.list.set_filter(Column::Tags, vec!["viaje".into()]);
        assert_eq!(tab.list.total(), 3);
        tab.list.set_filter(Column::Tags, vec!["lago".into()]);
        assert_eq!(tab.list.total(), 1);
    }

    /// El desplegable de filtros de una columna de metadatos pide la carpeta entera; mientras
    /// faltan datos muestra las cuentas parciales con «Leyendo datos…», y se actualiza al llegar.
    #[test]
    fn filter_dropdown_shows_reading_note() {
        let (mut tab, tx, rx) = tab_with("/meta-filtro", &names(300));
        FakeMetadata::serve(&PathBuf::from("/meta-filtro/f000.jpg"), duration(201_000));
        tab.list.toggle_column(Column::Duration);
        draw(&mut tab);
        // Llegan solo los visibles.
        tab.request_metadata(Handle(0), &tx);
        deliver(&mut tab, &rx);
        tab.list.open_filter_menu(Column::Duration);
        assert!(tab.list.filter_menu_reading());
        let p = painter();
        tab.list.draw_filter_menu(&p);
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        assert!(texts.iter().any(|t| t == READING_NOTE), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Corta (1–5 min) (1)"), "{texts:?}");
        assert!(!texts.iter().any(|t| t.starts_with(READING)), "{texts:?}");

        let visible = requested().iter().sum::<usize>();
        assert!(visible > 0 && visible < 300, "{visible}");
        // Con el desplegable abierto se pide el resto de la carpeta (sin repetir lo pedido).
        tab.request_metadata(Handle(0), &tx);
        assert_eq!(requested(), [200, 300 - visible - 200]);
        deliver(&mut tab, &rx);
        assert!(!tab.list.filter_menu_reading());
        let p = painter();
        tab.list.draw_filter_menu(&p);
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        assert!(!texts.iter().any(|t| t == READING_NOTE), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Sin valor (299)"), "{texts:?}");
    }
}

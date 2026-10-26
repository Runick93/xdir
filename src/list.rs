//! Lista de archivos, virtualizada: solo se dibuja lo que está a la vista, así que el costo por
//! frame no depende de cuántos archivos tenga la carpeta. En detalles (filas con columnas) o en
//! cuadrícula de íconos y miniaturas.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::format;
use crate::fs::Entry;
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;
use crate::thumbs;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

pub const ROW_H: f32 = 24.0;
pub const HEADER_H: f32 = 30.0;
const ICON: f32 = 16.0;
const SCROLLBAR_W: f32 = 12.0;
const NAME_MIN: f32 = 180.0;
const PAD: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Column {
    Name,
    Modified,
    Created,
    Type,
    Extension,
    Size,
    Attributes,
}

/// Todas las columnas: (columna, título, identificador para la sesión, ancho inicial).
pub const COLUMNS: [(Column, &str, &str, f32); 7] = [
    (Column::Name, "Nombre", "nombre", 0.0),
    (Column::Modified, "Fecha de modificación", "modificado", 160.0),
    (Column::Created, "Fecha de creación", "creado", 160.0),
    (Column::Type, "Tipo", "tipo", 160.0),
    (Column::Extension, "Extensión", "extension", 80.0),
    (Column::Size, "Tamaño", "tamano", 96.0),
    (Column::Attributes, "Atributos", "atributos", 80.0),
];

impl Column {
    pub fn title(self) -> &'static str {
        COLUMNS.iter().find(|c| c.0 == self).unwrap().1
    }

    pub fn id(self) -> &'static str {
        COLUMNS.iter().find(|c| c.0 == self).unwrap().2
    }

    pub fn from_id(id: &str) -> Option<Column> {
        COLUMNS.iter().find(|c| c.2 == id).map(|c| c.0)
    }

    fn default_width(self) -> f32 {
        COLUMNS.iter().find(|c| c.0 == self).unwrap().3
    }

    /// Texto alineado a la derecha (números).
    fn right_aligned(self) -> bool {
        self == Column::Size
    }
}

/// Columna a la vista con su ancho. El nombre no tiene ancho propio: ocupa lo que sobra.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColumnSpec {
    pub col: Column,
    pub width: f32,
}

/// Columnas de fábrica, de izquierda a derecha.
pub fn default_columns() -> Vec<ColumnSpec> {
    [Column::Name, Column::Modified, Column::Type, Column::Size]
        .into_iter()
        .map(|col| ColumnSpec { col, width: col.default_width() })
        .collect()
}

/// Un elemento de la lista visto desde afuera (lectores de pantalla).
pub struct ItemInfo {
    pub name: String,
    /// Tipo, tamaño y fecha en una frase.
    pub help: String,
    pub is_dir: bool,
    pub selected: bool,
    /// En DIPs, relativo a la ventana.
    pub rect: Rect,
    /// Fuera del área visible (scrolleado).
    pub offscreen: bool,
}

/// Cómo se muestran los elementos: filas con columnas, o una cuadrícula de íconos/miniaturas.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Details,
    Icons,
    LargeIcons,
    Thumbnails,
}

/// (vista, nombre, identificador para la sesión, lado del ícono en DIPs).
pub const VIEWS: [(View, &str, &str, f32); 4] = [
    (View::Details, "Detalles", "detalles", 16.0),
    (View::Icons, "Íconos medianos", "iconos", 48.0),
    (View::LargeIcons, "Íconos grandes", "iconos_grandes", 96.0),
    (View::Thumbnails, "Miniaturas", "miniaturas", 160.0),
];

impl View {
    pub fn index(self) -> usize {
        VIEWS.iter().position(|v| v.0 == self).unwrap()
    }

    pub fn id(self) -> &'static str {
        VIEWS[self.index()].2
    }

    pub fn from_id(id: &str) -> Option<View> {
        VIEWS.iter().find(|v| v.2 == id).map(|v| v.0)
    }

    pub fn name(self) -> &'static str {
        VIEWS[self.index()].1
    }

    fn icon(self) -> f32 {
        VIEWS[self.index()].3
    }

    pub fn is_grid(self) -> bool {
        self != View::Details
    }
}

/// Alto del nombre debajo de una miniatura (dos líneas) y márgenes de la cuadrícula.
const LABEL_H: f32 = 34.0;
const GRID_PAD: f32 = 8.0;

const MIN_COL_W: f32 = 48.0;
/// Distancia al borde de un encabezado que cuenta como "agarrar el borde" para cambiar el ancho.
const EDGE_GRAB: f32 = 5.0;

/// Arrastre en el encabezado.
#[derive(Clone, Copy, PartialEq, Debug)]
enum HeaderDrag {
    /// Cambiando el ancho de la columna `index` (en `columns`). `invert`: el borde agarrado es el
    /// izquierdo (a la derecha del nombre), así que mover a la izquierda la agranda.
    Resize { index: usize, start_x: f32, start_w: f32, invert: bool },
    /// Botón apretado sobre un encabezado: clic (ordenar) o, si se mueve, reordenar.
    Press { index: usize, start_x: f32, moved: bool },
}

/// Atributos como letras, al estilo del Explorador: R (solo lectura), H (oculto), S (sistema),
/// A (archivo), C (comprimido), E (cifrado).
fn attr_letters(attrs: u32) -> String {
    [(0x1, 'R'), (0x2, 'H'), (0x4, 'S'), (0x20, 'A'), (0x800, 'C'), (0x4000, 'E')]
        .iter()
        .filter(|(bit, _)| attrs & bit != 0)
        .map(|(_, c)| *c)
        .collect()
}

/// Extensión en minúsculas (sin punto); vacía para carpetas.
fn extension_of(e: &Entry) -> &str {
    if e.is_dir {
        return "";
    }
    match e.sort_key.rfind('.') {
        Some(i) if i > 0 => &e.sort_key[i + 1..],
        _ => "",
    }
}

/// Lo que la lista le pide al panel después de un evento.
#[derive(PartialEq, Debug)]
pub enum Action {
    None,
    Redraw,
    /// Abrir el elemento (índice en `entries`).
    Open(usize),
    /// Empezar a arrastrar la selección.
    BeginDrag,
    /// Cambiaron las columnas (orden, ancho): la app las pasa a las demás listas.
    ColumnsChanged,
}

pub struct FileList {
    pub entries: Vec<Entry>,
    /// Índices de `entries` visibles, ya filtrados y ordenados.
    visible: Vec<u32>,
    /// Selección por índice de `entries`.
    selected: Vec<bool>,
    /// Posición en `visible` del elemento con foco.
    focus: Option<usize>,
    /// El foco lo puso la lista (primera fila), no el usuario: durante la carga se queda arriba.
    auto_focus: bool,
    anchor: Option<usize>,
    /// Scroll que se dibuja; `scroll_target` es adonde va (ver `tick`, F-8.4). Los clics y teclas
    /// actúan sobre el destino, no sobre lo que falta recorrer.
    scroll: f32,
    scroll_target: f32,
    pub sort: Column,
    pub descending: bool,
    pub show_hidden: bool,
    hover: Option<usize>,
    typeahead: String,
    typeahead_at: Option<Instant>,
    /// Área de la última vez que se dibujó (para hit-testing).
    area: Rect,
    /// Arrastre de la barra de desplazamiento: distancia entre el mouse y el borde del pulgar.
    thumb_drag: Option<f32>,
    /// Clic sobre un elemento ya seleccionado: se resuelve al soltar (si no fue un arrastre).
    pending_select: Option<usize>,
    /// Dónde se apretó el botón sobre un elemento, para detectar el inicio de un arrastre.
    press: Option<(f32, f32)>,
    /// Carpeta resaltada como destino mientras se arrastra algo encima (índice en `entries`).
    pub drop_target: Option<usize>,
    /// Filtro rápido tal como se escribió (vacío: sin filtro) y sus partes ya en minúsculas.
    filter: String,
    filter_parts: Vec<String>,
    /// Columnas a la vista, de izquierda a derecha.
    pub columns: Vec<ColumnSpec>,
    pub view: View,
    /// Carpeta de la lista (para pedir las miniaturas por ruta); la pone quien dibuja.
    pub folder: Option<std::path::PathBuf>,
    header_drag: Option<HeaderDrag>,
}

/// ¿El nombre (en minúsculas) pasa el filtro? Cada parte separada por `;` es un patrón: con `*`
/// o `?` se compara el nombre entero como comodín; si no, alcanza con que lo contenga.
fn filter_matches(parts: &[String], name: &str) -> bool {
    parts.is_empty()
        || parts.iter().any(|p| if p.contains(['*', '?']) { wildcard(p, name) } else { name.contains(p.as_str()) })
}

/// Comodines de Windows: `*` cualquier secuencia, `?` un carácter. Sin recursión: vuelve al
/// último `*` cuando algo no coincide.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

impl Default for FileList {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            visible: Vec::new(),
            selected: Vec::new(),
            focus: None,
            auto_focus: true,
            anchor: None,
            scroll: 0.0,
            scroll_target: 0.0,
            sort: Column::Name,
            descending: false,
            show_hidden: false,
            hover: None,
            typeahead: String::new(),
            typeahead_at: None,
            area: rect(0.0, 0.0, 0.0, 0.0),
            thumb_drag: None,
            pending_select: None,
            press: None,
            drop_target: None,
            filter: String::new(),
            filter_parts: Vec::new(),
            columns: default_columns(),
            view: View::Details,
            folder: None,
            header_drag: None,
        }
    }
}

fn compare(a: &Entry, b: &Entry, sort: Column, desc: bool) -> Ordering {
    // Primero el grupo (secciones de "Este equipo"), después carpetas arriba, en cualquier orden.
    a.group.cmp(&b.group).then_with(|| b.is_dir.cmp(&a.is_dir)).then_with(|| {
        let ord = match sort {
            Column::Name => Ordering::Equal,
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Created => a.created.cmp(&b.created),
            Column::Type => format::caseless_cmp(&a.type_name, &b.type_name),
            Column::Extension => extension_of(a).cmp(extension_of(b)),
            Column::Size => a.size.cmp(&b.size),
            Column::Attributes => attr_letters(a.attrs).cmp(&attr_letters(b.attrs)),
        }
        .then_with(|| format::natural_cmp(&a.sort_key, &b.sort_key));
        if desc { ord.reverse() } else { ord }
    })
}

impl FileList {
    /// ¿Se ve el elemento con la configuración actual (ocultos y filtro)?
    fn shows(&self, e: &Entry) -> bool {
        (self.show_hidden || !e.is_hidden()) && filter_matches(&self.filter_parts, &e.sort_key)
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Aplica el filtro rápido al instante, conservando el foco si el elemento sigue a la vista.
    pub fn set_filter(&mut self, text: &str) {
        if text == self.filter {
            return;
        }
        self.filter = text.to_string();
        self.filter_parts = text.split(';').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
        let focus = self.focused_index();
        self.rebuild(focus);
        self.scroll = 0.0;
        self.scroll_target = 0.0;
        if let Some(f) = self.focus {
            self.ensure_visible(f);
            self.snap_scroll();
        }
    }

    /// Elementos que se verían sin el filtro (para mostrar "N de M").
    pub fn unfiltered_count(&self) -> usize {
        self.entries.iter().filter(|e| self.show_hidden || !e.is_hidden()).count()
    }

    /// Vacía la lista (navegación a otra carpeta). El filtro no pasa a la carpeta nueva.
    pub fn clear(&mut self) {
        let (columns, view) = (std::mem::take(&mut self.columns), self.view);
        *self = Self { sort: self.sort, descending: self.descending, show_hidden: self.show_hidden, columns, view, ..Default::default() };
    }

    /// Agrega un lote durante la carga progresiva. Solo se ordena el lote nuevo y se mezcla
    /// con lo que ya estaba ordenado: reordenar todo con cada lote no escala a 100.000 archivos.
    pub fn append(&mut self, batch: Vec<Entry>) {
        let focus_entry = self.focused_index();
        let start = self.entries.len() as u32;
        self.selected.extend(std::iter::repeat_n(false, batch.len()));
        self.entries.extend(batch);

        let mut new: Vec<u32> = (start..self.entries.len() as u32).filter(|&i| self.shows(&self.entries[i as usize])).collect();
        let entries = &self.entries;
        let (sort, desc) = (self.sort, self.descending);
        let cmp = |a: &u32, b: &u32| compare(&entries[*a as usize], &entries[*b as usize], sort, desc);
        new.sort_unstable_by(cmp);

        let old = std::mem::take(&mut self.visible);
        let mut merged = Vec::with_capacity(old.len() + new.len());
        let (mut i, mut j) = (0, 0);
        while i < old.len() && j < new.len() {
            if cmp(&new[j], &old[i]) == Ordering::Less {
                merged.push(new[j]);
                j += 1;
            } else {
                merged.push(old[i]);
                i += 1;
            }
        }
        merged.extend_from_slice(&old[i..]);
        merged.extend_from_slice(&new[j..]);
        self.visible = merged;

        self.restore_focus(focus_entry);
    }

    /// Reemplaza el contenido (recarga silenciosa) conservando selección, foco y scroll por nombre.
    pub fn replace(&mut self, entries: Vec<Entry>) {
        let selected: std::collections::HashSet<OsString> =
            self.selected_entries().map(|e| e.name.clone()).collect();
        let focus_name = self.focused_index().map(|i| self.entries[i].name.clone());
        self.selected = entries.iter().map(|e| selected.contains(&e.name)).collect();
        self.entries = entries;
        let focus = focus_name.and_then(|n| self.entries.iter().position(|e| e.name == n));
        self.rebuild(focus);
        self.hover = None;
        self.clamp_scroll();
    }

    pub fn set_overlays(&mut self, overlays: &[(u32, u8)]) {
        for &(pos, overlay) in overlays {
            if let Some(e) = self.entries.get_mut(pos as usize) {
                e.overlay = overlay;
            }
        }
    }

    /// Elementos visibles en el último frame cuya superposición no se consultó todavía.
    /// Quedan marcados como consultados.
    pub fn take_unchecked_visible(&mut self) -> Vec<usize> {
        if self.visible.is_empty() || self.view_h() <= 0.0 {
            return Vec::new();
        }
        let (first, last) = self.visible_range();
        let mut out = Vec::new();
        for pos in first..last {
            let idx = self.visible[pos] as usize;
            let e = &mut self.entries[idx];
            if !e.overlay_checked {
                e.overlay_checked = true;
                out.push(idx);
            }
        }
        out
    }

    pub fn set_icons(&mut self, icons: &[(u32, i32)]) {
        for &(pos, icon) in icons {
            if let Some(e) = self.entries.get_mut(pos as usize) {
                e.icon = icon;
            }
        }
    }

    /// Pone el foco en un elemento por nombre (p. ej. la carpeta de la que se subió).
    pub fn focus_name(&mut self, name: &OsString) -> bool {
        let Some(idx) = self.entries.iter().position(|e| &e.name == name) else { return false };
        let Some(pos) = self.visible.iter().position(|&v| v as usize == idx) else { return false };
        self.select_only(pos);
        // Al terminar de cargar una carpeta no se anima: ya está donde tiene que estar.
        self.ensure_visible(pos);
        self.snap_scroll();
        true
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let focus = self.focused_index();
        self.rebuild(focus);
        self.clamp_scroll();
    }

    pub fn set_sort(&mut self, column: Column) {
        if self.sort == column {
            self.descending = !self.descending;
        } else {
            self.sort = column;
            // Fechas y tamaños: lo más nuevo/grande primero, como el Explorador.
            self.descending = matches!(column, Column::Modified | Column::Size);
        }
        let focus = self.focused_index();
        self.rebuild(focus);
        if let Some(f) = self.focus {
            self.ensure_visible(f);
        }
    }

    // ---------- accesibilidad (UI Automation, ver uia.rs) ----------

    /// Posición con foco (en los elementos a la vista).
    pub fn focus_pos(&self) -> Option<usize> {
        self.focus
    }

    /// Lo que un lector de pantalla necesita saber del elemento en la posición `pos`.
    pub fn item_info(&self, pos: usize) -> Option<ItemInfo> {
        let idx = *self.visible.get(pos)? as usize;
        let e = &self.entries[idx];
        let mut help = vec![e.type_name.to_string()];
        if !e.is_dir {
            help.push(format::size_kb(e.size));
        }
        if e.modified != 0 {
            help.push(format!("modificado {}", format::date(e.modified)));
        }
        let rect = self.item_rect(pos);
        let rows = self.rows_area();
        Some(ItemInfo {
            name: e.display.clone(),
            help: help.join(", "),
            is_dir: e.is_dir,
            selected: self.selected[idx],
            rect,
            offscreen: rect.bottom <= rows.top || rect.top >= rows.bottom,
        })
    }

    /// Selecciona solo ese elemento y lo trae a la vista (patrón de selección de UI Automation).
    pub fn select_pos(&mut self, pos: usize) {
        if pos < self.visible.len() {
            self.select_only(pos);
            self.ensure_visible(pos);
        }
    }

    /// Agrega (o saca) el elemento de la selección sin tocar el resto.
    pub fn set_selected_pos(&mut self, pos: usize, on: bool) {
        if let Some(&idx) = self.visible.get(pos) {
            self.auto_focus = false;
            self.selected[idx as usize] = on;
            self.focus = Some(pos);
            self.anchor = Some(pos);
        }
    }

    /// Posiciones seleccionadas, en orden.
    pub fn selected_positions(&self) -> Vec<usize> {
        self.visible.iter().enumerate().filter(|(_, idx)| self.selected[**idx as usize]).map(|(pos, _)| pos).collect()
    }

    /// Elemento (posición) bajo un punto de la ventana, en DIPs.
    pub fn pos_at_point(&self, x: f32, y: f32) -> Option<usize> {
        self.pos_at(x, y)
    }

    /// Índice en `entries` del elemento de esa posición (para abrirlo).
    pub fn entry_of_pos(&self, pos: usize) -> Option<usize> {
        self.visible.get(pos).map(|&i| i as usize)
    }

    pub fn count(&self) -> usize {
        self.visible.len()
    }

    pub fn selected_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i)
    }

    pub fn selected_entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().zip(&self.selected).filter(|(_, s)| **s).map(|(e, _)| e)
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focus.and_then(|p| self.visible.get(p)).map(|&i| i as usize)
    }

    /// Filtra ocultos y filtro rápido, ordena y reubica el foco en `focus_entry`.
    fn rebuild(&mut self, focus_entry: Option<usize>) {
        let shown: Vec<bool> = self.entries.iter().map(|e| self.shows(e)).collect();
        self.visible = (0..self.entries.len() as u32).filter(|&i| shown[i as usize]).collect();
        // Lo que queda fuera se deselecciona para no operar sobre lo que no se ve.
        for (s, &v) in self.selected.iter_mut().zip(&shown) {
            *s &= v;
        }

        let (entries, sort, desc) = (&self.entries, self.sort, self.descending);
        self.visible.sort_unstable_by(|&a, &b| compare(&entries[a as usize], &entries[b as usize], sort, desc));

        self.restore_focus(focus_entry);
    }

    fn restore_focus(&mut self, focus_entry: Option<usize>) {
        self.focus = if self.auto_focus {
            None
        } else {
            focus_entry.and_then(|f| self.visible.iter().position(|&v| v as usize == f))
        };
        if self.focus.is_none() && !self.visible.is_empty() {
            self.focus = Some(0);
            self.auto_focus = true;
        }
        self.anchor = self.focus;
    }

    // ---------- geometría ----------

    // En detalles cada "línea" es una fila; en las cuadrículas, una fila de celdas. Todo lo que
    // ubica elementos pasa por `line_h`, `per_line` e `item_rect`.

    fn header_h(&self) -> f32 {
        if self.view.is_grid() { 0.0 } else { HEADER_H }
    }

    fn rows_area(&self) -> Rect {
        rect(self.area.left, self.area.top + self.header_h(), self.area.right, self.area.bottom)
    }

    fn view_h(&self) -> f32 {
        (self.area.bottom - self.area.top - self.header_h()).max(0.0)
    }

    /// Alto de una línea.
    fn line_h(&self) -> f32 {
        if self.view.is_grid() { self.view.icon() + LABEL_H + 14.0 } else { ROW_H }
    }

    /// Ancho de una celda de la cuadrícula.
    fn cell_w(&self) -> f32 {
        (self.view.icon() + 40.0).max(96.0)
    }

    /// Elementos por línea.
    fn per_line(&self) -> usize {
        if !self.view.is_grid() {
            return 1;
        }
        let usable = self.area.right - self.area.left - SCROLLBAR_W - GRID_PAD * 2.0;
        ((usable / self.cell_w()).floor() as usize).max(1)
    }

    /// Margen de arriba del contenido (la cuadrícula no arranca pegada al borde).
    fn top_pad(&self) -> f32 {
        if self.view.is_grid() { GRID_PAD } else { 0.0 }
    }

    fn content_h(&self) -> f32 {
        let lines = self.visible.len().div_ceil(self.per_line());
        lines as f32 * self.line_h() + self.top_pad() * 2.0
    }

    /// Elementos que entran en una página (Re Pág / Av Pág).
    fn page_rows(&self) -> usize {
        ((self.view_h() / self.line_h()).floor() as usize).max(1) * self.per_line()
    }

    /// Posiciones (en `visible`) a la vista: [primera, última).
    fn visible_range(&self) -> (usize, usize) {
        let (lh, per) = (self.line_h(), self.per_line());
        let first_line = ((self.scroll - self.top_pad()).max(0.0) / lh).floor() as usize;
        let lines = (self.view_h() / lh).ceil() as usize + 1;
        ((first_line * per).min(self.visible.len()), ((first_line + lines) * per).min(self.visible.len()))
    }

    /// Rectángulo de un elemento (fila en detalles, celda en las cuadrículas).
    fn item_rect(&self, pos: usize) -> Rect {
        let rows = self.rows_area();
        if !self.view.is_grid() {
            let y = rows.top + pos as f32 * ROW_H - self.scroll;
            return rect(rows.left, y, rows.right - SCROLLBAR_W, y + ROW_H);
        }
        let per = self.per_line();
        let (line, col) = (pos / per, pos % per);
        // Las celdas se reparten el ancho sobrante, para no dejar un hueco a la derecha.
        let usable = rows.right - rows.left - SCROLLBAR_W - GRID_PAD * 2.0;
        let w = (usable / per as f32).max(self.cell_w());
        let x = rows.left + GRID_PAD + col as f32 * w;
        let y = rows.top + self.top_pad() + line as f32 * self.line_h() - self.scroll;
        rect(x, y, x + w, y + self.line_h())
    }

    /// Elemento bajo el punto (posición en `visible`).
    fn pos_at(&self, x: f32, y: f32) -> Option<usize> {
        let rows = self.rows_area();
        if y < rows.top || y >= rows.bottom || x >= rows.right - SCROLLBAR_W {
            return None;
        }
        if !self.view.is_grid() {
            return self.row_at(y);
        }
        let per = self.per_line();
        let usable = rows.right - rows.left - SCROLLBAR_W - GRID_PAD * 2.0;
        let w = (usable / per as f32).max(self.cell_w());
        let col = ((x - rows.left - GRID_PAD) / w).floor();
        let line = ((y - rows.top - self.top_pad() + self.scroll) / self.line_h()).floor();
        if col < 0.0 || line < 0.0 || col as usize >= per {
            return None;
        }
        let pos = line as usize * per + col as usize;
        (pos < self.visible.len()).then_some(pos)
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_h() - self.view_h()).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
        self.scroll_target = self.scroll_target.clamp(0.0, max);
    }

    /// Lleva el scroll dibujado al destino sin animar.
    fn snap_scroll(&mut self) {
        self.scroll = self.scroll_target;
    }

    /// Lleva el elemento a la vista (el destino del scroll; se llega suave, ver `tick`).
    fn ensure_visible(&mut self, pos: usize) {
        let lh = self.line_h();
        let top = self.top_pad() + (pos / self.per_line()) as f32 * lh;
        if top < self.scroll_target {
            self.scroll_target = (top - self.top_pad()).max(0.0);
        } else if top + lh > self.scroll_target + self.view_h() {
            self.scroll_target = top + lh - self.view_h() + self.top_pad();
        }
        self.clamp_scroll();
        if !crate::anim::enabled() {
            self.snap_scroll();
        }
    }

    /// Avanza el scroll suave `dt` segundos. Devuelve si todavía le falta (hay que pedir otro frame).
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.scroll == self.scroll_target {
            return false;
        }
        self.scroll = if crate::anim::enabled() { crate::anim::smooth(self.scroll, self.scroll_target, dt) } else { self.scroll_target };
        self.scroll != self.scroll_target
    }

    /// Cambia la vista conservando el elemento con foco a la vista.
    pub fn set_view(&mut self, view: View) {
        self.view = view;
        self.header_drag = None;
        self.hover = None;
        self.scroll = 0.0;
        self.scroll_target = 0.0;
        if let Some(f) = self.focus {
            self.ensure_visible(f);
            self.snap_scroll();
        }
    }

    /// Columnas que entran en el ancho disponible, de izquierda a derecha: (columna, izquierda,
    /// derecha). El nombre ocupa lo que dejan las demás (al menos `NAME_MIN`); si no alcanza, se
    /// dejan de mostrar las del final.
    fn columns(&self) -> Vec<(Column, f32, f32)> {
        self.layout_columns().into_iter().map(|(_, c, l, r)| (c, l, r)).collect()
    }

    /// Igual que `columns`, con el índice de cada una en `self.columns`.
    fn layout_columns(&self) -> Vec<(usize, Column, f32, f32)> {
        let avail = self.area.right - SCROLLBAR_W - self.area.left;
        let room = (avail - NAME_MIN).max(0.0);
        // Si no entran, todas se angostan en proporción (hasta MIN_COL_W); recién si así tampoco
        // alcanza, se dejan de mostrar las del final.
        let widths = |n: usize| -> Vec<f32> {
            let fixed: f32 = self.columns.iter().take(n).filter(|c| c.col != Column::Name).map(|c| c.width).sum();
            let k = if fixed > room && fixed > 0.0 { room / fixed } else { 1.0 };
            self.columns.iter().take(n).map(|c| if c.col == Column::Name { 0.0 } else { (c.width * k).max(MIN_COL_W) }).collect()
        };
        let mut shown = self.columns.len();
        while shown > 1 && widths(shown).iter().sum::<f32>() > room && self.columns[shown - 1].col != Column::Name {
            shown -= 1;
        }
        let w = widths(shown);
        let name_w = (avail - w.iter().sum::<f32>()).max(NAME_MIN.min(avail));
        let mut x = self.area.left;
        let mut out = Vec::with_capacity(shown);
        for (i, c) in self.columns.iter().take(shown).enumerate() {
            let cw = if c.col == Column::Name { name_w } else { w[i] };
            out.push((i, c.col, x, x + cw));
            x += cw;
        }
        out
    }

    /// ¿El punto está en el encabezado de la lista?
    pub fn in_header(&self, x: f32, y: f32) -> bool {
        !self.view.is_grid() && y >= self.area.top && y < self.area.top + HEADER_H && x >= self.area.left && x < self.area.right
    }

    /// Borde de encabezado bajo el mouse, para cambiar un ancho: (índice, izquierdo).
    /// El borde entre el nombre y la columna de su derecha cambia esa columna.
    fn edge_at(&self, x: f32, y: f32) -> Option<(usize, bool)> {
        if !self.in_header(x, y) {
            return None;
        }
        let cols = self.layout_columns();
        for (k, &(i, col, _, right)) in cols.iter().enumerate() {
            if (x - right).abs() > EDGE_GRAB {
                continue;
            }
            if col != Column::Name {
                return Some((i, false));
            }
            if let Some(&(j, _, _, _)) = cols.get(k + 1) {
                return Some((j, true));
            }
        }
        None
    }

    /// El mouse está sobre un borde de encabezado (cursor de cambiar ancho).
    pub fn wants_resize_cursor(&self, x: f32, y: f32) -> bool {
        matches!(self.header_drag, Some(HeaderDrag::Resize { .. })) || self.edge_at(x, y).is_some()
    }

    /// Muestra u oculta una columna (el nombre no se puede ocultar). Una columna nueva va al final.
    pub fn toggle_column(&mut self, col: Column) {
        if col == Column::Name {
            return;
        }
        match self.columns.iter().position(|c| c.col == col) {
            Some(i) => {
                self.columns.remove(i);
                if self.sort == col {
                    self.sort = Column::Name;
                    self.descending = false;
                    let focus = self.focused_index();
                    self.rebuild(focus);
                }
            }
            None => self.columns.push(ColumnSpec { col, width: col.default_width() }),
        }
    }

    /// Elemento (índice en `entries`) bajo el punto, si hay uno.
    pub fn entry_at(&self, x: f32, y: f32) -> Option<usize> {
        if !contains(&self.rows_area(), x, y) {
            return None;
        }
        self.pos_at(x, y).map(|pos| self.visible[pos] as usize)
    }

    fn row_at(&self, y: f32) -> Option<usize> {
        let rows = self.rows_area();
        if y < rows.top || y >= rows.bottom {
            return None;
        }
        let pos = ((y - rows.top + self.scroll) / ROW_H).floor() as usize;
        (pos < self.visible.len()).then_some(pos)
    }

    /// Barra de desplazamiento: (riel, pulgar), o None si todo entra.
    fn scrollbar(&self) -> Option<(Rect, Rect)> {
        let (view, content) = (self.view_h(), self.content_h());
        if content <= view || view <= 0.0 {
            return None;
        }
        let rows = self.rows_area();
        let track = rect(rows.right - SCROLLBAR_W, rows.top, rows.right, rows.bottom);
        let thumb_h = (view * view / content).max(24.0).min(view);
        let max_scroll = content - view;
        let top = track.top + (view - thumb_h) * (self.scroll / max_scroll);
        Some((track, rect(track.left + 3.0, top, track.right - 3.0, top + thumb_h)))
    }

    // ---------- selección ----------

    fn select_only(&mut self, pos: usize) {
        self.auto_focus = false;
        self.selected.iter_mut().for_each(|s| *s = false);
        if let Some(&idx) = self.visible.get(pos) {
            self.selected[idx as usize] = true;
        }
        self.focus = Some(pos);
        self.anchor = Some(pos);
    }

    fn select_range(&mut self, to: usize, keep_others: bool) {
        self.auto_focus = false;
        if !keep_others {
            self.selected.iter_mut().for_each(|s| *s = false);
        }
        let from = self.anchor.unwrap_or(0);
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        for &idx in &self.visible[a..=b.min(self.visible.len() - 1)] {
            self.selected[idx as usize] = true;
        }
        self.focus = Some(to);
    }

    fn toggle(&mut self, pos: usize) {
        self.auto_focus = false;
        if let Some(&idx) = self.visible.get(pos) {
            self.selected[idx as usize] = !self.selected[idx as usize];
        }
        self.focus = Some(pos);
        self.anchor = Some(pos);
    }

    pub fn select_all(&mut self) {
        for &idx in &self.visible {
            self.selected[idx as usize] = true;
        }
    }

    pub fn invert_selection(&mut self) {
        for &idx in &self.visible {
            self.selected[idx as usize] = !self.selected[idx as usize];
        }
    }

    // ---------- entrada ----------

    /// Teclas de navegación. Devuelve None si la tecla no es de la lista.
    pub fn on_key(&mut self, vk: u16, ctrl: bool, shift: bool) -> Option<Action> {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let n = self.visible.len();
        if n == 0 {
            return None;
        }
        let cur = self.focus.unwrap_or(0);
        let page = self.page_rows();
        // Sin selección, la primera flecha selecciona la fila con foco en vez de saltearla.
        let step = usize::from(self.selected.iter().any(|&s| s));
        // En la cuadrícula arriba/abajo saltan una fila de celdas y izquierda/derecha, una celda.
        let per = self.per_line();
        let grid = self.view.is_grid();
        let target = match VIRTUAL_KEY(vk) {
            VK_UP => cur.saturating_sub(step * per),
            VK_DOWN if cur + step * per < n => cur + step * per,
            // Abajo desde la última fila incompleta: el último elemento.
            VK_DOWN => n - 1,
            VK_LEFT if grid => cur.saturating_sub(step),
            VK_RIGHT if grid => (cur + step).min(n - 1),
            VK_PRIOR => cur.saturating_sub(page),
            VK_NEXT => (cur + page).min(n - 1),
            VK_HOME => 0,
            VK_END => n - 1,
            VK_SPACE if ctrl => {
                self.toggle(cur);
                return Some(Action::Redraw);
            }
            VK_RETURN => return self.focused_index().map(Action::Open),
            _ => return None,
        };
        if shift {
            self.select_range(target, ctrl);
        } else if ctrl {
            self.focus = Some(target);
            self.auto_focus = false;
        } else {
            self.select_only(target);
        }
        self.ensure_visible(target);
        Some(Action::Redraw)
    }

    /// Búsqueda al teclear: salta al primer nombre que empieza con lo escrito.
    pub fn on_char(&mut self, c: char) -> Action {
        let now = Instant::now();
        let fresh = self.typeahead_at.is_none_or(|t| now - t > Duration::from_millis(900));
        if fresh {
            self.typeahead.clear();
        }
        self.typeahead_at = Some(now);
        self.typeahead.extend(c.to_lowercase());

        // Con una sola letra repetida ("aaa") se recorren los que empiezan con esa letra.
        let repeated = self.typeahead.chars().count() > 1
            && self.typeahead.chars().all(|ch| Some(ch) == self.typeahead.chars().next());
        let prefix: String = if repeated { self.typeahead.chars().take(1).collect() } else { self.typeahead.clone() };
        let start = match (fresh || repeated, self.focus) {
            (true, Some(f)) => f + 1,
            (false, Some(f)) => f,
            _ => 0,
        };
        let n = self.visible.len();
        for k in 0..n {
            let pos = (start + k) % n;
            if self.entries[self.visible[pos] as usize].sort_key.starts_with(&prefix) {
                self.select_only(pos);
                self.ensure_visible(pos);
                return Action::Redraw;
            }
        }
        Action::None
    }

    /// Clic derecho: si cae en un elemento no seleccionado, lo selecciona solo a él.
    /// Devuelve si cayó sobre un elemento (menú de elementos) o no (menú del fondo).
    pub fn on_right_down(&mut self, x: f32, y: f32) -> bool {
        match self.entry_at(x, y) {
            Some(idx) => {
                if !self.selected[idx] {
                    let pos = self.visible.iter().position(|&v| v as usize == idx).unwrap();
                    self.select_only(pos);
                }
                true
            }
            None => {
                self.selected.iter_mut().for_each(|s| *s = false);
                false
            }
        }
    }

    /// Selecciona solo un elemento por índice y lo deja a la vista.
    pub fn select_index(&mut self, idx: usize) {
        if let Some(pos) = self.visible.iter().position(|&v| v as usize == idx) {
            self.select_only(pos);
            self.ensure_visible(pos);
        }
    }

    /// Rectángulo del nombre del elemento (para el renombrado en línea); lo trae a la vista.
    pub fn name_rect(&mut self, idx: usize) -> Option<Rect> {
        let pos = self.visible.iter().position(|&v| v as usize == idx)?;
        self.ensure_visible(pos);
        if self.view.is_grid() {
            // Debajo de la miniatura, una línea.
            let cell = self.item_rect(pos);
            let y = cell.top + 6.0 + self.view.icon() + 4.0;
            return Some(rect(cell.left + 4.0, y, cell.right - 4.0, y + 22.0));
        }
        let (_, left, right) = self.columns().into_iter().find(|c| c.0 == Column::Name)?;
        let row = self.item_rect(pos);
        Some(rect(left + PAD + ICON + 4.0, row.top + 1.0, right - PAD, row.bottom - 1.0))
    }

    /// Posición en pantalla (DIPs) del elemento con foco, para el menú contextual por teclado.
    pub fn focus_point(&self) -> Option<(f32, f32)> {
        let r = self.item_rect(self.focus?);
        let x = if self.view.is_grid() { (r.left + r.right) / 2.0 } else { self.area.left + 40.0 };
        Some((x, r.bottom.clamp(self.area.top, self.area.bottom)))
    }

    pub fn on_wheel(&mut self, rows: f32) -> Action {
        let before = self.scroll_target;
        // En la cuadrícula, una muesca de la rueda mueve media celda por "línea" de la rueda.
        let step = if self.view.is_grid() { self.line_h() / 3.0 } else { ROW_H };
        self.scroll_target -= rows * step;
        self.clamp_scroll();
        if !crate::anim::enabled() {
            self.snap_scroll();
        }
        self.update_hover_after_scroll();
        if self.scroll_target != before { Action::Redraw } else { Action::None }
    }

    /// Botón izquierdo. Devuelve si hay que capturar el mouse (arrastre de la barra).
    pub fn on_mouse_down(&mut self, x: f32, y: f32, ctrl: bool, shift: bool) -> (Action, bool) {
        if self.in_header(x, y) {
            // Borde: cambiar el ancho. Encabezado: ordenar al soltar, o reordenar si se arrastra.
            if let Some((index, invert)) = self.edge_at(x, y) {
                self.header_drag = Some(HeaderDrag::Resize { index, start_x: x, start_w: self.columns[index].width, invert });
                return (Action::None, true);
            }
            if let Some(&(index, ..)) = self.layout_columns().iter().find(|(_, _, l, r)| x >= *l && x < *r) {
                self.header_drag = Some(HeaderDrag::Press { index, start_x: x, moved: false });
                return (Action::None, true);
            }
            return (Action::None, false);
        }
        if let Some((track, thumb)) = self.scrollbar() {
            if contains(&track, x, y) {
                if y >= thumb.top && y < thumb.bottom {
                    self.thumb_drag = Some(y - thumb.top);
                    return (Action::Redraw, true);
                }
                // Clic en el riel: avanza una página hacia ese lado.
                let page = (self.view_h() / self.line_h()).floor().max(1.0) * self.line_h();
                self.scroll_target += if y < thumb.top { -page } else { page };
                self.clamp_scroll();
                if !crate::anim::enabled() {
                    self.snap_scroll();
                }
                return (Action::Redraw, false);
            }
        }
        match self.pos_at(x, y) {
            Some(pos) if ctrl => self.toggle(pos),
            Some(pos) if shift => self.select_range(pos, false),
            Some(pos) => {
                let idx = self.visible[pos] as usize;
                if self.selected[idx] {
                    self.focus = Some(pos);
                    self.auto_focus = false;
                    self.pending_select = Some(pos);
                } else {
                    self.select_only(pos);
                }
                self.press = Some((x, y));
            }
            None if contains(&self.rows_area(), x, y) && !ctrl => {
                self.selected.iter_mut().for_each(|s| *s = false);
            }
            None => return (Action::None, false),
        }
        (Action::Redraw, true)
    }

    pub fn on_mouse_up(&mut self) -> Action {
        self.thumb_drag = None;
        self.press = None;
        match self.header_drag.take() {
            Some(HeaderDrag::Press { index, moved: false, .. }) => {
                if let Some(c) = self.columns.get(index) {
                    self.set_sort(c.col);
                }
                return Action::Redraw;
            }
            Some(_) => return Action::ColumnsChanged,
            None => {}
        }
        if let Some(pos) = self.pending_select.take() {
            self.select_only(pos);
            return Action::Redraw;
        }
        Action::None
    }

    pub fn on_double_click(&mut self, x: f32, y: f32) -> Action {
        if self.scrollbar().is_some_and(|(track, _)| contains(&track, x, y)) {
            return Action::None;
        }
        match self.pos_at(x, y) {
            Some(pos) => {
                self.select_only(pos);
                Action::Open(self.visible[pos] as usize)
            }
            None => Action::None,
        }
    }

    pub fn on_mouse_move(&mut self, x: f32, y: f32) -> Action {
        match self.header_drag {
            Some(HeaderDrag::Resize { index, start_x, start_w, invert }) => {
                let delta = if invert { start_x - x } else { x - start_x };
                // Sin quitarle al nombre su mínimo.
                let others: f32 = self.columns.iter().enumerate().filter(|(i, c)| *i != index && c.col != Column::Name).map(|(_, c)| c.width).sum();
                let max = (self.area.right - SCROLLBAR_W - self.area.left - NAME_MIN - others).max(MIN_COL_W);
                self.columns[index].width = (start_w + delta).clamp(MIN_COL_W, max);
                return Action::Redraw;
            }
            Some(HeaderDrag::Press { index, start_x, moved }) => {
                if !moved && (x - start_x).abs() < 6.0 {
                    return Action::None;
                }
                // Reordenar en vivo: la columna va a la posición de la que está bajo el mouse.
                let target = self.layout_columns().iter().find(|(_, _, l, r)| x >= *l && x < *r).map(|(i, ..)| *i);
                let mut index = index;
                if let Some(to) = target.filter(|&t| t != index) {
                    let c = self.columns.remove(index);
                    self.columns.insert(to, c);
                    index = to;
                }
                self.header_drag = Some(HeaderDrag::Press { index, start_x, moved: true });
                return Action::Redraw;
            }
            None => {}
        }
        // Unos píxeles de movimiento con el botón apretado sobre un elemento: arrastre.
        if let Some((px, py)) = self.press {
            if (x - px).abs() > 4.0 || (y - py).abs() > 4.0 {
                self.press = None;
                self.pending_select = None;
                return Action::BeginDrag;
            }
            return Action::None;
        }
        if let Some(grab) = self.thumb_drag {
            if let Some((track, thumb)) = self.scrollbar() {
                let thumb_h = thumb.bottom - thumb.top;
                let travel = (track.bottom - track.top - thumb_h).max(1.0);
                let frac = ((y - grab - track.top) / travel).clamp(0.0, 1.0);
                // Arrastrar el pulgar sigue al mouse sin demora.
                self.scroll = frac * (self.content_h() - self.view_h());
                self.scroll_target = self.scroll;
                return Action::Redraw;
            }
        }
        let hover = if contains(&self.rows_area(), x, y) { self.pos_at(x, y) } else { None };
        if hover != self.hover {
            self.hover = hover;
            return Action::Redraw;
        }
        Action::None
    }

    pub fn on_mouse_leave(&mut self) -> Action {
        if self.hover.take().is_some() { Action::Redraw } else { Action::None }
    }

    fn update_hover_after_scroll(&mut self) {
        self.hover = None;
    }

    // ---------- dibujo ----------

    pub fn draw(&mut self, p: &Painter, area: Rect, active: bool) {
        self.area = area;
        self.clamp_scroll();
        let th = p.theme;
        let cols = self.columns();

        p.fill(area, th.surface);

        // Encabezado (solo en detalles).
        let header = rect(area.left, area.top, area.right, area.top + HEADER_H);
        // La columna que se está arrastrando para reordenar, resaltada.
        let dragging = match self.header_drag {
            Some(HeaderDrag::Press { index, moved: true, .. }) => self.columns.get(index).map(|c| c.col),
            _ => None,
        };
        if !self.view.is_grid() {
            p.fill(header, th.surface_alt);
            p.fill(rect(header.left, header.bottom - 1.0, header.right, header.bottom), th.border);
        }
        for &(col, left, right) in cols.iter().filter(|_| !self.view.is_grid()) {
            if dragging == Some(col) {
                p.fill(rect(left, header.top, right, header.bottom - 1.0), alpha(th.accent, 0.18));
            }
            let format = if col.right_aligned() { &p.fonts.ui_right } else { &p.fonts.ui_bold };
            let text_r = rect(left + PAD, header.top, right - PAD, header.bottom);
            p.text(format, col.title(), text_r, th.text_dim);
            if col == self.sort {
                // Chevron arriba/abajo de Segoe Fluent Icons.
                let glyph = if self.descending { "\u{E70D}" } else { "\u{E70E}" };
                let gx = if col.right_aligned() { left + 2.0 } else { right - 22.0 };
                p.text(&p.fonts.glyphs, glyph, rect(gx, header.top, gx + 20.0, header.bottom), th.text_dim);
            }
            if left > area.left {
                p.fill(rect(left, header.top + 7.0, left + 1.0, header.bottom - 7.0), th.border);
            }
        }

        let rows = self.rows_area();
        if self.visible.is_empty() {
            return;
        }

        let (first, last) = self.visible_range();
        let grid = self.view.is_grid();

        p.clip(rows, || {
            for pos in first..last {
                let idx = self.visible[pos] as usize;
                let e = &self.entries[idx];
                let item = self.item_rect(pos);
                let y = item.top;
                let row = if grid {
                    rect(item.left + 3.0, item.top + 3.0, item.right - 3.0, item.bottom - 3.0)
                } else {
                    rect(item.left + 4.0, y + 1.0, item.right, y + ROW_H - 1.0)
                };

                if self.drop_target == Some(idx) {
                    p.round_fill(row, 4.0, th.selection);
                    p.round_stroke(row, 4.0, th.accent);
                } else if self.selected[idx] {
                    let c = if active { th.selection } else { alpha(th.text_dim, 0.18) };
                    p.round_fill(row, 4.0, c);
                } else if active && self.focus == Some(pos) {
                    // Foco sin selección: fondo tenue del acento, además del borde.
                    p.round_fill(row, 4.0, th.focus_fill);
                } else if self.hover == Some(pos) {
                    p.round_fill(row, 4.0, th.hover);
                }
                if active && self.focus == Some(pos) {
                    p.round_stroke(row, 4.0, alpha(th.accent, 0.8));
                }

                // Color por tipo de `config.toml`; si ninguna regla coincide, los ocultos van tenues.
                let color = crate::config::color_for(p.file_colors, &e.sort_key, e.is_dir, e.attrs)
                    .unwrap_or(if e.is_hidden() { th.text_dim } else { th.text });
                if grid {
                    self.draw_cell(p, e, row, color);
                    continue;
                }
                for &(col, left, right) in &cols {
                    let cell = rect(left + PAD, y, right - PAD, y + ROW_H);
                    match col {
                        Column::Name => {
                            let iy = y + (ROW_H - ICON) / 2.0;
                            p.shell_icon(e.icon, e.overlay, cell.left, iy, ICON);
                            let text = rect(cell.left + ICON + 8.0, y, cell.right, y + ROW_H);
                            p.text(&p.fonts.ui, &e.display, text, color);
                        }
                        Column::Modified => p.text(&p.fonts.ui, &format::date(e.modified), cell, th.text_secondary),
                        Column::Created => p.text(&p.fonts.ui, &format::date(e.created), cell, th.text_secondary),
                        Column::Type => p.text(&p.fonts.ui, &e.type_name, cell, th.text_secondary),
                        Column::Extension => p.text(&p.fonts.ui, extension_of(e), cell, th.text_secondary),
                        Column::Attributes => p.text(&p.fonts.ui, &attr_letters(e.attrs), cell, th.text_secondary),
                        Column::Size => {
                            let s = match (e.is_dir, e.modified) {
                                // Unidades (sin fecha): capacidad total.
                                (true, 0) if e.size > 0 => format::size_short(e.size),
                                (true, _) => String::new(),
                                (false, _) => format::size_kb(e.size),
                            };
                            p.text(&p.fonts.ui_right, &s, cell, th.text_secondary);
                        }
                    }
                }
            }
        });

        if let Some((_, thumb)) = self.scrollbar() {
            let c = if self.thumb_drag.is_some() { alpha(th.scrollbar, th.scrollbar.a * 1.6) } else { th.scrollbar };
            p.round_fill(thumb, 3.0, c);
        }
    }

    /// Celda de la cuadrícula: miniatura (o ícono grande) arriba y el nombre en hasta dos líneas.
    fn draw_cell(&self, p: &Painter, e: &Entry, cell: Rect, color: D2D1_COLOR_F) {
        let size = self.view.icon();
        let cx = (cell.left + cell.right) / 2.0;
        let image = rect(cx - size / 2.0, cell.top + 6.0, cx + size / 2.0, cell.top + 6.0 + size);
        let source = match (&e.pidl, &e.full, &self.folder) {
            (Some(pidl), _, _) => Some(thumbs::Source::Pidl(pidl.clone())),
            (_, Some(full), _) => Some(thumbs::Source::Path(full.into())),
            (_, _, Some(folder)) => Some(thumbs::Source::Path(folder.join(&e.name))),
            _ => None,
        };
        let drawn = source.is_some_and(|s| p.thumbnail(&(s, p.thumb_px(size)), image));
        if !drawn {
            // Mientras llega la miniatura: el ícono chico, centrado.
            let small = 32.0_f32.min(size);
            let (ix, iy) = (cx - small / 2.0, (image.top + image.bottom) / 2.0 - small / 2.0);
            p.shell_icon(e.icon, e.overlay, ix, iy, small);
        }
        let label = rect(cell.left + 4.0, image.bottom + 4.0, cell.right - 4.0, cell.bottom - 2.0);
        p.text(&p.fonts.label, &e.display, label, color);
    }
}

#[cfg(test)]
mod column_tests {
    use super::*;

    fn list(width: f32) -> FileList {
        let mut l = FileList::default();
        l.area = rect(0.0, 0.0, width, 400.0);
        l
    }

    #[test]
    fn name_takes_the_rest_and_columns_drop_when_narrow() {
        let l = list(800.0);
        let cols = l.columns();
        assert_eq!(cols.iter().map(|c| c.0).collect::<Vec<_>>(), [Column::Name, Column::Modified, Column::Type, Column::Size]);
        let name_w = cols[0].2 - cols[0].1;
        assert_eq!(name_w, 800.0 - SCROLLBAR_W - 160.0 - 160.0 - 96.0);
        // Angosta: primero se angostan todas; las del final dejan de verse si ni así entran.
        let narrow = list(NAME_MIN + SCROLLBAR_W + 170.0);
        assert_eq!(narrow.columns().iter().map(|c| c.0).collect::<Vec<_>>(), [Column::Name, Column::Modified, Column::Type]);
    }

    #[test]
    fn header_drag_resizes_and_reorders() {
        let mut l = list(800.0);
        let cols = l.columns();
        // Borde entre Nombre y Modificado: agrandar Modificado moviendo a la izquierda.
        let edge = cols[0].2;
        assert_eq!(l.edge_at(edge, 10.0), Some((1, true)));
        l.on_mouse_down(edge, 10.0, false, false);
        l.on_mouse_move(edge - 40.0, 10.0);
        assert_eq!(l.on_mouse_up(), Action::ColumnsChanged);
        assert_eq!(l.columns[1].width, 200.0);
        // Arrastrar "Tamaño" sobre "Tipo": cambia el orden.
        let cols = l.columns();
        let (size_mid, type_mid) = ((cols[3].1 + cols[3].2) / 2.0, (cols[2].1 + cols[2].2) / 2.0);
        l.on_mouse_down(size_mid, 10.0, false, false);
        l.on_mouse_move(type_mid, 10.0);
        assert_eq!(l.on_mouse_up(), Action::ColumnsChanged);
        assert_eq!(l.columns.iter().map(|c| c.col).collect::<Vec<_>>(), [Column::Name, Column::Modified, Column::Size, Column::Type]);
        // Clic sin mover: ordena.
        let cols = l.columns();
        l.on_mouse_down((cols[2].1 + cols[2].2) / 2.0, 10.0, false, false);
        assert_eq!(l.on_mouse_up(), Action::Redraw);
        assert_eq!(l.sort, Column::Size);
    }

    fn entry(name: &str) -> Entry {
        Entry {
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
            type_name: std::sync::Arc::from("Archivo"),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    #[test]
    fn grid_geometry_and_keys() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_LEFT, VK_RIGHT, VK_UP};
        let mut l = list(800.0);
        l.append((0..25).map(|i| entry(&format!("f{i:02}"))).collect());
        l.set_view(View::LargeIcons);
        let per = l.per_line();
        assert!(per > 1, "en 800 DIPs entra más de una celda por fila");
        assert!(!l.in_header(10.0, 10.0), "la cuadrícula no tiene encabezado");
        // El centro de cada celda a la vista devuelve esa celda.
        let (first, last) = l.visible_range();
        for pos in first..last {
            let r = l.item_rect(pos);
            if r.bottom <= l.rows_area().bottom {
                assert_eq!(l.pos_at((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0), Some(pos));
            }
        }
        // Flechas: derecha +1, abajo +una fila, izquierda/arriba vuelven.
        l.select_index(l.visible[0] as usize);
        l.on_key(VK_RIGHT.0, false, false);
        assert_eq!(l.focus, Some(1));
        l.on_key(VK_DOWN.0, false, false);
        assert_eq!(l.focus, Some(1 + per));
        l.on_key(VK_UP.0, false, false);
        l.on_key(VK_LEFT.0, false, false);
        assert_eq!(l.focus, Some(0));
        // Abajo desde la última fila incompleta va al último elemento.
        l.select_index(l.visible[24] as usize);
        l.on_key(VK_DOWN.0, false, false);
        assert_eq!(l.focus, Some(24));
        // En detalles, izquierda/derecha no son de la lista.
        l.set_view(View::Details);
        assert_eq!(l.on_key(VK_RIGHT.0, false, false), None);
    }

    /// Apretar sobre un archivo y mover unos píxeles pide empezar a arrastrar (y la app tiene que
    /// atenderlo: su `on_mouse_move` lo descartaba y el arrastre nunca arrancaba).
    #[test]
    fn press_and_move_begins_a_drag() {
        let mut l = list(800.0);
        l.append(vec![entry("a.txt"), entry("b.txt")]);
        let row = l.item_rect(0);
        let (x, y) = (row.left + 60.0, (row.top + row.bottom) / 2.0);
        let (action, capture) = l.on_mouse_down(x, y, false, false);
        assert_eq!((action, capture), (Action::Redraw, true));
        // Un temblor de 2 px no es un arrastre.
        assert_eq!(l.on_mouse_move(x + 2.0, y), Action::None);
        assert_eq!(l.on_mouse_move(x + 12.0, y + 3.0), Action::BeginDrag);
        // Una vez iniciado, no se repite (un Redraw por el resaltado bajo el mouse no es un arrastre).
        assert_ne!(l.on_mouse_move(x + 40.0, y), Action::BeginDrag);
        // Apretar en un lugar vacío (debajo de los elementos) no arrastra nada.
        let mut empty = list(800.0);
        empty.append(vec![entry("a.txt")]);
        let below = empty.rows_area().bottom - 10.0;
        empty.on_mouse_down(x, below, false, false);
        assert_eq!(empty.on_mouse_move(x + 30.0, below), Action::None);
    }

    #[test]
    fn smooth_scroll_targets_first_draws_later() {
        let mut l = list(800.0);
        l.append((0..200).map(|i| entry(&format!("f{i:03}"))).collect());
        assert_eq!(l.on_wheel(-3.0), Action::Redraw);
        // La lógica ya está en el destino (3 filas), lo dibujado todavía no llegó.
        assert_eq!(l.scroll_target, 3.0 * ROW_H);
        assert!(l.scroll < l.scroll_target);
        let mut frames = 0;
        while l.tick(1.0 / 60.0) {
            assert!(l.scroll <= l.scroll_target, "sin pasarse");
            frames += 1;
            assert!(frames < 60);
        }
        assert_eq!(l.scroll, l.scroll_target);
        // Llevar un elemento lejano a la vista también es un destino, no un salto.
        l.select_index(l.visible[150] as usize);
        assert!(l.scroll_target > l.scroll);
        // Cambiar de vista o de filtro sí es instantáneo.
        l.set_view(View::Icons);
        assert_eq!(l.scroll, l.scroll_target);
    }

    #[test]
    fn toggle_columns() {
        let mut l = list(800.0);
        l.toggle_column(Column::Attributes);
        assert_eq!(l.columns.last().map(|c| c.col), Some(Column::Attributes));
        l.set_sort(Column::Type);
        l.toggle_column(Column::Type);
        assert!(!l.columns.iter().any(|c| c.col == Column::Type));
        assert_eq!(l.sort, Column::Name, "ocultar la columna de orden vuelve a ordenar por nombre");
        l.toggle_column(Column::Name);
        assert!(l.columns.iter().any(|c| c.col == Column::Name));
        assert_eq!(attr_letters(0x1 | 0x2 | 0x20), "RHA");
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_matches, wildcard};

    #[test]
    fn wildcards() {
        assert!(wildcard("*.pdf", "informe.pdf"));
        assert!(!wildcard("*.pdf", "informe.pdf.bak"));
        assert!(wildcard("img?.png", "img2.png"));
        assert!(!wildcard("img?.png", "img10.png"));
        assert!(wildcard("*2026*", "foto 2026-10.jpg"));
        assert!(wildcard("*", ""));
        assert!(wildcard("a*b*c", "aXbYbZc"));
        assert!(!wildcard("a*b*c", "aXbYbZ"));
    }

    #[test]
    fn filter_parts() {
        let parts = |s: &str| s.split(';').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect::<Vec<_>>();
        assert!(filter_matches(&parts(""), "lo que sea"));
        assert!(filter_matches(&parts("Inform"), "informe final.docx"));
        assert!(filter_matches(&parts("*.jpg; *.png"), "foto.png"));
        assert!(!filter_matches(&parts("*.jpg;*.png"), "foto.gif"));
        assert!(filter_matches(&parts("año"), "resumen del año.txt"));
    }
}

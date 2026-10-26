//! Lista de archivos en vista de detalles, virtualizada: solo se dibujan las filas visibles,
//! así que el costo por frame no depende de cuántos archivos tenga la carpeta.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::format;
use crate::fs::Entry;
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;

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
    Type,
    Size,
}

/// Columnas a la derecha del nombre, de derecha a izquierda: (columna, título, ancho).
const RIGHT_COLUMNS: [(Column, &str, f32); 3] = [
    (Column::Size, "Tamaño", 96.0),
    (Column::Type, "Tipo", 160.0),
    (Column::Modified, "Fecha de modificación", 160.0),
];

/// Lo que la lista le pide al panel después de un evento.
#[derive(PartialEq, Debug)]
pub enum Action {
    None,
    Redraw,
    /// Abrir el elemento (índice en `entries`).
    Open(usize),
    /// Empezar a arrastrar la selección.
    BeginDrag,
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
    scroll: f32,
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
}

/// ¿El nombre (en minúsculas) pasa el filtro? Cada parte separada por `;` es un patrón: con `*`
/// o `?` se compara el nombre entero como comodín; si no, alcanza con que lo contenga.
fn filter_matches(parts: &[String], name: &str) -> bool {
    parts.is_empty()
        || parts.iter().any(|p| if p.contains(['*', '?']) { wildcard(p, name) } else { name.contains(p.as_str()) })
}

/// Comodines de Windows: `*` cualquier secuencia, `?` un carácter. Sin recursión: vuelve al
/// último `*` cuando algo no coincide.
fn wildcard(pattern: &str, text: &str) -> bool {
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
        }
    }
}

fn compare(a: &Entry, b: &Entry, sort: Column, desc: bool) -> Ordering {
    // Primero el grupo (secciones de "Este equipo"), después carpetas arriba, en cualquier orden.
    a.group.cmp(&b.group).then_with(|| b.is_dir.cmp(&a.is_dir)).then_with(|| {
        let ord = match sort {
            Column::Name => Ordering::Equal,
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Type => format::caseless_cmp(&a.type_name, &b.type_name),
            Column::Size => a.size.cmp(&b.size),
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
        if let Some(f) = self.focus {
            self.ensure_visible(f);
        }
    }

    /// Elementos que se verían sin el filtro (para mostrar "N de M").
    pub fn unfiltered_count(&self) -> usize {
        self.entries.iter().filter(|e| self.show_hidden || !e.is_hidden()).count()
    }

    /// Vacía la lista (navegación a otra carpeta). El filtro no pasa a la carpeta nueva.
    pub fn clear(&mut self) {
        *self = Self { sort: self.sort, descending: self.descending, show_hidden: self.show_hidden, ..Default::default() };
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
        let first = (self.scroll / ROW_H).floor() as usize;
        let last = (first + (self.view_h() / ROW_H).ceil() as usize + 1).min(self.visible.len());
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
        self.ensure_visible(pos);
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

    fn rows_area(&self) -> Rect {
        rect(self.area.left, self.area.top + HEADER_H, self.area.right, self.area.bottom)
    }

    fn view_h(&self) -> f32 {
        (self.area.bottom - self.area.top - HEADER_H).max(0.0)
    }

    fn content_h(&self) -> f32 {
        self.visible.len() as f32 * ROW_H
    }

    fn page_rows(&self) -> usize {
        ((self.view_h() / ROW_H).floor() as usize).max(1)
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_h() - self.view_h()).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }

    fn ensure_visible(&mut self, pos: usize) {
        let top = pos as f32 * ROW_H;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW_H > self.scroll + self.view_h() {
            self.scroll = top + ROW_H - self.view_h();
        }
        self.clamp_scroll();
    }

    /// Columnas que entran en el ancho disponible: (columna, izquierda, derecha).
    fn columns(&self) -> Vec<(Column, f32, f32)> {
        let right_edge = self.area.right - SCROLLBAR_W;
        let mut out = Vec::new();
        let mut right = right_edge;
        for (col, _, width) in RIGHT_COLUMNS {
            let left = right - width;
            if left < self.area.left + NAME_MIN {
                break;
            }
            out.push((col, left, right));
            right = left;
        }
        out.push((Column::Name, self.area.left, right));
        out.reverse();
        out
    }

    /// Elemento (índice en `entries`) bajo el punto, si hay uno.
    pub fn entry_at(&self, x: f32, y: f32) -> Option<usize> {
        if !contains(&self.rows_area(), x, y) {
            return None;
        }
        self.row_at(y).map(|pos| self.visible[pos] as usize)
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
        let target = match VIRTUAL_KEY(vk) {
            VK_UP => cur.saturating_sub(step),
            VK_DOWN => (cur + step).min(n - 1),
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
        let (_, left, right) = self.columns().into_iter().find(|c| c.0 == Column::Name)?;
        let y = self.rows_area().top + pos as f32 * ROW_H - self.scroll;
        Some(rect(left + PAD + ICON + 4.0, y + 1.0, right - PAD, y + ROW_H - 1.0))
    }

    /// Posición en pantalla (DIPs) de la fila con foco, para el menú contextual por teclado.
    pub fn focus_point(&self) -> Option<(f32, f32)> {
        let pos = self.focus?;
        let y = self.rows_area().top + pos as f32 * ROW_H - self.scroll + ROW_H;
        Some((self.area.left + 40.0, y.clamp(self.area.top, self.area.bottom)))
    }

    pub fn on_wheel(&mut self, rows: f32) -> Action {
        let before = self.scroll;
        self.scroll -= rows * ROW_H;
        self.clamp_scroll();
        self.update_hover_after_scroll();
        if self.scroll != before { Action::Redraw } else { Action::None }
    }

    /// Botón izquierdo. Devuelve si hay que capturar el mouse (arrastre de la barra).
    pub fn on_mouse_down(&mut self, x: f32, y: f32, ctrl: bool, shift: bool) -> (Action, bool) {
        if y >= self.area.top && y < self.area.top + HEADER_H {
            for (col, left, right) in self.columns() {
                if x >= left && x < right {
                    self.set_sort(col);
                    return (Action::Redraw, false);
                }
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
                let page = self.page_rows() as f32 * ROW_H;
                self.scroll += if y < thumb.top { -page } else { page };
                self.clamp_scroll();
                return (Action::Redraw, false);
            }
        }
        match self.row_at(y) {
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
        match self.row_at(y) {
            Some(pos) => {
                self.select_only(pos);
                Action::Open(self.visible[pos] as usize)
            }
            None => Action::None,
        }
    }

    pub fn on_mouse_move(&mut self, x: f32, y: f32) -> Action {
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
                self.scroll = frac * (self.content_h() - self.view_h());
                return Action::Redraw;
            }
        }
        let hover = if contains(&self.rows_area(), x, y) { self.row_at(y) } else { None };
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

        // Encabezado.
        let header = rect(area.left, area.top, area.right, area.top + HEADER_H);
        p.fill(header, th.surface_alt);
        p.fill(rect(header.left, header.bottom - 1.0, header.right, header.bottom), th.border);
        for &(col, left, right) in &cols {
            let title = match col {
                Column::Name => "Nombre",
                _ => RIGHT_COLUMNS.iter().find(|c| c.0 == col).unwrap().1,
            };
            let format = if col == Column::Size { &p.fonts.ui_right } else { &p.fonts.ui_bold };
            let text_r = rect(left + PAD, header.top, right - PAD, header.bottom);
            p.text(format, title, text_r, th.text_dim);
            if col == self.sort {
                // Chevron arriba/abajo de Segoe Fluent Icons.
                let glyph = if self.descending { "\u{E70D}" } else { "\u{E70E}" };
                let gx = if col == Column::Size { left + 2.0 } else { right - 22.0 };
                p.text(&p.fonts.glyphs, glyph, rect(gx, header.top, gx + 20.0, header.bottom), th.text_dim);
            }
            if col != Column::Name {
                p.fill(rect(left, header.top + 7.0, left + 1.0, header.bottom - 7.0), th.border);
            }
        }

        let rows = self.rows_area();
        if self.visible.is_empty() {
            return;
        }

        let first = (self.scroll / ROW_H).floor() as usize;
        let count = (self.view_h() / ROW_H).ceil() as usize + 1;
        let last = (first + count).min(self.visible.len());
        let text_right = rows.right - SCROLLBAR_W;

        p.clip(rows, || {
            for pos in first..last {
                let idx = self.visible[pos] as usize;
                let e = &self.entries[idx];
                let y = rows.top + pos as f32 * ROW_H - self.scroll;
                let row = rect(rows.left + 4.0, y + 1.0, text_right, y + ROW_H - 1.0);

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

                let color = if e.is_hidden() { th.text_dim } else { th.text };
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
                        Column::Type => p.text(&p.fonts.ui, &e.type_name, cell, th.text_secondary),
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

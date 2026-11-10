//! Grupos de la lista: a qué grupo va cada elemento según una columna («Hoy», «A–H»,
//! «Pequeño (16 KB–1 MB)»…). Agrupar y, después, los filtros por columna usan la misma función
//! (`group_key`), así los grupos y las opciones de filtro son siempre los mismos.

use super::{Column, FileList, SCROLLBAR_W, View, attr_letters, extension_of, parent_of};
use crate::fs::Entry;
use crate::geom::{Rect, rect};
use crate::paint::{FontId, Painter};
pub use crate::sys::LocalNow;

pub use crate::fs::Meta;

/// Grupo de un elemento: `order` ordena los grupos (después, el nombre); `label` es lo que se
/// muestra y lo que se guarda como grupo plegado.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GroupKey {
    pub order: i64,
    pub label: String,
}

/// Grupos con `order` menor que este van siempre primero (las carpetas), aunque se invierta el
/// orden de los grupos.
pub const PINNED_FIRST: i64 = 0;
/// Grupos con `order` desde este van siempre al final («Sin fecha»).
pub const PINNED_LAST: i64 = 1000;

/// Criterios de «Agrupar por», en el orden del menú y de `Command::GroupBy`; el último es
/// «(Ninguno)».
pub const GROUPINGS: [Option<Column>; 8] = [
    Some(Column::Name),
    Some(Column::Modified),
    Some(Column::Created),
    Some(Column::Type),
    Some(Column::Size),
    Some(Column::Dimensions),
    Some(Column::Duration),
    None,
];

fn key(order: i64, label: &str) -> GroupKey {
    GroupKey { order, label: label.to_string() }
}

/// Grupo de `e` según la columna `col`. `meta` son los metadatos leídos del elemento (`None`:
/// todavía no llegaron, «Leyendo…») y `now` la fecha de hoy en hora local. Pura.
pub fn group_key(col: Column, e: &Entry, meta: Option<&Meta>, now: LocalNow) -> GroupKey {
    match col {
        Column::Name => name_group(&e.sort_key),
        Column::Modified => date_group(e.modified, now),
        Column::Created => date_group(e.created, now),
        Column::Type => type_group(e),
        Column::Size => size_group(e),
        Column::Extension => value_group(&extension_of(e).to_uppercase()),
        Column::Attributes => value_group(&attr_letters(e.attrs)),
        Column::Folder => value_group(&parent_of(e)),
        Column::Dimensions | Column::Duration | Column::Authors | Column::Tags | Column::Artist | Column::Album => super::meta::meta_group(col, meta),
    }
}

/// Un grupo por valor; sin valor, «Sin valor» al final.
fn value_group(v: &str) -> GroupKey {
    if v.is_empty() { key(PINNED_LAST, "Sin valor") } else { key(1, v) }
}

/// Letra sin acento (para agrupar por nombre): «á» → «a», «ñ» → «n».
fn fold_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        'ý' | 'ÿ' => 'y',
        _ => c,
    }
}

/// Nombre: por la primera letra sin acento ni mayúsculas.
fn name_group(sort_key: &str) -> GroupKey {
    let c = sort_key.chars().next().map(|c| fold_accent(c.to_lowercase().next().unwrap_or(c)));
    match c {
        Some('0'..='9') => key(0, "0–9"),
        Some('a'..='h') => key(1, "A–H"),
        Some('i'..='p') => key(2, "I–P"),
        Some('q'..='z') => key(3, "Q–Z"),
        _ => key(4, "Otros"),
    }
}

/// Segundos entre 1601-01-01 (época de FILETIME) y 1970-01-01.
const SECS_1601_TO_1970: i64 = 11_644_473_600;

/// Fecha (FILETIME en UTC): de la más reciente a la más vieja, semana de lunes a domingo, en hora
/// local. Las fechas futuras cuentan como «Hoy».
fn date_group(ft: u64, now: LocalNow) -> GroupKey {
    if ft == 0 {
        return key(PINNED_LAST, "Sin fecha");
    }
    let unix = (ft / 10_000_000) as i64 - SECS_1601_TO_1970;
    let day = (unix + now.offset).div_euclid(86_400);
    let week_start = now.day - i64::from(now.weekday);
    let (y, m, _) = crate::format::civil_from_days(day);
    let months = |y: i64, m: i64| y * 12 + m - 1;
    let this_month = months(i64::from(now.year), i64::from(now.month));
    let month = months(y, i64::from(m));
    if day >= now.day {
        key(0, "Hoy")
    } else if day == now.day - 1 {
        key(1, "Ayer")
    } else if day >= week_start {
        key(2, "Antes en esta semana")
    } else if day >= week_start - 7 {
        key(3, "La semana pasada")
    } else if month == this_month {
        key(4, "Antes en este mes")
    } else if month == this_month - 1 {
        key(5, "El mes pasado")
    } else if y == i64::from(now.year) {
        key(6, "Antes en este año")
    } else {
        key(7, "Hace mucho")
    }
}

/// Tipo: un grupo por tipo; las carpetas primero. Sin tipo (carpeta o archivo), un solo grupo
/// «Sin valor» al final, como en las otras columnas.
fn type_group(e: &Entry) -> GroupKey {
    if e.type_name.is_empty() {
        return key(PINNED_LAST, "Sin valor");
    }
    key(if e.is_dir { PINNED_FIRST - 1 } else { 1 }, &e.type_name)
}

/// Tamaño: de chico a grande; las carpetas aparte, primero.
fn size_group(e: &Entry) -> GroupKey {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if e.is_dir {
        return key(PINNED_FIRST - 1, "Carpetas");
    }
    match e.size {
        0 => key(0, "Vacío"),
        s if s <= 16 * KB => key(1, "Diminuto (0–16 KB)"),
        s if s <= MB => key(2, "Pequeño (16 KB–1 MB)"),
        s if s <= 128 * MB => key(3, "Mediano (1–128 MB)"),
        s if s <= GB => key(4, "Grande (128 MB–1 GB)"),
        s if s <= 4 * GB => key(5, "Enorme (1–4 GB)"),
        _ => key(6, "Gigantesco (> 4 GB)"),
    }
}

/// Orden natural de los grupos de la columna: las fechas de la más reciente a la más vieja (que
/// es el orden descendente de su columna); el resto, ascendente.
pub fn natural_descending(col: Column) -> bool {
    matches!(col, Column::Modified | Column::Created)
}

/// Fila de la lista agrupada: el encabezado de un grupo (con cuántos elementos tiene y si está
/// plegado) o un elemento (índice en `entries`).
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Header { key: GroupKey, count: usize, collapsed: bool },
    Item(usize),
}

/// Alto del encabezado de un grupo y espacio libre arriba de cada uno salvo el primero (diseño
/// `list::FileList::draw_group_header`, pantalla X32).
pub const GROUP_H: f32 = 28.0;
pub const GROUP_GAP: f32 = 6.0;

/// Una línea de la lista agrupada: un encabezado o una fila de elementos (uno en Detalles, varios
/// en las cuadrículas; cada grupo empieza una línea nueva).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Line {
    /// Arriba, en coordenadas del contenido (con el margen de arriba incluido).
    pub top: f32,
    pub h: f32,
    /// Encabezado: índice en `rows`.
    pub header: Option<u32>,
    /// Elementos: primera posición (en `visible`) y cuántos.
    pub first: u32,
    pub count: u32,
}

/// Geometría de la lista agrupada para un ancho (elementos por línea) y una vista dados.
pub(super) struct Layout {
    pub per: usize,
    pub view: View,
    pub lines: Vec<Line>,
    /// Línea de cada posición de `visible`.
    pub pos_line: Vec<u32>,
    /// Donde termina la última línea.
    pub bottom: f32,
}

impl Layout {
    /// Línea bajo la altura `cy` del contenido.
    pub fn line_at(&self, cy: f32) -> Option<usize> {
        let i = self.lines.partition_point(|l| l.top <= cy);
        let l = self.lines.get(i.checked_sub(1)?)?;
        (cy < l.top + l.h).then_some(i - 1)
    }

    /// Líneas que tocan el tramo [top, bottom) del contenido.
    pub fn lines_between(&self, top: f32, bottom: f32) -> std::ops::Range<usize> {
        let start = self.lines.partition_point(|l| l.top + l.h <= top);
        let end = self.lines.partition_point(|l| l.top < bottom).max(start);
        start..end
    }
}

impl FileList {
    /// Agrupa por la columna (o deja de agrupar con `None`). Cambiar de criterio despliega todo.
    pub fn set_group(&mut self, col: Option<Column>) {
        if self.group_by == col {
            return;
        }
        // En la Lista, agrupar pasa de desplazarse a los costados a hacia abajo (y al revés).
        if self.view == View::List {
            self.scroll = 0.0;
            self.scroll_target = 0.0;
        }
        self.group_by = col;
        self.collapsed.clear();
        self.header_focus = None;
        self.regroup_keep_focus();
    }

    /// Pliega o despliega el grupo. Si el foco estaba en un elemento del grupo que se pliega,
    /// pasa a su encabezado.
    pub fn toggle_group(&mut self, label: &str) {
        let focus_entry = self.focused_index();
        if let Some(i) = self.collapsed.iter().position(|c| c == label) {
            self.collapsed.remove(i);
        } else {
            self.collapsed.push(label.to_string());
            let in_group = focus_entry.is_some_and(|i| self.label_of(i) == Some(label));
            if in_group && self.header_focus.is_none() && !self.auto_focus {
                self.header_focus = Some(label.to_string());
            }
        }
        self.regroup();
        self.restore_focus(focus_entry);
        self.clamp_scroll();
    }

    /// Filas de la lista agrupada: encabezados y elementos de los grupos desplegados, en orden.
    /// Vacía sin agrupar. Se rearma solo cuando cambian los elementos, el orden, el criterio, los
    /// grupos plegados o los filtros, nunca al dibujar.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Cuántos elementos se muestran, contando los de los grupos plegados (barra de estado).
    pub fn total(&self) -> usize {
        self.order.len()
    }

    /// Nombre del grupo de un elemento (índice en `entries`), con grupos.
    pub(super) fn label_of(&self, idx: usize) -> Option<&str> {
        self.group_by?;
        let k = *self.entry_keys.get(idx)?;
        Some(self.key_table[k as usize].label.as_str())
    }

    /// Encabezado con foco: índice en `rows`.
    pub(super) fn focused_header(&self) -> Option<usize> {
        let label = self.header_focus.as_deref()?;
        self.header_rows.iter().map(|&r| r as usize).find(|&r| matches!(&self.rows[r], Row::Header { key, .. } if key.label == label))
    }

    /// Pone el foco en el encabezado (índice en `rows`).
    pub(super) fn focus_header(&mut self, row: usize) {
        if let Some(Row::Header { key, .. }) = self.rows.get(row) {
            self.header_focus = Some(key.label.clone());
            self.auto_focus = false;
            self.focus_visible = true;
        }
    }

    /// Pliega o despliega el grupo del encabezado (índice en `rows`).
    pub(super) fn toggle_group_row(&mut self, row: usize) {
        if let Some(Row::Header { key, .. }) = self.rows.get(row) {
            let label = key.label.clone();
            self.toggle_group(&label);
        }
    }

    /// Rearma las filas conservando el foco.
    pub(super) fn regroup_keep_focus(&mut self) {
        let focus = self.focused_index();
        self.regroup();
        self.restore_focus(focus);
        self.clamp_scroll();
        if let Some(f) = self.focus.filter(|_| self.header_focus.is_none()) {
            self.ensure_visible(f);
        }
    }

    /// Grupo de cada elemento que todavía no lo tiene (todos, si cambió el criterio o se
    /// reemplazaron los elementos).
    fn ensure_keys(&mut self) {
        let Some(col) = self.group_by else { return };
        // La fecha de hoy solo hace falta para las fechas.
        let now = if natural_descending(col) { crate::sys::env().local_now() } else { LocalNow::from_unix(0, 0) };
        let new_day = natural_descending(col) && now.day != self.keys_day;
        if self.keys_for != Some(col) || self.entry_keys.len() > self.entries.len() || new_day {
            self.entry_keys.clear();
            self.key_table.clear();
            self.key_index.clear();
            self.keys_for = Some(col);
            self.keys_day = now.day;
        }
        if self.entry_keys.len() == self.entries.len() {
            return;
        }
        for i in self.entry_keys.len()..self.entries.len() {
            let k = group_key(col, &self.entries[i], self.meta_of(i), now);
            let id = self.intern_key(k);
            self.entry_keys.push(id);
        }
    }

    /// Agrupada por fecha: si cambió el día desde que se calcularon los grupos (pasó la
    /// medianoche), los rearma. Se llama al dibujar.
    pub(super) fn regroup_if_day_changed(&mut self) {
        let Some(col) = self.group_by.filter(|&c| natural_descending(c)) else { return };
        if self.keys_for == Some(col) && !self.entry_keys.is_empty() && crate::sys::env().local_now().day != self.keys_day {
            self.regroup_keep_focus();
        }
    }

    /// Índice del grupo en `key_table` (lo agrega si es nuevo).
    pub(super) fn intern_key(&mut self, k: GroupKey) -> u32 {
        let n = self.key_table.len() as u32;
        let i = *self.key_index.entry(k.clone()).or_insert(n);
        if i == n {
            self.key_table.push(k);
        }
        i
    }

    /// Arma lo que se ve a partir de `order` (elementos que pasan los filtros, ya ordenados):
    /// sin grupos, todos; con grupos, las filas con sus encabezados y los elementos de los grupos
    /// desplegados (lo de los plegados se deselecciona: no se opera sobre lo que no se ve).
    pub(super) fn regroup(&mut self) {
        self.rows_built += 1;
        self.rows.clear();
        self.header_rows.clear();
        let Some(col) = self.group_by else {
            self.visible = self.order.clone();
            self.layout = None;
            self.header_focus = None;
            return;
        };
        self.ensure_keys();
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); self.key_table.len()];
        for &i in &self.order {
            buckets[self.entry_keys[i as usize] as usize].push(i);
        }
        let table = &self.key_table;
        let mut groups: Vec<usize> = (0..table.len()).filter(|&g| !buckets[g].is_empty()).collect();
        groups.sort_by(|&a, &b| table[a].order.cmp(&table[b].order).then_with(|| crate::format::caseless_cmp(&table[a].label, &table[b].label)));
        // El sentido de la columna (si es la de orden) invierte el orden natural de los grupos;
        // las carpetas siguen primero y «Sin fecha» al final.
        if self.sort == col && self.descending != natural_descending(col) {
            let lo = groups.partition_point(|&g| table[g].order < PINNED_FIRST);
            let hi = groups.partition_point(|&g| table[g].order < PINNED_LAST);
            groups[lo..hi].reverse();
        }
        self.visible.clear();
        for g in groups {
            let key = table[g].clone();
            let collapsed = self.collapsed.contains(&key.label);
            let bucket = &buckets[g];
            self.header_rows.push(self.rows.len() as u32);
            self.rows.push(Row::Header { key, count: bucket.len(), collapsed });
            for &i in bucket {
                if collapsed {
                    self.selected[i as usize] = false;
                } else {
                    self.visible.push(i);
                    self.rows.push(Row::Item(i as usize));
                }
            }
        }
        if self.focused_header().is_none() {
            self.header_focus = None;
        }
        self.relayout();
    }

    /// Recalcula la geometría de la lista agrupada (al cambiar las filas, la vista o los elementos
    /// por línea).
    pub(super) fn relayout(&mut self) {
        if self.group_by.is_none() {
            self.layout = None;
            return;
        }
        let (per, lh, grid) = (self.per_line(), self.line_h(), self.flows());
        let mut lines: Vec<Line> = Vec::new();
        let mut pos_line = Vec::with_capacity(self.visible.len());
        let mut y = self.top_pad();
        let mut pos = 0u32;
        for (r, row) in self.rows.iter().enumerate() {
            match row {
                Row::Header { .. } => {
                    if !lines.is_empty() {
                        y += GROUP_GAP;
                    }
                    lines.push(Line { top: y, h: GROUP_H, header: Some(r as u32), first: pos, count: 0 });
                    y += GROUP_H;
                }
                Row::Item(_) => {
                    match lines.last_mut() {
                        Some(l) if l.header.is_none() && grid && (l.count as usize) < per => l.count += 1,
                        _ => {
                            lines.push(Line { top: y, h: lh, header: None, first: pos, count: 1 });
                            y += lh;
                        }
                    }
                    pos_line.push(lines.len() as u32 - 1);
                    pos += 1;
                }
            }
        }
        self.layout = Some(Layout { per, view: self.view, lines, pos_line, bottom: y });
    }

    /// Geometría al día con el ancho y la vista actuales (el ancho cambia al dibujar).
    pub(super) fn layout_current(&self) -> bool {
        self.layout.as_ref().is_none_or(|l| l.per == self.per_line() && l.view == self.view)
    }

    /// Rectángulo de una línea de encabezado (de lado a lado de la lista, sin la barra).
    pub(super) fn line_rect(&self, line: &Line) -> Rect {
        let rows = self.rows_area();
        let y = rows.top + line.top - self.scroll;
        rect(rows.left, y, rows.right - SCROLLBAR_W, y + line.h)
    }

    /// Encabezado bajo el punto (índice en `rows`).
    pub(super) fn header_at(&self, x: f32, y: f32) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let rows = self.rows_area();
        if y < rows.top || y >= rows.bottom || x < rows.left || x >= rows.right - SCROLLBAR_W {
            return None;
        }
        let li = layout.line_at(y - rows.top + self.scroll)?;
        layout.lines[li].header.map(|r| r as usize)
    }

    /// Encabezado de un grupo (diseño `list::FileList::draw_group_header`): flecha abajo
    /// (desplegado) o a la derecha (plegado), el nombre con la cantidad y una línea hasta el borde;
    /// con foco, la banda del acento.
    pub fn draw_group_header(&self, p: &dyn Painter, r: Rect, key: &GroupKey, count: usize, collapsed: bool, focused: bool) {
        let th = p.theme();
        if focused {
            let band = rect(r.left + 4.0, r.top + 1.0, r.right, r.bottom - 1.0);
            p.round_fill(band, 4.0, th.focus_fill);
            p.round_stroke(band, 4.0, th.accent);
        }
        let glyph = if collapsed { "\u{E76C}" } else { "\u{E70D}" };
        p.text(FontId::GlyphsSmall, glyph, rect(r.left + 8.0, r.top, r.left + 24.0, r.bottom), th.text_dim);
        let text = format!("{} ({count})", key.label);
        let tx = r.left + 30.0;
        let w = p.measure(FontId::UiBold, &text);
        p.text(FontId::UiBold, &text, rect(tx, r.top, (tx + w + 4.0).min(r.right - 10.0), r.bottom), th.text_secondary);
        let lx = tx + w + 10.0;
        if lx < r.right - 10.0 {
            let ly = r.top + 14.0;
            p.fill(rect(lx, ly, r.right - 10.0, ly + 1.0), th.border);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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
            type_name: Arc::from("Archivo"),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    /// FILETIME de las 12:00 UTC de una fecha.
    fn ft(y: i64, m: i64, d: i64) -> u64 {
        // Días desde 1970 (inverso de `civil_from_days`, algoritmo de Howard Hinnant).
        let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        ((days * 86_400 + 12 * 3600 + SECS_1601_TO_1970) * 10_000_000) as u64
    }

    fn labels(keys: &[GroupKey]) -> Vec<&str> {
        keys.iter().map(|k| k.label.as_str()).collect()
    }

    /// Un miércoles (11/11/2026): hoy, ayer, el lunes, la semana pasada, este mes, el mes pasado,
    /// este año, hace dos años y sin fecha, en ese orden.
    #[test]
    fn group_key_dates() {
        let now = LocalNow::from_unix(ft(2026, 11, 11) as i64 / 10_000_000 - SECS_1601_TO_1970, 0);
        assert_eq!((now.weekday, now.month, now.year), (2, 11, 2026), "miércoles");
        let dates = [ft(2026, 11, 11), ft(2026, 11, 10), ft(2026, 11, 9), ft(2026, 11, 4), ft(2026, 11, 1), ft(2026, 10, 15), ft(2026, 3, 1), ft(2024, 5, 1), 0];
        let keys: Vec<GroupKey> = dates
            .iter()
            .map(|&d| {
                let mut e = entry("a");
                e.modified = d;
                group_key(Column::Modified, &e, None, now)
            })
            .collect();
        assert_eq!(
            labels(&keys),
            ["Hoy", "Ayer", "Antes en esta semana", "La semana pasada", "Antes en este mes", "El mes pasado", "Antes en este año", "Hace mucho", "Sin fecha"]
        );
        assert!(keys.windows(2).all(|w| w[0].order < w[1].order), "en orden: {keys:?}");
        // En hora local: las 23:30 de ayer en UTC-3 ya son hoy en UTC, pero siguen siendo ayer.
        let local = LocalNow::from_unix(ft(2026, 11, 11) as i64 / 10_000_000 - SECS_1601_TO_1970, -3 * 3600);
        let mut e = entry("a");
        e.created = ft(2026, 11, 11) + 2 * 3600 * 10_000_000;
        // 14:00 UTC del 11 = 11:00 local: hoy.
        assert_eq!(group_key(Column::Created, &e, None, local).label, "Hoy");
        e.created = ft(2026, 11, 11) - 10 * 3600 * 10_000_000;
        // 02:00 UTC del 11 = 23:00 local del 10: ayer.
        assert_eq!(group_key(Column::Created, &e, None, local).label, "Ayer");
    }

    /// Tamaños: los 7 rangos y las carpetas aparte (primero).
    #[test]
    fn group_key_sizes() {
        const KB: u64 = 1024;
        let sizes = [0, 10 * KB, 500 * KB, 50 * KB * KB, 500 * KB * KB, 2 * KB * KB * KB, 5 * KB * KB * KB];
        let mut keys: Vec<GroupKey> = sizes
            .iter()
            .map(|&s| {
                let mut e = entry("a");
                e.size = s;
                group_key(Column::Size, &e, None, FakeNow::now())
            })
            .collect();
        let mut dir = entry("d");
        dir.is_dir = true;
        keys.push(group_key(Column::Size, &dir, None, FakeNow::now()));
        assert_eq!(
            labels(&keys),
            [
                "Vacío",
                "Diminuto (0–16 KB)",
                "Pequeño (16 KB–1 MB)",
                "Mediano (1–128 MB)",
                "Grande (128 MB–1 GB)",
                "Enorme (1–4 GB)",
                "Gigantesco (> 4 GB)",
                "Carpetas"
            ]
        );
        assert!(keys[..7].windows(2).all(|w| w[0].order < w[1].order));
        assert!(keys[7].order < PINNED_FIRST && keys[7] < keys[0], "las carpetas primero");
    }

    /// Nombre: por la primera letra, sin acentos ni mayúsculas.
    #[test]
    fn group_key_names() {
        let keys: Vec<GroupKey> =
            ["1abc", "Álvaro", "ivan", "zeta", "_x"].iter().map(|n| group_key(Column::Name, &entry(n), None, FakeNow::now())).collect();
        assert_eq!(labels(&keys), ["0–9", "A–H", "I–P", "Q–Z", "Otros"]);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
    }

    /// Tipo: un grupo por tipo y las carpetas primero.
    #[test]
    fn group_key_types() {
        let mut dir = entry("fotos");
        dir.is_dir = true;
        dir.type_name = Arc::from("Carpeta de archivos");
        let mut jpg = entry("a.jpg");
        jpg.type_name = Arc::from("Imagen JPEG");
        let mut aac = entry("b.aac");
        aac.type_name = Arc::from("Archivo AAC");
        let (d, j, a) = (
            group_key(Column::Type, &dir, None, FakeNow::now()),
            group_key(Column::Type, &jpg, None, FakeNow::now()),
            group_key(Column::Type, &aac, None, FakeNow::now()),
        );
        assert_eq!((d.label.as_str(), j.label.as_str(), a.label.as_str()), ("Carpeta de archivos", "Imagen JPEG", "Archivo AAC"));
        assert!(d < a && d < j, "las carpetas antes que cualquier tipo");
        assert!(d.order < PINNED_FIRST);
    }

    /// Sin tipo, carpeta o archivo, van al mismo grupo «Sin valor», al final.
    #[test]
    fn group_key_type_without_value_is_one_group() {
        let mut dir = entry("raro");
        dir.is_dir = true;
        dir.type_name = Arc::from("");
        let mut file = entry("x");
        file.type_name = Arc::from("");
        let (d, f) = (group_key(Column::Type, &dir, None, FakeNow::now()), group_key(Column::Type, &file, None, FakeNow::now()));
        assert_eq!(d, f);
        assert_eq!((d.label.as_str(), d.order), ("Sin valor", PINNED_LAST));
    }

    /// Fecha fija para los tests que no dependen de ella.
    struct FakeNow;
    impl FakeNow {
        fn now() -> LocalNow {
            crate::sys::testing::FakeEnv::DEFAULT_NOW
        }
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;
    use crate::geom::rect;
    use crate::keys::keys::{VK_DOWN, VK_LEFT, VK_RIGHT, VK_UP};
    use crate::sys::testing::{RecordingPainter, install_fakes};
    use std::sync::Arc;

    fn entry(name: &str, size: u64) -> Entry {
        Entry {
            name: name.into(),
            full: None,
            group: 0,
            pidl: None,
            display: name.into(),
            sort_key: name.to_lowercase(),
            is_dir: false,
            size,
            modified: 0,
            created: 0,
            attrs: 0,
            type_name: Arc::from("Archivo"),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    /// 8 elementos en 3 grupos por tamaño: «Vacío» (a, b, c), «Diminuto» (d, e) y «Pequeño» (f, g, h).
    fn grouped() -> FileList {
        let mut l = FileList::default();
        l.area = rect(0.0, 0.0, 800.0, 600.0);
        let sizes = [0, 0, 0, 1024, 1024, 100 * 1024, 100 * 1024, 100 * 1024];
        l.append(sizes.iter().enumerate().map(|(i, &s)| entry(&((b'a' + i as u8) as char).to_string(), s)).collect());
        l.set_group(Some(Column::Size));
        l
    }

    fn names(l: &FileList, positions: &[usize]) -> Vec<String> {
        positions.iter().map(|&p| l.entries[l.visible[p] as usize].display.clone()).collect()
    }

    fn headers(l: &FileList) -> Vec<(String, usize, bool)> {
        l.rows()
            .iter()
            .filter_map(|r| match r {
                Row::Header { key, count, collapsed } => Some((key.label.clone(), *count, *collapsed)),
                Row::Item(_) => None,
            })
            .collect()
    }

    /// Agrupar arma encabezados y elementos; plegar el grupo del medio saca sus filas y el
    /// encabezado sigue mostrando cuántos tiene.
    #[test]
    fn rows_group_and_collapse() {
        let mut l = grouped();
        assert_eq!(l.rows().len(), 11);
        assert_eq!(
            headers(&l),
            [("Vacío".into(), 3, false), ("Diminuto (0–16 KB)".into(), 2, false), ("Pequeño (16 KB–1 MB)".into(), 3, false)]
        );
        l.toggle_group("Diminuto (0–16 KB)");
        assert_eq!(l.rows().len(), 9);
        assert_eq!(headers(&l)[1], ("Diminuto (0–16 KB)".into(), 2, true));
        let items: Vec<usize> = l.rows().iter().filter_map(|r| if let Row::Item(i) = r { Some(*i) } else { None }).collect();
        assert!(!items.iter().any(|&i| l.entries[i].size == 1024), "las filas del grupo plegado no aparecen");
        assert_eq!(l.count(), 6, "solo los elementos a la vista son posiciones");
        assert_eq!(l.total(), 8);
        // Los encabezados ocupan su lugar: el primer elemento queda debajo del primero.
        assert_eq!(l.item_rect(0).top, l.rows_area().top + GROUP_H);
        // Desplegar lo vuelve a mostrar; sin agrupar no hay filas.
        l.toggle_group("Diminuto (0–16 KB)");
        assert_eq!(l.count(), 8);
        l.set_group(None);
        assert!(l.rows().is_empty());
        assert_eq!(l.item_rect(0).top, l.rows_area().top);
    }

    /// El teclado saltea los grupos plegados: ↓ desde el último del grupo 1 va al encabezado del
    /// 2 (plegado); → lo despliega; ↓ entra en él.
    #[test]
    fn keyboard_skips_collapsed() {
        let mut l = grouped();
        l.toggle_group("Diminuto (0–16 KB)");
        l.select_index(l.visible[2] as usize);
        assert_eq!(names(&l, &[l.focus.unwrap()]), ["c"]);
        l.on_key(VK_DOWN.0, false, false);
        assert_eq!(l.focused_header().map(|r| l.rows()[r].clone()), Some(Row::Header { key: GroupKey { order: 1, label: "Diminuto (0–16 KB)".into() }, count: 2, collapsed: true }));
        assert_eq!(l.focus_pos(), None, "el foco no está en un elemento");
        l.on_key(VK_RIGHT.0, false, false);
        assert!(!l.collapsed_row(l.focused_header().unwrap()), "→ despliega");
        l.on_key(VK_DOWN.0, false, false);
        assert_eq!(l.focused_header(), None);
        assert_eq!(names(&l, &[l.focus_pos().unwrap()]), ["d"]);
        assert_eq!(names(&l, &l.selected_positions()), ["d"]);
        // ↑ vuelve al encabezado; ← lo pliega y el foco queda en él.
        l.on_key(VK_UP.0, false, false);
        assert!(l.focused_header().is_some());
        l.on_key(VK_LEFT.0, false, false);
        assert!(l.collapsed_row(l.focused_header().unwrap()));
        // ↓ con el grupo plegado salta al encabezado siguiente.
        l.on_key(VK_DOWN.0, false, false);
        let r = l.focused_header().unwrap();
        assert!(matches!(&l.rows()[r], Row::Header { key, .. } if key.label == "Pequeño (16 KB–1 MB)"));
        // Shift+↓ desde ahí entra y selecciona.
        l.on_key(VK_DOWN.0, false, true);
        assert_eq!(names(&l, &[l.focus_pos().unwrap()]), ["f"]);
    }

    /// Con el foco en un encabezado no hay elemento con foco (F2 o Enter no actúan sobre el que
    /// lo tenía antes) y Re Pág / Av Pág cuentan desde el encabezado.
    #[test]
    fn header_focus_has_no_focused_item_and_pages_from_header() {
        use crate::keys::keys::{VK_NEXT, VK_PRIOR};
        let mut l = grouped();
        l.area = rect(0.0, 0.0, 800.0, 110.0);
        let page = l.page_rows();
        assert!((1..5).contains(&page), "página {page}");
        // h (pos 7) y ← lleva al encabezado «Pequeño» (su primer elemento es f, pos 5).
        l.select_index(l.visible[7] as usize);
        l.on_key(VK_LEFT.0, false, false);
        assert!(l.focused_header().is_some());
        assert_eq!(l.focused_index(), None);
        l.on_key(VK_PRIOR.0, false, false);
        assert_eq!(l.focus_pos(), Some(5 - page), "desde el encabezado");
        // Av Pág desde el encabezado «Diminuto» (primer elemento d, pos 3).
        l.select_index(l.visible[4] as usize);
        l.on_key(VK_LEFT.0, false, false);
        assert!(l.focused_header().is_some());
        l.on_key(VK_NEXT.0, false, false);
        assert_eq!(l.focus_pos(), Some((3 + page).min(7)));
    }

    /// Agrupada por fecha, pasada la medianoche los grupos se rearman con el día nuevo: lo de
    /// «Hoy» pasa a «Ayer» en el siguiente dibujo.
    #[test]
    fn date_groups_follow_midnight() {
        use crate::sys::testing::FakeEnv;
        install_fakes();
        let today = FakeEnv::DEFAULT_NOW;
        let unix = today.day * 86_400 + 3_600;
        let mut e = entry("a", 0);
        e.modified = ((unix + 11_644_473_600) * 10_000_000) as u64;
        let mut l = FileList::default();
        l.area = rect(0.0, 0.0, 800.0, 600.0);
        l.append(vec![e]);
        l.set_group(Some(Column::Modified));
        assert_eq!(headers(&l)[0].0, "Hoy");
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)));
        let area = l.area;
        l.draw(&p, area, true);
        assert_eq!(headers(&l)[0].0, "Hoy");
        FakeEnv::set_local_now(crate::sys::LocalNow { day: today.day + 1, weekday: today.weekday + 1, ..today });
        l.draw(&p, area, true);
        let label = headers(&l)[0].0.clone();
        FakeEnv::set_local_now(today);
        assert_eq!(label, "Ayer");
    }

    /// Ctrl+A (seleccionar todo) e invertir solo tocan lo que se ve.
    #[test]
    fn ctrl_a_selects_visible_only() {
        let mut l = grouped();
        l.toggle_group("Pequeño (16 KB–1 MB)");
        l.select_all();
        let selected: Vec<String> = l.selected_entries().map(|e| e.display.clone()).collect();
        assert_eq!(selected, ["a", "b", "c", "d", "e"]);
        l.invert_selection();
        assert_eq!(l.selected_entries().count(), 0);
        // Plegar deselecciona lo que queda oculto.
        l.select_all();
        l.toggle_group("Vacío");
        let selected: Vec<String> = l.selected_entries().map(|e| e.display.clone()).collect();
        assert_eq!(selected, ["d", "e"]);
    }

    /// Plegar el grupo del elemento con foco pasa el foco a su encabezado (nunca queda en un
    /// elemento oculto).
    #[test]
    fn fold_focused_group_moves_focus_to_header() {
        let mut l = grouped();
        l.select_index(l.visible[4] as usize);
        assert_eq!(names(&l, &[l.focus_pos().unwrap()]), ["e"]);
        l.toggle_group("Diminuto (0–16 KB)");
        let r = l.focused_header().expect("el foco pasa al encabezado");
        assert!(matches!(&l.rows()[r], Row::Header { key, collapsed: true, .. } if key.label == "Diminuto (0–16 KB)"));
        assert_eq!(l.focus_pos(), None);
        // Con el mouse: clic en un encabezado lo pliega y le da el foco.
        let mut l = grouped();
        let first = l.rows_area().top + GROUP_H / 2.0;
        assert_eq!(l.header_at(100.0, first), Some(0));
        l.on_mouse_down(100.0, first, false, false);
        assert_eq!(headers(&l)[0].2, true);
        assert_eq!(l.focused_header(), Some(0));
        assert_eq!(l.pos_at(100.0, first), None, "un encabezado no es un elemento");
    }

    /// Dibujar no rearma las filas: solo cambiar elementos, orden, criterio o plegado.
    #[test]
    fn rows_not_recomputed_each_frame() {
        install_fakes();
        let mut l = grouped();
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)));
        let before = l.rows_built;
        for _ in 0..10 {
            l.draw(&p, rect(0.0, 0.0, 800.0, 600.0), true);
            l.tick(1.0 / 60.0);
        }
        assert_eq!(l.rows_built, before, "dibujar no recalcula");
        // Cambiar el ancho recalcula la geometría, no las filas.
        l.set_view(View::Icons);
        let before = l.rows_built;
        l.draw(&p, rect(0.0, 0.0, 500.0, 600.0), true);
        l.draw(&p, rect(0.0, 0.0, 900.0, 600.0), true);
        assert_eq!(l.rows_built, before);
        l.toggle_group("Vacío");
        assert_eq!(l.rows_built, before + 1);
        l.set_sort(Column::Size);
        assert_eq!(l.rows_built, before + 2);
    }

    /// El encabezado se dibuja como en el diseño: «Hoy (3)» en seminegrita 13 (`UiBold`), con
    /// la flecha abajo (desplegado) en `GlyphsSmall`.
    #[test]
    fn group_header_drawn_like_design() {
        install_fakes();
        let now = crate::sys::testing::FakeEnv::DEFAULT_NOW;
        // Mediodía del día de hoy del falso, en FILETIME.
        let today = ((now.day * 86_400 + 12 * 3600 + SECS_1601_TO_1970) * 10_000_000) as u64;
        let mut l = FileList::default();
        let mut files: Vec<Entry> = ["a", "b", "c"].iter().map(|n| entry(n, 10)).collect();
        for f in &mut files {
            f.modified = today;
        }
        let mut old = entry("viejo", 10);
        old.modified = today - 400 * 86_400 * 10_000_000;
        files.push(old);
        l.append(files);
        l.set_group(Some(Column::Modified));
        l.toggle_group("Hace mucho");
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)));
        l.draw(&p, rect(0.0, 0.0, 800.0, 400.0), true);
        let texts = p.texts();
        let hoy: Vec<_> = texts.iter().filter(|(_, s, _)| s == "Hoy (3)").collect();
        assert_eq!(hoy.len(), 1, "{texts:?}");
        assert_eq!(hoy[0].0, FontId::UiBold);
        assert!(texts.iter().any(|(f, s, _)| *f == FontId::GlyphsSmall && s == "\u{E70D}"), "flecha abajo: {texts:?}");
        assert!(texts.iter().any(|(f, s, _)| *f == FontId::GlyphsSmall && s == "\u{E76C}"), "flecha a la derecha (plegado)");
        assert!(texts.iter().any(|(_, s, _)| s == "Hace mucho (1)"));
    }

    /// Agrupar en Contenido: encabezados de fila completa entre las filas altas de cada grupo.
    #[test]
    fn content_view_groups_headers() {
        install_fakes();
        let mut l = grouped();
        l.set_view(View::Content);
        let p = RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(false)));
        l.draw(&p, rect(0.0, 0.0, 800.0, 600.0), true);
        let texts = p.texts();
        for h in ["Vacío (3)", "Diminuto (0–16 KB) (2)"] {
            assert!(texts.iter().any(|(f, s, _)| *f == FontId::UiBold && s == h), "{h}: {texts:?}");
        }
        let rows = l.rows_area();
        let layout = l.layout.as_ref().unwrap();
        let header = layout.lines[0];
        assert!(header.header.is_some());
        let r = l.line_rect(&header);
        assert_eq!((r.left, r.right), (rows.left, rows.right - SCROLLBAR_W), "de lado a lado");
        // Un elemento por línea, de alto de Contenido; el grupo siguiente arranca debajo de su encabezado.
        let (i0, i2, i3) = (l.item_rect(0), l.item_rect(2), l.item_rect(3));
        assert_eq!(i0.top, rows.top + 4.0 + GROUP_H);
        assert_eq!(i0.bottom - i0.top, super::super::CONTENT_H);
        assert_eq!((i0.left, i0.right), (rows.left, rows.right - SCROLLBAR_W));
        assert_eq!(i3.top, i2.bottom + GROUP_GAP + GROUP_H);
        assert_eq!(l.header_at(100.0, rows.top + 4.0 + GROUP_H / 2.0), Some(0));
        assert_eq!(l.pos_at(100.0, (i3.top + i3.bottom) / 2.0), Some(3));
    }
}

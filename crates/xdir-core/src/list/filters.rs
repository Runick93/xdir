//! Filtros por columna (como los del encabezado del Explorador): en cada columna se tildan grupos
//! de `group_key` (los mismos de «Agrupar por») y se ven los elementos de cualquiera de ellos;
//! filtros en varias columnas se combinan (tiene que cumplir todos). Orden de aplicación: filtros
//! por columna → filtro rápido (la búsqueda, que ya trae sus resultados) → orden → grupos.

use super::groups::{PINNED_LAST, natural_descending};
use super::{Column, FileList, GroupKey, HEADER_H, LocalNow, group_key};
use crate::fs::Entry;
use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};

/// Desplegable de filtros (diseño `list::FileList::draw_filter_menu`, pantalla X33): ancho, alto
/// de cada opción y margen de arriba y de abajo.
pub const MENU_W: f32 = 240.0;
pub const OPTION_H: f32 = 28.0;
const MENU_PAD: f32 = 6.0;
/// Separador antes de «Quitar el filtro»: 4 de aire, la línea y 4 más.
const SEP_H: f32 = 9.0;
/// Opciones a la vista como mínimo aunque el panel sea bajo (las demás, con la rueda).
const MIN_ROWS: usize = 5;
/// Botón de la flecha en el encabezado: 20 × 21, a 4 del borde derecho de la columna y de arriba.
const BUTTON_W: f32 = 20.0;
const BUTTON_H: f32 = 21.0;
const BUTTON_INSET: f32 = 4.0;
/// Lo que el botón le quita al título de la columna (el botón y su aire).
pub(super) const BUTTON_ROOM: f32 = BUTTON_W + BUTTON_INSET * 2.0;

/// Lo que hay bajo el mouse en el encabezado o en el desplegable de filtros.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListHit {
    /// La flecha de la columna en el encabezado.
    HeaderFilter(Column),
    /// Una opción del desplegable (índice en `filter_options`).
    FilterOption(usize),
    /// «Quitar el filtro».
    FilterClear,
}

/// Desplegable abierto: la columna, sus opciones (se calculan al abrirlo y cuando cambian los
/// elementos o los filtros, no en cada cuadro), lo que está bajo el mouse y la primera opción a la
/// vista (si no entran todas).
pub(super) struct FilterMenu {
    pub col: Column,
    pub options: Vec<(GroupKey, usize, bool)>,
    pub hover: Option<ListHit>,
    pub first: usize,
}

/// ¿Pasa `e` los filtros (salvo el de la columna `except`)? Un filtro sin grupos no filtra.
fn passes(filters: &[(Column, Vec<String>)], e: &Entry, now: LocalNow, except: Option<Column>) -> bool {
    filters.iter().filter(|(c, _)| Some(*c) != except).all(|(col, labels)| {
        labels.is_empty() || {
            let key = group_key(*col, e, None, now);
            labels.iter().any(|l| *l == key.label)
        }
    })
}

/// Embudo del diseño (10 × 10): se dibuja con un trazo, no con un glifo.
const FUNNEL: [(f32, f32); 6] = [(0.0, 0.0), (10.0, 0.0), (6.0, 5.0), (6.0, 10.0), (4.0, 9.0), (4.0, 5.0)];
/// Tilde de la casilla (8 × 7).
const CHECK: [(f32, f32); 3] = [(0.0, 3.7331), (2.8, 7.0), (8.0, 0.0)];
const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

impl FileList {
    /// Fecha de hoy si algún filtro (o la columna `col`) es de fechas; si no, no hace falta.
    fn filter_now_for(&self, col: Option<Column>) -> LocalNow {
        if self.filters.iter().map(|(c, _)| *c).chain(col).any(natural_descending) {
            crate::sys::env().local_now()
        } else {
            LocalNow::from_unix(0, 0)
        }
    }

    pub(super) fn filter_now(&self) -> LocalNow {
        self.filter_now_for(None)
    }

    /// ¿El elemento pasa los filtros por columna? (`now`: ver `filter_now`).
    pub(super) fn passes_filters(&self, e: &Entry, now: LocalNow) -> bool {
        passes(&self.filters, e, now, None)
    }

    /// Opciones del filtro de la columna: cada grupo con elementos (de los que dejan pasar los
    /// ocultos y los filtros de las demás columnas), con su cantidad y si está tildado, en el orden
    /// de los grupos. Un grupo tildado sin elementos sigue apareciendo (con 0) para poder destildarlo.
    pub fn filter_options(&self, col: Column) -> Vec<(GroupKey, usize, bool)> {
        let checked: &[String] = self.filters.iter().find(|(c, _)| *c == col).map_or(&[], |(_, l)| l.as_slice());
        let now = self.filter_now_for(Some(col));
        let mut counts: std::collections::HashMap<GroupKey, usize> = std::collections::HashMap::new();
        for e in self.entries.iter().filter(|e| self.shows(e) && passes(&self.filters, e, now, Some(col))) {
            *counts.entry(group_key(col, e, None, now)).or_default() += 1;
        }
        let mut out: Vec<(GroupKey, usize, bool)> = counts
            .into_iter()
            .map(|(k, n)| {
                let on = checked.contains(&k.label);
                (k, n, on)
            })
            .collect();
        for l in checked {
            if !out.iter().any(|o| o.0.label == *l) {
                out.push((GroupKey { order: PINNED_LAST + 1, label: l.clone() }, 0, true));
            }
        }
        out.sort_by(|a, b| a.0.order.cmp(&b.0.order).then_with(|| crate::format::caseless_cmp(&a.0.label, &b.0.label)));
        out
    }

    /// Filtra la columna por esos grupos (por nombre); vacío quita el filtro de la columna. El foco
    /// se queda en su elemento si sigue a la vista; si no, pasa al primero.
    pub fn set_filter(&mut self, col: Column, labels: Vec<String>) {
        match (self.filters.iter().position(|(c, _)| *c == col), labels.is_empty()) {
            (Some(i), true) => {
                self.filters.remove(i);
            }
            (Some(i), false) => self.filters[i].1 = labels,
            (None, true) => return,
            (None, false) => self.filters.push((col, labels)),
        }
        self.refilter();
    }

    /// «Quitar los filtros»: los de todas las columnas.
    pub fn clear_filters(&mut self) {
        if !self.filters.is_empty() {
            self.filters.clear();
            self.refilter();
        }
    }

    /// Rearma lo que se ve después de cambiar los filtros, conservando el foco si se puede.
    pub(super) fn refilter(&mut self) {
        let focus = self.focused_index();
        self.rebuild(focus);
        self.clamp_scroll();
        if let Some(f) = self.focus.filter(|_| self.header_focus.is_none()) {
            self.ensure_visible(f);
        }
    }

    /// ¿Hay filtros por columna? (la barra de estado dice «(filtrado)»).
    pub fn is_filtered(&self) -> bool {
        !self.filters.is_empty()
    }

    /// ¿La columna está filtrada?
    pub fn is_column_filtered(&self, col: Column) -> bool {
        self.filters.iter().any(|(c, _)| *c == col)
    }

    /// Columna del desplegable abierto.
    pub fn filter_menu(&self) -> Option<Column> {
        self.menu.as_ref().map(|m| m.col)
    }

    /// Columna del encabezado bajo el mouse (muestra su flecha).
    pub fn header_hover(&self) -> Option<Column> {
        self.header_hover
    }

    pub fn open_filter_menu(&mut self, col: Column) {
        if self.view.is_grid() {
            return;
        }
        self.header_drag = None;
        self.menu = Some(FilterMenu { col, options: self.filter_options(col), hover: None, first: 0 });
    }

    /// Cierra el desplegable. Devuelve si estaba abierto.
    pub fn close_filter_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    /// Vuelve a contar las opciones del desplegable abierto (cambiaron los elementos o los filtros).
    pub(super) fn refresh_filter_menu(&mut self) {
        if let Some(col) = self.filter_menu() {
            let options = self.filter_options(col);
            if let Some(m) = &mut self.menu {
                m.first = m.first.min(options.len().saturating_sub(1));
                m.options = options;
            }
        }
    }

    /// ¿Se ve la flecha de la columna? (mouse encima o su desplegable abierto).
    pub(super) fn shows_filter_button(&self, col: Column) -> bool {
        self.header_hover == Some(col) || self.filter_menu() == Some(col)
    }

    /// Botón de la flecha de la columna en el encabezado (Detalles).
    pub fn filter_button_rect(&self, col: Column) -> Option<Rect> {
        if self.view.is_grid() {
            return None;
        }
        let (_, _, left, right) = self.layout_columns().into_iter().find(|c| c.1 == col)?;
        let bl = (right - BUTTON_INSET - BUTTON_W).max(left);
        let top = self.area.top + BUTTON_INSET;
        Some(rect(bl, top, right - BUTTON_INSET, top + BUTTON_H))
    }

    /// Flecha del encabezado bajo el punto.
    pub(super) fn header_filter_at(&self, x: f32, y: f32) -> Option<Column> {
        if !self.in_header(x, y) {
            return None;
        }
        self.layout_columns().into_iter().map(|c| c.1).find(|&col| self.filter_button_rect(col).is_some_and(|r| contains(&r, x, y)))
    }

    /// Opciones del desplegable a la vista (las demás se ven con la rueda).
    fn menu_rows(&self) -> usize {
        let Some(m) = &self.menu else { return 0 };
        let top = self.area.top + HEADER_H;
        let fit = ((self.area.bottom - top - MENU_PAD * 2.0 - SEP_H - OPTION_H) / OPTION_H).floor().max(0.0) as usize;
        m.options.len().min(fit.max(MIN_ROWS))
    }

    /// Rectángulo del desplegable: bajo el encabezado, pegado al borde derecho de su columna.
    pub fn filter_menu_rect(&self) -> Option<Rect> {
        let m = self.menu.as_ref()?;
        let (_, _, _, right) = self.layout_columns().into_iter().find(|c| c.1 == m.col)?;
        let left = (right - MENU_W).max(self.area.left);
        let top = self.area.top + HEADER_H;
        let h = MENU_PAD * 2.0 + self.menu_rows() as f32 * OPTION_H + SEP_H + OPTION_H;
        Some(rect(left, top, left + MENU_W, top + h))
    }

    /// Fila de la opción `i` (índice en las opciones), si está a la vista.
    pub fn filter_option_rect(&self, i: usize) -> Option<Rect> {
        let m = self.menu.as_ref()?;
        let r = self.filter_menu_rect()?;
        let row = i.checked_sub(m.first).filter(|&k| k < self.menu_rows() && i < m.options.len())?;
        let y = r.top + MENU_PAD + row as f32 * OPTION_H;
        Some(rect(r.left, y, r.right, y + OPTION_H))
    }

    /// Fila de «Quitar el filtro».
    pub fn filter_clear_rect(&self) -> Option<Rect> {
        let r = self.filter_menu_rect()?;
        Some(rect(r.left, r.bottom - MENU_PAD - OPTION_H, r.right, r.bottom - MENU_PAD))
    }

    /// Desplegable bajo el punto: `None` fuera de él; `Some(None)` dentro, sobre nada.
    pub fn filter_menu_at(&self, x: f32, y: f32) -> Option<Option<ListHit>> {
        let r = self.filter_menu_rect()?;
        if !contains(&r, x, y) {
            return None;
        }
        if self.filter_clear_rect().is_some_and(|c| contains(&c, x, y)) {
            return Some(Some(ListHit::FilterClear));
        }
        let first = self.menu.as_ref().map_or(0, |m| m.first);
        Some((first..first + self.menu_rows()).find(|&i| self.filter_option_rect(i).is_some_and(|o| contains(&o, x, y))).map(ListHit::FilterOption))
    }

    /// Flecha del encabezado, opción o «Quitar el filtro» bajo el punto.
    pub fn filter_hit(&self, x: f32, y: f32) -> Option<ListHit> {
        match self.filter_menu_at(x, y) {
            Some(hit) => hit,
            None => self.header_filter_at(x, y).map(ListHit::HeaderFilter),
        }
    }

    /// Clic en el desplegable o en una flecha: una opción tilda o destilda su grupo (el
    /// desplegable sigue abierto); «Quitar el filtro» lo quita y cierra; una flecha abre (o
    /// cierra) el desplegable de su columna.
    pub fn click_filter_menu(&mut self, hit: ListHit) {
        match hit {
            ListHit::HeaderFilter(col) => {
                if self.filter_menu() == Some(col) {
                    self.close_filter_menu();
                } else {
                    self.open_filter_menu(col);
                }
            }
            ListHit::FilterOption(i) => {
                let Some(m) = &self.menu else { return };
                let Some((key, _, on)) = m.options.get(i) else { return };
                let col = m.col;
                let label = key.label.clone();
                let mut labels: Vec<String> = self.filters.iter().find(|(c, _)| *c == col).map(|(_, l)| l.clone()).unwrap_or_default();
                if *on {
                    labels.retain(|l| *l != label);
                } else {
                    labels.push(label);
                }
                self.set_filter(col, labels);
            }
            ListHit::FilterClear => {
                if let Some(col) = self.filter_menu() {
                    self.set_filter(col, Vec::new());
                }
                self.close_filter_menu();
            }
        }
    }

    /// Lo que está bajo el mouse en el desplegable. Devuelve si cambió (hay que redibujar).
    pub fn set_filter_hover(&mut self, hover: Option<ListHit>) -> bool {
        match &mut self.menu {
            Some(m) if m.hover != hover => {
                m.hover = hover;
                true
            }
            _ => false,
        }
    }

    /// Rueda sobre el desplegable (cuando no entran todas las opciones). Devuelve si se movió.
    pub fn filter_menu_wheel(&mut self, rows: f32) -> bool {
        let shown = self.menu_rows();
        let Some(m) = &mut self.menu else { return false };
        let max = m.options.len().saturating_sub(shown);
        let first = (m.first as i64 - rows.round() as i64).clamp(0, max as i64) as usize;
        let moved = first != m.first;
        m.first = first;
        moved
    }

    /// Encabezado de una columna: con el mouse encima (o el desplegable abierto) el fondo de
    /// resaltado y la flecha; si la columna está filtrada, el embudo después del título.
    /// `title_right` es donde termina el título.
    pub(super) fn draw_header_filter(&self, p: &dyn Painter, col: Column, left: f32, title_right: f32) {
        let th = p.theme();
        if self.is_column_filtered(col) {
            let title_w = p.measure(FontId::UiBold, col.title());
            let fx = if col.right_aligned() { title_right - title_w - 16.0 } else { (left + super::PAD + title_w).min(title_right) + 6.0 };
            let room = if self.shows_filter_button(col) { BUTTON_ROOM } else { BUTTON_INSET };
            let right = self.layout_columns().into_iter().find(|c| c.1 == col).map_or(f32::MAX, |c| c.3);
            if fx > left && fx + 10.0 <= right - room {
                let fy = self.area.top + 10.0;
                let pts: Vec<(f32, f32, f32)> = FUNNEL.iter().map(|&(x, y)| (fx + x, fy + y, 0.0)).collect();
                p.rounded_path(&pts, true, Some(th.accent), None);
            }
        }
        if !self.shows_filter_button(col) {
            return;
        }
        let Some(b) = self.filter_button_rect(col) else { return };
        let open = self.filter_menu() == Some(col);
        if open {
            p.round_fill(b, 4.0, th.selection);
        }
        p.text(FontId::Glyphs, "\u{E70D}", b, if open { th.text } else { th.text_dim });
    }

    /// Desplegable de filtros (diseño `list::FileList::draw_filter_menu`): sombra y panel, una
    /// casilla por grupo con «nombre (cantidad)», separador y «Quitar el filtro». Lo dibuja la
    /// `App` encima de los paneles.
    pub fn draw_filter_menu(&self, p: &dyn Painter) {
        let (Some(m), Some(r)) = (&self.menu, self.filter_menu_rect()) else { return };
        let th = p.theme();
        p.round_fill(rect(r.left - 2.0, r.top, r.right + 2.0, r.bottom + 4.0), 10.0, crate::theme::alpha(th.background, 0.5));
        p.round_fill(r, 8.0, th.surface_alt);
        p.round_stroke(r, 8.0, th.border);
        for i in m.first..m.first + self.menu_rows() {
            let (Some(row), Some((key, count, on))) = (self.filter_option_rect(i), m.options.get(i)) else { continue };
            if m.hover == Some(ListHit::FilterOption(i)) {
                p.round_fill(rect(row.left + 4.0, row.top, row.right - 4.0, row.bottom), 6.0, th.hover);
            }
            let cy = (row.top + row.bottom) / 2.0;
            let bx = rect(row.left + 12.0, cy - 7.0, row.left + 26.0, cy + 7.0);
            if *on {
                p.round_fill(bx, 3.0, th.accent);
                let pts: Vec<(f32, f32, f32)> = CHECK.iter().map(|&(x, y)| (bx.left + 3.0 + x, bx.top + 3.5 + y, 0.0)).collect();
                p.rounded_path(&pts, false, None, Some((WHITE, 1.6)));
            } else {
                p.round_stroke_w(bx, 3.0, th.text_dim, 1.2);
            }
            p.text(FontId::Ui, &format!("{} ({count})", key.label), rect(row.left + 36.0, row.top, row.right - 12.0, row.bottom), th.text);
        }
        let Some(clear) = self.filter_clear_rect() else { return };
        let sep = clear.top - 5.0;
        p.fill(rect(r.left + 8.0, sep, r.right - 8.0, sep + 1.0), th.border);
        let filtered = self.is_column_filtered(m.col);
        if filtered && m.hover == Some(ListHit::FilterClear) {
            p.round_fill(rect(clear.left + 4.0, clear.top, clear.right - 4.0, clear.bottom), 6.0, th.hover);
        }
        let color = if filtered { th.text } else { th.text_dim };
        p.text(FontId::Ui, "Quitar el filtro", rect(clear.left + 36.0, clear.top, clear.right - 12.0, clear.bottom), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::Row;

    /// Elementos a la vista, en orden.
    fn visible(l: &FileList) -> impl Iterator<Item = &Entry> {
        (0..l.count()).map(|p| &l.entries[l.entry_of_pos(p).unwrap()])
    }
    use crate::sys::testing::install_fakes;
    use std::sync::Arc;

    fn entry(name: &str, type_name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            full: None,
            group: 0,
            pidl: None,
            display: name.into(),
            sort_key: name.to_lowercase(),
            is_dir,
            size: 0,
            modified: 0,
            created: 0,
            attrs: 0,
            type_name: Arc::from(type_name),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    /// Carpeta de las fotos del diseño (X33): 4 carpetas, 12 JPEG, 3 PNG y 2 PDF.
    fn photos() -> FileList {
        let mut l = FileList::default();
        l.area = rect(0.0, 0.0, 900.0, 600.0);
        let mut v = Vec::new();
        for i in 0..4 {
            v.push(entry(&format!("carpeta{i}"), "Carpeta de archivos", true));
        }
        for i in 0..12 {
            v.push(entry(&format!("IMG_{}.jpg", 2031 + i), "Imagen JPEG", false));
        }
        for n in ["captura-mapa.png", "logo-runick.png", "plano-casa.png"] {
            v.push(entry(n, "Imagen PNG", false));
        }
        for n in ["factura.pdf", "manual.pdf"] {
            v.push(entry(n, "Documento PDF", false));
        }
        l.append(v);
        l
    }

    fn options(l: &FileList, col: Column) -> Vec<(String, usize, bool)> {
        l.filter_options(col).into_iter().map(|(k, n, on)| (k.label, n, on)).collect()
    }

    /// Las opciones del filtro de una columna son sus grupos con elementos en la carpeta, con la
    /// cantidad de cada uno: las carpetas primero y después los tipos por nombre.
    #[test]
    fn filter_options_counts() {
        let l = photos();
        assert_eq!(
            options(&l, Column::Type),
            [
                ("Carpeta de archivos".to_string(), 4, false),
                ("Documento PDF".to_string(), 2, false),
                ("Imagen JPEG".to_string(), 12, false),
                ("Imagen PNG".to_string(), 3, false),
            ]
        );
        assert!(!l.is_filtered());
    }

    /// Tildar JPEG y PNG muestra los elementos de cualquiera de los dos; las opciones siguen
    /// mostrando todos los grupos, con los tildados marcados.
    #[test]
    fn filter_keeps_any_checked() {
        let mut l = photos();
        l.set_filter(Column::Type, vec!["Imagen JPEG".into(), "Imagen PNG".into()]);
        assert_eq!(l.count(), 15);
        assert_eq!(l.total(), 15);
        assert!(l.is_filtered());
        assert!(visible(&l).all(|e| e.type_name.starts_with("Imagen")));
        let opts = options(&l, Column::Type);
        assert_eq!(opts.len(), 4);
        assert_eq!(opts.iter().filter(|o| o.2).map(|o| o.0.as_str()).collect::<Vec<_>>(), ["Imagen JPEG", "Imagen PNG"]);
        // Vacío = quitar el filtro de la columna.
        l.set_filter(Column::Type, Vec::new());
        assert_eq!(l.count(), 21);
        assert!(!l.is_filtered());
        assert!(l.filters.is_empty());
    }

    /// Filtros en dos columnas se combinan: Tipo = JPEG y Fecha = Hoy deja solo los JPEG de hoy.
    #[test]
    fn filters_combine_across_columns() {
        install_fakes();
        let now = crate::sys::testing::FakeEnv::DEFAULT_NOW;
        // Mediodía de hoy (FILETIME) y un año antes.
        let today = ((now.day * 86_400 + 12 * 3600 + 11_644_473_600) * 10_000_000) as u64;
        let old = today - 400 * 86_400 * 10_000_000;
        let mut l = FileList::default();
        let mut v = Vec::new();
        for (name, t, date) in
            [("a.jpg", "Imagen JPEG", today), ("b.jpg", "Imagen JPEG", old), ("c.png", "Imagen PNG", today), ("d.jpg", "Imagen JPEG", today)]
        {
            let mut e = entry(name, t, false);
            e.modified = date;
            v.push(e);
        }
        l.append(v);
        l.set_filter(Column::Type, vec!["Imagen JPEG".into()]);
        l.set_filter(Column::Modified, vec!["Hoy".into()]);
        let names: Vec<&str> = visible(&l).map(|e| e.display.as_str()).collect();
        assert_eq!(names, ["a.jpg", "d.jpg"]);
        // Las opciones de una columna cuentan lo que dejan pasar los filtros de las otras.
        assert_eq!(options(&l, Column::Modified), [("Hoy".to_string(), 2, true), ("Hace mucho".to_string(), 1, false)]);
        assert_eq!(options(&l, Column::Type), [("Imagen JPEG".to_string(), 2, true), ("Imagen PNG".to_string(), 1, false)]);
        // Los filtros también valen para lo que llega después (carga progresiva).
        let mut late = entry("e.jpg", "Imagen JPEG", false);
        late.modified = old;
        let mut late2 = entry("f.jpg", "Imagen JPEG", false);
        late2.modified = today;
        l.append(vec![late, late2]);
        let names: Vec<&str> = visible(&l).map(|e| e.display.as_str()).collect();
        assert_eq!(names, ["a.jpg", "d.jpg", "f.jpg"]);
    }

    /// Si el elemento con foco queda filtrado, el foco pasa al primero a la vista (y lo oculto se
    /// deselecciona).
    #[test]
    fn filter_hides_focused_moves_focus() {
        let mut l = photos();
        let pdf = l.entries.iter().position(|e| e.display == "manual.pdf").unwrap();
        l.select_index(pdf);
        assert_eq!(l.focused_index(), Some(pdf));
        l.set_filter(Column::Type, vec!["Imagen PNG".into()]);
        assert_eq!(l.focus_pos(), Some(0));
        let focused = l.focused_index().map(|i| l.entries[i].display.clone());
        assert_eq!(focused.as_deref(), Some("captura-mapa.png"));
        assert_eq!(l.selected_entries().count(), 0, "lo oculto no queda seleccionado");
    }

    /// Con la lista agrupada, los grupos se arman con lo que pasa el filtro.
    #[test]
    fn filter_before_groups() {
        let mut l = photos();
        l.set_group(Some(Column::Type));
        l.set_filter(Column::Type, vec!["Imagen PNG".into(), "Documento PDF".into()]);
        let headers: Vec<(String, usize)> = l
            .rows()
            .iter()
            .filter_map(|r| match r {
                Row::Header { key, count, .. } => Some((key.label.clone(), *count)),
                Row::Item(_) => None,
            })
            .collect();
        assert_eq!(headers, [("Documento PDF".to_string(), 2), ("Imagen PNG".to_string(), 3)]);
    }

    /// El desplegable: la flecha del encabezado lo abre; una opción tilda o destilda su grupo
    /// (el desplegable sigue abierto); «Quitar el filtro» lo quita y lo cierra.
    #[test]
    fn filter_menu_hits() {
        install_fakes();
        let mut l = photos();
        let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        l.draw(&p, rect(0.0, 0.0, 900.0, 600.0), true);
        let b = l.filter_button_rect(Column::Type).expect("botón del filtro");
        let (bx, by) = ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
        assert_eq!(l.filter_hit(bx, by), Some(ListHit::HeaderFilter(Column::Type)));
        l.on_mouse_down(bx, by, false, false);
        assert_eq!(l.filter_menu(), Some(Column::Type));
        let r = l.filter_option_rect(2).unwrap();
        let (ox, oy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        assert_eq!(l.filter_hit(ox, oy), Some(ListHit::FilterOption(2)));
        l.click_filter_menu(ListHit::FilterOption(2));
        assert_eq!(l.filters, [(Column::Type, vec!["Imagen JPEG".to_string()])]);
        assert_eq!(l.filter_menu(), Some(Column::Type), "sigue abierto");
        l.click_filter_menu(ListHit::FilterOption(2));
        assert!(l.filters.is_empty(), "destildar el único quita el filtro");
        l.click_filter_menu(ListHit::FilterOption(3));
        assert_eq!(l.count(), 3);
        let c = l.filter_clear_rect().unwrap();
        assert_eq!(l.filter_hit((c.left + c.right) / 2.0, (c.top + c.bottom) / 2.0), Some(ListHit::FilterClear));
        l.click_filter_menu(ListHit::FilterClear);
        assert!(l.filters.is_empty());
        assert_eq!(l.filter_menu(), None);
        assert_eq!(l.count(), 21);
    }

    /// Dibujo según `draw_filter_menu` (X33): opciones «Imagen JPEG (12)» en `Ui`, «Quitar el
    /// filtro», y el embudo en el encabezado de la columna filtrada.
    #[test]
    fn filter_menu_drawn_like_design() {
        install_fakes();
        let mut l = photos();
        let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        l.draw(&p, rect(0.0, 0.0, 900.0, 600.0), true);
        l.set_filter(Column::Type, vec!["Imagen JPEG".into()]);
        l.open_filter_menu(Column::Type);
        l.draw(&p, rect(0.0, 0.0, 900.0, 600.0), true);
        l.draw_filter_menu(&p);
        let texts = p.texts();
        let jpeg: Vec<_> = texts.iter().filter(|(_, s, _)| s == "Imagen JPEG (12)").collect();
        assert_eq!(jpeg.len(), 1, "{texts:?}");
        assert_eq!(jpeg[0].0, FontId::Ui);
        assert!(texts.iter().any(|(_, s, _)| s == "Quitar el filtro"));
        let menu = l.filter_menu_rect().unwrap();
        assert_eq!(menu.right - menu.left, MENU_W);
        assert_eq!(menu.bottom - menu.top, 161.0, "alto del diseño con 4 opciones");
        let tipo = l.filter_button_rect(Column::Type).unwrap();
        assert_eq!((menu.right, menu.top), (tipo.right + 4.0, HEADER_H), "pegado al borde derecho de la columna, bajo el encabezado");
        // El embudo de la columna filtrada: un trazo cerrado relleno del acento.
        assert!(p.paths().iter().any(|(pts, closed, fill)| *closed && pts.len() == 6 && *fill == Some(p.theme().accent)), "falta el embudo");
    }
}

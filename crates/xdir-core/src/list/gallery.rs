//! Vista Galería (diseño X42, `list::FileList::draw_gallery` y `list::FileList::draw_strip_cell`):
//! arriba la imagen grande del elemento con foco y abajo una tira horizontal de miniaturas con el
//! enfocado centrado. La tira muestra `visible` en orden, sin encabezados de grupo.

use super::{Action, CheckState, FileList, draw_checkbox, grow};
use crate::geom::{Rect, contains, rect};
use crate::paint::Painter;
use crate::theme::alpha;
use crate::thumbs;

/// Alto de la tira de miniaturas (con la línea `border` arriba).
pub const STRIP_H: f32 = 112.0;
/// Lado de cada celda de la tira.
pub const CELL: f32 = 88.0;
/// Distancia entre el borde izquierdo de una celda y el de la siguiente.
pub const CELL_STEP: f32 = 96.0;
/// Lado de la miniatura dentro de la celda.
pub const STRIP_THUMB: f32 = 72.0;
/// Margen de la imagen grande y de los extremos de la tira.
pub const GALLERY_PAD: f32 = 16.0;

/// Margen de arriba de las celdas dentro de la tira.
const CELL_TOP: f32 = (STRIP_H - CELL) / 2.0;
/// Casilla de selección: desde la esquina de arriba a la izquierda de la celda.
pub(super) const CHECK_INSET: f32 = 12.0;
/// Ícono de las carpetas y de lo que no tiene imagen: en la tira y en grande.
const STRIP_ICON: f32 = 48.0;
const BIG_ICON: f32 = 96.0;
/// Rueda: filas (`on_wheel`) por paso; una muesca del mouse son 3.
const WHEEL_STEP: f32 = 3.0;

/// Corrimiento de la tira (cuánto se mueve a la izquierda la celda 0 respecto de su lugar a
/// `GALLERY_PAD` del borde) para que el enfocado quede centrado; en los extremos la tira no se
/// corre más allá de su principio o su fin. Si todo entra, el conjunto va centrado (negativo).
pub fn strip_offset(count: usize, focus: usize, width: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let content = GALLERY_PAD * 2.0 + count as f32 * CELL_STEP - (CELL_STEP - CELL);
    if content <= width {
        return -(width - content) / 2.0;
    }
    let center = GALLERY_PAD + focus.min(count - 1) as f32 * CELL_STEP + CELL / 2.0;
    (center - width / 2.0).clamp(0.0, content - width)
}

/// Estado de una celda de la tira (diseño `list::FileList::draw_strip_cell`). `Focus` (foco sin
/// selección, después de Ctrl+flecha) no está en el diseño: fondo tenue y borde del acento, como
/// en la lista.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellState {
    Normal,
    Hover,
    Selected,
    SelectedFocus,
    Focus,
}

impl FileList {
    /// La tira, abajo de todo el área.
    fn strip_rect(&self) -> Rect {
        let a = self.area;
        rect(a.left, (a.bottom - STRIP_H).max(a.top), a.right, a.bottom)
    }

    /// Dónde va la imagen grande: arriba de la tira, con su margen.
    pub(super) fn gallery_image_rect(&self) -> Rect {
        let (a, s) = (self.area, self.strip_rect());
        rect(a.left + GALLERY_PAD, a.top + GALLERY_PAD, (a.right - GALLERY_PAD).max(a.left + GALLERY_PAD), (s.top - GALLERY_PAD).max(a.top + GALLERY_PAD))
    }

    fn strip_shift(&self) -> f32 {
        let s = self.strip_rect();
        strip_offset(self.visible.len(), self.focus.unwrap_or(0), s.right - s.left)
    }

    /// Celda de la posición `pos` en la tira.
    pub(super) fn strip_cell_rect(&self, pos: usize) -> Rect {
        let s = self.strip_rect();
        let x = s.left + GALLERY_PAD + pos as f32 * CELL_STEP - self.strip_shift();
        let y = s.top + CELL_TOP;
        rect(x, y, x + CELL, y + CELL)
    }

    /// Celdas a la vista: [primera, última).
    pub(super) fn strip_range(&self) -> (usize, usize) {
        let n = self.visible.len();
        let s = self.strip_rect();
        let origin = s.left + GALLERY_PAD - self.strip_shift();
        // La celda i se ve si origin + i·paso + CELL > izquierda y origin + i·paso < derecha.
        let first = (((s.left - origin - CELL) / CELL_STEP).floor() + 1.0).max(0.0) as usize;
        let last = ((s.right - origin) / CELL_STEP).ceil().max(0.0) as usize;
        (first.min(n), last.min(n))
    }

    /// Celda bajo el punto (posición en `visible`); el espacio entre celdas no cuenta.
    pub(super) fn strip_pos_at(&self, x: f32, y: f32) -> Option<usize> {
        let s = self.strip_rect();
        if !contains(&s, x, y) {
            return None;
        }
        let origin = s.left + GALLERY_PAD - self.strip_shift();
        let i = ((x - origin) / CELL_STEP).floor();
        if i < 0.0 || i as usize >= self.visible.len() {
            return None;
        }
        let pos = i as usize;
        contains(&self.strip_cell_rect(pos), x, y).then_some(pos)
    }

    /// Teclas en la Galería: ←/↑ el anterior, →/↓ el siguiente, Inicio/Fin, Re Pág/Av Pág (lo que
    /// entra en la tira), Enter abre y Ctrl+Espacio alterna; Shift y Ctrl como en la lista.
    pub(super) fn on_key_gallery(&mut self, vk: u16, ctrl: bool, shift: bool) -> Option<Action> {
        use crate::keys::Key;
        use crate::keys::keys::*;
        let n = self.visible.len();
        if n == 0 {
            return None;
        }
        self.header_focus = None;
        self.focus_visible = true;
        let cur = self.focus.unwrap_or(0).min(n - 1);
        // Sin selección, la primera flecha selecciona el enfocado en vez de saltearlo.
        let step = usize::from(self.selected.iter().any(|&s| s));
        let s = self.strip_rect();
        let page = (((s.right - s.left) / CELL_STEP).floor() as usize).max(1);
        let target = match Key(vk) {
            VK_LEFT | VK_UP => cur.saturating_sub(step),
            VK_RIGHT | VK_DOWN => (cur + step).min(n - 1),
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
        self.move_focus_to(target, ctrl, shift);
        Some(Action::Redraw)
    }

    /// Rueda en la Galería: sobre la tira, el anterior (hacia arriba) o el siguiente.
    pub(super) fn gallery_wheel(&mut self, rows: f32) -> Action {
        let on_strip = self.mouse_at.is_some_and(|(x, y)| contains(&self.strip_rect(), x, y));
        if !on_strip || self.visible.is_empty() {
            self.wheel_acc = 0.0;
            return Action::None;
        }
        if self.wheel_acc * rows < 0.0 {
            self.wheel_acc = 0.0;
        }
        self.wheel_acc += rows;
        let steps = (self.wheel_acc / WHEEL_STEP).trunc();
        if steps == 0.0 {
            return Action::None;
        }
        self.wheel_acc -= steps * WHEEL_STEP;
        let n = self.visible.len() as i64;
        let cur = self.focus.unwrap_or(0) as i64;
        let target = (cur - steps as i64).clamp(0, n - 1) as usize;
        if target == cur as usize && self.selected.iter().any(|&s| s) {
            return Action::None;
        }
        self.focus_visible = true;
        self.move_focus_to(target, false, false);
        // Las celdas se corrieron: lo que está bajo el mouse es otra.
        self.hover = self.mouse_at.and_then(|(x, y)| self.strip_pos_at(x, y));
        Action::Redraw
    }

    /// Galería (diseño X42): la imagen grande del enfocado arriba (carpetas y lo que no tiene
    /// imagen, el ícono de 96) y la tira de miniaturas abajo. No escribe nombres: los detalles
    /// están en la columna de vista previa.
    pub fn draw_gallery(&mut self, p: &dyn Painter, area: Rect, active: bool) {
        self.area = area;
        let th = p.theme();
        let strip = self.strip_rect();

        let img = self.gallery_image_rect();
        if let Some(idx) = self.focused_index()
            && img.right - img.left >= 1.0
            && img.bottom - img.top >= 1.0
        {
            let e = &self.entries[idx];
            let (w, h) = (img.right - img.left, img.bottom - img.top);
            let drawn = !e.is_dir
                && self.thumb_source(e).is_some_and(|src| p.big_image(&(src, thumbs::big_side(p.thumb_px(w.max(h)))), img));
            if !drawn {
                let size = BIG_ICON.min(w).min(h);
                let (cx, cy) = ((img.left + img.right) / 2.0, (img.top + img.bottom) / 2.0);
                p.shell_icon(e.icon, e.overlay, cx - size / 2.0, cy - size / 2.0, size);
            }
        }

        p.fill(strip, th.surface);
        p.fill(rect(strip.left, strip.top, strip.right, strip.top + 1.0), th.border);
        let (first, last) = self.strip_range();
        let focus = self.focus_pos();
        p.clip(strip, &mut || {
            for pos in first..last {
                let selected = self.selected[self.visible[pos] as usize];
                let focused = active && self.focus_visible && focus == Some(pos);
                let state = match (selected, focused) {
                    (true, true) => CellState::SelectedFocus,
                    (true, false) => CellState::Selected,
                    (false, true) => CellState::Focus,
                    _ if self.hover == Some(pos) => CellState::Hover,
                    _ => CellState::Normal,
                };
                self.draw_strip_cell(p, self.strip_cell_rect(pos), pos, state);
            }
        });
    }

    /// Celda de la tira (diseño `list::FileList::draw_strip_cell`): fondo de 88 con radio 6 según
    /// el estado, la miniatura de 72 (o el ícono de 48) y, con casillas, la casilla arriba a la
    /// izquierda si está seleccionada o bajo el mouse.
    pub fn draw_strip_cell(&self, p: &dyn Painter, r: Rect, pos: usize, state: CellState) {
        let th = p.theme();
        let Some(&idx) = self.visible.get(pos) else { return };
        let idx = idx as usize;
        let e = &self.entries[idx];
        let ring = |p: &dyn Painter| p.round_stroke_w(grow(r, -0.75), 5.25, th.accent, 1.5);
        if self.drop_target == Some(idx) {
            p.round_fill(r, 6.0, th.selection);
            ring(p);
        } else {
            match state {
                CellState::Normal => {}
                CellState::Hover => p.round_fill(r, 6.0, th.hover),
                CellState::Selected => p.round_fill(r, 6.0, th.selection),
                CellState::SelectedFocus => {
                    p.round_fill(r, 6.0, th.selection);
                    ring(p);
                }
                CellState::Focus => {
                    p.round_fill(r, 6.0, th.focus_fill);
                    p.round_stroke_w(grow(r, -0.75), 5.25, alpha(th.accent, 0.8), 1.5);
                }
            }
        }
        let inset = (CELL - STRIP_THUMB) / 2.0;
        let thumb = rect(r.left + inset, r.top + inset, r.right - inset, r.bottom - inset);
        let drawn = !e.is_dir && self.thumb_source(e).is_some_and(|src| p.thumbnail(&(src, p.thumb_px(STRIP_THUMB)), thumb));
        if !drawn {
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            p.shell_icon(e.icon, e.overlay, cx - STRIP_ICON / 2.0, cy - STRIP_ICON / 2.0, STRIP_ICON);
        }
        let selected = self.selected[idx];
        if let Some(cb) = self.checkbox_rect_in(r, None)
            && (selected || self.hover == Some(pos))
        {
            draw_checkbox(p, cb, if selected { CheckState::Checked } else { CheckState::Empty });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::Entry;
    use crate::geom::rect;
    use crate::keys::keys::{VK_DOWN, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP};
    use crate::list::{Action, Column, FileList, View};
    use crate::paint::Painter;
    use crate::sys::testing::{RecordingPainter, install_fakes};
    use crate::theme::{DEFAULT_ACCENT, Theme};
    use crate::thumbs;

    fn entry(name: &str, kind: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            full: None,
            group: 0,
            pidl: None,
            display: name.into(),
            sort_key: name.to_lowercase(),
            is_dir: dir,
            size: 0,
            modified: 0,
            created: 0,
            attrs: 0,
            type_name: std::sync::Arc::from(kind),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    /// Lista vacía de 960×600 en la carpeta «/fotos».
    fn blank() -> FileList {
        FileList { area: rect(0.0, 0.0, 960.0, 600.0), folder: Some("/fotos".into()), ..FileList::default() }
    }

    /// Galería de 960×600 con `n` fotos (f00.jpg…).
    fn gallery(n: usize) -> FileList {
        let mut l = blank();
        l.append((0..n).map(|i| entry(&format!("f{i:02}.jpg"), "Imagen JPEG", false)).collect());
        l.set_view(View::Gallery);
        l
    }

    fn names(l: &FileList, positions: impl Iterator<Item = usize>) -> Vec<String> {
        positions.map(|p| l.entries[l.visible[p] as usize].display.clone()).collect()
    }

    fn center(r: crate::geom::Rect) -> (f32, f32) {
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    fn painter() -> RecordingPainter {
        install_fakes();
        RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)))
    }

    #[test]
    fn gallery_view_index_and_cycle() {
        assert_eq!(View::Gallery.index(), 6);
        assert_eq!(View::Gallery.id(), "galeria");
        assert_eq!(View::Gallery.name(), "Galería");
        assert_eq!(View::from_id("galeria"), Some(View::Gallery));
        assert_eq!(crate::list::VIEWS[6], (View::Gallery, "Galería", "galeria", 72.0));
        assert_eq!(
            crate::list::CYCLE,
            [View::Details, View::List, View::Content, View::Icons, View::LargeIcons, View::Thumbnails, View::Gallery]
        );
        assert!(!View::Gallery.is_grid());
        assert!(!View::Gallery.has_header());
    }

    /// 30 elementos en 960: el enfocado queda centrado; en los extremos, la tira se queda pegada
    /// al borde; si todo entra, el conjunto va centrado.
    #[test]
    fn strip_offset_centers_focus() {
        let left = |count: usize, focus: usize, pos: usize| GALLERY_PAD + pos as f32 * CELL_STEP - strip_offset(count, focus, 960.0);
        // Foco 15: su centro en el medio.
        assert_eq!(left(30, 15, 15) + CELL / 2.0, 480.0);
        // Foco 0: sin corrimiento.
        assert_eq!(strip_offset(30, 0, 960.0), 0.0);
        // Foco 29: la última celda pegada al borde derecho (con el margen).
        assert_eq!(left(30, 29, 29) + CELL, 960.0 - GALLERY_PAD);
        // 5 elementos: mismo margen a los dos lados, sea cual sea el foco.
        for focus in 0..5 {
            let (first, last) = (left(5, focus, 0), left(5, focus, 4) + CELL);
            assert_eq!(first, 960.0 - last, "foco {focus}");
        }
    }

    #[test]
    fn gallery_keys_move_one() {
        let mut l = gallery(10);
        l.select_index(l.visible[3] as usize);
        l.on_key(VK_RIGHT.0, false, false);
        assert_eq!(l.focus_pos(), Some(4));
        l.on_key(VK_DOWN.0, false, false);
        assert_eq!(l.focus_pos(), Some(5));
        l.on_key(VK_LEFT.0, false, false);
        assert_eq!(l.focus_pos(), Some(4));
        l.on_key(VK_UP.0, false, false);
        assert_eq!(l.focus_pos(), Some(3));
        assert_eq!(l.selected_positions(), [3]);
        // Shift+→ extiende la selección.
        l.on_key(VK_RIGHT.0, false, true);
        l.on_key(VK_RIGHT.0, false, true);
        assert_eq!(l.selected_positions(), [3, 4, 5]);
        // Enter abre el enfocado.
        assert_eq!(l.on_key(VK_RETURN.0, false, false), Some(Action::Open(l.visible[5] as usize)));
    }

    /// Agrupada por tipo con un grupo plegado: la tira tiene solo los visibles, en orden, sin
    /// lugar para encabezados; → no se detiene en los encabezados.
    #[test]
    fn gallery_skips_group_headers() {
        let mut l = blank();
        l.append(vec![
            entry("a.jpg", "Imagen JPEG", false),
            entry("b.png", "Imagen PNG", false),
            entry("c.jpg", "Imagen JPEG", false),
            entry("d.txt", "Documento de texto", false),
            entry("e.png", "Imagen PNG", false),
        ]);
        l.set_group(Some(Column::Type));
        l.toggle_group("Imagen PNG");
        l.set_view(View::Gallery);
        let shown = names(&l, 0..l.count());
        assert_eq!(shown.len(), 3);
        assert!(!shown.iter().any(|n| n.ends_with(".png")), "{shown:?}");
        // Celdas seguidas, sin huecos de encabezados.
        for pos in 1..l.count() {
            assert_eq!(l.item_rect(pos).left - l.item_rect(pos - 1).left, CELL_STEP);
        }
        l.select_index(l.visible[0] as usize);
        let mut seen = vec![shown[0].clone()];
        for _ in 1..l.count() {
            l.on_key(VK_RIGHT.0, false, false);
            let pos = l.focus_pos().expect("el foco nunca queda en un encabezado");
            seen.extend(names(&l, std::iter::once(pos)));
        }
        assert_eq!(seen, shown);
        // Pasado el último, se queda en el último.
        l.on_key(VK_RIGHT.0, false, false);
        assert_eq!(l.focus_pos(), Some(l.count() - 1));
    }

    #[test]
    fn gallery_filter_applies() {
        let mut l = blank();
        l.append(vec![entry("a.jpg", "Imagen JPEG", false), entry("b.png", "Imagen PNG", false), entry("c.jpg", "Imagen JPEG", false)]);
        l.set_view(View::Gallery);
        l.set_filter(Column::Type, vec!["Imagen JPEG".into()]);
        assert_eq!(names(&l, 0..l.count()), ["a.jpg", "c.jpg"]);
        // Las dos celdas a la vista, y la del PNG no existe.
        let (first, last) = l.visible_range();
        assert_eq!((first, last), (0, 2));
    }

    #[test]
    fn gallery_click_focuses_ctrl_adds() {
        let mut l = gallery(10);
        let (x, y) = center(l.item_rect(3));
        l.on_mouse_down(x, y, false, false);
        l.on_mouse_up();
        assert_eq!(l.focus_pos(), Some(3));
        assert_eq!(l.selected_positions(), [3]);
        // La tira se movió para centrar la 3: se busca la 5 donde está ahora.
        let (x, y) = center(l.item_rect(5));
        l.on_mouse_down(x, y, true, false);
        l.on_mouse_up();
        assert_eq!(l.selected_positions(), [3, 5]);
        assert_eq!(l.focus_pos(), Some(5));
        // Un clic en la imagen grande no cambia la selección.
        l.on_mouse_down(480.0, 100.0, false, false);
        l.on_mouse_up();
        assert_eq!(l.selected_positions(), [3, 5]);
    }

    #[test]
    fn gallery_double_click_opens_folder() {
        let mut l = blank();
        l.append(vec![entry("Carpeta", "Carpeta de archivos", true), entry("a.jpg", "Imagen JPEG", false)]);
        l.set_view(View::Gallery);
        let pos = (0..l.count()).find(|&p| l.entries[l.visible[p] as usize].is_dir).unwrap();
        let (x, y) = center(l.item_rect(pos));
        assert_eq!(l.on_double_click(x, y), Action::Open(l.visible[pos] as usize));
    }

    /// Con casillas: un clic en la casilla de una celda la suma a la selección.
    #[test]
    fn gallery_checkbox_in_strip() {
        let mut l = gallery(10);
        l.checkboxes = true;
        l.select_index(l.visible[2] as usize);
        let cell = l.item_rect(4);
        let (x, y) = (cell.left + 12.0 + 7.0, cell.top + 12.0 + 7.0);
        l.on_mouse_move(x, y);
        l.on_mouse_down(x, y, false, false);
        l.on_mouse_up();
        assert_eq!(l.selected_positions(), [2, 4]);
        // Dibujada: la de la celda bajo el mouse y las de las seleccionadas.
        let p = painter();
        l.draw(&p, l.area, true);
        let marks = p.paths().len();
        assert_eq!(marks, 2, "una tilde por cada seleccionada");
    }

    #[test]
    fn gallery_draws_big_image_of_focus() {
        let mut l = gallery(10);
        l.select_index(l.visible[2] as usize);
        let p = painter();
        l.draw(&p, l.area, true);
        let draws = p.big_draws();
        assert_eq!(draws.len(), 1);
        let (key, r) = &draws[0];
        assert_eq!(key.0, thumbs::Source::Path("/fotos/f02.jpg".into()));
        // Arriba de la tira, con 16 de margen.
        assert_eq!(*r, rect(16.0, 16.0, 944.0, 600.0 - STRIP_H - 16.0));
        assert_eq!(key.1, thumbs::big_side(p.thumb_px(928.0)));
        // Sin texto de archivos.
        assert!(p.texts().is_empty(), "{:?}", p.texts());
    }

    /// Una carpeta: el ícono de 96 centrado en vez de la imagen grande; en la tira, el de 48.
    #[test]
    fn gallery_folder_uses_icon() {
        let mut l = blank();
        l.append(vec![entry("Carpeta", "Carpeta de archivos", true)]);
        l.set_view(View::Gallery);
        l.select_index(0);
        let p = painter();
        l.draw(&p, l.area, true);
        assert!(p.big_draws().is_empty());
        let icons = p.icons();
        let image = rect(16.0, 16.0, 944.0, 600.0 - STRIP_H - 16.0);
        let (cx, cy) = center(image);
        assert!(icons.contains(&(cx - 48.0, cy - 48.0, 96.0)), "{icons:?}");
        let (sx, sy) = center(l.item_rect(0));
        assert!(icons.contains(&(sx - 24.0, sy - 24.0, 48.0)), "{icons:?}");
        // Un archivo cuya imagen grande no llega: también el ícono de 96.
        let mut l = gallery(1);
        l.select_index(0);
        let p = painter();
        l.draw(&p, l.area, true);
        assert_eq!(p.big_draws().len(), 1);
        assert!(p.icons().iter().any(|i| i.2 == 96.0));
    }

    /// La rueda sobre la tira pasa al anterior o al siguiente; sobre la imagen no hace nada.
    #[test]
    fn gallery_wheel_on_strip() {
        let mut l = gallery(10);
        l.select_index(l.visible[4] as usize);
        let (x, y) = center(l.item_rect(4));
        l.on_mouse_move(x, y);
        l.on_wheel(-3.0);
        assert_eq!(l.focus_pos(), Some(5));
        l.on_wheel(3.0);
        assert_eq!(l.focus_pos(), Some(4));
        l.on_mouse_move(480.0, 100.0);
        l.on_wheel(-3.0);
        assert_eq!(l.focus_pos(), Some(4));
    }
}

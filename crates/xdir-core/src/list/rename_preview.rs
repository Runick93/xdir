//! Vista previa de Renombrar varios en la lista (diseño `list::FileList::draw_rename_preview`,
//! X51): la columna Nombre muestra el nombre nuevo con lo que cambia sobre el acento al 30 %
//! (danger en un choque), el nombre viejo 12 a la derecha, tachado, en `text_dim`, y con un
//! choque el signo «!» y la fila en danger; los que no cambian dicen «sin cambios».

use super::*;
use crate::rename_many::Clash;

/// Separación entre el nombre nuevo y el viejo tachado.
const OLD_GAP: f32 = 12.0;
/// Radio del signo de choque.
const SIGN_R: f32 = 6.0;

/// (prefijo, sufijo) en común entre `a` y `b`, en bytes de `b` (sin partir caracteres).
fn common_ends(a: &str, b: &str) -> (usize, usize) {
    let pre: usize = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(_, y)| y.len_utf8()).sum();
    let (ra, rb) = (&a[pre.min(a.len())..], &b[pre..]);
    let suf: usize = ra.chars().rev().zip(rb.chars().rev()).take_while(|(x, y)| x == y).map(|(_, y)| y.len_utf8()).sum();
    (pre, suf)
}

impl FileList {
    /// Fija la vista previa (o la quita con `None`); queda ordenada por posición.
    pub fn set_rename_preview(&mut self, preview: Option<Vec<(usize, String, Option<Clash>)>>) {
        self.rename_preview = preview.map(|mut v| {
            v.sort_by_key(|(i, ..)| *i);
            v
        });
    }

    /// El nombre nuevo y el choque del elemento `idx` (de `entries`), si se renombra.
    pub(super) fn rename_of(&self, idx: usize) -> Option<(&str, Option<Clash>)> {
        let v = self.rename_preview.as_ref()?;
        let k = v.binary_search_by_key(&idx, |(i, ..)| *i).ok()?;
        Some((v[k].1.as_str(), v[k].2))
    }

    /// El cartel del choque de la fila bajo el mouse («Ya existe "x" en esta carpeta»).
    pub fn rename_clash_tip(&self) -> Option<String> {
        let idx = *self.visible.get(self.hover?)? as usize;
        let (new, clash) = self.rename_of(idx)?;
        clash.map(|c| c.tip(new))
    }

    /// `e` con el nombre nuevo (para las vistas que solo muestran una etiqueta).
    pub(super) fn relabeled(&self, e: &Entry, new: &str) -> Entry {
        Entry {
            name: OsString::from(new),
            full: None,
            group: e.group,
            pidl: None,
            display: new.to_string(),
            sort_key: new.to_lowercase(),
            is_dir: e.is_dir,
            size: e.size,
            modified: e.modified,
            created: e.created,
            attrs: e.attrs,
            type_name: e.type_name.clone(),
            icon: e.icon,
            overlay: e.overlay,
            overlay_checked: e.overlay_checked,
            packed: e.packed,
        }
    }

    /// El nombre de una fila de detalles con su nombre nuevo en `text`. Devuelve hasta dónde
    /// llega lo dibujado (como `name_ends`).
    pub(super) fn draw_rename_name(&self, p: &dyn Painter, e: &Entry, text: Rect, color: Color, new: &str, clash: Option<Clash>) -> f32 {
        let th = p.theme();
        let old = shown_name(e, self.show_extensions);
        let mid = (text.top + text.bottom) / 2.0;
        if new == e.display {
            p.text(FontId::Ui, old, text, color);
            let end = text.left + p.measure(FontId::Ui, old);
            let note = "sin cambios";
            let w = p.measure(FontId::Small, note);
            let x = (text.right - w - 4.0).max(end + OLD_GAP);
            p.text(FontId::Small, note, rect(x, text.top, text.right, text.bottom), th.text_dim);
            return end + 6.0;
        }
        let shown = display_name(new, e.is_dir, self.show_extensions);
        let (pre, suf) = common_ends(old, shown);
        let changed = &shown[pre..shown.len().saturating_sub(suf).max(pre)];
        let x0 = text.left + p.measure(FontId::Ui, &shown[..pre]);
        if !changed.is_empty() {
            let w = p.measure(FontId::Ui, changed);
            let mark = if clash.is_some() { th.danger } else { th.accent };
            let right = (x0 + w + 1.0).min(text.right);
            if right > x0 {
                p.round_fill(rect(x0 - 1.0, mid - 9.0, right, mid + 9.0), 3.0, alpha(mark, 0.30));
            }
        }
        let new_color = if clash.is_some() { th.danger } else { color };
        p.text(FontId::Ui, shown, text, new_color);
        let mut end = text.left + p.measure(FontId::Ui, shown);
        // El viejo, tachado.
        let ox = end + OLD_GAP;
        if ox < text.right {
            p.text(FontId::Ui, old, rect(ox, text.top, text.right, text.bottom), th.text_dim);
            let ow = p.measure(FontId::Ui, old).min(text.right - ox);
            p.fill(rect(ox, mid, ox + ow, mid + 1.0), th.text_dim);
            end = ox + ow;
        }
        if clash.is_some() && end + 8.0 + SIGN_R * 2.0 < text.right {
            let cx = end + 8.0 + SIGN_R;
            p.circle(cx, mid, SIGN_R, th.danger);
            p.text(FontId::UiCenter, "!", rect(cx - SIGN_R, mid - 8.0, cx + SIGN_R, mid + 8.0), crate::geom::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
            end = cx + SIGN_R;
        }
        end + 6.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_ends_marks_the_change() {
        assert_eq!(common_ends("IMG_3011.jpg", "Bariloche-3011.jpg"), (0, 8));
        assert_eq!(common_ends("foto.jpg", "2026 foto.jpg"), (0, 8));
        assert_eq!(common_ends("ab", "ab"), (2, 0));
        assert_eq!(common_ends("ñandú", "ñandúes"), (7, 0));
    }
}

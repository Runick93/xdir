//! Accesibilidad: el árbol que se les muestra a los lectores de pantalla, sin nada del sistema.
//!
//! ```text
//! ventana (la raíz, con los datos de la ventana que da el sistema)
//! ├─ Panel N                        (uno por cada panel a la vista)
//! │  ├─ barra de dirección          (texto con la ruta)
//! │  └─ lista de archivos
//! │     └─ elemento…                (nombre, tipo, tamaño y fecha; selección e invocación)
//! └─ barra de estado
//! ```
//!
//! La aplicación se ve a través de `A11yView` (solo lectura) y los cambios que piden los lectores
//! llegan como `A11yAction`. Cada sistema traduce esto a su API (UI Automation en Windows, ver
//! `uia.rs`); las coordenadas acá son DIPs del área de la ventana.

use crate::geom::{Rect, contains, rect};
use crate::ui;

/// Un elemento del árbol. Los paneles se identifican por su índice y los elementos de la lista
/// por su posición entre los que se ven (después de filtrar y ordenar).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum El {
    Root,
    Pane(usize),
    Address(usize),
    List(usize),
    Item(usize, usize),
    Status,
}

/// Hacia dónde se mueve un lector de pantalla por el árbol.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Parent,
    Next,
    Previous,
    FirstChild,
    LastChild,
}

/// Qué clase de control es un elemento (cada sistema lo traduce a su tipo de control).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Pane,
    Text,
    List,
    ListItem,
    StatusBar,
}

/// Un elemento de la lista visto desde afuera (lectores de pantalla).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemInfo {
    pub name: String,
    /// Nombre del tipo ("Documento de texto", "Carpeta de archivos"…).
    pub kind: String,
    /// Tamaño ya formateado; vacío en las carpetas.
    pub size: String,
    /// Fecha de modificación ya formateada; vacía si no se conoce.
    pub date: String,
    pub is_dir: bool,
    pub selected: bool,
    /// En DIPs, relativo a la ventana.
    pub rect: Rect,
    /// Fuera del área visible (scrolleado).
    pub offscreen: bool,
}

impl ItemInfo {
    /// Tipo, tamaño y fecha en una frase (el texto de ayuda que lee el lector).
    pub fn help(&self) -> String {
        let mut help = vec![self.kind.clone()];
        if !self.size.is_empty() {
            help.push(self.size.clone());
        }
        if !self.date.is_empty() {
            help.push(format!("modificado {}", self.date));
        }
        help.join(", ")
    }
}

/// Lo que se le cuenta a un lector de pantalla de un elemento.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub name: Option<String>,
    /// Frase extra (tipo, tamaño, fecha).
    pub help: Option<String>,
    pub role: Option<Role>,
    /// "carpeta", "archivo"…: cómo se llama el tipo de control cuando es más específico.
    pub kind: Option<&'static str>,
    /// Identificador estable del elemento ("panel1.lista"…).
    pub id: String,
    pub focusable: bool,
    pub offscreen: bool,
    /// Elemento de la lista seleccionado.
    pub selected: bool,
}

/// La aplicación vista por los lectores de pantalla. Los paneles van por índice; los elementos,
/// por posición entre los que se ven.
pub trait A11yView {
    /// Índices de los paneles a la vista.
    fn shown_panes(&self) -> Vec<usize>;
    /// Panel activo (el que tiene el teclado).
    fn active(&self) -> usize;
    /// Elementos a la vista en la lista del panel.
    fn item_count(&self, pane: usize) -> usize;
    /// Posición con foco en la lista del panel.
    fn focus_pos(&self, pane: usize) -> Option<usize>;
    fn item_info(&self, pane: usize, pos: usize) -> Option<ItemInfo>;
    /// Título de la ubicación de la pestaña activa del panel.
    fn pane_title(&self, pane: usize) -> String;
    /// (pestaña activa, cantidad de pestañas) del panel.
    fn tab_pos(&self, pane: usize) -> (usize, usize);
    /// Lo que muestra la barra de dirección (la ruta, o el título si no hay ruta).
    fn address_text(&self, pane: usize) -> String;
    /// Texto de la barra de estado.
    fn status(&self) -> String;
    /// (panel, barra de dirección, lista) del panel, en DIPs.
    fn pane_rects(&self, pane: usize) -> (Rect, Rect, Rect);
    /// Tamaño del área de la ventana en DIPs.
    fn client(&self) -> (f32, f32);
    /// Posiciones seleccionadas en la lista del panel, en orden.
    fn selected_positions(&self, pane: usize) -> Vec<usize>;
    /// Posición del elemento de la lista del panel bajo un punto (DIPs).
    fn item_at(&self, pane: usize, x: f32, y: f32) -> Option<usize>;
}

/// Lo que un lector de pantalla le pide a la aplicación.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum A11yAction {
    /// Llevar el foco a un elemento (activa su panel; en un elemento de la lista, lo selecciona).
    Focus(El),
    /// Seleccionar solo ese elemento (panel, posición).
    Select(usize, usize),
    AddToSelection(usize, usize),
    RemoveFromSelection(usize, usize),
    /// Abrir el elemento.
    Invoke(usize, usize),
}

// ---------- el árbol ----------

pub fn exists(v: &dyn A11yView, el: El) -> bool {
    let shown = v.shown_panes();
    match el {
        El::Root | El::Status => true,
        El::Pane(p) | El::Address(p) | El::List(p) => shown.contains(&p),
        El::Item(p, pos) => shown.contains(&p) && pos < v.item_count(p),
    }
}

pub fn parent(el: El) -> Option<El> {
    match el {
        El::Root => None,
        El::Pane(_) | El::Status => Some(El::Root),
        El::Address(p) | El::List(p) => Some(El::Pane(p)),
        El::Item(p, _) => Some(El::List(p)),
    }
}

/// El panel al que pertenece un elemento (la raíz y la barra de estado no son de ninguno).
pub fn pane_of(el: El) -> Option<usize> {
    match el {
        El::Pane(p) | El::Address(p) | El::List(p) | El::Item(p, _) => Some(p),
        _ => None,
    }
}

/// Hijos de un elemento que no sea la lista (esa tiene tantos como elementos).
pub fn kids(v: &dyn A11yView, el: El) -> Vec<El> {
    match el {
        El::Root => v.shown_panes().into_iter().map(El::Pane).chain([El::Status]).collect(),
        El::Pane(p) => vec![El::Address(p), El::List(p)],
        _ => Vec::new(),
    }
}

pub fn navigate(v: &dyn A11yView, el: El, dir: Nav) -> Option<El> {
    if dir == Nav::Parent {
        return parent(el);
    }
    if dir == Nav::FirstChild || dir == Nav::LastChild {
        let first = dir == Nav::FirstChild;
        return match el {
            El::List(p) => {
                let n = v.item_count(p);
                (n > 0).then(|| El::Item(p, if first { 0 } else { n - 1 }))
            }
            _ => {
                let k = kids(v, el);
                if first { k.first().copied() } else { k.last().copied() }
            }
        };
    }
    let next = dir == Nav::Next;
    if let El::Item(p, pos) = el {
        let n = v.item_count(p);
        return if next { (pos + 1 < n).then_some(El::Item(p, pos + 1)) } else { pos.checked_sub(1).map(|q| El::Item(p, q)) };
    }
    let siblings = kids(v, parent(el)?);
    let i = siblings.iter().position(|&s| s == el)?;
    if next { siblings.get(i + 1).copied() } else { i.checked_sub(1).and_then(|j| siblings.get(j).copied()) }
}

/// Elemento con el foco del teclado: el de la lista del panel activo.
pub fn focus_el(v: &dyn A11yView) -> El {
    let p = v.active();
    match v.focus_pos(p) {
        Some(pos) if pos < v.item_count(p) => El::Item(p, pos),
        _ => El::List(p),
    }
}

// ---------- datos de cada elemento ----------

fn pane_name(v: &dyn A11yView, p: usize) -> String {
    let title = v.pane_title(p);
    let (active, tabs) = v.tab_pos(p);
    if tabs > 1 {
        format!("Panel {}, pestaña {} de {}: {}", p + 1, active + 1, tabs, title)
    } else {
        format!("Panel {}: {}", p + 1, title)
    }
}

pub fn info(v: &dyn A11yView, el: El) -> Info {
    let mut i = Info { name: None, help: None, role: None, kind: None, id: String::new(), focusable: false, offscreen: false, selected: false };
    match el {
        El::Root => {}
        El::Pane(p) => {
            i.name = Some(pane_name(v, p));
            i.role = Some(Role::Pane);
            i.id = format!("panel{}", p + 1);
        }
        El::Address(p) => {
            i.name = Some(format!("Ruta: {}", v.address_text(p)));
            i.role = Some(Role::Text);
            i.id = format!("panel{}.ruta", p + 1);
        }
        El::List(p) => {
            i.name = Some(format!("Archivos de {}, {} elementos", v.pane_title(p), v.item_count(p)));
            i.role = Some(Role::List);
            i.id = format!("panel{}.lista", p + 1);
            i.focusable = true;
        }
        El::Item(p, pos) => {
            if let Some(it) = v.item_info(p, pos) {
                i.help = Some(it.help());
                i.kind = Some(if it.is_dir { "carpeta" } else { "archivo" });
                i.offscreen = it.offscreen;
                i.selected = it.selected;
                i.name = Some(it.name);
            }
            i.role = Some(Role::ListItem);
            i.id = format!("panel{}.elemento{}", p + 1, pos);
            i.focusable = true;
        }
        El::Status => {
            i.name = Some(v.status());
            i.role = Some(Role::StatusBar);
            i.id = "estado".into();
        }
    }
    i
}

// ---------- geometría ----------

/// Rectángulo del elemento en DIPs del área de la ventana.
pub fn bounds(v: &dyn A11yView, el: El) -> Rect {
    let (w, h) = v.client();
    match el {
        El::Root => rect(0.0, 0.0, w, h),
        El::Pane(p) => v.pane_rects(p).0,
        El::Address(p) => v.pane_rects(p).1,
        El::List(p) => v.pane_rects(p).2,
        El::Item(p, pos) => v.item_info(p, pos).map(|i| i.rect).unwrap_or_default(),
        El::Status => rect(0.0, h - ui::STATUSBAR_H, w, h),
    }
}

/// El elemento propio bajo un punto (DIPs del área de la ventana); `None`: nada propio ahí.
pub fn element_at(v: &dyn A11yView, x: f32, y: f32) -> Option<El> {
    let (_, h) = v.client();
    if y >= h - ui::STATUSBAR_H {
        return Some(El::Status);
    }
    for p in v.shown_panes() {
        let (pane, address, list) = v.pane_rects(p);
        if !contains(&pane, x, y) {
            continue;
        }
        return Some(if contains(&address, x, y) {
            El::Address(p)
        } else if contains(&list, x, y) {
            match v.item_at(p, x, y) {
                Some(pos) => El::Item(p, pos),
                None => El::List(p),
            }
        } else {
            El::Pane(p)
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::FakeA11y;

    #[test]
    fn kids_of_root_are_shown_panes_and_status() {
        let mut v = FakeA11y::new(4, &["a.txt"]);
        v.shown = vec![0, 2];
        assert_eq!(kids(&v, El::Root), vec![El::Pane(0), El::Pane(2), El::Status]);
        assert_eq!(kids(&v, El::Pane(2)), vec![El::Address(2), El::List(2)]);
        assert!(exists(&v, El::List(2)));
        assert!(!exists(&v, El::List(1)), "un panel oculto no está en el árbol");
        assert_eq!(navigate(&v, El::Pane(2), Nav::Next), Some(El::Status));
        assert_eq!(navigate(&v, El::Root, Nav::FirstChild), Some(El::Pane(0)));
    }

    #[test]
    fn navigate_next_item_stops_at_end() {
        let v = FakeA11y::new(1, &["a", "b", "c"]);
        assert_eq!(navigate(&v, El::List(0), Nav::FirstChild), Some(El::Item(0, 0)));
        assert_eq!(navigate(&v, El::List(0), Nav::LastChild), Some(El::Item(0, 2)));
        assert_eq!(navigate(&v, El::Item(0, 1), Nav::Next), Some(El::Item(0, 2)));
        assert_eq!(navigate(&v, El::Item(0, 2), Nav::Next), None);
        assert_eq!(navigate(&v, El::Item(0, 0), Nav::Previous), None);
        assert_eq!(navigate(&v, El::Item(0, 2), Nav::Parent), Some(El::List(0)));
    }

    #[test]
    fn focus_el_is_list_when_empty() {
        let mut v = FakeA11y::new(2, &[]);
        v.active = 1;
        v.panes[1].focus = Some(0);
        assert_eq!(focus_el(&v), El::List(1), "sin elementos el foco queda en la lista");
        let mut v = FakeA11y::new(2, &["a", "b"]);
        v.active = 1;
        v.panes[1].focus = Some(1);
        assert_eq!(focus_el(&v), El::Item(1, 1));
    }

    #[test]
    fn info_of_item_reads_name_and_selection() {
        let mut v = FakeA11y::new(1, &["informe.txt", "fotos"]);
        v.panes[0].items[1].is_dir = true;
        v.panes[0].items[1].selected = true;
        let i = info(&v, El::Item(0, 1));
        assert_eq!(i.name.as_deref(), Some("fotos"));
        assert!(i.selected);
        assert_eq!(i.kind, Some("carpeta"));
        assert_eq!(i.role, Some(Role::ListItem));
        assert_eq!(i.id, "panel1.elemento1");
        let i = info(&v, El::Item(0, 0));
        assert_eq!(i.name.as_deref(), Some("informe.txt"));
        assert!(!i.selected);
        assert_eq!(i.kind, Some("archivo"));
        assert_eq!(i.help.as_deref(), Some("Documento, 1 KB, modificado 01/01/2026"));
        assert_eq!(info(&v, El::List(0)).name.as_deref(), Some("Archivos de Carpeta 1, 2 elementos"));
        assert_eq!(info(&v, El::Pane(0)).name.as_deref(), Some("Panel 1: Carpeta 1"));
    }

    #[test]
    fn bounds_after_filter_follow_visible_items() {
        let mut v = FakeA11y::new(1, &["uno", "dos", "tres"]);
        let before = bounds(&v, El::Item(0, 0));
        v.filter(0, "tre");
        assert_eq!(info(&v, El::Item(0, 0)).name.as_deref(), Some("tres"));
        assert!(!exists(&v, El::Item(0, 1)));
        // El que quedó ocupa la primera fila, no la tercera.
        assert_eq!(bounds(&v, El::Item(0, 0)), before);
        assert_eq!(element_at(&v, before.left + 1.0, before.top + 1.0), Some(El::Item(0, 0)));
    }
}

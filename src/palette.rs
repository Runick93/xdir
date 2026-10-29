//! Ventana flotante de búsqueda, arriba al centro: salto rápido a carpetas (Ctrl+P) y paleta
//! de comandos (Ctrl+Shift+P, que además de acciones busca favoritos y carpetas recientes).
//! El texto lo escribe un control de edición encima del campo; acá se arman los resultados,
//! se dibujan y se resuelven los clics.

use std::path::PathBuf;

use crate::commands::{self, Command, Keymap};
use crate::favorites::{Fav, Favorites};
use crate::fs::Location;
use crate::groups::Groups;
use crate::jump::{self, History};
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;

const WIDTH: f32 = 640.0;
const TOP: f32 = 44.0;
const INPUT_H: f32 = 46.0;
const ROW_H: f32 = 36.0;
const FOOTER_H: f32 = 28.0;
pub const MAX_ROWS: usize = 10;
const MAX_ITEMS: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteMode {
    /// Solo carpetas: historial y favoritos.
    Folders,
    /// Acciones, y después carpetas.
    All,
    /// Nombre para guardar los paneles como grupo (o un grupo existente para reemplazar).
    GroupName,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Command(Command),
    Folder(Location),
    /// Abrir el grupo guardado N.
    Group(usize),
    /// Guardar los paneles actuales con ese nombre.
    SaveGroup(String),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub target: Target,
    pub title: String,
    pub detail: String,
    pub keys: String,
    glyph: &'static str,
}

/// De dónde salen los resultados.
pub struct Sources<'a> {
    pub history: &'a History,
    pub favorites: &'a Favorites,
    pub groups: &'a Groups,
    pub keymap: &'a Keymap,
}

pub struct Palette {
    pub mode: PaletteMode,
    pub items: Vec<Item>,
    pub selected: usize,
    /// Primer resultado a la vista.
    scroll: usize,
    pub hover: Option<usize>,
}

/// Minúsculas sin tildes, para que "pestana" encuentre "Pestaña".
pub fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// Favoritos sueltos (las carpetas de favoritos se recorren): (nombre, ubicación).
fn fav_links(items: &[Fav], out: &mut Vec<(String, Location)>) {
    for f in items {
        match f {
            Fav::Link { name, location } => out.push((name.clone(), location.clone())),
            Fav::Group { items, .. } => fav_links(items, out),
        }
    }
}

/// ¿Lo escrito es una ruta completa a una carpeta que existe? (`C:\x`, `\\servidor\x`, `%TEMP%`)
fn typed_path(query: &str, expand: impl Fn(&str) -> String) -> Option<PathBuf> {
    let q = query.trim().trim_matches('"');
    let looks_like_path = q.contains(":\\") || q.starts_with("\\\\") || q.starts_with('%') || q.ends_with(':');
    if !looks_like_path {
        return None;
    }
    let mut p = PathBuf::from(expand(q));
    if p.as_os_str().len() == 2 && p.to_string_lossy().ends_with(':') {
        p.push("\\");
    }
    p.is_dir().then(|| crate::fs::true_case(&p))
}

/// Grupos guardados que coinciden con lo escrito, para abrir (o, al guardar, reemplazar).
fn group_items(groups: &Groups, folded: &[String], replace: bool) -> Vec<Item> {
    groups
        .items
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            let name = fold(&g.name);
            folded.iter().all(|w| name.contains(w.as_str()))
        })
        .map(|(i, g)| {
            let shown = g.panes.iter().take(crate::ui::Layout::from_index(g.layout).map(|l| l.panes()).unwrap_or(4));
            let folders: Vec<String> = shown.filter_map(|p| p.tabs.get(p.active)).map(|l| l.title()).collect();
            Item {
                title: if replace { format!("Reemplazar «{}»", g.name) } else { g.name.clone() },
                detail: format!("Grupo · {}", folders.join(", ")),
                target: if replace { Target::SaveGroup(g.name.clone()) } else { Target::Group(i) },
                keys: String::new(),
                glyph: "\u{E8A9}",
            }
        })
        .collect()
}

/// Resultados para lo escrito.
pub fn search(mode: PaletteMode, query: &str, src: &Sources, expand: impl Fn(&str) -> String) -> Vec<Item> {
    let words = jump::query_words(query);
    let folded: Vec<String> = words.iter().map(|w| fold(w)).collect();
    let mut out = Vec::new();

    if mode == PaletteMode::GroupName {
        let name = query.trim();
        if !name.is_empty() && !src.groups.items.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
            out.push(Item {
                title: format!("Guardar como «{name}»"),
                detail: String::new(),
                target: Target::SaveGroup(name.to_string()),
                keys: String::new(),
                glyph: "\u{E74E}",
            });
        }
        out.extend(group_items(src.groups, &folded, true));
        return out;
    }

    if let Some(p) = typed_path(query, expand) {
        out.push(Item {
            title: format!("Ir a {}", p.display()),
            detail: String::new(),
            target: Target::Folder(Location::Path(p)),
            keys: String::new(),
            glyph: "\u{E8AD}",
        });
    }

    if mode == PaletteMode::All {
        let mut cmds: Vec<(f64, usize)> = commands::ALL
            .iter()
            .enumerate()
            // Abrir la paleta desde la paleta no hace nada; "ir a carpeta" solo sin buscar.
            .filter(|(_, s)| commands::available(s.command) && s.command != Command::Palette && (s.command != Command::JumpTo || folded.is_empty()))
            .filter_map(|(i, s)| {
                let name = fold(s.name);
                if !folded.iter().all(|w| name.contains(w.as_str())) {
                    return None;
                }
                // Primero los que empiezan con lo escrito; después, el orden de la lista.
                let starts = folded.first().is_some_and(|w| name.starts_with(w.as_str()));
                Some((if starts { 2.0 } else { 1.0 }, i))
            })
            .collect();
        cmds.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, i) in cmds {
            let s = &commands::ALL[i];
            out.push(Item {
                target: Target::Command(s.command),
                title: s.name.to_string(),
                detail: String::new(),
                keys: src.keymap.label(s.command),
                glyph: "\u{E756}",
            });
        }
    }

    // Grupos guardados, antes que las carpetas sueltas.
    out.extend(group_items(src.groups, &folded, false));

    // Carpetas: historial por frecencia y favoritos (con un piso de puntaje, para que aparezcan
    // aunque no se hayan visitado), sin repetir.
    let (history, favs) = (src.history, src.favorites);
    let now = jump::now();
    let mut folders: Vec<(f64, Item)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut links = Vec::new();
    fav_links(&favs.items, &mut links);
    for (name, loc) in links {
        let key = match &loc {
            Location::Path(p) => p.to_string_lossy().to_lowercase(),
            other => other.title().to_lowercase(),
        };
        let haystack = fold(&format!("{} {}", name, key));
        let Some(mult) = jump::match_path(&folded, &haystack) else { continue };
        let visits = history.visits.iter().find(|v| v.path.to_string_lossy().to_lowercase() == key);
        let score = visits.map(|v| jump::frecency(v, now)).unwrap_or(0.0).max(3.0) * mult;
        let detail = match &loc {
            Location::Path(p) => format!("Favorito · {}", p.display()),
            _ => "Favorito".into(),
        };
        seen.push(key.trim_end_matches('\\').to_string());
        folders.push((score, Item { title: name, detail, target: Target::Folder(loc), keys: String::new(), glyph: "\u{E735}" }));
    }
    for v in &history.visits {
        let lower = v.path.to_string_lossy().to_lowercase();
        if seen.iter().any(|s| s == lower.trim_end_matches('\\')) {
            continue;
        }
        let Some(mult) = jump::match_path(&folded, &fold(&lower)) else { continue };
        let loc = Location::Path(v.path.clone());
        folders.push((
            jump::frecency(v, now) * mult,
            Item { title: loc.title(), detail: v.path.display().to_string(), target: Target::Folder(loc), keys: String::new(), glyph: "\u{E8B7}" },
        ));
    }
    if folded.is_empty() || "este equipo".contains(folded.join(" ").as_str()) {
        folders.push((0.1, Item {
            title: "Este equipo".into(),
            detail: String::new(),
            target: Target::Folder(Location::Computer),
            keys: String::new(),
            glyph: "\u{E7F8}",
        }));
    }
    folders.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.extend(folders.into_iter().map(|(_, i)| i));
    out.truncate(MAX_ITEMS);
    out
}

impl Palette {
    pub fn new(mode: PaletteMode, items: Vec<Item>) -> Self {
        Self { mode, items, selected: 0, scroll: 0, hover: None }
    }

    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.selected = 0;
        self.scroll = 0;
        self.hover = None;
    }

    /// Mueve la selección (flechas, Re Pág / Av Pág), sin dar la vuelta.
    pub fn move_by(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, last) as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + MAX_ROWS {
            self.scroll = self.selected + 1 - MAX_ROWS;
        }
    }

    pub fn on_wheel(&mut self, rows: i32) {
        let max = self.items.len().saturating_sub(MAX_ROWS);
        self.scroll = (self.scroll as i32 - rows).clamp(0, max as i32) as usize;
    }

    fn rows_shown(&self) -> usize {
        self.items.len().clamp(1, MAX_ROWS)
    }

    /// Panel entero.
    pub fn rect(&self, w: f32) -> Rect {
        let width = WIDTH.min(w - 32.0);
        let left = (w - width) / 2.0;
        let h = INPUT_H + self.rows_shown() as f32 * ROW_H + FOOTER_H + 8.0;
        rect(left, TOP, left + width, TOP + h)
    }

    /// Dónde va el control de edición.
    pub fn input_rect(&self, w: f32) -> Rect {
        let r = self.rect(w);
        rect(r.left + 44.0, r.top + 13.0, r.right - 16.0, r.top + INPUT_H - 13.0)
    }

    fn row_rect(&self, w: f32, i: usize) -> Rect {
        let r = self.rect(w);
        let top = r.top + INPUT_H + 4.0 + i as f32 * ROW_H;
        rect(r.left + 6.0, top, r.right - 6.0, top + ROW_H)
    }

    /// `Some(None)`: dentro del panel pero no sobre un resultado.
    pub fn item_at(&self, w: f32, x: f32, y: f32) -> Option<Option<usize>> {
        if !contains(&self.rect(w), x, y) {
            return None;
        }
        let n = self.items.len().saturating_sub(self.scroll).min(MAX_ROWS);
        Some((0..n).find(|&i| contains(&self.row_rect(w, i), x, y)).map(|i| i + self.scroll))
    }

    pub fn draw(&self, p: &Painter, w: f32) {
        let th = p.theme;
        let panel = self.rect(w);
        p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 5.0), 12.0, alpha(th.background, 0.6));
        p.round_fill(panel, 10.0, th.surface_alt);
        p.round_stroke(panel, 10.0, th.accent);

        // Campo de búsqueda (el texto lo dibuja el control de edición).
        let field = rect(panel.left + 8.0, panel.top + 8.0, panel.right - 8.0, panel.top + INPUT_H - 6.0);
        p.round_fill(field, 6.0, th.surface_alt);
        p.round_stroke(field, 6.0, th.border);
        let glyph = match self.mode {
            PaletteMode::All => "{E756}",
            PaletteMode::Folders => "{E721}",
            PaletteMode::GroupName => "{E74E}",
        };
        p.text(&p.fonts.glyphs_small, glyph, rect(field.left + 6.0, field.top, field.left + 30.0, field.bottom), th.accent);

        if self.items.is_empty() {
            let r = self.row_rect(w, 0);
            let empty = if self.mode == PaletteMode::GroupName { "Escribí un nombre para el grupo" } else { "Sin resultados" };
            p.text(&p.fonts.ui, empty, rect(r.left + 40.0, r.top, r.right, r.bottom), th.text_dim);
        }
        for (i, item) in self.items.iter().enumerate().skip(self.scroll).take(MAX_ROWS) {
            let r = self.row_rect(w, i - self.scroll);
            if i == self.selected {
                p.round_fill(r, 6.0, alpha(th.accent, 0.22));
            } else if self.hover == Some(i) {
                p.round_fill(r, 6.0, th.hover);
            }
            p.text(&p.fonts.glyphs_small, item.glyph, rect(r.left + 8.0, r.top, r.left + 32.0, r.bottom), th.text_dim);
            let keys_w = if item.keys.is_empty() { 0.0 } else { 120.0 };
            let text_l = r.left + 40.0;
            let text_r = r.right - 10.0 - keys_w;
            if item.detail.is_empty() {
                p.text(&p.fonts.ui, &item.title, rect(text_l, r.top, text_r, r.bottom), th.text);
            } else {
                // Nombre y, más tenue, la ruta.
                let split = text_l + ((text_r - text_l) * 0.4).min(220.0);
                p.text(&p.fonts.ui, &item.title, rect(text_l, r.top, split - 8.0, r.bottom), th.text);
                p.text(&p.fonts.ui, &item.detail, rect(split, r.top, text_r, r.bottom), th.text_dim);
            }
            if !item.keys.is_empty() {
                p.text(&p.fonts.ui_right, &item.keys, rect(text_r, r.top, r.right - 10.0, r.bottom), th.text_dim);
            }
        }

        // Pie: cómo se usa, y barra que indica que hay más resultados.
        let foot = rect(panel.left + 16.0, panel.bottom - FOOTER_H - 2.0, panel.right - 16.0, panel.bottom - 2.0);
        let hint = match self.mode {
            PaletteMode::Folders => "↑↓ elegir · Enter abrir · Ctrl+Enter en pestaña nueva · Ctrl+Shift+P comandos",
            PaletteMode::All => "↑↓ elegir · Enter ejecutar · Ctrl+Enter carpetas en pestaña nueva · Esc cerrar",
            PaletteMode::GroupName => "Nombre del grupo nuevo, o elegí uno para reemplazarlo · Enter guardar · Esc cancelar",
        };
        p.text(&p.fonts.ui, hint, foot, th.text_dim);
        if self.items.len() > MAX_ROWS {
            let more = format!("{}–{} de {}", self.scroll + 1, (self.scroll + MAX_ROWS).min(self.items.len()), self.items.len());
            p.text(&p.fonts.ui_right, &more, foot, th.text_dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[Item]) -> Vec<String> {
        items.iter().map(|i| i.title.clone()).collect()
    }

    #[test]
    fn folds_accents() {
        assert_eq!(fold("Pestaña ÚLTIMA"), "pestana ultima");
    }

    #[test]
    fn folders_by_frecency_and_favorites() {
        let now = jump::now();
        let mut h = History::default();
        h.record(std::path::Path::new(r"C:\proyectos\viejo"), now - 40 * 86_400);
        for _ in 0..5 {
            h.record(std::path::Path::new(r"C:\proyectos\nuevo"), now);
        }
        let favs = Favorites { items: vec![Fav::Link { name: "Informes".into(), location: Location::Path(r"D:\informes".into()) }] };
        let (groups, keymap) = (Groups::default(), Keymap::default());
        let src = Sources { history: &h, favorites: &favs, groups: &groups, keymap: &keymap };
        let all = search(PaletteMode::Folders, "", &src, |s| s.to_string());
        // Un favorito nunca visitado queda por encima de una carpeta vista una vez hace mucho.
        assert_eq!(names(&all)[..3], ["nuevo", "Informes", "viejo"].map(String::from));
        let q = search(PaletteMode::Folders, "info", &src, |s| s.to_string());
        assert_eq!(names(&q), ["Informes"]);
        let q = search(PaletteMode::Folders, "proy vie", &src, |s| s.to_string());
        assert_eq!(names(&q), ["viejo"]);
    }

    #[test]
    fn commands_in_all_mode() {
        let (h, favs, groups) = (History::default(), Favorites { items: Vec::new() }, Groups::default());
        let mut keymap = Keymap::default();
        keymap.set(Command::NewTab, &[commands::parse_combo("Ctrl+K").unwrap()]);
        let src = Sources { history: &h, favorites: &favs, groups: &groups, keymap: &keymap };
        let q = search(PaletteMode::All, "pestana nueva", &src, |s| s.to_string());
        assert_eq!(q.first().map(|i| i.target.clone()), Some(Target::Command(Command::NewTab)));
        // El atajo que se muestra es el configurado.
        assert_eq!(q[0].keys, "Ctrl+K");
        assert!(search(PaletteMode::Folders, "pestana", &src, |s| s.to_string()).is_empty());
    }

    #[test]
    fn groups_open_and_save() {
        use crate::groups::Group;
        use crate::session::PaneState;
        let (h, favs, keymap) = (History::default(), Favorites { items: Vec::new() }, Keymap::default());
        let mut groups = Groups::default();
        groups.put(Group {
            name: "Trabajo".into(),
            layout: 1,
            split_x: 0.5,
            split_y: 0.5,
            temp: false,
            panes: vec![
                PaneState { tabs: vec![Location::Path(r"C:\dev".into())], active: 0 },
                PaneState { tabs: vec![Location::Computer], active: 0 },
                PaneState { tabs: vec![Location::Path(r"C:\oculto".into())], active: 0 },
            ],
        });
        let src = Sources { history: &h, favorites: &favs, groups: &groups, keymap: &keymap };
        let q = search(PaletteMode::Folders, "trab", &src, |s| s.to_string());
        assert_eq!(q[0].target, Target::Group(0));
        // El detalle muestra solo los paneles que se ven en ese layout (2 lado a lado).
        assert_eq!(q[0].detail, "Grupo · dev, Este equipo");
        let save = search(PaletteMode::GroupName, "Fotos", &src, |s| s.to_string());
        assert_eq!(save.iter().map(|i| i.target.clone()).collect::<Vec<_>>(), [Target::SaveGroup("Fotos".into())]);
        let replace = search(PaletteMode::GroupName, "", &src, |s| s.to_string());
        assert_eq!(names(&replace), ["Reemplazar «Trabajo»"]);
        // Con el nombre exacto de uno existente no se ofrece "guardar como": se reemplaza ese.
        let same = search(PaletteMode::GroupName, "trabajo", &src, |s| s.to_string());
        assert_eq!(same.iter().map(|i| i.target.clone()).collect::<Vec<_>>(), [Target::SaveGroup("Trabajo".into())]);
    }
}

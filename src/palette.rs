//! Ventana flotante de búsqueda, arriba al centro: salto rápido a carpetas (Ctrl+P) y paleta
//! de comandos (Ctrl+Shift+P, que además de acciones busca favoritos y carpetas recientes).
//! El texto lo escribe un control de edición encima del campo; acá se arman los resultados,
//! se dibujan y se resuelven los clics.

use std::path::PathBuf;

use crate::commands::{self, Command};
use crate::favorites::{Fav, Favorites};
use crate::fs::Location;
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Command(Command),
    Folder(Location),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub target: Target,
    pub title: String,
    pub detail: String,
    pub keys: &'static str,
    glyph: &'static str,
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

/// Resultados para lo escrito.
pub fn search(mode: PaletteMode, query: &str, history: &History, favs: &Favorites, expand: impl Fn(&str) -> String) -> Vec<Item> {
    let words = jump::query_words(query);
    let folded: Vec<String> = words.iter().map(|w| fold(w)).collect();
    let mut out = Vec::new();

    if let Some(p) = typed_path(query, expand) {
        out.push(Item {
            title: format!("Ir a {}", p.display()),
            detail: String::new(),
            target: Target::Folder(Location::Path(p)),
            keys: "",
            glyph: "\u{E8AD}",
        });
    }

    if mode == PaletteMode::All {
        let mut cmds: Vec<(f64, usize)> = commands::ALL
            .iter()
            .enumerate()
            .filter(|(_, s)| s.command != Command::JumpTo || folded.is_empty())
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
                keys: s.keys,
                glyph: "\u{E756}",
            });
        }
    }

    // Carpetas: historial por frecencia y favoritos (con un piso de puntaje, para que aparezcan
    // aunque no se hayan visitado), sin repetir.
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
        folders.push((score, Item { title: name, detail, target: Target::Folder(loc), keys: "", glyph: "\u{E735}" }));
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
            Item { title: loc.title(), detail: v.path.display().to_string(), target: Target::Folder(loc), keys: "", glyph: "\u{E8B7}" },
        ));
    }
    if folded.is_empty() || "este equipo".contains(folded.join(" ").as_str()) {
        folders.push((0.1, Item {
            title: "Este equipo".into(),
            detail: String::new(),
            target: Target::Folder(Location::Computer),
            keys: "",
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
        let glyph = if self.mode == PaletteMode::All { "\u{E756}" } else { "\u{E721}" };
        p.text(&p.fonts.glyphs_small, glyph, rect(field.left + 6.0, field.top, field.left + 30.0, field.bottom), th.accent);

        if self.items.is_empty() {
            let r = self.row_rect(w, 0);
            p.text(&p.fonts.ui, "Sin resultados", rect(r.left + 40.0, r.top, r.right, r.bottom), th.text_dim);
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
                p.text(&p.fonts.ui_right, item.keys, rect(text_r, r.top, r.right - 10.0, r.bottom), th.text_dim);
            }
        }

        // Pie: cómo se usa, y barra que indica que hay más resultados.
        let foot = rect(panel.left + 16.0, panel.bottom - FOOTER_H - 2.0, panel.right - 16.0, panel.bottom - 2.0);
        let hint = match self.mode {
            PaletteMode::Folders => "↑↓ elegir · Enter abrir · Ctrl+Enter en pestaña nueva · Ctrl+Shift+P comandos",
            PaletteMode::All => "↑↓ elegir · Enter ejecutar · Ctrl+Enter carpetas en pestaña nueva · Esc cerrar",
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
        let all = search(PaletteMode::Folders, "", &h, &favs, |s| s.to_string());
        // Un favorito nunca visitado queda por encima de una carpeta vista una vez hace mucho.
        assert_eq!(names(&all)[..3], ["nuevo", "Informes", "viejo"].map(String::from));
        let q = search(PaletteMode::Folders, "info", &h, &favs, |s| s.to_string());
        assert_eq!(names(&q), ["Informes"]);
        let q = search(PaletteMode::Folders, "proy vie", &h, &favs, |s| s.to_string());
        assert_eq!(names(&q), ["viejo"]);
    }

    #[test]
    fn commands_in_all_mode() {
        let h = History::default();
        let favs = Favorites { items: Vec::new() };
        let q = search(PaletteMode::All, "pestana nueva", &h, &favs, |s| s.to_string());
        assert_eq!(q.first().map(|i| i.target.clone()), Some(Target::Command(Command::NewTab)));
        assert!(search(PaletteMode::Folders, "pestana", &h, &favs, |s| s.to_string()).is_empty());
    }
}

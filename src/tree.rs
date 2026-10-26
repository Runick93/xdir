//! Árbol de carpetas lateral (F-3.5, experimental). Solo se compila con la feature `arbol`:
//! `scripts\build.cmd build --release --features arbol`.
//!
//! Panel a la izquierda de la ventana que navega el panel activo. Arranca con la carpeta del
//! usuario y las unidades; las subcarpetas se leen al expandir un nodo, en un hilo aparte (una
//! unidad de red lenta no congela la ventana), y el resultado vuelve con `WM_APP_TREE`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::fs::Location;
use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;

/// Terminó de leerse una carpeta del árbol.
pub const WM_APP_TREE: u32 = WM_APP + 20;

pub const WIDTH: f32 = 240.0;
const ROW_H: f32 = 24.0;
const INDENT: f32 = 16.0;
/// Margen entre el borde de arriba y la primera fila.
const TOP_PAD: f32 = 4.0;

struct Node {
    path: PathBuf,
    name: String,
    depth: usize,
    /// None: sin leer todavía. Some(vacío): no tiene subcarpetas.
    children: Option<Vec<usize>>,
    expanded: bool,
    loading: bool,
}

/// Qué tocó un clic.
pub enum TreeHit {
    /// Expandir o contraer el nodo.
    Toggle(usize),
    /// Abrir la carpeta en el panel activo.
    Open(Location),
    Nothing,
}

pub struct Tree {
    pub visible: bool,
    nodes: Vec<Node>,
    roots: Vec<usize>,
    scroll: f32,
    hover: Option<usize>,
    area: Rect,
    tx: Sender<(usize, Vec<(String, PathBuf)>)>,
    rx: Receiver<(usize, Vec<(String, PathBuf)>)>,
}

impl Tree {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let mut t = Self { visible: false, nodes: Vec::new(), roots: Vec::new(), scroll: 0.0, hover: None, area: rect(0.0, 0.0, 0.0, 0.0), tx, rx };
        if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            let name = home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            t.add_root(home, name);
        }
        let drives = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..26u32 {
            if drives & (1 << i) != 0 {
                let letter = (b'A' + i as u8) as char;
                t.add_root(PathBuf::from(format!("{letter}:\\")), format!("{letter}:"));
            }
        }
        t
    }

    fn add_root(&mut self, path: PathBuf, name: String) {
        self.nodes.push(Node { path, name, depth: 0, children: None, expanded: false, loading: false });
        self.roots.push(self.nodes.len() - 1);
    }

    /// Nodos a la vista, en orden (los hijos de los expandidos, recursivamente).
    fn rows(&self) -> Vec<usize> {
        fn walk(t: &Tree, ids: &[usize], out: &mut Vec<usize>) {
            for &id in ids {
                out.push(id);
                let n = &t.nodes[id];
                if n.expanded {
                    if let Some(c) = &n.children {
                        walk(t, c, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &self.roots, &mut out);
        out
    }

    /// Expande o contrae; la primera vez lee las subcarpetas en otro hilo.
    pub fn toggle(&mut self, hwnd: HWND, id: usize, show_hidden: bool) {
        let n = &mut self.nodes[id];
        n.expanded = !n.expanded;
        if n.expanded && n.children.is_none() && !n.loading {
            n.loading = true;
            let (tx, path) = (self.tx.clone(), n.path.clone());
            let hwnd = hwnd.0 as usize;
            std::thread::spawn(move || {
                let dirs = subfolders(&path, show_hidden);
                if tx.send((id, dirs)).is_ok() {
                    unsafe {
                        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_TREE, WPARAM(0), LPARAM(0));
                    }
                }
            });
        }
    }

    /// Aplica las lecturas terminadas.
    pub fn on_loaded(&mut self) {
        while let Ok((id, dirs)) = self.rx.try_recv() {
            let depth = self.nodes[id].depth + 1;
            let mut ids = Vec::with_capacity(dirs.len());
            for (name, path) in dirs {
                self.nodes.push(Node { path, name, depth, children: None, expanded: false, loading: false });
                ids.push(self.nodes.len() - 1);
            }
            let n = &mut self.nodes[id];
            n.children = Some(ids);
            n.loading = false;
        }
    }

    /// Nodo de la fila bajo `y` (las filas arrancan `TOP_PAD` debajo del borde).
    fn node_at(&self, y: f32) -> Option<usize> {
        let row = ((y - self.area.top - TOP_PAD + self.scroll) / ROW_H).floor();
        if row < 0.0 {
            return None;
        }
        self.rows().get(row as usize).copied()
    }

    pub fn hit(&self, x: f32, y: f32) -> TreeHit {
        if !contains(&self.area, x, y) {
            return TreeHit::Nothing;
        }
        let Some(id) = self.node_at(y) else { return TreeHit::Nothing };
        let n = &self.nodes[id];
        let chevron_right = self.area.left + 6.0 + n.depth as f32 * INDENT + 16.0;
        if x < chevron_right {
            TreeHit::Toggle(id)
        } else {
            TreeHit::Open(Location::Path(n.path.clone()))
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.visible && contains(&self.area, x, y)
    }

    pub fn on_mouse_move(&mut self, x: f32, y: f32) -> bool {
        let hover = if contains(&self.area, x, y) { self.node_at(y) } else { None };
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    pub fn on_wheel(&mut self, rows: f32) {
        let content = self.rows().len() as f32 * ROW_H;
        let max = (content - (self.area.bottom - self.area.top)).max(0.0);
        self.scroll = (self.scroll - rows * ROW_H).clamp(0.0, max);
    }

    /// `current`: carpeta del panel activo, resaltada si está a la vista.
    pub fn draw(&mut self, p: &Painter, area: Rect, current: Option<&Path>) {
        self.area = area;
        let th = p.theme;
        p.round_fill(area, 8.0, th.surface);
        p.round_stroke(area, 8.0, th.border);
        let rows = self.rows();
        let max = (rows.len() as f32 * ROW_H - (area.bottom - area.top)).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
        p.clip(rect(area.left + 1.0, area.top + 1.0, area.right - 1.0, area.bottom - 1.0), || {
            for (i, &id) in rows.iter().enumerate() {
                let y = area.top + TOP_PAD + i as f32 * ROW_H - self.scroll;
                if y + ROW_H < area.top || y > area.bottom {
                    continue;
                }
                let n = &self.nodes[id];
                let row = rect(area.left + 4.0, y, area.right - 4.0, y + ROW_H);
                let is_current = current.is_some_and(|c| crate::fs::same_path(c, &n.path));
                if is_current {
                    p.round_fill(row, 4.0, alpha(th.accent, 0.22));
                } else if self.hover == Some(id) {
                    p.round_fill(row, 4.0, th.hover);
                }
                let x = area.left + 6.0 + n.depth as f32 * INDENT;
                // Chevron: derecha (contraído), abajo (expandido); nada si no tiene subcarpetas.
                let has_children = n.children.as_ref().is_none_or(|c| !c.is_empty());
                if has_children {
                    let glyph = if n.loading { "\u{E712}" } else if n.expanded { "\u{E70D}" } else { "\u{E76C}" };
                    p.text(&p.fonts.glyphs_small, glyph, rect(x, y, x + 16.0, y + ROW_H), th.text_dim);
                }
                let icon = if n.depth == 0 && n.path.parent().is_none() { "\u{EDA2}" } else { "\u{E8B7}" };
                p.text(&p.fonts.glyphs_small, icon, rect(x + 16.0, y, x + 34.0, y + ROW_H), th.accent);
                p.text(&p.fonts.ui, &n.name, rect(x + 38.0, y, area.right - 8.0, y + ROW_H), th.text);
            }
        });
    }
}

/// Subcarpetas ordenadas por nombre (sin las ocultas, salvo que se muestren).
fn subfolders(path: &Path, show_hidden: bool) -> Vec<(String, PathBuf)> {
    use std::os::windows::fs::MetadataExt;
    let Ok(reader) = std::fs::read_dir(path) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = reader
        .flatten()
        .filter(|e| {
            e.metadata().is_ok_and(|m| {
                let a = m.file_attributes();
                m.is_dir() && (show_hidden || a & 0x2 == 0)
            })
        })
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    out.sort_by(|a, b| crate::format::natural_cmp(&a.0.to_lowercase(), &b.0.to_lowercase()));
    out
}

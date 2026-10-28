//! Historial de rutas escritas en la barra de dirección y su desplegable, como en el Explorador.
//!
//! El historial es el mismo del Explorador de Windows: `HKCU\Software\Microsoft\Windows\
//! CurrentVersion\Explorer\TypedPaths` (`url1`, `url2`, ... de la más reciente a la más antigua,
//! hasta 25). Así una ruta escrita en uno aparece en el otro. El desplegable lo dibuja xdir (el de
//! Windows no sigue el tema oscuro): une ese historial con las subcarpetas de lo que se escribe.

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegGetValueW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

use crate::render::{Painter, Rect, contains, rect};
use crate::theme::alpha;

const KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\TypedPaths");
const MAX: usize = 25;
const ROW_H: f32 = 26.0;
const MAX_ROWS: usize = 10;
/// Tope de sugerencias de subcarpetas, para no armar una lista enorme en carpetas gigantes.
const MAX_DIRS: usize = 100;

/// Rutas escritas, de la más reciente a la más antigua.
pub fn load() -> Vec<String> {
    (1..=MAX)
        .map_while(|i| {
            let name: Vec<u16> = format!("url{i}\0").encode_utf16().collect();
            read_string(PCWSTR(name.as_ptr()))
        })
        .collect()
}

fn read_string(name: PCWSTR) -> Option<String> {
    let mut size = 0u32;
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, KEY, name, RRF_RT_REG_SZ, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        );
        (status == ERROR_SUCCESS).then(|| {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..len])
        })
    }
}

/// Anota una ruta escrita: queda la primera y se quitan sus repetidas (sin distinguir mayúsculas).
pub fn push(path: &str) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    let mut list = load();
    list.retain(|p| !p.eq_ignore_ascii_case(path));
    list.insert(0, path.to_string());
    list.truncate(MAX);
    save(&list);
}

fn save(list: &[String]) {
    unsafe {
        let mut key = HKEY::default();
        if RegCreateKeyExW(HKEY_CURRENT_USER, KEY, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_READ | KEY_WRITE, None, &mut key, None)
            != ERROR_SUCCESS
        {
            return;
        }
        for i in 1..=MAX {
            let name: Vec<u16> = format!("url{i}\0").encode_utf16().collect();
            match list.get(i - 1) {
                Some(p) => {
                    let data: Vec<u8> = p.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
                    let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&data));
                }
                None => {
                    let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
                }
            }
        }
        let _ = RegCloseKey(key);
    }
}

/// Sugerencias para `query`: el historial que empieza igual y las subcarpetas de la carpeta escrita.
/// Con la consulta vacía, todo el historial.
fn suggestions(query: &str) -> Vec<String> {
    let history = load();
    if query.is_empty() {
        return history;
    }
    let lower = query.to_lowercase();
    let mut out: Vec<String> = history.into_iter().filter(|p| p.to_lowercase().starts_with(&lower)).collect();
    if let Some(cut) = query.rfind(['\\', '/']) {
        let (dir, leaf) = query.split_at(cut + 1);
        let leaf = leaf.to_lowercase();
        let mut dirs: Vec<String> = std::fs::read_dir(if dir.ends_with(':') { format!("{dir}\\") } else { dir.to_string() })
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.to_lowercase().starts_with(&leaf))
            .map(|n| format!("{dir}{n}"))
            .collect();
        dirs.sort_by_key(|d| d.to_lowercase());
        dirs.truncate(MAX_DIRS);
        for d in dirs {
            if !out.iter().any(|p| p.eq_ignore_ascii_case(&d)) {
                out.push(d);
            }
        }
    }
    out
}

/// Desplegable bajo la barra de dirección: rutas del historial y subcarpetas de lo escrito.
pub struct Suggest {
    pub items: Vec<String>,
    /// Fila elegida con las flechas (el texto de la barra pasa a ser esa ruta).
    pub selected: Option<usize>,
    pub hover: Option<usize>,
    /// Lo que se había escrito antes de recorrer la lista con las flechas.
    typed: String,
    scroll: usize,
    anchor: Rect,
    /// Verdadero mientras xdir mismo cambia el texto (flechas): no hay que volver a buscar.
    pub quiet: bool,
}

impl Suggest {
    /// `anchor` es la barra de dirección; el panel cuelga de ella. Con `query` vacío, todo el historial.
    pub fn new(anchor: Rect, query: &str) -> Self {
        let mut s = Self { items: Vec::new(), selected: None, hover: None, typed: String::new(), scroll: 0, anchor, quiet: false };
        s.set_query(query);
        s
    }

    pub fn set_query(&mut self, query: &str) {
        self.items = suggestions(query);
        self.typed = query.to_string();
        self.selected = None;
        self.hover = None;
        self.scroll = 0;
    }

    /// Mueve la elección (flechas, Re Pág / Av Pág). Devuelve el texto que debe mostrar la barra:
    /// la ruta elegida, o lo que se había escrito al subir por encima de la primera.
    pub fn move_by(&mut self, delta: i32) -> Option<String> {
        if self.items.is_empty() {
            return None;
        }
        let last = self.items.len() as i32 - 1;
        let now = self.selected.map_or(-1, |i| i as i32);
        let next = (now + delta).clamp(-1, last);
        self.selected = (next >= 0).then_some(next as usize);
        if let Some(i) = self.selected {
            if i < self.scroll {
                self.scroll = i;
            } else if i >= self.scroll + MAX_ROWS {
                self.scroll = i + 1 - MAX_ROWS;
            }
        }
        Some(self.selected.map_or_else(|| self.typed.clone(), |i| self.items[i].clone()))
    }

    pub fn page(&self) -> i32 {
        MAX_ROWS as i32
    }

    pub fn on_wheel(&mut self, rows: i32) {
        let max = self.items.len().saturating_sub(MAX_ROWS);
        self.scroll = (self.scroll as i32 - rows).clamp(0, max as i32) as usize;
    }

    fn rows_shown(&self) -> usize {
        self.items.len().min(MAX_ROWS)
    }

    fn rect(&self) -> Rect {
        let top = self.anchor.bottom + 2.0;
        rect(self.anchor.left, top, self.anchor.right, top + self.rows_shown() as f32 * ROW_H + 8.0)
    }

    fn row_rect(&self, i: usize) -> Rect {
        let r = self.rect();
        let top = r.top + 4.0 + i as f32 * ROW_H;
        rect(r.left + 4.0, top, r.right - 4.0, top + ROW_H)
    }

    /// `Some(None)`: dentro del panel pero no sobre una fila. `None`: fuera (o sin panel).
    pub fn item_at(&self, x: f32, y: f32) -> Option<Option<usize>> {
        if self.items.is_empty() || !contains(&self.rect(), x, y) {
            return None;
        }
        let n = self.items.len().saturating_sub(self.scroll).min(MAX_ROWS);
        Some((0..n).find(|&i| contains(&self.row_rect(i), x, y)).map(|i| i + self.scroll))
    }

    pub fn draw(&self, p: &Painter) {
        if self.items.is_empty() {
            return;
        }
        let th = p.theme;
        let panel = self.rect();
        p.round_fill(rect(panel.left - 2.0, panel.top, panel.right + 2.0, panel.bottom + 4.0), 10.0, alpha(th.background, 0.6));
        p.round_fill(panel, 8.0, th.surface_alt);
        p.round_stroke(panel, 8.0, th.border);
        for (i, item) in self.items.iter().enumerate().skip(self.scroll).take(MAX_ROWS) {
            let r = self.row_rect(i - self.scroll);
            if self.selected == Some(i) {
                p.round_fill(r, 5.0, alpha(th.accent, 0.22));
            } else if self.hover == Some(i) {
                p.round_fill(r, 5.0, th.hover);
            }
            p.text(&p.fonts.ui, item, rect(r.left + 8.0, r.top, r.right - 8.0, r.bottom), th.text);
        }
    }
}


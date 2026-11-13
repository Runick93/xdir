//! Historial de crecimiento en la vista de xtree (#58): guardar la foto de cada análisis en un hilo,
//! traer el historial de la raíz y lo que muestra la tarjeta (cambio en el período, gráfico, las
//! subcarpetas que más crecieron) y la flecha ▲ de la tabla.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::history::{self, Loaded};
use super::scan::WM_APP_XTREE;
use super::colors::fmt_size;
use super::tree::{NO_PARENT, Node};
use crate::geom::{Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::sys::Handle;

/// Períodos del selector, en días.
pub const PERIODS: [u16; 3] = [7, 30, 90];

/// Historial que deja el hilo: la raíz de la que es y lo leído.
type Delivered = (PathBuf, Arc<Loaded>);

/// Lo que la vista sabe del historial de su raíz.
pub struct HistState {
    /// `[opciones] xtree_historial`: guardar una foto con cada análisis completo.
    pub on: bool,
    /// Período elegido (7, 30 o 90 días); se recuerda en la sesión.
    pub days: u16,
    /// Carpeta del historial (`<configuración>/xtree-historial`); sin ella no se guarda ni se lee.
    pub dir: Option<PathBuf>,
    /// Día de hoy fijo (pruebas y gancho de pruebas); si no, el del sistema en hora local.
    pub today_fix: Option<i32>,
    /// El historial de la raíz analizada, cuando llegó del hilo.
    pub loaded: Option<Arc<Loaded>>,
    /// Lo que deja el hilo: la raíz de la que es y su historial.
    slot: Arc<Mutex<Option<Delivered>>>,
    /// Hay un hilo guardando o leyendo.
    pub pending: bool,
    /// Contador de «Borrar historial» (el de la App); el hilo no guarda si cambió.
    pub clears: history::Clears,
    /// Parte de la franja bajo el mouse.
    pub hover: Option<StripHit>,
    /// Las que más crecieron de la última carpeta pedida: (clave, lista).
    growers_cache: Option<(GrowersKey, Vec<(u32, Change)>)>,
}

/// De qué son las subcarpetas guardadas en `growers_cache`.
type GrowersKey = (usize, usize, u32, u16, i32, usize);

impl Default for HistState {
    fn default() -> Self {
        HistState { on: true, days: PERIODS[0], dir: None, today_fix: None, loaded: None, slot: Default::default(), pending: false, clears: Default::default(), hover: None, growers_cache: None }
    }
}

impl HistState {
    /// Día de hoy (días desde 1970 en hora local).
    pub fn today(&self) -> i32 {
        self.today_fix.unwrap_or_else(|| crate::sys::env().local_now().day as i32)
    }

    /// Terminó un análisis completo de `root` con el árbol `arena`: en un hilo guarda la foto (si
    /// está encendido) y lee el historial de la raíz; al terminar avisa a la ventana como el
    /// escáner (`WM_APP_XTREE` con `id`, `lparam` 0). Los discos de red no tienen historial.
    /// `complete` en falso (la raíz no se pudo leer): no guarda foto, solo lee.
    pub fn after_scan(&mut self, hwnd: Handle, id: usize, root: &Path, arena: &Arc<Vec<Node>>, complete: bool) {
        self.after_scan_at(hwnd, id, root, arena, complete, None);
    }

    /// `after_scan` con la generación de borrado con que sale el hilo (`None`: la de ahora; las
    /// pruebas pasan una vieja). Si se borró el historial desde entonces, el hilo no guarda ni lee
    /// y entrega uno vacío.
    pub fn after_scan_at(&mut self, hwnd: Handle, id: usize, root: &Path, arena: &Arc<Vec<Node>>, complete: bool, generation: Option<u64>) {
        self.loaded = None;
        self.pending = false;
        let Some(dir) = self.dir.clone() else { return };
        if crate::sys::network().is_remote(root) {
            return;
        }
        // La foto se arma en el hilo (recorre medio árbol): se le pasa el árbol compartido.
        let arena = (self.on && complete).then(|| arena.clone());
        let (root, today, slot) = (root.to_path_buf(), self.today(), self.slot.clone());
        let clears = self.clears.clone();
        let generation = generation.unwrap_or_else(|| clears.load(std::sync::atomic::Ordering::SeqCst));
        self.pending = true;
        let _ = std::thread::Builder::new().spawn(move || {
            let photo = arena.map(|a| history::select_photo(&a)).unwrap_or_default();
            let done = history::if_current(&clears, generation, || {
                if !photo.sizes.is_empty() {
                    let _ = history::save_photo(&dir, &root, today, &photo);
                }
                history::load_for(&dir, &root)
            });
            let loaded = Arc::new(done.unwrap_or_default());
            if let Ok(mut s) = slot.lock() {
                *s = Some((root, loaded));
            }
            hwnd.post(WM_APP_XTREE, id, 0);
        });
    }

    /// Toma el historial que dejó el hilo, si es de `root`. Devuelve si llegó.
    pub fn take(&mut self, root: &Path) -> bool {
        let Some((from, loaded)) = self.slot.lock().ok().and_then(|mut s| s.take()) else { return false };
        if from != root {
            return false;
        }
        self.loaded = Some(loaded);
        self.pending = false;
        true
    }

    /// Se analiza otra cosa (o se borró el historial): lo que había ya no vale.
    pub fn reset(&mut self) {
        self.loaded = None;
        self.pending = false;
        self.hover = None;
        self.growers_cache = None;
    }

    /// ¿La raíz `root` tiene franja de historial? (hay carpeta del historial y no es de red).
    pub fn visible(&self, root: &Path) -> bool {
        self.dir.is_some() && !crate::sys::network().is_remote(root)
    }

    /// Las 5 subcarpetas de `id` que más crecieron en el período (se guardan hasta que cambie el
    /// árbol, la carpeta, el período o el historial).
    pub fn top_growers(&mut self, arena: &[Node], id: u32, today: i32) -> &[(u32, Change)] {
        let Some(l) = self.loaded.clone() else { return &[] };
        let key = (arena.as_ptr() as usize, arena.len(), id, self.days, today, Arc::as_ptr(&l) as usize);
        if self.growers_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            self.growers_cache = Some((key, growers(&l, arena, id, today, self.days, GROWERS)));
        }
        self.growers_cache.as_ref().map_or(&[], |(_, g)| g.as_slice())
    }
}

// ---------- lo que muestra la tarjeta ----------

/// Ruta de un nodo relativa a la raíz del árbol, con los tramos unidos con `/` (`""` es la raíz).
pub fn rel_path(arena: &[Node], mut id: u32) -> String {
    let mut parts = Vec::new();
    while let Some(n) = arena.get(id as usize) {
        if n.parent == NO_PARENT {
            break;
        }
        parts.push(&*n.name);
        id = n.parent;
    }
    parts.reverse();
    parts.join("/")
}

/// Cambio de una carpeta en el período: la foto con que se compara (su índice, día y tamaño) y el
/// tamaño de ahora.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub snap: usize,
    pub base_day: i32,
    pub base: u64,
    pub now: u64,
}

impl Change {
    pub fn delta(&self) -> i128 {
        self.now as i128 - self.base as i128
    }
}

/// Compara `now` con la foto anterior a hoy más cercana al inicio del período (`today - period`)
/// que sepa el tamaño de `rel`; a igual distancia, la más vieja. `None` si no hay ninguna.
pub fn change(l: &Loaded, rel: &str, now: u64, today: i32, period: u16) -> Option<Change> {
    let start = today - period as i32;
    let mut best: Option<Change> = None;
    for (i, &day) in l.days.iter().enumerate() {
        if day >= today {
            break;
        }
        let Some(base) = l.value(i, rel) else { continue };
        if best.is_none_or(|b| (day - start).abs() < (b.base_day - start).abs()) {
            best = Some(Change { snap: i, base_day: day, base, now });
        }
    }
    best
}

/// Tamaño con signo y un decimal: «+14,2 GB», «−1,1 GB», «+512 B».
pub fn fmt_delta(d: i128) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let sign = if d < 0 { "\u{2212}" } else { "+" };
    let n = d.unsigned_abs();
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{sign}{n} B") } else { format!("{sign}{v:.1} {}", UNITS[u]).replace('.', ",") }
}

/// «en 7 días», «en 1 día».
fn in_days(days: i32) -> String {
    if days == 1 { "en 1 día".to_string() } else { format!("en {days} días") }
}

/// Texto del cambio: «+14,2 GB en 7 días», «−1,1 GB en 30 días» o «Sin cambios en 7 días»; los
/// días son los que pasaron desde la foto con que se compara.
pub fn change_text(c: &Change, today: i32) -> String {
    let days = in_days(today - c.base_day);
    match c.delta() {
        0 => format!("Sin cambios {days}"),
        d => format!("{} {days}", fmt_delta(d)),
    }
}

/// Fecha corta de un día (días desde 1970): «10/11».
pub fn day_text(day: i32) -> String {
    let (_, m, d) = crate::format::civil_from_days(day as i64);
    format!("{d:02}/{m:02}")
}

/// Puntos del gráfico: (día, tamaño) de cada foto desde la de comparación que sepa el tamaño de
/// `rel`, y hoy con el tamaño de ahora (reemplaza a la foto de hoy, si la hay).
pub fn points(l: &Loaded, rel: &str, c: &Change, today: i32) -> Vec<(i32, u64)> {
    let mut out: Vec<(i32, u64)> = l.days.iter().enumerate().skip(c.snap).filter(|&(_, &d)| d < today).filter_map(|(i, &d)| l.value(i, rel).map(|v| (d, v))).collect();
    out.push((today, c.now));
    out
}

/// Las subcarpetas de `id` que más crecieron en el período, de más a menos (hasta `n`; solo las
/// que crecieron): nodo y cambio.
pub fn growers(l: &Loaded, arena: &[Node], id: u32, today: i32, period: u16, n: usize) -> Vec<(u32, Change)> {
    let Some(parent) = arena.get(id as usize) else { return Vec::new() };
    let base = rel_path(arena, id);
    let mut out: Vec<(u32, Change)> = parent
        .children
        .iter()
        .filter_map(|&k| {
            let node = arena.get(k as usize).filter(|c| c.is_dir && !c.small)?;
            let rel = if base.is_empty() { node.name.to_string() } else { format!("{base}/{}", node.name) };
            let c = change(l, &rel, node.size, today, period)?;
            (c.delta() > 0).then_some((k, c))
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.1.delta()));
    out.truncate(n);
    out
}

/// ¿Creció mucho? Al menos 100 MB y (al menos 1 GB o al menos el 25 % de lo que pesaba al
/// inicio del período): la flecha ▲ de la tabla.
pub fn grew_a_lot(c: &Change) -> bool {
    let d = c.delta();
    d >= history::BIG as i128 && (d >= 1024 * 1024 * 1024 || d * 4 >= c.base as i128)
}

/// Cartel de la ▲ de la fila del nodo `id`, si creció mucho en el período: «+3,4 GB en 7 días».
pub fn row_mark(h: &HistState, arena: &[Node], id: u32, today: i32) -> Option<String> {
    let n = arena.get(id as usize).filter(|n| n.is_dir && !n.small)?;
    let c = change(h.loaded.as_deref()?, &rel_path(arena, id), n.size, today, h.days)?;
    grew_a_lot(&c).then(|| change_text(&c, today))
}

/// Qué muestra la franja de historial de la tarjeta para un nodo.
#[derive(Clone, Debug, PartialEq)]
pub enum CardState {
    /// Leyendo el historial.
    Loading,
    /// Ninguna foto todavía (`on`: el historial está encendido).
    Empty { on: bool },
    /// Hay historial pero no de esta carpeta (más abajo del segundo nivel y chica).
    NotTracked,
    /// Un archivo: no tiene historial.
    File,
    /// Una sola foto (la de este día).
    FirstPhoto(i32),
    /// Con qué se compara.
    Data(Change),
}

/// El estado de la tarjeta para el nodo `id` del árbol.
pub fn card_state(h: &HistState, arena: &[Node], id: u32, today: i32) -> CardState {
    let Some(n) = arena.get(id as usize) else { return CardState::NotTracked };
    if !n.is_dir && n.parent != NO_PARENT {
        return CardState::File;
    }
    let Some(l) = &h.loaded else { return CardState::Loading };
    if l.is_empty() {
        return CardState::Empty { on: h.on };
    }
    let rel = rel_path(arena, id);
    if let Some(c) = change(l, &rel, n.size, today, h.days) {
        return CardState::Data(c);
    }
    match (0..l.days.len()).find(|&i| l.value(i, &rel).is_some()) {
        Some(i) => CardState::FirstPhoto(l.days[i]),
        None => CardState::NotTracked,
    }
}

// ---------- franja de historial de la tarjeta ----------

/// Alto de la tarjeta de siempre (nombre, ruta y botones).
pub const CARD_BASE_H: f32 = 76.0;
/// Alto que suma la franja de historial.
pub const STRIP_H: f32 = 128.0;
/// Con menos mapa que esto, la franja no se muestra.
pub const MIN_MAP_H: f32 = 180.0;
/// Subcarpetas en «Más crecieron».
pub const GROWERS: usize = 5;
const LEFT_W: f32 = 236.0;
const RIGHT_W: f32 = 300.0;
const PILL_W: f32 = 72.0;
const PILL_H: f32 = 26.0;
const GROWER_H: f32 = 19.0;
/// Columna de los tamaños a la izquierda del gráfico.
const AXIS_W: f32 = 70.0;

/// Una parte de la franja con la que se puede hacer clic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripHit {
    /// Un período del selector (índice en `PERIODS`).
    Period(usize),
    /// Una de «Más crecieron».
    Grower(usize),
}

/// Rectángulos de la franja dentro de la tarjeta agrandada `card`.
pub struct Strip {
    pub area: Rect,
    pub periods: [Rect; 3],
    pub left: Rect,
    pub chart: Rect,
    pub right: Rect,
}

pub fn strip(card: Rect) -> Strip {
    let area = rect(card.left, card.top + CARD_BASE_H, card.right, card.bottom);
    let left = rect(area.left + 18.0, area.top + 12.0, area.left + 18.0 + LEFT_W, area.bottom - 10.0);
    let mut periods = [rect(0.0, 0.0, 0.0, 0.0); 3];
    for (i, p) in periods.iter_mut().enumerate() {
        let x = left.left + i as f32 * (PILL_W + 4.0);
        *p = rect(x, left.top, x + PILL_W, left.top + PILL_H);
    }
    let right = rect(area.right - 18.0 - RIGHT_W, area.top + 10.0, area.right - 18.0, area.bottom - 8.0);
    let chart = rect(left.right + 24.0, area.top + 14.0, right.left - 28.0, area.bottom - 26.0);
    Strip { area, periods, left, chart, right }
}

/// Fila `i` de «Más crecieron».
fn grower_row(s: &Strip, i: usize) -> Rect {
    let top = s.right.top + 20.0 + i as f32 * GROWER_H;
    rect(s.right.left - 6.0, top, s.right.right + 6.0, top + GROWER_H)
}

/// Qué parte de la franja de `card` está en `(x, y)`, con `growers` filas en «Más crecieron».
pub fn strip_hit(card: Rect, x: f32, y: f32, growers: usize) -> Option<StripHit> {
    let s = strip(card);
    if let Some(i) = s.periods.iter().position(|r| contains(r, x, y)) {
        return Some(StripHit::Period(i));
    }
    (0..growers.min(GROWERS)).find(|&i| contains(&grower_row(&s, i), x, y)).map(StripHit::Grower)
}

/// Lo que necesita la franja para dibujarse.
pub struct StripView<'a> {
    pub state: CardState,
    pub scanning: bool,
    pub days: u16,
    pub today: i32,
    /// Para los puntos del gráfico: el historial y la ruta de la carpeta.
    pub loaded: Option<&'a Loaded>,
    pub rel: String,
    /// Las que más crecieron, con su nombre.
    pub growers: Vec<(String, Change)>,
    pub hover: Option<StripHit>,
}

/// Dibuja la franja de historial en la tarjeta agrandada `card`.
pub fn draw_strip(p: &dyn Painter, card: Rect, v: &StripView) {
    let th = p.theme();
    let s = strip(card);
    p.fill(rect(card.left + 14.0, s.area.top, card.right - 14.0, s.area.top + 1.0), th.border);
    let center = rect(s.area.left + 18.0, s.area.top + 8.0, s.area.right - 18.0, s.area.bottom - 8.0);
    let mid = (center.top + center.bottom) / 2.0;
    let line1 = rect(center.left, mid - 22.0, center.right, mid);
    let line2 = rect(center.left, mid, center.right, mid + 22.0);
    let message = |a: &str, b: &str| {
        p.text(FontId::UiCenter, a, if b.is_empty() { center } else { line1 }, th.text_dim);
        if !b.is_empty() {
            p.text(FontId::UiCenter, b, line2, th.text_dim);
        }
    };
    if v.scanning {
        return message("El historial se actualiza al terminar el análisis", "");
    }
    let c = match &v.state {
        CardState::Loading => return message("Cargando el historial\u{2026}", ""),
        CardState::Empty { on: true } => return message("Sin historial todavía: se arma cada vez que analizás", ""),
        CardState::Empty { on: false } => return message("El historial está apagado", "Se enciende en Configuración \u{25B8} xtree"),
        CardState::NotTracked => return message("Sin historial de esta carpeta", "Se guardan los dos primeros niveles y las carpetas de 100 MB o más"),
        CardState::File => return message("Los archivos no tienen historial", ""),
        CardState::FirstPhoto(day) => return message(&format!("Primera foto: {}", day_text(*day)), "El cambio se ve desde el próximo análisis"),
        CardState::Data(c) => *c,
    };

    // Selector de período.
    for (i, (r, d)) in s.periods.iter().zip(PERIODS).enumerate() {
        let on = d == v.days;
        let hot = v.hover == Some(StripHit::Period(i));
        p.round_fill(*r, 6.0, if on { th.accent } else if hot { th.hover } else { th.surface_alt });
        if !on {
            p.round_stroke_w(*r, 6.0, if hot { th.accent } else { th.border }, 1.0);
        }
        let tc = if on { crate::xtree::colors::text_on(th.accent) } else { th.text };
        p.text(FontId::UiCenter, &format!("{d} días"), *r, tc);
    }
    // Cambio en el período.
    let tc = match c.delta() {
        0 => th.text_dim,
        d if d > 0 => th.warning,
        _ => th.ok,
    };
    let top = s.periods[0].bottom;
    p.text(FontId::Title, &change_text(&c, v.today), rect(s.left.left, top + 8.0, s.left.right, top + 36.0), tc);
    let from = format!("Foto del {}: {}", day_text(c.base_day), fmt_size(c.base));
    p.text(FontId::Small, &from, rect(s.left.left, top + 38.0, s.left.right, top + 54.0), th.text_dim);
    p.text(FontId::Small, &format!("Ahora: {}", fmt_size(c.now)), rect(s.left.left, top + 56.0, s.left.right, top + 72.0), th.text_dim);

    // Gráfico de línea: un punto por foto (x según la fecha), de la de comparación a hoy.
    let pts = v.loaded.map(|l| points(l, &v.rel, &c, v.today)).unwrap_or_default();
    let ch = s.chart;
    if ch.right - ch.left > AXIS_W + 40.0 && pts.len() >= 2 {
        let plot = rect(ch.left + AXIS_W, ch.top, ch.right, ch.bottom);
        let (lo, hi) = pts.iter().fold((u64::MAX, 0), |(lo, hi), &(_, v)| (lo.min(v), hi.max(v)));
        let (d0, d1) = (pts[0].0, pts[pts.len() - 1].0);
        p.fill(rect(plot.left, plot.top, plot.right, plot.top + 1.0), th.border);
        p.fill(rect(plot.left, plot.bottom - 1.0, plot.right, plot.bottom), th.border);
        p.text(FontId::Small, &fmt_size(hi), rect(ch.left, plot.top - 7.0, plot.left - 6.0, plot.top + 9.0), th.text_dim);
        if hi != lo {
            p.text(FontId::Small, &fmt_size(lo), rect(ch.left, plot.bottom - 9.0, plot.left - 6.0, plot.bottom + 7.0), th.text_dim);
        }
        let xy = |(d, val): (i32, u64)| {
            let fx = if d1 > d0 { (d - d0) as f32 / (d1 - d0) as f32 } else { 1.0 };
            let fy = if hi > lo { (val - lo) as f32 / (hi - lo) as f32 } else { 0.5 };
            (plot.left + 4.0 + fx * (plot.right - plot.left - 8.0), plot.bottom - 4.0 - fy * (plot.bottom - plot.top - 8.0))
        };
        let path: Vec<(f32, f32, f32)> = pts.iter().map(|&pt| xy(pt)).map(|(x, y)| (x, y, 0.0)).collect();
        p.rounded_path(&path, false, None, Some((th.accent, 2.0)));
        for (i, &pt) in pts.iter().enumerate() {
            let (x, y) = xy(pt);
            p.circle(x, y, if i + 1 == pts.len() { 4.0 } else { 3.0 }, th.accent);
        }
        let (a, b) = (day_text(d0), day_text(d1));
        p.text(FontId::Small, &a, rect(plot.left, plot.bottom + 4.0, plot.right, plot.bottom + 20.0), th.text_dim);
        let bw = p.measure(FontId::Small, &b);
        p.text(FontId::Small, &b, rect(plot.right - bw - 2.0, plot.bottom + 4.0, plot.right + 2.0, plot.bottom + 20.0), th.text_dim);
    }

    // Las que más crecieron.
    p.text(FontId::Small, "Más crecieron", rect(s.right.left, s.right.top, s.right.right, s.right.top + 16.0), th.text_dim);
    if v.growers.is_empty() {
        p.text(FontId::Ui, "Ninguna subcarpeta creció", rect(s.right.left, s.right.top + 20.0, s.right.right, s.right.top + 20.0 + GROWER_H), th.text_dim);
    }
    for (i, (name, g)) in v.growers.iter().take(GROWERS).enumerate() {
        let r = grower_row(&s, i);
        if v.hover == Some(StripHit::Grower(i)) {
            p.round_fill(r, 4.0, th.hover);
        }
        let delta = fmt_delta(g.delta());
        let dw = p.measure(FontId::Ui, &delta);
        p.text(FontId::Ui, name, rect(s.right.left, r.top, s.right.right - dw - 12.0, r.bottom), th.text);
        p.text(FontId::UiRight, &delta, rect(s.right.right - dw - 8.0, r.top, s.right.right, r.bottom), th.warning);
    }
}

/// Dibuja la franja de historial del nodo `shown` de `arena` en la tarjeta agrandada `card`.
pub fn paint_strip(p: &dyn Painter, card: Rect, h: &mut HistState, arena: &[Node], shown: u32, scanning: bool) {
    let today = h.today();
    let state = card_state(h, arena, shown, today);
    let growers = match state {
        CardState::Data(_) if !scanning => h.top_growers(arena, shown, today).iter().map(|(k, c)| (arena[*k as usize].name.to_string(), *c)).collect(),
        _ => Vec::new(),
    };
    let view = StripView { state, scanning, days: h.days, today, loaded: h.loaded.as_deref(), rel: rel_path(arena, shown), growers, hover: h.hover };
    draw_strip(p, card, &view);
}

/// La ▲ junto al tamaño: `right` es el borde derecho del texto del tamaño, de `size_w` de ancho,
/// en la fila que va de `top` a `bottom`.
pub fn draw_mark(p: &dyn Painter, right: f32, size_w: f32, top: f32, bottom: f32) {
    let th = p.theme();
    let x1 = right - size_w - 6.0;
    let (x0, cy) = (x1 - 9.0, (top + bottom) / 2.0);
    p.rounded_path(&[(x0, cy + 4.0, 0.0), ((x0 + x1) / 2.0, cy - 4.0, 0.0), (x1, cy + 4.0, 0.0)], true, Some(th.warning), None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xtree::history;

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children }
    }

    /// Historial con fotos en `snaps` (día, [(ruta, tamaño)]), leído como lo lee la vista.
    fn loaded(name: &str, snaps: &[(i32, &[(&str, u64)])]) -> Loaded {
        let dir = std::env::temp_dir().join(format!("xdir-crecimiento-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("raiz");
        let mut h = history::History { root: root.to_string_lossy().into_owned(), ..Default::default() };
        for (day, sizes) in snaps {
            let v: Vec<(String, u64)> = sizes.iter().map(|(p, s)| (p.to_string(), *s)).collect();
            h.add(*day, &v);
        }
        history::write_file(&history::file_for(&dir, &root), &h).unwrap();
        let l = history::load_for(&dir, &root);
        let _ = std::fs::remove_dir_all(&dir);
        l
    }

    #[test]
    fn rel_path_joins_names() {
        let t = vec![node("/r", NO_PARENT, 1, true, vec![1]), node("a", 0, 1, true, vec![2]), node("b", 1, 1, true, vec![])];
        assert_eq!(rel_path(&t, 0), "");
        assert_eq!(rel_path(&t, 2), "a/b");
    }

    #[test]
    fn change_uses_photo_closest_to_period_start() {
        let l = loaded("cambio", &[(80, &[("", GB)]), (91, &[("", 2 * GB)]), (95, &[("", 3 * GB)]), (100, &[("", 9 * GB)])]);
        // Hoy 100, período 7: inicio 93; la de 91 y la de 95 están a 2 días: la más vieja.
        let c = change(&l, "", 10 * GB, 100, 7).unwrap();
        assert_eq!((c.base_day, c.base, c.now), (91, 2 * GB, 10 * GB));
        assert_eq!(change_text(&c, 100), "+8,0 GB en 9 días");
        // Período 30: inicio 70, la más cercana es la de 80. La de hoy nunca es la base.
        assert_eq!(change(&l, "", 10 * GB, 100, 30).unwrap().base_day, 80);
        assert_eq!(change(&l, "", 10 * GB, 80, 7), None, "sin fotos anteriores a hoy");
    }

    #[test]
    fn texts() {
        assert_eq!(fmt_delta(14 * GB as i128 + 200 * MB as i128), "+14,2 GB");
        assert_eq!(fmt_delta(-(GB as i128) - 100 * MB as i128), "\u{2212}1,1 GB");
        assert_eq!(fmt_delta(512), "+512 B");
        let c = Change { snap: 0, base_day: 93, base: 5, now: 5 };
        assert_eq!(change_text(&c, 100), "Sin cambios en 7 días");
        assert_eq!(change_text(&Change { base_day: 99, now: 4, ..c }, 100), "\u{2212}1 B en 1 día");
        // 2026-11-10 es el día 20.402.
        assert_eq!(day_text(20_402), "10/11");
    }

    #[test]
    fn points_from_base_to_today() {
        let l = loaded("puntos", &[(80, &[("", 1)]), (90, &[("", 2)]), (95, &[("x", 1)]), (99, &[("", 4)]), (100, &[("", 5)])]);
        let c = change(&l, "", 7, 100, 10).unwrap();
        assert_eq!(c.base_day, 90);
        // Una foto sin la raíz no sirve; la de hoy va con el tamaño de ahora.
        assert_eq!(points(&l, "", &c, 100), [(90, 2), (99, 4), (100, 7)]);
    }

    #[test]
    fn growers_top_children() {
        let snaps: &[(i32, &[(&str, u64)])] = &[(93, &[("", 10 * GB), ("a", GB), ("b", 5 * GB), ("c", 3 * GB), ("d", 2 * GB)])];
        let l = loaded("crecieron", snaps);
        let t = vec![
            node("/r", NO_PARENT, 30 * GB, true, vec![1, 2, 3, 4, 5, 6]),
            node("a", 0, 7 * GB, true, vec![]),
            node("b", 0, 4 * GB, true, vec![]),
            node("c", 0, 6 * GB, true, vec![]),
            node("d", 0, 2 * GB, true, vec![]),
            node("nueva", 0, GB, true, vec![]),
            node("f.bin", 0, 9 * GB, false, vec![]),
        ];
        let g: Vec<(&str, i128)> = growers(&l, &t, 0, 100, 7, 5).iter().map(|(k, c)| (&*t[*k as usize].name, c.delta())).collect();
        assert_eq!(g, [("a", 6 * GB as i128), ("c", 3 * GB as i128), ("nueva", GB as i128)]);
        assert_eq!(growers(&l, &t, 0, 100, 7, 1).len(), 1);
    }

    #[test]
    fn card_states() {
        let t = vec![node("/r", NO_PARENT, 30, true, vec![1, 2]), node("a", 0, 20, true, vec![3]), node("f", 0, 10, false, vec![]), node("hondo", 1, 5, true, vec![])];
        let mut h = HistState::default();
        assert_eq!(card_state(&h, &t, 0, 100), CardState::Loading);
        assert_eq!(card_state(&h, &t, 2, 100), CardState::File);
        h.loaded = Some(Arc::new(Loaded::default()));
        assert_eq!(card_state(&h, &t, 0, 100), CardState::Empty { on: true });
        h.on = false;
        assert_eq!(card_state(&h, &t, 0, 100), CardState::Empty { on: false });
        h.loaded = Some(Arc::new(loaded("estados1", &[(100, &[("", 30), ("a", 20)])])));
        assert_eq!(card_state(&h, &t, 0, 100), CardState::FirstPhoto(100));
        h.loaded = Some(Arc::new(loaded("estados2", &[(90, &[("", 25), ("a", 20)]), (100, &[("", 30), ("a", 20)])])));
        assert!(matches!(card_state(&h, &t, 1, 100), CardState::Data(c) if c.base == 20 && c.now == 20));
        // Tercer nivel y chica: la foto propia cubre dos niveles; «a/hondo» está en el segundo.
        assert!(matches!(card_state(&h, &t, 3, 100), CardState::Data(c) if c.base == 0));
    }

    #[test]
    fn grew_a_lot_rule() {
        let c = |base: u64, now: u64| Change { snap: 0, base_day: 0, base, now };
        assert!(grew_a_lot(&c(0, 100 * MB)), "de la nada a 100 MB");
        assert!(!grew_a_lot(&c(0, 99 * MB)), "menos de 100 MB nunca");
        assert!(!grew_a_lot(&c(100 * MB, 199 * MB)));
        assert!(grew_a_lot(&c(300 * MB, 400 * MB)), "33 %");
        assert!(!grew_a_lot(&c(10 * GB, 10 * GB + 900 * MB)), "9 % y menos de 1 GB");
        assert!(grew_a_lot(&c(10 * GB, 11 * GB)), "1 GB");
        assert!(grew_a_lot(&c(400 * MB, 500 * MB)), "justo el 25 %");
        assert!(!grew_a_lot(&c(5 * GB, 4 * GB)), "achicarse no");
    }

    #[test]
    fn row_mark_text() {
        let t = vec![node("/r", NO_PARENT, 30 * GB, true, vec![1, 2]), node("a", 0, 20 * GB, true, vec![]), node("b", 0, 10 * GB, true, vec![])];
        let mut h = HistState::default();
        assert_eq!(row_mark(&h, &t, 1, 100), None, "sin historial");
        h.loaded = Some(Arc::new(loaded("marca", &[(93, &[("", 25 * GB), ("a", 16 * GB), ("b", 10 * GB)])])));
        assert_eq!(row_mark(&h, &t, 1, 100).as_deref(), Some("+4,0 GB en 7 días"));
        assert_eq!(row_mark(&h, &t, 2, 100), None);
    }
}

//! El panel de comparar (en el lugar de la columna de vista previa, como el de información):
//! barra superior de 40 con «Comparar contenido» y ✕; los dos archivos (ícono de 32, nombre,
//! carpeta y tamaño) en bloques de 64; el resultado («Comparando…» con barra de avance de 4 y
//! Cancelar; «Iguales» en `ok` o «Distintos» en `danger` con el detalle; el error) y la sección
//! «Suma de verificación (SHA-256)» con «Calcular SHA-256» de cada uno, la suma en letra de ancho
//! fijo y «Copiar».
//!
//! Como el de información, `layout` arma sin pintar lo que se dibuja (con las primitivas de
//! `info::draw`) y dónde está cada cosa que se toca; `draw` lo pinta y `hit` lo consulta.

use crate::geom::{Rect, contains, rect};
use crate::info::draw::{self as idraw, Prim, Tone};
use crate::paint::{FontId, Painter};

pub const TITLE: &str = "Comparar contenido";
const MARGIN: f32 = 16.0;
const TOP_H: f32 = 40.0;
/// Bloque de cada archivo.
const SIDE_H: f32 = 64.0;
const ICON: f32 = 32.0;
const BUTTON_H: f32 = 26.0;
const CANCEL_W: f32 = 84.0;
/// Alto de una línea de la suma (letra de ancho fijo).
const HEX_LINE_H: f32 = 16.0;
pub const CANCEL_TEXT: &str = "Cancelar";
pub const RETRY_TEXT: &str = "Comparar de nuevo";
pub const HASH_TEXT: &str = "Calcular SHA-256";
pub const COPY_TEXT: &str = "Copiar";
pub const SUMS_TITLE: &str = "Suma de verificación (SHA-256)";

/// Uno de los dos archivos.
#[derive(Clone, Debug, PartialEq)]
pub struct SideView {
    pub name: String,
    pub folder: String,
    pub size: String,
    pub icon: i32,
}

/// Cómo va la comparación.
#[derive(Clone, Debug, PartialEq)]
pub enum ResultView {
    /// `label` («Comparando…», «Extrayendo del comprimido…»), la parte hecha y el detalle.
    Running { label: String, fraction: f32, detail: String },
    /// «Iguales» (`same`) o «Distintos», con el detalle.
    Verdict { same: bool, detail: String },
    Failed(String),
    Cancelled,
}

/// La suma de uno.
#[derive(Clone, Debug, PartialEq)]
pub enum HashView {
    Idle,
    Running(f32),
    Done(String),
    Failed(String),
}

/// Lo que muestra el panel.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub sides: [SideView; 2],
    pub result: ResultView,
    pub hashes: [HashView; 2],
}

/// Lo que hay bajo el mouse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompareHit {
    Close,
    Cancel,
    Retry,
    /// «Calcular SHA-256» (o Cancelar mientras calcula) del archivo 0 o 1.
    Hash(usize),
    /// «Copiar» la suma del archivo 0 o 1.
    Copy(usize),
    /// El panel, sin nada que tocar.
    Body,
}

/// El panel armado: lo que se pinta y lo que se toca.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub panel: Rect,
    pub prims: Vec<Prim>,
    pub hits: Vec<(CompareHit, Rect)>,
}

struct Builder<'a> {
    prims: Vec<Prim>,
    hits: Vec<(CompareHit, Rect)>,
    measure: &'a dyn Fn(&str) -> f32,
    x1: f32,
}

impl Builder<'_> {
    fn text(&mut self, font: FontId, s: &str, r: Rect, tone: Tone) {
        self.prims.push(Prim::Text { font, s: s.to_string(), r, tone });
    }

    fn button(&mut self, b: Rect, label: &str, hit: CompareHit) {
        self.prims.push(Prim::Fill { r: b, radius: 4.0, tone: Tone::SurfaceAlt });
        self.prims.push(Prim::Stroke { r: b, radius: 4.0, tone: Tone::Border });
        let my = (b.top + b.bottom) / 2.0;
        self.text(FontId::UiCenter, label, rect(b.left + 4.0, my - 8.0, b.right - 4.0, my + 8.0), Tone::Text);
        self.hits.push((hit, b));
    }

    /// Un botón del ancho de su texto, desde `x`.
    fn button_at(&mut self, x: f32, y: f32, label: &str, hit: CompareHit) -> Rect {
        let w = ((self.measure)(label) + 28.0).min(self.x1 - x);
        let b = rect(x, y, x + w, y + BUTTON_H);
        self.button(b, label, hit);
        b
    }

    /// Barra de avance de 4 (pista `surface_alt`, lo hecho en acento).
    fn bar(&mut self, x0: f32, x1: f32, y: f32, fraction: f32) {
        self.prims.push(Prim::Fill { r: rect(x0, y, x1, y + 4.0), radius: 2.0, tone: Tone::SurfaceAlt });
        let w = (x1 - x0) * fraction.clamp(0.0, 1.0);
        if w > 0.5 {
            self.prims.push(Prim::Fill { r: rect(x0, y, x0 + w.max(4.0), y + 4.0), radius: 2.0, tone: Tone::Accent });
        }
    }
}

/// Arma el panel en `r`. `measure` da el ancho de un texto en la letra de la interfaz.
pub fn layout(r: Rect, v: &View, measure: &dyn Fn(&str) -> f32) -> Layout {
    let (x0, x1) = (r.left + MARGIN, r.right - MARGIN);
    let mut b = Builder { prims: Vec::new(), hits: Vec::new(), measure, x1 };
    // Barra superior.
    b.text(FontId::Ui, TITLE, rect(x0, r.top + 12.0, x1 - 28.0, r.top + 28.0), Tone::Text);
    let close = rect(r.right - 33.0, r.top + 8.0, r.right - 9.0, r.top + 32.0);
    b.prims.push(Prim::Cross { x: (close.left + close.right) / 2.0, y: (close.top + close.bottom) / 2.0, size: 5.0, tone: Tone::Secondary });
    b.hits.push((CompareHit::Close, close));
    let mut y = r.top + TOP_H + 4.0;
    // Los dos archivos.
    for s in &v.sides {
        b.prims.push(Prim::Icon { icon: s.icon, x: x0, y: y + 6.0, size: ICON });
        let tx = x0 + ICON + 12.0;
        b.text(FontId::UiBold, &s.name, rect(tx, y + 4.0, x1, y + 20.0), Tone::Text);
        b.text(FontId::Small, &s.folder, rect(tx, y + 24.0, x1, y + 40.0), Tone::Dim);
        b.text(FontId::Small, &s.size, rect(tx, y + 42.0, x1, y + 58.0), Tone::Dim);
        y += SIDE_H;
    }
    b.prims.push(Prim::Fill { r: rect(x0, y + 4.0, x1, y + 5.0), radius: 0.0, tone: Tone::Border });
    y += 16.0;
    // El resultado.
    match &v.result {
        ResultView::Running { label, fraction, detail } => {
            let pct = format!("{} %", (fraction * 100.0).floor() as u32);
            b.text(FontId::Ui, label, rect(x0, y, x1 - 48.0, y + 16.0), Tone::Text);
            b.text(FontId::UiRight, &pct, rect(x1 - 48.0, y, x1, y + 16.0), Tone::Secondary);
            b.bar(x0, x1, y + 24.0, *fraction);
            b.text(FontId::Small, detail, rect(x0, y + 34.0, x1, y + 50.0), Tone::Dim);
            b.button(rect(x1 - CANCEL_W, y + 56.0, x1, y + 56.0 + BUTTON_H), CANCEL_TEXT, CompareHit::Cancel);
            y += 56.0 + BUTTON_H + 12.0;
        }
        ResultView::Verdict { same, detail } => {
            let (title, tone) = if *same { ("Iguales", Tone::Ok) } else { ("Distintos", Tone::Danger) };
            b.prims.push(Prim::Dot { x: x0 + 6.0, y: y + 11.0, radius: 6.0, tone });
            b.text(FontId::Title, title, rect(x0 + 20.0, y + 1.0, x1, y + 21.0), tone);
            b.text(FontId::Small, detail, rect(x0, y + 28.0, x1, y + 44.0), Tone::Secondary);
            y += 56.0;
        }
        ResultView::Failed(m) => {
            b.text(FontId::UiBold, "No se pudo comparar", rect(x0, y, x1, y + 16.0), Tone::Danger);
            b.text(FontId::Small, m, rect(x0, y + 20.0, x1, y + 36.0), Tone::Secondary);
            b.button_at(x0, y + 44.0, RETRY_TEXT, CompareHit::Retry);
            y += 44.0 + BUTTON_H + 12.0;
        }
        ResultView::Cancelled => {
            b.text(FontId::Ui, "Comparación cancelada", rect(x0, y, x1, y + 16.0), Tone::Dim);
            b.button_at(x0, y + 24.0, RETRY_TEXT, CompareHit::Retry);
            y += 24.0 + BUTTON_H + 12.0;
        }
    }
    // Las sumas.
    b.prims.push(Prim::Fill { r: rect(x0, y, x1, y + 1.0), radius: 0.0, tone: Tone::Border });
    b.text(FontId::SmallBold, SUMS_TITLE, rect(x0, y + 10.0, x1, y + 24.0), Tone::Secondary);
    y += 32.0;
    for (i, (s, h)) in v.sides.iter().zip(&v.hashes).enumerate() {
        b.text(FontId::Small, &s.name, rect(x0, y, x1, y + 16.0), Tone::Secondary);
        y += 20.0;
        match h {
            HashView::Idle => {
                b.button_at(x0, y, HASH_TEXT, CompareHit::Hash(i));
                y += BUTTON_H;
            }
            HashView::Running(f) => {
                let label = format!("Calculando… {} %", (f * 100.0).floor() as u32);
                b.text(FontId::Small, &label, rect(x0, y + 1.0, x1 - CANCEL_W - 8.0, y + 15.0), Tone::Dim);
                b.bar(x0, x1 - CANCEL_W - 8.0, y + 18.0, *f);
                b.button(rect(x1 - CANCEL_W, y, x1, y + BUTTON_H), CANCEL_TEXT, CompareHit::Hash(i));
                y += BUTTON_H;
            }
            HashView::Done(hex) => {
                for line in crate::compare::hex_lines(hex, x1 - x0) {
                    b.text(FontId::Mono, &line, rect(x0, y, x1, y + HEX_LINE_H), Tone::Text);
                    y += HEX_LINE_H;
                }
                y += 4.0;
                b.button_at(x0, y, COPY_TEXT, CompareHit::Copy(i));
                y += BUTTON_H;
            }
            HashView::Failed(m) => {
                b.text(FontId::Small, m, rect(x0, y, x1, y + 16.0), Tone::Danger);
                y += 20.0;
                b.button_at(x0, y, HASH_TEXT, CompareHit::Hash(i));
                y += BUTTON_H;
            }
        }
        y += 12.0;
    }
    if let [HashView::Done(a), HashView::Done(c)] = &v.hashes {
        let (text, tone) = if a == c { ("Las dos sumas coinciden", Tone::Ok) } else { ("Las sumas no coinciden", Tone::Danger) };
        b.text(FontId::UiBold, text, rect(x0, y, x1, y + 16.0), tone);
    }
    Layout { panel: r, prims: b.prims, hits: b.hits }
}

/// Lo que hay en `(x, y)` (None: fuera del panel).
pub fn hit(l: &Layout, x: f32, y: f32) -> Option<CompareHit> {
    if !contains(&l.panel, x, y) {
        return None;
    }
    Some(l.hits.iter().find(|(_, r)| contains(r, x, y)).map_or(CompareHit::Body, |(h, _)| *h))
}

/// Pinta el panel ya armado; `hover` se resalta si es un botón.
pub fn draw(p: &dyn Painter, l: &Layout, hover: Option<CompareHit>) {
    let th = p.theme();
    let r = l.panel;
    p.round_fill(r, 8.0, th.surface);
    p.round_stroke(r, 8.0, th.border);
    if r.right - r.left < 120.0 {
        return;
    }
    p.clip(r, &mut || {
        for prim in &l.prims {
            idraw::paint(p, prim);
        }
        if let Some(h) = hover.filter(|h| *h != CompareHit::Body)
            && let Some((_, rc)) = l.hits.iter().find(|(x, _)| *x == h)
        {
            if h == CompareHit::Close {
                p.round_fill(*rc, 4.0, th.hover);
                let (cx, cy) = ((rc.left + rc.right) / 2.0, (rc.top + rc.bottom) / 2.0);
                idraw::paint(p, &Prim::Cross { x: cx, y: cy, size: 5.0, tone: Tone::Text });
            } else {
                p.round_stroke(*rc, 4.0, th.accent);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(result: ResultView, hashes: [HashView; 2]) -> View {
        let side = |n: &str| SideView { name: n.into(), folder: "/x".into(), size: "4 bytes".into(), icon: 0 };
        View { sides: [side("a.txt"), side("b.txt")], result, hashes }
    }

    fn texts(l: &Layout) -> Vec<String> {
        l.prims.iter().filter_map(|p| if let Prim::Text { s, .. } = p { Some(s.clone()) } else { None }).collect()
    }

    fn tone_of(l: &Layout, text: &str) -> Option<Tone> {
        l.prims.iter().find_map(|p| match p {
            Prim::Text { s, tone, .. } if s == text => Some(*tone),
            _ => None,
        })
    }

    const R: Rect = Rect { left: 1000.0, top: 40.0, right: 1320.0, bottom: 820.0 };

    /// Comparando: avance, Cancelar y los botones de las sumas.
    #[test]
    fn running_shows_progress_and_cancel() {
        let l = layout(R, &view(ResultView::Running { label: "Comparando…".into(), fraction: 0.37, detail: "1 de 3".into() }, [HashView::Idle, HashView::Idle]), &|s| s.len() as f32 * 7.0);
        let t = texts(&l);
        assert!(t.contains(&"Comparando…".to_string()) && t.contains(&"37 %".to_string()), "{t:?}");
        let cancel = l.hits.iter().find(|(h, _)| *h == CompareHit::Cancel).unwrap().1;
        assert_eq!((cancel.right - cancel.left, cancel.bottom - cancel.top), (CANCEL_W, BUTTON_H));
        assert_eq!(hit(&l, (cancel.left + cancel.right) / 2.0, cancel.top + 5.0), Some(CompareHit::Cancel));
        assert!(l.hits.iter().any(|(h, _)| *h == CompareHit::Hash(0)) && l.hits.iter().any(|(h, _)| *h == CompareHit::Hash(1)));
        assert_eq!(hit(&l, 10.0, 10.0), None);
        assert_eq!(hit(&l, R.left + 2.0, R.bottom - 2.0), Some(CompareHit::Body));
        let close = l.hits.iter().find(|(h, _)| *h == CompareHit::Close).unwrap().1;
        assert_eq!(hit(&l, close.left + 2.0, close.top + 2.0), Some(CompareHit::Close));
    }

    /// «Iguales» en `ok` y «Distintos» en `danger`.
    #[test]
    fn verdict_colors() {
        let l = layout(R, &view(ResultView::Verdict { same: true, detail: "x".into() }, [HashView::Idle, HashView::Idle]), &|s| s.len() as f32 * 7.0);
        assert_eq!(tone_of(&l, "Iguales"), Some(Tone::Ok));
        assert!(!l.hits.iter().any(|(h, _)| *h == CompareHit::Cancel));
        let l = layout(R, &view(ResultView::Verdict { same: false, detail: "x".into() }, [HashView::Idle, HashView::Idle]), &|s| s.len() as f32 * 7.0);
        assert_eq!(tone_of(&l, "Distintos"), Some(Tone::Danger));
    }

    /// Las sumas: en letra de ancho fijo partida al ancho, con «Copiar», y si las dos están, si
    /// coinciden.
    #[test]
    fn sums_with_copy_and_match() {
        let sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string();
        let l = layout(R, &view(ResultView::Cancelled, [HashView::Done(sha.clone()), HashView::Done(sha.clone())]), &|s| s.len() as f32 * 7.0);
        let mono: Vec<String> = l.prims.iter().filter_map(|p| if let Prim::Text { font: FontId::Mono, s, .. } = p { Some(s.clone()) } else { None }).collect();
        assert_eq!(mono.concat(), sha.repeat(2));
        assert!(mono.iter().all(|m| m.len() == 32), "{mono:?}");
        assert!(l.hits.iter().any(|(h, _)| *h == CompareHit::Copy(1)));
        assert_eq!(tone_of(&l, "Las dos sumas coinciden"), Some(Tone::Ok));
        assert!(l.hits.iter().any(|(h, _)| *h == CompareHit::Retry));
        let l = layout(R, &view(ResultView::Cancelled, [HashView::Done(sha.clone()), HashView::Done(sha.replace('e', "f"))]), &|s| s.len() as f32 * 7.0);
        assert_eq!(tone_of(&l, "Las sumas no coinciden"), Some(Tone::Danger));
    }
}

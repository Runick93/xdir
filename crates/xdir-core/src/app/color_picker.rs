//! Selector de color propio de xdir: reemplaza al cuadro de colores del sistema (el de Windows
//! rompía el estilo de la app). Un cuadrado de saturación y brillo, una franja de tono, una caja
//! de valor hexadecimal que se escribe a mano, la vista previa («Actual» contra «Nuevo») y una
//! fila de colores sugeridos. Se usa para el color de acento y para el de fondo.
//!
//! Como el diálogo «Acerca de», el diseño sale solo del tamaño de la ventana (`layout`), así el
//! clic se resuelve con las mismas medidas con que se dibujó. El panel es modal.

use crate::geom::{Color, Rect, contains, rect};
use crate::keys::{Key, keys::*};
use crate::paint::{FontId, Painter};
use crate::theme::{self, alpha};

const WIDTH: f32 = 360.0;
const SIDE: f32 = 16.0;
const SQUARE_H: f32 = 170.0;
const BAR_H: f32 = 16.0;
const SWATCH_STEP: f32 = 27.0;
const SWATCH_R: f32 = 10.0;
/// Lado de cada celdita con que se pinta el degradé (se solapan para que no queden costuras).
const CELL: f32 = 5.0;
/// Dónde empieza el texto del valor dentro de su caja.
const HEX_TEXT_X: f32 = 24.0;
const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Para qué se elige el color.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickTarget {
    Accent,
    Background,
}

impl PickTarget {
    fn title(self) -> &'static str {
        match self {
            PickTarget::Accent => "Color de acento",
            PickTarget::Background => "Color de fondo",
        }
    }

    /// Los colores sugeridos: los 12 énfasis, o fondos oscuros y claros para elegir de un toque.
    pub fn suggestions(self) -> [u32; 12] {
        match self {
            PickTarget::Accent => std::array::from_fn(|i| theme::PRESETS[i].1),
            PickTarget::Background => std::array::from_fn(|i| theme::BACKGROUNDS[i].1),
        }
    }
}

/// Qué hay bajo el mouse.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickHit {
    Square,
    Hue,
    Hex,
    Swatch(usize),
    Ok,
    Cancel,
    Close,
    Panel,
}

/// Lo que pasó con una tecla o un clic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Sigue abierto.
    Open,
    /// Aceptado con este color (0xRRGGBB).
    Accept(u32),
    /// Cancelado: no cambia nada.
    Cancel,
}

pub struct ColorPicker {
    pub target: PickTarget,
    h: f32,
    s: f32,
    v: f32,
    /// Lo escrito en la caja (sin «#», en mayúsculas).
    hex: String,
    /// Posición del cursor en la caja (en dígitos) y, si hay selección, desde dónde se eligió.
    caret: usize,
    anchor: Option<usize>,
    /// La caja tiene el foco del teclado: las flechas mueven su cursor. Si no, mueven el cuadrado.
    hex_focus: bool,
    /// Dónde cae cada posición del cursor al pintar (DIPs desde el inicio del texto).
    offsets: std::cell::RefCell<Vec<f32>>,
    /// El color con que se abrió, para compararlo en la vista previa.
    original: u32,
    /// Lo que se está arrastrando con el mouse apretado.
    dragging: Option<PickHit>,
}

struct Layout {
    panel: Rect,
    close: Rect,
    square: Rect,
    bar: Rect,
    preview: Rect,
    hex: Rect,
    swatches: [Rect; 12],
    cancel: Rect,
    ok: Rect,
}

fn layout(w: f32, h: f32) -> Layout {
    let left = ((w - WIDTH) / 2.0).max(8.0);
    let right = (left + WIDTH).min(w - 8.0);
    let height = 58.0 + SQUARE_H + 14.0 + BAR_H + 16.0 + 32.0 + 22.0 + 12.0 + 24.0 + 16.0 + 58.0;
    let top = ((h - height) / 2.0).max(8.0);
    let panel = rect(left, top, right, top + height);
    let close = rect(panel.right - 36.0, panel.top + 8.0, panel.right - 8.0, panel.top + 36.0);
    let (l, r) = (panel.left + SIDE, panel.right - SIDE);
    let square = rect(l, panel.top + 58.0, r, panel.top + 58.0 + SQUARE_H);
    let bar = rect(l, square.bottom + 14.0, r, square.bottom + 14.0 + BAR_H);
    let row = bar.bottom + 16.0;
    let preview = rect(l, row, l + 96.0, row + 32.0);
    let hex = rect(preview.right + 16.0, row, (preview.right + 16.0 + 130.0).min(r), row + 32.0);
    let sw_top = row + 32.0 + 22.0 + 12.0;
    let swatches = std::array::from_fn(|i| {
        let cx = l + SWATCH_R + 2.0 + SWATCH_STEP * i as f32;
        rect(cx - 13.0, sw_top - 1.0, cx + 13.0, sw_top + 25.0)
    });
    let ok = rect(panel.right - 16.0 - 104.0, panel.bottom - 44.0, panel.right - 16.0, panel.bottom - 12.0);
    let cancel = rect(ok.left - 8.0 - 104.0, ok.top, ok.left - 8.0, ok.bottom);
    Layout { panel, close, square, bar, preview, hex, swatches, cancel, ok }
}

// ---------- conversiones ----------

/// Tono (0 a 360), saturación y brillo (0 a 1) → 0xRRGGBB.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let h = h.rem_euclid(360.0) / 60.0;
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let ch = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    ch(r) << 16 | ch(g) << 8 | ch(b)
}

/// 0xRRGGBB → (tono, saturación, brillo).
pub fn rgb_to_hsv(c: u32) -> (f32, f32, f32) {
    let f = |sh: u32| (c >> sh & 0xFF) as f32 / 255.0;
    let (r, g, b) = (f(16), f(8), f(0));
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// «#RGB», «RGB», «#RRGGBB» o «RRGGBB» → 0xRRGGBB.
pub fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    match s.len() {
        6 => u32::from_str_radix(s, 16).ok(),
        3 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            let e = |sh: u32| (v >> sh & 0xF) * 0x11;
            Some(e(8) << 16 | e(4) << 8 | e(0))
        }
        _ => None,
    }
}

impl ColorPicker {
    pub fn new(target: PickTarget, current: u32) -> ColorPicker {
        let (h, s, v) = rgb_to_hsv(current);
        ColorPicker { target, h, s, v, hex: format!("{current:06X}"), caret: 6, anchor: Some(0), hex_focus: true, offsets: Default::default(), original: current, dragging: None }
    }

    /// El color de ahora.
    pub fn color(&self) -> u32 {
        hsv_to_rgb(self.h, self.s, self.v)
    }

    fn set_rgb(&mut self, c: u32) {
        let (h, s, v) = rgb_to_hsv(c);
        // En los grises el tono no existe: se conserva el de antes para que la franja no salte.
        if s > 0.0 && v > 0.0 {
            self.h = h;
        }
        (self.s, self.v) = (s, v);
        self.hex = format!("{c:06X}");
        self.select_all();
    }

    fn sync_hex(&mut self) {
        self.hex = format!("{:06X}", self.color());
        self.select_all();
    }

    /// Lo que hay en (`x`, `y`) con la ventana de `w` × `h`; `None` fuera del panel.
    pub fn hit(&self, w: f32, h: f32, x: f32, y: f32) -> Option<PickHit> {
        let l = layout(w, h);
        if !contains(&l.panel, x, y) {
            return None;
        }
        let part = if contains(&l.close, x, y) {
            PickHit::Close
        } else if contains(&l.ok, x, y) {
            PickHit::Ok
        } else if contains(&l.cancel, x, y) {
            PickHit::Cancel
        } else if contains(&l.square, x, y) {
            PickHit::Square
        } else if contains(&rect(l.bar.left, l.bar.top - 6.0, l.bar.right, l.bar.bottom + 6.0), x, y) {
            PickHit::Hue
        } else if contains(&l.hex, x, y) {
            PickHit::Hex
        } else if let Some(i) = l.swatches.iter().position(|r| contains(r, x, y)) {
            PickHit::Swatch(i)
        } else {
            PickHit::Panel
        };
        Some(part)
    }

    /// Mueve el cuadrado o la franja a (`x`, `y`) (también fuera de ellos: se arrastra con el mouse
    /// capturado y el valor queda en el borde).
    fn drag_to(&mut self, w: f32, h: f32, x: f32, y: f32) {
        let l = layout(w, h);
        match self.dragging {
            Some(PickHit::Square) => {
                self.s = ((x - l.square.left) / (l.square.right - l.square.left)).clamp(0.0, 1.0);
                self.v = 1.0 - ((y - l.square.top) / (l.square.bottom - l.square.top)).clamp(0.0, 1.0);
                self.sync_hex();
            }
            Some(PickHit::Hue) => {
                self.h = ((x - l.bar.left) / (l.bar.right - l.bar.left)).clamp(0.0, 1.0) * 360.0;
                self.sync_hex();
            }
            Some(PickHit::Hex) => {
                self.caret = self.index_at(w, h, x);
            }
            _ => {}
        }
    }

    fn select_all(&mut self) {
        self.caret = self.hex.chars().count();
        self.anchor = Some(0);
    }

    /// La selección de la caja como (desde, hasta), si hay.
    fn selection(&self) -> Option<(usize, usize)> {
        self.anchor.filter(|a| *a != self.caret).map(|a| (a.min(self.caret), a.max(self.caret)))
    }

    /// Qué posición del cursor queda más cerca de la coordenada `x` (según lo último pintado).
    fn index_at(&self, w: f32, h: f32, x: f32) -> usize {
        let start = layout(w, h).hex.left + HEX_TEXT_X;
        let offsets = self.offsets.borrow();
        if offsets.len() != self.hex.chars().count() + 1 {
            return self.hex.chars().count();
        }
        (0..offsets.len())
            .min_by(|a, b| (x - start - offsets[*a]).abs().total_cmp(&(x - start - offsets[*b]).abs()))
            .unwrap_or(0)
    }

    /// Borra la selección (deja el cursor donde estaba). Devuelve si había.
    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else { return false };
        self.hex = self.hex.chars().take(a).chain(self.hex.chars().skip(b)).collect();
        (self.caret, self.anchor) = (a, None);
        true
    }

    /// Si ya hay 6 dígitos, el color pasa a ser ese (sin tocar el texto ni el cursor).
    fn apply_typed_hex(&mut self) {
        if self.hex.len() == 6
            && let Some(c) = parse_hex(&self.hex)
        {
            let (h, s, v) = rgb_to_hsv(c);
            if s > 0.0 && v > 0.0 {
                self.h = h;
            }
            (self.s, self.v) = (s, v);
        }
    }

    /// Mueve el cursor de la caja; con Mayús la selección crece, si no se deshace.
    fn move_caret(&mut self, to: usize, shift: bool) {
        if shift {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = to;
    }

    /// Botón izquierdo apretado en (`x`, `y`).
    pub fn press(&mut self, w: f32, h: f32, x: f32, y: f32) -> Outcome {
        self.dragging = None;
        match self.hit(w, h, x, y) {
            Some(PickHit::Ok) => return self.accept(),
            Some(PickHit::Cancel | PickHit::Close) => return Outcome::Cancel,
            Some(part @ (PickHit::Square | PickHit::Hue)) => {
                self.hex_focus = false;
                self.dragging = Some(part);
                self.drag_to(w, h, x, y);
            }
            Some(PickHit::Swatch(i)) => self.set_rgb(self.target.suggestions()[i]),
            // Un clic en la caja pone el cursor ahí; arrastrando se selecciona.
            Some(PickHit::Hex) => {
                self.hex_focus = true;
                self.caret = self.index_at(w, h, x);
                self.anchor = Some(self.caret);
                self.dragging = Some(PickHit::Hex);
            }
            _ => {}
        }
        Outcome::Open
    }

    /// El mouse se movió con el botón apretado. Devuelve si hay que repintar.
    pub fn drag(&mut self, w: f32, h: f32, x: f32, y: f32) -> bool {
        if self.dragging.is_none() {
            return false;
        }
        self.drag_to(w, h, x, y);
        true
    }

    /// Se soltó el botón. Devuelve si se estaba arrastrando.
    pub fn release(&mut self) -> bool {
        self.dragging.take().is_some()
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.is_some()
    }

    fn accept(&mut self) -> Outcome {
        // Un valor a medio escribir de 3 dígitos («#F80») se completa antes de aceptar.
        if let Some(c) = parse_hex(&self.hex).filter(|_| self.hex.len() == 3) {
            self.set_rgb(c);
        }
        Outcome::Accept(self.color())
    }

    /// Una tecla: Esc cancela y Enter acepta. Con el foco en la caja del valor, las flechas, Inicio,
    /// Fin, Retroceso, Supr y Ctrl+A la editan (con Mayús se selecciona). Si no, las flechas mueven
    /// el cuadrado (Mayús + izquierda o derecha mueve el tono).
    pub fn key(&mut self, vk: Key, shift: bool, ctrl: bool) -> Outcome {
        const STEP: f32 = 0.02;
        let len = self.hex.chars().count();
        match vk {
            VK_ESCAPE => return Outcome::Cancel,
            VK_RETURN => return self.accept(),
            Key(0x41) if ctrl => {
                self.hex_focus = true;
                self.select_all();
            }
            VK_BACK | VK_DELETE if self.hex_focus => {
                if !self.delete_selection() {
                    if vk == VK_BACK && self.caret > 0 {
                        self.caret -= 1;
                        self.hex = self.hex.chars().take(self.caret).chain(self.hex.chars().skip(self.caret + 1)).collect();
                    } else if vk == VK_DELETE && self.caret < len {
                        self.hex = self.hex.chars().take(self.caret).chain(self.hex.chars().skip(self.caret + 1)).collect();
                    }
                }
                self.anchor = None;
            }
            VK_LEFT if self.hex_focus => match self.selection().filter(|_| !shift) {
                Some((a, _)) => self.move_caret(a, false),
                None => self.move_caret(self.caret.saturating_sub(1), shift),
            },
            VK_RIGHT if self.hex_focus => match self.selection().filter(|_| !shift) {
                Some((_, b)) => self.move_caret(b, false),
                None => self.move_caret((self.caret + 1).min(len), shift),
            },
            VK_HOME if self.hex_focus => self.move_caret(0, shift),
            VK_END if self.hex_focus => self.move_caret(len, shift),
            VK_LEFT | VK_RIGHT if shift => {
                self.h = (self.h + if vk == VK_LEFT { -4.0 } else { 4.0 }).rem_euclid(360.0);
                self.sync_hex();
            }
            VK_LEFT => {
                self.s = (self.s - STEP).max(0.0);
                self.sync_hex();
            }
            VK_RIGHT => {
                self.s = (self.s + STEP).min(1.0);
                self.sync_hex();
            }
            VK_UP => {
                self.v = (self.v + STEP).min(1.0);
                self.sync_hex();
            }
            VK_DOWN => {
                self.v = (self.v - STEP).max(0.0);
                self.sync_hex();
            }
            _ => {}
        }
        Outcome::Open
    }

    /// Un carácter escrito: va a la caja si es un dígito hexadecimal (reemplaza la selección, si
    /// hay); con 6 dígitos ya cambia el color.
    pub fn char(&mut self, c: char) {
        if !c.is_ascii_hexdigit() {
            return;
        }
        self.hex_focus = true;
        self.delete_selection();
        self.anchor = None;
        if self.hex.chars().count() < 6 {
            let at = self.caret.min(self.hex.len());
            self.hex.insert(at, c.to_ascii_uppercase());
            self.caret = at + 1;
        }
        self.apply_typed_hex();
    }

    /// Lo que copia Ctrl+C: la selección tal cual o, sin selección, el color entero con su «#».
    pub fn copy_text(&self) -> String {
        match self.selection() {
            Some((a, b)) => self.hex.chars().skip(a).take(b - a).collect(),
            None => format!("#{}", self.hex),
        }
    }

    /// Ctrl+X: devuelve lo copiado y borra la selección (sin selección, no borra nada).
    pub fn cut_text(&mut self) -> String {
        let text = self.copy_text();
        self.delete_selection();
        text
    }

    /// Ctrl+V: un color completo («#FF8800», «ff8800», «#F80») reemplaza todo el valor; otro texto
    /// entra con sus dígitos hexadecimales en el cursor, reemplazando la selección.
    pub fn paste(&mut self, text: &str) {
        self.hex_focus = true;
        let digits: String = text.chars().filter(char::is_ascii_hexdigit).map(|c| c.to_ascii_uppercase()).collect();
        let whole = text.trim().trim_start_matches('#');
        if whole.chars().all(|c| c.is_ascii_hexdigit()) && matches!(whole.len(), 3 | 6) {
            self.hex = digits.clone();
            self.caret = digits.len();
            self.anchor = None;
        } else {
            self.delete_selection();
            self.anchor = None;
            for c in digits.chars() {
                if self.hex.chars().count() < 6 {
                    self.hex.insert(self.caret.min(self.hex.len()), c);
                    self.caret += 1;
                }
            }
        }
        if self.hex.len() == 3
            && let Some(c) = parse_hex(&self.hex)
        {
            // «#F80» pegado: se ve ya como «FF8800».
            self.set_rgb(c);
            self.anchor = None;
            self.caret = 6;
        }
        self.apply_typed_hex();
    }

    /// Pinta el selector sobre toda la ventana (`w` × `h`).
    pub fn draw(&self, p: &dyn Painter, w: f32, h: f32, hover: Option<PickHit>) {
        let th = p.theme();
        let l = layout(w, h);
        crate::net::drives_dialog::frame(p, w, h, l.panel, self.target.title(), l.close, hover == Some(PickHit::Close));

        // Cuadrado: saturación hacia la derecha, brillo hacia arriba, con el tono de ahora.
        let sq = l.square;
        let (nx, ny) = (((sq.right - sq.left) / CELL).ceil() as u32, ((sq.bottom - sq.top) / CELL).ceil() as u32);
        p.clip(sq, &mut || {
            for iy in 0..ny {
                for ix in 0..nx {
                    let s = (ix as f32 + 0.5) / nx as f32;
                    let v = 1.0 - (iy as f32 + 0.5) / ny as f32;
                    let (x, y) = (sq.left + ix as f32 * CELL, sq.top + iy as f32 * CELL);
                    p.fill(rect(x, y, x + CELL + 1.0, y + CELL + 1.0), crate::ui::hex(hsv_to_rgb(self.h, s, v)));
                }
            }
        });
        p.round_stroke(sq, 3.0, th.border);
        let (px, py) = (sq.left + self.s * (sq.right - sq.left), sq.top + (1.0 - self.v) * (sq.bottom - sq.top));
        p.circle_stroke(px, py, 8.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 }, 1.5);
        p.circle_stroke(px, py, 6.5, WHITE, 2.0);

        // Franja de tono.
        let bar = l.bar;
        let segs = ((bar.right - bar.left) / 3.0).ceil() as u32;
        p.round_fill(bar, 8.0, th.background);
        p.clip(bar, &mut || {
            for i in 0..segs {
                let x = bar.left + i as f32 * 3.0;
                p.fill(rect(x, bar.top, x + 4.0, bar.bottom), crate::ui::hex(hsv_to_rgb(i as f32 / segs as f32 * 360.0, 1.0, 1.0)));
            }
        });
        p.round_stroke(bar, 3.0, th.border);
        let hx = bar.left + self.h / 360.0 * (bar.right - bar.left);
        let cy = (bar.top + bar.bottom) / 2.0;
        p.circle(hx, cy, 9.0, crate::ui::hex(hsv_to_rgb(self.h, 1.0, 1.0)));
        p.circle_stroke(hx, cy, 9.0, WHITE, 2.0);

        // Vista previa: el color con que se abrió contra el nuevo.
        let pv = l.preview;
        let mid = (pv.left + pv.right) / 2.0;
        p.round_fill(rect(pv.left, pv.top, mid + 6.0, pv.bottom), 6.0, crate::ui::hex(self.original));
        p.round_fill(rect(mid - 6.0, pv.top, pv.right, pv.bottom), 6.0, crate::ui::hex(self.color()));
        p.fill(rect(mid - 6.0, pv.top, mid, pv.bottom), crate::ui::hex(self.original));
        p.round_stroke(pv, 6.0, th.border);
        for (label, a, b) in [("Actual", pv.left, mid), ("Nuevo", mid, pv.right)] {
            let tw = p.measure(FontId::Small, label);
            let x = (a + b) / 2.0 - tw / 2.0;
            p.text(FontId::Small, label, rect(x, pv.bottom + 4.0, x + tw + 4.0, pv.bottom + 20.0), th.text_dim);
        }

        // Caja del valor hexadecimal.
        let hx = l.hex;
        p.round_fill(hx, 6.0, th.background);
        p.round_stroke_w(hx, 6.0, if hover == Some(PickHit::Hex) { th.text_dim } else { th.border }, 1.0);
        p.text(FontId::Ui, "#", rect(hx.left + 10.0, hx.top, hx.left + 24.0, hx.bottom), th.text_dim);
        // Dónde cae cada posición del cursor (también lo usa el clic para ubicarlo).
        let chars: Vec<char> = self.hex.chars().collect();
        let offsets: Vec<f32> = (0..=chars.len()).map(|i| p.measure(FontId::Ui, &chars[..i].iter().collect::<String>())).collect();
        let tx = hx.left + HEX_TEXT_X;
        if self.hex_focus
            && let Some((a, b)) = self.selection()
        {
            p.round_fill(rect(tx + offsets[a] - 1.0, hx.top + 6.0, tx + offsets[b] + 1.0, hx.bottom - 6.0), 3.0, alpha(th.accent, 0.45));
        }
        p.text(FontId::Ui, &self.hex, rect(tx, hx.top, hx.right - 6.0, hx.bottom), th.text);
        if self.hex_focus && self.selection().is_none() {
            let cx = tx + offsets[self.caret.min(chars.len())];
            p.fill(rect(cx, hx.top + 7.0, cx + 1.5, hx.bottom - 7.0), th.text);
        }
        *self.offsets.borrow_mut() = offsets;
        p.text(FontId::Small, "Valor hexadecimal", rect(hx.left, hx.bottom + 4.0, hx.right + 40.0, hx.bottom + 20.0), th.text_dim);

        // Colores sugeridos.
        let current = self.color();
        for (i, c) in self.target.suggestions().into_iter().enumerate() {
            let r = l.swatches[i];
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            if c == current {
                p.circle_stroke(cx, cy, SWATCH_R + 3.0, th.text, 2.0);
            } else if hover == Some(PickHit::Swatch(i)) {
                p.circle_stroke(cx, cy, SWATCH_R + 3.0, th.text_dim, 1.0);
            }
            p.circle(cx, cy, SWATCH_R, crate::ui::hex(c));
            p.circle_stroke(cx, cy, SWATCH_R, alpha(th.text, 0.18), 1.0);
        }

        // Botones.
        let text = |r: Rect| rect(r.left, r.top + 8.0, r.right, r.bottom - 6.0);
        let hot = hover == Some(PickHit::Cancel);
        p.round_fill(l.cancel, 6.0, if hot { th.hover } else { th.surface });
        p.round_stroke(l.cancel, 6.0, th.border);
        p.text(FontId::UiCenter, "Cancelar", text(l.cancel), th.text);
        let hot = hover == Some(PickHit::Ok);
        p.round_fill(l.ok, 6.0, if hot { alpha(th.accent, 0.85) } else { th.accent });
        p.text(FontId::UiCenter, "Aceptar", text(l.ok), WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    const W: f32 = 1000.0;
    const H: f32 = 800.0;

    fn center(r: &Rect) -> (f32, f32) {
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    #[test]
    fn hsv_roundtrips_every_color_on_a_grid() {
        for r in (0..=255u32).step_by(15) {
            for g in (0..=255u32).step_by(15) {
                for b in (0..=255u32).step_by(15) {
                    let c = r << 16 | g << 8 | b;
                    let (h, s, v) = rgb_to_hsv(c);
                    assert_eq!(hsv_to_rgb(h, s, v), c, "{c:06X}");
                }
            }
        }
    }

    #[test]
    fn known_colors() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), 0xFF0000);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), 0x00FF00);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), 0x0000FF);
        assert_eq!(hsv_to_rgb(0.0, 0.0, 1.0), 0xFFFFFF);
        assert_eq!(hsv_to_rgb(200.0, 0.5, 0.0), 0x000000);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#3B9EFF"), Some(0x3B9EFF));
        assert_eq!(parse_hex("3b9eff"), Some(0x3B9EFF));
        assert_eq!(parse_hex("#F80"), Some(0xFF8800));
        assert_eq!(parse_hex("12345"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn opens_on_the_current_color() {
        let p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        assert_eq!(p.color(), 0x3B9EFF);
        assert_eq!(p.hex, "3B9EFF");
    }

    #[test]
    fn clicking_the_square_picks_saturation_and_value() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0xFF0000);
        let l = layout(W, H);
        // Arriba a la derecha: saturación y brillo al máximo; abajo: negro.
        assert_eq!(p.press(W, H, l.square.right - 0.5, l.square.top + 0.1), Outcome::Open);
        assert_eq!(p.color(), 0xFF0000);
        p.press(W, H, l.square.left + 0.1, l.square.top + 0.1);
        assert_eq!(p.color(), 0xFFFFFF);
        p.press(W, H, l.square.left + 0.1, l.square.bottom - 0.1);
        assert_eq!(p.color(), 0x000000);
        assert!(p.is_dragging());
        assert!(p.release());
        assert!(!p.is_dragging());
    }

    #[test]
    fn dragging_past_the_edge_clamps_and_keeps_going() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0xFF0000);
        let l = layout(W, H);
        let (x, y) = center(&l.square);
        p.press(W, H, x, y);
        assert!(p.drag(W, H, l.square.right + 500.0, l.square.top - 500.0));
        assert_eq!(p.color(), 0xFF0000, "afuera queda en la esquina: todo saturado y brillante");
        assert!(p.drag(W, H, l.square.left - 500.0, l.square.bottom + 500.0));
        assert_eq!(p.color(), 0x000000);
    }

    #[test]
    fn the_hue_bar_walks_the_spectrum() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0xFF0000);
        let l = layout(W, H);
        let y = (l.bar.top + l.bar.bottom) / 2.0;
        p.press(W, H, l.bar.left + (l.bar.right - l.bar.left) / 3.0, y);
        assert_eq!(p.color(), 0x00FF00);
        p.drag(W, H, l.bar.left + (l.bar.right - l.bar.left) * 2.0 / 3.0, y);
        assert_eq!(p.color(), 0x0000FF);
        p.release();
    }

    #[test]
    fn typing_hex_applies_at_six_digits() {
        let mut p = ColorPicker::new(PickTarget::Background, 0x000000);
        for c in "12AB3".chars() {
            p.char(c);
        }
        assert_eq!(p.color(), 0x000000, "con 5 dígitos todavía no cambia");
        p.char('f');
        assert_eq!(p.color(), 0x12AB3F);
        p.char('g');
        assert_eq!(p.hex, "12AB3F", "lo que no es hexadecimal se ignora");
        p.char('0');
        assert_eq!(p.hex, "12AB3F", "con los seis completos no entra otro dígito");
    }

    #[test]
    fn enter_accepts_and_completes_a_short_hex() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0x000000);
        p.key(VK_BACK, false, false);
        for _ in 0..6 {
            p.key(VK_BACK, false, false);
        }
        for c in "f80".chars() {
            p.char(c);
        }
        assert_eq!(p.key(VK_RETURN, false, false), Outcome::Accept(0xFF8800));
    }

    #[test]
    fn escape_cancels_and_enter_accepts_the_picked_color() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        assert_eq!(p.key(VK_ESCAPE, false, false), Outcome::Cancel);
        assert_eq!(p.key(VK_RETURN, false, false), Outcome::Accept(0x3B9EFF));
    }

    #[test]
    fn arrows_move_the_picker_and_shift_moves_the_hue() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0xFF0000);
        p.hex_focus = false;
        p.key(VK_LEFT, false, false);
        assert!(p.s < 1.0);
        let before = p.h;
        p.key(VK_RIGHT, true, false);
        assert!(p.h > before);
        p.key(VK_DOWN, false, false);
        assert!(p.v < 1.0);
        assert_eq!(p.hex, format!("{:06X}", p.color()), "la caja sigue al color");
    }

    #[test]
    fn buttons_close_and_swatches() {
        let l = layout(W, H);
        let mut p = ColorPicker::new(PickTarget::Accent, 0x000000);
        let (x, y) = center(&l.swatches[3]);
        assert_eq!(p.hit(W, H, x, y), Some(PickHit::Swatch(3)));
        p.press(W, H, x, y);
        assert_eq!(p.color(), theme::PRESETS[3].1);
        let (x, y) = center(&l.cancel);
        assert_eq!(p.press(W, H, x, y), Outcome::Cancel);
        let (x, y) = center(&l.close);
        assert_eq!(p.press(W, H, x, y), Outcome::Cancel);
        let (x, y) = center(&l.ok);
        assert_eq!(p.press(W, H, x, y), Outcome::Accept(theme::PRESETS[3].1));
        assert_eq!(p.hit(W, H, 1.0, 1.0), None, "fuera del panel no hay nada");
        assert_eq!(p.press(W, H, 1.0, 1.0), Outcome::Open, "un clic afuera no cierra");
    }

    #[test]
    fn grey_keeps_the_hue_so_the_bar_does_not_jump() {
        let mut p = ColorPicker::new(PickTarget::Background, 0xFF0000);
        let l = layout(W, H);
        let y = (l.bar.top + l.bar.bottom) / 2.0;
        p.press(W, H, l.bar.left + (l.bar.right - l.bar.left) / 3.0, y);
        p.release();
        let h = p.h;
        p.char('8');
        for c in "08080".chars() {
            p.char(c);
        }
        // Un 808080 escrito a mano no mueve el tono.
        let mut q = ColorPicker::new(PickTarget::Background, 0xFF0000);
        q.h = h;
        q.set_rgb(0x808080);
        assert_eq!(q.h, h);
    }

    #[test]
    fn draws_title_labels_and_buttons_for_each_target() {
        crate::sys::testing::install_fakes();
        for (target, title) in [(PickTarget::Accent, "Color de acento"), (PickTarget::Background, "Color de fondo")] {
            let painter = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)));
            ColorPicker::new(target, 0x3B9EFF).draw(&painter, W, H, None);
            let t: Vec<String> = painter.texts().into_iter().map(|x| x.1).collect();
            for s in [title, "Actual", "Nuevo", "Aceptar", "Cancelar", "Valor hexadecimal"] {
                assert!(t.iter().any(|x| x == s), "{s}: {t:?}");
            }
        }
    }

    #[test]
    fn panel_stays_inside_a_small_window() {
        let l = layout(400.0, 300.0);
        assert!(l.panel.left >= 8.0 && l.panel.top >= 8.0 && l.panel.right <= 392.0);
    }

    #[test]
    fn every_suggestion_is_distinct() {
        for t in [PickTarget::Accent, PickTarget::Background] {
            let s = t.suggestions();
            let mut v = s.to_vec();
            v.sort_unstable();
            v.dedup();
            assert_eq!(v.len(), 12);
        }
    }
}

#[cfg(test)]
mod hex_selection_tests {
    use super::*;

    const W: f32 = 1000.0;
    const H: f32 = 800.0;

    /// Un selector con lo medido como lo dejaría el pintado: 10 DIPs por dígito.
    fn picker() -> ColorPicker {
        let p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        *p.offsets.borrow_mut() = (0..=6).map(|i| i as f32 * 10.0).collect();
        p
    }

    fn hex_x(i: usize) -> (f32, f32) {
        let l = layout(W, H);
        (l.hex.left + HEX_TEXT_X + i as f32 * 10.0, (l.hex.top + l.hex.bottom) / 2.0)
    }

    #[test]
    fn opens_with_everything_selected_and_backspace_clears_it() {
        let mut p = picker();
        assert_eq!(p.selection(), Some((0, 6)));
        p.key(VK_BACK, false, false);
        assert_eq!(p.hex, "");
        assert_eq!(p.selection(), None);
    }

    #[test]
    fn clicking_puts_the_cursor_there_and_typing_inserts() {
        let mut p = picker();
        let (x, y) = hex_x(2);
        p.press(W, H, x + 3.0, y);
        p.release();
        assert_eq!((p.caret, p.selection()), (2, None));
        p.char('0');
        assert_eq!(p.hex, "3B9EFF", "con los seis dígitos completos no entra otro");
        p.key(VK_BACK, false, false);
        p.char('0');
        assert_eq!(p.hex, "309EFF", "borra el de la izquierda y escribe en su lugar");
        assert_eq!(p.caret, 2);
    }

    #[test]
    fn arrows_home_end_move_the_cursor_and_shift_selects() {
        let mut p = picker();
        let (x, y) = hex_x(3);
        p.press(W, H, x, y);
        p.release();
        p.key(VK_LEFT, false, false);
        assert_eq!(p.caret, 2);
        p.key(VK_RIGHT, true, false);
        p.key(VK_RIGHT, true, false);
        assert_eq!(p.selection(), Some((2, 4)));
        p.key(VK_RIGHT, false, false);
        assert_eq!((p.caret, p.selection()), (4, None), "sin Mayús, la flecha cierra la selección en su borde");
        p.key(VK_HOME, false, false);
        assert_eq!(p.caret, 0);
        p.key(VK_END, true, false);
        assert_eq!(p.selection(), Some((0, 6)));
    }

    #[test]
    fn dragging_across_the_digits_selects_them_and_typing_replaces() {
        let mut p = picker();
        let (x, y) = hex_x(1);
        p.press(W, H, x, y);
        p.drag(W, H, hex_x(4).0, y);
        p.release();
        assert_eq!(p.selection(), Some((1, 4)));
        p.char('F');
        assert_eq!(p.hex, "3F".to_string() + &"3B9EFF"[4..]);
        assert_eq!(p.caret, 2);
    }

    #[test]
    fn backspace_and_delete_work_around_the_cursor() {
        let mut p = picker();
        let (x, y) = hex_x(3);
        p.press(W, H, x, y);
        p.release();
        p.key(VK_BACK, false, false);
        assert_eq!((p.hex.as_str(), p.caret), ("3BEFF", 2));
        p.key(VK_DELETE, false, false);
        assert_eq!(p.hex, "3BFF");
    }

    #[test]
    fn ctrl_a_selects_all_and_the_arrows_move_the_color_when_the_box_is_not_focused() {
        let mut p = picker();
        let (x, y) = hex_x(2);
        p.press(W, H, x, y);
        p.release();
        p.key(Key(0x41), false, true);
        assert_eq!(p.selection(), Some((0, 6)));
        // Un clic en el cuadrado le saca el foco a la caja: las flechas mueven el color.
        let l = layout(W, H);
        p.press(W, H, (l.square.left + l.square.right) / 2.0, (l.square.top + l.square.bottom) / 2.0);
        p.release();
        let before = p.s;
        p.key(VK_LEFT, false, false);
        assert!(p.s < before);
    }

    #[test]
    fn typing_never_goes_past_six_digits() {
        let mut p = picker();
        p.key(VK_BACK, false, false);
        for c in "12AB3FFF".chars() {
            p.char(c);
        }
        assert_eq!(p.hex, "12AB3F");
        assert_eq!(p.color(), 0x12AB3F);
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    #[test]
    fn copy_gives_the_selection_or_the_whole_color() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        assert_eq!(p.copy_text(), "3B9EFF", "abre con todo seleccionado");
        p.key(VK_END, false, false);
        assert_eq!(p.copy_text(), "#3B9EFF", "sin selección copia el color con su #");
        p.key(VK_LEFT, true, false);
        p.key(VK_LEFT, true, false);
        assert_eq!(p.copy_text(), "FF");
    }

    #[test]
    fn cut_removes_the_selection() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        assert_eq!(p.cut_text(), "3B9EFF");
        assert_eq!(p.hex, "");
    }

    #[test]
    fn pasting_a_whole_color_replaces_everything() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        p.key(VK_END, false, false);
        p.paste(" #f85149 ");
        assert_eq!((p.hex.as_str(), p.color()), ("F85149", 0xF85149));
        p.paste("#F80");
        assert_eq!((p.hex.as_str(), p.color()), ("FF8800", 0xFF8800));
    }

    #[test]
    fn pasting_pieces_inserts_at_the_cursor_and_ignores_the_rest() {
        let mut p = ColorPicker::new(PickTarget::Accent, 0x3B9EFF);
        p.key(VK_BACK, false, false);
        p.paste("12-zz-34");
        assert_eq!(p.hex, "1234");
        p.paste("5678");
        assert_eq!(p.hex, "123456", "no pasa de seis dígitos");
    }
}

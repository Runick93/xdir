//! Diseño del treemap: reparto "squarified" de las carpetas anidadas en bloques.

use super::tree::Node;
use crate::platform::{Rect, rect};

pub struct Cell {
    pub r: Rect,
    pub node: u32,
    /// Hijo directo de la carpeta actual que contiene este bloque (a donde lleva un doble clic).
    pub top: u32,
    pub dir: bool,
    /// Profundidad de anidado: 0 para los hijos directos de la carpeta actual.
    pub depth: u8,
    /// Posición (0..1) en la rueda de colores: la hereda de su carpeta de primer nivel.
    pub hue: f32,
    /// Grupo de archivos pequeños.
    pub small: bool,
}

/// Reparte `areas` (ordenadas de mayor a menor, que suman el área de `r`) en `r` con el algoritmo
/// "squarified": filas de bloques lo más cuadrados posible.
pub fn squarify(areas: &[f64], r: Rect) -> Vec<Rect> {
    let n = areas.len();
    let mut out = Vec::with_capacity(n);
    let (mut x, mut y) = (r.left as f64, r.top as f64);
    let (mut w, mut h) = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
    let mut i = 0;
    while i < n {
        let short = w.min(h);
        if short <= 0.0 {
            out.resize(n, rect(x as f32, y as f32, x as f32, y as f32));
            break;
        }
        let mut end = i;
        let mut sum = 0.0;
        let mut best = f64::MAX;
        while end < n {
            let s = sum + areas[end];
            let worst = (short * short * areas[i] / (s * s)).max(s * s / (short * short * areas[end]));
            if worst > best {
                break;
            }
            best = worst;
            sum = s;
            end += 1;
        }
        if w >= h {
            let cw = sum / h;
            let mut yy = y;
            for &a in &areas[i..end] {
                let ch = a / cw;
                out.push(rect(x as f32, yy as f32, (x + cw) as f32, (yy + ch) as f32));
                yy += ch;
            }
            x += cw;
            w -= cw;
        } else {
            let rh = sum / w;
            let mut xx = x;
            for &a in &areas[i..end] {
                let cw = a / rh;
                out.push(rect(xx as f32, y as f32, (xx + cw) as f32, (y + rh) as f32));
                xx += cw;
            }
            y += rh;
            h -= rh;
        }
        i = end;
    }
    out
}

const MAX_CELLS: usize = 400_000;

/// Alto de la franja con el nombre de una carpeta, según su profundidad y el tamaño del bloque.
pub fn header_h(depth: u8, w: f32, h: f32) -> f32 {
    if w < 64.0 || h < 36.0 {
        0.0
    } else if depth == 0 {
        24.0
    } else {
        17.0
    }
}

/// Diseña en `r` los hijos del nodo `id` (y, anidados, los de sus carpetas) y agrega los bloques a `out`.
pub fn layout(arena: &[Node], id: u32, r: Rect, out: &mut Vec<Cell>) {
    layout_at(arena, id, r, None, 0, out);
}

fn layout_at(arena: &[Node], id: u32, r: Rect, top: Option<(u32, f32)>, depth: u8, out: &mut Vec<Cell>) {
    let Some(node) = arena.get(id as usize) else { return };
    if node.size == 0 || out.len() >= MAX_CELLS {
        return;
    }
    let mut kids: Vec<u32> = node.children.iter().copied().filter(|&c| arena[c as usize].size > 0).collect();
    // Durante el escaneo los hijos todavía no están ordenados.
    if !kids.windows(2).all(|w| arena[w[0] as usize].size >= arena[w[1] as usize].size) {
        kids.sort_unstable_by_key(|&c| std::cmp::Reverse(arena[c as usize].size));
    }
    let area = ((r.right - r.left) * (r.bottom - r.top)) as f64;
    if kids.is_empty() || area <= 0.0 {
        return;
    }
    let sum: f64 = kids.iter().map(|&c| arena[c as usize].size as f64).sum();
    let areas: Vec<f64> = kids.iter().map(|&c| arena[c as usize].size as f64 / sum * area).collect();
    // Color: cada hijo directo de la carpeta actual recibe un matiz según su posición acumulada.
    let mut acc = 0.0f64;
    for ((&c, a), cr) in kids.iter().zip(&areas).zip(squarify(&areas, r)) {
        let size = arena[c as usize].size as f64;
        let mid = ((acc + size / 2.0) / sum) as f32;
        acc += size;
        if *a < 1.0 {
            break;
        }
        let (top_id, hue) = top.unwrap_or((c, mid));
        let child = &arena[c as usize];
        let (w, h) = (cr.right - cr.left, cr.bottom - cr.top);
        if child.is_dir && w >= 10.0 && h >= 10.0 {
            out.push(Cell { r: cr, node: c, top: top_id, dir: true, depth, hue, small: false });
            let pad = if depth == 0 { 3.0 } else { 2.0 };
            let head = header_h(depth, w, h);
            layout_at(
                arena,
                c,
                rect(cr.left + pad, cr.top + pad + head, cr.right - pad, cr.bottom - pad),
                Some((top_id, hue)),
                depth + 1,
                out,
            );
        } else {
            out.push(Cell { r: cr, node: c, top: top_id, dir: false, depth, hue, small: child.small });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xtree::tree::NO_PARENT;

    fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children }
    }

    #[test]
    fn squarify_fills_the_rect() {
        let r = rect(0.0, 0.0, 40.0, 25.0);
        let out = squarify(&[600.0, 300.0, 100.0], r);
        assert_eq!(out.len(), 3);
        let total: f32 = out.iter().map(|c| (c.right - c.left) * (c.bottom - c.top)).sum();
        assert!((total - 1000.0).abs() < 0.01, "{total}");
        for c in &out {
            assert!(c.left >= -0.01 && c.top >= -0.01 && c.right <= 40.01 && c.bottom <= 25.01);
        }
    }

    #[test]
    fn layout_of_empty_folder_has_no_cells() {
        let mut out = Vec::new();
        let r = rect(0.0, 0.0, 100.0, 100.0);
        layout(&[node("r", NO_PARENT, 0, true, vec![])], 0, r, &mut out);
        assert!(out.is_empty());
        // Con tamaño cero aunque tenga hijos.
        let arena = vec![node("r", NO_PARENT, 0, true, vec![1]), node("a", 0, 5, false, vec![])];
        layout(&arena, 0, r, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn layout_nests_folders() {
        let arena = vec![
            node("r", NO_PARENT, 200, true, vec![1, 4]),
            node("A", 0, 100, true, vec![2, 3]),
            node("a1", 1, 70, false, vec![]),
            node("a2", 1, 30, false, vec![]),
            node("B", 0, 100, false, vec![]),
        ];
        let mut out = Vec::new();
        layout(&arena, 0, rect(0.0, 0.0, 400.0, 300.0), &mut out);
        assert_eq!(out.iter().filter(|c| c.dir && c.node == 1).count(), 1);
        assert_eq!(out.iter().filter(|c| !c.dir && c.top == 1).count(), 2);
        assert_eq!(out.iter().filter(|c| !c.dir && c.node == 4 && c.top == 4).count(), 1);
    }
}

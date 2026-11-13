//! Historial de tamaños de xtree: una foto por análisis completo con el tamaño de las carpetas de
//! los dos primeros niveles y de las que pesan 100 MB o más, guardada por raíz en la carpeta de
//! configuración (`xtree-historial/<clave>.bin`).

use super::tree::{NO_PARENT, Node};

/// 100 MB: desde este tamaño una carpeta entra en la foto aunque esté más abajo del segundo nivel.
pub const BIG: u64 = 100 * 1024 * 1024;
/// Niveles bajo la raíz que entran siempre (con algo adentro).
pub const LEVELS: usize = 2;
/// Tope de carpetas por foto (quedan las más pesadas): un disco con miles de carpetas grandes no
/// hace crecer el archivo sin control.
pub const MAX_ENTRIES: usize = 10_000;

/// Las carpetas de la foto de un árbol terminado: ruta relativa a la raíz (tramos unidos con `/`;
/// `""` es la raíz) y tamaño. La raíz siempre; después las carpetas con algo adentro de los dos
/// primeros niveles y las de 100 MB o más. Nunca archivos ni el grupo de archivos chicos.
pub fn select(arena: &[Node]) -> Vec<(String, u64)> {
    let Some(root) = arena.first().filter(|n| n.parent == NO_PARENT) else { return Vec::new() };
    let mut out = vec![(String::new(), root.size)];
    // (nodo, nivel, ruta). Debajo del segundo nivel solo se baja por carpetas grandes: lo de
    // adentro de una carpeta chica es más chico todavía.
    let mut stack: Vec<(u32, usize, String)> = vec![(0, 0, String::new())];
    while let Some((id, depth, path)) = stack.pop() {
        let Some(n) = arena.get(id as usize) else { continue };
        for &c in &n.children {
            let Some(k) = arena.get(c as usize) else { continue };
            if !k.is_dir || k.small || k.size == 0 {
                continue;
            }
            let d = depth + 1;
            if d > LEVELS && k.size < BIG {
                continue;
            }
            let p = if path.is_empty() { k.name.to_string() } else { format!("{path}/{}", k.name) };
            out.push((p.clone(), k.size));
            stack.push((c, d, p));
        }
    }
    if out.len() > MAX_ENTRIES {
        out[1..].sort_unstable_by(|a, b| b.1.cmp(&a.1));
        out.truncate(MAX_ENTRIES);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children }
    }

    const MB: u64 = 1024 * 1024;

    /// raíz ─ a ─ b ─ c (200 MB) ─ d (150 MB) ─ e (50 MB)
    ///      │       └ chica (1 MB)
    ///      ├ vacia (0)
    ///      └ f.bin (archivo)
    fn sample() -> Vec<Node> {
        let mut t = vec![
            node("/r", NO_PARENT, 300 * MB, true, vec![1, 6, 7]),
            node("a", 0, 290 * MB, true, vec![2]),
            node("b", 1, 290 * MB, true, vec![3, 5]),
            node("c", 2, 200 * MB, true, vec![4]),
            node("d", 3, 150 * MB, true, vec![8]),
            node("chica", 2, MB, true, vec![]),
            node("vacia", 0, 0, true, vec![]),
            node("f.bin", 0, 10 * MB, false, vec![]),
            node("e", 4, 50 * MB, true, vec![9]),
            node("… 3 archivos pequeños", 8, 50 * MB, false, vec![]),
        ];
        t[9].small = true;
        t
    }

    #[test]
    fn select_two_levels_or_big() {
        let mut got = select(&sample());
        got.sort();
        let want: Vec<(String, u64)> = vec![
            ("".into(), 300 * MB),
            ("a".into(), 290 * MB),
            ("a/b".into(), 290 * MB),
            ("a/b/c".into(), 200 * MB),
            ("a/b/c/d".into(), 150 * MB),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn select_keeps_root_and_biggest_when_too_many() {
        let n = MAX_ENTRIES + 50;
        let mut t = vec![node("/r", NO_PARENT, 0, true, (1..=n as u32).collect())];
        for i in 0..n {
            t.push(node(&format!("d{i}"), 0, i as u64 + 1, true, vec![]));
        }
        let got = select(&t);
        assert_eq!(got.len(), MAX_ENTRIES);
        assert!(got.iter().any(|(p, _)| p.is_empty()), "la raíz queda");
        assert!(got.iter().all(|(p, s)| p.is_empty() || *s > 51), "quedan las más pesadas");
    }

    #[test]
    fn select_empty_arena() {
        assert!(select(&[]).is_empty());
    }
}

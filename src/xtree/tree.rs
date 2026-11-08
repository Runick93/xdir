//! Árbol de carpetas y archivos en una arena (`Vec<Node>`) y su serialización binaria.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    pub size: u64,
    pub is_dir: bool,
    /// Grupo "… N archivos pequeños": suma de los archivos que no entran como nodos propios.
    pub small: bool,
    pub children: Vec<u32>,
}

pub const NO_PARENT: u32 = u32::MAX;

/// Árbol compartido entre el escáner (que escribe) y la vista (que lee).
pub type Tree = Arc<RwLock<Vec<Node>>>;

/// Ordena los hijos de cada nodo de mayor a menor tamaño.
pub fn sort_children(arena: &mut [Node]) {
    for id in 0..arena.len() {
        let mut kids = std::mem::take(&mut arena[id].children);
        kids.sort_unstable_by_key(|&c| std::cmp::Reverse(arena[c as usize].size));
        arena[id].children = kids;
    }
}

/// Filas de la tabla: `None` es "subir"; el resto son los hijos de `current`, de mayor a menor.
pub fn rows_of(arena: &[Node], current: u32) -> Vec<Option<u32>> {
    let Some(cur) = arena.get(current as usize) else { return Vec::new() };
    let mut rows = Vec::new();
    if cur.parent != NO_PARENT {
        rows.push(None);
    }
    let mut kids: Vec<u32> = cur.children.iter().copied().filter(|&c| arena[c as usize].size > 0).collect();
    // Durante el escaneo los hijos todavía no están ordenados.
    if !kids.windows(2).all(|w| arena[w[0] as usize].size >= arena[w[1] as usize].size) {
        kids.sort_unstable_by_key(|&c| std::cmp::Reverse(arena[c as usize].size));
    }
    rows.extend(kids.into_iter().map(Some));
    rows
}

/// Ruta real de un nodo: se arma desde `root` con los nombres de sus ancestros (el nombre del
/// nodo raíz es solo para mostrar).
pub fn node_path(arena: &[Node], root: &Path, mut id: u32) -> PathBuf {
    let mut parts = Vec::new();
    while let Some(n) = arena.get(id as usize) {
        if n.parent == NO_PARENT {
            break;
        }
        parts.push(&*n.name);
        id = n.parent;
    }
    let mut p = root.to_path_buf();
    for name in parts.into_iter().rev() {
        p.push(name);
    }
    p
}

/// Serializa el árbol (little endian): `u32` cantidad y, por nodo, `u32` largo + nombre, `u32` padre,
/// `u64` tamaño, `u8` banderas (dir = 1, small = 2) y `u32` cantidad de hijos + `u32` por hijo.
pub fn encode(arena: &[Node]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((arena.len() as u32).to_le_bytes());
    for n in arena {
        out.extend((n.name.len() as u32).to_le_bytes());
        out.extend(n.name.as_bytes());
        out.extend(n.parent.to_le_bytes());
        out.extend(n.size.to_le_bytes());
        out.push(n.is_dir as u8 | (n.small as u8) << 1);
        out.extend((n.children.len() as u32).to_le_bytes());
        for c in &n.children {
            out.extend(c.to_le_bytes());
        }
    }
    out
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

/// Inverso de `encode`. Devuelve `None` ante cualquier lectura fuera de rango o dato inválido.
pub fn decode(bytes: &[u8]) -> Option<Vec<Node>> {
    let mut r = Reader(bytes);
    let count = r.u32()? as usize;
    // Cada nodo ocupa al menos 21 bytes: evita reservar de más con una cantidad falsa.
    let mut arena = Vec::with_capacity(count.min(r.0.len() / 21));
    for _ in 0..count {
        let len = r.u32()? as usize;
        let name = std::str::from_utf8(r.take(len)?).ok()?.into();
        let parent = r.u32()?;
        let size = r.u64()?;
        let flags = *r.take(1)?.first()?;
        let n = r.u32()? as usize;
        let mut children = Vec::with_capacity(n.min(r.0.len() / 4));
        for _ in 0..n {
            children.push(r.u32()?);
        }
        arena.push(Node { name, parent, size, is_dir: flags & 1 != 0, small: flags & 2 != 0, children });
    }
    Some(arena)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children }
    }

    #[test]
    fn rows_of_puts_up_first_and_sorts_by_size() {
        let arena = vec![
            node("r", NO_PARENT, 60, true, vec![1]),
            node("d", 0, 60, true, vec![2, 3, 4, 5]),
            node("menor", 1, 10, false, vec![]),
            node("mayor", 1, 30, false, vec![]),
            node("medio", 1, 20, false, vec![]),
            node("cero", 1, 0, false, vec![]),
        ];
        assert_eq!(rows_of(&arena, 1), vec![None, Some(3), Some(4), Some(2)]);
        assert_eq!(rows_of(&arena, 0), vec![Some(1)]);
    }

    #[test]
    fn node_path_joins_from_root() {
        let arena = vec![
            node("raiz", NO_PARENT, 1, true, vec![1]),
            node("a", 0, 1, true, vec![2]),
            node("b", 1, 1, false, vec![]),
        ];
        let root = PathBuf::from(if cfg!(windows) { r"C:\r" } else { "/tmp/r" });
        assert_eq!(node_path(&arena, &root, 2), root.join("a").join("b"));
        assert_eq!(node_path(&arena, &root, 0), root);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let mut t = vec![
            node("raíz", NO_PARENT, 600, true, vec![1, 2]),
            node("a", 0, 500, true, vec![3, 4]),
            node("b.txt", 0, 100, false, vec![]),
            node("c", 1, 300, false, vec![]),
            node("… 7 archivos pequeños", 1, 200, false, vec![]),
        ];
        t[4].small = true;
        assert_eq!(decode(&encode(&t)), Some(t));
        assert_eq!(decode(&[1, 2, 3]), None);
        let bytes = encode(&[node("x", NO_PARENT, 1, false, vec![])]);
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
    }

    #[test]
    fn sort_children_orders_by_size_desc() {
        let mut t = vec![
            node("r", NO_PARENT, 6, true, vec![1, 2, 3]),
            node("a", 0, 1, false, vec![]),
            node("b", 0, 3, false, vec![]),
            node("c", 0, 2, false, vec![]),
        ];
        sort_children(&mut t);
        assert_eq!(t[0].children, vec![2, 3, 1]);
    }
}

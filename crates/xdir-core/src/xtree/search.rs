//! Búsqueda por nombre en el árbol de xtree (#57): todo lo analizado, no solo el nivel a la vista,
//! con lo que ya está en memoria (sin leer el disco). Sin mayúsculas ni tildes; carpetas primero y
//! después por tamaño; hasta `MAX_RESULTS` y el total. Con un árbol grande la `Xtree` la corre en
//! otro hilo (`WM_APP_XTREE_FOUND`) y la corta si cambia lo escrito (`stop`).

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::tree::{NO_PARENT, Node, node_path};

/// Terminó una búsqueda en otro hilo.
pub const WM_APP_XTREE_FOUND: u32 = crate::WM_APP + 44;
/// Resultados que se muestran como mucho; del resto se dice cuántos hay.
pub const MAX_RESULTS: usize = 200;
/// Desde cuántos nodos la búsqueda va en otro hilo (con menos, en el momento).
pub const THREAD_FROM: usize = 20_000;
/// Nodos que se leen por vez con el bloqueo del árbol en vivo tomado (entre tandas el escáner
/// sigue escribiendo).
const CHUNK: usize = 65_536;

/// Un resultado: el nodo, su nombre, la carpeta que lo contiene relativa a lo analizado (vacía:
/// la raíz) y su tamaño.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub node: u32,
    pub name: String,
    pub rel: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

/// Lo encontrado para `query`: los primeros `MAX_RESULTS` en orden y cuántos coinciden en total.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub query: String,
    pub hits: Vec<Hit>,
    pub total: usize,
}

/// ¿El nombre contiene lo buscado (`query` ya pasado por `palette::fold`)? Sin armar cadenas para
/// los nombres ASCII (casi todos), así un árbol de millones de nodos se recorre rápido.
fn matches(name: &str, query: &str) -> bool {
    if name.is_ascii() && query.is_ascii() {
        let (n, q) = (name.as_bytes(), query.as_bytes());
        return q.is_empty() || n.windows(q.len()).any(|w| w.eq_ignore_ascii_case(q));
    }
    crate::palette::fold(name).contains(query)
}

/// Los nodos de `from..to` que coinciden (sin la raíz ni los grupos de archivos chicos), a `out`.
/// `false` si se cortó.
fn collect(arena: &[Node], query: &str, from: usize, to: usize, out: &mut Vec<u32>, stop: &dyn Fn() -> bool) -> bool {
    for (i, n) in arena.iter().enumerate().take(to).skip(from) {
        if i % 4096 == 0 && stop() {
            return false;
        }
        if n.parent != NO_PARENT && !n.small && matches(&n.name, query) {
            out.push(i as u32);
        }
    }
    true
}

/// Ordena lo encontrado (carpetas primero, después de mayor a menor, y por nombre) y arma los
/// primeros `MAX_RESULTS` con su ruta.
fn finish(arena: &[Node], query: &str, mut ids: Vec<u32>) -> Found {
    // Solo ids de este árbol (nunca se indexa fuera).
    ids.retain(|&id| (id as usize) < arena.len());
    let key = |&id: &u32| {
        let n = &arena[id as usize];
        (!n.is_dir, std::cmp::Reverse(n.size), id)
    };
    let total = ids.len();
    if ids.len() > MAX_RESULTS {
        ids.select_nth_unstable_by_key(MAX_RESULTS - 1, key);
        ids.truncate(MAX_RESULTS);
    }
    ids.sort_unstable_by_key(key);
    let hits = ids
        .into_iter()
        .map(|id| {
            let n = &arena[id as usize];
            Hit { node: id, name: n.name.to_string(), rel: node_path(arena, Path::new(""), n.parent), size: n.size, is_dir: n.is_dir }
        })
        .collect();
    Found { query: query.to_string(), hits, total }
}

/// Busca `query` en un árbol terminado. `None` si `stop` cortó la búsqueda (cambió lo escrito).
pub fn search(arena: &[Node], query: &str, stop: &dyn Fn() -> bool) -> Option<Found> {
    let q = crate::palette::fold(query.trim());
    let mut ids = Vec::new();
    collect(arena, &q, 0, arena.len(), &mut ids, stop).then(|| finish(arena, query, ids))
}

/// Busca `query` en el árbol que el escáner todavía está llenando: por tandas de `CHUNK` nodos,
/// soltando el bloqueo entre una y otra (los nodos solo se agregan al final).
pub fn search_live(tree: &RwLock<Vec<Node>>, query: &str, stop: &dyn Fn() -> bool) -> Option<Found> {
    search_live_with(tree, query, stop, &mut || {})
}

/// `search_live` con algo que corre entre una tanda y la siguiente (con el bloqueo suelto): las
/// pruebas cambian ahí el árbol, como el escáner.
fn search_live_with(tree: &RwLock<Vec<Node>>, query: &str, stop: &dyn Fn() -> bool, between: &mut dyn FnMut()) -> Option<Found> {
    let q = crate::palette::fold(query.trim());
    let mut ids = Vec::new();
    let mut from = 0;
    loop {
        let arena = tree.read().ok()?;
        // El escáner solo agrega nodos al final; si el árbol se achicó, terminó (lo vació al
        // entregar el terminado) o es otro: lo juntado ya no vale. La búsqueda se repite sobre el
        // árbol terminado (`Xtree::refresh_search`).
        if arena.len() < from {
            return None;
        }
        let to = (from + CHUNK).min(arena.len());
        if !collect(&arena, &q, from, to, &mut ids, stop) {
            return None;
        }
        if to == arena.len() {
            return Some(finish(&arena, query, ids));
        }
        from = to;
        drop(arena);
        between();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: u32, size: u64, is_dir: bool) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children: Vec::new() }
    }

    /// raiz › Documentos › Facturas (carpeta) y factura-1.pdf; raiz › FACTURAS.zip; un grupo de
    /// archivos chicos que dice «factura».
    fn sample() -> Vec<Node> {
        let mut a = vec![
            node("raiz-factura", NO_PARENT, 1000, true),
            node("Documentos", 0, 600, true),
            node("Facturas", 1, 300, true),
            node("factura-1.pdf", 2, 200, false),
            node("FACTURAS.zip", 0, 350, false),
            node("Fotos", 0, 50, true),
            node("Fáctura vieja", 5, 10, true),
        ];
        let mut small = node("12 archivos factura", 1, 5, false);
        small.small = true;
        a.push(small);
        a[0].children = vec![1, 4, 5];
        a[1].children = vec![2, 7];
        a[2].children = vec![3];
        a[5].children = vec![6];
        a
    }

    #[test]
    fn search_finds_by_name_folders_first() {
        let a = sample();
        let f = search(&a, "factura", &|| false).unwrap();
        let names: Vec<&str> = f.hits.iter().map(|h| h.name.as_str()).collect();
        // Carpetas primero (por tamaño), después archivos (por tamaño); sin tildes ni mayúsculas;
        // ni la raíz ni el grupo de archivos chicos.
        assert_eq!(names, ["Facturas", "Fáctura vieja", "FACTURAS.zip", "factura-1.pdf"]);
        assert_eq!(f.total, 4);
        assert_eq!(f.query, "factura");
        assert_eq!(f.hits[0].rel, Path::new("Documentos"));
        assert_eq!(f.hits[3].rel, Path::new("Documentos").join("Facturas"));
        assert_eq!(f.hits[2].rel, PathBuf::new(), "en la raíz");
        assert_eq!((f.hits[0].node, f.hits[0].size, f.hits[0].is_dir), (2, 300, true));
        assert!(search(&a, "nada-asi", &|| false).unwrap().hits.is_empty());
        assert_eq!(search(&a, "  FOTOS ", &|| false).unwrap().hits.len(), 1, "sin espacios a los costados");
    }

    /// M2: un nombre con el acento descompuesto (NFD) se encuentra sin tilde.
    #[test]
    fn search_finds_decomposed_names() {
        let mut a = vec![node("raiz", NO_PARENT, 10, true), node("A\u{301}rbol", 0, 10, true)];
        a[0].children = vec![1];
        assert_eq!(search(&a, "arb", &|| false).unwrap().total, 1);
        assert_eq!(search(&a, "árb", &|| false).unwrap().total, 1);
    }

    #[test]
    fn search_caps_results_and_counts_all() {
        let mut a = vec![node("raiz", NO_PARENT, 0, true)];
        for i in 0..(MAX_RESULTS + 57) {
            a.push(node(&format!("archivo-{i}.txt"), 0, i as u64, false));
        }
        let f = search(&a, "ARCHIVO", &|| false).unwrap();
        assert_eq!(f.total, MAX_RESULTS + 57);
        assert_eq!(f.hits.len(), MAX_RESULTS);
        // Los más grandes, en orden.
        assert_eq!(f.hits[0].name, format!("archivo-{}.txt", MAX_RESULTS + 56));
        assert!(f.hits.windows(2).all(|w| w[0].size >= w[1].size));
    }

    #[test]
    fn search_stops_when_asked() {
        let mut a = vec![node("raiz", NO_PARENT, 0, true)];
        a.extend((0..10_000).map(|i| node(&format!("n{i}"), 0, 1, false)));
        assert_eq!(search(&a, "n", &|| true), None);
        let live = RwLock::new(a);
        assert_eq!(search_live(&live, "n", &|| true), None);
    }

    /// C1: el escáner termina (vacía el árbol en vivo) entre dos tandas: la búsqueda se descarta
    /// (`None`) en lugar de leer nodos que ya no están.
    #[test]
    fn search_live_tree_taken_between_chunks() {
        let mut a = vec![node("raiz", NO_PARENT, 0, true)];
        a.extend((0..(CHUNK * 2)).map(|i| node(if i < 10 { "objetivo" } else { "otro" }, 0, 1, false)));
        let live = RwLock::new(a);
        let mut first = true;
        let r = search_live_with(&live, "objetivo", &|| false, &mut || {
            if std::mem::take(&mut first) {
                let _ = std::mem::take(&mut *live.write().unwrap());
            }
        });
        assert_eq!(r, None);
        // Y si el árbol cambió por otro más chico (otro escaneo), también.
        let mut b = vec![node("raiz", NO_PARENT, 0, true)];
        b.extend((0..(CHUNK + 5)).map(|_| node("objetivo", 0, 1, false)));
        let live = RwLock::new(b);
        let mut first = true;
        let r = search_live_with(&live, "objetivo", &|| false, &mut || {
            if std::mem::take(&mut first) {
                *live.write().unwrap() = vec![node("raiz", NO_PARENT, 0, true), node("x", 0, 1, false)];
            }
        });
        assert_eq!(r, None);
    }

    #[test]
    fn search_live_reads_in_chunks() {
        let mut a = vec![node("raiz", NO_PARENT, 0, true)];
        a.extend((0..(CHUNK * 2 + 10)).map(|i| node(if i % 1000 == 0 { "objetivo" } else { "otro" }, 0, i as u64, false)));
        let live = RwLock::new(a.clone());
        let f = search_live(&live, "objetivo", &|| false).unwrap();
        assert_eq!(f, search(&a, "objetivo", &|| false).unwrap());
        assert_eq!(f.total, (CHUNK * 2 + 10).div_ceil(1000));
    }
}

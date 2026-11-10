//! Claves de las miniaturas: qué elemento y de qué tamaño. La carga vive en cada plataforma
//! (`xdir::thumbs`). También los escalones de la imagen grande (vista previa, Galería, vista
//! rápida) y su caché (`BigCache`), que usa cada renderizador.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// De qué elemento: una ruta de disco o un PIDL absoluto (elementos virtuales del Shell).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Path(PathBuf),
    Pidl(Arc<[u8]>),
}

/// (elemento, lado en píxeles).
pub type Key = (Source, u32);

/// Escalones del lado de la imagen grande, en píxeles físicos.
const BIG_STEPS: [u32; 4] = [256, 512, 1024, 2048];

/// Tope de la caché de imágenes grandes de cada renderizador.
pub const BIG_CACHE_BYTES: usize = 64 << 20;

/// Lado que se pide para mostrar una imagen grande de `px` píxeles: el menor escalón que alcanza
/// (tope 2048). Así cambiar el ancho de la columna o el DPI no pide una imagen por cada píxel.
pub fn big_side(px: u32) -> u32 {
    BIG_STEPS.into_iter().find(|&s| s >= px).unwrap_or(BIG_STEPS[BIG_STEPS.len() - 1])
}

/// Caché de imágenes grandes, aparte de la de la lista: se expulsan las menos usadas cuando la
/// suma de bytes pasa el tope.
pub struct BigCache<V> {
    cap: usize,
    used: usize,
    /// Contador de usos: el valor más chico es el menos usado.
    tick: u64,
    items: HashMap<Key, (V, usize, u64)>,
}

impl<V> Default for BigCache<V> {
    /// Con el tope de fábrica (`BIG_CACHE_BYTES`).
    fn default() -> Self {
        Self::new(BIG_CACHE_BYTES)
    }
}

impl<V> BigCache<V> {
    pub fn new(cap_bytes: usize) -> Self {
        Self { cap: cap_bytes, used: 0, tick: 0, items: HashMap::new() }
    }

    fn touch(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// La imagen de `key`, si está (cuenta como uso).
    pub fn get(&mut self, key: &Key) -> Option<&V> {
        let t = self.touch();
        let item = self.items.get_mut(key)?;
        item.2 = t;
        Some(&item.0)
    }

    /// Guarda `v` (de `bytes` bytes) y expulsa las menos usadas hasta entrar en el tope. Una sola
    /// más grande que el tope igual queda (sola).
    pub fn insert(&mut self, key: Key, v: V, bytes: usize) {
        if let Some((_, old, _)) = self.items.remove(&key) {
            self.used -= old;
        }
        while self.used + bytes > self.cap {
            let Some(oldest) = self.items.iter().min_by_key(|(_, (_, _, t))| *t).map(|(k, _)| k.clone()) else { break };
            if let Some((_, b, _)) = self.items.remove(&oldest) {
                self.used -= b;
            }
        }
        let t = self.touch();
        self.used += bytes;
        self.items.insert(key, (v, bytes, t));
    }

    /// El escalón más grande que haya de `src` (para dibujarlo escalado mientras llega el justo).
    pub fn best_for(&mut self, src: &Source) -> Option<&V> {
        let t = self.touch();
        let item = self.items.iter_mut().filter(|((s, _), _)| s == src).max_by_key(|((_, side), _)| *side).map(|(_, v)| v)?;
        item.2 = t;
        Some(&item.0)
    }

    /// Vacía la caché (se perdió el dispositivo de dibujo).
    pub fn clear(&mut self) {
        self.items.clear();
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, side: u32) -> Key {
        (Source::Path(PathBuf::from(name)), side)
    }

    /// El lado pedido se redondea al escalón de arriba (256, 512, 1024, 2048), con tope 2048.
    #[test]
    fn big_side_steps() {
        for (px, want) in [(100, 256), (256, 256), (257, 512), (900, 1024), (1500, 2048), (5000, 2048)] {
            assert_eq!(big_side(px), want, "{px}");
        }
    }

    /// Tope de 100 bytes y tres de 40: sale la menos usada (`get` la renueva).
    #[test]
    fn big_cache_evicts_by_bytes() {
        let mut c = BigCache::new(100);
        c.insert(key("a", 256), 'a', 40);
        c.insert(key("b", 256), 'b', 40);
        assert_eq!(c.get(&key("a", 256)), Some(&'a'));
        c.insert(key("c", 256), 'c', 40);
        assert_eq!(c.get(&key("b", 256)), None, "b era la menos usada");
        assert_eq!(c.get(&key("a", 256)), Some(&'a'));
        assert_eq!(c.get(&key("c", 256)), Some(&'c'));
        // Reemplazar una clave no cuenta sus bytes dos veces.
        c.insert(key("c", 256), 'C', 40);
        assert_eq!(c.get(&key("a", 256)), Some(&'a'));
        assert_eq!(c.get(&key("c", 256)), Some(&'C'));
    }

    /// `best_for` da el escalón más grande que haya de esa fuente.
    #[test]
    fn big_cache_best_for_largest_step() {
        let mut c = BigCache::new(BIG_CACHE_BYTES);
        assert_eq!(c.best_for(&Source::Path(PathBuf::from("a"))), None);
        c.insert(key("a", 512), 512, 1);
        c.insert(key("a", 256), 256, 1);
        c.insert(key("a", 1024), 1024, 1);
        c.insert(key("b", 2048), 2048, 1);
        assert_eq!(c.best_for(&Source::Path(PathBuf::from("a"))), Some(&1024));
        assert_eq!(c.best_for(&Source::Path(PathBuf::from("b"))), Some(&2048));
        assert_eq!(c.best_for(&Source::Path(PathBuf::from("z"))), None);
    }
}

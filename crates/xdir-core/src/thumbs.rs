//! Claves de las miniaturas: qué elemento y de qué tamaño. La carga vive en cada plataforma
//! (`xdir::thumbs`). También los escalones de la imagen grande (vista previa, Galería, vista
//! rápida) y su caché (`BigCache`), que usa cada renderizador.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
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

    /// Olvida todos los escalones de `src` (el archivo cambió: la próxima vez se vuelve a pedir).
    pub fn forget(&mut self, src: &Source) {
        let used = &mut self.used;
        self.items.retain(|(s, _), (_, b, _)| {
            let keep = s != src;
            if !keep {
                *used -= *b;
            }
            keep
        });
    }

    /// Vacía la caché (se perdió el dispositivo de dibujo).
    pub fn clear(&mut self) {
        self.items.clear();
        self.used = 0;
    }

    /// Lo que se dibuja para `key` y si hay que pedirlo: el justo (sin pedir) o, si no está, el
    /// escalón más grande que haya con su lado. Se pide salvo que ese ya sea igual o más grande
    /// que el pedido o que `key` haya fallado hace menos de `BIG_RETRY`.
    pub fn lookup(&mut self, key: &Key, failures: &mut BigFailures, now: Instant) -> (Option<(u32, &V)>, bool) {
        let t = self.touch();
        let best = if self.items.contains_key(key) {
            Some(key.clone())
        } else {
            self.items.keys().filter(|(s, _)| *s == key.0).max_by_key(|(_, side)| *side).cloned()
        };
        let ask = best.as_ref().is_none_or(|(_, side)| *side < key.1) && !failures.blocked(key, now);
        let hit = best.and_then(|k| {
            let item = self.items.get_mut(&k)?;
            item.2 = t;
            Some((k.1, &item.0))
        });
        (hit, ask)
    }
}

/// Cuánto se espera para volver a pedir una imagen grande que falló (un archivo que se estaba
/// copiando, uno de la nube que ya se bajó, un error pasajero del sistema).
pub const BIG_RETRY: Duration = Duration::from_secs(10);

/// Imágenes grandes que fallaron y cuándo: no se vuelven a pedir hasta `BIG_RETRY` después.
#[derive(Default)]
pub struct BigFailures {
    at: HashMap<Key, Instant>,
}

impl BigFailures {
    /// Anota que `key` falló en `now`.
    pub fn fail(&mut self, key: Key, now: Instant) {
        if self.at.len() > 256 {
            self.at.retain(|_, t| now.saturating_duration_since(*t) <= BIG_RETRY);
        }
        self.at.insert(key, now);
    }

    /// ¿`key` falló hace menos de `BIG_RETRY`? Si ya venció, la olvida.
    pub fn blocked(&mut self, key: &Key, now: Instant) -> bool {
        match self.at.get(key) {
            Some(t) if now.saturating_duration_since(*t) <= BIG_RETRY => true,
            Some(_) => {
                self.at.remove(key);
                false
            }
            None => false,
        }
    }
}

/// Factor para dibujar una imagen de escalón `side` (lado mayor `img_max` píxeles) en lugar de la
/// del escalón `want`: `want / side` si llenaba su escalón (se ve del tamaño que tendrá la justa);
/// 1 si es más chica que su escalón (su tamaño real, el mismo que tendrá la justa).
pub fn big_grow(img_max: f32, side: u32, want: u32) -> f32 {
    if side == 0 || img_max < side as f32 * 0.9 { 1.0 } else { want as f32 / side as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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
    fn big_cache_forget_source() {
        let mut c = BigCache::new(1000);
        c.insert(key("a", 256), 1, 10);
        c.insert(key("a", 1024), 2, 40);
        c.insert(key("b", 256), 3, 10);
        c.forget(&Source::Path(PathBuf::from("a")));
        assert_eq!(c.best_for(&Source::Path(PathBuf::from("a"))), None);
        assert_eq!(c.get(&key("b", 256)), Some(&3));
        assert_eq!(c.used, 10);
    }

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

    /// Una imagen grande que falló no se vuelve a pedir enseguida (si no, pedir → falla →
    /// pintar → pedir sería un bucle), pero sí pasado `BIG_RETRY`.
    #[test]
    fn big_failure_retried_after_ttl() {
        let t0 = Instant::now();
        let mut f = BigFailures::default();
        let k = key("a", 512);
        assert!(!f.blocked(&k, t0));
        f.fail(k.clone(), t0);
        assert!(f.blocked(&k, t0 + Duration::from_secs(1)));
        assert!(!f.blocked(&key("a", 256), t0), "otro escalón no");
        assert!(!f.blocked(&k, t0 + BIG_RETRY + Duration::from_millis(1)), "vencido: se reintenta");
        assert!(!f.blocked(&k, t0), "y se olvidó");
    }

    /// Buscar: el justo sin pedir; si no, pide y da el más grande que haya; uno más grande que
    /// el escalón ya alcanza (no se pide el chico); una falla reciente no se pide.
    #[test]
    fn big_lookup_requests_only_when_needed() {
        let t0 = Instant::now();
        let mut c = BigCache::new(BIG_CACHE_BYTES);
        let mut f = BigFailures::default();
        let (hit, ask) = c.lookup(&key("a", 512), &mut f, t0);
        assert_eq!((hit, ask), (None, true));
        c.insert(key("a", 256), 'p', 1);
        assert_eq!(c.lookup(&key("a", 512), &mut f, t0), (Some((256, &'p')), true));
        c.insert(key("a", 512), 'm', 1);
        assert_eq!(c.lookup(&key("a", 512), &mut f, t0), (Some((512, &'m')), false));
        c.insert(key("c", 1024), 'g', 1);
        assert_eq!(c.lookup(&key("c", 512), &mut f, t0), (Some((1024, &'g')), false), "el grande alcanza");
        f.fail(key("b", 512), t0);
        assert_eq!(c.lookup(&key("b", 512), &mut f, t0), (None, false));
        assert_eq!(c.lookup(&key("b", 512), &mut f, t0 + BIG_RETRY * 2), (None, true));
    }

    /// El escalón provisorio se agranda como se vería el justo solo si llenaba su escalón; una
    /// imagen más chica que su escalón queda en su tamaño (no salta al llegar la justa).
    #[test]
    fn big_grow_only_when_step_filled() {
        assert_eq!(big_grow(256.0, 256, 512), 2.0);
        assert_eq!(big_grow(240.0, 256, 1024), 4.0);
        assert_eq!(big_grow(100.0, 256, 512), 1.0);
        assert_eq!(big_grow(512.0, 512, 512), 1.0);
        assert_eq!(big_grow(1024.0, 1024, 512), 0.5, "uno más grande se ve como el justo");
        assert_eq!(big_grow(100.0, 1024, 512), 1.0);
    }
}

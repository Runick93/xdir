//! Búsqueda al teclear: la de la lista de archivos y la de la tabla de xtree. Las letras que llegan
//! seguidas (menos de `TYPEAHEAD_MS` entre una y otra) se juntan («te» → «temp»); la misma letra
//! repetida («ttt») recorre los que empiezan con esa letra.

use std::time::{Duration, Instant};

/// Lo escrito sigue el mismo nombre si la tecla siguiente llega antes de esto.
pub const TYPEAHEAD_MS: u64 = 900;

/// Lo escrito hasta ahora y cuándo llegó la última letra.
#[derive(Default, Debug)]
pub struct TypeAhead {
    text: String,
    at: Option<Instant>,
}

/// Qué buscar después de una letra.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    /// En minúsculas: lo escrito o, si es una sola letra repetida, esa letra.
    pub prefix: String,
    /// Buscar desde el elemento siguiente al actual (una búsqueda nueva o la letra repetida); si
    /// no, desde el actual (se sigue escribiendo el mismo nombre).
    pub next: bool,
}

impl TypeAhead {
    /// Una letra escrita a la hora `now`.
    pub fn push(&mut self, c: char, now: Instant) -> Step {
        let fresh = self.at.is_none_or(|t| now.saturating_duration_since(t) > Duration::from_millis(TYPEAHEAD_MS));
        if fresh {
            self.text.clear();
        }
        self.at = Some(now);
        self.text.extend(c.to_lowercase());
        let mut chars = self.text.chars();
        let first = chars.next();
        let repeated = self.text.chars().count() > 1 && chars.all(|ch| Some(ch) == first);
        let prefix = if repeated { first.map(String::from).unwrap_or_default() } else { self.text.clone() };
        Step { prefix, next: fresh || repeated }
    }

    /// ¿Hay una búsqueda en curso a la hora `now`? (se escribió algo hace menos de `TYPEAHEAD_MS`).
    pub fn active(&self, now: Instant) -> bool {
        !self.text.is_empty() && self.at.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(TYPEAHEAD_MS))
    }

    /// Olvida lo escrito (la próxima letra empieza una búsqueda nueva).
    pub fn clear(&mut self) {
        self.text.clear();
        self.at = None;
    }
}

/// La primera posición de `0..n` que cumple `matches`, empezando en `start` y dando la vuelta.
pub fn find_from(n: usize, start: usize, mut matches: impl FnMut(usize) -> bool) -> Option<usize> {
    (0..n).map(|k| (start + k) % n).find(|&pos| matches(pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn typeahead_joins_quick_letters() {
        let t0 = Instant::now();
        let mut t = TypeAhead::default();
        assert_eq!(t.push('T', t0), Step { prefix: "t".into(), next: true });
        assert_eq!(t.push('e', at(t0, 300)), Step { prefix: "te".into(), next: false });
        assert!(t.active(at(t0, 1000)));
        assert!(!t.active(at(t0, 1300)));
        // Después de la pausa, empieza de nuevo.
        assert_eq!(t.push('w', at(t0, 1300)), Step { prefix: "w".into(), next: true });
    }

    #[test]
    fn typeahead_repeated_letter_cycles() {
        let t0 = Instant::now();
        let mut t = TypeAhead::default();
        t.push('p', t0);
        assert_eq!(t.push('P', at(t0, 100)), Step { prefix: "p".into(), next: true });
        assert_eq!(t.push('p', at(t0, 200)), Step { prefix: "p".into(), next: true });
        t.clear();
        assert!(!t.active(at(t0, 200)));
        assert_eq!(t.push('p', at(t0, 250)).next, true);
    }

    #[test]
    fn typeahead_find_wraps() {
        let names = ["alfa", "beta", "bravo", "carlos"];
        assert_eq!(find_from(4, 2, |i| names[i].starts_with('b')), Some(2));
        assert_eq!(find_from(4, 3, |i| names[i].starts_with('b')), Some(1));
        assert_eq!(find_from(4, 0, |i| names[i].starts_with('z')), None);
        assert_eq!(find_from(0, 0, |_| true), None);
    }
}

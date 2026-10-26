//! Historial de carpetas visitadas para el salto rápido (Ctrl+P), ordenado por "frecencia":
//! cuántas veces se visitó cada carpeta, pesado por cuán reciente fue la última visita (como
//! zoxide). Se guarda en `historial.txt`, junto a la sesión, una línea por carpeta:
//!
//! ```text
//! 12	1761430000	C:\Users\yo\Documents
//! ```
//!
//! (visitas, última visita en segundos Unix y ruta, separados por tabulaciones).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::session;

const FILE_NAME: &str = "historial.txt";
/// Más que esto y se descartan las de menor puntaje.
const MAX_ENTRIES: usize = 500;
/// Cuando la suma de visitas pasa este valor, todas se reducen (las viejas terminan saliendo).
const MAX_TOTAL: f64 = 5000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Visit {
    pub path: PathBuf,
    pub count: f64,
    /// Segundos Unix.
    pub last: u64,
}

#[derive(Default, Debug)]
pub struct History {
    pub visits: Vec<Visit>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Puntaje: visitas multiplicadas según la antigüedad de la última.
pub fn frecency(v: &Visit, now: u64) -> f64 {
    let age = now.saturating_sub(v.last);
    let weight = match age {
        a if a < 3600 => 4.0,
        a if a < 86_400 => 2.0,
        a if a < 7 * 86_400 => 0.5,
        _ => 0.25,
    };
    v.count * weight
}

impl History {
    pub fn load() -> Self {
        let text = session::config_dir().and_then(|d| std::fs::read_to_string(d.join(FILE_NAME)).ok());
        text.map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let visits = text
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let count = parts.next()?.trim().parse().ok()?;
                let last = parts.next()?.trim().parse().ok()?;
                let path = parts.next()?.trim();
                (!path.is_empty()).then(|| Visit { path: PathBuf::from(path), count, last })
            })
            .collect();
        Self { visits }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = session::config_dir().ok_or(std::io::ErrorKind::NotFound)?;
        std::fs::create_dir_all(&dir)?;
        let mut out = String::new();
        for v in &self.visits {
            out.push_str(&format!("{:.2}\t{}\t{}\n", v.count, v.last, v.path.display()));
        }
        std::fs::write(dir.join(FILE_NAME), out)
    }

    /// Anota una visita a la carpeta.
    pub fn record(&mut self, path: &Path, now: u64) {
        match self.visits.iter_mut().find(|v| crate::fs::same_path(&v.path, path)) {
            Some(v) => {
                v.count += 1.0;
                v.last = now;
            }
            None => self.visits.push(Visit { path: path.to_path_buf(), count: 1.0, last: now }),
        }
        let total: f64 = self.visits.iter().map(|v| v.count).sum();
        if total > MAX_TOTAL {
            for v in &mut self.visits {
                v.count *= 0.9;
            }
            self.visits.retain(|v| v.count >= 1.0);
        }
        if self.visits.len() > MAX_ENTRIES {
            self.visits.sort_by(|a, b| frecency(b, now).total_cmp(&frecency(a, now)));
            self.visits.truncate(MAX_ENTRIES);
        }
    }

    /// Saca una carpeta (ya no existe).
    pub fn forget(&mut self, path: &Path) {
        self.visits.retain(|v| !crate::fs::same_path(&v.path, path));
    }
}

/// Coincidencia de una ruta con lo escrito: cada palabra tiene que aparecer en la ruta, en orden
/// cualquiera. Devuelve un multiplicador del puntaje (más si la última palabra está en el nombre
/// de la carpeta, más todavía si es su comienzo) o `None` si no coincide.
pub fn match_path(query_words: &[String], path_lower: &str) -> Option<f64> {
    if query_words.is_empty() {
        return Some(1.0);
    }
    if !query_words.iter().all(|w| path_lower.contains(w.as_str())) {
        return None;
    }
    let name = path_lower.trim_end_matches('\\').rsplit('\\').next().unwrap_or(path_lower);
    // De la última palabra cuenta su último tramo ("yo\doc" → "doc").
    let last = query_words.last().unwrap().rsplit('\\').next().unwrap_or("");
    Some(if last.is_empty() {
        1.0
    } else if name.starts_with(last) {
        4.0
    } else if name.contains(last) {
        2.0
    } else {
        1.0
    })
}

/// Palabras de la búsqueda, en minúsculas. Las barras se toman como separadores de ruta
/// (`doc/inf` o `doc\inf` buscan las dos partes).
pub fn query_words(query: &str) -> Vec<String> {
    query.to_lowercase().replace('/', "\\").split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut h = History::default();
        h.record(Path::new(r"C:\a b\c"), 100);
        h.record(Path::new(r"C:\a b\c"), 200);
        h.record(Path::new(r"D:\x"), 150);
        let text = {
            let mut out = String::new();
            for v in &h.visits {
                out.push_str(&format!("{:.2}\t{}\t{}\n", v.count, v.last, v.path.display()));
            }
            out
        };
        let back = History::parse(&text);
        assert_eq!(back.visits, h.visits);
        assert_eq!(back.visits[0].count, 2.0);
        assert_eq!(back.visits[0].last, 200);
        assert!(History::parse("basura\n\t\t\n3\tx\tC:\\").visits.is_empty());
    }

    #[test]
    fn recent_beats_old() {
        let now = 10_000_000;
        let old = Visit { path: "a".into(), count: 10.0, last: now - 30 * 86_400 };
        let recent = Visit { path: "b".into(), count: 2.0, last: now - 60 };
        assert!(frecency(&recent, now) > frecency(&old, now));
    }

    #[test]
    fn matching() {
        let q = |s: &str| query_words(s);
        assert_eq!(match_path(&q(""), r"c:\x"), Some(1.0));
        assert_eq!(match_path(&q("doc"), r"c:\users\yo\documents"), Some(4.0));
        assert_eq!(match_path(&q("ment"), r"c:\users\yo\documents"), Some(2.0));
        assert_eq!(match_path(&q("users doc"), r"c:\users\yo\documents"), Some(4.0));
        assert_eq!(match_path(&q("users"), r"c:\users\yo\documents"), Some(1.0));
        assert_eq!(match_path(&q("xyz"), r"c:\users\yo\documents"), None);
        assert_eq!(match_path(&q("yo/doc"), r"c:\users\yo\documents"), Some(4.0));
    }
}

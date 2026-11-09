//! Formato de textos: fechas y tamaños según la configuración regional del sistema (por
//! `sys::env()`), miles y orden natural de nombres.

use std::cmp::Ordering;

/// Fecha y hora local corta del usuario, p. ej. "24/10/2026 14:05". Vacío si `ft` (un FILETIME)
/// es 0.
pub fn date(ft: u64) -> String {
    crate::sys::env().format_date(ft)
}

/// Tamaño de archivo como lo muestra el explorador del sistema: "1.234 KB" (Windows), "1,2 MB"
/// (macOS).
pub fn size_kb(bytes: u64) -> String {
    crate::sys::env().format_size_kb(bytes)
}

/// Tamaño abreviado: "476 GB", "1,2 MB".
pub fn size_short(bytes: u64) -> String {
    crate::sys::env().format_size_short(bytes)
}

/// Número con separador de miles: 12345 → "12.345".
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Orden natural como el del Explorador: "img2" < "img10". Espera textos ya en minúsculas.
///
/// Trabaja sobre los bytes UTF-8 sin reservar memoria: el orden de bytes UTF-8 coincide con el
/// de los caracteres, y un dígito ASCII nunca aparece dentro de un carácter multibyte.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (na, nb) = (&a[si..i], &b[sj..j]);
            // Sin ceros a la izquierda, el número más largo es el mayor.
            let (ta, tb) = (trim_zeros(na), trim_zeros(nb));
            let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            if a[i] != b[j] {
                return a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn trim_zeros(s: &[u8]) -> &[u8] {
    let n = s.iter().take_while(|&&c| c == b'0').count();
    &s[n..]
}

/// Comparación sin distinguir mayúsculas y sin reservar memoria.
pub fn caseless_cmp(a: &str, b: &str) -> Ordering {
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

#[cfg(test)]
mod tests {
    use super::natural_cmp;
    use std::cmp::Ordering::*;

    #[test]
    fn natural_order() {
        assert_eq!(natural_cmp("img2", "img10"), Less);
        assert_eq!(natural_cmp("img10", "img2"), Greater);
        assert_eq!(natural_cmp("a", "a"), Equal);
        assert_eq!(natural_cmp("a", "ab"), Less);
        assert_eq!(natural_cmp("file007", "file7"), Greater);
        assert_eq!(natural_cmp("v1.9", "v1.10"), Less);
        assert_eq!(natural_cmp("año2", "año10"), Less);
        assert_eq!(natural_cmp("b", "á"), Less);
    }
}

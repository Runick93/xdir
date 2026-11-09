//! Formato de textos: fechas y tamaños según la configuración regional del sistema (por
//! `sys::env()`), los cálculos puros que pueden usar los sistemas para eso (`date_time_text`,
//! `size_kb_text`, `size_short_text`), miles y orden natural de nombres.

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

/// Días entre 1601-01-01 (época de FILETIME) y 1970-01-01.
pub const DAYS_1601_TO_1970: i64 = 134_774;

/// Fecha civil (año, mes, día) de un número de días desde 1970-01-01 (algoritmo de Howard
/// Hinnant; negativo es antes de 1970). Cálculo puro, sin zona horaria.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// "AAAA-MM-DD HH:MM" de un FILETIME corrido `offset_secs` segundos respecto de UTC (la diferencia
/// de la zona horaria: -10800 es UTC-3). Vacío si `filetime` es 0. Cálculo puro: quien llama
/// consigue la diferencia de su sistema.
pub fn date_time_text(filetime: u64, offset_secs: i64) -> String {
    if filetime == 0 {
        return String::new();
    }
    let secs = (filetime / 10_000_000) as i64 + offset_secs;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days - DAYS_1601_TO_1970);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

/// Tamaño en KB como el Explorador de Windows: "1.234 KB" (redondea hacia arriba). Cálculo
/// puro, sin pasar por `sys::env()`.
pub fn size_kb_text(bytes: u64) -> String {
    format!("{} KB", thousands(bytes.div_ceil(1024)))
}

/// Tamaño abreviado como Windows: "123 bytes", "1,23 KB", "12,3 MB", "476 GB" (tres cifras,
/// truncadas). Cálculo puro, sin pasar por `sys::env()`.
pub fn size_short_text(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB", "PB", "EB"];
    let mut unit = 0;
    let mut scale = 1024u128;
    while (bytes as u128) >= scale * 1024 && unit + 1 < units.len() {
        scale *= 1024;
        unit += 1;
    }
    // Centésimas de la unidad, truncadas.
    let hundredths = bytes as u128 * 100 / scale;
    let text = match hundredths {
        0..1000 => format!("{},{:02}", hundredths / 100, hundredths % 100),
        1000..10000 => format!("{},{}", hundredths / 100, hundredths % 100 / 10),
        _ => format!("{}", hundredths / 100),
    };
    format!("{text} {}", units[unit])
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

//! Formato de fechas y tamaños según la configuración regional, y orden natural de nombres.

use std::cmp::Ordering;

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Shell::{
    SFBS_FLAGS_TRUNCATE_UNDISPLAYED_DECIMAL_DIGITS, StrFormatByteSizeEx, StrFormatKBSizeW,
};
use windows::core::PCWSTR;

/// Fecha y hora local corta del usuario, p. ej. "24/10/2026 14:05". Vacío si `ft` es 0.
pub fn date(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    let mut date = [0u16; 64];
    let mut time = [0u16; 64];
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
        {
            return String::new();
        }
        // PCWSTR nulo = configuración regional del usuario.
        let d = GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&local), PCWSTR::null(), Some(&mut date), PCWSTR::null());
        let t = GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&local), PCWSTR::null(), Some(&mut time));
        format!("{} {}", wide_str(&date, d), wide_str(&time, t))
    }
}

/// Tamaño de archivo como lo muestra el Explorador: "1.234 KB".
pub fn size_kb(bytes: u64) -> String {
    let mut buf = [0u16; 64];
    unsafe {
        StrFormatKBSizeW(bytes as i64, &mut buf);
    }
    wide_str(&buf, -1)
}

/// Tamaño abreviado: "476 GB", "1,2 MB".
pub fn size_short(bytes: u64) -> String {
    let mut buf = [0u16; 64];
    unsafe {
        if StrFormatByteSizeEx(bytes, SFBS_FLAGS_TRUNCATE_UNDISPLAYED_DECIMAL_DIGITS, &mut buf).is_err() {
            return format!("{bytes} bytes");
        }
    }
    wide_str(&buf, -1)
}

/// `len` incluye el nulo final (como devuelven las APIs); -1 = buscar el nulo.
fn wide_str(buf: &[u16], len: i32) -> String {
    let end = if len > 0 {
        (len as usize - 1).min(buf.len())
    } else {
        buf.iter().position(|&c| c == 0).unwrap_or(buf.len())
    };
    String::from_utf16_lossy(&buf[..end])
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

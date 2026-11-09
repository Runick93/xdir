//! Formato de fechas y tamaños según la configuración regional, y orden natural de nombres.

pub use xdir_core::format::{caseless_cmp, natural_cmp, thousands};

#[cfg(windows)]
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
#[cfg(windows)]
use windows::Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
#[cfg(windows)]
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    SFBS_FLAGS_TRUNCATE_UNDISPLAYED_DECIMAL_DIGITS, StrFormatByteSizeEx, StrFormatKBSizeW,
};
#[cfg(windows)]
use windows::core::PCWSTR;

#[cfg(target_os = "macos")]
pub use self::macos::{date, size_kb, size_short};

/// Fecha y hora local corta del usuario, p. ej. "24/10/2026 14:05". Vacío si `ft` es 0.
#[cfg(windows)]
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
#[cfg(windows)]
pub fn size_kb(bytes: u64) -> String {
    let mut buf = [0u16; 64];
    unsafe {
        StrFormatKBSizeW(bytes as i64, &mut buf);
    }
    wide_str(&buf, -1)
}

/// Tamaño abreviado: "476 GB", "1,2 MB".
#[cfg(windows)]
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
#[cfg(windows)]
fn wide_str(buf: &[u16], len: i32) -> String {
    let end = if len > 0 {
        (len as usize - 1).min(buf.len())
    } else {
        buf.iter().position(|&c| c == 0).unwrap_or(buf.len())
    };
    String::from_utf16_lossy(&buf[..end])
}

/// macOS: fechas con el formato corto del usuario y tamaños como el Finder.
#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2_foundation::{NSByteCountFormatter, NSByteCountFormatterCountStyle, NSDate, NSDateFormatter, NSDateFormatterStyle};

    /// Segundos entre 1601 (época de FILETIME, que usan las entradas) y 1970.
    const EPOCH_DIFF: f64 = 11_644_473_600.0;

    thread_local! {
        static DATES: Retained<NSDateFormatter> = {
            let f = NSDateFormatter::new();
            f.setDateStyle(NSDateFormatterStyle::ShortStyle);
            f.setTimeStyle(NSDateFormatterStyle::ShortStyle);
            f
        };
        static SIZES: Retained<NSByteCountFormatter> = {
            let f = NSByteCountFormatter::new();
            f.setCountStyle(NSByteCountFormatterCountStyle::File);
            f
        };
    }

    /// Fecha y hora local corta del usuario, p. ej. "24/10/26 14:05". Vacío si `ft` es 0.
    pub fn date(ft: u64) -> String {
        if ft == 0 {
            return String::new();
        }
        let secs = ft as f64 / 10_000_000.0 - EPOCH_DIFF;
        let d = NSDate::dateWithTimeIntervalSince1970(secs);
        DATES.with(|f| f.stringFromDate(&d).to_string())
    }

    /// Tamaño de archivo como lo muestra el Finder: "1,2 MB".
    pub fn size_kb(bytes: u64) -> String {
        size_short(bytes)
    }

    /// Tamaño abreviado: "476 GB", "1,2 MB".
    pub fn size_short(bytes: u64) -> String {
        SIZES.with(|f| f.stringFromByteCount(bytes as i64).to_string())
    }
}

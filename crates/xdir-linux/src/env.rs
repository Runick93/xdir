//! Entorno de Linux: carpeta de configuración según XDG, formatos neutros y el historial de rutas
//! escritas. Sin interfaz todavía: no hay tema, acento ni teclado del sistema que leer.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use xdir_core::format::thousands;
use xdir_core::keys::Key;
use xdir_core::sys::{ConsoleInfo, Handle, SystemEnv};

/// Carpeta de configuración: `$XDG_CONFIG_HOME/xdir` si es una ruta absoluta (la especificación
/// XDG manda ignorar las relativas), si no `$HOME/.config/xdir`. Recibe los valores en vez de
/// leerlos, así se prueba sin tocar las variables del proceso.
pub fn config_dir_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    if let Some(x) = xdg.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(x.join("xdir"));
    }
    home.filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".config").join("xdir"))
}

/// El historial de rutas escritas va en `rutas.txt` de la carpeta de configuración, una por línea
/// (como en macOS).
fn typed_file() -> Option<PathBuf> {
    xdir_core::session::config_dir().map(|d| d.join("rutas.txt"))
}

/// Días entre 1601-01-01 (época de FILETIME) y 1970-01-01.
const DAYS_1601_TO_1970: i64 = 134_774;

/// Fecha civil (año, mes, día) de un número de días desde 1970-01-01 (algoritmo de Howard
/// Hinnant, el mismo del entorno falso del núcleo).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
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

/// Entorno de Linux sin interfaz.
pub struct LinuxEnv;

impl SystemEnv for LinuxEnv {
    /// Sin interfaz no hay tema del sistema: claro.
    fn system_dark(&self) -> bool {
        false
    }

    fn system_accent(&self) -> Option<u32> {
        None
    }

    fn animations_allowed(&self) -> bool {
        true
    }

    /// Sin ventana no hay teclado que consultar.
    fn key_down(&self, _k: Key) -> bool {
        false
    }

    fn config_dir(&self) -> Option<PathBuf> {
        config_dir_from(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
    }

    /// "AAAA-MM-DD HH:MM" (UTC) del FILETIME; vacío si es 0.
    fn format_date(&self, filetime: u64) -> String {
        if filetime == 0 {
            return String::new();
        }
        let secs = (filetime / 10_000_000) as i64;
        let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        let (y, m, d) = civil_from_days(days - DAYS_1601_TO_1970);
        format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
    }

    /// "1.234 KB" (redondea hacia arriba), como el entorno falso del núcleo.
    fn format_size_kb(&self, bytes: u64) -> String {
        format!("{} KB", thousands(bytes.div_ceil(1024)))
    }

    /// "123 bytes", "1,23 KB", "12,3 MB", "476 GB" (tres cifras, truncadas).
    fn format_size_short(&self, bytes: u64) -> String {
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

    fn typed_paths_load(&self) -> Vec<String> {
        let Some(text) = typed_file().and_then(|f| std::fs::read_to_string(f).ok()) else { return Vec::new() };
        text.lines().map(str::trim).filter(|l| !l.is_empty()).take(xdir_core::typed::MAX).map(str::to_string).collect()
    }

    fn typed_paths_save(&self, list: &[String]) {
        let Some(file) = typed_file() else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, list.join("\n") + "\n");
    }

    /// Todavía no se abren consolas en Linux.
    fn consoles(&self) -> Vec<ConsoleInfo> {
        Vec::new()
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    fn open_console(&self, _owner: Handle, _id: u8, _dir: &Path, _verb: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::config_dir_from;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn config_dir_uses_xdg_config_home() {
        assert_eq!(config_dir_from(os("/x/conf"), os("/home/a")), Some(PathBuf::from("/x/conf/xdir")));
    }

    #[test]
    fn config_dir_falls_back_to_home() {
        assert_eq!(config_dir_from(None, os("/home/a")), Some(PathBuf::from("/home/a/.config/xdir")));
    }

    #[test]
    fn config_dir_ignores_relative_xdg() {
        assert_eq!(config_dir_from(os("rel"), os("/home/a")), Some(PathBuf::from("/home/a/.config/xdir")));
    }

    #[test]
    fn config_dir_none_without_home() {
        assert_eq!(config_dir_from(None, None), None);
    }
}

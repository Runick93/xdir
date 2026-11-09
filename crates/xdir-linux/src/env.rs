//! Entorno de Linux: carpeta de configuración según XDG, formatos neutros y el historial de rutas
//! escritas. Sin interfaz todavía: no hay tema, acento ni teclado del sistema que leer.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use xdir_core::format::{date_time_text, size_kb_text, size_short_text};
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

/// Segundos entre 1601-01-01 (época de FILETIME) y 1970-01-01.
const SECS_1601_TO_1970: i64 = 11_644_473_600;

/// Diferencia con UTC (en segundos) de la hora local en el instante `unix` (segundos desde 1970),
/// según la zona horaria del sistema; 0 si no se puede saber.
fn utc_offset(unix: i64) -> i64 {
    let t = unix as libc::time_t;
    // SAFETY: `localtime_r` escribe en `tm`, que es nuestro (la versión `_r` no usa memoria
    // compartida entre hilos).
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return 0;
    }
    i64::from(tm.tm_gmtoff)
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

    /// "AAAA-MM-DD HH:MM" (hora local) del FILETIME; vacío si es 0.
    fn format_date(&self, filetime: u64) -> String {
        let unix = (filetime / 10_000_000) as i64 - SECS_1601_TO_1970;
        date_time_text(filetime, utc_offset(unix))
    }

    /// "1.234 KB" (redondea hacia arriba), como el entorno falso del núcleo.
    fn format_size_kb(&self, bytes: u64) -> String {
        size_kb_text(bytes)
    }

    /// "123 bytes", "1,23 KB", "12,3 MB", "476 GB" (tres cifras, truncadas).
    fn format_size_short(&self, bytes: u64) -> String {
        size_short_text(bytes)
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
    use xdir_core::format::date_time_text;

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
    fn format_date_known_filetime() {
        // 2024-03-05 14:07 UTC.
        const FT: u64 = 133_541_212_200_000_000;
        assert_eq!(date_time_text(FT, 0), "2024-03-05 14:07");
        // UTC-3 (Argentina) y una diferencia que cruza la medianoche.
        assert_eq!(date_time_text(FT, -3 * 3600), "2024-03-05 11:07");
        assert_eq!(date_time_text(FT, 10 * 3600), "2024-03-06 00:07");
        assert_eq!(date_time_text(0, -3 * 3600), "");
    }

    #[test]
    fn config_dir_none_without_home() {
        assert_eq!(config_dir_from(None, None), None);
    }
}

//! Favoritos y recientes de red, en `red.toml` (carpeta de configuración; lo escribe xdir y se
//! puede editar a mano). Cada dirección pasa por `Address::parse` al leer y al guardar: las
//! inválidas se descartan y ninguna conserva una contraseña.

use std::path::{Path, PathBuf};

use super::Address;
use crate::session;
use crate::toml::{self, Value};

const FILE_NAME: &str = "red.toml";
const MAX_RECENTS: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetFavorite {
    pub name: String,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetRecent {
    pub address: String,
    /// Segundos desde 1970 (UTC).
    pub date: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetPlaces {
    pub favorites: Vec<NetFavorite>,
    pub recents: Vec<NetRecent>,
}

/// La dirección guardable (sin clave), o `None` si no es válida.
fn clean(text: &str) -> Option<String> {
    Address::parse(text).ok().map(|a| a.to_url())
}

fn same_text(a: &str, b: &Address) -> bool {
    Address::parse(a).is_ok_and(|a| a.same(b))
}

/// Hace cuánto fue `date` visto desde `now` (segundos UTC; `offset`: la zona horaria en
/// segundos): «hoy, 10:42», «ayer», «hace 3 días», «la semana pasada» o la fecha.
pub fn ago(now: i64, date: i64, offset: i64) -> String {
    let day = |t: i64| (t + offset).div_euclid(86_400);
    let days = day(now) - day(date);
    match days {
        i64::MIN..=0 => {
            let secs = (date + offset).rem_euclid(86_400);
            format!("hoy, {:02}:{:02}", secs / 3600, secs % 3600 / 60)
        }
        1 => "ayer".into(),
        2..=6 => format!("hace {days} días"),
        7..=13 => "la semana pasada".into(),
        _ => {
            let (y, m, d) = crate::format::civil_from_days(day(date));
            format!("{d:02}/{m:02}/{y}")
        }
    }
}

pub fn file_path() -> Option<PathBuf> {
    session::config_dir().map(|d| d.join(FILE_NAME))
}

impl NetPlaces {
    pub fn parse(text: &str) -> NetPlaces {
        let doc = toml::parse(text);
        let favorites = doc
            .sections("favorito")
            .filter_map(|s| {
                let address = clean(s.get("direccion")?.as_str()?)?;
                let name = s.get("nombre").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
                Some(NetFavorite { name: name.map_or_else(|| address.clone(), str::to_string), address })
            })
            .collect();
        let recents = doc
            .sections("reciente")
            .filter_map(|s| {
                let address = clean(s.get("direccion")?.as_str()?)?;
                Some(NetRecent { address, date: s.get("fecha").and_then(Value::as_int).unwrap_or(0) })
            })
            .take(MAX_RECENTS)
            .collect();
        NetPlaces { favorites, recents }
    }

    pub fn write(&self) -> String {
        let mut out = String::from(
            "# Favoritos y recientes de red de xdir (Conectar a un servidor…).\n\
             # Lo escribe xdir; se puede editar a mano. Nunca guarda contraseñas.\n",
        );
        for f in &self.favorites {
            out += "\n[[favorito]]\n";
            out += &format!("nombre = {}\n", toml::quote(&f.name));
            out += &format!("direccion = {}\n", toml::quote(clean(&f.address).as_deref().unwrap_or("")));
        }
        for r in &self.recents {
            out += "\n[[reciente]]\n";
            out += &format!("direccion = {}\n", toml::quote(clean(&r.address).as_deref().unwrap_or("")));
            out += &format!("fecha = {}\n", r.date);
        }
        out
    }

    /// Lee `red.toml` (bloqueante: en un hilo de trabajo).
    pub fn load() -> NetPlaces {
        session::config_dir().map(|d| Self::load_from(&d)).unwrap_or_default()
    }

    pub fn load_from(dir: &Path) -> NetPlaces {
        std::fs::read_to_string(dir.join(FILE_NAME)).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    /// Escribe `red.toml` (bloqueante: en un hilo de trabajo).
    pub fn save(&self) -> std::io::Result<()> {
        let dir = session::config_dir().ok_or(std::io::ErrorKind::NotFound)?;
        self.save_in(&dir)
    }

    /// Escritura atómica: un temporal en la misma carpeta y renombrar.
    pub fn save_in(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        std::fs::write(&tmp, self.write())?;
        std::fs::rename(&tmp, dir.join(FILE_NAME)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Al principio, sin repetir el mismo lugar, hasta 10.
    pub fn add_recent(&mut self, addr: &Address, now: i64) {
        self.recents.retain(|r| !same_text(&r.address, addr));
        self.recents.insert(0, NetRecent { address: addr.to_url(), date: now });
        self.recents.truncate(MAX_RECENTS);
    }

    pub fn is_favorite(&self, addr: &Address) -> bool {
        self.favorites.iter().any(|f| same_text(&f.address, addr))
    }

    /// Agrega la dirección a favoritos (con `name`, o la dirección si viene vacío) o la quita si
    /// ya estaba.
    pub fn toggle_favorite(&mut self, addr: &Address, name: Option<String>) {
        if self.is_favorite(addr) {
            self.favorites.retain(|f| !same_text(&f.address, addr));
            return;
        }
        let address = addr.to_url();
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| address.clone());
        self.favorites.push(NetFavorite { name, address });
    }

    pub fn move_favorite(&mut self, i: usize, up: bool) {
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        if let Some(j) = j.filter(|&j| j < self.favorites.len() && i < self.favorites.len()) {
            self.favorites.swap(i, j);
        }
    }

    pub fn remove_favorite(&mut self, i: usize) {
        if i < self.favorites.len() {
            self.favorites.remove(i);
        }
    }

    pub fn remove_recent(&mut self, i: usize) {
        if i < self.recents.len() {
            self.recents.remove(i);
        }
    }

    pub fn clear_recents(&mut self) {
        self.recents.clear();
    }
}

#[cfg(test)]
#[path = "places_tests.rs"]
mod tests;

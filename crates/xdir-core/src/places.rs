//! Los lugares del árbol lateral: las carpetas del usuario, la Papelera, Acceso rápido y carpetas
//! propias, en el orden que elige el usuario (Configuración) y con la opción de ocultar. Se
//! guarda en `lugares.toml` (carpeta de configuración; lo escribe xdir y se puede editar a mano).

use std::path::{Path, PathBuf};

use crate::session;
use crate::toml::{self, Value};

const FILE_NAME: &str = "lugares.toml";

/// Claves de los lugares fijos, en el orden por defecto. `User(i)` es la posición en
/// `fs::user_folder` (0..6).
const KEYS: [(&str, Place); 8] = [
    ("escritorio", Place::User(0)),
    ("descargas", Place::User(1)),
    ("documentos", Place::User(2)),
    ("imagenes", Place::User(3)),
    ("musica", Place::User(4)),
    ("videos", Place::User(5)),
    ("papelera", Place::Trash),
    ("acceso_rapido", Place::QuickAccess),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    User(usize),
    Trash,
    QuickAccess,
    Folder(PathBuf),
}

impl Place {
    fn key(&self) -> Option<&'static str> {
        KEYS.iter().find(|(_, p)| p == self).map(|(k, _)| *k)
    }

    fn from_text(text: &str) -> Option<Place> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        Some(KEYS.iter().find(|(k, _)| *k == text).map_or_else(|| Place::Folder(PathBuf::from(text)), |(_, p)| p.clone()))
    }

    fn to_text(&self) -> String {
        match self {
            Place::Folder(p) => p.to_string_lossy().into_owned(),
            other => other.key().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    pub order: Vec<Place>,
    pub hidden: Vec<Place>,
}

impl Default for Places {
    fn default() -> Self {
        Places { order: KEYS.iter().map(|(_, p)| p.clone()).collect(), hidden: Vec::new() }
    }
}

pub fn file_path() -> Option<PathBuf> {
    session::config_dir().map(|d| d.join(FILE_NAME))
}

impl Places {
    /// Lo que no se entiende se ignora; sin `orden` queda el de por defecto, y los lugares fijos
    /// que falten se suman después del último fijo presente.
    pub fn parse(text: &str) -> Places {
        let doc = toml::parse(text);
        let top = doc.sections.iter().find(|s| s.name.is_empty());
        let list = |key: &str| -> Vec<Place> {
            let items = top.and_then(|s| s.get(key)).and_then(Value::as_str_list).unwrap_or_default();
            items.into_iter().filter_map(Place::from_text).collect()
        };
        let mut order: Vec<Place> = Vec::new();
        for p in list("orden") {
            if !order.contains(&p) {
                order.push(p);
            }
        }
        for (_, builtin) in &KEYS {
            if order.contains(builtin) {
                continue;
            }
            let at = order.iter().rposition(|p| !matches!(p, Place::Folder(_))).map_or(0, |i| i + 1);
            order.insert(at, builtin.clone());
        }
        let hidden = list("ocultos").into_iter().filter(|p| order.contains(p)).fold(Vec::new(), |mut acc, p| {
            if !acc.contains(&p) {
                acc.push(p);
            }
            acc
        });
        Places { order, hidden }
    }

    pub fn to_toml(&self) -> String {
        let list = |items: &[Place]| items.iter().map(|p| toml::quote(&p.to_text())).collect::<Vec<_>>().join(", ");
        format!(
            "# Lugares del árbol lateral de xdir (Configuración > Árbol).\n\
             # Lo escribe xdir; se puede editar a mano. Cualquier texto que no sea un lugar fijo\n\
             # (escritorio, descargas, documentos, imagenes, musica, videos, papelera, acceso_rapido)\n\
             # es una carpeta propia.\n\
             orden = [{}]\nocultos = [{}]\n",
            list(&self.order),
            list(&self.hidden)
        )
    }

    /// Lee `lugares.toml` (bloqueante: en un hilo de trabajo).
    pub fn load() -> Places {
        session::config_dir().map(|d| Self::load_from(&d)).unwrap_or_default()
    }

    pub fn load_from(dir: &Path) -> Places {
        std::fs::read_to_string(dir.join(FILE_NAME)).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    /// Escribe `lugares.toml` (bloqueante: en un hilo de trabajo).
    pub fn save(&self) -> std::io::Result<()> {
        let dir = session::config_dir().ok_or(std::io::ErrorKind::NotFound)?;
        self.save_in(&dir)
    }

    /// Escritura atómica: un temporal propio de esta escritura en la misma carpeta y renombrar.
    pub fn save_in(&self, dir: &Path) -> std::io::Result<()> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::fs::create_dir_all(dir)?;
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.join(format!("{FILE_NAME}.{}.{n}.tmp", std::process::id()));
        std::fs::write(&tmp, self.to_toml()).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        std::fs::rename(&tmp, dir.join(FILE_NAME)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }

    pub fn move_up(&mut self, i: usize) {
        if i > 0 && i < self.order.len() {
            self.order.swap(i, i - 1);
        }
    }

    pub fn move_down(&mut self, i: usize) {
        if i + 1 < self.order.len() {
            self.order.swap(i, i + 1);
        }
    }

    /// Oculta o muestra `order[i]`.
    pub fn toggle(&mut self, i: usize) {
        let Some(p) = self.order.get(i) else { return };
        match self.hidden.iter().position(|h| h == p) {
            Some(at) => {
                self.hidden.remove(at);
            }
            None => self.hidden.push(p.clone()),
        }
    }

    pub fn is_hidden(&self, p: &Place) -> bool {
        self.hidden.contains(p)
    }

    /// Al final, sin repetir una carpeta propia que ya está (compara con `same_path` del sistema).
    /// Solo rutas absolutas: una relativa como «papelera» se leería después como un lugar fijo.
    pub fn add_folder(&mut self, path: PathBuf) {
        if !path.is_absolute() {
            return;
        }
        let fs = crate::sys::file_system();
        if self.order.iter().any(|p| matches!(p, Place::Folder(q) if fs.same_path(q, &path))) {
            return;
        }
        self.order.push(Place::Folder(path));
    }

    /// Quita una carpeta propia; los lugares fijos no se quitan (se ocultan).
    pub fn remove(&mut self, i: usize) {
        if matches!(self.order.get(i), Some(Place::Folder(_))) {
            let p = self.order.remove(i);
            self.hidden.retain(|h| *h != p);
        }
    }
}

#[cfg(test)]
#[path = "places_tests.rs"]
mod tests;

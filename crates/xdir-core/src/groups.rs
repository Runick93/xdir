//! Grupos guardados (como los Quick-Links de Q-Dir): layout, divisores y las pestañas de cada
//! panel con un nombre, para volver a esa disposición con un clic. Se guardan en `grupos.toml`,
//! junto a la sesión; lo escribe xdir al guardar un grupo, pero se puede editar a mano.

use crate::session::{self, PaneState, location_from, location_str};
use crate::toml::{self, Value};

const FILE_NAME: &str = "grupos.toml";

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub layout: usize,
    pub split_x: f32,
    pub split_y: f32,
    pub panes: Vec<PaneState>,
    /// Pestaña sin guardar (como un archivo nuevo de un editor): al cerrarla se decide si se conserva.
    pub temp: bool,
}

impl Group {
    /// Mismo contenido, con la tolerancia de los divisores (se guardan con 4 decimales).
    pub fn same_as(&self, other: &Group) -> bool {
        self.name == other.name
            && self.layout == other.layout
            && self.panes == other.panes
            && self.temp == other.temp
            && (self.split_x - other.split_x).abs() < 1e-3
            && (self.split_y - other.split_y).abs() < 1e-3
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Groups {
    pub items: Vec<Group>,
}

impl Groups {
    pub fn load() -> Self {
        let text = session::config_dir().and_then(|d| std::fs::read_to_string(d.join(FILE_NAME)).ok());
        text.map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = session::config_dir().ok_or(std::io::ErrorKind::NotFound)?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(FILE_NAME), self.write())
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.items.iter().position(|g| g.name.eq_ignore_ascii_case(name))
    }

    /// Agrega el grupo, o reemplaza el que tenga el mismo nombre.
    pub fn put(&mut self, group: Group) {
        match self.items.iter_mut().find(|g| g.name.eq_ignore_ascii_case(&group.name)) {
            Some(g) => *g = group,
            None => self.items.push(group),
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    pub fn write(&self) -> String {
        let mut out = String::from(
            "# Grupos guardados de xdir: layout y carpetas de cada panel.\n\
             # Se agregan desde el botón de grupos de la barra de estado; también se pueden editar acá.\n",
        );
        for g in &self.items {
            out += "\n[[grupo]]\n";
            out += &format!("nombre = {}\n", toml::quote(&g.name));
            if g.temp {
                out += "temporal = true\n";
            }
            out += &format!("layout = {}\n", g.layout);
            out += &format!("division_x = {:.4}\n", g.split_x);
            out += &format!("division_y = {:.4}\n", g.split_y);
            let panes: Vec<String> = g
                .panes
                .iter()
                .map(|p| format!("[{}]", p.tabs.iter().map(|t| toml::quote(&location_str(t))).collect::<Vec<_>>().join(", ")))
                .collect();
            out += &format!("paneles = [{}]\n", panes.join(", "));
            let active: Vec<String> = g.panes.iter().map(|p| p.active.to_string()).collect();
            out += &format!("pestanas_activas = [{}]\n", active.join(", "));
        }
        out
    }

    pub fn parse(text: &str) -> Self {
        let doc = toml::parse(text);
        let items = doc
            .sections("grupo")
            .filter_map(|s| {
                let name = s.get("nombre")?.as_str()?.trim().to_string();
                let Some(Value::List(panes)) = s.get("paneles") else { return None };
                let active: Vec<usize> = match s.get("pestanas_activas") {
                    Some(Value::List(v)) => v.iter().map(|x| x.as_int().unwrap_or(0).max(0) as usize).collect(),
                    _ => Vec::new(),
                };
                let panes = panes
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let tabs = p.as_str_list().unwrap_or_default().into_iter().map(location_from).collect::<Vec<_>>();
                        let active = active.get(i).copied().unwrap_or(0).min(tabs.len().saturating_sub(1));
                        PaneState { tabs, active }
                    })
                    .collect();
                let num = |k: &str, d: f64| s.get(k).and_then(Value::as_float).unwrap_or(d);
                (!name.is_empty()).then(|| Group {
                    name,
                    layout: num("layout", 4.0).clamp(0.0, 4.0) as usize,
                    split_x: num("division_x", 0.5) as f32,
                    split_y: num("division_y", 0.5) as f32,
                    panes,
                    temp: s.get("temporal").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        Self { items }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::Location;

    #[test]
    fn round_trip() {
        let g = Group {
            name: "Trabajo \"cliente\"".into(),
            layout: 3,
            split_x: 0.3,
            split_y: 0.6,
            panes: vec![
                PaneState { tabs: vec![Location::Path(r"C:\dev\a b".into()), Location::Computer], active: 1 },
                PaneState { tabs: vec![Location::Path(r"\\servidor\recurso".into())], active: 0 },
            ],
            temp: true,
        };
        let mut gs = Groups::default();
        gs.put(g.clone());
        let back = Groups::parse(&gs.write());
        assert_eq!(back.items.len(), 1);
        let b = &back.items[0];
        assert_eq!(b.name, g.name);
        assert_eq!(b.layout, 3);
        assert!((b.split_x - 0.3).abs() < 1e-4 && (b.split_y - 0.6).abs() < 1e-4);
        assert_eq!(b.panes, g.panes);
        assert!(b.temp);
    }

    #[test]
    fn put_replaces_by_name() {
        let mut gs = Groups::default();
        let g = |name: &str, layout| Group { name: name.into(), layout, split_x: 0.5, split_y: 0.5, panes: vec![], temp: false };
        gs.put(g("Uno", 1));
        gs.put(g("dos", 2));
        gs.put(g("UNO", 4));
        assert_eq!(gs.items.iter().map(|g| (g.name.as_str(), g.layout)).collect::<Vec<_>>(), [("UNO", 4), ("dos", 2)]);
    }

    #[test]
    fn hand_edited_file() {
        let gs = Groups::parse("[[grupo]]\nnombre = 'Fotos'\npaneles = [['D:\\fotos'], ['E:\\backup']]\n[[grupo]]\npaneles = []\n");
        assert_eq!(gs.items.len(), 1, "el segundo no tiene nombre");
        assert_eq!(gs.items[0].layout, 4);
        assert_eq!(gs.items[0].panes[1].tabs, [Location::Path(r"E:\backup".into())]);
    }
}

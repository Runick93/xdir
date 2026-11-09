//! Favoritos: carpetas sueltas o agrupadas en carpetas de favoritos (anidables), como la barra
//! de marcadores de Chrome.
//!
//! Se guardan en `favoritos.txt`, junto a la sesión, en un formato pensado para editar a mano:
//!
//! ```text
//! Proyectos = C:\dev
//! Trabajo:
//!   Clientes = D:\clientes
//!   Informes = C:\Users\yo\Documents\Informes
//! ```
//!
//! Una línea por favorito (`nombre = ruta`); las carpetas terminan en `:` y su contenido va
//! indentado con dos espacios.

use std::path::PathBuf;

use crate::fs::Location;
use crate::session::{self, location_from, location_str};

const FILE_NAME: &str = "favoritos.txt";
const INDENT: &str = "  ";

#[derive(Clone, Debug, PartialEq)]
pub enum Fav {
    Link { name: String, location: Location },
    Group { name: String, items: Vec<Fav> },
}

impl Fav {
    pub fn name(&self) -> &str {
        match self {
            Fav::Link { name, .. } | Fav::Group { name, .. } => name,
        }
    }

    pub fn set_name(&mut self, new: String) {
        match self {
            Fav::Link { name, .. } | Fav::Group { name, .. } => *name = new,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Fav::Group { .. })
    }
}

/// Posición de un favorito en el árbol: índices desde la raíz.
pub type FavPath = Vec<usize>;

#[derive(Default, Debug, PartialEq)]
pub struct Favorites {
    pub items: Vec<Fav>,
}

impl Favorites {
    pub fn load() -> Self {
        let text = session::config_dir().and_then(|d| std::fs::read_to_string(d.join(FILE_NAME)).ok());
        let mut f = text.map(|t| parse(&t)).unwrap_or_default();
        // Rutas cortas 8.3 escritas a mano: a su forma larga, como las de las pestañas.
        fn normalize(items: &mut [Fav]) {
            for item in items {
                match item {
                    Fav::Link { location: Location::Path(p), .. } => *p = crate::sys::file_system().long_path(p),
                    Fav::Group { items, .. } => normalize(items),
                    _ => {}
                }
            }
        }
        normalize(&mut f.items);
        f
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = session::config_dir().ok_or(std::io::ErrorKind::NotFound)?;
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        std::fs::write(&tmp, write(self))?;
        std::fs::rename(&tmp, dir.join(FILE_NAME))
    }

    pub fn file_path() -> Option<PathBuf> {
        session::config_dir().map(|d| d.join(FILE_NAME))
    }

    /// ¿Hay un favorito (en cualquier nivel) que apunte a `location`?
    pub fn find(&self, location: &Location) -> Option<FavPath> {
        fn walk(items: &[Fav], location: &Location, path: &mut FavPath) -> bool {
            for (i, f) in items.iter().enumerate() {
                path.push(i);
                match f {
                    Fav::Link { location: l, .. } if same(l, location) => return true,
                    Fav::Group { items, .. } if walk(items, location, path) => return true,
                    _ => {}
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        walk(&self.items, location, &mut path).then_some(path)
    }

    pub fn get(&self, path: &[usize]) -> Option<&Fav> {
        let (first, rest) = path.split_first()?;
        let mut f = self.items.get(*first)?;
        for &i in rest {
            match f {
                Fav::Group { items, .. } => f = items.get(i)?,
                Fav::Link { .. } => return None,
            }
        }
        Some(f)
    }

    pub fn get_mut(&mut self, path: &[usize]) -> Option<&mut Fav> {
        let (first, rest) = path.split_first()?;
        let mut f = self.items.get_mut(*first)?;
        for &i in rest {
            match f {
                Fav::Group { items, .. } => f = items.get_mut(i)?,
                Fav::Link { .. } => return None,
            }
        }
        Some(f)
    }

    /// Lista que contiene al elemento `path` (la raíz o el contenido de una carpeta).
    fn parent_list(&mut self, path: &[usize]) -> Option<&mut Vec<Fav>> {
        let (_, parent) = path.split_last()?;
        if parent.is_empty() {
            return Some(&mut self.items);
        }
        match self.get_mut(parent)? {
            Fav::Group { items, .. } => Some(items),
            Fav::Link { .. } => None,
        }
    }

    pub fn remove(&mut self, path: &[usize]) -> Option<Fav> {
        let last = *path.last()?;
        let list = self.parent_list(path)?;
        (last < list.len()).then(|| list.remove(last))
    }

    /// Inserta en la raíz en `index` (al final si se pasa).
    pub fn insert_root(&mut self, index: usize, fav: Fav) {
        let i = index.min(self.items.len());
        self.items.insert(i, fav);
    }

    /// Agrega al final de una carpeta de favoritos.
    pub fn push_into(&mut self, group: &[usize], fav: Fav) -> bool {
        match self.get_mut(group) {
            Some(Fav::Group { items, .. }) => {
                items.push(fav);
                true
            }
            _ => false,
        }
    }

    /// Mueve un elemento de la raíz a otra posición de la raíz.
    pub fn move_root(&mut self, from: usize, to: usize) {
        if from < self.items.len() && from != to {
            let f = self.items.remove(from);
            let to = if to > from { to - 1 } else { to };
            self.items.insert(to.min(self.items.len()), f);
        }
    }
}

/// El favorito de un elemento de un menú de favoritos, a partir de su camino de posiciones dentro
/// del menú (`positions`, como lo da `WindowHost::popup_menu`). El menú muestra la carpeta `base`
/// (vacía: la raíz) desde el índice `first` (el desborde «»» de la barra; 0 si no hay). `None` si el
/// camino no lleva a un favorito (como «(vacía)» dentro de una carpeta sin nada).
pub fn fav_path_at(favs: &Favorites, base: &FavPath, first: usize, positions: &[usize]) -> Option<FavPath> {
    let (&top, rest) = positions.split_first()?;
    let mut path = base.clone();
    path.push(top + first);
    path.extend_from_slice(rest);
    favs.get(&path).is_some().then_some(path)
}

/// Dos ubicaciones son el mismo favorito (rutas sin distinguir mayúsculas).
fn same(a: &Location, b: &Location) -> bool {
    match (a, b) {
        (Location::Path(x), Location::Path(y)) => crate::sys::file_system().same_path(x, y),
        (Location::Shell { parsing: x, .. }, Location::Shell { parsing: y, .. }) => x.eq_ignore_ascii_case(y),
        (Location::Computer, Location::Computer) => true,
        _ => false,
    }
}

fn write(f: &Favorites) -> String {
    fn walk(items: &[Fav], depth: usize, out: &mut String) {
        for item in items {
            out.push_str(&INDENT.repeat(depth));
            match item {
                Fav::Link { name, location } => out.push_str(&format!("{} = {}\n", clean(name), location_str(location))),
                Fav::Group { name, items } => {
                    out.push_str(&format!("{}:\n", clean(name)));
                    walk(items, depth + 1, out);
                }
            }
        }
    }
    let mut out = String::from(
        "# Favoritos de xdir. Una línea por favorito: nombre = ruta.\n\
         # Las carpetas terminan en \":\" y su contenido va indentado con dos espacios.\n",
    );
    walk(&f.items, 0, &mut out);
    out
}

/// Un nombre no puede romper el formato: sin " = " ni ":" final.
fn clean(name: &str) -> String {
    name.replace(" = ", " - ").trim_end_matches(':').trim().to_string()
}

fn parse(text: &str) -> Favorites {
    // Pila de carpetas abiertas: (profundidad, contenido).
    let mut stack: Vec<(usize, String, Vec<Fav>)> = Vec::new();
    let mut root: Vec<Fav> = Vec::new();

    fn close_until(depth: usize, stack: &mut Vec<(usize, String, Vec<Fav>)>, root: &mut Vec<Fav>) {
        while stack.last().is_some_and(|(d, _, _)| *d >= depth) {
            let (_, name, items) = stack.pop().unwrap();
            let group = Fav::Group { name, items };
            match stack.last_mut() {
                Some((_, _, parent)) => parent.push(group),
                None => root.push(group),
            }
        }
    }

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        let depth = spaces / INDENT.len();
        close_until(depth, &mut stack, &mut root);

        if let Some((name, target)) = trimmed.split_once(" = ") {
            let fav = Fav::Link { name: name.trim().to_string(), location: location_from(target.trim()) };
            match stack.last_mut() {
                Some((_, _, items)) => items.push(fav),
                None => root.push(fav),
            }
        } else if let Some(name) = trimmed.strip_suffix(':') {
            stack.push((depth, name.trim().to_string(), Vec::new()));
        }
    }
    close_until(0, &mut stack, &mut root);
    Favorites { items: root }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::install_fakes;

    fn link(name: &str, path: &str) -> Fav {
        Fav::Link { name: name.into(), location: Location::Path(path.into()) }
    }

    fn sample() -> Favorites {
        Favorites {
            items: vec![
                link("Proyectos", r"C:\dev"),
                Fav::Group {
                    name: "Trabajo".into(),
                    items: vec![
                        link("Clientes", r"D:\clientes"),
                        Fav::Group { name: "Viejos".into(), items: vec![link("2019", r"D:\2019")] },
                        link("Informes", r"C:\inf"),
                    ],
                },
                Fav::Link { name: "Equipo".into(), location: Location::Computer },
            ],
        }
    }

    #[test]
    fn round_trip() {
        install_fakes();
        let f = sample();
        assert_eq!(parse(&write(&f)), f);
    }

    #[test]
    fn find_and_edit() {
        install_fakes();
        let mut f = sample();
        assert_eq!(f.find(&Location::Path(r"c:\INF".into())), Some(vec![1, 2]));
        assert_eq!(f.get(&[1, 1, 0]).map(|x| x.name()), Some("2019"));
        let removed = f.remove(&[1, 0]).unwrap();
        assert_eq!(removed.name(), "Clientes");
        assert!(f.push_into(&[1], removed));
        assert_eq!(f.get(&[1, 2]).map(|x| x.name()), Some("Clientes"));
        f.move_root(0, 3);
        assert_eq!(f.items[2].name(), "Proyectos");
    }

    /// La raíz de un menú: `a`, la carpeta `g` (con `x` y la carpeta vacía `nada`), `c`, `d`, `e`.
    fn menu_sample() -> Favorites {
        Favorites {
            items: vec![
                link("a", r"C:\a"),
                Fav::Group { name: "g".into(), items: vec![link("x", r"C:\x"), Fav::Group { name: "nada".into(), items: vec![] }] },
                link("c", r"C:\c"),
                link("d", r"C:\d"),
                link("e", r"C:\e"),
            ],
        }
    }

    #[test]
    fn fav_path_at_root() {
        let f = menu_sample();
        assert_eq!(fav_path_at(&f, &vec![], 0, &[2]), Some(vec![2]));
        assert_eq!(fav_path_at(&f, &vec![], 0, &[0]), Some(vec![0]));
        assert_eq!(fav_path_at(&f, &vec![], 0, &[]), None);
    }

    #[test]
    fn fav_path_at_overflow() {
        let f = menu_sample();
        assert_eq!(fav_path_at(&f, &vec![], 3, &[0]), Some(vec![3]));
        assert_eq!(fav_path_at(&f, &vec![], 3, &[1]), Some(vec![4]));
        assert_eq!(fav_path_at(&f, &vec![], 3, &[2]), None);
    }

    #[test]
    fn fav_path_at_nested_group() {
        let f = menu_sample();
        // Desde la raíz, dentro del submenú de la carpeta `g`.
        assert_eq!(fav_path_at(&f, &vec![], 0, &[1, 0]), Some(vec![1, 0]));
        // El menú de la carpeta `g` misma.
        assert_eq!(fav_path_at(&f, &vec![1], 0, &[0]), Some(vec![1, 0]));
        assert_eq!(fav_path_at(&f, &vec![1], 0, &[1]), Some(vec![1, 1]));
    }

    #[test]
    fn fav_path_at_group_itself() {
        let f = menu_sample();
        assert_eq!(fav_path_at(&f, &vec![], 0, &[1]), Some(vec![1]));
    }

    #[test]
    fn fav_path_at_empty_marker_is_none() {
        let f = menu_sample();
        // «(vacía)» dentro de la carpeta `nada`.
        assert_eq!(fav_path_at(&f, &vec![], 0, &[1, 1, 0]), None);
        assert_eq!(fav_path_at(&f, &vec![1, 1], 0, &[0]), None);
    }

    #[test]
    fn hand_edited() {
        install_fakes();
        let f = parse("# comentario\nA = C:\\a\nGrupo:\n  B = C:\\b\n    mal indentado = C:\\x\nC = C:\\c\n");
        assert_eq!(f.items.len(), 3);
        assert_eq!(f.get(&[1, 0]).map(|x| x.name()), Some("B"));
    }
}

//! Tipos neutros del sistema de archivos: el lugar que muestra un panel.

use std::path::{Component, Path, PathBuf};

use crate::sys::{self, SystemEnv};

/// Lugar que muestra un panel.
#[derive(Clone, PartialEq, Debug)]
pub enum Location {
    /// "Este equipo": carpetas del usuario, unidades, dispositivos, Red y Papelera.
    Computer,
    Path(PathBuf),
    /// Carpeta virtual del Shell sin ruta de disco: Papelera, Red, un teléfono (MTP)...
    Shell {
        /// Nombre de análisis del Shell ("::{645FF040-...}" para la Papelera).
        parsing: String,
        name: String,
        /// De dónde se llegó, para "subir"; sin él, se sube a "Este equipo".
        parent: Option<Box<Location>>,
    },
}

impl Location {
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Computer => None,
            Location::Path(p) => Some(match p.parent() {
                Some(parent) => Location::Path(parent.to_path_buf()),
                None => Location::Computer,
            }),
            Location::Shell { parent, .. } => Some(parent.as_deref().cloned().unwrap_or(Location::Computer)),
        }
    }

    /// Título de la pestaña (con el entorno instalado).
    pub fn title(&self) -> String {
        self.title_in(sys::env())
    }

    /// Título de la pestaña con el entorno `env`: el último tramo de la ruta; una raíz se titula
    /// como la nombra el sistema ("Disco local (C:)", "Macintosh HD").
    pub fn title_in(&self, env: &dyn SystemEnv) -> String {
        match self {
            Location::Computer => "Este equipo".into(),
            Location::Path(p) => match p.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => env.root_title(p).unwrap_or_else(|| p.to_string_lossy().trim_end_matches('\\').to_string()),
            },
            Location::Shell { name, .. } => name.clone(),
        }
    }

    /// Segmentos de la barra de dirección: (texto, destino), con el entorno instalado.
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        self.crumbs_in(sys::env())
    }

    /// Segmentos de la barra de dirección con el entorno `env`.
    pub fn crumbs_in(&self, env: &dyn SystemEnv) -> Vec<(String, Location)> {
        let mut out = vec![("Este equipo".to_string(), Location::Computer)];
        match self {
            Location::Computer => {}
            Location::Path(p) => {
                let mut acc = PathBuf::new();
                for comp in p.components() {
                    acc.push(comp.as_os_str());
                    let label = comp.as_os_str().to_string_lossy().into_owned();
                    match comp {
                        // La raíz sola ("/" o "\"): si el sistema la nombra es un tramo propio (el
                        // disco de arranque de macOS); si no ("C:" y "\" de Windows llegan como
                        // dos componentes), se pega al tramo anterior.
                        Component::RootDir => match env.root_title(Path::new(comp.as_os_str())) {
                            Some(name) => out.push((name, Location::Path(acc.clone()))),
                            None => out.last_mut().unwrap().1 = Location::Path(acc.clone()),
                        },
                        // Una unidad se muestra como en el Explorador: "Disco local (C:)".
                        Component::Prefix(_) => out.push((env.root_title(Path::new(&label)).unwrap_or(label), Location::Path(acc.clone()))),
                        _ => out.push((label, Location::Path(acc.clone()))),
                    }
                }
            }
            Location::Shell { name, parent, .. } => {
                if let Some(parent) = parent {
                    out = parent.crumbs_in(env);
                }
                out.push((name.clone(), self.clone()));
            }
        }
        out
    }

    /// Ruta o nombre de análisis que entiende `IFileOperation` como destino (pegar, soltar).
    pub fn target(&self) -> Option<PathBuf> {
        match self {
            Location::Computer => None,
            Location::Path(p) => Some(p.clone()),
            Location::Shell { parsing, .. } => Some(PathBuf::from(parsing)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::FakeEnv;

    fn path(p: &str) -> Location {
        Location::Path(PathBuf::from(p))
    }

    /// Con el entorno falso las raíces no tienen nombre propio: el título es el texto de la ruta
    /// y "/" se pega a «Este equipo» (como "\" sin unidad en Windows).
    #[test]
    fn titles_and_crumbs_with_the_fake_env() {
        assert_eq!(Location::Computer.title_in(&FakeEnv), "Este equipo");
        assert_eq!(path("/a/b").title_in(&FakeEnv), "b");
        assert_eq!(path("/").title_in(&FakeEnv), "/");
        assert_eq!(
            path("/a/b").crumbs_in(&FakeEnv),
            [("Este equipo".to_string(), path("/")), ("a".to_string(), path("/a")), ("b".to_string(), path("/a/b"))]
        );
        let bin = Location::Shell { parsing: "::{bin}".into(), name: "Papelera".into(), parent: Some(Box::new(Location::Computer)) };
        assert_eq!(bin.title_in(&FakeEnv), "Papelera");
        assert_eq!(bin.crumbs_in(&FakeEnv), [("Este equipo".to_string(), Location::Computer), ("Papelera".to_string(), bin.clone())]);
    }

    #[test]
    fn parent_and_target() {
        assert_eq!(path("/a/b").parent(), Some(path("/a")));
        assert_eq!(path("/").parent(), Some(Location::Computer));
        assert_eq!(Location::Computer.parent(), None);
        let bin = Location::Shell { parsing: "::{bin}".into(), name: "Papelera".into(), parent: None };
        assert_eq!(bin.parent(), Some(Location::Computer));
        assert_eq!(bin.target(), Some(PathBuf::from("::{bin}")));
        assert_eq!(Location::Computer.target(), None);
    }
}

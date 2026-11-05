//! Lectura de carpetas en segundo plano y vigilancia de cambios.
//!
//! Cada lectura corre en su propio hilo y manda los resultados por un canal, avisando a la
//! ventana con `WM_APP_LOAD`. La interfaz nunca espera al disco.
//!
//! Acá están los tipos comunes; la lectura en sí es de cada sistema (`win32.rs`, `macos.rs`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::platform::WM_APP;

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Hay mensajes nuevos en el canal de carga. `wparam` = panel.
pub const WM_APP_LOAD: u32 = WM_APP + 1;
/// Cambió algo en la carpeta vigilada. `wparam` = panel, `lparam` = generación.
pub const WM_APP_CHANGED: u32 = WM_APP + 2;

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

    pub fn title(&self) -> String {
        match self {
            Location::Computer => "Este equipo".into(),
            Location::Path(p) => match p.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => {
                    let root = p.to_string_lossy().trim_end_matches('\\').to_string();
                    // Una unidad se titula como en el Explorador: "Disco local (C:)".
                    if root.len() == 2 && root.ends_with(':') { drive_label(&root) } else { root }
                }
            },
            Location::Shell { name, .. } => name.clone(),
        }
    }

    /// Segmentos de la barra de dirección: (texto, destino).
    pub fn crumbs(&self) -> Vec<(String, Location)> {
        let mut out = vec![("Este equipo".to_string(), Location::Computer)];
        match self {
            Location::Computer => {}
            Location::Path(p) => {
                let mut acc = PathBuf::new();
                for comp in p.components() {
                    acc.push(comp.as_os_str());
                    // "C:" y "\" llegan como dos componentes; el segundo se pega al primero.
                    if matches!(comp, std::path::Component::RootDir) {
                        out.last_mut().unwrap().1 = Location::Path(acc.clone());
                        continue;
                    }
                    let mut label = comp.as_os_str().to_string_lossy().into_owned();
                    // Una unidad se muestra como en el Explorador: "Disco local (C:)".
                    if matches!(comp, std::path::Component::Prefix(_)) && label.len() == 2 && label.ends_with(':') {
                        label = drive_label(&label);
                    }
                    out.push((label, Location::Path(acc.clone())));
                }
            }
            Location::Shell { name, parent, .. } => {
                if let Some(parent) = parent {
                    out = parent.crumbs();
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

impl Location {
    /// ¿Esta ubicación es la carpeta `p`?
    pub fn is_path(&self, p: &Path) -> bool {
        matches!(self, Location::Path(own) if same_path(own, p))
    }
}

pub struct Entry {
    /// Nombre real en disco (se usa para abrir y navegar).
    pub name: OsString,
    /// Ruta completa o nombre de análisis del Shell, cuando no sale de "carpeta + nombre"
    /// (carpetas virtuales, accesos de "Este equipo").
    pub full: Option<OsString>,
    /// Grupo de orden: va antes que cualquier columna (en "Este equipo": carpetas,
    /// unidades, dispositivos, lugares del Shell).
    pub group: u8,
    /// PIDL absoluto de los elementos del Shell (ver `shell::Item`).
    pub pidl: Option<Arc<[u8]>>,
    /// Nombre para mostrar (en unidades, "Disco local (C:)").
    pub display: String,
    /// `display` en minúsculas, para ordenar y buscar sin recalcular.
    pub sort_key: String,
    pub is_dir: bool,
    pub size: u64,
    /// FILETIME en UTC; 0 si no aplica.
    pub modified: u64,
    /// Fecha de creación (FILETIME en UTC); 0 si no aplica.
    pub created: u64,
    pub attrs: u32,
    pub type_name: Arc<str>,
    /// Índice en la lista de imágenes del sistema.
    pub icon: i32,
    /// Superposición del ícono (0 = ninguna).
    pub overlay: u8,
    /// Ya se consultó la superposición (se hace solo para las filas que se llegan a ver).
    pub overlay_checked: bool,
}

impl Entry {
    pub fn is_hidden(&self) -> bool {
        const HIDDEN: u32 = 0x2;
        self.attrs & HIDDEN != 0
    }
}

pub enum LoadMsg {
    /// Un lote de elementos (en orden de llegada).
    Entries(Vec<Entry>),
    /// Ya se listó todo. Error si no se pudo abrir la carpeta.
    Listed(Result<(), String>),
    /// Íconos individuales resueltos después: (posición en orden de llegada, ícono).
    Icons(Vec<(u32, i32)>),
    /// Superposiciones: (posición, índice de superposición).
    Overlays(Vec<(u32, u8)>),
}

pub struct LoadEvent {
    pub panel: usize,
    pub generation: u64,
    pub msg: LoadMsg,
}

/// Una carpeta del Acceso rápido de Windows.
#[derive(Clone, Debug, PartialEq)]
pub struct QuickItem {
    pub name: String,
    pub path: PathBuf,
    /// Anclada (`System.Home.IsPinned`); si no, es una carpeta frecuente.
    pub pinned: bool,
}

/// Quita los `.` y resuelve los `..` de una ruta escrita a mano, sin tocar el disco
/// ("C:\a\b\.." → "C:\a"). Un `..` en la raíz se ignora; en una ruta relativa sin nada que
/// quitar, se conserva.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::Prefix(_) | Component::RootDir) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// ¿Tiene extensión .zip?
pub fn is_zip(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// ¿Es una ruta de red (`\\servidor\recurso\...`)? Los prefijos `\\?\` y `\\.\` no cuentan.
pub fn is_unc(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.starts_with(r"\\") && !s.starts_with(r"\\?\") && !s.starts_with(r"\\.\")
}

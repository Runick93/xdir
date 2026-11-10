//! Tipos de las operaciones de archivos (copiar, mover, borrar, renombrar, carpeta nueva, archivo
//! de texto nuevo y deshacer). Cada operación corre en su propio hilo (la implementa cada sistema detrás de
//! `sys::FileOps`) y al terminar avisa a la ventana con `WM_APP_OP_DONE`.

use std::path::PathBuf;

use crate::WM_APP;

/// Terminó una operación; el resultado está en el canal.
pub const WM_APP_OP_DONE: u32 = WM_APP + 4;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Copy { sources: Vec<PathBuf>, dest: PathBuf },
    Move { sources: Vec<PathBuf>, dest: PathBuf },
    Delete { paths: Vec<PathBuf>, recycle: bool },
    Rename { path: PathBuf, new_name: String },
    NewFolder { parent: PathBuf },
    /// Archivo de texto vacío «Nuevo documento de texto.txt» (« (2)», « (3)»… si ya existe).
    NewFile { parent: PathBuf },
    /// Devuelve elementos de la Papelera a su lugar original (macOS y Linux). La `App` lo hace con
    /// el motor de copia (`JobSpec::Restore`); queda como respaldo, como `Copy` y `Move`. Duplicar
    /// y «Carpeta nueva con la selección» son solo del motor.
    Restore { items: Vec<PathBuf> },
    /// Un acceso directo (alias, enlace) junto a cada uno (`names::shortcut_name`).
    Shortcuts { targets: Vec<PathBuf> },
    Undo(Undo),
}

/// Cómo deshacer una operación ya hecha.
#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    /// Volver a llamar `path` como `old_name`.
    Rename { path: PathBuf, old_name: String },
    /// Copias, carpetas o archivos nuevos: se mandan a la papelera.
    Created(Vec<PathBuf>),
    /// (dónde quedó, dónde estaba).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// (nombre del elemento dentro de la papelera, dónde estaba).
    Recycled(Vec<(String, PathBuf)>),
    /// Varios pasos de una sola operación; se deshacen en orden inverso.
    Batch(Vec<Undo>),
    /// Carpeta creada por la operación: va a la Papelera solo si quedó vacía (si no, se deja).
    CreatedFolder(PathBuf),
    /// Renombrar varios: (cómo se llama ahora, nombre anterior). Se deshace en el núcleo, con
    /// `rename_many::rename_all` (el mismo cruce seguro), nunca por `FileOps`.
    Renames(Vec<(PathBuf, String)>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpResult {
    pub undo: Option<Undo>,
    /// Elemento a enfocar al terminar (renombrado o carpeta nueva).
    pub focus: Option<PathBuf>,
    /// Carpeta o archivo nuevo: se abre el renombrado en línea sobre él.
    pub rename_after: bool,
    pub error: Option<String>,
    /// Elementos que quedan seleccionados al recargar (las copias de Duplicar…).
    pub select: Vec<PathBuf>,
}

//! Tipos de las operaciones de archivos (borrar, renombrar, carpeta nueva, archivo de texto nuevo,
//! accesos directos y deshacer). Copiar, mover, duplicar, «Carpeta nueva con la selección» y
//! devolver de la Papelera son del motor de copia (`transfer`). Cada operación corre en su propio
//! hilo (la implementa cada sistema detrás de `sys::FileOps`) y al terminar avisa a la ventana con
//! `WM_APP_OP_DONE`.

use std::path::{Path, PathBuf};

use crate::WM_APP;
use crate::sys::{CopyError, shown_name};

/// Terminó una operación; el resultado está en el canal.
pub const WM_APP_OP_DONE: u32 = WM_APP + 4;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Delete { paths: Vec<PathBuf>, recycle: bool },
    Rename { path: PathBuf, new_name: String },
    NewFolder { parent: PathBuf },
    /// Archivo de texto vacío «Nuevo documento de texto.txt» (« (2)», « (3)»… si ya existe).
    NewFile { parent: PathBuf },
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
    /// Lo mismo, con la identidad de cada uno al crearlo (disco, número de archivo: `ItemStat::
    /// file_id`). Deshacer manda a la Papelera solo lo que en esa ruta sigue siendo lo mismo (si
    /// apareció otra cosa en su lugar, no se toca). Lo arma el motor de copia y lo deshace él.
    CreatedIds(Vec<(PathBuf, (u64, u64))>),
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

impl Undo {
    /// Lo mismo sin las identidades de lo creado (`CreatedIds` → `Created`): para comparar.
    pub fn without_ids(self) -> Undo {
        match self {
            Undo::CreatedIds(v) => Undo::Created(v.into_iter().map(|(p, _)| p).collect()),
            Undo::Batch(v) => Undo::Batch(v.into_iter().map(Undo::without_ids).collect()),
            other => other,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpResult {
    pub undo: Option<Undo>,
    /// Elemento a enfocar al terminar (renombrado o carpeta nueva).
    pub focus: Option<PathBuf>,
    /// Carpeta o archivo nuevo: se abre el renombrado en línea sobre él.
    pub rename_after: bool,
    /// Lo que salió mal, como lo ve el usuario (frases completas: `failure`); varios, con «; ».
    pub error: Option<String>,
    /// La explicación entera del error, si el aviso la acorta (`failure_detail`): el cartel al
    /// pasar el mouse por el aviso.
    pub error_detail: Option<String>,
    /// Elementos que quedan seleccionados al recargar (las copias de Duplicar…).
    pub select: Vec<PathBuf>,
}

/// Qué hacía `op` con `path` (el elemento del error), para el aviso: «crear la carpeta nueva en
/// «Docs»», «renombrar «a.txt» como «b.txt»».
pub fn doing(op: &Op, path: &Path) -> String {
    let name = shown_name(path);
    match op {
        Op::Delete { recycle: true, .. } => format!("mandar a la Papelera «{name}»"),
        Op::Delete { recycle: false, .. } => format!("eliminar «{name}»"),
        Op::Rename { new_name, .. } => format!("renombrar «{name}» como «{new_name}»"),
        Op::NewFolder { .. } => format!("crear la carpeta nueva en «{name}»"),
        Op::NewFile { .. } => format!("crear el archivo nuevo en «{name}»"),
        Op::Shortcuts { .. } => format!("crear el acceso directo a «{name}»"),
        Op::Undo(_) => format!("deshacer lo hecho con «{name}»"),
    }
}

/// El aviso de una operación que falló: qué no se pudo hacer, con qué elemento, por qué y qué
/// hacer (`CopyError::notice`). Lo usan las plataformas al armar `OpResult::error`.
pub fn failure(op: &Op, e: &CopyError) -> String {
    e.notice(&doing(op, &e.path))
}

/// La explicación entera del error de `op`, si el aviso de `failure` la acorta (lo bloqueado por
/// el sistema).
pub fn failure_detail(op: &Op, e: &CopyError) -> Option<String> {
    e.detail(&doing(op, &e.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::ErrKind;

    /// Cada operación dice qué no se pudo hacer, con el nombre entre « ».
    #[test]
    fn failure_says_what_and_why() {
        let d = PathBuf::from("/u/Docs");
        let f = |op: &Op, kind: ErrKind, p: &Path| failure(op, &CopyError::new(kind, p));
        assert_eq!(
            f(&Op::NewFolder { parent: d.clone() }, ErrKind::Denied, &d),
            "No se pudo crear la carpeta nueva en «Docs»: no tenés permiso. Pedí acceso a quien administra esa carpeta."
        );
        assert_eq!(f(&Op::NewFile { parent: d.clone() }, ErrKind::ReadOnly, &d), "No se pudo crear el archivo nuevo en «Docs»: el disco es de solo lectura. Quitale la protección contra escritura o usá otro disco.");
        let a = d.join("a.txt");
        assert_eq!(
            f(&Op::Rename { path: a.clone(), new_name: "CON".into() }, ErrKind::InvalidName, &a),
            "No se pudo renombrar «a.txt» como «CON»: nombre no válido para ese disco. Sacale caracteres como : * ? \" < > | o no uses nombres reservados como CON o NUL."
        );
        assert_eq!(f(&Op::Delete { paths: vec![a.clone()], recycle: true }, ErrKind::InUse, &a), "No se pudo mandar a la Papelera «a.txt»: lo está usando otro programa. Cerralo y reintentá.");
        assert_eq!(f(&Op::Delete { paths: vec![a.clone()], recycle: false }, ErrKind::NotFound, &a), "No se pudo eliminar «a.txt»: ya no existe.");
        assert_eq!(f(&Op::Shortcuts { targets: vec![a.clone()] }, ErrKind::DiskFull, &a), "No se pudo crear el acceso directo a «a.txt»: el disco está lleno. Liberá espacio y reintentá.");
        assert_eq!(f(&Op::Undo(Undo::Created(vec![a.clone()])), ErrKind::Offline, &a), "No se pudo deshacer lo hecho con «a.txt»: el disco o la red no responde. Revisá la conexión y reintentá.");
        // Bloqueado por Windows: la frase completa, con el nombre de la carpeta.
        let blocked = f(&Op::NewFolder { parent: d.clone() }, ErrKind::Blocked { sure: true }, &d);
        assert!(blocked.starts_with("Windows bloqueó a xdir en «Docs»"), "{blocked}");
        let e = CopyError::new(ErrKind::Blocked { sure: true }, &d);
        assert!(failure_detail(&Op::NewFolder { parent: d.clone() }, &e).is_some_and(|t| t.contains("Permitir una aplicación")));
        assert_eq!(failure_detail(&Op::NewFolder { parent: d.clone() }, &CopyError::new(ErrKind::InUse, &d)), None);
    }
}

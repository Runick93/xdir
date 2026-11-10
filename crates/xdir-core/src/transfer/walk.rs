//! Recorrido inicial de un trabajo: solo metadatos (no lee contenido ni baja nada de la nube).

use std::path::{Path, PathBuf};

use super::{Control, Flow};
use crate::sys::{CopyError, CopyPrims, ErrKind, ItemKind, ItemStat};

/// Un elemento a copiar o mover: su ruta, su ruta relativa a la carpeta de la raíz (empieza con
/// el nombre de la raíz) y sus datos.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Planned {
    pub src: PathBuf,
    pub rel: PathBuf,
    pub stat: ItemStat,
}

/// Cada cuántos elementos se avisa «Preparando… N elementos».
const SEEN_EVERY: u64 = 200;

/// Recorre `root` en preorden (cada carpeta antes que su contenido, por nombre), sin seguir
/// enlaces ni leer contenido. `seen(n)` cada 200 elementos. Falla si no se puede leer la raíz o
/// alguna carpeta (el trabajo anota ese error para esta raíz y sigue con las demás), o con
/// `Cancelled` si se canceló.
pub fn walk(prims: &dyn CopyPrims, root: &Path, ctl: &Control, seen: &mut dyn FnMut(u64)) -> Result<Vec<Planned>, CopyError> {
    let stat = prims.stat(root)?;
    let name = root.file_name().map(PathBuf::from).unwrap_or_else(|| root.to_path_buf());
    let mut out = vec![Planned { src: root.to_path_buf(), rel: name, stat }];
    let mut count = 1u64;
    // Pila de carpetas por recorrer (índices en `out`); se insertan los hijos justo después de
    // su carpeta para mantener el preorden.
    let mut stack = vec![0usize];
    let mut ordered = Vec::with_capacity(16);
    while let Some(i) = stack.pop() {
        ordered.push(i);
        if out[i].stat.kind != ItemKind::Dir {
            continue;
        }
        let (dir, rel) = (out[i].src.clone(), out[i].rel.clone());
        let mut names = prims.read_dir(&dir)?;
        names.sort();
        let first = out.len();
        for n in names {
            if ctl.checkpoint() == Flow::Stop {
                return Err(CopyError::new(ErrKind::Cancelled, root));
            }
            let src = dir.join(&n);
            let stat = prims.stat(&src)?;
            out.push(Planned { src, rel: rel.join(&n), stat });
            count += 1;
            if count.is_multiple_of(SEEN_EVERY) {
                seen(count);
            }
        }
        stack.extend((first..out.len()).rev());
    }
    let mut slots: Vec<Option<Planned>> = out.into_iter().map(Some).collect();
    Ok(ordered.into_iter().map(|i| slots[i].take().unwrap()).collect())
}

/// (elementos, bytes) de un recorrido.
pub fn totals(items: &[Planned]) -> (u64, u64) {
    let bytes = items.iter().filter(|i| i.stat.kind == ItemKind::File).map(|i| i.stat.size).sum();
    (items.len() as u64, bytes)
}

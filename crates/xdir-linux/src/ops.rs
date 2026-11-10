//! Operaciones de archivos de Linux (`xdir_core::sys::FileOps`): copiar, mover, borrar (a la
//! papelera de `crate::trash` o definitivamente), renombrar, carpeta nueva, archivo de texto nuevo
//! y deshacer. Cada operación corre en su propio hilo y avisa con `WM_APP_OP_DONE`. Los nombres
//! repetidos se resuelven como en macOS ("informe copia.txt", "informe copia 2.txt").

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use xdir_core::names;
use xdir_core::ops::{Op, OpResult, Undo, WM_APP_OP_DONE};
use xdir_core::sys::{FileOps, Handle};

use crate::trash;

type Result<T> = std::result::Result<T, String>;

/// Operaciones de archivos de Linux (lo que instala `sys::install`). Sin interfaz no hay
/// confirmación previa al borrar definitivamente: la pide quien arme la operación.
pub struct LinuxFileOps;

impl FileOps for LinuxFileOps {
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op) {
        std::thread::spawn(move || {
            let trash = trash::data_home().map(|d| trash::trash_dir(&d));
            let result = run(op, trash.as_deref()).unwrap_or_else(|e| OpResult { error: Some(e), ..Default::default() });
            if tx.send(result).is_ok() {
                owner.post(WM_APP_OP_DONE, 0, 0);
            }
        });
    }
}

fn ok(undo: Option<Undo>, focus: Option<PathBuf>, rename_after: bool, errors: Vec<String>) -> Result<OpResult> {
    Ok(OpResult { undo, focus, rename_after, error: (!errors.is_empty()).then(|| errors.join("; ")), ..Default::default() })
}

/// Error de una ruta como lo ve el usuario.
fn err_at(p: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", p.display())
}

/// Papelera para borrar; sin ella, error (nunca se borra definitivamente por no encontrarla).
fn need_trash(trash: Option<&Path>) -> Result<&Path> {
    trash.ok_or_else(|| "no se encontró la Papelera ($XDG_DATA_HOME o $HOME)".to_string())
}

/// Manda a la papelera; devuelve el nombre que quedó adentro (para deshacer).
fn to_trash(p: &Path, trash: Option<&Path>) -> Result<String> {
    trash::recycle(p, need_trash(trash)?).map_err(|e| err_at(p, e))
}

/// Corre `op` con la papelera `trash` (`None` si no se encontró: borrar a la papelera falla en
/// vez de borrar definitivamente).
pub fn run(op: Op, trash: Option<&Path>) -> Result<OpResult> {
    match op {
        Op::Copy { sources, dest } => {
            let (mut created, mut errors) = (Vec::new(), Vec::new());
            for src in &sources {
                if dest.starts_with(src) && std::fs::symlink_metadata(src).is_ok_and(|m| m.is_dir()) {
                    errors.push(err_at(src, "no se puede copiar una carpeta dentro de sí misma"));
                    continue;
                }
                let target = copy_target(&dest, src, src.parent() == Some(dest.as_path()));
                match copy_new(src, &target) {
                    Ok(()) => created.push(target),
                    Err(e) => errors.push(err_at(src, e)),
                }
            }
            let focus = created.last().cloned();
            ok((!created.is_empty()).then(|| Undo::Created(created)), focus, false, errors)
        }
        Op::Move { sources, dest } => {
            let (mut moved, mut errors) = (Vec::new(), Vec::new());
            for src in &sources {
                if src.parent() == Some(dest.as_path()) || dest.starts_with(src) {
                    continue;
                }
                let target = copy_target(&dest, src, false);
                match move_path(src, &target) {
                    Ok(()) => moved.push((target, src.clone())),
                    Err(e) => errors.push(err_at(src, e)),
                }
            }
            let focus = moved.last().map(|(t, _)| t.clone());
            ok((!moved.is_empty()).then(|| Undo::Moved(moved)), focus, false, errors)
        }
        Op::Delete { paths, recycle } => {
            let (mut recycled, mut errors) = (Vec::new(), Vec::new());
            for p in &paths {
                if recycle {
                    match to_trash(p, trash) {
                        Ok(name) => recycled.push((name, p.clone())),
                        Err(e) => errors.push(e),
                    }
                } else if let Err(e) = remove_item(p) {
                    errors.push(err_at(p, e));
                }
            }
            ok((!recycled.is_empty()).then(|| Undo::Recycled(recycled)), None, false, errors)
        }
        Op::Rename { path, new_name } => {
            if new_name.contains('/') {
                return Err("el nombre no puede tener «/»".into());
            }
            let target = path.with_file_name(&new_name);
            if target != path && std::fs::symlink_metadata(&target).is_ok() {
                return Err(format!("ya existe «{new_name}»"));
            }
            std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
            let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ok(Some(Undo::Rename { path: target.clone(), old_name }), Some(target), false, Vec::new())
        }
        Op::NewFolder { parent } => {
            let mut target = parent.join(NEW_FOLDER);
            let mut n = 2;
            while std::fs::symlink_metadata(&target).is_ok() {
                target = parent.join(format!("{NEW_FOLDER} ({n})"));
                n += 1;
            }
            std::fs::create_dir(&target).map_err(|e| e.to_string())?;
            ok(Some(Undo::Created(vec![target.clone()])), Some(target), true, Vec::new())
        }
        Op::NewFile { parent } => {
            let target = create_new_file(&parent).map_err(|e| err_at(&parent, e))?;
            ok(Some(Undo::Created(vec![target.clone()])), Some(target), true, Vec::new())
        }
        Op::Duplicate { sources } => {
            let (mut created, mut errors) = (Vec::new(), Vec::new());
            for src in &sources {
                let Some(parent) = src.parent() else { continue };
                let target = copy_target(parent, src, true);
                match copy_new(src, &target) {
                    Ok(()) => created.push(target),
                    Err(e) => errors.push(err_at(src, e)),
                }
            }
            let focus = created.last().cloned();
            let mut r = ok((!created.is_empty()).then(|| Undo::Created(created.clone())), focus, false, errors)?;
            r.select = created;
            Ok(r)
        }
        Op::NewFolderWith { parent, items } => {
            let base = names::folder_with_name(items.len());
            let folder = names::free_name(&parent, &|n| names::numbered_name(&base, true, n), &taken);
            std::fs::create_dir(&folder).map_err(|e| err_at(&folder, e))?;
            let (mut moved, mut errors) = (Vec::new(), Vec::new());
            for src in &items {
                let Some(name) = src.file_name() else { continue };
                let target = folder.join(name);
                // Dos elementos con el mismo nombre (de una búsqueda): el segundo no pisa al primero.
                if taken(&target) {
                    errors.push(format!("ya existe «{}» en «{}»", name.to_string_lossy(), base));
                    continue;
                }
                match move_path(src, &target) {
                    Ok(()) => moved.push((target, src.clone())),
                    Err(e) => errors.push(err_at(src, e)),
                }
            }
            if moved.is_empty() {
                let _ = std::fs::remove_dir(&folder);
                return Err(errors.join("; "));
            }
            // El nombre cuenta lo que de verdad quedó adentro.
            let mut folder = folder;
            if moved.len() != items.len() {
                let base = names::folder_with_name(moved.len());
                let renamed = names::free_name(&parent, &|n| names::numbered_name(&base, true, n), &taken);
                if rename_noreplace(&folder, &renamed).is_ok() {
                    for (now, _) in &mut moved {
                        if let Some(name) = now.file_name() {
                            *now = renamed.join(name);
                        }
                    }
                    folder = renamed;
                }
            }
            let undo = Undo::Batch(vec![Undo::CreatedFolder(folder.clone()), Undo::Moved(moved)]);
            ok(Some(undo), Some(folder), true, errors)
        }
        Op::Restore { items } => {
            let (mut restored, mut errors) = (Vec::new(), Vec::new());
            for item in &items {
                match restore_item(item) {
                    Ok(to) => restored.push(to),
                    Err(e) => errors.push(e),
                }
            }
            let focus = restored.last().cloned();
            let mut r = ok((!restored.is_empty()).then(|| Undo::Created(restored.clone())), focus, false, errors)?;
            r.select = restored;
            Ok(r)
        }
        Op::Shortcuts { targets } => {
            let (mut created, mut errors) = (Vec::new(), Vec::new());
            for target in &targets {
                let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else { continue };
                let name = name.to_string_lossy();
                let link = names::free_name(parent, &|n| names::shortcut_name(&name, names::LinkStyle::Symlink, n), &taken);
                match std::os::unix::fs::symlink(target, &link) {
                    Ok(()) => created.push(link),
                    Err(e) => errors.push(err_at(&link, e)),
                }
            }
            // Con uno solo, queda para renombrar.
            let one = created.len() == 1;
            let focus = created.last().cloned();
            let mut r = ok((!created.is_empty()).then(|| Undo::Created(created.clone())), focus, one, errors)?;
            r.select = created;
            Ok(r)
        }
        Op::Undo(undo) => undo_op(undo, trash),
    }
}

/// Devuelve `<papelera>/files/<nombre>` a su lugar (de su `.trashinfo`), creando la carpeta si ya
/// no está; nunca pisa.
fn restore_item(item: &Path) -> Result<PathBuf> {
    let name = item.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let unknown = || format!("No se sabe de dónde vino «{name}»");
    let files = item.parent().filter(|p| p.file_name().is_some_and(|n| n == "files")).ok_or_else(unknown)?;
    let trash = files.parent().ok_or_else(unknown)?;
    let to = trash::origin(&name, trash).map_err(|_| unknown())?;
    let parent = to.parent().ok_or_else(unknown)?;
    if taken(&to) {
        let folder = parent.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| parent.display().to_string());
        let shown = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(format!("Ya existe «{shown}» en «{folder}»"));
    }
    // La papelera de un disco (no la del usuario) solo devuelve dentro de ese disco; y en ningún
    // caso se siguen enlaces en las carpetas del camino.
    let home = trash::data_home().map(|d| trash::trash_dir(&d));
    let root = if home.as_deref() == Some(trash) { PathBuf::from("/") } else { trash::topdir(trash) };
    prepare_parent(&root, parent, true)?;
    prepare_parent(&root, parent, false)?;
    trash::restore(&name, &to, trash).map_err(|e| err_at(item, e))?;
    Ok(to)
}

/// Deja lista la carpeta `parent` (dentro de `root`) para devolver ahí, sin seguir enlaces: cada
/// carpeta del camino debajo de `root` tiene que ser una carpeta de verdad y las que faltan se
/// crean una por una (`create`); al final la ruta real tiene que seguir dentro de la de `root`.
/// Con `root` = `/` valen los enlaces del sistema (de root); en otro disco, ninguno.
fn prepare_parent(root: &Path, parent: &Path, create: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    let refuse = |p: &Path| format!("«{}» no es una carpeta de ese disco (es un enlace o está afuera): no se devuelve ahí", p.display());
    let rest = parent.strip_prefix(root).map_err(|_| refuse(parent))?;
    let system = root == Path::new("/");
    // Disco del arranque: un enlace del sistema tiene que estar en el mismo disco que «/».
    let root_dev = std::fs::symlink_metadata("/").map(|m| m.dev()).ok();
    let mut cur = root.to_path_buf();
    for c in rest.components() {
        let Component::Normal(n) = c else { return Err(refuse(parent)) };
        cur.push(n);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                // Solo los enlaces de root del primer nivel del disco de arranque («/tmp», «/var»,
                // «/bin»…), en el mismo disco que «/». Uno de root más adentro o en otro disco
                // montado (un pendrive armado) no vale.
                let first_level = cur.parent() == Some(Path::new("/"));
                if !(system && first_level && m.uid() == 0 && Some(m.dev()) == root_dev) {
                    return Err(refuse(&cur));
                }
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(refuse(&cur)),
            Err(e) if e.kind() == io::ErrorKind::NotFound && create => {
                std::fs::create_dir(&cur).map_err(|e| err_at(&cur, e))?;
                if !std::fs::symlink_metadata(&cur).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
                    return Err(refuse(&cur));
                }
            }
            Err(e) => return Err(err_at(&cur, e)),
        }
    }
    let real_root = std::fs::canonicalize(root).map_err(|e| err_at(root, e))?;
    let real = std::fs::canonicalize(parent).map_err(|e| err_at(parent, e))?;
    if real.starts_with(&real_root) { Ok(()) } else { Err(refuse(parent)) }
}

/// Nombre de una carpeta nueva (con " (2)", " (3)"... si ya existe, como en Windows).
const NEW_FOLDER: &str = "Nueva carpeta";

/// Nombre de un archivo de texto nuevo, sin la extensión (`.txt`).
const NEW_FILE: &str = "Nuevo documento de texto";

/// Crea un archivo vacío «Nuevo documento de texto.txt» en `parent` («… (2).txt», «… (3).txt»… si
/// ya existe). `create_new` no pisa nada aunque otro lo cree al mismo tiempo.
fn create_new_file(parent: &Path) -> io::Result<PathBuf> {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{NEW_FILE}.txt") } else { format!("{NEW_FILE} ({n}).txt") };
        let target = parent.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(_) => return Ok(target),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

fn undo_op(undo: Undo, trash: Option<&Path>) -> Result<OpResult> {
    let mut errors = Vec::new();
    match undo {
        Undo::Rename { path, old_name } => {
            let target = path.with_file_name(&old_name);
            // Lo que apareció después con el nombre de antes no se pisa (`rename` lo reemplazaría).
            if std::fs::symlink_metadata(&target).is_ok() {
                return Err(err_at(&target, "ya existe, no se pisa"));
            }
            std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
            return ok(None, Some(target), false, errors);
        }
        Undo::Created(paths) => {
            for p in paths {
                if let Err(e) = to_trash(&p, trash) {
                    errors.push(e);
                }
            }
        }
        Undo::Moved(pairs) => {
            for (now, before) in pairs {
                // Lo que apareció después en el lugar de antes no se pisa (`rename` lo reemplazaría).
                if std::fs::symlink_metadata(&before).is_ok() {
                    errors.push(err_at(&before, "ya existe, no se pisa"));
                    continue;
                }
                if let Err(e) = move_path(&now, &before) {
                    errors.push(err_at(&now, e));
                }
            }
        }
        Undo::Recycled(pairs) => {
            let trash = need_trash(trash)?;
            for (name, original) in pairs {
                if let Err(e) = trash::restore(&name, &original, trash) {
                    errors.push(err_at(&original, e));
                }
            }
        }
        // En orden inverso; los errores de cada paso se juntan.
        Undo::Batch(steps) => {
            for step in steps.into_iter().rev() {
                match undo_op(step, trash) {
                    Ok(r) => errors.extend(r.error),
                    Err(e) => errors.push(e),
                }
            }
        }
        // Solo si quedó vacía; si alguien puso algo adentro, se deja.
        Undo::CreatedFolder(folder) => {
            if std::fs::read_dir(&folder).is_ok_and(|mut it| it.next().is_none())
                && let Err(e) = to_trash(&folder, trash)
            {
                errors.push(e);
            }
        }
    }
    ok(None, None, false, errors)
}

/// Dónde copiar o mover `src` dentro de `dest`: el mismo nombre, o "nombre copia", "nombre copia
/// 2"... (`always_copy`: copiar en la misma carpeta siempre lleva "copia"; ver `names`).
fn copy_target(dest: &Path, src: &Path, always_copy: bool) -> PathBuf {
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let first = dest.join(&name);
    if !always_copy && !taken(&first) {
        return first;
    }
    let is_dir = std::fs::symlink_metadata(src).is_ok_and(|m| m.is_dir());
    names::free_name(dest, &|n| names::copy_name(&name, is_dir, n), &taken)
}

/// ¿Hay algo con ese nombre? (también un enlace roto).
fn taken(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Copia `src` en `dst` sin pisar nunca: si `dst` ya existe, error sin tocarlo. Si falla a
/// mitad, se quita lo copiado (es solo de esta copia).
fn copy_new(src: &Path, dst: &Path) -> io::Result<()> {
    if taken(dst) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "ya existe, no se pisa"));
    }
    // Se borra solo si esta copia creó `dst` (aunque el error de adentro sea AlreadyExists); si
    // lo que falló fue crearlo, lo que apareció en el medio no es nuestro y no se toca.
    let mut created = false;
    let r = copy_tracked(src, dst, &mut created);
    if r.is_err() && created {
        let _ = remove_item(dst);
    }
    r
}

/// Copia un archivo, un enlace simbólico (como enlace) o una carpeta entera. Los permisos de las
/// carpetas se ponen al final, así una carpeta de solo lectura igual se llena. Los archivos
/// especiales (FIFO, sockets, dispositivos) no se copian: abrir una FIFO se queda esperando.
fn copy_item(src: &Path, dst: &Path) -> io::Result<()> {
    copy_tracked(src, dst, &mut false)
}

/// `copy_item` que pone `created` en cuanto crea `dst` (siempre de forma exclusiva).
fn copy_tracked(src: &Path, dst: &Path, created: &mut bool) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)?;
        *created = true;
        Ok(())
    } else if ft.is_dir() {
        std::fs::create_dir(dst)?;
        *created = true;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_item(&entry.path(), &dst.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dst, meta.permissions())
    } else if ft.is_file() {
        // `create_new`: nunca pisa algo que haya aparecido en el medio.
        let mut from = std::fs::File::open(src)?;
        let mut to = std::fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
        *created = true;
        io::copy(&mut from, &mut to)?;
        to.set_permissions(meta.permissions())
    } else {
        Err(io::Error::other("no se copian archivos especiales"))
    }
}

/// Renombra sin pisar nunca lo que haya en `to` (`renameat2` con `RENAME_NOREPLACE`). Si el
/// sistema de archivos no lo admite, mira antes y renombra (como siempre).
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let bad = |_| io::Error::other("ruta inválida");
    let (a, b) = (std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(bad)?, std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(bad)?);
    // SAFETY: dos cadenas C válidas; AT_FDCWD como carpeta de las dos.
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_NOREPLACE) };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::ENOSYS) => {
            if taken(to) {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "ya existe, no se pisa"));
            }
            std::fs::rename(from, to)
        }
        _ => Err(e),
    }
}

/// Borra definitivamente un archivo, enlace o carpeta entera (sin seguir enlaces).
fn remove_item(p: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

/// En qué paso falló un movimiento (`move_path`, `copy_then_remove`).
#[derive(Debug)]
pub enum MoveError {
    /// No se movió nada: el original quedó entero (lo copiado a medias se quitó).
    Copy(io::Error),
    /// El destino ya tiene la copia completa, pero el original no se pudo borrar del todo: el
    /// destino no se toca, es la única copia entera.
    Remove(io::Error),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoveError::Copy(e) => write!(f, "{e}"),
            MoveError::Remove(e) => write!(f, "se copió, pero no se pudo borrar del todo el original: {e}"),
        }
    }
}

/// Mueve: renombra si es el mismo disco; si no (`EXDEV`), copia y borra el original.
pub fn move_path(src: &Path, dst: &Path) -> std::result::Result<(), MoveError> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => copy_then_remove(src, dst),
        Err(e) => Err(MoveError::Copy(e)),
    }
}

/// Mover entre discos: copia `src` en `dst` (sin seguir enlaces) y recién después borra el
/// original. Si la copia falla, el original queda y se quita lo copiado a medias; si lo que falla
/// es borrar el original, la copia completa queda (`MoveError::Remove`).
pub fn copy_then_remove(src: &Path, dst: &Path) -> std::result::Result<(), MoveError> {
    if let Err(e) = copy_new(src, dst) {
        return Err(MoveError::Copy(e));
    }
    remove_item(src).map_err(MoveError::Remove)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Carpeta temporal propia de la prueba.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-linux-ops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_renames_on_conflict() {
        let dir = tmp("copia");
        let src = dir.join("informe.txt");
        std::fs::write(&src, "datos").unwrap();
        // En la misma carpeta siempre lleva "copia"; la segunda vez, "copia 2".
        let r = run(Op::Copy { sources: vec![src.clone()], dest: dir.clone() }, None).unwrap();
        assert_eq!(r.error, None);
        assert_eq!(r.focus, Some(dir.join("informe copia.txt")));
        assert_eq!(r.undo, Some(Undo::Created(vec![dir.join("informe copia.txt")])));
        run(Op::Copy { sources: vec![src.clone()], dest: dir.clone() }, None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("informe copia 2.txt")).unwrap(), "datos");
        // En otra carpeta, el mismo nombre si está libre; si no, "copia". Las carpetas se copian enteras.
        let otra = dir.join("otra");
        std::fs::create_dir_all(otra.join("sub")).unwrap();
        std::fs::write(otra.join("sub/x"), "x").unwrap();
        let dest = dir.join("dest");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("informe.txt"), "viejo").unwrap();
        let r = run(Op::Copy { sources: vec![src.clone(), otra.clone()], dest: dest.clone() }, None).unwrap();
        assert_eq!(r.error, None);
        assert_eq!(std::fs::read_to_string(dest.join("informe.txt")).unwrap(), "viejo");
        assert_eq!(std::fs::read_to_string(dest.join("informe copia.txt")).unwrap(), "datos");
        assert_eq!(std::fs::read_to_string(dest.join("otra/sub/x")).unwrap(), "x");
        // Una carpeta dentro de sí misma no se copia (no termina nunca).
        let r = run(Op::Copy { sources: vec![otra.clone()], dest: otra.join("sub") }, None).unwrap();
        assert!(r.error.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Duplicar dos veces el mismo archivo: «a copia.txt» y «a copia 2.txt», con el mismo
    /// contenido; el deshacer y la selección son las copias.
    #[test]
    fn duplicate_creates_copy_names() {
        let dir = tmp("duplicar");
        let a = dir.join("a.txt");
        std::fs::write(&a, "datos").unwrap();
        let r = run(Op::Duplicate { sources: vec![a.clone(), a.clone()] }, None).unwrap();
        let copies = vec![dir.join("a copia.txt"), dir.join("a copia 2.txt")];
        assert_eq!(r.error, None);
        assert_eq!(r.undo, Some(Undo::Created(copies.clone())));
        assert_eq!(r.select, copies);
        assert_eq!(r.focus, Some(copies[1].clone()));
        for c in &copies {
            assert_eq!(std::fs::read_to_string(c).unwrap(), "datos");
        }
        let f = dir.join("Fotos.2024");
        std::fs::create_dir(&f).unwrap();
        std::fs::write(f.join("x"), "x").unwrap();
        let r = run(Op::Duplicate { sources: vec![f.clone()] }, None).unwrap();
        assert_eq!(r.select, [dir.join("Fotos.2024 copia")]);
        assert_eq!(std::fs::read_to_string(dir.join("Fotos.2024 copia/x")).unwrap(), "x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tres archivos → «Carpeta nueva con 3 elementos» con los tres, enfocada y para renombrar; la
    /// segunda vez «… 2». Deshacer los devuelve y manda la carpeta vacía a la Papelera.
    #[test]
    fn new_folder_with_moves_and_undoes() {
        let dir = tmp("carpeta-con");
        let trash = dir.join("Trash");
        let files: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| dir.join(n)).collect();
        for f in &files {
            std::fs::write(f, "x").unwrap();
        }
        let folder = dir.join("Carpeta nueva con 3 elementos");
        let r = run(Op::NewFolderWith { parent: dir.clone(), items: files.clone() }, Some(&trash)).unwrap();
        assert_eq!(r.error, None);
        assert_eq!((r.focus.clone(), r.rename_after), (Some(folder.clone()), true));
        for n in ["a.txt", "b.txt", "c.txt"] {
            assert!(folder.join(n).is_file(), "{n}");
            assert!(!dir.join(n).exists(), "{n}");
        }
        let more: Vec<PathBuf> = ["d", "e", "f"].iter().map(|n| dir.join(n)).collect();
        for f in &more {
            std::fs::write(f, "y").unwrap();
        }
        let r2 = run(Op::NewFolderWith { parent: dir.clone(), items: more }, Some(&trash)).unwrap();
        assert_eq!(r2.focus, Some(dir.join("Carpeta nueva con 3 elementos 2")));
        let r = run(Op::Undo(r.undo.unwrap()), Some(&trash)).unwrap();
        assert_eq!(r.error, None);
        for f in &files {
            assert!(f.is_file(), "{}", f.display());
        }
        assert!(!folder.exists());
        assert!(trash.join("files/Carpeta nueva con 3 elementos").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El nombre cuenta lo que de verdad se movió: de 3 pedidos, si uno ya no existe, «… con 2
    /// elementos»; el deshacer apunta a la carpeta con su nombre final.
    #[test]
    fn new_folder_with_counts_moved() {
        let dir = tmp("carpeta-cuenta");
        let trash = dir.join("Trash");
        let items: Vec<PathBuf> = ["a", "b", "no-esta"].iter().map(|n| dir.join(n)).collect();
        std::fs::write(&items[0], "a").unwrap();
        std::fs::write(&items[1], "b").unwrap();
        let r = run(Op::NewFolderWith { parent: dir.clone(), items: items.clone() }, Some(&trash)).unwrap();
        let folder = dir.join("Carpeta nueva con 2 elementos");
        assert_eq!(r.focus, Some(folder.clone()));
        assert!(r.error.is_some(), "avisa del que no se pudo mover");
        assert!(folder.join("a").is_file() && folder.join("b").is_file());
        assert!(!dir.join("Carpeta nueva con 3 elementos").exists());
        let Some(Undo::Batch(steps)) = r.undo else { panic!("deshacer") };
        assert_eq!(steps[0], Undo::CreatedFolder(folder.clone()));
        assert_eq!(steps[1], Undo::Moved(vec![(folder.join("a"), items[0].clone()), (folder.join("b"), items[1].clone())]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Si después de crearla se agregó algo a la carpeta, deshacer devuelve lo movido y la deja.
    #[test]
    fn undo_created_folder_keeps_non_empty() {
        let dir = tmp("carpeta-con-llena");
        let trash = dir.join("Trash");
        let a = dir.join("a.txt");
        std::fs::write(&a, "x").unwrap();
        let r = run(Op::NewFolderWith { parent: dir.clone(), items: vec![a.clone()] }, Some(&trash)).unwrap();
        let folder = r.focus.clone().unwrap();
        std::fs::write(folder.join("nuevo.txt"), "n").unwrap();
        let r = run(Op::Undo(r.undo.unwrap()), Some(&trash)).unwrap();
        assert_eq!(r.error, None);
        assert!(a.is_file());
        assert!(folder.join("nuevo.txt").is_file());
        // Ninguno se pudo mover: no queda la carpeta y vuelve el error.
        assert!(run(Op::NewFolderWith { parent: dir.clone(), items: vec![dir.join("no-existe")] }, Some(&trash)).is_err());
        assert!(!dir.join("Carpeta nueva con 1 elemento 2").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Devolver desde la Papelera: vuelve a su carpeta (creándola si se borró), no pisa lo que
    /// haya, y el deshacer los manda otra vez a la Papelera.
    #[test]
    fn restore_op_returns_undo_created() {
        let dir = tmp("devolver");
        let trash = dir.join("Trash");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let (a, b) = (sub.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let na = trash::recycle(&a, &trash).unwrap();
        let nb = trash::recycle(&b, &trash).unwrap();
        std::fs::remove_dir(&sub).unwrap();
        std::fs::write(&b, "nuevo").unwrap();
        let items = vec![trash.join("files").join(&na), trash.join("files").join(&nb)];
        let r = run(Op::Restore { items }, Some(&trash)).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "nuevo");
        assert_eq!(r.undo, Some(Undo::Created(vec![a.clone()])));
        assert_eq!(r.select, [a.clone()]);
        assert!(r.error.unwrap().contains("Ya existe «b.txt» en «"));
        assert!(trash.join("files").join(&nb).exists());
        // Sin `.trashinfo` no se sabe de dónde vino.
        std::fs::write(trash.join("files/suelto"), "s").unwrap();
        let r = run(Op::Restore { items: vec![trash.join("files/suelto")] }, Some(&trash)).unwrap();
        assert_eq!(r.error.as_deref(), Some("No se sabe de dónde vino «suelto»"));
        let r = run(Op::Undo(Undo::Created(vec![a.clone()])), Some(&trash)).unwrap();
        assert_eq!(r.error, None);
        assert!(!a.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// «Crear acceso directo» en Linux: un enlace simbólico «Enlace a a.txt» que se resuelve.
    #[test]
    fn symlink_shortcut_and_target() {
        let dir = tmp("enlace");
        let a = dir.join("a.txt");
        std::fs::write(&a, "a").unwrap();
        let r = run(Op::Shortcuts { targets: vec![a.clone()] }, None).unwrap();
        let link = dir.join("Enlace a a.txt");
        assert_eq!(r.error, None);
        assert_eq!((r.focus.clone(), r.rename_after), (Some(link.clone()), true));
        assert_eq!(r.undo, Some(Undo::Created(vec![link.clone()])));
        assert_eq!(crate::shell::link_target(&link), Some(a.clone()));
        assert_eq!(crate::shell::link_target(&a), None);
        let r = run(Op::Shortcuts { targets: vec![a.clone()] }, None).unwrap();
        assert_eq!(r.focus, Some(dir.join("Enlace a a.txt 2")));
        std::os::unix::fs::symlink("a.txt", dir.join("rel")).unwrap();
        assert_eq!(crate::shell::link_target(&dir.join("rel")), Some(a.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// La papelera de un disco no devuelve a través de un enlace («docs» → otra carpeta) ni a una
    /// ruta absoluta fuera del disco.
    #[test]
    fn restore_refuses_symlinked_folder_and_outside_paths() {
        let dir = tmp("devolver-enlace");
        let (top, outside) = (dir.join("usb"), dir.join("autostart"));
        let trash = top.join(".Trash-1000");
        std::fs::create_dir_all(trash.join("files")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, top.join("docs")).unwrap();
        for (name, path) in [("a", "docs/a".to_string()), ("b", outside.join("b").display().to_string())] {
            std::fs::write(trash.join("files").join(name), name).unwrap();
            std::fs::write(trash.join("info").join(format!("{name}.trashinfo")), format!("[Trash Info]\nPath={path}\n")).unwrap();
        }
        let items = vec![trash.join("files/a"), trash.join("files/b")];
        let r = run(Op::Restore { items }, None).unwrap();
        assert!(r.select.is_empty(), "{:?}", r.select);
        assert!(r.error.is_some());
        assert!(!outside.join("a").exists() && !outside.join("b").exists());
        assert!(trash.join("files/a").exists() && trash.join("files/b").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// La limpieza depende de quién creó el destino: lo creado por esta copia se quita aunque el
    /// error de adentro sea AlreadyExists; lo que ya estaba no se toca.
    #[test]
    fn rename_noreplace_never_overwrites() {
        let dir = tmp("renombrar-sin-pisar");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        assert!(rename_noreplace(&a, &b).is_err());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");
        rename_noreplace(&a, &dir.join("c")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("c")).unwrap(), "a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_cleanup_tracks_creation() {
        let dir = tmp("copia-creada");
        let src = dir.join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a"), "a").unwrap();
        let mut created = false;
        copy_tracked(&src, &dir.join("nueva"), &mut created).unwrap();
        assert!(created);
        let ya = dir.join("ya");
        std::fs::create_dir(&ya).unwrap();
        let mut created = false;
        assert!(copy_tracked(&src, &ya, &mut created).is_err());
        assert!(!created, "no la creó esta copia");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Desde la Papelera del usuario (raíz «/»), un enlace de root más adentro (un pendrive ext4
    /// armado: «/media/u/USB/docs» → «~/.config/autostart») no vale: solo los del primer nivel
    /// del disco de arranque (como «/tmp» o «/bin» en algunas distribuciones). En Docker los tests
    /// corren como root, así que el enlace es de root, como en el pendrive.
    #[test]
    fn restore_refuses_root_owned_link_below_first_level() {
        let dir = tmp("enlace-de-root");
        let (usb, autostart) = (dir.join("media/u/USB"), dir.join("home/u/.config/autostart"));
        std::fs::create_dir_all(&usb).unwrap();
        std::fs::create_dir_all(&autostart).unwrap();
        std::os::unix::fs::symlink(&autostart, usb.join("docs")).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let docs = dir.join("media/u/USB/docs");
        assert!(prepare_parent(Path::new("/"), &docs, true).is_err());
        assert!(prepare_parent(Path::new("/"), &docs.join("sub"), true).is_err());
        assert!(!autostart.join("sub").exists());
        // Sin enlaces, desde «/» anda.
        prepare_parent(Path::new("/"), &dir.join("media/u/USB/fotos"), true).unwrap();
        assert!(dir.join("media/u/USB/fotos").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_falls_back_to_copy() {
        let dir = tmp("mover");
        let src = dir.join("carpeta");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/x"), "contenido").unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        std::os::unix::fs::symlink("a.txt", src.join("enlace")).unwrap();
        let dst = dir.join("destino");
        copy_then_remove(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(std::fs::read_to_string(dst.join("sub/x")).unwrap(), "contenido");
        assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "a");
        // El enlace se copia como enlace, no como lo que apunta.
        assert_eq!(std::fs::read_link(dst.join("enlace")).unwrap(), PathBuf::from("a.txt"));
        // Un archivo suelto.
        let f = dir.join("f.bin");
        std::fs::write(&f, [1u8, 2, 3]).unwrap();
        copy_then_remove(&f, &dir.join("g.bin")).unwrap();
        assert!(!f.exists());
        assert_eq!(std::fs::read(dir.join("g.bin")).unwrap(), [1u8, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_move_does_not_overwrite() {
        let dir = tmp("deshacer-mover");
        let (a, b) = (dir.join("A"), dir.join("B"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("a.txt"), "viejo").unwrap();
        let r = run(Op::Move { sources: vec![a.join("a.txt")], dest: b.clone() }, None).unwrap();
        assert_eq!(r.error, None);
        std::fs::write(a.join("a.txt"), "nuevo").unwrap();
        let r = run(Op::Undo(r.undo.unwrap()), None).unwrap();
        assert!(r.error.is_some());
        assert_eq!(std::fs::read_to_string(a.join("a.txt")).unwrap(), "nuevo");
        assert_eq!(std::fs::read_to_string(b.join("a.txt")).unwrap(), "viejo");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_undo_restores_name() {
        let dir = tmp("renombrar");
        let path = dir.join("viejo.txt");
        std::fs::write(&path, "x").unwrap();
        let r = run(Op::Rename { path: path.clone(), new_name: "nuevo.txt".into() }, None).unwrap();
        assert_eq!(r.focus, Some(dir.join("nuevo.txt")));
        assert!(dir.join("nuevo.txt").exists() && !path.exists());
        // Un nombre ocupado o con "/" no se acepta.
        std::fs::write(dir.join("otro.txt"), "y").unwrap();
        assert!(run(Op::Rename { path: dir.join("nuevo.txt"), new_name: "otro.txt".into() }, None).is_err());
        assert!(run(Op::Rename { path: dir.join("nuevo.txt"), new_name: "a/b".into() }, None).is_err());
        let r = run(Op::Undo(r.undo.unwrap()), None).unwrap();
        assert_eq!(r.focus, Some(path.clone()));
        assert!(path.exists() && !dir.join("nuevo.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_rename_does_not_overwrite() {
        let dir = tmp("deshacer-renombrar");
        let path = dir.join("viejo.txt");
        std::fs::write(&path, "renombrado").unwrap();
        let r = run(Op::Rename { path: path.clone(), new_name: "nuevo.txt".into() }, None).unwrap();
        // Después aparece otro archivo con el nombre de antes: deshacer no lo pisa.
        std::fs::write(&path, "otro").unwrap();
        assert!(run(Op::Undo(r.undo.unwrap()), None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "otro");
        assert_eq!(std::fs::read_to_string(dir.join("nuevo.txt")).unwrap(), "renombrado");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_skips_fifo() {
        let dir = tmp("fifo");
        let fifo = dir.join("tubo");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` es una cadena C válida.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        // En otro hilo: si se intentara copiar, abrir la FIFO se colgaría y el test fallaría a los 5 s.
        let (tx, rx) = std::sync::mpsc::channel();
        let (src, dst) = (fifo.clone(), dir.join("copia"));
        std::thread::spawn(move || {
            let _ = tx.send(copy_item(&src, &dst).is_err());
        });
        let failed = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("copiar la FIFO se colgó");
        assert!(failed, "una FIFO no se copia");
        assert!(std::fs::symlink_metadata(dir.join("copia")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_folder_is_focused_for_rename() {
        let dir = tmp("nueva");
        let r = run(Op::NewFolder { parent: dir.clone() }, None).unwrap();
        assert_eq!(r.focus, Some(dir.join("Nueva carpeta")));
        assert!(r.rename_after);
        assert!(dir.join("Nueva carpeta").is_dir());
        assert_eq!(r.undo, Some(Undo::Created(vec![dir.join("Nueva carpeta")])));
        let r = run(Op::NewFolder { parent: dir.clone() }, None).unwrap();
        assert_eq!(r.focus, Some(dir.join("Nueva carpeta (2)")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Copiar nunca pisa: sobre algo que ya existe, error y lo de antes intacto. Un duplicado que
    /// falla a mitad no deja su «copia» a medias.
    #[test]
    fn copy_never_overwrites_and_failed_duplicate_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("copia-excl");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "nuevo").unwrap();
        std::fs::write(&b, "viejo").unwrap();
        assert!(copy_new(&a, &b).is_err());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "viejo");
        if unsafe { libc::geteuid() } != 0 {
            let f = dir.join("carpeta");
            std::fs::create_dir(&f).unwrap();
            std::fs::write(f.join("ok.txt"), "x").unwrap();
            let locked = f.join("zz-cerrado.txt");
            std::fs::write(&locked, "y").unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            let r = run(Op::Duplicate { sources: vec![f.clone()] }, None).unwrap();
            let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644));
            assert!(r.error.is_some());
            assert!(r.select.is_empty());
            assert!(!dir.join("carpeta copia").exists(), "no queda la copia a medias");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_file_creates_and_renames_on_conflict() {
        let dir = tmp("archivo-nuevo");
        let first = dir.join("Nuevo documento de texto.txt");
        let second = dir.join("Nuevo documento de texto (2).txt");
        let r = run(Op::NewFile { parent: dir.clone() }, None).unwrap();
        assert_eq!(r, OpResult { undo: Some(Undo::Created(vec![first.clone()])), focus: Some(first.clone()), rename_after: true, error: None, ..Default::default() });
        let r = run(Op::NewFile { parent: dir.clone() }, None).unwrap();
        assert_eq!(r, OpResult { undo: Some(Undo::Created(vec![second.clone()])), focus: Some(second.clone()), rename_after: true, error: None, ..Default::default() });
        assert_eq!(std::fs::read(&first).unwrap(), b"");
        assert_eq!(std::fs::read(&second).unwrap(), b"");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_file_error_without_permission() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("new_file_error_without_permission: se saltea como root (los permisos no aplican)");
            return;
        }
        let dir = tmp("archivo-sin-permiso");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = run(Op::NewFile { parent: dir.clone() }, None);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        let e = r.expect_err("sin permiso de escritura no se puede crear");
        assert!(!e.is_empty());
        assert!(!dir.join("Nuevo documento de texto.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_recycles_and_undo_restores() {
        let dir = tmp("borrar");
        let trash = dir.join("Trash");
        let f = dir.join("a.txt");
        std::fs::write(&f, "a").unwrap();
        let r = run(Op::Delete { paths: vec![f.clone()], recycle: true }, Some(&trash)).unwrap();
        assert_eq!(r.error, None);
        assert!(!f.exists());
        assert_eq!(r.undo, Some(Undo::Recycled(vec![("a.txt".into(), f.clone())])));
        run(Op::Undo(r.undo.unwrap()), Some(&trash)).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a");
        // Sin papelera no se borra definitivamente por error.
        let r = run(Op::Delete { paths: vec![f.clone()], recycle: true }, None).unwrap();
        assert!(r.error.is_some());
        assert!(f.exists());
        // Sin papelera a pedido: se borra.
        let r = run(Op::Delete { paths: vec![f.clone()], recycle: false }, None).unwrap();
        assert_eq!((r.error, r.undo), (None, None));
        assert!(!f.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

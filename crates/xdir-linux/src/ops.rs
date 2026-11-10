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
                match copy_item(src, &target) {
                    Ok(()) => created.push(target),
                    Err(e) => {
                        let _ = remove_item(&target);
                        errors.push(err_at(src, e));
                    }
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
                match copy_item(src, &target) {
                    Ok(()) => created.push(target),
                    Err(e) => {
                        let _ = remove_item(&target);
                        errors.push(err_at(src, e));
                    }
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
    std::fs::create_dir_all(parent).map_err(|e| err_at(parent, e))?;
    trash::restore(&name, &to, trash).map_err(|e| err_at(item, e))?;
    Ok(to)
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

/// Copia un archivo, un enlace simbólico (como enlace) o una carpeta entera. Los permisos de las
/// carpetas se ponen al final, así una carpeta de solo lectura igual se llena. Los archivos
/// especiales (FIFO, sockets, dispositivos) no se copian: abrir una FIFO se queda esperando.
fn copy_item(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)
    } else if ft.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_item(&entry.path(), &dst.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dst, meta.permissions())
    } else if ft.is_file() {
        std::fs::copy(src, dst).map(|_| ())
    } else {
        Err(io::Error::other("no se copian archivos especiales"))
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
    if let Err(e) = copy_item(src, dst) {
        let _ = remove_item(dst);
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

//! macOS: operaciones de archivos con `copyfile` (clona en APFS y conserva metadatos), `rename`
//! y la Papelera de `NSFileManager`. Los nombres repetidos se resuelven como el Finder
//! ("informe copia.txt", "informe copia 2.txt"). Cada operación anota cómo deshacerla.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle};
use objc2_foundation::{MainThreadMarker, NSFileManager, NSString, NSURL};

use super::{Op, OpResult, Undo, WM_APP_OP_DONE};
use xdir_core::names;
use crate::platform::Handle;

type Result<T> = std::result::Result<T, String>;

pub fn spawn(owner: Handle, tx: Sender<OpResult>, op: Op) {
    // Borrar sin pasar por la Papelera no se puede deshacer: se confirma antes (en el hilo de la
    // ventana), como el Finder.
    if let Op::Delete { paths, recycle: false } = &op {
        if !confirm_delete(paths) {
            return;
        }
    }
    std::thread::spawn(move || {
        let result = run(op).unwrap_or_else(|e| OpResult { error: Some(e), ..Default::default() });
        if tx.send(result).is_ok() {
            owner.post(WM_APP_OP_DONE, 0, 0);
        }
    });
}

fn confirm_delete(paths: &[PathBuf]) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let alert = NSAlert::new(mtm);
    let what = match paths {
        [one] => format!("«{}»", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        _ => format!("{} elementos", paths.len()),
    };
    alert.setMessageText(&NSString::from_str(&format!("¿Eliminar {what} definitivamente?")));
    alert.setInformativeText(&NSString::from_str("No pasa por la Papelera y no se puede deshacer."));
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.addButtonWithTitle(&NSString::from_str("Eliminar"));
    alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
    alert.runModal() == NSAlertFirstButtonReturn
}

fn ok(undo: Option<Undo>, focus: Option<PathBuf>, rename_after: bool, errors: Vec<String>) -> Result<OpResult> {
    Ok(OpResult { undo, focus, rename_after, error: (!errors.is_empty()).then(|| errors.join("; ")), ..Default::default() })
}

/// Lo que las operaciones usan de la Papelera: los tests ponen las suyas (nunca la real).
struct Hooks<'a> {
    /// Manda a la Papelera y devuelve dónde quedó.
    trash: &'a dyn Fn(&Path) -> Result<PathBuf>,
    /// De dónde vino algo que está en la Papelera.
    origin: &'a dyn Fn(&Path) -> Option<PathBuf>,
    /// Olvida un elemento que salió de la Papelera (registro propio).
    forget: &'a dyn Fn(&Path),
}

const REAL: Hooks<'static> = Hooks { trash: &trash, origin: &crate::trash::origin, forget: &crate::trash::forget };

fn run(op: Op) -> Result<OpResult> {
    run_with(op, &REAL)
}

fn run_with(op: Op, hooks: &Hooks) -> Result<OpResult> {
    let trash = hooks.trash;
    match op {
        Op::Copy { sources, dest } => {
            let (mut created, mut errors) = (Vec::new(), Vec::new());
            for src in &sources {
                let target = copy_target(&dest, src, src.parent() == Some(dest.as_path()));
                match copy(src, &target) {
                    Ok(()) => created.push(target),
                    Err(e) => errors.push(e),
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
                match move_item(src, &target) {
                    Ok(()) => moved.push((target, src.clone())),
                    Err(e) => errors.push(e),
                }
            }
            let focus = moved.last().map(|(t, _)| t.clone());
            ok((!moved.is_empty()).then(|| Undo::Moved(moved)), focus, false, errors)
        }
        Op::Delete { paths, recycle } => {
            let (mut recycled, mut errors) = (Vec::new(), Vec::new());
            for p in &paths {
                if recycle {
                    match trash(p) {
                        Ok(in_trash) => recycled.push((in_trash.to_string_lossy().into_owned(), p.clone())),
                        Err(e) => errors.push(e),
                    }
                } else {
                    let r = if p.is_dir() && !p.is_symlink() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                    if let Err(e) = r {
                        errors.push(format!("{}: {e}", p.display()));
                    }
                }
            }
            ok((!recycled.is_empty()).then(|| Undo::Recycled(recycled)), None, false, errors)
        }
        Op::Rename { path, new_name } => {
            if new_name.contains('/') {
                return Err("el nombre no puede tener «/»".into());
            }
            let target = path.with_file_name(&new_name);
            // Solo cambia mayúsculas: el disco dice que "ya existe" (es el mismo).
            if taken(&target) && !same_file(&path, &target) {
                return Err(format!("ya existe «{new_name}»"));
            }
            rename_excl(&path, &target).map_err(|e| e.to_string())?;
            let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ok(Some(Undo::Rename { path: target.clone(), old_name }), Some(target), false, Vec::new())
        }
        Op::NewFolder { parent } => {
            let mut target = parent.join("carpeta sin título");
            let mut n = 2;
            while target.exists() {
                target = parent.join(format!("carpeta sin título {n}"));
                n += 1;
            }
            std::fs::create_dir(&target).map_err(|e| e.to_string())?;
            ok(Some(Undo::Created(vec![target.clone()])), Some(target), true, Vec::new())
        }
        Op::NewFile { parent } => {
            let target = create_new_file(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            ok(Some(Undo::Created(vec![target.clone()])), Some(target), true, Vec::new())
        }
        Op::Duplicate { sources } => {
            let (mut created, mut errors) = (Vec::new(), Vec::new());
            for src in &sources {
                let Some(parent) = src.parent() else { continue };
                let target = copy_target(parent, src, true);
                match copy(src, &target) {
                    Ok(()) => created.push(target),
                    Err(e) => errors.push(e),
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
            std::fs::create_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            let (mut moved, mut errors) = (Vec::new(), Vec::new());
            for src in &items {
                let Some(name) = src.file_name() else { continue };
                let target = folder.join(name);
                // Dos elementos con el mismo nombre (de una búsqueda): el segundo no pisa al primero.
                if taken(&target) {
                    errors.push(format!("ya existe «{}» en «{}»", name.to_string_lossy(), base));
                    continue;
                }
                match move_item(src, &target) {
                    Ok(()) => moved.push((target, src.clone())),
                    Err(e) => errors.push(e),
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
                if rename_excl(&folder, &renamed).is_ok() {
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
                match restore_one(item, hooks) {
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
                let alias = names::free_name(parent, &|n| names::shortcut_name(&name, names::LinkStyle::Alias, n), &taken);
                match crate::shell::macos::create_alias(target, &alias) {
                    Ok(()) => created.push(alias),
                    Err(e) => errors.push(e),
                }
            }
            // Con uno solo, queda para renombrar.
            let one = created.len() == 1;
            let focus = created.last().cloned();
            let mut r = ok((!created.is_empty()).then(|| Undo::Created(created.clone())), focus, one, errors)?;
            r.select = created;
            Ok(r)
        }
        Op::Undo(undo) => undo_op(undo, hooks),
    }
}

/// Devuelve un elemento de la Papelera a su lugar (creando la carpeta si ya no está); nunca pisa.
fn restore_one(item: &Path, hooks: &Hooks) -> Result<PathBuf> {
    let name = item.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let to = (hooks.origin)(item).ok_or_else(|| format!("No se sabe de dónde vino «{name}»"))?;
    let parent = to.parent().ok_or_else(|| format!("No se sabe de dónde vino «{name}»"))?;
    if taken(&to) {
        let folder = parent.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| parent.display().to_string());
        let shown = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(format!("Ya existe «{shown}» en «{folder}»"));
    }
    // Sin seguir enlaces: un disco armado podría tener «docs» apuntando a otro lado (también una
    // carpeta del registro propio que después se cambió por un enlace).
    let root = crate::trash::volume_root(&to);
    crate::trash::prepare_parent(&root, parent, true)?;
    crate::trash::prepare_parent(&root, parent, false)?;
    move_item(item, &to)?;
    (hooks.forget)(item);
    Ok(to)
}

fn undo_op(undo: Undo, hooks: &Hooks) -> Result<OpResult> {
    let trash = hooks.trash;
    let mut errors = Vec::new();
    match undo {
        // Renombrar varios se deshace en el núcleo (`rename_many::rename_all`): nunca llega acá.
        Undo::Renames(_) => return Err("Renombrar varios se deshace en el núcleo".into()),
        Undo::Rename { path, old_name } => {
            let target = path.with_file_name(&old_name);
            // Lo que apareció después con el nombre de antes no se pisa.
            rename_excl(&path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
            return ok(None, Some(target), false, errors);
        }
        Undo::Created(paths) => {
            for p in paths {
                if let Err(e) = trash(&p) {
                    errors.push(e);
                }
            }
        }
        Undo::Moved(pairs) => {
            for (now, before) in pairs {
                if let Err(e) = move_item(&now, &before) {
                    errors.push(e);
                }
            }
        }
        Undo::Recycled(pairs) => {
            for (in_trash, original) in pairs {
                match move_item(Path::new(&in_trash), &original) {
                    Ok(()) => (hooks.forget)(Path::new(&in_trash)),
                    Err(e) => errors.push(e),
                }
            }
        }
        // En orden inverso; los errores de cada paso se juntan.
        Undo::Batch(steps) => {
            for step in steps.into_iter().rev() {
                match undo_op(step, hooks) {
                    Ok(r) => errors.extend(r.error),
                    Err(e) => errors.push(e),
                }
            }
        }
        Undo::CreatedFolder(folder) => {
            if is_empty_dir(&folder)
                && let Err(e) = trash(&folder)
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
    let is_dir = src.is_dir();
    names::free_name(dest, &|n| names::copy_name(&name, is_dir, n), &taken)
}

/// ¿Carpeta vacía? (un `.DS_Store` que dejó el Finder no cuenta).
fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut it| it.all(|e| e.is_ok_and(|e| e.file_name() == ".DS_Store")))
}

/// ¿Hay algo con ese nombre? (también un enlace roto).
fn taken(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Crea un archivo vacío «Nuevo documento de texto.txt» en `parent` («… (2).txt», «… (3).txt»… si
/// ya existe, como en Windows). `create_new` no pisa nada aunque otro lo cree al mismo tiempo.
fn create_new_file(parent: &Path) -> std::io::Result<PathBuf> {
    let mut n = 1;
    loop {
        let name = if n == 1 { "Nuevo documento de texto.txt".to_string() } else { format!("Nuevo documento de texto ({n}).txt") };
        let target = parent.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(_) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

fn c_path(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| format!("ruta inválida: {}", p.display()))
}

/// Copia un archivo o una carpeta entera, clonando en APFS y conservando los metadatos.
/// Nunca pisa: si `dst` ya existe, error sin tocarlo (`COPYFILE_EXCL`). Si falla a mitad, lo
/// copiado (que es solo de esta copia) se quita.
fn copy(src: &Path, dst: &Path) -> Result<()> {
    if taken(dst) {
        return Err(format!("{}: ya existe, no se pisa", dst.display()));
    }
    copy_unchecked(src, dst)
}

/// `copy` sin mirar antes: el destino lo crea esta copia de forma exclusiva (una carpeta con
/// `create_dir`, un archivo o enlace con `COPYFILE_EXCL`), así lo que haya aparecido en el medio
/// nunca se toca ni se llena. Si falla después, se quita lo que creó.
fn copy_unchecked(src: &Path, dst: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if !meta.is_dir() {
        return copy_entry(src, dst, false);
    }
    // `copyfile` con una carpeta que ya existe copia adentro de ella: la carpeta la crea esta copia.
    std::fs::create_dir(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let result = (|| {
        // Cada elemento de adentro igual: las carpetas, creadas por esta copia; los archivos y
        // enlaces, uno por uno (nunca con COPYFILE_RECURSIVE, que copia adentro de lo que haya).
        for child in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
            let child = child.map_err(|e| format!("{}: {e}", src.display()))?;
            copy_unchecked(&child.path(), &dst.join(child.file_name()))?;
        }
        // Los metadatos de la carpeta (permisos, atributos extendidos, fechas) al final.
        copy_entry(src, dst, true)
    })();
    if result.is_err() {
        let _ = remove_tree(dst);
    }
    result
}

/// `copyfile` de un archivo o enlace `src` en `dst` (que no existe; con `meta_only`, solo los
/// metadatos de una carpeta sobre `dst`). Sin `COPYFILE_RECURSIVE`: con `COPYFILE_EXCL`, un
/// destino que apareció en el medio (archivo, carpeta o enlace) da EEXIST y no se toca.
fn copy_entry(src: &Path, dst: &Path, meta_only: bool) -> Result<()> {
    let (from, to) = (c_path(src)?, c_path(dst)?);
    let flags = if meta_only {
        libc::COPYFILE_METADATA | libc::COPYFILE_NOFOLLOW_SRC
    } else {
        libc::COPYFILE_METADATA | libc::COPYFILE_DATA | libc::COPYFILE_CLONE | libc::COPYFILE_NOFOLLOW_SRC | libc::COPYFILE_EXCL
    };
    // ¿Había algo antes? Si no y después de fallar hay, lo creó esta copia.
    let existed = taken(dst);
    // SAFETY: dos cadenas C válidas; sin estado de copyfile.
    let r = unsafe { libc::copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), flags) };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        // EEXIST: no lo creó esta copia, no se toca.
        if !meta_only && !existed && e.raw_os_error() != Some(libc::EEXIST) && taken(dst) {
            let _ = remove_tree(dst);
        }
        return Err(format!("{}: {e}", src.display()));
    }
    Ok(())
}

/// Borra definitivamente un archivo, enlace o carpeta entera (sin seguir enlaces); lo usa solo
/// la limpieza de una copia fallida.
fn remove_tree(p: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

/// ¿Es el mismo archivo? (mismo disco e inodo: cambiar solo mayúsculas en un disco que no las
/// distingue).
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// Renombra sin pisar nunca lo que haya en `dst` (`renamex_np` con `RENAME_EXCL`). Si `dst` es el
/// mismo archivo (solo cambian mayúsculas), un renombrado común.
fn rename_excl(src: &Path, dst: &Path) -> std::io::Result<()> {
    if same_file(src, dst) {
        return std::fs::rename(src, dst);
    }
    let bad = |_| std::io::Error::other("ruta inválida");
    let (from, to) = (CString::new(src.as_os_str().as_bytes()).map_err(bad)?, CString::new(dst.as_os_str().as_bytes()).map_err(bad)?);
    // SAFETY: dos cadenas C válidas.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Mueve sin pisar: renombra si es el mismo disco; si no, copia y borra el original.
fn move_item(src: &Path, dst: &Path) -> Result<()> {
    match rename_excl(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            if taken(dst) {
                return Err(format!("{}: ya existe, no se pisa", dst.display()));
            }
            copy(src, dst)?;
            let r = if src.is_dir() && !src.is_symlink() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) };
            r.map_err(|e| format!("{}: {e}", src.display()))
        }
        Err(e) => Err(format!("{}: {e}", src.display())),
    }
}

/// Manda a la Papelera; devuelve dónde quedó (para deshacer).
pub(super) fn trash(p: &Path) -> Result<PathBuf> {
    let fm = NSFileManager::defaultManager();
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
    let mut resulting: Option<Retained<NSURL>> = None;
    fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting)).map_err(|e| format!("{}: {}", p.display(), e.localizedDescription()))?;
    let in_trash = resulting.and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string())).unwrap_or_default();
    // Para «Devolver», por si el `.DS_Store` de la Papelera no lo anota.
    if !in_trash.as_os_str().is_empty() {
        crate::trash::log_trashed(&in_trash, p);
    }
    Ok(in_trash)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Carpeta temporal propia de la prueba.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-mac-ops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Duplicar dos veces el mismo archivo: «a copia.txt» y «a copia 2.txt», con el mismo
    /// contenido; el deshacer y la selección son las copias.
    #[test]
    fn duplicate_creates_copy_names() {
        let dir = tmp("duplicar");
        let a = dir.join("a.txt");
        std::fs::write(&a, "datos").unwrap();
        let r = run(Op::Duplicate { sources: vec![a.clone(), a.clone()] }).unwrap();
        let copies = vec![dir.join("a copia.txt"), dir.join("a copia 2.txt")];
        assert_eq!(r.error, None);
        assert_eq!(r.undo, Some(Undo::Created(copies.clone())));
        assert_eq!(r.select, copies);
        assert_eq!(r.focus, Some(copies[1].clone()));
        for c in &copies {
            assert_eq!(std::fs::read_to_string(c).unwrap(), "datos");
        }
        // Una carpeta se copia entera, sin extensión aunque tenga punto.
        let f = dir.join("Fotos.2024");
        std::fs::create_dir(&f).unwrap();
        std::fs::write(f.join("x"), "x").unwrap();
        let r = run(Op::Duplicate { sources: vec![f.clone()] }).unwrap();
        assert_eq!(r.select, [dir.join("Fotos.2024 copia")]);
        assert_eq!(std::fs::read_to_string(dir.join("Fotos.2024 copia/x")).unwrap(), "x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Papelera de prueba: mueve a `dir/.papelera` (nunca a la Papelera real del usuario).
    fn hooks<'a>(trash: &'a dyn Fn(&Path) -> Result<PathBuf>) -> Hooks<'a> {
        Hooks { trash, origin: &|_| None, forget: &|_| {} }
    }

    fn fake_trash(dir: &Path) -> impl Fn(&Path) -> Result<PathBuf> + '_ {
        move |p: &Path| {
            let bin = dir.join(".papelera");
            std::fs::create_dir_all(&bin).unwrap();
            let to = bin.join(p.file_name().unwrap());
            std::fs::rename(p, &to).map_err(|e| e.to_string())?;
            Ok(to)
        }
    }

    /// Tres archivos → «Carpeta nueva con 3 elementos» con los tres, enfocada y para renombrar; la
    /// segunda vez «… 2». Deshacer los devuelve y manda la carpeta vacía a la Papelera.
    #[test]
    fn new_folder_with_moves_and_undoes() {
        let dir = tmp("carpeta-con");
        let trash = fake_trash(&dir);
        let files: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| dir.join(n)).collect();
        for f in &files {
            std::fs::write(f, "x").unwrap();
        }
        let folder = dir.join("Carpeta nueva con 3 elementos");
        let r = run_with(Op::NewFolderWith { parent: dir.clone(), items: files.clone() }, &hooks(&trash)).unwrap();
        assert_eq!(r.error, None);
        assert_eq!((r.focus.clone(), r.rename_after), (Some(folder.clone()), true));
        for n in ["a.txt", "b.txt", "c.txt"] {
            assert!(folder.join(n).is_file(), "{n}");
            assert!(!dir.join(n).exists(), "{n}");
        }
        // Otra vez con otros tres: « 2».
        let more: Vec<PathBuf> = ["d", "e", "f"].iter().map(|n| dir.join(n)).collect();
        for f in &more {
            std::fs::write(f, "y").unwrap();
        }
        let r2 = run_with(Op::NewFolderWith { parent: dir.clone(), items: more }, &hooks(&trash)).unwrap();
        assert_eq!(r2.focus, Some(dir.join("Carpeta nueva con 3 elementos 2")));
        // Deshacer la primera.
        let r = run_with(Op::Undo(r.undo.unwrap()), &hooks(&trash)).unwrap();
        assert_eq!(r.error, None);
        for f in &files {
            assert!(f.is_file(), "{}", f.display());
        }
        assert!(!folder.exists());
        assert!(dir.join(".papelera/Carpeta nueva con 3 elementos").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El nombre cuenta lo que de verdad se movió: de 3 pedidos, si uno ya no existe, «… con 2
    /// elementos»; el deshacer apunta a la carpeta con su nombre final.
    #[test]
    fn new_folder_with_counts_moved() {
        let dir = tmp("carpeta-cuenta");
        let trash = fake_trash(&dir);
        let items: Vec<PathBuf> = ["a", "b", "no-esta"].iter().map(|n| dir.join(n)).collect();
        std::fs::write(&items[0], "a").unwrap();
        std::fs::write(&items[1], "b").unwrap();
        let r = run_with(Op::NewFolderWith { parent: dir.clone(), items: items.clone() }, &hooks(&trash)).unwrap();
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
        let trash = fake_trash(&dir);
        let a = dir.join("a.txt");
        std::fs::write(&a, "x").unwrap();
        let r = run_with(Op::NewFolderWith { parent: dir.clone(), items: vec![a.clone()] }, &hooks(&trash)).unwrap();
        let folder = r.focus.clone().unwrap();
        std::fs::write(folder.join("nuevo.txt"), "n").unwrap();
        let r = run_with(Op::Undo(r.undo.unwrap()), &hooks(&trash)).unwrap();
        assert_eq!(r.error, None);
        assert!(a.is_file());
        assert!(folder.join("nuevo.txt").is_file());
        assert!(!dir.join(".papelera").exists());
        // Ninguno se pudo mover: no queda la carpeta y vuelve el error.
        let gone = dir.join("no-existe.txt");
        assert!(run_with(Op::NewFolderWith { parent: dir.clone(), items: vec![gone] }, &hooks(&trash)).is_err());
        assert!(!dir.join("Carpeta nueva con 1 elemento 2").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Devolver: crea la carpeta de origen si ya no está, no pisa lo que haya con ese nombre y
    /// sin origen conocido avisa; deshacer los vuelve a mandar a la Papelera.
    #[test]
    fn restore_creates_missing_parent_and_refuses_overwrite() {
        let dir = tmp("devolver");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for n in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(bin.join(n), n).unwrap();
        }
        // a vuelve a una carpeta que ya no existe; b choca con uno nuevo; c no tiene origen.
        let a_to = dir.join("borrada/sub/a.txt");
        let b_to = dir.join("b.txt");
        std::fs::write(&b_to, "nuevo").unwrap();
        let origin = |p: &Path| match p.file_name()?.to_str()? {
            "a.txt" => Some(a_to.clone()),
            "b.txt" => Some(b_to.clone()),
            _ => None,
        };
        let forgotten = std::cell::RefCell::new(Vec::new());
        let forget = |p: &Path| forgotten.borrow_mut().push(p.to_path_buf());
        let trash = fake_trash(&dir);
        let h = Hooks { trash: &trash, origin: &origin, forget: &forget };
        let items: Vec<PathBuf> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| bin.join(n)).collect();
        let r = run_with(Op::Restore { items }, &h).unwrap();
        assert_eq!(std::fs::read_to_string(&a_to).unwrap(), "a.txt");
        assert_eq!(std::fs::read_to_string(&b_to).unwrap(), "nuevo");
        assert!(bin.join("b.txt").exists() && bin.join("c.txt").exists());
        assert_eq!(r.undo, Some(Undo::Created(vec![a_to.clone()])));
        assert_eq!(r.select, std::slice::from_ref(&a_to));
        let e = r.error.unwrap();
        assert!(e.contains("Ya existe «b.txt» en «"), "{e}");
        assert!(e.contains("No se sabe de dónde vino «c.txt»"), "{e}");
        assert_eq!(*forgotten.borrow(), [bin.join("a.txt")]);
        // Deshacer: a vuelve a la Papelera.
        run_with(Op::Undo(r.undo.unwrap()), &h).unwrap();
        assert!(!a_to.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// «Crear acceso directo» hace un alias del Finder «a.txt alias» que `link_target` resuelve
    /// y que la carga marca como enlace.
    #[test]
    fn alias_roundtrip() {
        let dir = tmp("alias");
        let a = dir.join("a.txt");
        std::fs::write(&a, "a").unwrap();
        let r = run(Op::Shortcuts { targets: vec![a.clone()] }).unwrap();
        let alias = dir.join("a.txt alias");
        assert_eq!(r.error, None);
        assert_eq!((r.focus.clone(), r.rename_after), (Some(alias.clone()), true));
        assert_eq!(r.select, std::slice::from_ref(&alias));
        assert!(crate::fs::is_alias(&alias));
        assert!(!crate::fs::is_alias(&a));
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        assert_eq!(crate::shell::macos::link_target(&alias).map(|t| canon(&t)), Some(canon(&a)));
        assert_eq!(crate::shell::macos::link_target(&a), None);
        // Un segundo alias: «a.txt alias 2». Un enlace simbólico también se resuelve.
        let r = run(Op::Shortcuts { targets: vec![a.clone()] }).unwrap();
        assert_eq!(r.focus, Some(dir.join("a.txt alias 2")));
        std::os::unix::fs::symlink("a.txt", dir.join("enlace")).unwrap();
        assert_eq!(crate::shell::macos::link_target(&dir.join("enlace")), Some(dir.join("a.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Deshacer un mover no pisa lo que apareció después en el lugar de antes.
    #[test]
    fn undo_move_does_not_overwrite() {
        let dir = tmp("deshacer-mover");
        let trash = fake_trash(&dir);
        let (a, b) = (dir.join("A"), dir.join("B"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("a.txt"), "viejo").unwrap();
        let r = run_with(Op::Move { sources: vec![a.join("a.txt")], dest: b.clone() }, &hooks(&trash)).unwrap();
        assert_eq!(r.error, None);
        std::fs::write(a.join("a.txt"), "nuevo").unwrap();
        let r = run_with(Op::Undo(r.undo.unwrap()), &hooks(&trash)).unwrap();
        assert!(r.error.is_some());
        assert_eq!(std::fs::read_to_string(a.join("a.txt")).unwrap(), "nuevo");
        assert_eq!(std::fs::read_to_string(b.join("a.txt")).unwrap(), "viejo");
        // Lo mismo al devolver de la Papelera con deshacer (`Undo::Recycled`).
        let in_bin = dir.join("bin-a.txt");
        std::fs::write(&in_bin, "papelera").unwrap();
        let undo = Undo::Recycled(vec![(in_bin.to_string_lossy().into_owned(), a.join("a.txt"))]);
        let r = run_with(Op::Undo(undo), &hooks(&trash)).unwrap();
        assert!(r.error.is_some());
        assert_eq!(std::fs::read_to_string(a.join("a.txt")).unwrap(), "nuevo");
        assert!(in_bin.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Deshacer un renombrado no pisa un archivo nuevo con el nombre de antes; cambiar solo
    /// mayúsculas sigue andando.
    #[test]
    fn undo_rename_does_not_overwrite() {
        let dir = tmp("deshacer-renombrar");
        let trash = fake_trash(&dir);
        let path = dir.join("viejo.txt");
        std::fs::write(&path, "renombrado").unwrap();
        let r = run_with(Op::Rename { path: path.clone(), new_name: "nuevo.txt".into() }, &hooks(&trash)).unwrap();
        std::fs::write(&path, "otro").unwrap();
        assert!(run_with(Op::Undo(r.undo.unwrap()), &hooks(&trash)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "otro");
        assert_eq!(std::fs::read_to_string(dir.join("nuevo.txt")).unwrap(), "renombrado");
        let r = run_with(Op::Rename { path: dir.join("nuevo.txt"), new_name: "Nuevo.txt".into() }, &hooks(&trash)).unwrap();
        assert_eq!(r.error, None);
        run_with(Op::Undo(r.undo.unwrap()), &hooks(&trash)).unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name() == "nuevo.txt"));
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
        assert!(copy(&a, &b).is_err());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "viejo");
        if unsafe { libc::geteuid() } != 0 {
            let f = dir.join("carpeta");
            std::fs::create_dir(&f).unwrap();
            std::fs::write(f.join("ok.txt"), "x").unwrap();
            let locked = f.join("zz-cerrado.txt");
            std::fs::write(&locked, "y").unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            let r = run(Op::Duplicate { sources: vec![f.clone()] }).unwrap();
            let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644));
            assert!(r.error.is_some());
            assert!(r.select.is_empty());
            assert!(!dir.join("carpeta copia").exists(), "no queda la copia a medias");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Devolver a una carpeta que es un enlace (un disco armado: «docs» → otra carpeta) no deja
    /// el archivo del otro lado.
    #[test]
    fn restore_refuses_symlinked_folder() {
        let dir = tmp("devolver-enlace");
        let (vol, outside) = (dir.join("vol"), dir.join("LaunchAgents"));
        std::fs::create_dir_all(vol.join(".Trashes/501")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, vol.join("docs")).unwrap();
        let item = vol.join(".Trashes/501/evil.plist");
        std::fs::write(&item, "x").unwrap();
        let to = vol.join("docs/evil.plist");
        let origin = |_: &Path| Some(to.clone());
        let trash = fake_trash(&dir);
        let h = Hooks { trash: &trash, origin: &origin, forget: &|_| {} };
        let r = run_with(Op::Restore { items: vec![item.clone()] }, &h).unwrap();
        assert!(r.error.is_some());
        assert!(item.exists(), "sigue en la Papelera");
        assert!(!outside.join("evil.plist").exists(), "no llegó a la carpeta del enlace");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Una carpeta que aparece en el medio con el nombre del destino (la carrera después de mirar
    /// si existe): la copia falla sin meterse adentro ni borrarla.
    #[test]
    fn copy_dir_never_goes_into_existing_dir() {
        let dir = tmp("copia-carrera");
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/b"), "b").unwrap();
        let dst = dir.join("dst");
        std::fs::create_dir(&dst).unwrap();
        std::fs::write(dst.join("mio.txt"), "mio").unwrap();
        assert!(copy_unchecked(&src, &dst).is_err());
        assert_eq!(std::fs::read_to_string(dst.join("mio.txt")).unwrap(), "mio");
        assert!(!dst.join("src").exists() && !dst.join("sub").exists(), "no copió adentro");
        // A una carpeta nueva: entera, con su contenido.
        copy_unchecked(&src, &dir.join("nueva")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("nueva/sub/b")).unwrap(), "b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un archivo o un enlace sobre un destino que apareció en el medio (archivo, carpeta,
    /// enlace a una carpeta, enlace roto): error, sin copiar adentro ni tocar lo que había.
    #[test]
    fn copy_file_or_link_never_goes_into_existing() {
        use std::os::unix::fs::symlink;
        let dir = tmp("copia-archivo-carrera");
        let f = dir.join("f");
        std::fs::write(&f, "f").unwrap();
        symlink("objetivo", dir.join("srclink")).unwrap();
        let entries = |p: &Path| std::fs::read_dir(p).map(|r| r.count()).unwrap_or(0);
        for (i, src) in [f.clone(), dir.join("srclink")].iter().enumerate() {
            let d = dir.join(format!("d{i}"));
            std::fs::create_dir(&d).unwrap();
            let target = dir.join(format!("t{i}"));
            std::fs::create_dir(&target).unwrap();
            let dirlink = dir.join(format!("dirlink{i}"));
            symlink(&target, &dirlink).unwrap();
            let file = dir.join(format!("dst{i}"));
            std::fs::write(&file, "viejo").unwrap();
            let dangling = dir.join(format!("roto{i}"));
            symlink(dir.join("no-existe"), &dangling).unwrap();
            for dst in [&d, &dirlink, &file, &dangling] {
                assert!(copy_unchecked(src, dst).is_err(), "{} → {}", src.display(), dst.display());
            }
            assert_eq!(entries(&d), 0, "nada adentro de la carpeta");
            assert_eq!(entries(&target), 0, "nada adentro de la carpeta del enlace");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "viejo");
            assert!(std::fs::symlink_metadata(&dangling).unwrap().file_type().is_symlink());
            assert!(!dir.join("no-existe").exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_file_creates_and_renames_on_conflict() {
        let dir = tmp("archivo-nuevo");
        let first = dir.join("Nuevo documento de texto.txt");
        let second = dir.join("Nuevo documento de texto (2).txt");
        let r = run(Op::NewFile { parent: dir.clone() }).unwrap();
        assert_eq!(r, OpResult { undo: Some(Undo::Created(vec![first.clone()])), focus: Some(first.clone()), rename_after: true, error: None, ..Default::default() });
        let r = run(Op::NewFile { parent: dir.clone() }).unwrap();
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
        let r = run(Op::NewFile { parent: dir.clone() });
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
        let e = r.expect_err("sin permiso de escritura no se puede crear");
        assert!(!e.is_empty());
        assert!(!dir.join("Nuevo documento de texto.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

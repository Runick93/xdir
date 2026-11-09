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
        let result = run(op).unwrap_or_else(|e| OpResult { undo: None, focus: None, rename_after: false, error: Some(e) });
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
    Ok(OpResult { undo, focus, rename_after, error: (!errors.is_empty()).then(|| errors.join("; ")) })
}

fn run(op: Op) -> Result<OpResult> {
    match op {
        Op::Copy { sources, dest } => {
            let (mut created, mut errors) = (Vec::new(), Vec::new());
            for src in &sources {
                let target = free_name(&dest, src, src.parent() == Some(dest.as_path()));
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
                let target = free_name(&dest, src, false);
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
            let same = crate::fs::same_path(&path, &target);
            if target.exists() && !same {
                return Err(format!("ya existe «{new_name}»"));
            }
            std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
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
        Op::Undo(undo) => undo_op(undo),
    }
}

fn undo_op(undo: Undo) -> Result<OpResult> {
    let mut errors = Vec::new();
    match undo {
        Undo::Rename { path, old_name } => {
            let target = path.with_file_name(&old_name);
            std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
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
                if let Err(e) = move_item(Path::new(&in_trash), &original) {
                    errors.push(e);
                }
            }
        }
    }
    ok(None, None, false, errors)
}

/// Nombre libre para `src` dentro de `dest`: el mismo, o "nombre copia", "nombre copia 2"...
/// (`always_copy`: copiar en la misma carpeta siempre lleva "copia").
fn free_name(dest: &Path, src: &Path, always_copy: bool) -> PathBuf {
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let first = dest.join(&name);
    if !always_copy && !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 && !src.is_dir() => (name[..dot].to_string(), name[dot..].to_string()),
        _ => (name.clone(), String::new()),
    };
    let mut n = 1;
    loop {
        let candidate = if n == 1 { dest.join(format!("{stem} copia{ext}")) } else { dest.join(format!("{stem} copia {n}{ext}")) };
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn c_path(p: &Path) -> Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| format!("ruta inválida: {}", p.display()))
}

/// Copia un archivo o una carpeta entera, clonando en APFS y conservando los metadatos.
fn copy(src: &Path, dst: &Path) -> Result<()> {
    let (from, to) = (c_path(src)?, c_path(dst)?);
    let flags = libc::COPYFILE_METADATA | libc::COPYFILE_DATA | libc::COPYFILE_RECURSIVE | libc::COPYFILE_CLONE | libc::COPYFILE_NOFOLLOW_SRC;
    let r = unsafe { libc::copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), flags) };
    if r != 0 {
        return Err(format!("{}: {}", src.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Mueve: renombra si es el mismo disco; si no, copia y borra el original.
fn move_item(src: &Path, dst: &Path) -> Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            copy(src, dst)?;
            let r = if src.is_dir() && !src.is_symlink() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) };
            r.map_err(|e| format!("{}: {e}", src.display()))
        }
        Err(e) => Err(format!("{}: {e}", src.display())),
    }
}

/// Manda a la Papelera; devuelve dónde quedó (para deshacer).
fn trash(p: &Path) -> Result<PathBuf> {
    let fm = NSFileManager::defaultManager();
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
    let mut resulting: Option<Retained<NSURL>> = None;
    fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting)).map_err(|e| format!("{}: {}", p.display(), e.localizedDescription()))?;
    Ok(resulting.and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string())).unwrap_or_default())
}

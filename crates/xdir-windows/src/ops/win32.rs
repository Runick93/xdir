//! Operaciones de archivos en Windows con `IFileOperation`: la misma maquinaria del Explorador
//! (progreso, conflictos, confirmaciones, papelera).
//!
//! Un `IFileOperationProgressSink` anota qué se creó, movió o mandó a la papelera, y con eso se
//! arma la operación inversa para deshacer (Ctrl+Z).

use super::{Op, OpResult, Undo, WM_APP_OP_DONE};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMINVOKECOMMANDINFO, FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMMKDIR,
    FOF_RENAMEONCOLLISION, FOFX_RECYCLEONDELETE, FileOperation, IContextMenu, IFileOperation,
    IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName,
    SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, PostMessageW};
use windows::core::{AsImpl, Error, HRESULT, HSTRING, PCWSTR, Ref, Result, implement, s, w};

use crate::platform::{Handle, HandleExt};
use crate::fs::wide_path;
use xdir_core::names;



pub fn spawn(owner: Handle, tx: Sender<OpResult>, op: Op) {
    let hwnd = owner.hwnd();
    let hwnd_raw = hwnd.0 as usize;
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd_raw as *mut _);
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        let result = run(hwnd, op).unwrap_or_else(|e| OpResult { error: Some(e.message().to_string()), ..Default::default() });
        if tx.send(result).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_OP_DONE, WPARAM(0), LPARAM(0));
            }
        }
        if com {
            unsafe { CoUninitialize() };
        }
    });
}

fn run(hwnd: HWND, op: Op) -> Result<OpResult> {
    // Los deshacer de varios pasos usan una operación del Shell por paso.
    if let Op::Undo(undo) = &op {
        return undo_any(hwnd, undo);
    }
    if let Op::Shortcuts { targets } = &op {
        return create_shortcuts(targets);
    }
    // En Windows «Devolver» lo hace el Shell (verbo `undelete` sobre la Papelera virtual).
    if let Op::Restore { .. } = &op {
        return Err(Error::new(E_FAIL, "Se devuelve desde la Papelera del Explorador"));
    }
    let fo: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL)? };
    let recorder: IFileOperationProgressSink = Recorder::default().into();
    unsafe {
        fo.SetOwnerWindow(hwnd)?;
        fo.Advise(&recorder)?;
    }
    let mut flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR;
    let mut rename_after = false;
    let mut focus = None;
    // Carpeta que creó «Carpeta nueva con la selección» (se quita si no se movió nada).
    let mut created_folder: Option<PathBuf> = None;

    match &op {
        Op::Copy { sources, dest } | Op::Move { sources, dest } => {
            let dest_item = item(dest)?;
            for src in sources {
                let src_item = item(src)?;
                unsafe {
                    if matches!(op, Op::Copy { .. }) {
                        fo.CopyItem(&src_item, &dest_item, PCWSTR::null(), None)?;
                    } else {
                        fo.MoveItem(&src_item, &dest_item, PCWSTR::null(), None)?;
                    }
                }
            }
        }
        Op::Duplicate { sources } => {
            // «nombre copia», «nombre copia 2»… como en macOS y Linux (no « - copia» del Explorador):
            // el nombre se calcula acá y se pasa a `CopyItem`, sin `FOF_RENAMEONCOLLISION`.
            for src in sources {
                let (Some(parent), Some(name)) = (src.parent(), src.file_name()) else { continue };
                let name = name.to_string_lossy();
                let is_dir = src.is_dir();
                let target = names::free_name(parent, &|n| names::copy_name(&name, is_dir, n), &|p| std::fs::symlink_metadata(p).is_ok());
                let Some(copy_name) = target.file_name() else { continue };
                unsafe { fo.CopyItem(&item(src)?, &item(parent)?, &HSTRING::from(copy_name), None)? };
            }
        }
        Op::NewFolderWith { parent, items } => {
            let base = names::folder_with_name(items.len());
            let folder = names::free_name(parent, &|n| names::numbered_name(&base, true, n), &|p| std::fs::symlink_metadata(p).is_ok());
            std::fs::create_dir(&folder).map_err(|e| Error::new(E_FAIL, format!("{}: {e}", folder.display())))?;
            created_folder = Some(folder.clone());
            let dest = item(&folder)?;
            for src in items {
                unsafe { fo.MoveItem(&item(src)?, &dest, PCWSTR::null(), None)? };
            }
            rename_after = true;
            focus = Some(folder);
        }
        Op::Delete { paths, recycle } => {
            flags = if *recycle { FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE } else { FILEOPERATION_FLAGS(0) };
            for p in paths {
                unsafe { fo.DeleteItem(&item(p)?, None)? };
            }
        }
        Op::Rename { path, new_name } => {
            unsafe { fo.RenameItem(&item(path)?, &HSTRING::from(new_name.as_str()), None)? };
            focus = path.parent().map(|p| p.join(new_name));
        }
        Op::NewFolder { parent } => {
            flags |= FOF_RENAMEONCOLLISION;
            rename_after = true;
            unsafe {
                fo.NewItem(&item(parent)?, FILE_ATTRIBUTE_DIRECTORY.0, w!("Nueva carpeta"), PCWSTR::null(), None)?;
            }
        }
        Op::NewFile { parent } => {
            // Con FOF_RENAMEONCOLLISION el Shell lo llama «Nuevo documento de texto (2).txt» si ya existe.
            flags |= FOF_RENAMEONCOLLISION;
            rename_after = true;
            unsafe {
                fo.NewItem(&item(parent)?, FILE_ATTRIBUTE_NORMAL.0, w!("Nuevo documento de texto.txt"), PCWSTR::null(), None)?;
            }
        }
        Op::Undo(undo) => return undo_any(hwnd, undo),
        Op::Restore { .. } | Op::Shortcuts { .. } => return Ok(OpResult::default()),
    }

    unsafe {
        fo.SetOperationFlags(flags)?;
        // Si el usuario cancela, PerformOperations devuelve error: no es una falla real.
        let performed = fo.PerformOperations();
        let aborted = fo.GetAnyOperationsAborted().map(|b| b.as_bool()).unwrap_or(false);
        if let Err(e) = performed {
            if !aborted {
                // Solo se quita si quedó vacía.
                if let Some(folder) = &created_folder {
                    let _ = std::fs::remove_dir(folder);
                }
                return Err(e);
            }
        }
    }

    let log = recorder_log(&recorder);
    let mut select = Vec::new();
    let undo = match op {
        Op::Duplicate { .. } => {
            select = log.iter().filter_map(|r| r.created.clone()).map(PathBuf::from).collect::<Vec<_>>();
            focus = select.last().cloned();
            Some(Undo::Created(select.clone()))
        }
        Op::NewFolderWith { .. } => {
            let moved: Vec<(PathBuf, PathBuf)> =
                log.iter().filter_map(|r| Some((PathBuf::from(r.created.clone()?), r.source.clone()?))).collect();
            let folder = created_folder.clone().unwrap_or_default();
            if moved.is_empty() {
                let _ = std::fs::remove_dir(&folder);
                return Err(Error::new(E_FAIL, "No se movió ningún elemento"));
            }
            Some(Undo::Batch(vec![Undo::CreatedFolder(folder), Undo::Moved(moved)]))
        }
        Op::Copy { .. } => Some(Undo::Created(log.iter().filter_map(|r| r.created.clone()).map(PathBuf::from).collect())),
        Op::Move { .. } => Some(Undo::Moved(
            log.iter().filter_map(|r| Some((PathBuf::from(r.created.clone()?), r.source.clone()?))).collect(),
        )),
        Op::Delete { recycle: true, .. } => Some(Undo::Recycled(
            log.iter().filter_map(|r| Some((r.created.clone()?, r.source.clone()?))).collect(),
        )),
        Op::Delete { recycle: false, .. } => None,
        Op::Rename { path, .. } => {
            let new = log.iter().find_map(|r| r.created.clone()).map(PathBuf::from);
            let old_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            focus = new.clone().or(focus);
            match (new, old_name) {
                (Some(new), Some(old_name)) => Some(Undo::Rename { path: new, old_name }),
                _ => None,
            }
        }
        Op::NewFolder { .. } | Op::NewFile { .. } => {
            let created: Vec<PathBuf> = log.iter().filter_map(|r| r.created.clone()).map(PathBuf::from).collect();
            focus = created.first().cloned();
            Some(Undo::Created(created))
        }
        Op::Undo(_) | Op::Restore { .. } | Op::Shortcuts { .. } => None,
    };
    // Nada que deshacer si no pasó nada (todo cancelado u omitido).
    let undo = undo.filter(|u| match u {
        Undo::Created(v) => !v.is_empty(),
        Undo::Moved(v) => !v.is_empty(),
        Undo::Recycled(v) => !v.is_empty(),
        Undo::Rename { .. } | Undo::Batch(_) | Undo::CreatedFolder(_) => true,
    });
    Ok(OpResult { undo, focus, rename_after, error: None, select })
}

/// «Crear acceso directo»: un `.lnk` junto a cada uno («nombre - Acceso directo.lnk», « (2)»…).
fn create_shortcuts(targets: &[PathBuf]) -> Result<OpResult> {
    let (mut created, mut errors) = (Vec::new(), Vec::new());
    for target in targets {
        let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else { continue };
        let name = name.to_string_lossy();
        let lnk = names::free_name(parent, &|n| names::shortcut_name(&name, names::LinkStyle::Lnk, n), &|p| std::fs::symlink_metadata(p).is_ok());
        match crate::shell::win32::create_lnk(target, &lnk) {
            Ok(()) => created.push(lnk),
            Err(e) => errors.push(format!("{}: {}", lnk.display(), e.message())),
        }
    }
    let rename_after = created.len() == 1;
    Ok(OpResult {
        undo: (!created.is_empty()).then(|| Undo::Created(created.clone())),
        focus: created.last().cloned(),
        rename_after,
        error: (!errors.is_empty()).then(|| errors.join("; ")),
        select: created,
    })
}

/// Deshace `undo`: los pasos de un `Batch` en orden inverso (juntando los errores) y una carpeta
/// creada solo si quedó vacía; lo demás con `run_undo`.
fn undo_any(hwnd: HWND, undo: &Undo) -> Result<OpResult> {
    match undo {
        Undo::Batch(steps) => {
            let mut errors = Vec::new();
            for step in steps.iter().rev() {
                match undo_any(hwnd, step) {
                    Ok(r) => errors.extend(r.error),
                    Err(e) => errors.push(e.message().to_string()),
                }
            }
            Ok(OpResult { error: (!errors.is_empty()).then(|| errors.join("; ")), ..Default::default() })
        }
        Undo::CreatedFolder(folder) => {
            let empty = std::fs::read_dir(folder).is_ok_and(|mut it| it.next().is_none());
            if empty {
                unsafe {
                    let fo: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
                    fo.SetOwnerWindow(hwnd)?;
                    fo.SetOperationFlags(FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE)?;
                    fo.DeleteItem(&item(folder)?, None)?;
                    fo.PerformOperations()?;
                }
            }
            Ok(OpResult::default())
        }
        other => {
            let fo: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL)? };
            unsafe { fo.SetOwnerWindow(hwnd)? };
            run_undo(&fo, other)
        }
    }
}

fn run_undo(fo: &IFileOperation, undo: &Undo) -> Result<OpResult> {
    let mut focus = None;
    unsafe {
        fo.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR)?;
        match undo {
            Undo::Rename { path, old_name } => {
                fo.RenameItem(&item(path)?, &HSTRING::from(old_name.as_str()), None)?;
                focus = path.parent().map(|p| p.join(old_name));
            }
            Undo::Created(paths) => {
                fo.SetOperationFlags(FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE)?;
                for p in paths.iter().filter(|p| p.exists()) {
                    fo.DeleteItem(&item(p)?, None)?;
                }
            }
            Undo::Moved(pairs) => {
                for (now, before) in pairs {
                    let (Some(parent), Some(name)) = (before.parent(), before.file_name()) else { continue };
                    fo.MoveItem(&item(now)?, &item(parent)?, &HSTRING::from(name), None)?;
                }
            }
            Undo::Recycled(items) => {
                // "Restaurar" de la papelera devuelve cada elemento a su lugar original.
                for (recycled, original) in items {
                    if restore_from_recycle_bin(recycled).is_err() {
                        let (Some(parent), Some(name)) = (original.parent(), original.file_name()) else { continue };
                        let it: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(recycled.as_str()), None)?;
                        fo.MoveItem(&it, &item(parent)?, &HSTRING::from(name), None)?;
                    }
                    focus = Some(original.clone());
                }
                fo.PerformOperations().ok();
                return Ok(OpResult { focus, ..Default::default() });
            }
            // Los resuelve `undo_any` antes de llegar acá.
            Undo::Batch(_) | Undo::CreatedFolder(_) => return Ok(OpResult::default()),
        }
        fo.PerformOperations()?;
    }
    Ok(OpResult { focus, ..Default::default() })
}

fn restore_from_recycle_bin(parsing_name: &str) -> Result<()> {
    unsafe {
        let it: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(parsing_name), None)?;
        let menu: IContextMenu = it.BindToHandler(None, &BHID_SFUIObject)?;
        // Algunos handlers solo aceptan verbos después de armar el menú.
        let hmenu = CreatePopupMenu()?;
        let _ = menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, 0);
        let info = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            lpVerb: s!("undelete"),
            nShow: 1,
            ..Default::default()
        };
        let result = menu.InvokeCommand(&info);
        let _ = DestroyMenu(hmenu);
        result
    }
}

pub fn item(path: &Path) -> Result<IShellItem> {
    let wide = wide_path(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
}

fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Lo que pasó con cada elemento.
#[derive(Clone, Debug, Default)]
struct Record {
    source: Option<PathBuf>,
    /// Ruta del elemento resultante (en la papelera: su nombre de análisis en el Shell).
    created: Option<String>,
}

#[implement(IFileOperationProgressSink)]
#[derive(Default)]
struct Recorder {
    log: RefCell<Vec<Record>>,
}

fn recorder_log(sink: &IFileOperationProgressSink) -> Vec<Record> {
    // El objeto vive en este hilo: se recupera la implementación para leer lo anotado.
    let imp: &Recorder = unsafe { sink.as_impl() };
    imp.log.borrow().clone()
}

impl Recorder_Impl {
    fn record(&self, hr: HRESULT, source: Ref<IShellItem>, created: Ref<IShellItem>, kind: SIGDN) {
        if hr.is_err() {
            return;
        }
        let source = source.as_ref().and_then(|i| display_name(i, SIGDN_FILESYSPATH)).map(PathBuf::from);
        let created = created.as_ref().and_then(|i| display_name(i, kind));
        self.log.borrow_mut().push(Record { source, created });
    }
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for Recorder_Impl {
    fn StartOperations(&self) -> Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hr: HRESULT) -> Result<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostRenameItem(&self, _: u32, item: Ref<IShellItem>, _: &PCWSTR, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, item, created, SIGDN_FILESYSPATH);
        Ok(())
    }
    fn PreMoveItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostMoveItem(&self, _: u32, item: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, item, created, SIGDN_FILESYSPATH);
        Ok(())
    }
    fn PreCopyItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostCopyItem(&self, _: u32, item: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, item, created, SIGDN_FILESYSPATH);
        Ok(())
    }
    fn PreDeleteItem(&self, _: u32, _: Ref<IShellItem>) -> Result<()> {
        Ok(())
    }
    fn PostDeleteItem(&self, _: u32, item: Ref<IShellItem>, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        // Al reciclar, `created` es el elemento dentro de la papelera.
        self.record(hr, item, created, SIGDN_DESKTOPABSOLUTEPARSING);
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR, _: &PCWSTR, _: u32, hr: HRESULT, created: Ref<IShellItem>) -> Result<()> {
        self.record(hr, Ref::default(), created, SIGDN_FILESYSPATH);
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Crear acceso directo»: «a.txt - Acceso directo.lnk» (y « (2)»), que `link_target` resuelve.
    #[test]
    fn lnk_roundtrip() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::env::temp_dir().join(format!("xdir-win-lnk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        std::fs::write(&a, "a").unwrap();
        let r = create_shortcuts(std::slice::from_ref(&a)).unwrap();
        let lnk = dir.join("a.txt - Acceso directo.lnk");
        assert_eq!(r.error, None);
        assert_eq!((r.focus.clone(), r.rename_after), (Some(lnk.clone()), true));
        assert_eq!(r.undo, Some(Undo::Created(vec![lnk.clone()])));
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        assert_eq!(crate::shell::win32::link_target(&lnk).map(|t| canon(&t)), Some(canon(&a)));
        assert_eq!(crate::shell::win32::link_target(&a), None);
        let r = create_shortcuts(std::slice::from_ref(&a)).unwrap();
        assert_eq!(r.focus, Some(dir.join("a.txt - Acceso directo (2).lnk")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

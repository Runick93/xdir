//! Operaciones de archivos en Windows con `IFileOperation`: la misma maquinaria del Explorador
//! (progreso, conflictos, confirmaciones, papelera).
//!
//! Un `IFileOperationProgressSink` anota qué se creó, movió o mandó a la papelera, y con eso se
//! arma la operación inversa para deshacer (Ctrl+Z).

use super::{Op, OpResult, Undo, WM_APP_OP_DONE};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMINVOKECOMMANDINFO, FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
    FOF_RENAMEONCOLLISION, FOFX_RECYCLEONDELETE, FileOperation, IContextMenu, IFileOperation,
    IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName,
    SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, PostMessageW};
use windows::core::{AsImpl, HRESULT, HSTRING, PCWSTR, Ref, Result, implement, s, w};

use crate::platform::{Handle, HandleExt};
use crate::fs::wide_path;



pub fn spawn(owner: Handle, tx: Sender<OpResult>, op: Op) {
    let hwnd = owner.hwnd();
    let hwnd_raw = hwnd.0 as usize;
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd_raw as *mut _);
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        let result = run(hwnd, op).unwrap_or_else(|e| OpResult {
            undo: None,
            focus: None,
            rename_after: false,
            error: Some(e.message().to_string()),
            ..Default::default()
        });
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
    let fo: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL)? };
    let recorder: IFileOperationProgressSink = Recorder::default().into();
    unsafe {
        fo.SetOwnerWindow(hwnd)?;
        fo.Advise(&recorder)?;
    }
    let mut flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR;
    let mut rename_after = false;
    let mut focus = None;

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
        Op::Undo(undo) => return run_undo(&fo, undo),
    }

    unsafe {
        fo.SetOperationFlags(flags)?;
        // Si el usuario cancela, PerformOperations devuelve error: no es una falla real.
        let performed = fo.PerformOperations();
        let aborted = fo.GetAnyOperationsAborted().map(|b| b.as_bool()).unwrap_or(false);
        if let Err(e) = performed {
            if !aborted {
                return Err(e);
            }
        }
    }

    let log = recorder_log(&recorder);
    let undo = match op {
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
        Op::Undo(_) => None,
    };
    // Nada que deshacer si no pasó nada (todo cancelado u omitido).
    let undo = undo.filter(|u| match u {
        Undo::Created(v) => !v.is_empty(),
        Undo::Moved(v) => !v.is_empty(),
        Undo::Recycled(v) => !v.is_empty(),
        Undo::Rename { .. } => true,
        Undo::Batch(v) => !v.is_empty(),
        Undo::CreatedFolder(_) => true,
    });
    Ok(OpResult { undo, focus, rename_after, error: None, ..Default::default() })
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
                // "Restaurar" de la papelera devuelve cada elemento a su lugar original, si está
                // libre: lo que apareció después con ese nombre no se pisa.
                let mut taken = Vec::new();
                for (recycled, original) in items {
                    if let Some(busy) = super::prims::first_taken(&[original.as_path()], &|p| p.exists()) {
                        taken.push(format!("{}: ya existe, no se pisa", busy.display()));
                        continue;
                    }
                    if restore_from_recycle_bin(recycled).is_err() {
                        let (Some(parent), Some(name)) = (original.parent(), original.file_name()) else { continue };
                        let it: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(recycled.as_str()), None)?;
                        fo.MoveItem(&it, &item(parent)?, &HSTRING::from(name), None)?;
                    }
                    focus = Some(original.clone());
                }
                fo.PerformOperations().ok();
                let error = (!taken.is_empty()).then(|| taken.join("; "));
                return Ok(OpResult { undo: None, focus, rename_after: false, error, ..Default::default() });
            }
            // Solo los arman los trabajos del motor de copia, que los deshace él (`transfer`).
            Undo::Batch(_) | Undo::CreatedFolder(_) => {
                return Ok(OpResult { error: Some("lo deshace el motor de copia".into()), ..Default::default() });
            }
        }
        fo.PerformOperations()?;
    }
    Ok(OpResult { undo: None, focus, rename_after: false, error: None, ..Default::default() })
}

/// `CMIC_MASK_NOASYNC`: `InvokeCommand` no vuelve hasta que el verbo terminó (el motor de copia
/// renombra lo restaurado enseguida). Sin `CMIC_MASK_FLAG_NO_UI`: si el lugar está ocupado, el
/// sistema pregunta (Reemplazar / Omitir) en vez de decidir solo.
const CMIC_MASK_NOASYNC: u32 = 0x0000_0100;

pub(super) fn restore_from_recycle_bin(parsing_name: &str) -> Result<()> {
    unsafe {
        let it: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(parsing_name), None)?;
        let menu: IContextMenu = it.BindToHandler(None, &BHID_SFUIObject)?;
        // Algunos handlers solo aceptan verbos después de armar el menú.
        let hmenu = CreatePopupMenu()?;
        let _ = menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, 0);
        let info = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            fMask: CMIC_MASK_NOASYNC,
            lpVerb: s!("undelete"),
            nShow: 1,
            ..Default::default()
        };
        let result = menu.InvokeCommand(&info);
        let _ = DestroyMenu(hmenu);
        result
    }
}

/// Manda un elemento a la Papelera (para el motor de copia: lo reemplazado). Devuelve su nombre de
/// análisis dentro de la Papelera (lo que usa `restore_from_recycle_bin`), o `None` si no quedó
/// ahí: borrado definitivamente (no entraba) o no se hizo nada. `FOF_WANTNUKEWARNING` hace que el
/// sistema pregunte antes de borrar algo que no entra en la Papelera. COM tiene que estar iniciado
/// en el hilo.
pub(super) fn recycle_one(path: &Path) -> Result<Option<String>> {
    let fo: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL)? };
    let recorder: IFileOperationProgressSink = Recorder::default().into();
    unsafe {
        fo.Advise(&recorder)?;
        fo.SetOperationFlags(FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_WANTNUKEWARNING)?;
        fo.DeleteItem(&item(path)?, None)?;
        fo.PerformOperations()?;
        if fo.GetAnyOperationsAborted().map(|b| b.as_bool()).unwrap_or(false) {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_ABORT));
        }
    }
    Ok(recorder_log(&recorder).into_iter().find_map(|r| r.created))
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

//! Operaciones de archivos con `IFileOperation`: la misma maquinaria del Explorador (progreso,
//! conflictos, confirmaciones, papelera). Cada operación corre en su propio hilo para que la
//! ventana nunca se congele; al terminar avisa con `WM_APP_OP_DONE`.
//!
//! Un `IFileOperationProgressSink` anota qué se creó, movió o mandó a la papelera, y con eso se
//! arma la operación inversa para deshacer (Ctrl+Z).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMINVOKECOMMANDINFO, FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMMKDIR,
    FOF_RENAMEONCOLLISION, FOFX_RECYCLEONDELETE, FileOperation, IContextMenu, IFileOperation,
    IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName,
    SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, PostMessageW, WM_APP};
use windows::core::{AsImpl, HRESULT, HSTRING, PCWSTR, Ref, Result, implement, s, w};

use crate::fs::wide_path;

/// Terminó una operación; el resultado está en el canal.
pub const WM_APP_OP_DONE: u32 = WM_APP + 4;

pub enum Op {
    Copy { sources: Vec<PathBuf>, dest: PathBuf },
    Move { sources: Vec<PathBuf>, dest: PathBuf },
    Delete { paths: Vec<PathBuf>, recycle: bool },
    Rename { path: PathBuf, new_name: String },
    NewFolder { parent: PathBuf },
    Undo(Undo),
}

/// Cómo deshacer una operación ya hecha.
#[derive(Clone, Debug)]
pub enum Undo {
    /// Volver a llamar `path` como `old_name`.
    Rename { path: PathBuf, old_name: String },
    /// Copias o carpetas nuevas: se mandan a la papelera.
    Created(Vec<PathBuf>),
    /// (dónde quedó, dónde estaba).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// (nombre del elemento dentro de la papelera, dónde estaba).
    Recycled(Vec<(String, PathBuf)>),
}

pub struct OpResult {
    pub undo: Option<Undo>,
    /// Elemento a enfocar al terminar (renombrado o carpeta nueva).
    pub focus: Option<PathBuf>,
    /// Carpeta nueva: se abre el renombrado en línea sobre ella.
    pub rename_after: bool,
    pub error: Option<String>,
}

pub fn spawn(hwnd: HWND, tx: Sender<OpResult>, op: Op) {
    let hwnd_raw = hwnd.0 as usize;
    std::thread::spawn(move || {
        let hwnd = HWND(hwnd_raw as *mut _);
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        let result = run(hwnd, op).unwrap_or_else(|e| OpResult {
            undo: None,
            focus: None,
            rename_after: false,
            error: Some(e.message().to_string()),
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
        Op::NewFolder { .. } => {
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
    });
    Ok(OpResult { undo, focus, rename_after, error: None })
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
                return Ok(OpResult { undo: None, focus, rename_after: false, error: None });
            }
        }
        fo.PerformOperations()?;
    }
    Ok(OpResult { undo: None, focus, rename_after: false, error: None })
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

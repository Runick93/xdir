//! Integración con el Shell de Windows: portapapeles compatible con el Explorador, propiedades,
//! menú contextual (con las extensiones instaladas) y objetos de datos para arrastrar.

use std::cell::RefCell;
use std::ffi::OsString;
use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, OleGetClipboard, OleSetClipboard, ReleaseStgMedium};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, BHID_SFObject, BHID_SFUIObject, CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_NORMAL,
    CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DragQueryFileW,
    FOLDERID_ComputerFolder, GCS_VERBW, HDROP, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder,
    IShellItem, IShellItemArray, ILFree, KF_FLAG_DEFAULT, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
    SHCreateShellItemArrayFromIDLists, ShellExecuteExW,
    SHGetKnownFolderItem, SHMultiFileProperties, SHOP_FILEPATH, SHObjectProperties, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemID, HMENU, MF_BYPOSITION, MF_CHECKED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, ModifyMenuW, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{Interface, PCSTR, PCWSTR, PSTR, Result, w};

use crate::platform::{Handle, HandleExt};
use crate::fs::{Location, wide_path};

use super::{Entry, Item, UNPIN_QUICK};

// ---------- elementos y objetos de datos ----------


/// PIDLs absolutos de una lista de elementos. Los que se obtuvieron de una ruta se liberan al
/// soltarlos; los guardados en el elemento solo se prestan.
struct Pidls {
    ptrs: Vec<*const ITEMIDLIST>,
    owned: Vec<*mut ITEMIDLIST>,
}

impl Pidls {
    fn new(items: &[Item]) -> Result<Self> {
        let mut out = Pidls { ptrs: Vec::with_capacity(items.len()), owned: Vec::new() };
        for item in items {
            if let Some(bytes) = &item.pidl {
                out.ptrs.push(bytes.as_ptr() as *const ITEMIDLIST);
                continue;
            }
            let wide = wide_path(&item.path);
            let mut pidl = std::ptr::null_mut();
            unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None)? };
            out.owned.push(pidl);
            out.ptrs.push(pidl);
        }
        Ok(out)
    }
}

impl Drop for Pidls {
    fn drop(&mut self) {
        for &p in &self.owned {
            unsafe { ILFree(Some(p)) };
        }
    }
}

fn item_array(items: &[Item]) -> Result<IShellItemArray> {
    let pidls = Pidls::new(items)?;
    unsafe { SHCreateShellItemArrayFromIDLists(&pidls.ptrs) }
}

/// Objeto de datos del Shell para un grupo de archivos (portapapeles y arrastrar).
pub fn data_object(items: &[Item]) -> Result<IDataObject> {
    unsafe { item_array(items)?.BindToHandler(None, &BHID_DataObject) }
}

// ---------- portapapeles ----------

/// Copia o corta archivos como lo hace el Explorador (se pueden pegar en él y viceversa).
pub fn copy_files(items: &[Item], cut: bool) -> Result<()> {
    let obj = data_object(items)?;
    set_drop_effect(&obj, if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY })?;
    unsafe { OleSetClipboard(&obj) }
}

/// Archivos en el portapapeles y si fueron cortados.
pub fn clipboard_files() -> Option<(Vec<PathBuf>, bool)> {
    let obj = unsafe { OleGetClipboard().ok()? };
    let paths = hdrop_paths(&obj)?;
    let cut = drop_effect(&obj).is_some_and(|e| e.0 & DROPEFFECT_MOVE.0 != 0);
    Some((paths, cut))
}

/// Pega en la carpeta `dest` lo que el portapapeles ofrece como archivos virtuales (FileGroupDescriptor
/// + FileContents) en vez de rutas de disco: lo que se copia por Escritorio remoto (RDP) o desde un
/// zip, un teléfono, etc. Lo hace el verbo "pegar" de la propia carpeta, como el Explorador.
/// Devuelve `false` si el portapapeles no tiene archivos de ese tipo.
pub fn paste_virtual(owner: Handle, dest: &Location) -> Result<bool> {
    unsafe {
        let obj = OleGetClipboard()?;
        let descriptor = FORMATETC {
            cfFormat: RegisterClipboardFormatW(w!("FileGroupDescriptorW")) as u16,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        if obj.QueryGetData(&descriptor).is_err() {
            return Ok(false);
        }
    }
    invoke_verb(owner, dest, &[], "paste")?;
    Ok(true)
}

pub fn clear_clipboard() {
    unsafe {
        let _ = OleSetClipboard(None);
    }
}

/// Rutas de un objeto de datos con formato CF_HDROP (archivos de disco).
pub fn hdrop_paths(obj: &IDataObject) -> Option<Vec<PathBuf>> {
    let fmt = format(CF_HDROP.0);
    unsafe {
        let mut medium = obj.GetData(&fmt).ok()?;
        let hdrop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            paths.push(PathBuf::from(OsString::from_wide(&buf[..len])));
        }
        ReleaseStgMedium(&mut medium);
        (!paths.is_empty()).then_some(paths)
    }
}

fn drop_effect_format() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16 }
}

fn format(cf: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn set_drop_effect(obj: &IDataObject, effect: DROPEFFECT) -> Result<()> {
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, 4)?;
        *(GlobalLock(mem) as *mut u32) = effect.0;
        let _ = GlobalUnlock(mem);
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: mem },
            pUnkForRelease: ManuallyDrop::new(None),
        };
        // `true`: el objeto de datos se queda con la memoria.
        obj.SetData(&format(drop_effect_format()), &medium, true)
    }
}

fn drop_effect(obj: &IDataObject) -> Option<DROPEFFECT> {
    unsafe {
        let mut medium = obj.GetData(&format(drop_effect_format())).ok()?;
        let p = GlobalLock(medium.u.hGlobal) as *const u32;
        let value = (!p.is_null()).then(|| *p);
        let _ = GlobalUnlock(medium.u.hGlobal);
        ReleaseStgMedium(&mut medium);
        value.map(DROPEFFECT)
    }
}

/// Copia texto al portapapeles (para "copiar como ruta").
pub fn copy_text(owner: Handle, text: &str) -> Result<()> {
    let hwnd = owner.hwnd();
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        OpenClipboard(Some(hwnd))?;
        let result = (|| {
            EmptyClipboard()?;
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
            std::ptr::copy_nonoverlapping(wide.as_ptr(), GlobalLock(mem) as *mut u16, wide.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0)))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Diálogo del sistema para elegir una carpeta (`IFileOpenDialog` con `FOS_PICKFOLDERS`).
pub fn pick_folder(owner: Handle, title: &str) -> Option<PathBuf> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::{FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    use windows::core::HSTRING;
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dialog.SetTitle(&HSTRING::from(title));
        // Cancelar devuelve un error: no se eligió nada.
        dialog.Show(Some(owner.hwnd())).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const std::ffi::c_void));
        path.map(PathBuf::from)
    }
}

/// Abre un elemento con su acción por defecto a partir de su PIDL (sirve para elementos
/// virtuales, que no tienen ruta de disco).
pub fn open_item(owner: Handle, item: &Item) {
    let hwnd = owner.hwnd();
    let Ok(pidls) = Pidls::new(std::slice::from_ref(item)) else { return };
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST,
        hwnd,
        lpIDList: pidls.ptrs[0] as *mut _,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe {
        let _ = ShellExecuteExW(&mut info);
    }
}

// ---------- propiedades ----------

/// Abre un archivo de disco con su programa, con `dir` como carpeta de trabajo.
pub fn open_file(owner: Handle, file: &Path, dir: Option<&Path>) {
    let file = wide_path(file);
    let dir = dir.map(wide_path).unwrap_or_else(|| vec![0]);
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(Some(owner.hwnd()), w!("open"), PCWSTR(file.as_ptr()), None, PCWSTR(dir.as_ptr()), SW_SHOWNORMAL);
    }
}

pub fn properties(owner: Handle, items: &[Item]) {
    let hwnd = owner.hwnd();
    unsafe {
        if let [Item { path, pidl: None }] = items {
            let wide = wide_path(path);
            let _ = SHObjectProperties(Some(hwnd), SHOP_FILEPATH, PCWSTR(wide.as_ptr()), PCWSTR::null());
        } else if let Ok(obj) = data_object(items) {
            let _ = SHMultiFileProperties(&obj, 0);
        }
    }
}

// ---------- menú contextual ----------

/// Falta en los bindings; mismo valor que SEE_MASK_UNICODE.
const CMIC_MASK_UNICODE: u32 = 0x4000;

thread_local! {
    /// Menú abierto: sus submenús dibujados por el handler ("Enviar a", "Abrir con")
    /// necesitan recibir los mensajes del menú.
    static OPEN_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

/// Reenvía mensajes de menú al menú contextual abierto. Devuelve Some si los manejó.
pub fn handle_menu_message(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    OPEN_MENU.with(|m| {
        let menu = m.borrow().clone()?;
        unsafe {
            if let Ok(cm3) = menu.cast::<IContextMenu3>() {
                let mut result = LRESULT(0);
                cm3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)).ok()?;
                return Some(result);
            }
            if let Ok(cm2) = menu.cast::<IContextMenu2>() {
                cm2.HandleMenuMsg(msg, wparam, lparam).ok()?;
                return Some(LRESULT(0));
            }
        }
        None
    })
}

/// Verbos que resuelve la propia aplicación en vez del Shell.
/// «Propiedades» abre el panel de información de xdir («Más en el sistema…» abre el diálogo del Shell).
pub const OWN_VERBS: [&str; 6] = ["rename", "delete", "cut", "copy", "paste", "properties"];


/// Identificadores propios: por encima de los que el Shell recibe (1..=0x7FFF).
const OWN_BASE: u32 = 0x8000;

/// Texto de la entrada propia que se omite si el Shell ya trae el verbo `copyaspath`.
const COPY_AS_PATH: &str = "Copiar como ruta de acceso";

fn append_entries(hmenu: HMENU, entries: &[Entry]) {
    for e in entries {
        unsafe {
            match e {
                Entry::Sep => {
                    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                Entry::Item { id, label, enabled, checked } => {
                    let mut flags = MF_STRING;
                    if !enabled {
                        flags |= MF_GRAYED;
                    }
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    let text: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    let _ = AppendMenuW(hmenu, flags, (OWN_BASE + id) as usize, PCWSTR(text.as_ptr()));
                }
                Entry::Sub(label, children) => {
                    let Ok(sub) = CreatePopupMenu() else { continue };
                    append_entries(sub, children);
                    let text: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                    let _ = AppendMenuW(hmenu, MF_POPUP, sub.0 as usize, PCWSTR(text.as_ptr()));
                }
            }
        }
    }
}

/// Muestra el menú contextual del Shell. Sin `items`, es el menú del fondo de la carpeta.
/// Con `intercept`, devuelve el verbo elegido si es uno de `OWN_VERBS` para que lo resuelva
/// la aplicación; el resto (y todo, en carpetas virtuales) lo ejecuta el Shell y devuelve
/// `"shell"`. Las `own` van arriba en el fondo y al final (tras un separador) en los elementos; si
/// se elige una, devuelve `own:<id>`. Con `pinned` (la carpeta
/// única ya está anclada al Acceso rápido) el verbo "Anclar a Acceso rápido" del Shell se muestra
/// como "Desanclar de Acceso rápido" y, si se elige, devuelve `UNPIN_QUICK`.
#[allow(clippy::too_many_arguments)]
pub fn context_menu(
    owner: Handle,
    folder: &Location,
    items: &[Item],
    at: (f32, f32),
    extended: bool,
    intercept: bool,
    own: &[Entry],
    pinned: bool,
) -> Option<String> {
    let hwnd = owner.hwnd();
    // Punto del área de la ventana (DIPs) → pantalla (píxeles).
    let screen = unsafe {
        let k = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) as f32 / 96.0;
        let mut pt = POINT { x: (at.0 * k) as i32, y: (at.1 * k) as i32 };
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt);
        pt
    };
    let menu = build_menu(hwnd, folder, items).ok()?;
    unsafe {
        let hmenu = CreatePopupMenu().ok()?;
        let mut flags = CMF_NORMAL | CMF_CANRENAME;
        if extended {
            flags |= CMF_EXTENDEDVERBS;
        }
        // En el fondo, las entradas propias van arriba; en los elementos, después de las del Shell
        // para que «Abrir» siga primero. Los ids propios no dependen de la posición (`OWN_BASE`).
        let background = items.is_empty();
        if background {
            append_entries(hmenu, own);
            if !own.is_empty() {
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            }
        }
        let first = GetMenuItemCount(Some(hmenu)).max(0) as u32;
        if menu.QueryContextMenu(hmenu, first, 1, 0x7FFF, flags).is_err() {
            let _ = DestroyMenu(hmenu);
            return None;
        }
        if !background {
            // Si el Shell ya trae su «Copiar como ruta de acceso» (verbo `copyaspath`), el propio
            // sobra.
            let shell_copy = find_verb(&menu, hmenu, "copyaspath").is_some();
            let own: Vec<Entry> = own
                .iter()
                .filter(|e| !(shell_copy && matches!(e, Entry::Item { label, .. } if label == COPY_AS_PATH)))
                .cloned()
                .collect();
            if own.iter().any(|e| !matches!(e, Entry::Sep)) {
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
                append_entries(hmenu, &own);
            }
        }
        let unpin_id = if pinned { relabel_pin(&menu, hmenu) } else { None };
        OPEN_MENU.with(|m| *m.borrow_mut() = Some(menu.clone()));
        let cmd = TrackPopupMenuEx(hmenu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, screen.x, screen.y, hwnd, None).0;
        OPEN_MENU.with(|m| *m.borrow_mut() = None);

        let mut chosen = None;
        if cmd >= OWN_BASE as i32 {
            chosen = Some(format!("own:{}", cmd as u32 - OWN_BASE));
        } else if cmd > 0 {
            let id = (cmd - 1) as usize;
            let verb = command_verb(&menu, id);
            if unpin_id == Some(id) {
                chosen = Some(UNPIN_QUICK.to_string());
            } else if intercept && verb.as_deref().is_some_and(|v| OWN_VERBS.contains(&v)) {
                chosen = verb;
            } else {
                let dir = match folder {
                    Location::Path(p) => wide_path(p),
                    _ => vec![0],
                };
                let info = CMINVOKECOMMANDINFOEX {
                    cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                    fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                    hwnd,
                    // El comando se identifica por su número (MAKEINTRESOURCE).
                    lpVerb: PCSTR(id as *const u8),
                    lpVerbW: PCWSTR(id as *const u16),
                    lpDirectoryW: PCWSTR(dir.as_ptr()),
                    nShow: SW_SHOWNORMAL.0,
                    ptInvoke: screen,
                    ..Default::default()
                };
                let _ = menu.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO);
                chosen = Some("shell".to_string());
            }
        }
        let _ = DestroyMenu(hmenu);
        chosen
    }
}

/// Cambia el texto del verbo "pintohome" del menú a "Desanclar de Acceso rápido" y devuelve su
/// número de comando.
unsafe fn relabel_pin(menu: &IContextMenu, hmenu: HMENU) -> Option<usize> {
    let (pos, id) = find_verb(menu, hmenu, "pintohome")?;
    unsafe {
        let _ = ModifyMenuW(hmenu, pos, MF_BYPOSITION | MF_STRING, id as usize, w!("Desanclar de &Acceso rápido"));
    }
    Some((id - 1) as usize)
}

/// Posición y número de comando del verbo `verb` del Shell en el primer nivel del menú.
fn find_verb(menu: &IContextMenu, hmenu: HMENU, verb: &str) -> Option<(u32, u32)> {
    unsafe {
        for pos in 0..GetMenuItemCount(Some(hmenu)).max(0) {
            let id = GetMenuItemID(hmenu, pos);
            // Separadores, submenús y entradas propias no son comandos del Shell.
            if id == 0 || id == u32::MAX || id >= OWN_BASE {
                continue;
            }
            if command_verb(menu, (id - 1) as usize).as_deref() == Some(verb) {
                return Some((pos as u32, id));
            }
        }
        None
    }
}

/// Anclar una carpeta al Acceso rápido de Windows (verbo nativo `pintohome`).
pub fn pin_quick_access(owner: Handle, path: &Path) -> Result<()> {
    invoke_verb(owner, &Location::Path(path.to_path_buf()), &[Item::from(path.to_path_buf())], "pintohome")
}

/// Desanclar una carpeta del Acceso rápido (verbo nativo `unpinfromhome`). El Shell solo ofrece
/// ese verbo sobre el elemento del propio Acceso rápido, no sobre la carpeta de disco.
pub fn unpin_quick_access(owner: Handle, path: &Path) -> Result<()> {
    let hwnd = owner.hwnd();
    let item = crate::fs::quick_access_item(path).ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
    unsafe {
        let menu: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject)?;
        let hmenu = CreatePopupMenu()?;
        let _ = menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL);
        let info = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR(c"unpinfromhome".as_ptr() as *const u8),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let result = menu.InvokeCommand(&info);
        let _ = DestroyMenu(hmenu);
        result
    }
}

/// Ejecuta un verbo del Shell ("delete", "undelete"...) sobre elementos, como si se
/// eligiera en su menú contextual.
pub fn invoke_verb(owner: Handle, folder: &Location, items: &[Item], verb: &str) -> Result<()> {
    let hwnd = owner.hwnd();
    let menu = build_menu(hwnd, folder, items)?;
    let verb = std::ffi::CString::new(verb).unwrap_or_default();
    unsafe {
        let hmenu = CreatePopupMenu()?;
        let _ = menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL);
        let info = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd,
            lpVerb: PCSTR(verb.as_ptr() as *const u8),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let result = menu.InvokeCommand(&info);
        let _ = DestroyMenu(hmenu);
        result
    }
}

fn build_menu(hwnd: HWND, folder: &Location, items: &[Item]) -> Result<IContextMenu> {
    unsafe {
        if !items.is_empty() {
            return item_array(items)?.BindToHandler(None, &BHID_SFUIObject);
        }
        let folder_item: IShellItem = match folder {
            Location::Computer => SHGetKnownFolderItem(&FOLDERID_ComputerFolder, KF_FLAG_DEFAULT, None)?,
            Location::Path(p) => crate::ops::item(p)?,
            Location::Shell { parsing, .. } => crate::ops::item(Path::new(parsing))?,
        };
        let sf: IShellFolder = folder_item.BindToHandler(None, &BHID_SFObject)?;
        sf.CreateViewObject(hwnd)
    }
}

fn command_verb(menu: &IContextMenu, id: usize) -> Option<String> {
    let mut buf = [0u16; 128];
    unsafe {
        // Con GCS_VERBW el búfer es de UTF-16 aunque el tipo diga PSTR.
        menu.GetCommandString(id, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (len > 0).then(|| String::from_utf16_lossy(&buf[..len]).to_lowercase())
}

/// Modo de los menús nativos (contextual, favoritos): `None` sigue al modo de apps de Windows,
/// `Some(true/false)` los fuerza a oscuro o claro.
///
/// Usa `SetPreferredAppMode` de uxtheme (exportada por ordinal, estable desde Windows 10 1809),
/// la misma que usan el Explorador y el Bloc de notas.
pub fn set_menu_mode(dark: Option<bool>) {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(set_mode) = GetProcAddress(uxtheme, PCSTR(135 as *const u8)) {
            let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            // 1 = AllowDark (sigue al sistema), 2 = ForceDark, 3 = ForceLight.
            set_mode(match dark {
                None => 1,
                Some(true) => 2,
                Some(false) => 3,
            });
        }
        if let Some(flush) = GetProcAddress(uxtheme, PCSTR(136 as *const u8)) {
            let flush: extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}

/// Raíz de una ruta ("C:\"), para decidir si arrastrar mueve o copia.
pub fn volume_of(p: &Path) -> Option<OsString> {
    p.components().next().map(|c| c.as_os_str().to_ascii_lowercase())
}

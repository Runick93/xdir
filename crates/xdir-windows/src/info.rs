//! Panel de información en Windows (`sys::Info`): atributos y fechas con `GetFileAttributesExW`,
//! tamaño en disco con `GetCompressedFileSizeW` redondeado al clúster, etiquetas
//! (`System.Keywords`) y comentario (`System.Comment`) por `IPropertyStore` (nunca con
//! `GPS_OPENSLOWITEM`, y nada de eso si el archivo está solo en la nube), disco con
//! `GetVolumeInformationW` / `GetDiskFreeSpaceExW`, dueño y acceso del usuario por la ACL (sin abrir
//! el archivo) y «Abrir con» por `SHAssocEnumHandlers`. Elegir una app lo abre una vez con ella;
//! «Cambiar todos…» abre el diálogo del sistema (Windows no deja cambiar el predeterminado desde
//! otra app). Los permisos son solo para ver. Todo corre en hilos de trabajo con COM.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree, PROPERTYKEY, S_OK};
use windows::Win32::Security::Authorization::{BuildTrusteeWithSidW, GetEffectiveRightsFromAclW, GetNamedSecurityInfoW, SE_FILE_OBJECT, TRUSTEE_W};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetTokenInformation, LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::Storage::EnhancedStorage::{PKEY_Comment, PKEY_Keywords};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_FLAGS_AND_ATTRIBUTES,
    GetCompressedFileSizeW, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetFileAttributesExW, GetFileExInfoStandard,
    GetVolumeInformationW, SetFileAttributesW, WIN32_FILE_ATTRIBUTE_DATA,
};
use windows::Win32::System::Com::StructuredStorage::{InitPropVariantFromStringVector, PROPVAR_CHANGE_FLAGS, PROPVARIANT, PropVariantChangeType};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::{GPS_HANDLERPROPERTIESONLY, GPS_READWRITE, IPropertyStore, IPropertyStoreCapabilities, SHGetPropertyStoreFromParsingName};
use windows::Win32::UI::Shell::{
    ASSOC_FILTER_RECOMMENDED, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR, ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW, IAssocHandler, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_FORCE_REGISTRATION, OAIF_REGISTER_EXT, OPENASINFO,
    SHAssocEnumHandlers, SHFILEINFOW, SHGFI_SYSICONINDEX, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW, SHOpenWithDialog,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};
use xdir_core::info::{AppRef, Flag, InfoChange, InfoReply, InfoRequest, ItemInfo, OpenWith, Perms, Tag, VolumeData};
use xdir_core::shell::Item;
use xdir_core::sys::{Handle, Info};

use crate::metadata::Value;
use crate::platform::HandleExt;

/// Tipos de `GetDriveTypeW` (están en `WindowsProgramming`, que no se usa para nada más).
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;

/// Derechos de lectura y escritura de datos (de `FILE_GENERIC_READ` / `FILE_GENERIC_WRITE`).
const FILE_READ_DATA: u32 = 0x0001;
const FILE_WRITE_DATA: u32 = 0x0002;

/// El servicio que instala `sys::install`.
pub struct Win32Info;

/// COM del hilo de trabajo: se inicia al crearlo y se cierra al soltarlo (solo si se inició).
struct Com(bool);

impl Com {
    fn init() -> Com {
        Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

impl Info for Win32Info {
    fn request(&self, req: InfoRequest) {
        let spawned = std::thread::Builder::new().name("xdir-informacion".into()).spawn(move || {
            let _com = Com::init();
            xdir_core::info::answer(req, &read);
        });
        debug_assert!(spawned.is_ok(), "hilo de información");
    }

    fn apply(&self, owner: Handle, tx: Sender<InfoReply>, generation: u64, change: InfoChange) {
        // «Cambiar todos…» es un diálogo modal del sistema: va en el hilo de interfaz (el que
        // llama, con COM ya iniciado), dueño de la ventana, como los otros diálogos de xdir. Desde
        // otro hilo no deshabilitaría la ventana de xdir.
        if let InfoChange::ChangeAll { path, .. } = &change {
            let error = change_all(owner, path).err();
            xdir_core::info::send_applied(owner, &tx, generation, change.section(), error);
            return;
        }
        let spawned = std::thread::Builder::new().name("xdir-informacion-cambio".into()).spawn(move || {
            let _com = Com::init();
            let section = change.section();
            let error = apply(owner, &change).err();
            xdir_core::info::send_applied(owner, &tx, generation, section, error);
        });
        debug_assert!(spawned.is_ok(), "hilo de cambio de información");
    }

    fn more_in_system(&self, owner: Handle, items: &[Item]) -> bool {
        crate::shell::win32::properties(owner, items);
        true
    }
}

/// Tamaño en disco: `size` redondeado hacia arriba al clúster (0 queda en 0).
pub fn round_to_cluster(size: u64, cluster: u64) -> u64 {
    if cluster == 0 {
        return size;
    }
    size.div_ceil(cluster) * cluster
}

fn ft(t: windows::Win32::Foundation::FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

/// «C:\» de una ruta (o la ruta de red `\\servidor\recurso\`).
fn root_of(path: &Path) -> Option<PathBuf> {
    let mut root = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => root.push(c.as_os_str()),
            _ => break,
        }
    }
    (!root.as_os_str().is_empty()).then_some(root)
}

/// ¿La ruta es un disco entero (`C:\`)?
fn is_root(path: &Path) -> bool {
    root_of(path).is_some_and(|r| r.as_os_str().len() == path.as_os_str().len() || path.parent().is_none())
}

fn cluster_size(root: &Path) -> u64 {
    let (mut spc, mut bps) = (0u32, 0u32);
    if unsafe { GetDiskFreeSpaceW(&HSTRING::from(root), Some(&mut spc), Some(&mut bps), None, None) }.is_err() {
        return 0;
    }
    spc as u64 * bps as u64
}

fn type_name(path: &Path, attrs: u32) -> String {
    let mut info = SHFILEINFOW::default();
    let ok = unsafe { SHGetFileInfoW(&HSTRING::from(path), FILE_FLAGS_AND_ATTRIBUTES(attrs), Some(&mut info), size_of::<SHFILEINFOW>() as u32, SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES) };
    if ok == 0 {
        return String::new();
    }
    let len = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(info.szTypeName.len());
    String::from_utf16_lossy(&info.szTypeName[..len])
}

/// Todo lo del elemento (con COM ya inicializado en el hilo), sin abrir lo que está en la nube.
pub fn read(path: &Path) -> ItemInfo {
    let wide = HSTRING::from(path);
    let mut info = ItemInfo { path: path.to_path_buf(), last_opened_is_access: true, ..ItemInfo::default() };
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    let ok = unsafe { GetFileAttributesExW(&wide, GetFileExInfoStandard, (&mut data as *mut WIN32_FILE_ATTRIBUTE_DATA).cast()) }.is_ok();
    let attrs = if ok { data.dwFileAttributes } else { 0 };
    info.is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0 || (!ok && is_root(path));
    info.cloud_only = attrs & (FILE_ATTRIBUTE_OFFLINE.0 | FILE_ATTRIBUTE_RECALL_ON_OPEN.0 | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0) != 0;
    if ok {
        info.created = ft(data.ftCreationTime);
        info.modified = ft(data.ftLastWriteTime);
        info.last_opened = ft(data.ftLastAccessTime);
    }
    info.kind_name = type_name(path, if attrs == 0 { FILE_ATTRIBUTE_NORMAL.0 } else { attrs });
    if !info.is_dir {
        info.size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
        let mut high = 0u32;
        let low = unsafe { GetCompressedFileSizeW(&wide, Some(&mut high)) };
        let compressed = if low == u32::MAX { info.size } else { ((high as u64) << 32) | low as u64 };
        info.on_disk = root_of(path).map_or(compressed, |r| round_to_cluster(compressed, cluster_size(&r)));
    }
    if is_root(path) {
        info.volume = Some(volume(path));
        info.kind_name = match unsafe { GetDriveTypeW(&wide) } {
            DRIVE_REMOVABLE => "Disco extraíble".into(),
            DRIVE_FIXED => "Disco local".into(),
            DRIVE_REMOTE => "Unidad de red".into(),
            DRIVE_CDROM => "Unidad de CD/DVD".into(),
            _ => "Disco".into(),
        };
    } else {
        info.flags = vec![(Flag::Hidden, attrs & FILE_ATTRIBUTE_HIDDEN.0 != 0, ok), (Flag::ReadOnly, attrs & FILE_ATTRIBUTE_READONLY.0 != 0, ok && !info.is_dir)];
        // En una carpeta, «solo lectura» marca las carpetas personalizadas (desktop.ini), no
        // protege nada: se muestra pero no se cambia.
        // Etiquetas y comentario: del property store, salvo lo que está solo en la nube (no se abre).
        if !info.cloud_only {
            let (tags, comment) = keywords_and_comment(&wide);
            info.tags = tags;
            info.comment = comment;
        }
        if !info.is_dir {
            info.open_with = open_with(path);
        }
    }
    info.perms = perms(&wide);
    info
}

fn volume(root: &Path) -> VolumeData {
    let wide = HSTRING::from(root);
    let mut fs = [0u16; 64];
    let format = match unsafe { GetVolumeInformationW(&wide, None, None, None, None, Some(&mut fs)) } {
        Ok(()) => String::from_utf16_lossy(&fs[..fs.iter().position(|&c| c == 0).unwrap_or(fs.len())]),
        Err(_) => String::new(),
    };
    let (mut free, mut total) = (0u64, 0u64);
    let _ = unsafe { GetDiskFreeSpaceExW(&wide, Some(&mut free), Some(&mut total), None) };
    VolumeData { total, free, format }
}

/// ¿El formato admite esa propiedad? Sin la interfaz de capacidades, se supone que sí.
fn writable(store: &IPropertyStore, key: &PROPERTYKEY) -> bool {
    match store.cast::<IPropertyStoreCapabilities>() {
        Ok(caps) => (unsafe { caps.IsPropertyWritable(key) }) == S_OK,
        Err(_) => true,
    }
}

/// Etiquetas y comentario del manejador de propiedades del formato: sin manejador (un .txt) o si
/// no admite la propiedad (`IsPropertyWritable` = S_FALSE), `None`.
fn keywords_and_comment(wide: &HSTRING) -> (Option<Vec<Tag>>, Option<String>) {
    let Ok(store) = (unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(wide, None, GPS_HANDLERPROPERTIESONLY) }) else { return (None, None) };
    let get = |key: &PROPERTYKEY| unsafe { store.GetValue(key) }.ok().map(Value);
    let tags = writable(&store, &PKEY_Keywords)
        .then(|| get(&PKEY_Keywords).map(|v| v.strings()).unwrap_or_default().into_iter().map(|name| Tag { name, color: None }).collect());
    let comment = writable(&store, &PKEY_Comment).then(|| get(&PKEY_Comment).and_then(|v| v.string()).unwrap_or_default());
    (tags, comment)
}

/// Texto de un SID: «DOMINIO\nombre» (o el nombre solo).
fn account_name(sid: PSID) -> String {
    let (mut n, mut d) = (256u32, 256u32);
    let (mut name, mut domain) = (vec![0u16; 256], vec![0u16; 256]);
    let mut use_ = SID_NAME_USE::default();
    if unsafe { LookupAccountSidW(PCWSTR::null(), sid, Some(PWSTR(name.as_mut_ptr())), &mut n, Some(PWSTR(domain.as_mut_ptr())), &mut d, &mut use_) }.is_err() {
        return String::new();
    }
    let name = String::from_utf16_lossy(&name[..n as usize]);
    let domain = String::from_utf16_lossy(&domain[..d as usize]);
    if domain.is_empty() { name } else { format!("{domain}\\{name}") }
}

/// SID del usuario del proceso (en un búfer que hay que conservar mientras se usa).
/// El búfer es de `u64`: `TOKEN_USER` lleva un puntero y tiene que quedar alineado.
pub(crate) fn token_user() -> Option<Vec<u64>> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), (buf.len() * 8) as u32, &mut len).is_ok();
        let _ = CloseHandle(token);
        ok.then_some(buf)
    }
}

/// Dueño y si el usuario actual puede leer y escribir, de la ACL (sin abrir el archivo).
fn perms(wide: &HSTRING) -> Option<Perms> {
    unsafe {
        let mut owner = PSID::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let r = GetNamedSecurityInfoW(wide, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, Some(&mut owner), None, Some(&mut dacl), None, &mut sd);
        if r != ERROR_SUCCESS {
            return None;
        }
        let owner_name = account_name(owner);
        let (mut can_read, mut can_write) = (true, true);
        if let Some(buf) = token_user() {
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut trustee = TRUSTEE_W::default();
            BuildTrusteeWithSidW(&mut trustee, Some(user.User.Sid));
            let mut mask = 0u32;
            if !dacl.is_null() && GetEffectiveRightsFromAclW(dacl, &trustee, &mut mask) == ERROR_SUCCESS {
                can_read = mask & FILE_READ_DATA != 0;
                can_write = mask & FILE_WRITE_DATA != 0;
            }
        }
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        Some(Perms { owner: owner_name, group: String::new(), mode: 0, editable: false, can_read, can_write })
    }
}

fn pwstr_string(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { p.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

/// Las apps recomendadas para la extensión (por el tipo: no se lee el archivo), con su ícono.
fn handlers(path: &Path) -> Vec<IAssocHandler> {
    let Some(ext) = path.extension().map(|e| format!(".{}", e.to_string_lossy())) else { return Vec::new() };
    let Ok(list) = (unsafe { SHAssocEnumHandlers(&HSTRING::from(ext), ASSOC_FILTER_RECOMMENDED) }) else { return Vec::new() };
    let mut out = Vec::new();
    loop {
        let mut one = [None];
        let mut got = 0u32;
        if unsafe { list.Next(&mut one, Some(&mut got)) }.is_err() || got == 0 {
            break;
        }
        if let Some(h) = one[0].take() {
            out.push(h);
        }
        if out.len() >= 30 {
            break;
        }
    }
    out
}

fn app_ref(h: &IAssocHandler) -> Option<AppRef> {
    let id = pwstr_string(unsafe { h.GetName() }.ok()?)?;
    let name = pwstr_string(unsafe { h.GetUIName() }.ok()?).unwrap_or_else(|| id.clone());
    let mut icon_path = PWSTR::null();
    let mut index = 0i32;
    let icon = if unsafe { h.GetIconLocation(&mut icon_path, &mut index) }.is_ok() {
        let p = pwstr_string(icon_path).unwrap_or_default();
        let mut info = SHFILEINFOW::default();
        let ok = unsafe { SHGetFileInfoW(&HSTRING::from(p), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) };
        if ok != 0 { info.iIcon } else { 0 }
    } else {
        0
    };
    Some(AppRef { id, name, icon })
}

/// «Abrir con»: la primera recomendada es la predeterminada (así las ordena el sistema).
/// Un texto de `AssocQueryStringW` para la extensión (sin leer el archivo).
fn assoc(ext: &HSTRING, what: ASSOCSTR) -> Option<String> {
    let mut len = 0u32;
    let _ = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, what, ext, PCWSTR::null(), None, &mut len) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    if unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, what, ext, PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut len) }.is_err() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end])).filter(|s| !s.is_empty())
}

/// La app predeterminada de la extensión: la de la lista con ese ejecutable o, si no está (apps
/// de la tienda, programas sin registrar), el ejecutable con su nombre amigable.
fn default_app(path: &Path, apps: &[AppRef]) -> Option<AppRef> {
    let ext = HSTRING::from(format!(".{}", path.extension()?.to_string_lossy()));
    let exe = assoc(&ext, ASSOCSTR_EXECUTABLE);
    let name = assoc(&ext, ASSOCSTR_FRIENDLYAPPNAME);
    if let Some(e) = &exe
        && let Some(a) = apps.iter().find(|a| a.id.eq_ignore_ascii_case(e))
    {
        return Some(a.clone());
    }
    if let Some(n) = &name
        && let Some(a) = apps.iter().find(|a| a.name.eq_ignore_ascii_case(n))
    {
        return Some(a.clone());
    }
    let id = exe?;
    let mut info = SHFILEINFOW::default();
    let ok = unsafe { SHGetFileInfoW(&HSTRING::from(&id), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) };
    Some(AppRef { name: name.unwrap_or_else(|| id.clone()), icon: if ok != 0 { info.iIcon } else { 0 }, id })
}

fn open_with(path: &Path) -> Option<OpenWith> {
    let apps: Vec<AppRef> = handlers(path).iter().filter_map(app_ref).collect();
    let current = default_app(path, &apps);
    (!apps.is_empty() || current.is_some()).then_some(OpenWith { current, apps, per_file: false, system_dialog: true })
}

/// El error de E/S como lo ve el usuario.
fn win_text(e: &windows::core::Error) -> String {
    match e.code().0 as u32 {
        0x8007_0005 => "No tenés permiso".into(),
        0x8007_0002 | 0x8007_0003 => "Ya no existe".into(),
        // ERROR_SHARING_VIOLATION y los de los property store (STG_E_SHAREVIOLATION,
        // STG_E_LOCKVIOLATION).
        0x8007_0020 | 0x8003_0020 | 0x8003_0021 => "Lo está usando otro programa".into(),
        _ => e.message(),
    }
}

fn joined(errors: Vec<String>) -> Result<(), String> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().unwrap_or_default()),
        n => Err(format!("{} (y {} errores más)", errors[0], n - 1)),
    }
}

fn set_attr(path: &Path, bit: u32, on: bool) -> Result<(), String> {
    let wide = HSTRING::from(path);
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe { GetFileAttributesExW(&wide, GetFileExInfoStandard, (&mut data as *mut WIN32_FILE_ATTRIBUTE_DATA).cast()) }.map_err(|e| win_text(&e))?;
    let attrs = data.dwFileAttributes;
    let next = if on { attrs | bit } else { attrs & !bit };
    if next == attrs {
        return Ok(());
    }
    unsafe { SetFileAttributesW(&wide, FILE_FLAGS_AND_ATTRIBUTES(next)) }.map_err(|e| win_text(&e))
}

fn write_store(path: &Path, f: &dyn Fn(&IPropertyStore) -> windows::core::Result<()>) -> Result<(), String> {
    let wide = HSTRING::from(path);
    if crate::metadata::cloud_only(&wide) {
        return Err("Está solo en la nube: no se cambia sin bajarlo".into());
    }
    let store = unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&wide, None, GPS_READWRITE) }.map_err(|e| win_text(&e))?;
    f(&store).map_err(|e| win_text(&e))?;
    unsafe { store.Commit() }.map_err(|e| win_text(&e))
}

fn set_keywords(path: &Path, add: &[String], remove: &[String]) -> Result<(), String> {
    write_store(path, &|store| {
        let mut names = unsafe { store.GetValue(&PKEY_Keywords) }.map(|v| Value(v).strings()).unwrap_or_default();
        names.retain(|n| !remove.contains(n));
        for a in add {
            if !names.contains(a) {
                names.push(a.clone());
            }
        }
        let wides: Vec<HSTRING> = names.iter().map(HSTRING::from).collect();
        let ptrs: Vec<PCWSTR> = wides.iter().map(|w| PCWSTR(w.as_ptr())).collect();
        let value = unsafe { InitPropVariantFromStringVector(Some(&ptrs)) }?;
        unsafe { store.SetValue(&PKEY_Keywords, &value) }
    })
}

fn set_comment(path: &Path, text: &str) -> Result<(), String> {
    write_store(path, &|store| {
        let src = PROPVARIANT::from(text);
        let mut value = PROPVARIANT::default();
        unsafe { PropVariantChangeType(&mut value, &src, PROPVAR_CHANGE_FLAGS(0), VT_LPWSTR) }?;
        unsafe { store.SetValue(&PKEY_Comment, &value) }
    })
}

/// Lo abre una vez con la app `id` (la de `SHAssocEnumHandlers`).
fn open_once(path: &Path, id: &str) -> Result<(), String> {
    let handler = handlers(path).into_iter().find(|h| app_ref(h).is_some_and(|a| a.id == id)).ok_or("Esa app ya no está")?;
    let data = crate::shell::win32::data_object(&[Item::from(path.to_path_buf())]).map_err(|e| win_text(&e))?;
    unsafe { handler.Invoke(&data) }.map_err(|e| win_text(&e))
}

/// «Cambiar todos…»: el diálogo «Abrir con» del sistema ya con «Usar siempre» (desde Windows 8 una
/// app no puede cambiar el predeterminado sola).
fn change_all(owner: Handle, path: &Path) -> Result<(), String> {
    let file = HSTRING::from(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_FORCE_REGISTRATION | OAIF_EXEC,
    };
    match unsafe { SHOpenWithDialog(Some(owner.hwnd()), &info) } {
        Ok(()) => Ok(()),
        // Cancelado por el usuario: no es un error.
        Err(e) if e.code().0 as u32 == 0x8007_04C7 => Ok(()),
        Err(e) => Err(win_text(&e)),
    }
}

/// Aplica un cambio (en el hilo que llama, con COM).
pub fn apply(owner: Handle, change: &InfoChange) -> Result<(), String> {
    let each = |paths: &[PathBuf], f: &dyn Fn(&Path) -> Result<(), String>| joined(paths.iter().filter_map(|p| f(p).err()).collect());
    match change {
        InfoChange::Tags { paths, add, remove } => each(paths, &|p| set_keywords(p, add, remove)),
        InfoChange::Comment { path, text } => set_comment(path, text),
        InfoChange::Flag { paths, flag, on } => {
            let bit = match flag {
                Flag::Hidden => FILE_ATTRIBUTE_HIDDEN.0,
                Flag::ReadOnly if paths.iter().any(|p| p.is_dir()) => return Err("En una carpeta, «Solo lectura» no se cambia desde acá".into()),
                Flag::ReadOnly => FILE_ATTRIBUTE_READONLY.0,
                Flag::Locked => return Err("Esto no existe en Windows".into()),
            };
            each(paths, &|p| set_attr(p, bit, *on))
        }
        InfoChange::HideExt { .. } => Err("Esto no existe en Windows".into()),
        InfoChange::OpenWith { paths, app } => each(paths, &|p| open_once(p, app)),
        InfoChange::ChangeAll { path, .. } => change_all(owner, path),
        InfoChange::Perms { .. } => Err("En Windows los permisos se cambian en «Más en el sistema…»".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn com() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("xdir-win-info").join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_to_cluster_values() {
        assert_eq!(round_to_cluster(0, 4096), 0);
        assert_eq!(round_to_cluster(1, 4096), 4096);
        assert_eq!(round_to_cluster(4096, 4096), 4096);
        assert_eq!(round_to_cluster(4097, 4096), 8192);
        assert_eq!(round_to_cluster(10, 0), 10);
    }

    #[test]
    fn win_info_reads_general() {
        com();
        let d = temp("general");
        let f = d.join("informe.txt");
        std::fs::write(&f, vec![b'x'; 5000]).unwrap();
        let i = read(&f);
        assert_eq!(i.size, 5000);
        assert!(i.on_disk >= 5000 || i.on_disk == 0, "{}", i.on_disk);
        assert!(!i.kind_name.is_empty());
        assert!(i.created > 0 && i.modified > 0);
        assert!(!i.is_dir && !i.cloud_only && i.last_opened_is_access);
        let root = read(Path::new("C:\\"));
        let v = root.volume.expect("C:\\ es un disco");
        assert!(v.total > 0 && !v.format.is_empty());
    }

    #[test]
    fn win_hidden_readonly_roundtrip() {
        com();
        let d = temp("atributos");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let flag = |i: &ItemInfo, f: Flag| i.flags.iter().find(|x| x.0 == f).unwrap().1;
        apply(Handle::default(), &InfoChange::Flag { paths: vec![f.clone()], flag: Flag::Hidden, on: true }).unwrap();
        apply(Handle::default(), &InfoChange::Flag { paths: vec![f.clone()], flag: Flag::ReadOnly, on: true }).unwrap();
        let i = read(&f);
        assert!(flag(&i, Flag::Hidden) && flag(&i, Flag::ReadOnly));
        apply(Handle::default(), &InfoChange::Flag { paths: vec![f.clone()], flag: Flag::ReadOnly, on: false }).unwrap();
        apply(Handle::default(), &InfoChange::Flag { paths: vec![f.clone()], flag: Flag::Hidden, on: false }).unwrap();
        let i = read(&f);
        assert!(!flag(&i, Flag::Hidden) && !flag(&i, Flag::ReadOnly));
    }

    /// En una carpeta «Solo lectura» se ve pero no se cambia (marca carpetas personalizadas).
    #[test]
    fn win_folder_readonly_not_editable() {
        com();
        let d = temp("carpeta");
        let ro = read(&d).flags.iter().find(|x| x.0 == Flag::ReadOnly).copied().unwrap();
        assert!(!ro.2);
        assert!(apply(Handle::default(), &InfoChange::Flag { paths: vec![d], flag: Flag::ReadOnly, on: true }).is_err());
    }

    #[test]
    fn win_keywords_unsupported_on_txt() {
        com();
        let d = temp("txt");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(read(&f).tags, None);
    }

    #[test]
    fn win_owner_present() {
        com();
        let d = temp("dueno");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let p = read(&f).perms.expect("permisos");
        assert!(!p.owner.is_empty());
        assert!(p.can_read && p.can_write && !p.editable);
    }
}

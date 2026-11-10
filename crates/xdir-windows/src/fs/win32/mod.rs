//! Lectura de carpetas y vigilancia de cambios en Windows (Shell, FindFirstFile y
//! ReadDirectoryChangesW).

use super::*;

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, PROPERTYKEY, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_SYSTEM, FILE_FLAGS_AND_ATTRIBUTES, FILE_NOTIFY_CHANGE_ATTRIBUTES,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
    FILE_NOTIFY_CHANGE_SIZE, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadDirectoryChangesW,
    FindClose, FindFirstFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetLongPathNameW,
    WIN32_FIND_DATAW,
};
use windows::Win32::Storage::EnhancedStorage::{PKEY_Capacity, PKEY_DateModified, PKEY_ItemNameDisplay, PKEY_Size};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::SystemServices::{SFGAO_FOLDER, SFGAO_HIDDEN, SFGAO_STREAM};
use windows::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
    FOLDERID_Pictures, FOLDERID_Videos, IEnumShellItems, ILFree, ILGetSize, IShellItem, IShellItem2,
    KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SHFILEINFOW, SHGFI_DISPLAYNAME, SHGFI_FLAGS,
    SHGFI_PIDL, SHGFI_SYSICONINDEX, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
    SHGetIDListFromObject, SHGetKnownFolderPath, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING,
    SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::platform::Handle;
use windows::core::{GUID, HSTRING, Interface, PCWSTR, PWSTR};



/// Interpreta lo escrito en la barra de dirección como elemento del Shell
/// ("shell:RecycleBinFolder", "::{GUID}", ...).
pub fn shell_location(text: &str) -> Option<Location> {
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(text), None).ok()? };
    if let Some(path) = item_name(&item, SIGDN_FILESYSPATH) {
        return Some(Location::Path(PathBuf::from(path)));
    }
    Some(Location::Shell {
        parsing: item_name(&item, SIGDN_DESKTOPABSOLUTEPARSING)?,
        name: item_name(&item, SIGDN_NORMALDISPLAY).unwrap_or_else(|| text.to_string()),
        parent: None,
    })
}


/// Lanza la lectura de `location` en un hilo. Se abandona si `current` deja de valer `generation`.
pub fn spawn_load(
    hwnd: Handle,
    tx: Sender<LoadEvent>,
    panel: usize,
    generation: u64,
    current: Arc<AtomicU64>,
    location: Location,
) {
    std::thread::spawn(move || {
        let _com = ComGuard::new();
        crate::icons::wait_shell_ready();
        let mut out = Batcher { hwnd, tx, panel, generation, current, pending: Vec::new(), last: Instant::now() };
        match &location {
            Location::Computer => list_computer(&mut out),
            Location::Path(path) => list_dir(&mut out, path),
            Location::Shell { parsing, .. } => list_shell(&mut out, parsing),
        }
    });
}

struct Batcher {
    hwnd: Handle,
    tx: Sender<LoadEvent>,
    panel: usize,
    generation: u64,
    current: Arc<AtomicU64>,
    pending: Vec<Entry>,
    last: Instant,
}

impl Batcher {
    fn cancelled(&self) -> bool {
        self.current.load(Ordering::Relaxed) != self.generation
    }

    fn push(&mut self, e: Entry) {
        self.pending.push(e);
        // Primer lote enseguida para que la lista aparezca rápido; después cada ~40 ms.
        if self.pending.len() >= 4096 || self.last.elapsed() >= Duration::from_millis(40) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            let batch = std::mem::take(&mut self.pending);
            self.send(LoadMsg::Entries(batch));
        }
        self.last = Instant::now();
    }

    fn send(&self, msg: LoadMsg) {
        let event = LoadEvent { panel: self.panel, generation: self.generation, msg };
        if self.tx.send(event).is_ok() {
            self.hwnd.post(WM_APP_LOAD, self.panel, 0);
        }
    }
}

fn list_dir(out: &mut Batcher, path: &Path) {
    let reader = match std::fs::read_dir(path) {
        Ok(r) => r,
        Err(e) => {
            out.send(LoadMsg::Listed(Err(e.to_string())));
            return;
        }
    };

    // Archivos cuyo ícono depende del contenido (ejecutables, accesos directos, carpetas
    // personalizadas): se resuelven después, para no demorar la lista.
    let mut needs_icon: Vec<(u32, OsString)> = Vec::new();
    let mut count = 0u32;

    for item in reader {
        if out.cancelled() {
            return;
        }
        let Ok(item) = item else { continue };
        // En Windows `DirEntry::metadata` sale de la misma búsqueda: no toca el disco otra vez.
        let Ok(meta) = item.metadata() else { continue };
        let name = item.file_name();
        let attrs = meta.file_attributes();
        let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        let display = name.to_string_lossy().into_owned();

        let ext = if is_dir { None } else { extension(&display) };
        let (type_name, icon) = type_info(ext.as_deref(), is_dir);
        let custom_icon = if is_dir {
            attrs & (FILE_ATTRIBUTE_READONLY.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0
        } else {
            ext.as_deref().is_some_and(has_own_icon)
        };
        if custom_icon {
            needs_icon.push((count, name.clone()));
        }

        out.push(Entry {
            sort_key: display.to_lowercase(),
            display,
            name,
            full: None,
            group: 0,
            pidl: None,
            is_dir,
            size: if is_dir { 0 } else { meta.file_size() },
            modified: meta.last_write_time(),
            created: meta.creation_time(),
            attrs,
            type_name,
            icon,
            overlay: 0,
            // Se consulta recién cuando la fila se ve.
            overlay_checked: false,
        });
        count += 1;
    }
    out.flush();
    out.send(LoadMsg::Listed(Ok(())));

    // Segunda pasada: íconos propios de cada archivo.
    let mut batch = Vec::new();
    let mut last = Instant::now();
    for (pos, name) in needs_icon {
        if out.cancelled() {
            return;
        }
        if let Some(info) = shell_info(&path.join(name), FILE_FLAGS_AND_ATTRIBUTES(0), SHGFI_SYSICONINDEX) {
            batch.push((pos, info.iIcon));
        }
        if last.elapsed() >= Duration::from_millis(60) {
            out.send(LoadMsg::Icons(std::mem::take(&mut batch)));
            last = Instant::now();
        }
    }
    if !batch.is_empty() {
        out.send(LoadMsg::Icons(batch));
    }
}

/// Prepara el hilo de una búsqueda para hablar con el Shell (tipos e íconos) y corre `f`.
pub fn with_search_thread(f: impl FnOnce()) {
    let _com = ComGuard::new();
    crate::icons::wait_shell_ready();
    f();
}

/// Datos de un elemento encontrado por la búsqueda (en Windows salen de la misma lectura).
pub fn search_metadata(item: &std::fs::DirEntry, _path: &Path) -> Option<std::fs::Metadata> {
    item.metadata().ok()
}

/// ¿Es la raíz de otro disco montado adentro? En Windows los puntos de montaje son enlaces (la
/// búsqueda ya no los recorre).
pub fn is_mount_point(_path: &Path) -> bool {
    false
}

/// ¿La búsqueda entra en esta carpeta? En Windows, en todas.
pub fn searchable_dir(_name: &std::ffi::OsStr) -> bool {
    true
}

/// Entrada de la lista para un elemento de disco fuera de la carpeta del panel (resultado de
/// búsqueda): con su ruta completa, tipo e ícono.
pub fn disk_entry(path: &Path, name: OsString, meta: &std::fs::Metadata) -> Entry {
    let attrs = meta.file_attributes();
    let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let display = name.to_string_lossy().into_owned();
    let ext = if is_dir { None } else { extension(&display) };
    let (type_name, mut icon) = type_info(ext.as_deref(), is_dir);
    let custom_icon = if is_dir {
        attrs & (FILE_ATTRIBUTE_READONLY.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0
    } else {
        ext.as_deref().is_some_and(has_own_icon)
    };
    if custom_icon {
        if let Some(info) = shell_info(path, FILE_FLAGS_AND_ATTRIBUTES(0), SHGFI_SYSICONINDEX) {
            icon = info.iIcon;
        }
    }
    Entry {
        sort_key: display.to_lowercase(),
        display,
        name,
        full: Some(path.as_os_str().to_os_string()),
        group: 0,
        pidl: None,
        is_dir,
        size: if is_dir { 0 } else { meta.file_size() },
        modified: meta.last_write_time(),
        created: meta.creation_time(),
        attrs,
        type_name,
        icon,
        overlay: 0,
        overlay_checked: false,
    }
}

/// Carpetas del usuario que se muestran en "Este equipo".
const USER_FOLDERS: [GUID; 6] =
    [FOLDERID_Desktop, FOLDERID_Downloads, FOLDERID_Documents, FOLDERID_Pictures, FOLDERID_Music, FOLDERID_Videos];
/// Lugares virtuales al final de "Este equipo": Red y Papelera.
const SHELL_PLACES: [&str; 2] = ["::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}", "::{645FF040-5081-101B-9F08-00AA002F954E}"];
const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";

/// Ruta de la carpeta del usuario `i` (0..6: Escritorio, Descargas, Documentos, Imágenes, Música,
/// Vídeos), si existe.
pub fn user_folder(i: usize) -> Option<PathBuf> {
    let id = USER_FOLDERS.get(i)?;
    let p = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    let path = unsafe {
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    };
    path.map(PathBuf::from).filter(|p| p.is_dir())
}

/// "Este equipo": carpetas del usuario, unidades, dispositivos sin letra (teléfonos) y,
/// al final, Red y Papelera. Cada grupo se ordena por separado.
fn list_computer(out: &mut Batcher) {
    for id in USER_FOLDERS {
        let Ok(p) = (unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) }) else { continue };
        let path = unsafe {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s
        };
        let Some(path) = path.map(PathBuf::from) else { continue };
        let flags = SHGFI_DISPLAYNAME | SHGFI_TYPENAME | SHGFI_SYSICONINDEX;
        let Some(info) = shell_info(&path, FILE_FLAGS_AND_ATTRIBUTES(0), flags) else { continue };
        let display = from_wide(&info.szDisplayName);
        out.push(Entry {
            name: path.clone().into_os_string(),
            full: Some(path.into_os_string()),
            group: 0,
            pidl: None,
            sort_key: display.to_lowercase(),
            display,
            is_dir: true,
            size: 0,
            modified: 0,
            created: 0,
            attrs: FILE_ATTRIBUTE_DIRECTORY.0,
            type_name: Arc::from(from_wide(&info.szTypeName)),
            icon: info.iIcon,
            overlay: 0,
            overlay_checked: true,
        });
    }
    out.flush();

    list_drives(out);

    // Lo que "Este equipo" del Shell tiene sin ruta de disco: teléfonos, cámaras, ubicaciones de red.
    if let Ok(item) = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(THIS_PC), None) } {
        // Si falla, "Este equipo" queda igual con carpetas y unidades.
        let _ = for_each_child(&item, |child| {
            if item_name(child, SIGDN_FILESYSPATH).is_none() {
                if let Some(e) = shell_entry(child, 2) {
                    out.push(e);
                }
            }
            !out.cancelled()
        });
    }
    for place in SHELL_PLACES {
        if let Ok(item) = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(place), None) } {
            if let Some(e) = shell_entry(&item, 3) {
                out.push(e);
            }
        }
    }
    out.flush();
    out.send(LoadMsg::Listed(Ok(())));
}

/// Contenido de una carpeta virtual del Shell.
fn list_shell(out: &mut Batcher, parsing: &str) {
    let item = match unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(parsing), None) } {
        Ok(i) => i,
        Err(e) => {
            out.send(LoadMsg::Listed(Err(e.message().to_string())));
            return;
        }
    };
    let ok = for_each_child(&item, |child| {
        if let Some(e) = shell_entry(child, 0) {
            out.push(e);
        }
        !out.cancelled()
    });
    out.flush();
    out.send(LoadMsg::Listed(ok.map_err(|e| e.message().to_string())));
}

/// Carpeta virtual de Linux (WSL) del Explorador.
pub const LINUX_PLACE: &str = "::{B2B4A4D1-2754-4140-A2EB-9A76D9D7CDC6}";
/// Red del Explorador.
pub const NETWORK_PLACE: &str = SHELL_PLACES[0];

/// ¿Existe esa carpeta virtual en este equipo? (Linux solo está con WSL instalado.)
pub fn shell_place_exists(parsing: &str) -> bool {
    unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(parsing), None) }.is_ok()
}

/// Subcarpetas con ruta de disco de una carpeta virtual (computadoras de la Red, distribuciones de
/// Linux), para el árbol lateral: (nombre, ruta). Ordenadas por nombre.
pub fn shell_subfolders(parsing: &str) -> Vec<(String, PathBuf)> {
    let mut out = shell_subfolders_unsorted(parsing);
    out.sort_by(|a, b| xdir_core::format::natural_cmp(&a.0.to_lowercase(), &b.0.to_lowercase()));
    out
}

/// Lo mismo, en el orden del Shell.
fn shell_subfolders_unsorted(parsing: &str) -> Vec<(String, PathBuf)> {
    let Ok(item) = (unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(parsing), None) }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let _ = for_each_child(&item, |child| {
        // Las computadoras de la Red (\\tsclient) no tienen ruta de archivos, pero su nombre de análisis
        // es la ruta UNC; de las distribuciones de Linux el nombre a mostrar trae el servidor entre paréntesis.
        let unc = item_name(child, SIGDN_DESKTOPABSOLUTEPARSING).filter(|p| p.starts_with("\\\\"));
        if let Some(path) = item_name(child, SIGDN_FILESYSPATH).or(unc) {
            // El último tramo de la ruta (en `\\servidor\recurso` Path no lo ve como nombre de archivo).
            let last = path.trim_end_matches('\\').rsplit('\\').next().filter(|n| !n.is_empty()).map(str::to_string);
            if let Some(name) = last.or_else(|| item_name(child, SIGDN_NORMALDISPLAY)) {
                out.push((name, PathBuf::from(path)));
            }
        }
        true
    });
    out
}

/// Recorre los hijos de un elemento del Shell mientras `f` devuelva true.
fn for_each_child(item: &IShellItem, mut f: impl FnMut(&IShellItem) -> bool) -> windows::core::Result<()> {
    unsafe {
        let children: IEnumShellItems = item.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut one = [None];
            let mut fetched = 0u32;
            if children.Next(&mut one, Some(&mut fetched)).is_err() || fetched == 0 {
                return Ok(());
            }
            if let Some(child) = &one[0] {
                if !f(child) {
                    return Ok(());
                }
            }
        }
    }
}

/// Elemento de lista para un hijo del Shell: nombre, tipo, ícono, tamaño y fecha
/// salen de las propiedades que expone el propio Shell.
fn shell_entry(item: &IShellItem, group: u8) -> Option<Entry> {
    // En la Papelera el nombre "normal" es la ruta original completa; ItemNameDisplay es
    // solo el nombre, como lo muestra el Explorador.
    let display = item_property_string(item, &PKEY_ItemNameDisplay).or_else(|| item_name(item, SIGDN_NORMALDISPLAY))?;
    // El nombre de análisis conserva el contexto virtual (en la Papelera, el menú ofrece
    // "Restaurar"); para elementos de disco es la misma ruta.
    let full = item_name(item, SIGDN_DESKTOPABSOLUTEPARSING).or_else(|| item_name(item, SIGDN_FILESYSPATH))?;
    let mask = SFGAO_FOLDER | SFGAO_STREAM | SFGAO_HIDDEN;
    let attrs_sh = unsafe { item.GetAttributes(mask) }.unwrap_or_default();
    // Un ZIP es carpeta y archivo a la vez: se trata como archivo (se abre con su programa).
    let is_dir = attrs_sh.0 & SFGAO_FOLDER.0 != 0 && attrs_sh.0 & SFGAO_STREAM.0 == 0;
    let mut attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY.0 } else { 0 };
    if attrs_sh.0 & SFGAO_HIDDEN.0 != 0 {
        attrs |= 0x2;
    }

    let (mut size, mut modified) = (0u64, 0u64);
    if let Ok(item2) = item.cast::<IShellItem2>() {
        unsafe {
            size = item2.GetUInt64(&PKEY_Size).or_else(|_| item2.GetUInt64(&PKEY_Capacity)).unwrap_or(0);
            if let Ok(ft) = item2.GetFileTime(&PKEY_DateModified) {
                modified = (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64;
            }
        }
    }

    let (type_name, icon, pidl_bytes) = unsafe {
        let mut info = SHFILEINFOW::default();
        match SHGetIDListFromObject(item) {
            Ok(pidl) => {
                let len = ILGetSize(Some(pidl)) as usize;
                let bytes: Arc<[u8]> = Arc::from(std::slice::from_raw_parts(pidl as *const u8, len));
                SHGetFileInfoW(
                    PCWSTR(pidl as *const u16),
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    Some(&mut info),
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_PIDL | SHGFI_SYSICONINDEX | SHGFI_TYPENAME,
                );
                ILFree(Some(pidl));
                (from_wide(&info.szTypeName), info.iIcon, Some(bytes))
            }
            Err(_) => (String::new(), 0, None),
        }
    };

    Some(Entry {
        name: OsString::from(&display),
        full: Some(OsString::from(full)),
        group,
        pidl: pidl_bytes,
        sort_key: display.to_lowercase(),
        display,
        is_dir,
        size,
        modified,
        created: 0,
        attrs,
        type_name: Arc::from(type_name),
        icon,
        overlay: 0,
        overlay_checked: true,
    })
}

fn item_property_string(item: &IShellItem, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let p = item.cast::<IShellItem2>().ok()?.GetString(key).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty())
    }
}

fn item_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty())
    }
}

fn list_drives(out: &mut Batcher) {
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i) as char);
        let path = PathBuf::from(&root);
        let flags = SHGFI_DISPLAYNAME | SHGFI_TYPENAME | SHGFI_SYSICONINDEX;
        let (display, type_name, icon) = match shell_info(&path, FILE_FLAGS_AND_ATTRIBUTES(0), flags) {
            Some(info) => (
                from_wide(&info.szDisplayName),
                Arc::from(from_wide(&info.szTypeName)),
                info.iIcon,
            ),
            None => (root.clone(), Arc::from("Unidad"), 0),
        };
        let mut total = 0u64;
        let wide = wide_path(&path);
        unsafe {
            let _ = GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), None, Some(&mut total), None);
        }
        out.push(Entry {
            name: OsString::from(&root),
            full: None,
            group: 1,
            pidl: None,
            sort_key: root.to_lowercase(),
            display,
            is_dir: true,
            size: total,
            modified: 0,
            created: 0,
            attrs: FILE_ATTRIBUTE_DIRECTORY.0,
            type_name,
            icon,
            overlay: 0,
            // Las unidades no tienen superposición.
            overlay_checked: true,
        });
        // Cada unidad sale apenas se conoce: una unidad de red lenta no frena a las demás.
        out.flush();
    }
}

fn extension(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| name[dot + 1..].to_lowercase())
}

/// Extensiones cuyo ícono sale del propio archivo y no del tipo.
fn has_own_icon(ext: &str) -> bool {
    matches!(
        ext,
        "exe" | "lnk" | "ico" | "url" | "cur" | "ani" | "msc" | "cpl" | "scr" | "appref-ms" | "website"
    )
}

static TYPE_CACHE: OnceLock<Mutex<HashMap<String, (Arc<str>, i32)>>> = OnceLock::new();

/// Ícono genérico de carpeta, si ya se conoce (lo calcula el primer lector de carpetas; la
/// interfaz no llama al Shell para esto).
pub fn folder_icon() -> Option<i32> {
    TYPE_CACHE.get()?.lock().ok()?.get("\\dir").map(|v| v.1)
}

/// Tipo de una carpeta, el mismo que lleva en la lista.
pub fn folder_type() -> Arc<str> {
    type_info(None, true).0
}

/// Nombre de tipo e ícono genérico por extensión, cacheados para todo el proceso.
fn type_info(ext: Option<&str>, is_dir: bool) -> (Arc<str>, i32) {
    let cache_cell = &TYPE_CACHE;
    let key = match (is_dir, ext) {
        (true, _) => "\\dir".to_string(),
        (false, Some(e)) => e.to_string(),
        (false, None) => String::new(),
    };
    let cache = cache_cell.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(&key) {
        return hit.clone();
    }

    let (probe, attrs) = if is_dir {
        ("carpeta".to_string(), FILE_ATTRIBUTE_DIRECTORY)
    } else {
        (format!("archivo.{}", ext.unwrap_or("")), FILE_ATTRIBUTE_NORMAL)
    };
    let flags = SHGFI_TYPENAME | SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES;
    let value = match shell_info(Path::new(&probe), attrs, flags) {
        Some(info) => {
            let mut name = from_wide(&info.szTypeName);
            if name.is_empty() {
                name = match ext {
                    Some(e) => format!("Archivo {}", e.to_uppercase()),
                    None => "Archivo".into(),
                };
            }
            (Arc::from(name), info.iIcon)
        }
        None => (Arc::from("Archivo"), 0),
    };
    cache.lock().unwrap().insert(key, value.clone());
    value
}

fn shell_info(path: &Path, attrs: FILE_FLAGS_AND_ATTRIBUTES, flags: SHGFI_FLAGS) -> Option<SHFILEINFOW> {
    let wide = wide_path(path);
    let mut info = SHFILEINFOW::default();
    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            attrs,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    (ok != 0).then_some(info)
}

/// Subcarpetas de un lugar, para el menú de las flechas de la barra de dirección: las unidades de
/// "Este equipo" o las carpetas de una carpeta (ocultas incluidas), por nombre. Vacío si no aplica.
pub fn subfolders(loc: &Location) -> Vec<(String, Location)> {
    const MAX: usize = 500;
    match loc {
        Location::Computer => {
            let mask = unsafe { GetLogicalDrives() };
            (0..26u8)
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| {
                    let letter = (b'A' + i) as char;
                    (drive_label(&format!("{letter}:")), Location::Path(PathBuf::from(format!("{letter}:\\"))))
                })
                .collect()
        }
        Location::Path(dir) => {
            let mut out: Vec<(String, Location)> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|name| (name.clone(), Location::Path(dir.join(name))))
                .collect();
            out.sort_by_key(|(n, _)| n.to_lowercase());
            out.truncate(MAX);
            out
        }
        Location::Shell { .. } => Vec::new(),
    }
}

static DRIVE_LABELS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Nombre de una unidad local ("C:" → "Disco local (C:)"), cacheado: se pide en cada dibujo de la
/// barra de dirección. Las que no son de disco fijo (red, CD, USB) quedan como "X:" porque
/// preguntarles el nombre puede demorar segundos.
pub fn drive_label(drive: &str) -> String {
    let cache = DRIVE_LABELS.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap().get(drive) {
        return hit.clone();
    }
    let root = format!("{drive}\\");
    let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
    let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
    // Disco fijo (3) y USB (2): el nombre que muestra el Shell. Unidad de red (4): "recurso (\servidor) (Z:)".
    let label = match kind {
        3 | 2 => shell_info(Path::new(&root), FILE_FLAGS_AND_ATTRIBUTES(0), SHGFI_DISPLAYNAME)
            .map(|info| from_wide(&info.szDisplayName))
            .filter(|n| !n.is_empty()),
        4 => network_drive_label(drive),
        _ => None,
    }
    .unwrap_or_else(|| drive.to_string());
    cache.lock().unwrap().insert(drive.to_string(), label.clone());
    label
}

/// Raíces de las unidades con letra ("C:\\", "D:\\"...).
pub fn drives() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8).filter(|i| mask & (1 << i) != 0).map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char))).collect()
}

/// Nombre corto de una unidad para `drive_label`: "C:\\" → "C:".
pub fn drive_name(root: &Path) -> String {
    root.to_string_lossy().trim_end_matches('\\').to_string()
}

/// ¿Está oculto? (atributo de Windows).
pub fn is_hidden(meta: &std::fs::Metadata, _name: &std::ffi::OsStr) -> bool {
    meta.file_attributes() & 0x2 != 0
}

/// Olvida los nombres de unidad guardados (cambió el conjunto de unidades: otro pendrive
/// puede tomar la misma letra).
pub fn forget_drive_labels() {
    if let Some(cache) = DRIVE_LABELS.get() {
        cache.lock().unwrap().clear();
    }
}

/// ¿Es una ruta de una unidad con letra que ya no existe (se quitó el pendrive)?
pub fn drive_missing(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() && !Path::new(&format!("{}:\\", b[0] as char)).exists()
}

/// Nombre de una unidad de red como lo muestra el Explorador: "recurso (\\servidor) (Z:)".
fn network_drive_label(drive: &str) -> Option<String> {
    use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
    let local: Vec<u16> = drive.encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let err = unsafe { WNetGetConnectionW(PCWSTR(local.as_ptr()), Some(PWSTR(buf.as_mut_ptr())), &mut len) };
    if err.0 != 0 {
        return None;
    }
    let remote = from_wide(&buf);
    let mut parts = remote.trim_start_matches('\\').split('\\').filter(|p| !p.is_empty());
    let (server, share) = (parts.next()?, parts.next()?);
    Some(format!("{share} (\\\\{server}) ({drive})"))
}


/// Carpeta virtual del Acceso rápido del Explorador (con `shell:::`; sin el prefijo el Shell no la resuelve).
const QUICK_ACCESS_PLACE: &str = "shell:::{3936E9E4-D92C-4EEE-A85A-BC16D5EA0819}";

/// Carpetas del Acceso rápido de Windows, enumeradas del propio Shell y en el mismo orden que
/// el Explorador (ancladas primero, después las frecuentes). Solo las que son carpetas de disco.
pub fn quick_access() -> Vec<QuickItem> {
    let _com = ComGuard::new();
    let Ok(item) = (unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS_PLACE), None) }) else {
        return Vec::new();
    };
    let mut key = PROPERTYKEY::default();
    let have_key = unsafe { windows::Win32::UI::Shell::PropertiesSystem::PSGetPropertyKeyFromName(windows::core::w!("System.Home.IsPinned"), &mut key) }.is_ok();
    let mut out = Vec::new();
    let _ = for_each_child(&item, |child| {
        if let Some(path) = item_name(child, SIGDN_FILESYSPATH).map(PathBuf::from).filter(|p| p.is_dir()) {
            let name = item_name(child, SIGDN_NORMALDISPLAY).unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            let pinned = have_key && child.cast::<IShellItem2>().ok().is_some_and(|i2| unsafe { i2.GetBool(&key) }.is_ok_and(|b| b.as_bool()));
            out.push(QuickItem { name, path, pinned });
        }
        true
    });
    out
}

/// ¿Está anclada ahora mismo al Acceso rápido de Windows? (lectura nueva, no la del árbol)
pub fn is_quick_pinned(path: &Path) -> bool {
    quick_access().iter().any(|i| i.pinned && same_path(&i.path, path))
}

/// El elemento del Acceso rápido que corresponde a una carpeta (el que ofrece "Desanclar").
pub fn quick_access_item(path: &Path) -> Option<IShellItem> {
    let _com = ComGuard::new();
    let item = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS_PLACE), None) }.ok()?;
    let mut found = None;
    let _ = for_each_child(&item, |child| {
        if item_name(child, SIGDN_FILESYSPATH).is_some_and(|p| same_path(Path::new(&p), path)) {
            found = Some(child.clone());
            return false;
        }
        true
    });
    found
}

/// Carpeta cuyos cambios indican que el Acceso rápido se modificó (anclar, desanclar, uso reciente).
pub fn quick_access_watch_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Recent\AutomaticDestinations")).filter(|p| p.is_dir())
}

/// Forma larga de una ruta ("C:\Users\LEANDR~1" → "C:\Users\Leandro-Dev"). Si no existe o
/// falla, la devuelve igual.
pub fn long_path(path: &Path) -> PathBuf {
    // Solo las rutas con "~" pueden tener nombres cortos; las demás no tocan el disco
    // (una ruta de red caída podría demorar segundos).
    if !path.as_os_str().to_string_lossy().contains('~') {
        return path.to_path_buf();
    }
    let wide = wide_path(path);
    let mut buf = vec![0u16; 32 * 1024];
    let n = unsafe { GetLongPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buf)) } as usize;
    if n == 0 || n >= buf.len() {
        return path.to_path_buf();
    }
    PathBuf::from(OsString::from_wide(&buf[..n]))
}

/// Ruta escrita a mano con las mayúsculas reales del disco ("c:\program files" → "C:\Program
/// Files"): cada tramo se busca con FindFirstFileW. La unidad va en mayúscula; las rutas de red
/// (`\\servidor\recurso`) conservan esa parte como se escribió. Si algo falla, queda como estaba.
pub fn true_case(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => out.push(format!("{}:", (d as char).to_ascii_uppercase())),
                _ => out.push(p.as_os_str()),
            },
            Component::RootDir => out.push("\\"),
            Component::Normal(name) => {
                let probe = out.join(name);
                let wide = wide_path(&probe);
                let mut data = WIN32_FIND_DATAW::default();
                match unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data) } {
                    Ok(h) => {
                        let _ = unsafe { FindClose(h) };
                        let len = data.cFileName.iter().position(|&ch| ch == 0).unwrap_or(data.cFileName.len());
                        out.push(OsString::from_wide(&data.cFileName[..len]));
                    }
                    Err(_) => out.push(name),
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}


/// Misma ruta en Windows: sin distinguir mayúsculas ni separador final.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}


pub fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len]).to_string_lossy().into_owned()
}

/// Inicializa COM en el hilo actual mientras viva (lo pide `SHGetFileInfoW`).
struct ComGuard(bool);

impl ComGuard {
    fn new() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// Vigila una carpeta y avisa a la ventana con un mensaje (`FileSystem::watch`). Se detiene al soltarlo.
pub struct Watcher {
    stop: usize,
    io: usize,
    thread: Option<JoinHandle<()>>,
}

impl xdir_core::sys::Watch for Watcher {}

impl Watcher {
    /// Vigila una carpeta y avisa a `hwnd` con el mensaje `msg` (y estos `wparam`/`lparam`).
    pub fn with_message(hwnd: Handle, msg: u32, panel: usize, generation: isize, path: &Path) -> Option<Self> {
        let wide = wide_path(path);
        unsafe {
            // FILE_SHARE_DELETE: otra ventana puede borrar o renombrar la carpeta mientras se vigila.
            let dir = CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
            .ok()?;
            let stop = CreateEventW(None, true, false, None).ok()?;
            let io = CreateEventW(None, true, false, None).ok()?;
            let (dir, stop_raw, io, hwnd) = (dir.0 as usize, stop.0 as usize, io.0 as usize, hwnd.0 as usize);
            let thread = std::thread::spawn(move || {
                let dir = HANDLE(dir as *mut _);
                let stop = HANDLE(stop_raw as *mut _);
                let io = HANDLE(io as *mut _);
                let filter = FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES;
                let mut buf = [0u32; 1024];
                let post = || {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        msg,
                        WPARAM(panel),
                        LPARAM(generation),
                    );
                };
                loop {
                    let mut ov = OVERLAPPED { hEvent: io, ..Default::default() };
                    let _ = ResetEvent(io);
                    let queued = ReadDirectoryChangesW(
                        dir,
                        buf.as_mut_ptr().cast(),
                        std::mem::size_of_val(&buf) as u32,
                        false,
                        filter,
                        None,
                        Some(&mut ov),
                        None,
                    );
                    if queued.is_err() {
                        post();
                        break;
                    }
                    if WaitForMultipleObjects(&[io, stop], false, INFINITE) != WAIT_OBJECT_0 {
                        let _ = CancelIoEx(dir, Some(&ov));
                        let mut n = 0;
                        let _ = GetOverlappedResult(dir, &ov, &mut n, true);
                        break;
                    }
                    let mut n = 0;
                    if GetOverlappedResult(dir, &ov, &mut n, false).is_err() {
                        // La carpeta se borró, renombró o desapareció: soltarla y avisar.
                        post();
                        break;
                    }
                    // Junta ráfagas de cambios (copias de muchos archivos) en un solo aviso.
                    if WaitForSingleObject(stop, 250) == WAIT_OBJECT_0 {
                        break;
                    }
                    post();
                }
                let _ = CloseHandle(dir);
            });
            Some(Self { stop: stop_raw, io, thread: Some(thread) })
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let stop = HANDLE(self.stop as *mut _);
        unsafe {
            let _ = SetEvent(stop);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(stop);
            let _ = CloseHandle(HANDLE(self.io as *mut _));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::true_case;
    use xdir_core::fs::normalize_lexical;
    use std::path::Path;

    #[test]
    fn normalize_resolves_dots_without_touching_the_disk() {
        let n = |s: &str| normalize_lexical(Path::new(s));
        assert_eq!(n(r"C:\a\b\.."), Path::new(r"C:\a"));
        assert_eq!(n(r"C:\a\.\b"), Path::new(r"C:\a\b"));
        assert_eq!(n(r"C:\a\b\..\..\c"), Path::new(r"C:\c"));
        // Más `..` que carpetas: se queda en la raíz.
        assert_eq!(n(r"C:\a\..\..\.."), Path::new(r"C:\"));
        assert_eq!(n(r"\\srv\share\x\.."), Path::new(r"\\srv\share\"));
        // Sin `..` no cambia nada.
        assert_eq!(n(r"C:\Users\yo"), Path::new(r"C:\Users\yo"));
        // Relativa: un `..` sin nada que quitar se conserva.
        assert_eq!(n(r"..\x"), Path::new(r"..\x"));
        assert_eq!(n(r"a\..\b"), Path::new("b"));
    }

    #[test]
    fn true_case_fixes_typed_paths() {
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let typed = windir.to_lowercase() + r"\SYSTEM32";
        let fixed = true_case(Path::new(&typed));
        assert_eq!(fixed.to_string_lossy().to_lowercase(), typed.to_lowercase());
        assert!(fixed.to_string_lossy().ends_with(r"\System32"), "{}", fixed.display());
        assert!(fixed.to_string_lossy().starts_with(|c: char| c.is_ascii_uppercase()));
        // Lo que no existe queda como se escribió.
        assert_eq!(true_case(Path::new(r"C:\no\EXISTE")), Path::new(r"C:\no\EXISTE"));
    }
}

// ---------- xtree (escáner de uso de disco) ----------

mod mft;
mod xtree;
pub use self::xtree::*;

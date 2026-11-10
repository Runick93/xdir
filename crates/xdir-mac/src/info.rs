//! Panel de información en macOS (`sys::Info`): lee con las claves de `NSURL`, Spotlight
//! (`MDItem`), los atributos extendidos del Finder y `lstat`, sin abrir el contenido (lo que está
//! solo en la nube, `SF_DATALESS`, no se baja); escribe etiquetas y «Ocultar la extensión» con
//! `NSURL`, Bloqueado y Oculto con `chflags`, el comentario por el Finder (`osascript`, la única
//! forma que el Finder respeta), «Abrir con» con `NSWorkspace` (macOS 12+) y los permisos con
//! `chmod`. Todo corre en hilos de trabajo.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_app_kit::NSWorkspace;
use objc2_core_foundation::{CFDate, CFString, CFURL};
use objc2_core_services::{MDItem, kMDItemFinderComment, kMDItemLastUsedDate};
use objc2_foundation::{
    NSArray, NSData, NSDate, NSError, NSNumber, NSPropertyListSerialization, NSString, NSURL, NSURLContentModificationDateKey,
    NSURLCreationDateKey, NSURLFileSizeKey, NSURLHasHiddenExtensionKey, NSURLIsVolumeKey, NSURLLocalizedTypeDescriptionKey, NSURLResourceKey,
    NSURLTagNamesKey, NSURLTotalFileAllocatedSizeKey, NSURLVolumeAvailableCapacityKey, NSURLVolumeIsEjectableKey, NSURLVolumeIsInternalKey,
    NSURLVolumeIsLocalKey, NSURLVolumeLocalizedFormatDescriptionKey, NSURLVolumeTotalCapacityKey,
};
use xdir_core::info::{AppRef, Flag, InfoChange, InfoReply, InfoRequest, ItemInfo, OpenWith, Perms, Tag, VolumeData};
use xdir_core::shell::Item;
use xdir_core::sys::{Handle, Info};

use crate::icons::{IconKey, index_of};

/// `chflags` del usuario.
const UF_IMMUTABLE: u32 = 0x0000_0002;
const UF_HIDDEN: u32 = 0x0000_8000;
/// Segundos entre 1970 y 2001 (el origen de `CFAbsoluteTime`).
const CF_EPOCH: f64 = 978_307_200.0;

/// El servicio que instala `sys::install`.
pub struct MacInfo;

impl Info for MacInfo {
    fn request(&self, req: InfoRequest) {
        let spawned = std::thread::Builder::new().name("xdir-informacion".into()).spawn(move || xdir_core::info::answer(req, &read));
        debug_assert!(spawned.is_ok(), "hilo de información");
    }

    fn apply(&self, owner: Handle, tx: Sender<InfoReply>, generation: u64, change: InfoChange) {
        let spawned = std::thread::Builder::new().name("xdir-informacion-cambio".into()).spawn(move || {
            let section = change.section();
            let error = autoreleasepool(|_| apply_with(&change, &SystemDefaults)).err();
            xdir_core::info::send_applied(owner, &tx, generation, section, error);
        });
        debug_assert!(spawned.is_ok(), "hilo de cambio de información");
    }

    fn more_in_system(&self, owner: Handle, items: &[Item]) -> bool {
        crate::shell::macos::properties(owner, items);
        true
    }
}

// ---------- lectura ----------

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn value(url: &NSURL, key: &NSURLResourceKey) -> Option<Retained<AnyObject>> {
    let mut v: Option<Retained<AnyObject>> = None;
    unsafe { url.getResourceValue_forKey_error(&mut v, key) }.ok()?;
    v
}

fn number(url: &NSURL, key: &NSURLResourceKey) -> Option<Retained<NSNumber>> {
    value(url, key)?.downcast::<NSNumber>().ok()
}

fn string(url: &NSURL, key: &NSURLResourceKey) -> Option<String> {
    value(url, key)?.downcast::<NSString>().ok().map(|s| s.to_string())
}

fn date(url: &NSURL, key: &NSURLResourceKey) -> u64 {
    value(url, key).and_then(|v| v.downcast::<NSDate>().ok()).map_or(0, |d| filetime_of_secs(d.timeIntervalSince1970()))
}

/// Segundos de Unix con decimales → FILETIME.
fn filetime_of_secs(secs: f64) -> u64 {
    if !secs.is_finite() || secs <= -11_644_473_600.0 {
        return 0;
    }
    ((secs + 11_644_473_600.0) * 10_000_000.0) as u64
}

/// Todo lo del elemento, sin abrir su contenido.
pub fn read(path: &Path) -> ItemInfo {
    autoreleasepool(|_| read_inner(path))
}

fn read_inner(path: &Path) -> ItemInfo {
    let cloud = crate::metadata::cloud_only(path);
    let mut info = xdir_core::info::read_minimal(path, &|_| cloud);
    info.last_opened = 0;
    info.last_opened_is_access = false;
    let url = file_url(path);
    info.kind_name = string(&url, unsafe { NSURLLocalizedTypeDescriptionKey }).unwrap_or_default();
    if !info.is_dir {
        if let Some(n) = number(&url, unsafe { NSURLFileSizeKey }) {
            info.size = n.unsignedLongLongValue();
        }
        info.on_disk = number(&url, unsafe { NSURLTotalFileAllocatedSizeKey }).map_or(0, |n| n.unsignedLongLongValue());
    }
    let created = date(&url, unsafe { NSURLCreationDateKey });
    let modified = date(&url, unsafe { NSURLContentModificationDateKey });
    if created != 0 {
        info.created = created;
    }
    if modified != 0 {
        info.modified = modified;
    }
    // Un disco entero: capacidad, libre y formato.
    let is_volume = number(&url, unsafe { NSURLIsVolumeKey }).is_some_and(|n| n.boolValue());
    if is_volume {
        let total = number(&url, unsafe { NSURLVolumeTotalCapacityKey }).map_or(0, |n| n.unsignedLongLongValue());
        let free = number(&url, unsafe { NSURLVolumeAvailableCapacityKey }).map_or(0, |n| n.unsignedLongLongValue());
        let format = string(&url, unsafe { NSURLVolumeLocalizedFormatDescriptionKey }).unwrap_or_default();
        let flag = |k: &NSURLResourceKey| number(&url, k).is_some_and(|n| n.boolValue());
        info.kind_name = if !flag(unsafe { NSURLVolumeIsLocalKey }) {
            "Disco de red".into()
        } else if flag(unsafe { NSURLVolumeIsInternalKey }) {
            "Disco interno".into()
        } else if flag(unsafe { NSURLVolumeIsEjectableKey }) {
            "Disco externo extraíble".into()
        } else {
            "Disco externo".into()
        };
        info.volume = Some(VolumeData { total, free, format });
    }
    // Etiquetas con los colores del Finder (el atributo `_kMDItemUserTags`).
    let colors = finder_tag_colors(path);
    let names = tag_names(&url);
    info.tags = Some(names.into_iter().map(|n| Tag { color: colors.iter().find(|(x, _)| *x == n).and_then(|(_, c)| *c), name: n }).collect());
    info.comment = Some(finder_comment(path).unwrap_or_default());
    if !is_volume {
        info.hidden_ext = number(&url, unsafe { NSURLHasHiddenExtensionKey }).map(|n| n.boolValue());
    }
    // Bloqueado y Oculto: banderas del usuario (las cambia el dueño).
    let (flags, uid, gid, mode) = lstat(path).unwrap_or((0, u32::MAX, u32::MAX, 0));
    let euid = unsafe { libc::geteuid() };
    let mine = euid == uid || euid == 0;
    if !is_volume {
        info.flags = vec![(Flag::Locked, flags & UF_IMMUTABLE != 0, mine), (Flag::Hidden, flags & UF_HIDDEN != 0, mine)];
    }
    info.last_opened = last_used(path);
    if uid != u32::MAX {
        let c = cpath(path);
        let ok = |m| c.as_ref().is_some_and(|c| unsafe { libc::access(c.as_ptr(), m) } == 0);
        info.perms = Some(Perms {
            owner: user_name(uid),
            group: group_name(gid),
            mode: mode & 0o7777,
            editable: mine && !is_volume,
            can_read: ok(libc::R_OK),
            can_write: ok(libc::W_OK),
        });
    }
    if !info.is_dir {
        info.open_with = open_with(&url);
    }
    info
}

fn cpath(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

/// (banderas, dueño, grupo, modo) sin seguir enlaces.
fn lstat(path: &Path) -> Option<(u32, u32, u32, u32)> {
    use std::os::macos::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path).ok()?;
    Some((m.st_flags(), m.st_uid(), m.st_gid(), m.st_mode()))
}

fn user_name(uid: u32) -> String {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() {
        return uid.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }.to_string_lossy().into_owned()
}

fn group_name(gid: u32) -> String {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::group = std::ptr::null_mut();
    let r = unsafe { libc::getgrgid_r(gid, &mut grp, buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() {
        return gid.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(grp.gr_name) }.to_string_lossy().into_owned()
}

fn tag_names(url: &NSURL) -> Vec<String> {
    let Some(array) = value(url, unsafe { NSURLTagNamesKey }).and_then(|v| v.downcast::<NSArray>().ok()) else { return Vec::new() };
    array.iter().filter_map(|t| t.downcast::<NSString>().ok()).map(|t| t.to_string()).filter(|t| !t.is_empty()).collect()
}

/// Un atributo extendido (sin seguir enlaces); no toca el contenido.
fn xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let (c, n) = (cpath(path)?, CString::new(name).ok()?);
    let opts = libc::XATTR_NOFOLLOW;
    let len = unsafe { libc::getxattr(c.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0, 0, opts) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let got = unsafe { libc::getxattr(c.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len(), 0, opts) };
    (got > 0).then(|| {
        buf.truncate(got as usize);
        buf
    })
}

fn plist(bytes: &[u8]) -> Option<Retained<AnyObject>> {
    let data = NSData::with_bytes(bytes);
    unsafe { NSPropertyListSerialization::propertyListWithData_options_format_error(&data, objc2_foundation::NSPropertyListMutabilityOptions::Immutable, std::ptr::null_mut()) }.ok()
}

/// Color 0xRRGGBB de cada número de etiqueta del Finder (1 gris … 7 naranja).
pub fn finder_color(n: u32) -> Option<u32> {
    match n {
        1 => Some(0x98989D),
        2 => Some(0x32D74B),
        3 => Some(0xBF5AF2),
        4 => Some(0x0A84FF),
        5 => Some(0xFFD60A),
        6 => Some(0xFF453A),
        7 => Some(0xFF9F0A),
        _ => None,
    }
}

/// «Nombre\nN» del atributo de etiquetas → (nombre, color).
pub fn parse_tag_entry(s: &str) -> (String, Option<u32>) {
    match s.rsplit_once('\n') {
        Some((name, n)) => (name.to_string(), n.trim().parse().ok().and_then(finder_color)),
        None => (s.to_string(), None),
    }
}

fn finder_tag_colors(path: &Path) -> Vec<(String, Option<u32>)> {
    let Some(obj) = xattr(path, "com.apple.metadata:_kMDItemUserTags").and_then(|b| plist(&b)) else { return Vec::new() };
    let Ok(array) = obj.downcast::<NSArray>() else { return Vec::new() };
    array.iter().filter_map(|t| t.downcast::<NSString>().ok()).map(|t| parse_tag_entry(&t.to_string())).collect()
}

/// El comentario del Finder: el atributo que escribe el Finder (al día) o, si no está, Spotlight.
fn finder_comment(path: &Path) -> Option<String> {
    if let Some(obj) = xattr(path, "com.apple.metadata:kMDItemFinderComment").and_then(|b| plist(&b)) {
        return obj.downcast::<NSString>().ok().map(|s| s.to_string());
    }
    let url = CFURL::from_file_path(path)?;
    let item = unsafe { MDItem::with_url(None, Some(&url)) }?;
    let v = unsafe { item.attribute(kMDItemFinderComment) }?;
    v.downcast_ref::<CFString>().map(|s| s.to_string())
}

/// Última apertura según Spotlight (`kMDItemLastUsedDate`), como FILETIME (0 si no hay).
fn last_used(path: &Path) -> u64 {
    let Some(url) = CFURL::from_file_path(path) else { return 0 };
    let Some(item) = (unsafe { MDItem::with_url(None, Some(&url)) }) else { return 0 };
    let Some(v) = (unsafe { item.attribute(kMDItemLastUsedDate) }) else { return 0 };
    v.downcast_ref::<CFDate>().map_or(0, |d| filetime_of_secs(d.absolute_time() + CF_EPOCH))
}

fn app_ref(app: &NSURL) -> Option<AppRef> {
    let p = PathBuf::from(app.path()?.to_string());
    let name = p.file_stem()?.to_string_lossy().into_owned();
    Some(AppRef { id: p.to_string_lossy().into_owned(), name, icon: index_of(IconKey::Path(p)) })
}

/// La app que lo abre y las que pueden (la de ahora primero), por el tipo: no se lee el archivo.
fn open_with(url: &NSURL) -> Option<OpenWith> {
    let ws = NSWorkspace::sharedWorkspace();
    let current = ws.URLForApplicationToOpenURL(url).and_then(|u| app_ref(&u));
    let mut apps: Vec<AppRef> = current.iter().cloned().collect();
    for u in ws.URLsForApplicationsToOpenURL(url).iter() {
        if let Some(a) = app_ref(&u).filter(|a| !apps.iter().any(|x| x.id == a.id)) {
            apps.push(a);
        }
        if apps.len() >= 30 {
            break;
        }
    }
    (!apps.is_empty()).then_some(OpenWith { current, apps, per_file: true })
}

// ---------- escritura ----------

/// Las apps por defecto (lo de `NSWorkspace`): aparte para probar «Cambiar todos…» sin tocar la
/// configuración de la Mac.
pub trait Defaults {
    /// Esta app abre este archivo (queda para el archivo, como en el Finder).
    fn for_file(&self, app: &Path, file: &Path) -> Result<(), String>;
    /// Esta app abre todos los documentos del tipo de `file`.
    fn for_type(&self, app: &Path, file: &Path) -> Result<(), String>;
}

/// `NSWorkspace setDefaultApplicationAtURL:…` (macOS 12+), esperando su respuesta.
pub struct SystemDefaults;

fn wait_for(f: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>)) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let block = block2::RcBlock::new(move |err: *mut NSError| {
        let msg = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string());
        let _ = tx.send(msg);
    });
    f(&block);
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(None) => Ok(()),
        Ok(Some(e)) => Err(e),
        Err(_) => Err("El sistema no respondió".into()),
    }
}

impl Defaults for SystemDefaults {
    fn for_file(&self, app: &Path, file: &Path) -> Result<(), String> {
        let ws = NSWorkspace::sharedWorkspace();
        let (a, f) = (file_url(app), file_url(file));
        wait_for(|b| ws.setDefaultApplicationAtURL_toOpenFileAtURL_completionHandler(&a, &f, Some(b)))
    }

    fn for_type(&self, app: &Path, file: &Path) -> Result<(), String> {
        let ws = NSWorkspace::sharedWorkspace();
        let (a, f) = (file_url(app), file_url(file));
        wait_for(|b| ws.setDefaultApplicationAtURL_toOpenContentTypeOfFileAtURL_completionHandler(&a, &f, Some(b)))
    }
}

/// Texto entre comillas de AppleScript: barras y comillas escapadas.
pub fn applescript_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// El error de E/S como lo ve el usuario.
fn io_text(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "No tenés permiso".into(),
        std::io::ErrorKind::NotFound => "Ya no existe".into(),
        _ => match e.raw_os_error() {
            Some(libc::EPERM) => "No tenés permiso".into(),
            Some(libc::EROFS) => "El disco es solo de lectura".into(),
            _ => e.to_string(),
        },
    }
}

/// Varios errores juntos: el primero y cuántos más.
fn joined(errors: Vec<String>) -> Result<(), String> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().unwrap_or_default()),
        n => Err(format!("{} (y {} errores más)", errors[0], n - 1)),
    }
}

fn chflags(path: &Path, bit: u32, on: bool) -> Result<(), String> {
    let (flags, ..) = lstat(path).ok_or_else(|| "Ya no existe".to_string())?;
    let next = if on { flags | bit } else { flags & !bit };
    if next == flags {
        return Ok(());
    }
    let c = cpath(path).ok_or("Ruta inválida")?;
    if unsafe { libc::lchflags(c.as_ptr(), next) } != 0 {
        return Err(io_text(&std::io::Error::last_os_error()));
    }
    Ok(())
}

fn set_tags(path: &Path, add: &[String], remove: &[String]) -> Result<(), String> {
    let url = file_url(path);
    let mut names = tag_names(&url);
    names.retain(|n| !remove.contains(n));
    for a in add {
        if !names.contains(a) {
            names.push(a.clone());
        }
    }
    let strings: Vec<Retained<NSString>> = names.iter().map(|n| NSString::from_str(n)).collect();
    let array = NSArray::from_retained_slice(&strings);
    unsafe { url.setResourceValue_forKey_error(Some(&array), NSURLTagNamesKey) }.map_err(|e| e.localizedDescription().to_string())
}

fn set_comment(path: &Path, text: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"Finder\" to set comment of (POSIX file {} as alias) to {}",
        applescript_string(&path.to_string_lossy()),
        applescript_string(text)
    );
    let out = std::process::Command::new("osascript").arg("-e").arg(script).output().map_err(|e| io_text(&e))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.contains("-1743") { "xdir no tiene permiso para usar el Finder (Ajustes del Sistema > Privacidad > Automatización)".into() } else { format!("El Finder no guardó el comentario: {err}") })
    }
}

/// `chmod` del elemento con `mode`; con `recursive`, lo de adentro toma la lectura y la escritura
/// de cada clase (la ejecución de cada uno queda como estaba). Junta los errores.
pub fn chmod(path: &Path, mode: u32, recursive: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut errors = Vec::new();
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
        errors.push(io_text(&e));
    }
    if recursive && path.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(e) => {
                    errors.push(format!("{}: {}", dir.display(), io_text(&e)));
                    continue;
                }
            };
            for entry in rd.flatten() {
                let p = entry.path();
                let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
                if m.file_type().is_symlink() {
                    continue;
                }
                let next = (m.permissions().mode() & 0o7777 & !0o666) | (mode & 0o666);
                if let Err(e) = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(next)) {
                    errors.push(format!("{}: {}", p.display(), io_text(&e)));
                }
                if m.is_dir() {
                    stack.push(p);
                }
            }
        }
    }
    joined(errors)
}

/// Aplica un cambio (en el hilo que llama).
pub fn apply_with(change: &InfoChange, defaults: &dyn Defaults) -> Result<(), String> {
    let each = |paths: &[PathBuf], f: &dyn Fn(&Path) -> Result<(), String>| joined(paths.iter().filter_map(|p| f(p).err()).collect());
    match change {
        InfoChange::Tags { paths, add, remove } => each(paths, &|p| set_tags(p, add, remove)),
        InfoChange::Comment { path, text } => set_comment(path, text),
        InfoChange::Flag { paths, flag, on } => {
            let bit = match flag {
                Flag::Locked => UF_IMMUTABLE,
                Flag::Hidden => UF_HIDDEN,
                Flag::ReadOnly => return Err("Esto no existe en macOS".into()),
            };
            each(paths, &|p| chflags(p, bit, *on))
        }
        InfoChange::HideExt { path, on } => {
            let url = file_url(path);
            let value = NSNumber::numberWithBool(*on);
            unsafe { url.setResourceValue_forKey_error(Some(&value), NSURLHasHiddenExtensionKey) }.map_err(|e| e.localizedDescription().to_string())
        }
        InfoChange::OpenWith { paths, app } => each(paths, &|p| defaults.for_file(Path::new(app), p)),
        InfoChange::ChangeAll { path, app } => defaults.for_type(Path::new(app), path),
        InfoChange::Perms { paths, mode, recursive } => each(paths, &|p| chmod(p, *mode, *recursive)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("xdir-mac-info").join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn applescript_string_escapes() {
        assert_eq!(applescript_string("hola"), "\"hola\"");
        assert_eq!(applescript_string(r#"dijo "sí" \ chau"#), r#""dijo \"sí\" \\ chau""#);
        assert_eq!(parse_tag_entry("Rojo\n6"), ("Rojo".into(), Some(0xFF453A)));
        assert_eq!(parse_tag_entry("Viaje"), ("Viaje".into(), None));
        assert_eq!(parse_tag_entry("Viaje\n0"), ("Viaje".into(), None));
    }

    #[test]
    fn mac_info_reads_general() {
        let d = temp("general");
        let f = d.join("informe.txt");
        std::fs::write(&f, vec![b'x'; 5000]).unwrap();
        let i = read(&f);
        assert_eq!(i.size, 5000);
        assert!(i.on_disk >= 5000, "{}", i.on_disk);
        assert!(!i.kind_name.is_empty());
        assert!(i.created > 0 && i.modified > 0);
        assert!(!i.is_dir && !i.cloud_only);
        assert_eq!(i.flags.iter().map(|f| f.0).collect::<Vec<_>>(), [Flag::Locked, Flag::Hidden]);
        let p = i.perms.expect("permisos");
        assert!(p.editable && p.can_read && p.can_write);
        assert!(!p.owner.is_empty());
        assert_eq!(i.hidden_ext, Some(false));
        assert!(i.open_with.is_some(), "algo abre un .txt");
        assert_eq!(i.comment.as_deref(), Some(""));
        let dir = read(&d);
        assert!(dir.is_dir && dir.open_with.is_none());
    }

    #[test]
    fn mac_tags_roundtrip() {
        let d = temp("etiquetas");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        apply_with(&InfoChange::Tags { paths: vec![f.clone()], add: vec!["Viaje".into(), "xdir-prueba".into()], remove: vec![] }, &SystemDefaults).unwrap();
        let names: Vec<String> = read(&f).tags.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Viaje", "xdir-prueba"]);
        apply_with(&InfoChange::Tags { paths: vec![f.clone()], add: vec![], remove: vec!["Viaje".into()] }, &SystemDefaults).unwrap();
        let names: Vec<String> = read(&f).tags.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["xdir-prueba"]);
    }

    #[test]
    fn mac_hidden_flag_roundtrip() {
        let d = temp("oculto");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let flag = |i: &ItemInfo, f: Flag| i.flags.iter().find(|x| x.0 == f).unwrap().1;
        apply_with(&InfoChange::Flag { paths: vec![f.clone()], flag: Flag::Hidden, on: true }, &SystemDefaults).unwrap();
        assert!(flag(&read(&f), Flag::Hidden));
        apply_with(&InfoChange::Flag { paths: vec![f.clone()], flag: Flag::Locked, on: true }, &SystemDefaults).unwrap();
        assert!(flag(&read(&f), Flag::Locked));
        assert!(std::fs::remove_file(&f).is_err(), "bloqueado no se borra");
        apply_with(&InfoChange::Flag { paths: vec![f.clone()], flag: Flag::Locked, on: false }, &SystemDefaults).unwrap();
        apply_with(&InfoChange::Flag { paths: vec![f.clone()], flag: Flag::Hidden, on: false }, &SystemDefaults).unwrap();
        let i = read(&f);
        assert!(!flag(&i, Flag::Hidden) && !flag(&i, Flag::Locked));
        apply_with(&InfoChange::HideExt { path: f.clone(), on: true }, &SystemDefaults).unwrap();
        assert_eq!(read(&f).hidden_ext, Some(true));
    }

    #[test]
    fn mac_perms_chmod_recursive() {
        use std::os::unix::fs::PermissionsExt;
        let d = temp("permisos");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/a.txt"), b"x").unwrap();
        std::fs::set_permissions(d.join("sub/a.txt"), std::fs::Permissions::from_mode(0o600)).unwrap();
        apply_with(&InfoChange::Perms { paths: vec![d.clone()], mode: 0o775, recursive: true }, &SystemDefaults).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&d), 0o775);
        assert_eq!(mode(&d.join("sub")), 0o775);
        // A un archivo le llegan la lectura y la escritura; no la ejecución.
        assert_eq!(mode(&d.join("sub/a.txt")), 0o664);
        assert_eq!(read(&d).perms.unwrap().mode & 0o777, 0o775);
    }

    #[test]
    fn mac_volume_info_root() {
        let i = read(Path::new("/"));
        let v = i.volume.expect("la raíz es un disco");
        assert!(v.total > 0 && v.free > 0 && v.free <= v.total);
        assert!(!v.format.is_empty());
        assert!(!i.kind_name.is_empty());
        assert!(i.flags.is_empty() && i.hidden_ext.is_none());
    }

    /// «Abrir con» y «Cambiar todos…» van a `Defaults` (acá falso: no se toca la Mac).
    #[test]
    fn mac_open_with_uses_defaults() {
        #[derive(Default)]
        struct Fake(RefCell<Vec<(String, PathBuf, PathBuf)>>);
        impl Defaults for Fake {
            fn for_file(&self, app: &Path, file: &Path) -> Result<(), String> {
                self.0.borrow_mut().push(("archivo".into(), app.into(), file.into()));
                Ok(())
            }
            fn for_type(&self, app: &Path, file: &Path) -> Result<(), String> {
                self.0.borrow_mut().push(("tipo".into(), app.into(), file.into()));
                Err("falso".into())
            }
        }
        let fake = Fake::default();
        let app = "/System/Applications/TextEdit.app".to_string();
        apply_with(&InfoChange::OpenWith { paths: vec!["/x/a.txt".into()], app: app.clone() }, &fake).unwrap();
        assert_eq!(apply_with(&InfoChange::ChangeAll { path: "/x/a.txt".into(), app: app.clone() }, &fake), Err("falso".into()));
        let calls = fake.0.borrow();
        assert_eq!(calls[0], ("archivo".into(), PathBuf::from(&app), PathBuf::from("/x/a.txt")));
        assert_eq!(calls[1].0, "tipo");
    }
}

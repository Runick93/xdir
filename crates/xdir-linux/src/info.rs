//! Panel de información en Linux (`sys::Info`, sin interfaz todavía): `lstat` (en disco
//! `st_blocks × 512`), etiquetas y comentario en los atributos `user.xdg.tags` / `user.xdg.comment`
//! (los de KDE y Baloo), Oculto informativo (nombre con punto), Solo lectura = sin la escritura del
//! dueño, disco con `statvfs` y el formato de `/proc/self/mountinfo`, «Abrir con» por el tipo MIME
//! (de la extensión, sin leer el archivo) con `xdg-mime` y `mimeinfo.cache`, y permisos con
//! `chmod`. Sin «Más en el sistema…».

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use xdir_core::info::{AppRef, Flag, InfoChange, InfoReply, InfoRequest, ItemInfo, OpenWith, Perms, Tag, VolumeData};
use xdir_core::shell::Item;
use xdir_core::sys::{Handle, Info};

const TAGS: &str = "user.xdg.tags";
const COMMENT: &str = "user.xdg.comment";

/// El servicio que instala `sys::install`.
pub struct LinuxInfo;

impl Info for LinuxInfo {
    fn request(&self, req: InfoRequest) {
        let spawned = std::thread::Builder::new().name("xdir-informacion".into()).spawn(move || xdir_core::info::answer(req, &read));
        debug_assert!(spawned.is_ok(), "hilo de información");
    }

    fn apply(&self, owner: Handle, tx: Sender<InfoReply>, generation: u64, change: InfoChange) {
        let spawned = std::thread::Builder::new().name("xdir-informacion-cambio".into()).spawn(move || {
            let section = change.section();
            let error = apply(&change).err();
            xdir_core::info::send_applied(owner, &tx, generation, section, error);
        });
        debug_assert!(spawned.is_ok(), "hilo de cambio de información");
    }

    fn more_in_system(&self, _owner: Handle, _items: &[Item]) -> bool {
        false
    }

    fn has_more_in_system(&self) -> bool {
        false
    }
}

fn cpath(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

/// Un atributo extendido sin seguir enlaces: `Ok(None)` si no está; `Err` si el sistema de
/// archivos no los admite.
fn get_xattr(path: &Path, name: &str) -> Result<Option<Vec<u8>>, i32> {
    let (Some(c), Ok(n)) = (cpath(path), CString::new(name)) else { return Err(libc::EINVAL) };
    let len = unsafe { libc::lgetxattr(c.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0) };
    if len < 0 {
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return if e == libc::ENODATA { Ok(None) } else { Err(e) };
    }
    let mut buf = vec![0u8; len as usize];
    let got = unsafe { libc::lgetxattr(c.as_ptr(), n.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if got < 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
    }
    buf.truncate(got as usize);
    Ok(Some(buf))
}

fn set_xattr(path: &Path, name: &str, value: Option<&[u8]>) -> Result<(), String> {
    let (Some(c), Ok(n)) = (cpath(path), CString::new(name)) else { return Err("Ruta inválida".into()) };
    let r = match value {
        Some(v) => unsafe { libc::lsetxattr(c.as_ptr(), n.as_ptr(), v.as_ptr().cast(), v.len(), 0) },
        None => unsafe { libc::lremovexattr(c.as_ptr(), n.as_ptr()) },
    };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        if value.is_none() && e.raw_os_error() == Some(libc::ENODATA) {
            return Ok(());
        }
        return Err(match e.raw_os_error() {
            Some(libc::ENOTSUP) => "Este sistema de archivos no admite etiquetas ni comentarios".into(),
            _ => io_text(&e),
        });
    }
    Ok(())
}

/// Las etiquetas de `user.xdg.tags` (separadas por coma).
pub fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// El sistema de archivos de `path` según `/proc/self/mountinfo`: el del punto de montaje más
/// largo que la contiene (`\040` y compañía se desarman).
pub fn fs_type_for(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let (left, right) = line.split_once(" - ")?;
        let mount = unescape(left.split(' ').nth(4)?);
        let fstype = right.split(' ').next()?.to_string();
        let mp = Path::new(&mount);
        if path.starts_with(mp) && best.as_ref().is_none_or(|(len, _)| mount.len() >= *len) {
            best = Some((mount.len(), fstype));
        }
    }
    best.map(|(_, t)| t)
}

/// ¿`path` es un punto de montaje?
fn is_mount_point(mountinfo: &str, path: &Path) -> bool {
    mountinfo.lines().filter_map(|l| l.split(' ').nth(4)).any(|m| Path::new(&unescape(m)) == path)
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let n = (b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0');
            out.push(n as char);
            i += 4;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// Tipo MIME por el nombre, con los `globs2` de shared-mime-info (`peso:tipo:patrón`): no se lee
/// el archivo. Solo patrones `*.ext`, sin distinguir mayúsculas.
pub fn mime_for_name(globs2: &str, name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mut best: Option<(u32, usize, String)> = None;
    for line in globs2.lines().filter(|l| !l.starts_with('#')) {
        let mut parts = line.splitn(3, ':');
        let (Some(w), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let glob = glob.split(':').next().unwrap_or(glob);
        let Some(ext) = glob.strip_prefix('*') else { continue };
        if ext.contains(['*', '?', '[']) || !lower.ends_with(&ext.to_lowercase()) {
            continue;
        }
        let w: u32 = w.parse().unwrap_or(50);
        let better = best.as_ref().is_none_or(|(bw, bl, _)| (w, ext.len()) > (*bw, *bl));
        if better {
            best = Some((w, ext.len(), mime.to_string()));
        }
    }
    best.map(|(_, _, m)| m)
}

/// Las `.desktop` que abren `mime` según un `mimeinfo.cache`.
pub fn apps_for_mime(cache: &str, mime: &str) -> Vec<String> {
    cache
        .lines()
        .find_map(|l| l.strip_prefix(mime).and_then(|r| r.strip_prefix('=')))
        .map(|apps| apps.split(';').filter(|a| !a.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Carpetas de datos de XDG (la del usuario primero).
fn data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) => out.push(PathBuf::from(d)),
        None => {
            if let Some(h) = std::env::var_os("HOME") {
                out.push(PathBuf::from(h).join(".local/share"));
            }
        }
    }
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    out.extend(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    out
}

fn mime_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    data_dirs().iter().find_map(|d| std::fs::read_to_string(d.join("mime/globs2")).ok().and_then(|g| mime_for_name(&g, &name)))
}

fn desktop_file(id: &str) -> Option<PathBuf> {
    data_dirs().into_iter().map(|d| d.join("applications").join(id)).find(|p| p.is_file())
}

fn app_ref(id: &str) -> AppRef {
    let name = desktop_file(id)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("Name=").map(str::to_string)))
        .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
    AppRef { id: id.to_string(), name, icon: 0 }
}

fn open_with(path: &Path) -> Option<OpenWith> {
    let mime = mime_of(path)?;
    let default = std::process::Command::new("xdg-mime").args(["query", "default", &mime]).output().ok().and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        (o.status.success() && !s.is_empty()).then_some(s)
    });
    let mut ids: Vec<String> = default.iter().cloned().collect();
    for d in data_dirs() {
        if let Ok(cache) = std::fs::read_to_string(d.join("applications/mimeinfo.cache")) {
            for a in apps_for_mime(&cache, &mime) {
                if !ids.contains(&a) {
                    ids.push(a);
                }
            }
        }
    }
    let apps: Vec<AppRef> = ids.iter().map(|id| app_ref(id)).collect();
    (!apps.is_empty()).then(|| OpenWith { current: default.map(|d| app_ref(&d)), apps, per_file: false, system_dialog: false })
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

/// Todo lo del elemento, sin abrir su contenido.
pub fn read(path: &Path) -> ItemInfo {
    let mut info = xdir_core::info::read_minimal(path, &|_| false);
    let Ok(m) = std::fs::symlink_metadata(path) else { return info };
    if !info.is_dir {
        info.on_disk = m.blocks() * 512;
    }
    // Sin permiso para leer el atributo se admite igual (se ve vacío; escribir dirá el error);
    // `None` es solo cuando el sistema de archivos no los tiene.
    info.tags = match get_xattr(path, TAGS) {
        Ok(v) => Some(parse_tags(&String::from_utf8_lossy(&v.unwrap_or_default())).into_iter().map(|name| Tag { name, color: None }).collect()),
        Err(libc::EACCES | libc::EPERM) => Some(Vec::new()),
        Err(_) => None,
    };
    info.comment = match get_xattr(path, COMMENT) {
        Ok(v) => Some(String::from_utf8_lossy(&v.unwrap_or_default()).into_owned()),
        Err(libc::EACCES | libc::EPERM) => Some(String::new()),
        Err(_) => None,
    };
    let euid = unsafe { libc::geteuid() };
    let mine = euid == m.uid() || euid == 0;
    let dot = path.file_name().is_some_and(|n| n.as_bytes().first() == Some(&b'.'));
    // Un enlace: no se cambian Solo lectura ni los permisos (cambiarían el original).
    let link = m.file_type().is_symlink();
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mount = is_mount_point(&mountinfo, path);
    if mount {
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if let Some(c) = cpath(path)
            && unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0
        {
            let unit = st.f_frsize as u64;
            info.volume = Some(VolumeData { total: st.f_blocks as u64 * unit, free: st.f_bavail as u64 * unit, format: fs_type_for(&mountinfo, path).unwrap_or_default() });
            info.kind_name = "Disco".into();
        }
    } else {
        info.flags = vec![(Flag::Hidden, dot, false), (Flag::ReadOnly, m.mode() & 0o200 == 0, mine && !link)];
        if !info.is_dir {
            info.open_with = open_with(path);
        }
    }
    let c = cpath(path);
    let ok = |mode| c.as_ref().is_some_and(|c| unsafe { libc::access(c.as_ptr(), mode) } == 0);
    info.perms = Some(Perms {
        owner: user_name(m.uid()),
        group: group_name(m.gid()),
        mode: m.mode() & 0o7777,
        // Como en macOS: los de un disco entero (punto de montaje) y los de un enlace, solo para ver.
        editable: mine && !link && !mount,
        can_read: ok(libc::R_OK),
        can_write: ok(libc::W_OK),
    });
    info
}

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

fn joined(errors: Vec<String>) -> Result<(), String> {
    match errors.len() {
        0 => Ok(()),
        1 => Err(errors.into_iter().next().unwrap_or_default()),
        n => Err(format!("{} (y {} errores más)", errors[0], n - 1)),
    }
}

fn set_tags(path: &Path, add: &[String], remove: &[String]) -> Result<(), String> {
    let mut tags = match get_xattr(path, TAGS) {
        Ok(v) => parse_tags(&String::from_utf8_lossy(&v.unwrap_or_default())),
        Err(libc::EACCES | libc::EPERM) => return Err("No tenés permiso".into()),
        Err(_) => return Err("Este sistema de archivos no admite etiquetas".into()),
    };
    tags.retain(|t| !remove.contains(t));
    for a in add {
        // La coma separa etiquetas: se cambia antes de ver si ya está.
        let a = a.replace(',', " ");
        if !tags.contains(&a) {
            tags.push(a);
        }
    }
    let joined = tags.join(",");
    set_xattr(path, TAGS, (!joined.is_empty()).then_some(joined.as_bytes()))
}

/// `chmod` del elemento; con `recursive`, lo de adentro toma la lectura y la escritura de cada
/// clase (la ejecución queda como estaba). Lo de adentro va primero y cada carpeta al final (después
/// de listarla), así quitarle la lectura al dueño no corta el recorrido. Un enlace no se toca
/// (cambiaría el original). Junta los errores.
pub fn chmod(path: &Path, mode: u32, recursive: bool) -> Result<(), String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| io_text(&e))?;
    if m.file_type().is_symlink() {
        return Err("Es un enlace: los permisos se cambian en el original".into());
    }
    let mut errors = Vec::new();
    if recursive && m.is_dir() {
        chmod_contents(path, mode, &mut errors);
    }
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
        errors.push(io_text(&e));
    }
    joined(errors)
}

/// Lo de adentro de `dir`, de abajo hacia arriba (ver `chmod`).
fn chmod_contents(dir: &Path, mode: u32, errors: &mut Vec<String>) {
    let entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(e) => {
            errors.push(format!("{}: {}", dir.display(), io_text(&e)));
            return;
        }
    };
    for p in entries {
        let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
        if m.file_type().is_symlink() {
            continue;
        }
        if m.is_dir() {
            chmod_contents(&p, mode, errors);
        }
        let next = (m.mode() & 0o7777 & !0o666) | (mode & 0o666);
        if let Err(e) = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(next)) {
            errors.push(format!("{}: {}", p.display(), io_text(&e)));
        }
    }
}

fn set_read_only(path: &Path, on: bool) -> Result<(), String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| io_text(&e))?;
    if m.file_type().is_symlink() {
        return Err("Es un enlace: Solo lectura se cambia en el original".into());
    }
    let mode = m.mode() & 0o7777;
    let next = if on { mode & !0o200 } else { mode | 0o200 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(next)).map_err(|e| io_text(&e))
}

/// Lo abre una vez con la app (`gio launch` con su `.desktop`; si no está, `gtk-launch`).
/// Se espera a que el lanzador termine (corre en el hilo del cambio, vuelve enseguida) para saber
/// si anduvo.
fn open_once(path: &Path, id: &str) -> Result<(), String> {
    let ok = |cmd: &mut std::process::Command| cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
    if desktop_file(id).is_some_and(|d| ok(std::process::Command::new("gio").arg("launch").arg(d).arg(path))) {
        return Ok(());
    }
    if ok(std::process::Command::new("gtk-launch").arg(id.trim_end_matches(".desktop")).arg(path)) {
        return Ok(());
    }
    Err("No se pudo abrir con esa app".into())
}

fn change_all(path: &Path, id: &str) -> Result<(), String> {
    let mime = mime_of(path).ok_or("No se sabe el tipo de este archivo")?;
    let out = std::process::Command::new("xdg-mime").args(["default", id, &mime]).output().map_err(|e| io_text(&e))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Aplica un cambio (en el hilo que llama).
pub fn apply(change: &InfoChange) -> Result<(), String> {
    let each = |paths: &[PathBuf], f: &dyn Fn(&Path) -> Result<(), String>| joined(paths.iter().filter_map(|p| f(p).err()).collect());
    match change {
        InfoChange::Tags { paths, add, remove } => each(paths, &|p| set_tags(p, add, remove)),
        InfoChange::Comment { path, text } => set_xattr(path, COMMENT, (!text.is_empty()).then_some(text.as_bytes())),
        InfoChange::Flag { paths, flag: Flag::ReadOnly, on } => each(paths, &|p| set_read_only(p, *on)),
        InfoChange::Flag { flag: Flag::Hidden, .. } => Err("En Linux, ocultar es renombrar con un punto".into()),
        InfoChange::Flag { .. } | InfoChange::HideExt { .. } => Err("Esto no existe en Linux".into()),
        InfoChange::OpenWith { paths, app } => each(paths, &|p| open_once(p, app)),
        InfoChange::ChangeAll { path, app } => change_all(path, app),
        InfoChange::Perms { paths, mode, recursive } => each(paths, &|p| chmod(p, *mode, *recursive)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("xdir-linux-info").join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn linux_tags_xattr_roundtrip() {
        let d = temp("etiquetas");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        if read(&f).tags.is_none() {
            eprintln!("el sistema de archivos de {} no admite user.*: se saltea", d.display());
            return;
        }
        apply(&InfoChange::Tags { paths: vec![f.clone()], add: vec!["Viaje".into(), "Rojo".into()], remove: vec![] }).unwrap();
        assert_eq!(get_xattr(&f, TAGS).unwrap().unwrap(), b"Viaje,Rojo");
        apply(&InfoChange::Tags { paths: vec![f.clone()], add: vec![], remove: vec!["Viaje".into()] }).unwrap();
        let names: Vec<String> = read(&f).tags.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Rojo"]);
        apply(&InfoChange::Comment { path: f.clone(), text: "hola".into() }).unwrap();
        assert_eq!(read(&f).comment.as_deref(), Some("hola"));
        apply(&InfoChange::Comment { path: f.clone(), text: String::new() }).unwrap();
        assert_eq!(read(&f).comment.as_deref(), Some(""));
    }

    #[test]
    fn linux_readonly_toggles_owner_write() {
        let d = temp("solo-lectura");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o664)).unwrap();
        apply(&InfoChange::Flag { paths: vec![f.clone()], flag: Flag::ReadOnly, on: true }).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().mode() & 0o777, 0o464);
        let ro = read(&f).flags.iter().find(|x| x.0 == Flag::ReadOnly).copied().unwrap();
        assert!(ro.1 && ro.2);
        apply(&InfoChange::Flag { paths: vec![f.clone()], flag: Flag::ReadOnly, on: false }).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().mode() & 0o777, 0o664);
        // Oculto: informativo (punto) y apagado.
        let dot = d.join(".oculto");
        std::fs::write(&dot, b"x").unwrap();
        assert!(read(&dot).flags.contains(&(Flag::Hidden, true, false)));
        assert!(apply(&InfoChange::Flag { paths: vec![dot], flag: Flag::Hidden, on: false }).is_err());
        // Permisos recursivos.
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/b.txt"), b"x").unwrap();
        std::fs::set_permissions(d.join("sub/b.txt"), std::fs::Permissions::from_mode(0o600)).unwrap();
        apply(&InfoChange::Perms { paths: vec![d.join("sub")], mode: 0o775, recursive: true }).unwrap();
        assert_eq!(std::fs::metadata(d.join("sub/b.txt")).unwrap().mode() & 0o777, 0o664);
    }

    /// Un enlace: ni Solo lectura ni los permisos se pueden cambiar (cambiarían el original con el
    /// modo del enlace, 0o777).
    #[test]
    fn linux_symlink_not_editable() {
        let d = temp("enlace");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        let link = d.join("enlace");
        std::os::unix::fs::symlink(&f, &link).unwrap();
        let i = read(&link);
        assert!(!i.perms.unwrap().editable);
        assert!(i.flags.iter().any(|x| x.0 == Flag::ReadOnly && !x.2));
        assert!(apply(&InfoChange::Flag { paths: vec![link.clone()], flag: Flag::ReadOnly, on: true }).is_err());
        assert!(apply(&InfoChange::Perms { paths: vec![link.clone()], mode: 0o600, recursive: false }).is_err());
        assert_eq!(std::fs::metadata(&f).unwrap().mode() & 0o777, 0o644);
    }

    /// Sin lectura del dueño y aplicado a lo de adentro: se cambia todo (de abajo hacia arriba).
    #[test]
    fn linux_perms_recursive_without_owner_read() {
        let d = temp("sin-lectura");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/a.txt"), b"x").unwrap();
        std::fs::set_permissions(d.join("sub/a.txt"), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(d.join("sub"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let mode = |p: &Path| std::fs::symlink_metadata(p).unwrap().mode() & 0o777;
        let r = apply(&InfoChange::Perms { paths: vec![d.clone()], mode: 0o155, recursive: true });
        let (root, sub) = (mode(&d), mode(&d.join("sub")));
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(d.join("sub"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(r, Ok(()));
        assert_eq!((root, sub, mode(&d.join("sub/a.txt"))), (0o155, 0o155, 0o044));
    }

    /// Una etiqueta con coma se guarda sin ella y no se duplica.
    #[test]
    fn linux_tag_with_comma_not_duplicated() {
        let d = temp("coma");
        let f = d.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        if read(&f).tags.is_none() {
            return;
        }
        apply(&InfoChange::Tags { paths: vec![f.clone()], add: vec!["a,b".into()], remove: vec![] }).unwrap();
        apply(&InfoChange::Tags { paths: vec![f.clone()], add: vec!["a,b".into()], remove: vec![] }).unwrap();
        assert_eq!(get_xattr(&f, TAGS).unwrap().unwrap(), b"a b");
    }

    #[test]
    fn linux_fs_type_from_mountinfo() {
        let mi = "22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n\
                  30 22 8:2 / /home rw - btrfs /dev/sda2 rw\n\
                  40 22 8:3 / /media/usb\\040key rw - exfat /dev/sdb1 rw\n";
        assert_eq!(fs_type_for(mi, Path::new("/etc/passwd")).as_deref(), Some("ext4"));
        assert_eq!(fs_type_for(mi, Path::new("/home/javier/a")).as_deref(), Some("btrfs"));
        assert_eq!(fs_type_for(mi, Path::new("/media/usb key/x")).as_deref(), Some("exfat"));
        assert_eq!(fs_type_for(mi, Path::new("/homes")).as_deref(), Some("ext4"));
        assert!(is_mount_point(mi, Path::new("/media/usb key")));
        assert!(!is_mount_point(mi, Path::new("/media")));
        let root = read(Path::new("/"));
        assert!(root.volume.is_some_and(|v| v.total > 0 && !v.format.is_empty()));
    }

    #[test]
    fn linux_mime_cache_apps() {
        let cache = "[MIME Cache]\nimage/png=org.gnome.eog.desktop;gimp.desktop;\ntext/plain=org.gnome.TextEditor.desktop;\n";
        assert_eq!(apps_for_mime(cache, "image/png"), ["org.gnome.eog.desktop", "gimp.desktop"]);
        assert!(apps_for_mime(cache, "image/jpeg").is_empty());
        let globs = "# comentario\n50:image/png:*.png\n50:text/plain:*.txt\n10:text/x-readme:README*\n55:application/x-compressed-tar:*.tar.gz\n50:application/gzip:*.gz\n";
        assert_eq!(mime_for_name(globs, "Foto.PNG").as_deref(), Some("image/png"));
        assert_eq!(mime_for_name(globs, "a.tar.gz").as_deref(), Some("application/x-compressed-tar"));
        assert_eq!(mime_for_name(globs, "nada"), None);
        assert_eq!(parse_tags(" Rojo, Viaje,,"), ["Rojo", "Viaje"]);
    }
}

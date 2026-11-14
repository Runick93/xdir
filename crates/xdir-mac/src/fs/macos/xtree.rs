//! xtree en macOS: listado de carpetas con `getattrlistbulk` (una llamada por bloque de
//! entradas), discos de `/` y `/Volumes`, y el ajuste de Acceso total al disco (`xtree_permisos`).

use std::cell::RefCell;
use std::ffi::{CString, OsString, c_void};
use std::os::darwin::fs::MetadataExt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSURL};

use super::{UF_HIDDEN, drive_label, drive_name, drives};
use crate::platform::Handle;
use crate::xtree::colors::fmt_size;
use crate::xtree::scan::{DirListing, DriveEntry, FastScan, FileEntry, ListError, Lister, Progress};
use crate::xtree::tree::Node;

/// Buffer de `getattrlistbulk` (256 KiB), uno por hilo de trabajo.
const BULK_BUF: usize = 256 * 1024;
/// `SF_DATALESS` de `chflags`: archivo o carpeta que solo está en la nube (iCloud y similares) y
/// no ocupa lugar en el disco; entrar a la carpeta haría que el sistema la baje.
const SF_DATALESS: u32 = 0x4000_0000;
/// `EF_MAY_SHARE_BLOCKS` de `ATTR_CMNEXT_EXT_FLAGS`: puede compartir bloques con otro archivo
/// (copia APFS hecha con `clonefile`).
const EF_MAY_SHARE_BLOCKS: u64 = 0x1;
/// Tipos de `fsobj_type_t` (`VREG`, `VDIR`); el resto (enlaces simbólicos, sockets...) se ignora.
const VREG: u32 = 1;
const VDIR: u32 = 2;
/// Datos del sistema montados aparte en el disco de arranque (Data, Preboot, VM...): lo que
/// importa de Data ya se ve por los firmlinks de `/` (`/Users`, `/Applications`...).
const SYSTEM_VOLUMES: &str = "/System/Volumes";
const DATA_VOLUME: &str = "/System/Volumes/Data";
/// Ajuste de Privacidad y seguridad > Acceso total al disco.
const FULL_DISK_ACCESS_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

thread_local! {
    /// Buffer del listado: cada hilo del escáner reusa el suyo en todas sus carpetas (en `u64`
    /// para que quede alineado).
    static BULK: RefCell<Vec<u64>> = RefCell::new(vec![0u64; BULK_BUF / 8]);
}

fn c_path(path: &Path) -> Option<CString> {
    CString::new(path.as_os_str().as_bytes()).ok()
}

/// Dispositivo (`st_dev`) de `path`, siguiendo enlaces.
fn dev_of(path: &Path) -> Option<i32> {
    std::fs::metadata(path).ok().map(|m| m.st_dev() as i32)
}

/// Listador de carpetas de xtree: no sale de los discos permitidos.
pub struct MacLister {
    /// Dispositivos que se recorren: el de la raíz y, si la raíz es `/`, también el de los datos
    /// (`/System/Volumes/Data`, adonde llevan los firmlinks). Vacío si no se pudo leer la raíz.
    allowed: Vec<i32>,
    /// Carpeta que no se recorre (`/System/Volumes` cuando la raíz es `/`).
    skip: Option<PathBuf>,
}

impl MacLister {
    pub fn new(root: &Path) -> MacLister {
        let mut allowed: Vec<i32> = dev_of(root).into_iter().collect();
        let mut skip = None;
        if root == Path::new("/") {
            allowed.extend(dev_of(Path::new(DATA_VOLUME)));
            skip = Some(PathBuf::from(SYSTEM_VOLUMES));
        }
        MacLister { allowed, skip }
    }

    fn allows(&self, dev: i32) -> bool {
        self.allowed.contains(&dev)
    }

    fn skips(&self, path: &Path) -> bool {
        self.skip.as_deref() == Some(path)
    }
}

impl Lister for MacLister {
    fn list(&self, path: &Path) -> Result<DirListing, ListError> {
        BULK.with(|b| self.list_dir(path, &mut b.borrow_mut()))
    }
}

/// Atributos de una entrada de `getattrlistbulk`; `None` los que no vinieron.
#[derive(Default, Debug, PartialEq)]
struct Attrs<'a> {
    name: Option<&'a [u8]>,
    dev: Option<i32>,
    objtype: Option<u32>,
    flags: Option<u32>,
    fileid: Option<u64>,
    /// `ATTR_DIR_MOUNTSTATUS` (solo carpetas).
    mount: Option<u32>,
    links: Option<u32>,
    alloc: Option<i64>,
    /// `ATTR_CMNEXT_CLONEID` y `ATTR_CMNEXT_EXT_FLAGS` (van en `forkattr`).
    clone: Option<u64>,
    ext_flags: Option<u64>,
}

/// Lector con control de límites sobre los bytes de una entrada.
struct Cursor<'a> {
    entry: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.pos.checked_add(N)?;
        let bytes = self.entry.get(self.pos..end)?.try_into().ok()?;
        self.pos = end;
        Some(bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take::<4>().map(u32::from_ne_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.take::<4>().map(i32::from_ne_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.take::<8>().map(u64::from_ne_bytes)
    }
    fn i64(&mut self) -> Option<i64> {
        self.take::<8>().map(i64::from_ne_bytes)
    }
}

/// Decodifica una entrada (`entry` = sus bytes, empezando por el largo `u32`). Los atributos van
/// en el orden de sus bits (comunes, de carpeta, de archivo y los comunes extendidos, que van en
/// `forkattr`) y solo aparecen los que figuran en
/// `ATTR_CMN_RETURNED_ATTRS`, que siempre viene primero. El nombre es un `attrreference_t`:
/// desplazamiento relativo a la propia referencia y largo con el NUL final. `None` si la entrada
/// está cortada o el nombre apunta fuera de ella.
fn parse_entry(entry: &[u8]) -> Option<Attrs<'_>> {
    let mut c = Cursor { entry, pos: 4 };
    // attribute_set_t: commonattr, volattr, dirattr, fileattr, forkattr.
    let common = c.u32()?;
    let _vol = c.u32()?;
    let dir = c.u32()?;
    let file = c.u32()?;
    let ext = c.u32()?;
    let mut a = Attrs::default();
    if common & libc::ATTR_CMN_NAME != 0 {
        let at = c.pos;
        let (off, len) = (c.i32()?, c.u32()?);
        let start = usize::try_from(at as i64 + off as i64).ok()?;
        let raw = entry.get(start..start.checked_add(len as usize)?)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        a.name = Some(&raw[..end]);
    }
    if common & libc::ATTR_CMN_DEVID != 0 {
        a.dev = Some(c.i32()?);
    }
    if common & libc::ATTR_CMN_OBJTYPE != 0 {
        a.objtype = Some(c.u32()?);
    }
    if common & libc::ATTR_CMN_FLAGS != 0 {
        a.flags = Some(c.u32()?);
    }
    if common & libc::ATTR_CMN_FILEID != 0 {
        a.fileid = Some(c.u64()?);
    }
    if dir & libc::ATTR_DIR_MOUNTSTATUS != 0 {
        a.mount = Some(c.u32()?);
    }
    if file & libc::ATTR_FILE_LINKCOUNT != 0 {
        a.links = Some(c.u32()?);
    }
    if file & libc::ATTR_FILE_ALLOCSIZE != 0 {
        a.alloc = Some(c.i64()?);
    }
    if ext & libc::ATTR_CMNEXT_CLONEID != 0 {
        a.clone = Some(c.u64()?);
    }
    if ext & libc::ATTR_CMNEXT_EXT_FLAGS != 0 {
        a.ext_flags = Some(c.u64()?);
    }
    Some(a)
}

/// Cierra el descriptor al salir (también si algo entra en pánico).
struct Fd(i32);

impl Drop for Fd {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}

impl MacLister {
    fn list_dir(&self, path: &Path, buf: &mut [u64]) -> Result<DirListing, ListError> {
        let cpath = c_path(path).ok_or(ListError::Other)?;
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if fd < 0 {
            let err = std::io::Error::last_os_error().raw_os_error();
            return Err(if matches!(err, Some(libc::EACCES | libc::EPERM)) { ListError::Denied } else { ListError::Other });
        }
        let fd = Fd(fd);
        let mut out = DirListing::default();
        // Otro disco montado que se haya colado (o una raíz inaccesible): no se recorre.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd.0, &mut st) } != 0 || !self.allows(st.st_dev) {
            return Ok(out);
        }
        let mut al = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_RETURNED_ATTRS
                | libc::ATTR_CMN_NAME
                | libc::ATTR_CMN_DEVID
                | libc::ATTR_CMN_OBJTYPE
                | libc::ATTR_CMN_FLAGS
                | libc::ATTR_CMN_FILEID,
            volattr: 0,
            dirattr: libc::ATTR_DIR_MOUNTSTATUS,
            fileattr: libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE,
            // Comunes extendidos (con `FSOPT_ATTR_CMN_EXTENDED`): para contar una sola vez las copias APFS.
            forkattr: libc::ATTR_CMNEXT_CLONEID | libc::ATTR_CMNEXT_EXT_FLAGS,
        };
        let size = buf.len() * 8;
        loop {
            let n = unsafe {
                libc::getattrlistbulk(fd.0, &mut al as *mut libc::attrlist as *mut c_void, buf.as_mut_ptr() as *mut c_void, size, libc::FSOPT_ATTR_CMN_EXTENDED as u64)
            };
            if n <= 0 {
                break; // 0: no hay más entradas; -1: error a mitad de carpeta, queda lo leído
            }
            // Vista en bytes del buffer (solo lectura) para recorrer las entradas con límites.
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, size) };
            let mut off = 0usize;
            for _ in 0..n {
                let Some(len) = bytes.get(off..off + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap()) as usize) else { break };
                let Some(entry) = bytes.get(off..off + len).filter(|_| len >= 4) else { break };
                off += len;
                if let Some(a) = parse_entry(entry) {
                    self.add(path, a, &mut out);
                }
            }
        }
        Ok(out)
    }

    /// Suma una entrada al listado: carpetas sin enlaces simbólicos ni otros discos; archivos con
    /// su tamaño asignado. Lo que solo está en la nube (archivos y carpetas) no se cuenta.
    fn add(&self, dir: &Path, a: Attrs, out: &mut DirListing) {
        let Some(name) = a.name.filter(|n| !n.is_empty() && *n != b"." && *n != b"..") else { return };
        if a.dev.is_some_and(|d| !self.allows(d)) {
            return;
        }
        if a.flags.is_some_and(|f| f & SF_DATALESS != 0) {
            return;
        }
        match a.objtype {
            Some(VDIR) => {
                let full = dir.join(std::ffi::OsStr::from_bytes(name));
                if self.skips(&full) {
                    return;
                }
                // En un punto de montaje la entrada trae el dispositivo de la carpeta de abajo: el
                // del disco montado se pregunta aparte.
                if a.mount.is_some_and(|m| m & libc::DIR_MNTSTATUS_MNTPOINT != 0) && !dev_of(&full).is_some_and(|d| self.allows(d)) {
                    return;
                }
                out.dirs.push(OsString::from_vec(name.to_vec()));
            }
            Some(VREG) => {
                let size = a.alloc.unwrap_or(0).max(0) as u64;
                // El id solo hace falta si el espacio puede estar repetido: copias APFS que
                // comparten los datos (mismo id de clon) u otros enlaces duros al mismo archivo.
                let shares = a.ext_flags.is_some_and(|f| f & EF_MAY_SHARE_BLOCKS != 0);
                let id = match a.clone {
                    Some(clone) if shares => Some(clone),
                    _ => a.fileid.filter(|_| a.links.is_some_and(|l| l > 1)),
                };
                out.files.push(FileEntry { name: String::from_utf8_lossy(name).into(), size, id });
            }
            _ => {}
        }
    }
}

/// Listador de carpetas para el escáner de xtree.
pub fn xtree_lister(root: &Path) -> impl Lister + 'static {
    MacLister::new(root)
}

/// En macOS no hay un escaneo rápido de volumen entero: siempre se recorre.
pub fn xtree_fast_scan(_root: &Path, _progress: &Progress, _cancel: &AtomicBool) -> FastScan {
    FastScan::NotApplicable
}

/// Tecla A: abre Ajustes en Privacidad y seguridad > Acceso total al disco para que el usuario le
/// dé permiso a xdir. No hay escaneo elevado: devuelve `None` y el aviso sigue; el permiso nuevo
/// recién vale al reabrir xdir. NSWorkspace se usa en el hilo principal (la vista llama desde un hilo propio).
pub fn xtree_elevate(_hwnd: Handle, _root: &Path, _progress: &Progress, _cancel: &AtomicBool) -> Option<Vec<Node>> {
    dispatch2::DispatchQueue::main().exec_async(|| {
        if let Some(url) = NSURL::URLWithString(&NSString::from_str(FULL_DISK_ACCESS_URL)) {
            NSWorkspace::sharedWorkspace().openURL(&url);
        }
    });
    None
}

/// Sistema de archivos ("apfs") y capacidad total del volumen de `path`.
fn statfs(path: &Path) -> Option<(String, u64, u64)> {
    let c = c_path(path)?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let bytes: Vec<u8> = st.f_fstypename.iter().take_while(|&&b| b != 0).map(|&b| b as u8).collect();
    let bsize = st.f_bsize as u64;
    Some((String::from_utf8_lossy(&bytes).into_owned(), st.f_blocks.saturating_mul(bsize), st.f_bavail.saturating_mul(bsize)))
}

/// Discos que se ofrecen para analizar: el de arranque y los visibles de `/Volumes`; se vuelve a
/// leer al abrir el menú.
pub fn xtree_drives() -> Vec<DriveEntry> {
    let hidden = |p: &Path| {
        p.file_name().is_some_and(|n| n.as_bytes().starts_with(b"."))
            || std::fs::symlink_metadata(p).is_ok_and(|m| m.st_flags() & UF_HIDDEN != 0)
    };
    drives()
        .into_iter()
        .filter(|p| p == Path::new("/") || !hidden(p))
        .map(|path| {
            let detail = match statfs(&path) {
                Some((fs, total, _)) if total > 0 => format!("{fs} \u{B7} {}", fmt_size(total)),
                Some((fs, _, _)) => fs,
                None => String::new(),
            };
            DriveEntry { label: drive_label(&drive_name(&path)), path, detail }
        })
        .collect()
}

/// (total, libre) en bytes del volumen que contiene `path`. En APFS son los del contenedor: el
/// «Utilizado» incluye los otros volúmenes que lo comparten (sistema, VM, Preboot...).
pub fn volume_space(path: &Path) -> Option<(u64, u64)> {
    let (_, total, free) = statfs(path)?;
    (total > 0).then_some((total, free))
}

/// `true` si `path` es un disco entero: `/` o `/Volumes/X`.
pub fn whole_volume(path: &Path) -> bool {
    let comps: Vec<Component> = path.components().collect();
    match comps.as_slice() {
        [Component::RootDir] => true,
        [Component::RootDir, Component::Normal(v), Component::Normal(_)] => *v == "Volumes",
        _ => false,
    }
}

/// Carpeta que se analiza al abrir xtree sin elegir nada: el disco de arranque.
pub fn xtree_default_root() -> PathBuf {
    PathBuf::from("/")
}

/// Aviso de la barra de datos cuando hay carpetas sin permiso; la vista cambia `{n}` por la
/// cantidad (`xtree_permisos`: abre el ajuste de Acceso total al disco). macOS le da el permiso al proceso
/// recién al volver a abrirlo: por eso se pide reabrir xdir.
pub const XTREE_PERMISSION_HINT: &str = "{n} carpetas sin permiso: dale a xdir Acceso total al disco ({teclas}) y reabrí xdir";

/// Faltaron permisos (aviso de la barra de datos y `xtree_permisos`): carpetas que no se pudieron leer.
pub fn xtree_permission_issue(p: &Progress) -> bool {
    p.denied.load(Ordering::Relaxed) > 0
}

/// Texto de la barra de datos y de estado mientras se abre el ajuste de permisos (`xtree_permisos`).
pub const XTREE_ELEVATING_TEXT: &str = "Abriendo Ajustes > Acceso total al disco\u{2026}";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xtree::scan::walk;
    use crate::xtree::tree::{Tree, node_path};
    use std::ffi::OsStr;
    use std::os::darwin::fs::MetadataExt;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    /// Carpeta temporal propia de la prueba (nunca se toca el disco real del usuario).
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-xtree-mac-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn list(dir: &Path) -> DirListing {
        xtree_lister(dir).list(dir).expect("se puede listar")
    }

    /// Recorre `root` en otro hilo; falla si no termina en 20 s (un ciclo lo colgaría).
    fn walk_tmp(root: &Path) -> Vec<Node> {
        let (tx, rx) = std::sync::mpsc::channel();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let live: Tree = Arc::new(RwLock::new(Vec::new()));
            let r = walk(&root, "raiz", &xtree_lister(&root), &live, &Progress::default(), &AtomicBool::new(false));
            let _ = tx.send(r);
        });
        rx.recv_timeout(Duration::from_secs(20)).expect("el recorrido no terminó").expect("sin cancelar")
    }

    #[test]
    fn lists_sizes_and_dirs() {
        let dir = tmp("sizes");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("datos.bin"), vec![7u8; 10_000]).unwrap();
        let l = list(&dir);
        assert_eq!(l.dirs, ["sub"]);
        assert_eq!(l.files.len(), 1);
        assert_eq!(&*l.files[0].name, "datos.bin");
        assert!(l.files[0].size >= 10_000, "tamaño asignado: {}", l.files[0].size);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_links_have_id() {
        let dir = tmp("links");
        std::fs::write(dir.join("a"), vec![1u8; 5000]).unwrap();
        std::fs::hard_link(dir.join("a"), dir.join("b")).unwrap();
        std::fs::write(dir.join("c"), vec![1u8; 5000]).unwrap();
        let l = list(&dir);
        let id = |n: &str| l.files.iter().find(|f| &*f.name == n).unwrap_or_else(|| panic!("falta {n}")).id;
        assert!(id("a").is_some());
        assert_eq!(id("a"), id("b"));
        assert_eq!(id("c"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clones_share_id() {
        // Copias APFS (clonefile, como `cp -c` o el Finder al duplicar): comparten los bloques y
        // ocupan una sola vez.
        let dir = tmp("clones");
        std::fs::write(dir.join("orig"), vec![3u8; 50_000]).unwrap();
        let (src, dst) = (c_path(&dir.join("orig")).unwrap(), c_path(&dir.join("copia")).unwrap());
        assert_eq!(unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 0) }, 0, "{}", std::io::Error::last_os_error());
        std::fs::write(dir.join("otro"), vec![3u8; 50_000]).unwrap();
        let l = list(&dir);
        let id = |n: &str| l.files.iter().find(|f| &*f.name == n).unwrap_or_else(|| panic!("falta {n}")).id;
        assert!(id("orig").is_some());
        assert_eq!(id("orig"), id("copia"));
        assert_eq!(id("otro"), None);
        let t = walk_tmp(&dir);
        assert!(t[0].size < 2 * 50_000 + 20_000, "la copia no suma: {}", t[0].size);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn does_not_follow_symlinks() {
        let dir = tmp("symlink");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/x"), vec![1u8; 3000]).unwrap();
        std::os::unix::fs::symlink("..", dir.join("sub/loop")).unwrap();
        std::os::unix::fs::symlink("sub/x", dir.join("enlace")).unwrap();
        let l = list(&dir.join("sub"));
        assert!(l.dirs.is_empty(), "{:?}", l.dirs);
        assert_eq!(l.files.len(), 1);
        let l = list(&dir);
        assert_eq!(l.dirs, ["sub"]);
        assert!(l.files.is_empty(), "el enlace no cuenta como archivo");
        let t = walk_tmp(&dir);
        assert_eq!(t.len(), 3, "raíz, sub y x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_non_utf8_names() {
        let dir = tmp("nombres");
        // Nombres con acentos, otros alfabetos y espacios: `node_path` llega al archivo real.
        let name = "ñandú – 日本 (copia).txt";
        std::fs::create_dir(dir.join("carpeta ñ")).unwrap();
        std::fs::write(dir.join("carpeta ñ").join(name), vec![1u8; 2000]).unwrap();
        // Bytes que no son UTF-8: APFS (y HFS+) no los aceptan como nombre (EILSEQ), así que en
        // esos discos el nombre que devuelve el sistema siempre es UTF-8 válido.
        let raw = dir.join(OsStr::from_bytes(b"a\xffb"));
        let raw_ok = match std::fs::write(&raw, vec![1u8; 2000]) {
            Ok(()) => true,
            Err(e) => {
                assert_eq!(e.raw_os_error(), Some(libc::EILSEQ), "{e}");
                false
            }
        };
        let t = walk_tmp(&dir);
        let find = |n: &str| t.iter().position(|x| &*x.name == n).unwrap_or_else(|| panic!("falta {n}")) as u32;
        assert!(node_path(&t, &dir, find(name)).exists());
        if raw_ok {
            let lossy = OsStr::from_bytes(b"a\xffb").to_string_lossy().into_owned();
            assert!(node_path(&t, &dir, find(&lossy)).exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn denied_folder_is_reported() {
        let dir = tmp("denegada");
        let no = dir.join("no");
        std::fs::create_dir(&no).unwrap();
        std::fs::set_permissions(&no, std::fs::Permissions::from_mode(0o000)).unwrap();
        let r = xtree_lister(&dir).list(&no);
        std::fs::set_permissions(&no, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(r.err(), Some(ListError::Denied));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Entrada armada a mano: largo, atributos devueltos (nombre y tipo) y nombre "ab".
    fn entry(name_off: i32, name_len: u32) -> Vec<u8> {
        let mut e = Vec::new();
        e.extend(0u32.to_ne_bytes()); // largo, se completa al final
        e.extend((libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_NAME | libc::ATTR_CMN_OBJTYPE).to_ne_bytes());
        e.extend([0u8; 16]);
        e.extend(name_off.to_ne_bytes());
        e.extend(name_len.to_ne_bytes());
        e.extend(VREG.to_ne_bytes());
        e.extend(b"ab\0\0");
        let len = e.len() as u32;
        e[..4].copy_from_slice(&len.to_ne_bytes());
        e
    }

    #[test]
    fn parse_entry_checks_bounds() {
        // La referencia del nombre está en el byte 24; el nombre, 12 bytes después.
        let ok = entry(12, 3);
        let a = parse_entry(&ok).unwrap();
        assert_eq!(a.name, Some(&b"ab"[..]));
        assert_eq!(a.objtype, Some(VREG));
        assert_eq!(a.links, None);
        // Nombre que se sale de la entrada (hacia adelante o hacia atrás) o entrada cortada.
        assert!(parse_entry(&entry(12, 100)).is_none());
        assert!(parse_entry(&entry(-100, 3)).is_none());
        assert!(parse_entry(&ok[..30]).is_none());
        assert!(parse_entry(&ok[..10]).is_none());
    }

    /// Carpetas que solo están en la nube (iCloud, `SF_DATALESS`): no se entra (las bajaría).
    #[test]
    fn dataless_dirs_are_skipped() {
        let dir = tmp("nube");
        let l = MacLister::new(&dir);
        let mut out = DirListing::default();
        let carpeta = |name: &'static [u8], flags| Attrs { name: Some(name), objtype: Some(VDIR), flags: Some(flags), ..Default::default() };
        l.add(&dir, carpeta(b"nube", SF_DATALESS), &mut out);
        l.add(&dir, carpeta(b"local", 0), &mut out);
        assert_eq!(out.dirs, ["local"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El aviso dice que hay que reabrir xdir: el permiso nuevo no le llega al proceso abierto.
    #[test]
    fn permission_hint_asks_to_reopen() {
        assert_eq!(
            XTREE_PERMISSION_HINT.replace("{n}", "3"),
            "3 carpetas sin permiso: dale a xdir Acceso total al disco ({teclas}) y reabrí xdir"
        );
    }

    #[test]
    fn root_allows_data_volume() {
        let l = MacLister::new(Path::new("/"));
        let dev = |p: &Path| std::fs::symlink_metadata(p).unwrap().st_dev() as i32;
        let (root, data) = (dev(Path::new("/")), dev(Path::new("/System/Volumes/Data")));
        assert!(l.allows(root));
        assert!(l.allows(data));
        assert!(l.skips(Path::new("/System/Volumes")));
        assert!(!l.skips(Path::new("/System")));
        // Otro disco montado en /Volumes (si hay alguno): queda afuera.
        let other = std::fs::read_dir("/Volumes")
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
            .map(|p| dev(&p))
            .find(|&d| d != root && d != data);
        if let Some(d) = other {
            assert!(!l.allows(d));
        }
        // Una raíz que no es `/` no sale de su disco ni saltea nada.
        let dir = tmp("raiz");
        let l = MacLister::new(&dir);
        assert!(l.allows(dev(&dir)));
        assert!(!l.skips(Path::new("/System/Volumes")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

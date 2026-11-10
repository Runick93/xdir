//! Red en macOS (`MacNetwork`): conectar con NetFS (el sistema pide usuario y contraseña en su
//! ventana y los guarda en el Llavero), lo conectado con `NSFileManager`, desconectar con el
//! flujo de Expulsar y descubrir máquinas con Bonjour (`bonjour.rs`). Todo lo bloqueante corre
//! en hilos de trabajo; `is_remote` solo mira la lista de puntos de montaje de red guardada.

mod bonjour;

use std::ffi::{CStr, CString, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSFileManager, NSMutableDictionary, NSString, NSURL, NSVolumeEnumerationOptions};
use xdir_core::net::{Address, ConnectOptions, NetConnection, NetError};
use xdir_core::sys::{Handle, HostEvent, Network};

#[cfg(test)]
use bonjour::{BonjourHosts, Change};

#[link(name = "NetFS", kind = "framework")]
unsafe extern "C" {
    /// `NetFS.h`: monta `url` (todos los parámetros son objetos de Core Foundation; los nulos
    /// toman lo del sistema). `mountpoints` vuelve con un `CFArrayRef` propio (regla Create).
    fn NetFSMountURLSync(
        url: *const c_void,
        mountpath: *const c_void,
        user: *const c_void,
        passwd: *const c_void,
        open_options: *const c_void,
        mount_options: *const c_void,
        mountpoints: *mut *const c_void,
    ) -> i32;
}

/// `kNAUIOptionKey` / `kNAUIOptionAllowUI` de `NetFS.h`.
const UI_OPTION_KEY: &str = "UIOption";
const UI_OPTION_ALLOW: &str = "AllowUI";

/// Código de error de NetFS (errno o `-128`, cancelado por el usuario) → `NetError`.
pub fn mac_error(code: i32, host: &str, share: &str) -> NetError {
    match code {
        80 => NetError::BadCredentials,
        2 => NetError::NoShare { host: host.into(), share: share.into() },
        65 | 51 | 61 => NetError::NotFound { host: host.into() },
        60 => NetError::Timeout { host: host.into() },
        89 | -128 => NetError::Cancelled,
        other => NetError::Other(format!("error {other}")),
    }
}

/// Tipos de sistema de archivos de red.
const REMOTE_TYPES: &[&str] = &["smbfs", "afpfs", "nfs", "webdav"];

/// De los puntos de montaje (tipo, ruta), los de red.
pub fn remote_roots(mounts: &[(String, PathBuf)]) -> Vec<PathBuf> {
    mounts.iter().filter(|(t, _)| REMOTE_TYPES.contains(&t.as_str())).map(|(_, p)| p.clone()).collect()
}

/// Los puntos de montaje del sistema (`getfsstat` con `MNT_NOWAIT`: lo que sabe el kernel, sin
/// preguntar a ningún servidor).
fn mounts() -> Vec<(String, PathBuf)> {
    // SAFETY: primero se pide la cantidad (puntero nulo) y después se llena un búfer de ese tamaño.
    unsafe {
        let n = libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT);
        if n <= 0 {
            return Vec::new();
        }
        let mut buf: Vec<libc::statfs> = vec![std::mem::zeroed(); n as usize + 4];
        let size = (buf.len() * std::mem::size_of::<libc::statfs>()) as libc::c_int;
        let got = libc::getfsstat(buf.as_mut_ptr(), size, libc::MNT_NOWAIT);
        if got <= 0 {
            return Vec::new();
        }
        buf.truncate(got as usize);
        buf.iter()
            .map(|s| {
                let t = CStr::from_ptr(s.f_fstypename.as_ptr()).to_string_lossy().into_owned();
                let p = PathBuf::from(std::ffi::OsStr::from_bytes(CStr::from_ptr(s.f_mntonname.as_ptr()).to_bytes()));
                (t, p)
            })
            .collect()
    }
}

/// Un booleano de recurso de un volumen.
fn url_flag(url: &NSURL, key: &objc2_foundation::NSURLResourceKey) -> Option<bool> {
    let mut value: Option<Retained<AnyObject>> = None;
    // SAFETY: `value` recibe un objeto (o nada) del tipo que corresponde a la clave.
    unsafe { url.getResourceValue_forKey_error(&mut value, key) }.ok()?;
    value?.downcast::<objc2_foundation::NSNumber>().ok().map(|n| n.boolValue())
}

fn url_value(url: &NSURL, key: &objc2_foundation::NSURLResourceKey) -> Option<Retained<NSURL>> {
    let mut value: Option<Retained<AnyObject>> = None;
    // SAFETY: como `url_flag`.
    unsafe { url.getResourceValue_forKey_error(&mut value, key) }.ok()?;
    value?.downcast::<NSURL>().ok()
}

/// La dirección de una URL de volumen, sin contraseña (si no se entiende, la URL sin lo que
/// haya antes de `@`).
fn clean_address(raw: &str) -> String {
    Address::parse(raw).map(|a| a.to_url()).unwrap_or_else(|_| match (raw.split_once("://"), raw.rsplit_once('@')) {
        (Some((scheme, _)), Some((_, host))) => format!("{scheme}://{host}"),
        _ => raw.to_string(),
    })
}

pub struct MacNetwork {
    /// Puntos de montaje de red (para `is_remote`, sin E/S).
    remote: RwLock<Vec<PathBuf>>,
    /// Se negó el permiso de «Red local» (lo descubre `browse`).
    denied: Arc<AtomicBool>,
}

impl MacNetwork {
    pub fn new() -> MacNetwork {
        MacNetwork { remote: RwLock::new(remote_roots(&mounts())), denied: Arc::new(AtomicBool::new(false)) }
    }
}

impl Network for MacNetwork {
    fn supports(&self, scheme: &str) -> bool {
        matches!(scheme, "smb" | "afp")
    }

    fn connect(&self, _owner: Handle, addr: &Address, _opts: ConnectOptions, _cancel: &AtomicBool) -> Result<PathBuf, NetError> {
        if !self.supports(&addr.scheme) {
            return Err(NetError::Unsupported { scheme: addr.scheme.clone() });
        }
        let share = addr.share.clone().unwrap_or_default();
        let url = NSURL::URLWithString(&NSString::from_str(&addr.to_url())).ok_or_else(|| NetError::Other("dirección inválida".into()))?;
        let options: Retained<NSMutableDictionary<NSString, NSString>> = NSMutableDictionary::new();
        // SAFETY: claves y valores son `NSString` (los que espera NetFS).
        unsafe { options.setObject_forKey(&NSString::from_str(UI_OPTION_ALLOW), objc2::runtime::ProtocolObject::from_ref(&*NSString::from_str(UI_OPTION_KEY))) };
        let mut points: *const c_void = std::ptr::null();
        // SAFETY: NSURL y NSMutableDictionary son puentes sin costo de CFURL y CFMutableDictionary;
        // `points` queda nulo o con un arreglo propio que se toma abajo.
        let code = unsafe {
            NetFSMountURLSync(
                Retained::as_ptr(&url).cast(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                Retained::as_ptr(&options).cast(),
                std::ptr::null(),
                &mut points,
            )
        };
        // SAFETY: regla Create: el arreglo es nuestro y se suelta al salir.
        let points: Option<Retained<NSArray<NSString>>> = unsafe { Retained::from_raw(points as *mut NSArray<NSString>) };
        if code != 0 {
            return Err(mac_error(code, &addr.host, &share));
        }
        self.refresh_remote();
        let first = points.and_then(|p| p.firstObject()).map(|s| PathBuf::from(s.to_string()));
        let base = first.unwrap_or_else(|| PathBuf::from("/Volumes").join(&share));
        Ok(addr.rest.iter().fold(base, |p, r| p.join(r)))
    }

    fn connections(&self) -> Result<Vec<NetConnection>, NetError> {
        use objc2_foundation::{NSURLVolumeIsLocalKey, NSURLVolumeURLForRemountingKey};
        // SAFETY: claves estáticas de Foundation.
        let keys = unsafe { NSArray::from_slice(&[NSURLVolumeIsLocalKey, NSURLVolumeURLForRemountingKey]) };
        let fm = NSFileManager::defaultManager();
        let Some(urls) = fm.mountedVolumeURLsIncludingResourceValuesForKeys_options(Some(&keys), NSVolumeEnumerationOptions::SkipHiddenVolumes) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for url in urls.iter() {
            // SAFETY: claves estáticas de Foundation.
            if unsafe { url_flag(&url, NSURLVolumeIsLocalKey) } != Some(false) {
                continue;
            }
            let Some(path) = url.path() else { continue };
            // SAFETY: clave estática de Foundation.
            let remount = unsafe { url_value(&url, NSURLVolumeURLForRemountingKey) };
            let address = remount.and_then(|u| u.absoluteString()).map(|s| clean_address(&s.to_string())).unwrap_or_default();
            out.push(NetConnection { address, local: PathBuf::from(path.to_string()), letter: None, reconnect: false, connected: true });
        }
        Ok(out)
    }

    fn disconnect(&self, local: &Path, force: bool) -> Result<(), NetError> {
        let result = if force {
            let c = CString::new(local.as_os_str().as_bytes()).map_err(|_| NetError::Other("ruta inválida".into()))?;
            // SAFETY: ruta terminada en cero.
            match unsafe { libc::unmount(c.as_ptr(), libc::MNT_FORCE) } {
                0 => Ok(()),
                _ => Err(NetError::Other(std::io::Error::last_os_error().to_string())),
            }
        } else {
            crate::shell::macos::eject(local).map_err(|e| if e.busy { NetError::OpenFiles { by: e.busy_by } } else { NetError::Other(e.message) })
        };
        self.refresh_remote();
        result
    }

    fn shares(&self, _host: &str) -> Result<Vec<String>, NetError> {
        // macOS elige el recurso en su ventana al conectar a `smb://máquina`.
        Ok(Vec::new())
    }

    fn browse(&self, hwnd: Handle, tx: Sender<HostEvent>, cancel: Arc<AtomicBool>) {
        bonjour::browse(hwnd, tx, cancel, self.denied.clone());
    }

    fn is_remote(&self, path: &Path) -> bool {
        self.remote.read().map(|r| r.iter().any(|root| path.starts_with(root))).unwrap_or(false)
    }

    fn refresh_remote(&self) {
        let roots = remote_roots(&mounts());
        if let Ok(mut r) = self.remote.write() {
            *r = roots;
        }
    }

    fn can_force(&self) -> bool {
        true
    }

    fn local_network_denied(&self) -> bool {
        self.denied.load(Ordering::Relaxed)
    }

    fn open_credentials_place(&self, _owner: Handle) -> bool {
        std::process::Command::new("/usr/bin/open").args(["-b", "com.apple.keychainaccess"]).status().is_ok_and(|s| s.success())
    }

    fn has_credentials_place(&self) -> bool {
        true
    }

    fn open_local_network_settings(&self, _owner: Handle) {
        let _ = std::process::Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork").status();
    }
}

#[cfg(test)]
mod tests;

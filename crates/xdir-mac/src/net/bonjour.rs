//! Máquinas de la red local por Bonjour: un `nw_browser` de Network.framework por servicio
//! (`_smb._tcp` y `_afpovertcp._tcp`) en una cola propia; los nombres se juntan sin repetir entre
//! los dos servicios y salen como `HostEvent::Up/Down`. Si el sistema niega el permiso de «Red
//! local» (`kDNSServiceErr_PolicyDenied`), no se manda nada y queda anotado.

use std::collections::BTreeSet;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use xdir_core::net::WM_APP_NET;
use xdir_core::sys::{Handle, HostEvent};

/// Lo que cambió en los resultados de un servicio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(String),
    Removed(String),
}

/// Los nombres que anuncia cada servicio; una máquina está mientras la anuncie alguno.
#[derive(Default)]
pub struct BonjourHosts {
    sets: [BTreeSet<String>; 2],
}

impl BonjourHosts {
    /// Aplica un cambio del servicio `service` (0 o 1) y dice si la máquina apareció o se fue.
    pub fn apply(&mut self, service: usize, change: Change) -> Option<HostEvent> {
        let present = |s: &[BTreeSet<String>; 2], n: &str| s.iter().any(|set| set.contains(n));
        match change {
            Change::Added(n) => {
                let was = present(&self.sets, &n);
                self.sets[service].insert(n.clone());
                (!was).then_some(HostEvent::Up(n))
            }
            Change::Removed(n) => {
                if !self.sets[service].remove(&n) {
                    return None;
                }
                (!present(&self.sets, &n)).then_some(HostEvent::Down(n))
            }
        }
    }
}

const SERVICES: [&CStr; 2] = [c"_smb._tcp", c"_afpovertcp._tcp"];
/// `nw_browser_state_waiting` / `nw_browser_state_failed`.
const STATE_FAILED: i32 = 2;
const STATE_WAITING: i32 = 4;
/// `kDNSServiceErr_PolicyDenied`: el usuario negó el permiso de «Red local».
const POLICY_DENIED: i32 = -65570;

#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_browse_descriptor_create_bonjour_service(kind: *const c_char, domain: *const c_char) -> *mut c_void;
    fn nw_parameters_create() -> *mut c_void;
    fn nw_browser_create(descriptor: *mut c_void, parameters: *mut c_void) -> *mut c_void;
    fn nw_browser_set_queue(browser: *mut c_void, queue: *const c_void);
    fn nw_browser_set_browse_results_changed_handler(browser: *mut c_void, handler: *const c_void);
    fn nw_browser_set_state_changed_handler(browser: *mut c_void, handler: *const c_void);
    fn nw_browser_start(browser: *mut c_void);
    fn nw_browser_cancel(browser: *mut c_void);
    fn nw_browse_result_copy_endpoint(result: *mut c_void) -> *mut c_void;
    fn nw_endpoint_get_bonjour_service_name(endpoint: *mut c_void) -> *const c_char;
    fn nw_error_get_error_code(error: *mut c_void) -> i32;
    fn nw_release(obj: *mut c_void);
}

/// El nombre de máquina de un resultado.
fn result_name(result: *mut c_void) -> Option<String> {
    if result.is_null() {
        return None;
    }
    // SAFETY: `result` es un `nw_browse_result_t` vivo durante el bloque; el extremo se suelta.
    unsafe {
        let ep = nw_browse_result_copy_endpoint(result);
        if ep.is_null() {
            return None;
        }
        let name = nw_endpoint_get_bonjour_service_name(ep);
        let out = (!name.is_null()).then(|| CStr::from_ptr(name).to_string_lossy().into_owned());
        nw_release(ep);
        out.filter(|n| !n.is_empty())
    }
}

/// `nw_browser_state_ready`.
const STATE_READY: i32 = 1;

/// ¿Sigue negado el permiso de «Red local» después de este estado del navegador? Esperando o
/// fallido con `PolicyDenied`: sí; listo: ya no (el usuario lo dio después); otro: como estaba.
pub fn denied_after(state: i32, code: Option<i32>, prev: bool) -> bool {
    match (state, code) {
        (STATE_WAITING | STATE_FAILED, Some(POLICY_DENIED)) => true,
        (STATE_READY, _) => false,
        _ => prev,
    }
}

/// Corre hasta `cancel` (bloqueante: en su propio hilo).
pub fn browse(hwnd: Handle, tx: Sender<HostEvent>, cancel: Arc<AtomicBool>, denied: Arc<AtomicBool>) {
    let hosts = Arc::new(Mutex::new(BonjourHosts::default()));
    let tx = Arc::new(Mutex::new(tx));
    let queue = DispatchQueue::new("xdir.red.bonjour", None);
    let mut browsers = Vec::new();
    for (service, kind) in SERVICES.iter().enumerate() {
        let (hosts, tx, denied_r) = (hosts.clone(), tx.clone(), denied.clone());
        // El tercer argumento es el `bool` de C (un byte): «terminó el lote».
        let results: RcBlock<dyn Fn(*mut c_void, *mut c_void, u8)> = RcBlock::new(move |old: *mut c_void, new: *mut c_void, _complete: u8| {
            if denied_r.load(Ordering::Relaxed) {
                return;
            }
            let change = match (old.is_null(), new.is_null()) {
                (true, false) => result_name(new).map(Change::Added),
                (false, true) => result_name(old).map(Change::Removed),
                _ => None,
            };
            let Some(change) = change else { return };
            let event = hosts.lock().ok().and_then(|mut h| h.apply(service, change));
            if let (Some(e), Ok(tx)) = (event, tx.lock())
                && tx.send(e).is_ok()
            {
                hwnd.post(WM_APP_NET, 0, 0);
            }
        });
        let denied_s = denied.clone();
        let state: RcBlock<dyn Fn(i32, *mut c_void)> = RcBlock::new(move |state: i32, error: *mut c_void| {
            // SAFETY: `error` es un `nw_error_t` vivo durante el bloque (o nulo).
            let code = (!error.is_null()).then(|| unsafe { nw_error_get_error_code(error) });
            denied_s.store(denied_after(state, code, denied_s.load(Ordering::Relaxed)), Ordering::Relaxed);
        });
        // SAFETY: objetos creados acá; los bloques los copia Network.framework; la cola vive
        // hasta después de cancelar y soltar los navegadores.
        unsafe {
            let desc = nw_browse_descriptor_create_bonjour_service(kind.as_ptr(), std::ptr::null());
            let params = nw_parameters_create();
            let browser = nw_browser_create(desc, params);
            nw_release(desc);
            nw_release(params);
            if browser.is_null() {
                continue;
            }
            nw_browser_set_queue(browser, (&*queue as *const DispatchQueue).cast());
            nw_browser_set_browse_results_changed_handler(browser, RcBlock::as_ptr(&results).cast());
            nw_browser_set_state_changed_handler(browser, RcBlock::as_ptr(&state).cast());
            nw_browser_start(browser);
            browsers.push(browser);
        }
    }
    while !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(200));
    }
    for b in browsers {
        // SAFETY: navegadores creados arriba.
        unsafe {
            nw_browser_cancel(b);
            nw_release(b);
        }
    }
}

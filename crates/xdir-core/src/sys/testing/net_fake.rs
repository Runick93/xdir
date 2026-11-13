//! `Network` falso. Su estado es de todo el proceso (lo llaman los hilos de trabajo), salvo lo que
//! solo se pregunta en el hilo de interfaz (`supports`, `can_force`, `local_network_denied`,
//! `free_drive_letters`, la marca del hilo de interfaz), que es por hilo: cada test usa hosts y
//! rutas propios.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::net::{Address, ConnectOptions, NetConnection, NetError, WM_APP_NET};
use crate::sys::{Handle, HostEvent, Network};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

static CONNECT: Mutex<BTreeMap<String, Vec<Result<PathBuf, NetError>>>> = Mutex::new(BTreeMap::new());
static CONNECTS: Mutex<Vec<(String, ConnectOptions)>> = Mutex::new(Vec::new());
static CONNECTIONS: Mutex<BTreeMap<String, Vec<NetConnection>>> = Mutex::new(BTreeMap::new());
static CONNECTIONS_CALLS: Mutex<usize> = Mutex::new(0);
static DISCONNECT: Mutex<BTreeMap<PathBuf, Vec<Result<(), NetError>>>> = Mutex::new(BTreeMap::new());
static DISCONNECTS: Mutex<Vec<(PathBuf, bool)>> = Mutex::new(Vec::new());
static HOSTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static REMOTE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
static HANGS: Mutex<Vec<(String, Duration)>> = Mutex::new(Vec::new());
static BROWSING: Mutex<usize> = Mutex::new(0);
static FREE_LETTERS: Mutex<Vec<char>> = Mutex::new(Vec::new());

thread_local! {
    static SUPPORTS: RefCell<Vec<String>> = RefCell::new(vec!["smb".into(), "afp".into(), "sftp".into()]);
    static CAN_FORCE: Cell<bool> = const { Cell::new(false) };
    static DENIED: Cell<bool> = const { Cell::new(false) };
    static FREE_LETTERS_HERE: Cell<usize> = const { Cell::new(0) };
    static UI_THREAD: Cell<bool> = const { Cell::new(false) };
    static UI_IO: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CONNECT_ON_UI: Cell<usize> = const { Cell::new(0) };
}

pub struct FakeNetwork;

impl FakeNetwork {
    /// Esquemas que acepta (en este hilo); por defecto `smb`, `afp` y `sftp`.
    pub fn set_supports(schemes: &[&str]) {
        SUPPORTS.with(|s| *s.borrow_mut() = schemes.iter().map(|s| s.to_string()).collect());
    }

    /// Lo que devuelve `connect` para ese host (sin esto, `NotFound`).
    pub fn set_connect(host: &str, result: Result<PathBuf, NetError>) {
        lock(&CONNECT).insert(host.to_ascii_lowercase(), vec![result]);
    }

    /// Resultados de `connect` para ese host, en orden (el último se repite).
    pub fn set_connect_seq(host: &str, results: Vec<Result<PathBuf, NetError>>) {
        lock(&CONNECT).insert(host.to_ascii_lowercase(), results);
    }

    /// Las llamadas a `connect`: (URL, opciones sin la clave). La URL nunca lleva contraseña.
    pub fn connects() -> Vec<(String, ConnectOptions)> {
        lock(&CONNECTS).iter().map(|(u, o)| (u.clone(), o.without_secret())).collect()
    }

    /// Las de un host.
    pub fn connects_to(host: &str) -> Vec<(String, ConnectOptions)> {
        Self::connects().into_iter().filter(|(u, _)| Address::parse(u).is_ok_and(|a| a.host.eq_ignore_ascii_case(host))).collect()
    }

    /// Lo conectado de ese host (`connections` devuelve las de todos).
    pub fn set_connections(host: &str, list: Vec<NetConnection>) {
        lock(&CONNECTIONS).insert(host.to_ascii_lowercase(), list);
    }

    /// Cuántas veces se pidió `connections` (en todo el proceso).
    pub fn connections_calls() -> usize {
        *lock(&CONNECTIONS_CALLS)
    }

    /// Resultados de `disconnect(local)`, en orden (el último se repite); sin esto, `Ok`.
    pub fn set_disconnect(local: &Path, results: Vec<Result<(), NetError>>) {
        lock(&DISCONNECT).insert(local.to_path_buf(), results);
    }

    /// Las llamadas a `disconnect` de lo que está bajo `prefix`: (ruta, forzado).
    pub fn disconnects(prefix: &Path) -> Vec<(PathBuf, bool)> {
        lock(&DISCONNECTS).iter().filter(|(p, _)| p.starts_with(prefix)).cloned().collect()
    }

    /// Lo que manda `browse` al arrancar.
    pub fn set_hosts(hosts: Vec<String>) {
        *lock(&HOSTS) = hosts;
    }

    /// Cuántos `browse` están corriendo.
    pub fn browsing() -> usize {
        *lock(&BROWSING)
    }

    pub fn set_can_force(v: bool) {
        CAN_FORCE.with(|c| c.set(v));
    }

    pub fn set_local_network_denied(v: bool) {
        DENIED.with(|c| c.set(v));
    }

    pub fn set_free_letters(letters: Vec<char>) {
        *lock(&FREE_LETTERS) = letters;
    }

    /// Cuántas veces se pidieron las letras libres en este hilo.
    pub fn free_letters_here() -> usize {
        FREE_LETTERS_HERE.with(Cell::get)
    }

    /// Todo lo que está bajo `prefix` es remoto.
    pub fn set_remote(prefix: &Path) {
        lock(&REMOTE).insert(prefix.to_path_buf());
    }

    /// Las llamadas bloqueantes sobre ese host o lo que está bajo esa ruta duermen `d` (y las
    /// cargas del `FileSystem` falso también).
    pub fn hang(host_or_prefix: &str, d: Duration) {
        lock(&HANGS).push((host_or_prefix.to_string(), d));
    }

    /// Cuánto duerme lo que toca `key` (host o ruta).
    pub fn hang_for(key: &str) -> Option<Duration> {
        lock(&HANGS).iter().find(|(k, _)| key.eq_ignore_ascii_case(k) || Path::new(key).starts_with(k)).map(|(_, d)| *d)
    }

    /// Este hilo es el de interfaz: desde ahora se anota lo que toque una ruta remota en él.
    pub fn mark_ui_thread() {
        UI_THREAD.with(|c| c.set(true));
    }

    /// Lo que se hizo sobre rutas remotas en el hilo marcado (lo anotan los falsos).
    pub fn ui_io_calls() -> Vec<String> {
        UI_IO.with(|c| c.borrow().clone())
    }

    /// Cuántas veces se llamó `connect` en el hilo de este test.
    pub fn connects_on_this_thread() -> usize {
        CONNECT_ON_UI.with(Cell::get)
    }

    /// Anota `what` si corre en el hilo marcado y `path` es remota (lo llaman los falsos).
    pub fn note_io(what: &str, path: &Path) {
        if UI_THREAD.with(Cell::get) && Self::remote(path) {
            UI_IO.with(|c| c.borrow_mut().push(format!("{what} {}", path.display())));
        }
    }

    /// Anota `what` si corre en el hilo marcado, sea la ruta local o remota: para lo que nunca debe
    /// tocar el disco en el hilo de interfaz (el historial de xtree).
    pub fn note_disk_io(what: &str, path: &Path) {
        if UI_THREAD.with(Cell::get) {
            UI_IO.with(|c| c.borrow_mut().push(format!("{what} {}", path.display())));
        }
    }

    fn remote(path: &Path) -> bool {
        crate::fs::is_unc(path) || lock(&REMOTE).iter().any(|p| path.starts_with(p))
    }

    fn sleep_if_hung(key: &str) {
        if let Some(d) = Self::hang_for(key) {
            std::thread::sleep(d);
        }
    }
}

impl Network for FakeNetwork {
    fn supports(&self, scheme: &str) -> bool {
        SUPPORTS.with(|s| s.borrow().iter().any(|x| x == scheme))
    }

    fn connect(&self, _owner: Handle, addr: &Address, opts: ConnectOptions, _cancel: &AtomicBool) -> Result<PathBuf, NetError> {
        CONNECT_ON_UI.with(|c| c.set(c.get() + 1));
        lock(&CONNECTS).push((addr.to_url(), opts.without_secret()));
        drop(opts);
        Self::sleep_if_hung(&addr.host);
        let mut map = lock(&CONNECT);
        match map.get_mut(&addr.host.to_ascii_lowercase()) {
            Some(v) if v.len() > 1 => v.remove(0),
            Some(v) => v[0].clone(),
            None => Err(NetError::NotFound { host: addr.host.clone() }),
        }
    }

    fn connections(&self) -> Result<Vec<NetConnection>, NetError> {
        *lock(&CONNECTIONS_CALLS) += 1;
        Ok(lock(&CONNECTIONS).values().flatten().cloned().collect())
    }

    fn disconnect(&self, local: &Path, force: bool) -> Result<(), NetError> {
        lock(&DISCONNECTS).push((local.to_path_buf(), force));
        Self::sleep_if_hung(&local.to_string_lossy());
        let mut map = lock(&DISCONNECT);
        match map.get_mut(local) {
            Some(v) if v.len() > 1 => v.remove(0),
            Some(v) => v.first().cloned().unwrap_or(Ok(())),
            None => Ok(()),
        }
    }

    fn browse(&self, hwnd: Handle, tx: Sender<HostEvent>, cancel: Arc<AtomicBool>) {
        *lock(&BROWSING) += 1;
        for h in lock(&HOSTS).clone() {
            if tx.send(HostEvent::Up(h)).is_ok() {
                hwnd.post(WM_APP_NET, 0, 0);
            }
        }
        while !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
        }
        *lock(&BROWSING) -= 1;
    }

    fn is_remote(&self, path: &Path) -> bool {
        Self::remote(path)
    }

    fn refresh_remote(&self) {}

    fn free_drive_letters(&self) -> Vec<char> {
        FREE_LETTERS_HERE.with(|c| c.set(c.get() + 1));
        lock(&FREE_LETTERS).clone()
    }

    fn can_force(&self) -> bool {
        CAN_FORCE.with(Cell::get)
    }

    fn local_network_denied(&self) -> bool {
        DENIED.with(Cell::get)
    }

    fn open_credentials_place(&self, _owner: Handle) -> bool {
        true
    }

    fn has_credentials_place(&self) -> bool {
        true
    }
}

//! Pruebas de la red en la `App`: conectar, lo conectado, desconectar, «Red» en el árbol y que
//! nada de la red corra en el hilo de interfaz. Cada test usa hosts y rutas propios (el
//! `FakeNetwork` es de todo el proceso).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::App;
use super::tasks::TaskDone;
use crate::app::Init;
use crate::fs::Location;
use crate::net::{Address, NetConnection, NetError};
use crate::sys::Handle;
use crate::sys::testing::{FakeFileSystem, FakeHost, FakeNetwork, install_fakes};

pub(super) fn net_app() -> App {
    net_app_with(FakeHost::new())
}

pub(super) fn net_app_with(host: Rc<FakeHost>) -> App {
    install_fakes();
    App::new(Init::blank(), host)
}

/// Entrega los trabajos sueltos que lleguen hasta que `done` (o 10 s).
pub(super) fn wait_until(a: &mut App, done: impl Fn(&App) -> bool) {
    let t0 = Instant::now();
    while !done(a) {
        assert!(t0.elapsed() < Duration::from_secs(10), "no terminó a tiempo");
        if let Ok(t) = a.task_rx.recv_timeout(Duration::from_millis(20)) {
            a.on_task_done(t);
        }
        while let Ok(ev) = a.rx.try_recv() {
            a.on_load_event(ev);
        }
    }
}

#[allow(dead_code)]
/// Espera un trabajo suelto que cumpla `pick` y lo entrega (los demás también se entregan).
pub(super) fn wait_task(a: &mut App, pick: impl Fn(&TaskDone) -> bool) {
    let t0 = Instant::now();
    loop {
        assert!(t0.elapsed() < Duration::from_secs(10), "el trabajo no llegó");
        let Ok(t) = a.task_rx.recv_timeout(Duration::from_millis(20)) else { continue };
        let hit = pick(&t);
        a.on_task_done(t);
        if hit {
            return;
        }
    }
}

pub(super) fn conn(address: &str, local: &str) -> NetConnection {
    NetConnection { address: address.into(), local: PathBuf::from(local), letter: None, reconnect: false, connected: true }
}

#[test]
fn unc_auth_failure_connects_in_worker() {
    let mut a = net_app();
    let path = PathBuf::from(r"\\nas-n2auth\fotos");
    FakeFileSystem::set_load_error(&path, Some("Acceso denegado (os error 1326)"));
    FakeNetwork::set_connect("nas-n2auth", Ok(path.clone()));
    let before = FakeFileSystem::loads(&path);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(path.clone()));
    let calls_here = FakeNetwork::connects_on_this_thread();
    // Falla por credenciales → se conecta en otro hilo y, al volver, la pestaña recarga.
    wait_until(&mut a, |_| FakeNetwork::connects_to("nas-n2auth").len() == 1);
    // Al volver, recarga; si igual falla por credenciales, no insiste en esta navegación.
    wait_until(&mut a, |a| FakeFileSystem::loads(&path) >= before + 2 && !a.panes[0].tab().loading);
    assert_eq!(FakeNetwork::connects_on_this_thread(), calls_here, "connect no corre en el hilo de interfaz");
    assert_eq!(FakeNetwork::connects_to("nas-n2auth")[0].0, "smb://nas-n2auth/fotos");
    std::thread::sleep(Duration::from_millis(50));
    while let Ok(t) = a.task_rx.try_recv() {
        a.on_task_done(t);
    }
    assert_eq!(FakeNetwork::connects_to("nas-n2auth").len(), 1);
    FakeFileSystem::set_load_error(&path, None);
}

#[test]
fn connections_requested_at_start() {
    FakeNetwork::set_connections("nas-n2start", vec![conn("smb://nas-n2start/x", "/net/n2start/x")]);
    let mut a = net_app();
    a.attach(Handle(0), 96);
    wait_until(&mut a, |a| a.net.connections.iter().any(|c| c.address == "smb://nas-n2start/x"));
}

#[test]
fn fake_network_hang_sleeps() {
    install_fakes();
    FakeNetwork::hang("lento-n2", Duration::from_millis(200));
    let t0 = Instant::now();
    let addr = Address::parse("smb://lento-n2/x").unwrap();
    let r = crate::sys::network().connect(Handle(0), &addr, Default::default(), &Default::default());
    assert!(t0.elapsed() >= Duration::from_millis(200));
    assert_eq!(r, Err(NetError::NotFound { host: "lento-n2".into() }));
}

#[test]
fn connect_share_removed() {
    // Compila sin `FileSystem::connect_share` (los `cargo check` de las 3 plataformas lo
    // verifican del lado de cada sistema); la ruta UNC escrita en la barra navega sin tocarla.
    let mut a = net_app();
    let p = Path::new(r"\\nas-n2bar\x");
    a.go_to_text(0, &p.to_string_lossy());
    assert_eq!(a.panes[0].tab().location, Location::Path(p.to_path_buf()));
}

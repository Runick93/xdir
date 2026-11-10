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

// ---------- N4: Conectar a un servidor… ----------

use crate::commands::{Command, Keymap, combo_label_in};
use crate::host::TextKind;
use crate::net::dialog::DialogHit;
use crate::sys::KeyStyle;
use crate::sys::testing::{FakeShell, ShellCall};

pub(super) fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-red").join(format!("{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn field(a: &App) -> String {
    a.connect_field_text()
}

fn type_field(a: &mut App, host: &FakeHost, text: &str) {
    let b = a.net.dialog.as_ref().and_then(|d| d.field).expect("caja de la dirección");
    crate::host::WindowHost::text_set(host, b, text);
}

fn open_with(text: &str) -> (App, Rc<FakeHost>) {
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    a.run_command(Command::ConnectServer);
    type_field(&mut a, &host, text);
    (a, host)
}

fn wait_dialog_idle(a: &mut App) {
    wait_until(a, |a| a.net.dialog.as_ref().is_none_or(|d| d.busy.is_none()));
}

#[test]
fn ctrl_k_opens_dialog_with_focus() {
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    let km = Keymap::default();
    let k = crate::commands::parse_combo("Ctrl+K").unwrap();
    assert_eq!(km.lookup(crate::keys::Key(k.vk), true, false, false, false), Some(Command::ConnectServer));
    assert_eq!(combo_label_in(KeyStyle::Mac, k), "⌘K");
    a.run_command(Command::ConnectServer);
    let b = a.net.dialog.as_ref().and_then(|d| d.field).unwrap();
    assert!(crate::host::WindowHost::text_has_focus(&*host, b));
    assert_eq!(host.text_boxes_of(TextKind::Field).len(), 1);
    // Abrirlo otra vez no duplica nada; Esc lo cierra y suelta la caja.
    a.run_command(Command::ConnectServer);
    assert_eq!(host.text_boxes_of(TextKind::Field).len(), 1);
    a.on_key(crate::keys::keys::VK_ESCAPE);
    assert!(a.net.dialog.is_none());
    assert!(host.text_boxes_of(TextKind::Field).is_empty());
}

#[test]
fn dialog_prefills_remote_folder() {
    let mut a = net_app();
    FakeNetwork::set_remote(Path::new("/net/n4pre"));
    a.net.connections = vec![conn("smb://nas-n4pre/fotos", "/net/n4pre/fotos")];
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(PathBuf::from("/net/n4pre/fotos/2024")));
    a.run_command(Command::ConnectServer);
    assert_eq!(field(&a), "smb://nas-n4pre/fotos/2024");
    a.close_connect_dialog();
    // Una carpeta local: vacío.
    let local = tmp_dir("n4-local");
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(local));
    a.run_command(Command::ConnectServer);
    assert_eq!(field(&a), "");
}

#[test]
fn invalid_address_keeps_dialog() {
    let (mut a, _h) = open_with("smb://na s/x");
    a.on_field_done(true);
    let d = a.net.dialog.as_ref().expect("el panel sigue abierto");
    assert_eq!(d.error.as_deref(), Some("Escribí una dirección como smb://servidor/recurso"));
    assert!(d.busy.is_none());
}

#[test]
fn unsupported_scheme_message() {
    FakeNetwork::set_supports(&["smb"]);
    let (mut a, _h) = open_with("sftp://x-n4/y");
    a.connect_dialog_click(Some(DialogHit::Connect), false);
    assert_eq!(a.net.dialog.as_ref().unwrap().error.as_deref(), Some("En este sistema no se puede conectar a direcciones sftp://"));
    assert!(FakeNetwork::connects_to("x-n4").is_empty());
}

#[test]
fn connect_opens_new_tab_and_adds_recent() {
    let target = tmp_dir("n4-ok");
    FakeNetwork::set_connect("nas-n4ok", Ok(target.clone()));
    let (mut a, _h) = open_with("smb://nas-n4ok/fotos");
    let before = a.panes[a.active].tabs.len();
    let other = a.panes[a.active].tab().location.clone();
    a.connect_dialog_click(Some(DialogHit::Connect), false);
    assert!(a.net.dialog.as_ref().unwrap().busy.is_some());
    wait_until(&mut a, |a| a.net.dialog.is_none());
    let pane = &a.panes[a.active];
    assert_eq!(pane.tabs.len(), before + 1);
    assert_eq!(pane.tab().location, Location::Path(target));
    assert!(pane.tabs.iter().any(|t| t.location == other));
    assert_eq!(a.net.places.recents[0].address, "smb://nas-n4ok/fotos");
    let red = crate::session::config_dir().unwrap().join("red.toml");
    wait_until(&mut a, |_| std::fs::read_to_string(&red).is_ok_and(|t| t.contains("smb://nas-n4ok/fotos")));
}

#[test]
fn connect_error_stays_in_dialog() {
    FakeNetwork::set_connect("nas-n4bad", Err(NetError::BadCredentials));
    let (mut a, _h) = open_with("smb://nas-n4bad/fotos");
    a.on_field_done(true);
    wait_dialog_idle(&mut a);
    let d = a.net.dialog.as_ref().expect("abierto");
    assert_eq!(d.error.as_deref(), Some("Usuario o contraseña incorrectos"));
    // Se corrige y se reintenta.
    FakeNetwork::set_connect("nas-n4bad", Ok(tmp_dir("n4-bad")));
    a.on_field_done(true);
    wait_until(&mut a, |a| a.net.dialog.is_none());
}

#[test]
fn cancel_discards_late_result() {
    FakeNetwork::set_connect("nas-n4late", Ok(tmp_dir("n4-late")));
    FakeNetwork::hang("nas-n4late", Duration::from_millis(300));
    let (mut a, _h) = open_with("smb://nas-n4late/x");
    let tabs = a.panes[a.active].tabs.len();
    a.connect_dialog_click(Some(DialogHit::Connect), false);
    a.connect_dialog_click(Some(DialogHit::Cancel), false);
    assert!(a.net.dialog.as_ref().is_some_and(|d| d.busy.is_none()), "Cancelar con un intento en curso lo descarta y el panel queda");
    wait_task(&mut a, |t| matches!(t, TaskDone::Connected { .. }));
    assert!(a.net.dialog.is_some());
    assert_eq!(a.panes[a.active].tabs.len(), tabs);
    assert!(a.net.places.recents.is_empty());
}

#[test]
fn connect_dialog_strips_password() {
    let target = tmp_dir("n4-pw");
    FakeNetwork::set_connect("nas-n4pw", Ok(target));
    let (mut a, _h) = open_with("smb://ana:secreto@nas-n4pw/x");
    a.on_field_done(true);
    assert!(!field(&a).contains("secreto"), "la caja ya no muestra la clave");
    wait_until(&mut a, |a| a.net.dialog.is_none());
    for (url, _) in FakeNetwork::connects_to("nas-n4pw") {
        assert!(!url.contains("secreto"));
    }
    assert_eq!(a.net.places.recents[0].address, "smb://ana@nas-n4pw/x");
    let red = crate::session::config_dir().unwrap().join("red.toml");
    wait_until(&mut a, |_| std::fs::read_to_string(&red).is_ok_and(|t| t.contains("nas-n4pw")));
    assert!(!std::fs::read_to_string(&red).unwrap().contains("secreto"));
    assert!(!a.notice.as_deref().unwrap_or("").contains("secreto"));
    assert!(a.panes.iter().flat_map(|p| p.tabs.iter()).all(|t| !t.error.as_deref().unwrap_or("").contains("secreto")));
    // Un error tampoco la muestra.
    FakeNetwork::set_connect("nas-n4pw2", Err(NetError::NotFound { host: "nas-n4pw2".into() }));
    a.run_command(Command::ConnectServer);
    let host_text = "smb://ana:secreto@nas-n4pw2/x";
    let b = a.net.dialog.as_ref().and_then(|d| d.field).unwrap();
    crate::host::WindowHost::text_set(&*a.host, b, host_text);
    a.on_field_done(true);
    wait_dialog_idle(&mut a);
    assert!(!a.net.dialog.as_ref().unwrap().error.as_deref().unwrap().contains("secreto"));
}

#[test]
fn star_toggles_favorite_with_name() {
    let (mut a, host) = open_with("smb://nas-n4star/f");
    a.connect_dialog_click(Some(DialogHit::Star), false);
    let nb = a.net.dialog.as_ref().and_then(|d| d.naming).expect("pide el nombre");
    crate::host::WindowHost::text_set(&*host, nb, "Mi NAS");
    a.on_field_done(true);
    assert!(a.net.dialog.as_ref().unwrap().naming.is_none());
    assert_eq!(a.net.places.favorites.len(), 1);
    assert_eq!(a.net.places.favorites[0].name, "Mi NAS");
    assert!(a.net.dialog.is_some(), "Enter en el nombre no conecta ni cierra");
    // Otra vez: la quita.
    a.connect_dialog_click(Some(DialogHit::Star), false);
    assert!(a.net.places.favorites.is_empty());
    // Nombre vacío: la dirección.
    a.connect_dialog_click(Some(DialogHit::Star), false);
    a.on_field_done(true);
    assert_eq!(a.net.places.favorites[0].name, "smb://nas-n4star/f");
}

#[test]
fn double_click_recent_connects() {
    FakeNetwork::set_connect("nas-n4dbl", Ok(tmp_dir("n4-dbl")));
    let (mut a, _h) = open_with("");
    a.net.places.add_recent(&Address::parse("smb://nas-n4dbl/r").unwrap(), 5);
    a.connect_dialog_click(Some(DialogHit::Recent(0)), false);
    assert_eq!(field(&a), "smb://nas-n4dbl/r");
    assert!(FakeNetwork::connects_to("nas-n4dbl").is_empty(), "un clic solo pone la dirección");
    a.connect_dialog_click(Some(DialogHit::Recent(0)), true);
    wait_until(&mut a, |a| a.net.dialog.is_none());
    assert_eq!(FakeNetwork::connects_to("nas-n4dbl").len(), 1);
    // ✕ quita un reciente.
    a.run_command(Command::ConnectServer);
    a.connect_dialog_click(Some(DialogHit::RemoveRecent(0)), false);
    assert!(a.net.places.recents.is_empty());
}

#[test]
fn address_bar_url_connects_in_place() {
    let target = tmp_dir("n4-bar");
    FakeNetwork::set_connect("nas-n4bar", Ok(target.clone()));
    let mut a = net_app();
    let tabs = a.panes[0].tabs.len();
    a.go_to_text(0, "smb://nas-n4bar/x");
    wait_until(&mut a, |a| a.panes[0].tab().location == Location::Path(target.clone()));
    assert_eq!(a.panes[0].tabs.len(), tabs);
    assert_eq!(a.net.places.recents[0].address, "smb://nas-n4bar/x");
    // Un error queda en la pestaña.
    FakeNetwork::set_connect("nas-n4bar2", Err(NetError::NotFound { host: "nas-n4bar2".into() }));
    a.go_to_text(0, "smb://nas-n4bar2/x");
    wait_until(&mut a, |a| a.panes[0].tab().error.is_some());
    assert_eq!(a.panes[0].tab().error.as_deref(), Some("No se encuentra el servidor \"nas-n4bar2\""));
}

#[test]
fn computer_background_menu_has_connect() {
    let mut a = net_app();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Computer);
    FakeShell::take_calls();
    a.on_context_menu(Some((40.0, 300.0)));
    let own = FakeShell::take_calls().into_iter().find_map(|c| match c {
        ShellCall::ContextMenu { own, .. } => Some(own),
        _ => None,
    });
    let labels: Vec<String> = own
        .unwrap()
        .into_iter()
        .filter_map(|e| match e {
            crate::shell::Entry::Item { label, .. } => Some(label),
            _ => None,
        })
        .collect();
    for l in ["Conectar a un servidor…", "Conectar unidad de red…", "Desconectar unidad de red…"] {
        assert!(labels.iter().any(|x| x == l), "{l}: {labels:?}");
    }
}

#[test]
fn windows_only_commands_available_in_fake_env() {
    install_fakes();
    assert!(crate::commands::available(Command::MapNetworkDrive));
    assert!(crate::commands::available(Command::DisconnectNetworkDrive));
}

#[test]
fn draw_connect_dialog_in_app() {
    let (mut a, host) = open_with("smb://nas-n4draw/x");
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|t| t.1).collect();
    assert!(texts.iter().any(|t| t == "Conectar a un servidor"), "{texts:?}");
}

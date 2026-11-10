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
    FakeNetwork::hang("nas-n4bar", Duration::from_millis(100));
    a.go_to_text(0, "smb://nas-n4bar/x");
    a.paint();
    assert_eq!(a.panes[0].tab().list.placeholder, Some(crate::list::placeholder::Placeholder::Connecting("nas-n4bar".into())));
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

// ---------- N5: que la red nunca trabe la ventana ----------

use crate::list::placeholder::{Placeholder, PlaceholderButton};
use crate::pane::PaneHit;
use crate::tab::Slow;

/// Una carpeta real que el `FakeNetwork` trata como de red, con una subcarpeta y un archivo.
fn remote_dir(name: &str) -> PathBuf {
    let d = tmp_dir(name);
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("a.txt"), "x").unwrap();
    FakeNetwork::set_remote(&d);
    d
}

fn pump_pane(a: &mut App, pane: usize) {
    wait_until(a, |a| !a.panes[pane].tab().loading);
}

#[test]
fn remote_navigation_no_ui_io() {
    let root = remote_dir("n5-nav");
    let mut init = Init::blank();
    init.session.panes = vec![crate::session::PaneState { tabs: vec![Location::Path(root.join("sub"))], active: 0 }];
    FakeNetwork::mark_ui_thread();
    let host = FakeHost::new();
    install_fakes();
    let mut a = App::new(init, host);
    a.sync_visibility();
    pump_pane(&mut a, 0);
    // Pestaña nueva, navegar, abrir una subcarpeta de la lista, Atrás.
    a.new_tab(0, Location::Path(root.clone()), true);
    pump_pane(&mut a, 0);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    let idx = a.panes[0].tab().list.entries.iter().position(|e| e.name == "sub").expect("la lista tiene «sub»");
    a.panes[0].tab_mut().open(hwnd, &tx, idx);
    pump_pane(&mut a, 0);
    assert_eq!(a.panes[0].tab().location, Location::Path(root.join("sub")));
    a.panes[0].tab_mut().go_back(hwnd, &tx);
    pump_pane(&mut a, 0);
    // Un archivo de la lista: se abre sin mirar el disco.
    let f = a.panes[0].tab().list.entries.iter().position(|e| e.name == "a.txt").unwrap();
    a.panes[0].tab_mut().open(hwnd, &tx, f);
    // La barra: una carpeta y una que no existe (navega igual; la carga dice el error).
    a.go_to_text(0, &root.join("sub").to_string_lossy());
    pump_pane(&mut a, 0);
    a.go_to_text(0, &root.join("no-existe").to_string_lossy());
    assert_eq!(a.panes[0].tab().location, Location::Path(root.join("no-existe")));
    pump_pane(&mut a, 0);
    // Cambió el conjunto de unidades; un grupo con esa ruta.
    a.drives_changed();
    a.groups.put(crate::groups::Group {
        name: "red-n5".into(),
        layout: 0,
        split_x: 0.5,
        split_y: 0.5,
        panes: vec![crate::session::PaneState { tabs: vec![Location::Path(root.clone())], active: 0 }],
        temp: false,
    });
    let gi = a.groups.find("red-n5").unwrap();
    a.apply_group(gi);
    pump_pane(&mut a, 0);
    a.paint();
    assert_eq!(FakeNetwork::ui_io_calls(), Vec::<String>::new());
}

#[test]
fn remote_watcher_created_off_thread() {
    let root = remote_dir("n5-watch");
    FakeNetwork::mark_ui_thread();
    let mut a = net_app();
    a.sync_visibility();
    assert!(a.panes[0].tab().is_visible());
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(root.clone()));
    assert!(FakeNetwork::ui_io_calls().is_empty());
    wait_until(&mut a, |_| FakeFileSystem::watches(&root).0 == 1);
    // Al irse, el guardián se suelta.
    let local = tmp_dir("n5-watch-local");
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(local));
    wait_until(&mut a, |_| FakeFileSystem::watches(&root).1 == 1);
}

#[test]
fn slow_state_thresholds() {
    let root = remote_dir("n5-slow");
    FakeNetwork::hang(&root.to_string_lossy(), Duration::from_secs(2));
    let mut a = net_app();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(root.clone()));
    let t0 = Instant::now();
    a.panes[0].tab_mut().set_load_started(Some(t0));
    let tab = a.panes[0].tab();
    let host = crate::tab::remote_host(&root);
    assert_eq!(tab.slow_state(t0 + Duration::from_millis(400)), Slow::No);
    assert_eq!(tab.slow_state(t0 + Duration::from_millis(600)), Slow::Connecting { host: host.clone() });
    assert_eq!(tab.slow_state(t0 + Duration::from_secs(21)), Slow::NotResponding { host });
    // Una carpeta local nunca.
    let mut b = net_app();
    let local = tmp_dir("n5-slow-local");
    b.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(local));
    assert_eq!(b.panes[0].tab().slow_state(Instant::now() + Duration::from_secs(30)), Slow::No);
    // Con un lote ya no es lenta.
    wait_until(&mut a, |a| !a.panes[0].tab().loading);
    assert_eq!(a.panes[0].tab().slow_state(t0 + Duration::from_secs(21)), Slow::No);
}

#[test]
fn hung_load_shows_not_responding_and_close_works() {
    let root = remote_dir("n5-hung");
    FakeNetwork::hang(&root.to_string_lossy(), Duration::from_secs(3));
    FakeNetwork::set_connections("nas-n5hung", vec![conn("smb://nas-n5hung/fotos", &root.to_string_lossy())]);
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    a.net.connections = vec![conn("smb://nas-n5hung/fotos", &root.to_string_lossy())];
    a.new_tab(0, Location::Path(root.clone()), true);
    let t0 = Instant::now() - Duration::from_millis(700);
    a.panes[0].tab_mut().set_load_started(Some(t0));
    a.paint();
    let texts = |h: &FakeHost| h.drawn_texts().into_iter().map(|t| t.1).collect::<Vec<_>>();
    assert!(texts(&host).iter().any(|t| t == "Conectando con \"nas-n5hung\"…"), "{:?}", texts(&host));
    a.panes[0].tab_mut().set_load_started(Some(Instant::now() - Duration::from_secs(21)));
    a.paint();
    assert!(texts(&host).iter().any(|t| t == "\"nas-n5hung\" no responde"));
    assert_eq!(a.panes[0].tab().list.placeholder, Some(Placeholder::NotResponding("nas-n5hung".into())));
    // Cambiar de panel funciona en el medio.
    a.set_layout(crate::ui::Layout::TwoSide);
    a.set_active(1);
    a.set_active(0);
    a.paint();
    // «Cerrar la pestaña».
    let tabs = a.panes[0].tabs.len();
    let (r, _) = *a.panes[0].tab().list.placeholder_buttons.iter().find(|(_, b)| *b == PlaceholderButton::CloseTab).unwrap();
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert!(matches!(a.hit(x, y), super::Hit::Pane(0, PaneHit::Placeholder(PlaceholderButton::CloseTab))));
    a.on_left_down(x, y, false);
    assert_eq!(a.panes[0].tabs.len(), tabs - 1);
}

#[test]
fn retry_reloads() {
    let root = remote_dir("n5-retry");
    FakeNetwork::hang(&root.to_string_lossy(), Duration::from_millis(200));
    let mut a = net_app();
    a.new_tab(0, Location::Path(root.clone()), true);
    a.panes[0].tab_mut().set_load_started(Some(Instant::now() - Duration::from_secs(21)));
    a.paint();
    let loads = FakeFileSystem::loads(&root);
    let (r, _) = *a.panes[0].tab().list.placeholder_buttons.iter().find(|(_, b)| *b == PlaceholderButton::Retry).unwrap();
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
    assert_eq!(FakeFileSystem::loads(&root), loads + 1);
    a.paint();
    assert!(a.panes[0].tab().list.placeholder.is_none(), "recién empezada la recarga no es lenta");
}

#[test]
fn late_result_discarded_after_close() {
    let root = remote_dir("n5-late");
    FakeNetwork::hang(&root.to_string_lossy(), Duration::from_millis(150));
    let mut a = net_app();
    a.new_tab(0, Location::Path(root.clone()), true);
    let n = a.panes[0].tabs.len();
    a.close_tab(0, a.panes[0].active);
    assert_eq!(a.panes[0].tabs.len(), n - 1);
    std::thread::sleep(Duration::from_millis(300));
    while let Ok(ev) = a.rx.try_recv() {
        a.on_load_event(ev);
    }
    assert_eq!(a.panes[0].tabs.len(), n - 1);
}

#[test]
fn local_paths_still_probe() {
    let local = tmp_dir("n5-local-file");
    std::fs::write(local.join("nota.txt"), "x").unwrap();
    let mut a = net_app();
    a.go_to_text(0, &local.join("nota.txt").to_string_lossy());
    assert_eq!(a.panes[0].tab().location, Location::Path(local.clone()));
    pump_pane(&mut a, 0);
    let list = &a.panes[0].tab().list;
    let focused = list.focused_index().map(|i| list.entries[i].name.clone());
    assert_eq!(focused.as_deref(), Some(std::ffi::OsStr::new("nota.txt")));
}

#[cfg(feature = "arbol")]
#[test]
fn drive_label_of_remote_off_thread() {
    let z = tmp_dir("n5-drive-z");
    FakeNetwork::set_remote(&z);
    FakeFileSystem::set_drives(&[(&z, "compartido (Z:)")]);
    FakeNetwork::mark_ui_thread();
    let mut a = net_app();
    a.drives_changed();
    assert!(FakeNetwork::ui_io_calls().is_empty(), "{:?}", FakeNetwork::ui_io_calls());
    // El nombre llega desde otro hilo (el falso lo da por hilo: ahí no lo conoce).
    wait_task(&mut a, |t| matches!(t, TaskDone::DriveLabels(_)));
}

// ---------- N6: desconectar (⏏) ----------

use crate::sys::{VolumeInfo, testing::FakeCopyPrims};
use crate::transfer::JobSpec;

fn drive_row(root: &Path, label: &str) -> crate::fs::Entry {
    crate::fs::Entry {
        name: root.as_os_str().to_os_string(),
        full: None,
        group: 1,
        pidl: None,
        display: label.to_string(),
        sort_key: label.to_lowercase(),
        is_dir: true,
        size: 0,
        modified: 0,
        created: 0,
        attrs: 0x10,
        type_name: std::sync::Arc::from("Disco"),
        icon: 0,
        overlay: 0,
        overlay_checked: true,
    }
}

/// Una conexión de red falsa en `root` (una carpeta real, para que las pestañas carguen).
fn connected(a: &mut App, host_name: &str, root: &Path) {
    let c = conn(&format!("smb://{host_name}/fotos"), &root.to_string_lossy());
    FakeNetwork::set_connections(host_name, vec![c.clone()]);
    FakeNetwork::set_remote(root);
    a.net.connections = vec![c];
}

fn wait_disconnected(a: &mut App) {
    wait_task(a, |t| matches!(t, TaskDone::Disconnected { .. }));
}

#[test]
fn disconnect_releases_tabs_xtree_preview() {
    let root = tmp_dir("n6-rel").join("fotos");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let mut a = net_app();
    connected(&mut a, "nas-n6rel", &root);
    a.set_layout(crate::ui::Layout::TwoSide);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(root.join("sub")));
    a.panes[1].tab_mut().navigate(hwnd, &tx, Location::Path(root.clone()));
    a.set_active(0);
    a.run_command(Command::Eject);
    // Primero se suelta todo (las pestañas pasan a «Este equipo»), después se desconecta.
    assert_eq!(a.panes[0].tab().location, Location::Computer);
    assert_eq!(a.panes[1].tab().location, Location::Computer);
    wait_disconnected(&mut a);
    assert_eq!(FakeNetwork::disconnects(&root), vec![(root.clone(), false)]);
    assert!(a.xtree.as_ref().is_none_or(|x| !x.root().starts_with(&root)));
}

#[test]
fn disconnect_with_copy_asks() {
    let root = PathBuf::from("/net-n6copy/fotos");
    let f = FakeCopyPrims::shared();
    f.set_volume(&root, VolumeInfo { id: "n6copy".into(), label: "fotos".into(), case_insensitive: false, has_trash: false, max_file: None, free: u64::MAX });
    f.add_file(&root.join("grande.bin"), vec![1; 64 * 1024 * 20]);
    f.add_dir(&root.join("d"));
    f.set_slow(&root.join("grande.bin"), Some(Duration::from_millis(20)));
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    connected(&mut a, "nas-n6copy", &root);
    a.start_transfer(JobSpec::Copy { sources: vec![root.join("grande.bin")], dest: root.join("d") });
    host.set_answer(Some(false));
    a.eject(root.clone());
    assert_eq!(host.questions().last().unwrap(), &("Desconectar".to_string(), "Hay 1 operación sobre \"fotos\". ¿Cancelarla y desconectar?".to_string()));
    assert!(FakeNetwork::disconnects(&root).is_empty());
    assert_eq!(a.running_jobs(), 1);
    assert_eq!(super::disconnect_question(2, "fotos"), "Hay 2 operaciones sobre \"fotos\". ¿Cancelarlas y desconectar?");
    // Sí: cancela, espera la limpieza y desconecta.
    host.set_answer(Some(true));
    a.eject(root.clone());
    for _ in 0..500 {
        a.on_transfer_updates();
        if a.running_jobs() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_disconnected(&mut a);
    assert_eq!(FakeNetwork::disconnects(&root), vec![(root.clone(), false)]);
}

#[test]
fn disconnect_success_notice() {
    let root = tmp_dir("n6-ok").join("fotos");
    std::fs::create_dir_all(&root).unwrap();
    let mut a = net_app();
    connected(&mut a, "nas-n6ok", &root);
    a.eject(root.clone());
    wait_disconnected(&mut a);
    assert_eq!(a.notice.as_deref(), Some("Se desconectó \"fotos\""));
    assert!(a.net.connections.iter().all(|c| c.local != root));
}

#[test]
fn disconnect_busy_retry_then_force() {
    let root = PathBuf::from("/net-n6busy/fotos");
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    connected(&mut a, "nas-n6busy", &root);
    FakeNetwork::set_can_force(true);
    FakeNetwork::set_disconnect(&root, vec![Err(NetError::OpenFiles { by: Some("Word".into()) })]);
    host.set_answer(Some(true));
    a.eject(root.clone());
    wait_disconnected(&mut a);
    assert_eq!(host.questions().last().unwrap().1, "No se pudo desconectar \"fotos\": lo está usando Word. ¿Reintentar?");
    // Sí → segundo intento, otra vez ocupado → «¿Desconectar igual?» → a la fuerza.
    wait_disconnected(&mut a);
    assert_eq!(host.questions().last().unwrap().1, "¿Desconectar igual? Lo que esté abierto en \"fotos\" puede perder cambios.");
    FakeNetwork::set_disconnect(&root, vec![Ok(())]);
    wait_disconnected(&mut a);
    assert_eq!(FakeNetwork::disconnects(&root), vec![(root.clone(), false), (root.clone(), false), (root.clone(), true)]);
    assert_eq!(a.notice.as_deref(), Some("Se desconectó \"fotos\""));
}

#[test]
fn busy_without_force_only_retries() {
    let root = PathBuf::from("/net-n6noforce/fotos");
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    connected(&mut a, "nas-n6nf", &root);
    FakeNetwork::set_can_force(false);
    FakeNetwork::set_disconnect(&root, vec![Err(NetError::OpenFiles { by: None }), Err(NetError::OpenFiles { by: None })]);
    host.set_answer(Some(true));
    a.eject(root.clone());
    wait_disconnected(&mut a);
    wait_disconnected(&mut a);
    host.set_answer(Some(false));
    let q = host.questions();
    assert!(q.iter().all(|(_, t)| !t.contains("igual")), "{q:?}");
    assert!(FakeNetwork::disconnects(&root).iter().all(|(_, f)| !f));
}

#[test]
fn eject_command_on_remote_folder_disconnects() {
    let root = tmp_dir("n6-cmd").join("fotos");
    std::fs::create_dir_all(root.join("a")).unwrap();
    let mut a = net_app();
    connected(&mut a, "nas-n6cmd", &root);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(root.join("a")));
    assert_eq!(a.eject_target(), Some(root.clone()));
    a.run_command(Command::Eject);
    wait_disconnected(&mut a);
    assert_eq!(FakeNetwork::disconnects(&root).len(), 1);
}

#[test]
fn computer_list_shows_eject_for_connections() {
    let root = PathBuf::from("/net-n6list/fotos");
    let other = PathBuf::from("/net-n6list-local");
    let mut a = net_app();
    connected(&mut a, "nas-n6list", &root);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Computer);
    wait_until(&mut a, |a| !a.panes[0].tab().loading);
    a.panes[0].tab_mut().list.replace(vec![drive_row(&other, "Disco"), drive_row(&root, "fotos")]);
    a.paint();
    let list = &a.panes[0].tab().list;
    let rows: Vec<String> = list.eject_rows.iter().map(|&i| list.entries[i].display.clone()).collect();
    assert_eq!(rows, ["fotos"]);
    // Clic derecho en la conexión: «Expulsar» (desconecta).
    a.panes[0].tab_mut().list.select_index(1);
    assert_eq!(a.eject_target(), Some(root));
}

// ---------- N7: «Red» en el árbol ----------

#[cfg(feature = "arbol")]
mod tree_net {
    use super::*;
    use crate::sys::testing::FAKE_NETWORK_PLACE;
    use crate::tree::{NetRow, TreeHit};

    fn tree_app(host: Rc<FakeHost>) -> App {
        let mut a = net_app_with(host);
        a.tree.visible = true;
        a.paint();
        a
    }

    fn with_network(a: &mut App, tag: &str, root: &Path) {
        a.net.places.toggle_favorite(&Address::parse(&format!("smb://fav1-{tag}/x")).unwrap(), Some(format!("Fav uno {tag}")));
        a.net.places.toggle_favorite(&Address::parse(&format!("smb://fav2-{tag}/y")).unwrap(), Some(format!("Fav dos {tag}")));
        a.net.connections = vec![conn(&format!("smb://nas-{tag}/fotos"), &root.to_string_lossy())];
        a.net.hosts = vec![format!("MAC-{tag}"), format!("PC-{tag}")];
        a.net_changed();
        a.tree.expand_network(a.hwnd, true);
        a.paint();
    }

    #[test]
    fn tree_network_root_on_all_systems() {
        let mut a = tree_app(FakeHost::new());
        let rows = a.tree.network_rows();
        assert_eq!(rows[0], (NetRow::Root, "Red".to_string()));
        // Sin Red del Shell: un clic solo la despliega (no abre nada en el panel).
        let (x, y) = a.tree.row_point("Red").unwrap();
        assert!(matches!(a.tree.hit(x, y), TreeHit::Toggle(_)));
        let before = a.panes[a.active].tab().location.clone();
        a.on_tree_click(x, y);
        assert_eq!(a.panes[a.active].tab().location, before);
        assert!(a.tree.network_expanded());
    }

    #[test]
    fn tree_network_groups() {
        let root = tmp_dir("n7-groups").join("fotos");
        std::fs::create_dir_all(&root).unwrap();
        let mut a = tree_app(FakeHost::new());
        with_network(&mut a, "n7g", &root);
        let rows: Vec<String> = a.tree.network_rows().into_iter().map(|(_, n)| n).collect();
        assert_eq!(rows, ["Red", "Fav uno n7g", "Fav dos n7g", "fotos (nas-n7g)", "—", "MAC-n7g", "PC-n7g"]);
        // ⏏ solo al pasar el mouse por la conexión (o si es la carpeta del panel).
        assert!(a.tree.eject_button(&root).is_none());
        let (x, y) = a.tree.row_point("fotos (nas-n7g)").unwrap();
        a.tree.on_mouse_move(x, y, false);
        a.paint();
        assert!(a.tree.eject_button(&root).is_some());
    }

    #[test]
    fn host_up_down_updates_tree() {
        let mut a = tree_app(FakeHost::new());
        a.tree.expand_network(a.hwnd, true);
        let (tx, rx) = std::sync::mpsc::channel();
        a.net.host_rx = Some(rx);
        tx.send(crate::sys::HostEvent::Up("IMPRESORA-N7".into())).unwrap();
        tx.send(crate::sys::HostEvent::Up("IMPRESORA-N7".into())).unwrap();
        a.on_net_hosts();
        assert_eq!(a.tree.network_rows().iter().filter(|(_, n)| n == "IMPRESORA-N7").count(), 1);
        tx.send(crate::sys::HostEvent::Down("IMPRESORA-N7".into())).unwrap();
        a.on_net_hosts();
        assert!(a.tree.network_rows().iter().all(|(_, n)| n != "IMPRESORA-N7" && n != "—"));
    }

    #[test]
    fn favorite_click_connects_or_opens() {
        let target = tmp_dir("n7-fav");
        FakeNetwork::set_connect("fav1-n7f", Ok(target.clone()));
        let mut a = tree_app(FakeHost::new());
        with_network(&mut a, "n7f", &tmp_dir("n7-fav-conn"));
        // Sin conexión: conecta (sin panel) y abre en el panel activo.
        let (x, y) = a.tree.row_point("Fav uno n7f").unwrap();
        assert_eq!(a.tree.hit(x, y), TreeHit::ConnectFavorite("smb://fav1-n7f/x".into()));
        a.on_tree_click(x, y);
        assert!(a.net.dialog.is_none());
        wait_until(&mut a, |a| a.panes[a.active].tab().location == Location::Path(target.clone()));
        assert_eq!(FakeNetwork::connects_to("fav1-n7f").len(), 1);
        // Con conexión: navega sin conectar.
        let conn_root = tmp_dir("n7-fav-yes");
        std::fs::create_dir_all(conn_root.join("x")).unwrap();
        a.net.connections = vec![conn("smb://fav2-n7f/y", &conn_root.to_string_lossy())];
        a.net_changed();
        a.open_net_address("smb://fav2-n7f/y");
        assert_eq!(a.panes[a.active].tab().location, Location::Path(conn_root));
        assert!(FakeNetwork::connects_to("fav2-n7f").is_empty());
        // Si falla, el panel de conectar con el error.
        FakeNetwork::set_connect("fav-bad-n7f", Err(NetError::NotFound { host: "fav-bad-n7f".into() }));
        a.open_net_address("smb://fav-bad-n7f/z");
        wait_until(&mut a, |a| a.net.dialog.as_ref().is_some_and(|d| d.error.is_some()));
        assert_eq!(a.connect_field_text(), "smb://fav-bad-n7f/z");
    }

    #[test]
    fn host_click_connects_smb() {
        FakeNetwork::set_connect("MAC-n7h", Ok(tmp_dir("n7-host")));
        let mut a = tree_app(FakeHost::new());
        with_network(&mut a, "n7h", &tmp_dir("n7-host-conn"));
        let (x, y) = a.tree.row_point("MAC-n7h").unwrap();
        assert_eq!(a.tree.hit(x, y), TreeHit::ConnectHost("MAC-n7h".into()));
        a.on_tree_click(x, y);
        wait_until(&mut a, |_| FakeNetwork::connects_to("MAC-n7h").len() == 1);
        assert_eq!(FakeNetwork::connects_to("MAC-n7h")[0].0, "smb://MAC-n7h");
    }

    #[test]
    fn connected_eject_runs_disconnect_flow() {
        let root = tmp_dir("n7-eject").join("fotos");
        std::fs::create_dir_all(&root).unwrap();
        let mut a = tree_app(FakeHost::new());
        with_network(&mut a, "n7e", &root);
        FakeNetwork::set_remote(&root);
        let (x, y) = a.tree.row_point("fotos (nas-n7e)").unwrap();
        a.tree.on_mouse_move(x, y, false);
        a.paint();
        let r = a.tree.eject_button(&root).unwrap();
        a.on_tree_click((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        wait_task(&mut a, |t| matches!(t, TaskDone::Disconnected { .. }));
        assert_eq!(FakeNetwork::disconnects(&root), vec![(root.clone(), false)]);
    }

    #[test]
    fn tree_network_menu_entries() {
        let host = FakeHost::new();
        let mut a = tree_app(host.clone());
        host.set_menu_choice(None);
        a.net_tree_menu(NetRow::Connected(PathBuf::from("/net-n7menu/fotos"), "smb://nas-n7menu/fotos".into()));
        let labels: Vec<String> = host
            .menus()
            .last()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                crate::shell::Entry::Item { label, .. } => Some(label.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["Conectar a un servidor…", "Desconectar", "Agregar a favoritos", "Copiar la dirección"]);
        // «Copiar la dirección».
        host.set_menu_choice(Some(4));
        FakeShell::take_calls();
        a.net_tree_menu(NetRow::Favorite("smb://nas-n7menu/fotos".into()));
        assert!(FakeShell::take_calls().contains(&ShellCall::CopyText("smb://nas-n7menu/fotos".into())));
        // «Agregar a favoritos» → después dice «Quitar de favoritos».
        host.set_menu_choice(Some(3));
        a.net_tree_menu(NetRow::Host("PC-N7MENU".into()));
        assert!(a.net.places.is_favorite(&Address::parse("smb://PC-N7MENU").unwrap()));
    }

    #[test]
    fn windows_shell_children_kept() {
        FakeFileSystem::set_network_place(true);
        FakeFileSystem::set_shell_subfolders(FAKE_NETWORK_PLACE, vec![("SRV01".into(), PathBuf::from(r"\\SRV01"))]);
        let mut a = tree_app(FakeHost::new());
        with_network(&mut a, "n7w", &tmp_dir("n7-win"));
        for _ in 0..500 {
            a.tree.on_loaded();
            if a.tree.network_rows().iter().any(|(_, n)| n == "SRV01") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(a.tree.network_rows().iter().any(|(_, n)| n == "SRV01"), "{:?} {}", a.tree.network_rows(), FakeFileSystem::shell_subfolder_calls(FAKE_NETWORK_PLACE));
        assert_eq!(FakeFileSystem::shell_subfolder_calls(FAKE_NETWORK_PLACE), 1);
        // Un clic en «Red» abre la Red del Shell, como hoy.
        a.paint();
        let (x, y) = a.tree.row_point("Red").unwrap();
        assert!(matches!(a.tree.hit(x + 60.0, y), TreeHit::Open(Location::Shell { .. })));
        FakeFileSystem::set_network_place(false);
    }

    #[test]
    fn browse_cancelled_when_tree_hidden() {
        let mut a = tree_app(FakeHost::new());
        let cancel = a.net.browse_cancel.clone().expect("con el árbol a la vista se buscan máquinas");
        a.tree.visible = false;
        a.paint();
        assert!(a.net.browse_cancel.is_none());
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    }
}

// ---------- N8: sección «Red» de la configuración ----------

mod red_page {
    use super::*;
    use crate::app::network::unix_now;
    use crate::app::network_page::{self, LOCAL_NETWORK_HINT, PageHit, password_text};
    use crate::geom::rect;
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};

    fn painter() -> RecordingPainter {
        RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(false)))
    }

    fn draw(a: &mut App) -> Vec<String> {
        let p = painter();
        let h = network_page::draw(a, &p, rect(0.0, 0.0, 900.0, 2000.0), None);
        assert!(h > 100.0);
        p.texts().into_iter().map(|t| t.1).collect()
    }

    fn click(a: &mut App, hit: PageHit) {
        let (r, _) = *a.net.page.hits.iter().find(|(_, h)| *h == hit).unwrap_or_else(|| panic!("{hit:?}: {:?}", a.net.page.hits));
        assert!(network_page::click(a, (r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0));
    }

    fn page_app(host: Rc<FakeHost>) -> App {
        let mut a = net_app_with(host);
        a.net.places.toggle_favorite(&Address::parse("smb://nas-n8/fotos").unwrap(), Some("NAS — fotos".into()));
        a.net.places.toggle_favorite(&Address::parse("smb://srv-n8/comp").unwrap(), Some("Oficina".into()));
        a.net.places.add_recent(&Address::parse("smb://srv-n8/respaldo").unwrap(), unix_now() - 2 * 86_400);
        a
    }

    #[test]
    fn red_page_lists_and_edits_favorites() {
        let host = FakeHost::new();
        let mut a = page_app(host.clone());
        let texts = draw(&mut a);
        for t in ["Red", "Favoritos", "NAS — fotos", "smb://nas-n8/fotos", "Oficina", "Recientes", "smb://srv-n8/respaldo", "hace 2 días", "Conectados ahora"] {
            assert!(texts.iter().any(|x| x == t), "{t}: {texts:?}");
        }
        // Renombrar en línea: una caja, la del nombre.
        click(&mut a, PageHit::EditFavorite(1));
        let field = |a: &App| a.net.page.editing.as_ref().unwrap().field;
        assert_eq!(host.text_boxes_of(crate::host::TextKind::Field).len(), 1);
        crate::host::WindowHost::text_set(&*host, field(&a), "Oficina centro");
        a.on_field_done(true);
        assert_eq!(a.net.places.favorites[1].name, "Oficina centro");
        assert!(a.net.page.editing.is_none());
        assert!(host.text_boxes_of(crate::host::TextKind::Field).is_empty());
        // Tab pasa a la dirección; una inválida deja el error en la fila y sigue editando.
        draw(&mut a);
        click(&mut a, PageHit::EditFavorite(0));
        a.on_field_tab(false);
        assert!(a.net.page.editing.as_ref().unwrap().on_address);
        assert_eq!(host.text_boxes_of(crate::host::TextKind::Field).len(), 1);
        crate::host::WindowHost::text_set(&*host, field(&a), "smb://na s");
        a.on_field_done(true);
        assert_eq!(a.net.page.error, Some((0, "Escribí una dirección como smb://servidor/recurso".to_string())));
        assert!(draw(&mut a).iter().any(|t| t == "Escribí una dirección como smb://servidor/recurso"));
        a.on_field_done(false);
        assert!(a.net.page.editing.is_none());
        // ↑ cambia el orden de `places` y del árbol.
        draw(&mut a);
        click(&mut a, PageHit::FavoriteUp(1));
        assert_eq!(a.net.places.favorites[0].name, "Oficina centro");
        #[cfg(feature = "arbol")]
        assert_eq!(a.tree.network_rows()[1].1, "Oficina centro");
        // Agregar…: una fila nueva con la caja de la dirección; Enter la suma.
        draw(&mut a);
        click(&mut a, PageHit::AddFavorite);
        let e = a.net.page.editing.as_ref().unwrap();
        assert_eq!((e.index, e.on_address), (2, true));
        crate::host::WindowHost::text_set(&*host, field(&a), r"\\nuevo-n8\x");
        a.on_field_done(true);
        assert_eq!(a.net.places.favorites[2].address, "smb://nuevo-n8/x");
        assert_eq!(a.net.places.favorites[2].name, "smb://nuevo-n8/x");
        // ✕ quita.
        draw(&mut a);
        click(&mut a, PageHit::RemoveFavorite(2));
        assert_eq!(a.net.places.favorites.len(), 2);
    }

    #[test]
    fn red_page_recents_star_and_clear() {
        let mut a = page_app(FakeHost::new());
        draw(&mut a);
        click(&mut a, PageHit::StarRecent(0));
        assert!(a.net.places.is_favorite(&Address::parse("smb://srv-n8/respaldo").unwrap()));
        draw(&mut a);
        click(&mut a, PageHit::RemoveRecent(0));
        assert!(a.net.places.recents.is_empty());
        a.net.places.add_recent(&Address::parse("smb://x-n8/y").unwrap(), 1);
        draw(&mut a);
        click(&mut a, PageHit::ClearRecents);
        assert!(a.net.places.recents.is_empty());
        let red = crate::session::config_dir().unwrap().join("red.toml");
        wait_until(&mut a, |_| std::fs::read_to_string(&red).is_ok_and(|t| !t.contains("x-n8") && t.contains("srv-n8/respaldo")));
    }

    #[test]
    fn red_page_connected_eject() {
        let mut a = page_app(FakeHost::new());
        let root = PathBuf::from("/net-n8ej/fotos");
        a.net.connections = vec![conn("smb://nas-n8ej/fotos", "/net-n8ej/fotos")];
        let texts = draw(&mut a);
        assert!(texts.iter().any(|t| t == "smb://nas-n8ej/fotos"));
        click(&mut a, PageHit::Eject(0));
        wait_task(&mut a, |t| matches!(t, TaskDone::Disconnected { .. }));
        assert_eq!(FakeNetwork::disconnects(&root), vec![(root.clone(), false)]);
    }

    #[test]
    fn red_page_password_text() {
        assert_eq!(password_text(KeyStyle::Mac, true), "xdir no guarda contraseñas: las guarda el sistema (Llavero)");
        assert_eq!(password_text(KeyStyle::Windows, true), "xdir no guarda contraseñas: las guarda el sistema (Administrador de credenciales)");
        assert_eq!(password_text(KeyStyle::Windows, false), "xdir no guarda contraseñas: las guarda el sistema (llavero del escritorio)");
        crate::sys::testing::FakeEnv::set_key_style(KeyStyle::Mac);
        let mut a = page_app(FakeHost::new());
        let texts = draw(&mut a);
        crate::sys::testing::FakeEnv::set_key_style(KeyStyle::Windows);
        assert!(texts.iter().any(|t| t == "xdir no guarda contraseñas: las guarda el sistema (Llavero)"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Abrir"));
    }

    #[test]
    fn red_page_local_network_hint() {
        let mut a = page_app(FakeHost::new());
        assert!(!draw(&mut a).iter().any(|t| t == LOCAL_NETWORK_HINT));
        FakeNetwork::set_local_network_denied(true);
        let texts = draw(&mut a);
        FakeNetwork::set_local_network_denied(false);
        assert!(texts.iter().any(|t| t == LOCAL_NETWORK_HINT));
        assert!(a.net.page.hits.iter().any(|(_, h)| *h == PageHit::OpenLocalNetwork));
    }

    #[test]
    fn red_toml_reloaded_when_changed() {
        let mut a = net_app();
        a.attach(Handle(0), 96);
        wait_until(&mut a, |a| a.net.places_mtime.is_some() || std::fs::metadata(crate::session::config_dir().unwrap().join("red.toml")).is_err());
        let path = crate::session::config_dir().unwrap().join("red.toml");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[[favorito]]\nnombre = \"A mano\"\ndireccion = \"smb://mano-n8/x\"\n").unwrap();
        a.reload_config();
        wait_until(&mut a, |a| a.net.places.favorites.iter().any(|f| f.name == "A mano"));
    }

    #[test]
    fn red_page_search_items() {
        let a = net_app();
        let items = network_page::search(&a);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Favoritos", "Recientes", "Conectados ahora", "Contraseñas"]);
        assert!(items[3].keywords.iter().any(|k| k == "contraseña") && items[3].keywords.iter().any(|k| k == "llavero"));
        assert!(items.iter().all(|i| i.keywords.iter().any(|k| k == "servidor")));
    }
}

// ---------- N10: unidades de red (Windows) ----------

mod drives {
    use super::*;
    use crate::net::drives_dialog::{DrivesHit, MAP_TITLE, NO_DRIVES};

    fn map_app(host: Rc<FakeHost>) -> App {
        FakeNetwork::set_free_letters(vec!['Z', 'Y']);
        let mut a = net_app_with(host);
        a.run_command(Command::MapNetworkDrive);
        a
    }

    fn type_folder(a: &mut App, text: &str) {
        let b = a.net.map.as_ref().and_then(|m| m.folder).unwrap();
        crate::host::WindowHost::text_set(&*a.host, b, text);
        a.on_text_changed(b.0 as isize);
    }

    #[test]
    fn map_dialog_defaults() {
        let host = FakeHost::new();
        let mut a = map_app(host.clone());
        let m = a.net.map.as_ref().unwrap();
        assert_eq!((m.letter, m.reconnect, m.other_credentials), ('Z', true, false));
        assert!(crate::host::WindowHost::text_has_focus(&*host, m.folder.unwrap()));
        a.paint();
        assert!(host.drawn_texts().iter().any(|t| t.1 == MAP_TITLE));
        // La letra se elige con un menú.
        host.set_menu_choice(Some(1));
        a.map_click(Some(DrivesHit::Letter));
        assert_eq!(a.net.map.as_ref().unwrap().letter, 'Y');
        let labels: Vec<String> = host.menus().last().unwrap().iter().filter_map(|e| match e {
            crate::shell::Entry::Item { label, .. } => Some(label.clone()),
            _ => None,
        }).collect();
        assert_eq!(labels, ["Z:", "Y:"]);
    }

    #[test]
    fn map_finish_connects_with_letter() {
        let z = tmp_dir("n10-z");
        FakeNetwork::set_connect("srv-n10", Ok(z.clone()));
        let mut a = map_app(FakeHost::new());
        type_folder(&mut a, r"\\srv-n10\compartido");
        a.map_click(Some(DrivesHit::OtherCredentials));
        let tabs = a.panes[a.active].tabs.len();
        a.map_click(Some(DrivesHit::Finish));
        wait_until(&mut a, |a| a.net.map.is_none());
        let (url, opts) = FakeNetwork::connects_to("srv-n10").pop().unwrap();
        assert_eq!(url, "smb://srv-n10/compartido");
        assert_eq!((opts.drive_letter, opts.reconnect, opts.other_credentials), (Some('Z'), true, true));
        assert_eq!(a.panes[a.active].tabs.len(), tabs + 1);
        assert_eq!(a.panes[a.active].tab().location, Location::Path(z));
        assert_eq!(a.net.places.recents[0].address, "smb://srv-n10/compartido");
    }

    #[test]
    fn map_error_stays() {
        FakeNetwork::set_connect("srv-n10e", Err(NetError::Other("La letra ya está en uso".into())));
        let mut a = map_app(FakeHost::new());
        type_folder(&mut a, r"\\srv-n10e\x");
        a.on_field_done(true);
        wait_until(&mut a, |a| a.net.map.as_ref().is_some_and(|m| m.busy.is_none() && m.error.is_some()));
        assert_eq!(a.net.map.as_ref().unwrap().error.as_deref(), Some("La letra ya está en uso"));
        // Sin recurso: lo dice sin conectar.
        type_folder(&mut a, r"\\srv-n10e");
        a.map_click(Some(DrivesHit::Finish));
        assert!(a.net.map.as_ref().unwrap().error.as_deref().unwrap().contains("servidor"));
        // Esc cierra.
        a.on_field_done(false);
        assert!(a.net.map.is_none());
    }

    #[test]
    fn map_autocomplete_from_places() {
        let mut a = map_app(FakeHost::new());
        a.net.places.toggle_favorite(&Address::parse(r"\\srv-n10a\compartido").unwrap(), None);
        type_folder(&mut a, r"\\srv-n10");
        let s = a.net.map.as_ref().unwrap().suggestion.clone();
        assert!(s.is_some(), "sugiere algo de favoritos o recientes");
        a.map_click(Some(DrivesHit::Suggestion));
        let b = a.net.map.as_ref().and_then(|m| m.folder).unwrap();
        assert_eq!(crate::host::WindowHost::text_get(&*a.host, b), s.unwrap());
    }

    #[test]
    fn disconnect_dialog_lists_and_runs_flow() {
        let host = FakeHost::new();
        let mut a = net_app_with(host.clone());
        let mut z = conn("smb://srv-n10d/compartido", "/net-n10d/Z");
        z.letter = Some('Z');
        let mut x = conn("smb://srv-n10d/respaldo", "/net-n10d/X");
        (x.letter, x.connected) = (Some('X'), false);
        a.net.connections = vec![z, x, conn("smb://sin-letra-n10d/y", "/net-n10d/sin")];
        a.run_command(Command::DisconnectNetworkDrive);
        a.paint();
        let texts: Vec<String> = host.drawn_texts().into_iter().map(|t| t.1).collect();
        assert!(texts.iter().any(|t| t == "desconectada") && texts.iter().any(|t| t == "Z:"));
        assert_eq!(a.net.unmap.as_ref().unwrap().rows.len(), 2);
        a.unmap_click(Some(DrivesHit::Row(0)));
        a.unmap_click(Some(DrivesHit::Finish));
        assert!(a.net.unmap.is_none());
        wait_task(&mut a, |t| matches!(t, TaskDone::Disconnected { .. }));
        assert_eq!(FakeNetwork::disconnects(Path::new("/net-n10d")), vec![(PathBuf::from("/net-n10d/Z"), false)]);
    }

    #[test]
    fn disconnect_dialog_empty_text() {
        let host = FakeHost::new();
        let mut a = net_app_with(host.clone());
        a.net.connections.clear();
        a.run_command(Command::DisconnectNetworkDrive);
        a.paint();
        assert!(host.drawn_texts().iter().any(|t| t.1 == NO_DRIVES));
    }

    #[test]
    fn drive_commands_in_computer_menu() {
        // Ya cubierto en `computer_background_menu_has_connect` (los tres textos); acá, que
        // elegirlos abre los paneles.
        let mut a = net_app();
        a.run_command(Command::MapNetworkDrive);
        assert!(a.net.map.is_some());
        a.close_map_dialog();
        a.run_command(Command::DisconnectNetworkDrive);
        assert!(a.net.unmap.is_some());
    }
}

// ---------- Ronda 1: más llamadas del hilo de interfaz ----------

/// C1: el desplegable de la barra no lista una carpeta de red (solo el historial).
#[test]
fn address_suggestions_skip_remote_folders() {
    install_fakes();
    let remote = tmp_dir("r1-sug-remote");
    std::fs::create_dir_all(remote.join("subcarpeta")).unwrap();
    let local = tmp_dir("r1-sug-local");
    std::fs::create_dir_all(local.join("subcarpeta")).unwrap();
    FakeNetwork::set_remote(&remote);
    let typed = |dir: &Path| format!("{}{}sub", dir.display(), std::path::MAIN_SEPARATOR);
    let mut s = crate::typed::Suggest::new(crate::geom::Rect::default(), "");
    s.set_query(&typed(&local));
    assert!(s.items.iter().any(|i| i.ends_with("subcarpeta")), "{:?}", s.items);
    s.set_query(&typed(&remote));
    assert!(s.items.iter().all(|i| !i.ends_with("subcarpeta")), "{:?}", s.items);
    // Una ruta UNC tampoco (resolver el nombre ya es ir a la red).
    s.set_query(r"\\servidor-r1\recurso\");
    assert!(s.items.is_empty() || s.items.iter().all(|i| !i.contains("servidor-r1\\recurso\\")));
}

/// I1: la flecha de las migas de una carpeta de red lista en otro hilo y abre el menú al volver.
#[test]
fn chevron_of_remote_folder_lists_off_thread() {
    let root = remote_dir("r1-chevron");
    FakeNetwork::mark_ui_thread();
    let host = FakeHost::new();
    let mut a = net_app_with(host.clone());
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(root.join("sub")));
    a.paint();
    let k = (0..10).find(|&k| a.panes[0].chevron(k).is_some_and(|(_, l)| l == Location::Path(root.clone()))).expect("flecha de la carpeta");
    host.set_menu_choice(None);
    let menus = host.menus().len();
    a.show_subfolder_menu(0, k);
    assert_eq!(host.menus().len(), menus, "el menú espera a la lista");
    wait_task(&mut a, |t| matches!(t, TaskDone::Subfolders { .. }));
    let labels: Vec<String> = host.menus().last().unwrap().iter().filter_map(|e| match e {
        crate::shell::Entry::Item { label, .. } => Some(label.clone()),
        _ => None,
    }).collect();
    assert_eq!(labels, ["sub"]);
    assert!(FakeNetwork::ui_io_calls().is_empty(), "{:?}", FakeNetwork::ui_io_calls());
}

/// I3: arrastrar desde o hacia una carpeta de red no pregunta el volumen al sistema (iría al
/// servidor con cada movimiento): misma conexión mueve, otra copia.
#[test]
fn drag_effect_on_remote_without_io() {
    use crate::drop::Effect;
    install_fakes();
    FakeNetwork::set_remote(Path::new("/net-r1drag"));
    FakeNetwork::mark_ui_thread();
    let roots = vec![PathBuf::from("/net-r1drag/fotos"), PathBuf::from("/net-r1drag/musica")];
    let src = vec![PathBuf::from("/net-r1drag/fotos/a/x.jpg")];
    let all = Effect::ALL;
    assert_eq!(super::fileops::drop_effect(&src, Path::new("/net-r1drag/fotos/b"), false, false, all, &roots), Effect::MOVE);
    assert_eq!(super::fileops::drop_effect(&src, Path::new("/net-r1drag/musica"), false, false, all, &roots), Effect::COPY);
    assert_eq!(super::fileops::drop_effect(&src, Path::new("/local-r1drag/x"), false, false, all, &roots), Effect::COPY);
    assert!(FakeNetwork::ui_io_calls().iter().all(|c| !c.starts_with("volume_of /net-r1drag")), "{:?}", FakeNetwork::ui_io_calls());
}

/// I4: xtree sobre una carpeta de red pide el espacio del volumen en otro hilo.
#[test]
fn xtree_remote_volume_off_thread() {
    let root = remote_dir("r1-xtree");
    FakeNetwork::mark_ui_thread();
    let a = net_app();
    let _x = crate::xtree::view::Xtree::new(a.hwnd, root.clone());
    std::thread::sleep(Duration::from_millis(50));
    assert!(FakeNetwork::ui_io_calls().iter().all(|c| !c.starts_with("volume_space")), "{:?}", FakeNetwork::ui_io_calls());
}

/// I5: soltar el guardián de una carpeta de red colgada no espera en el hilo de la ventana.
#[test]
fn remote_watcher_released_off_thread() {
    let root = remote_dir("r1-watchdrop");
    let mut a = net_app();
    a.sync_visibility();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(root.clone()));
    wait_until(&mut a, |_| FakeFileSystem::watches(&root).0 == 1);
    FakeNetwork::hang(&root.to_string_lossy(), Duration::from_millis(400));
    let t0 = Instant::now();
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(tmp_dir("r1-watchdrop-local")));
    assert!(t0.elapsed() < Duration::from_millis(200), "navegar esperó al guardián: {:?}", t0.elapsed());
    wait_until(&mut a, |_| FakeFileSystem::watches(&root).1 == 1);
}

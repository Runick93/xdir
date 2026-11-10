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

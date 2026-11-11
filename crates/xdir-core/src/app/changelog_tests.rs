//! Pruebas de la pestaña Novedades: una sola, sesión, rueda y teclado, rueda dentada, Acerca de,
//! paleta y el aviso al actualizar.

use super::*;
use crate::commands;
use crate::keys::keys::{VK_CONTROL, VK_END, VK_HOME, VK_NEXT};
use crate::sys::testing::{FakeEnv, FakeFileSystem, FakeHost, FakeShell};

fn app() -> App {
    crate::sys::testing::install_fakes();
    App::new(Init::blank(), FakeHost::new())
}

fn app_host() -> (App, Rc<FakeHost>) {
    crate::sys::testing::install_fakes();
    let host = FakeHost::new();
    (App::new(Init::blank(), host.clone()), host)
}

/// Una `App` que arranca con una sesión cuya última versión vista es `seen`.
fn app_seen(seen: Option<&str>) -> (App, Rc<FakeHost>) {
    crate::sys::testing::install_fakes();
    let mut init = Init::blank();
    init.session.seen_version = seen.map(str::to_string);
    let host = FakeHost::new();
    (App::new(init, host.clone()), host)
}

fn changelog_count(a: &App) -> usize {
    a.panes.iter().flat_map(|p| p.tabs.iter()).filter(|t| t.is_changelog()).count()
}

fn press(a: &mut App, mods: &[crate::keys::Key], vk: crate::keys::Key) -> bool {
    FakeEnv::hold_keys(mods);
    let used = a.on_key(vk);
    FakeEnv::hold_keys(&[]);
    used
}

/// El centro de lo primero que cumple `f` en una grilla de la ventana.
fn find_point(a: &App, f: impl Fn(f32, f32) -> bool) -> Option<(f32, f32)> {
    let (w, h) = a.client_dips();
    (0..w as i32).step_by(4).flat_map(|x| (0..h as i32).step_by(4).map(move |y| (x as f32, y as f32))).find(|&(x, y)| f(x, y))
}

#[test]
fn changelog_opens_once() {
    let mut a = app();
    let before = a.panes[0].tabs[0].location.clone();
    a.run_command(Command::Changelog);
    a.run_command(Command::Changelog);
    assert_eq!(changelog_count(&a), 1);
    assert_eq!(a.panes[0].tabs.len(), 2, "abre una pestaña nueva, no reemplaza la carpeta");
    assert_eq!(a.panes[0].tabs[0].location, before);
    assert!(a.panes[0].tab().is_changelog() && a.panes[0].tab().is_page());
    // Desde otro panel, la enfoca.
    a.set_active(2);
    a.run_command(Command::Changelog);
    assert_eq!(changelog_count(&a), 1);
    assert_eq!(a.active, 0);
}

#[test]
fn changelog_not_loaded_and_title() {
    let (mut a, host) = app_host();
    a.run_command(Command::Changelog);
    a.sync_visibility();
    a.run_command(Command::Refresh);
    let tab = a.panes[0].tab();
    assert!(tab.is_loaded() && !tab.loading);
    assert_eq!(FakeFileSystem::loads(std::path::Path::new("xdir:novedades")), 0);
    assert_eq!(tab.status(), format!("Novedades   ·   versión instalada: {}", about::VERSION));
    assert_eq!(tab.location.title(), "Novedades");
    assert!(!tab.can_back() && !tab.can_forward() && !tab.can_up());
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(texts.iter().filter(|t| *t == "Novedades").count() >= 3, "pestaña, migas y título: {texts:?}");
    // La primera versión del CHANGELOG (o «En desarrollo» si hay algo sin publicar).
    let first = crate::changelog_draw::bundled().iter().find_map(|b| match b {
        crate::changelog::Block::Version { version, .. } if version.is_empty() => Some("En desarrollo".to_string()),
        crate::changelog::Block::Version { version, .. } => Some(version.clone()),
        _ => None,
    });
    assert!(texts.iter().any(|t| Some(t) == first.as_ref()), "{first:?}: {texts:?}");
    // Ir a otro lado desde Novedades la reemplaza y no queda en Atrás.
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.tab_mut().navigate(hwnd, &tx, Location::Computer);
    assert!(a.panes[0].tab().changelog.is_none() && !a.panes[0].tab().can_back());
}

#[test]
fn changelog_session_roundtrip_keeps_one() {
    let mut a = app();
    a.run_command(Command::Changelog);
    let s = a.session();
    assert!(s.panes[0].tabs.contains(&Location::Changelog));
    let mut init = Init::blank();
    init.session = s;
    let b = App::new(init, FakeHost::new());
    assert_eq!(changelog_count(&b), 1);
    let mut init = Init::blank();
    init.session.panes[1].tabs = vec![Location::Changelog, Location::Changelog];
    let c = App::new(init, FakeHost::new());
    assert_eq!(changelog_count(&c), 1, "una sola aunque la sesión traiga dos");
}

#[test]
fn wheel_and_keys_scroll() {
    let mut a = app();
    a.layout = Layout::One;
    a.run_command(Command::Changelog);
    a.paint();
    let r = a.panes[0].list;
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert_eq!(a.hit(x, y), Hit::Changelog(0));
    let scroll = |a: &App| a.panes[0].tab().changelog.as_ref().unwrap().scroll;
    assert_eq!(scroll(&a), 0.0);
    a.on_wheel(x, y, -1.0, 3.0);
    let after_wheel = scroll(&a);
    assert!(after_wheel > 0.0);
    assert!(press(&mut a, &[], VK_END));
    let max = scroll(&a);
    assert!(max > after_wheel);
    assert!(press(&mut a, &[], VK_HOME));
    assert_eq!(scroll(&a), 0.0);
    assert!(press(&mut a, &[], VK_DOWN));
    assert_eq!(scroll(&a), crate::changelog_draw::STEP);
    assert!(press(&mut a, &[], VK_NEXT));
    assert!(scroll(&a) > crate::changelog_draw::STEP);
    assert!(press(&mut a, &[], VK_UP));
}

#[test]
fn folder_commands_ignored_in_changelog() {
    let mut a = app();
    a.run_command(Command::Changelog);
    FakeShell::take_calls();
    press(&mut a, &[VK_CONTROL], Key(b'C' as u16));
    a.run_command(Command::NewFolder);
    a.run_command(Command::Filter);
    assert!(FakeShell::take_calls().is_empty());
    assert!(a.panes[0].tab().is_changelog());
    assert!(a.filter_edit.is_none(), "sin buscador");
    // Pestaña nueva desde Novedades: «Este equipo».
    a.run_command(Command::NewTab);
    assert_eq!(changelog_count(&a), 1);
    assert_eq!(a.panes[0].tab().location, Location::Computer);
    // Cerrar y reabrir.
    a.run_command(Command::Changelog);
    let i = a.panes[0].active;
    a.close_tab(0, i);
    assert_eq!(changelog_count(&a), 0);
    a.run_command(Command::ReopenTab);
    assert_eq!(changelog_count(&a), 1);
}

#[test]
fn picker_changelog_opens_it() {
    let mut a = app();
    a.picker_open = true;
    let (w, h) = a.client_dips();
    let at = find_point(&a, |x, y| ui::picker_at(w, h, x, y) == Some(ui::PickerHit::Changelog)).expect("la rueda tiene «Novedades»");
    a.on_left_down(at.0, at.1, false);
    assert!(!a.picker_open);
    assert_eq!(changelog_count(&a), 1);
    assert!(a.panes[a.active].tab().is_changelog());
}

#[test]
fn about_changelog_button_opens_it_and_closes_about() {
    let mut a = app();
    a.run_command(Command::About);
    assert!(a.about_open);
    let (w, h) = a.client_dips();
    let at = find_point(&a, |x, y| about::at(w, h, x, y) == Some(about::AboutHit::Changelog)).expect("Acerca de tiene «Novedades»");
    a.on_left_down(at.0, at.1, false);
    assert!(!a.about_open);
    assert_eq!(changelog_count(&a), 1);
}

#[test]
fn palette_command() {
    let spec = commands::ALL.iter().find(|s| s.id == "novedades").expect("comando novedades");
    assert_eq!(spec.command, Command::Changelog);
    assert!(spec.default_keys.is_empty(), "sin atajo de fábrica");
    assert_eq!(commands::group(Command::Changelog), commands::Group::Other);
}

#[test]
fn no_notice_on_first_install_same_or_older() {
    // Sesión nueva (primera instalación): ya tiene la versión de ahora.
    let a = app();
    assert!(a.update_notice.is_none());
    assert_eq!(a.session().seen_version.as_deref(), Some(about::VERSION));
    let (a, _) = app_seen(Some(about::VERSION));
    assert!(a.update_notice.is_none());
    // Bajar de versión no avisa ni olvida la más nueva.
    let (a, _) = app_seen(Some("99.0.0"));
    assert!(a.update_notice.is_none());
    assert_eq!(a.session().seen_version.as_deref(), Some("99.0.0"));
}

#[test]
fn notice_after_update_opens_changelog_at_version() {
    let (mut a, host) = app_seen(Some("0.0.1"));
    assert_eq!(a.update_notice.as_deref(), Some(about::VERSION));
    // Se marca como visto: la sesión que se guarde ya tiene la versión de ahora.
    assert_eq!(a.session().seen_version.as_deref(), Some(about::VERSION));
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(texts.iter().any(|t| *t == format!("xdir se actualizó a {}", about::VERSION)), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Ver novedades"));
    let link = a.update_link.expect("el enlace quedó ubicado");
    let (x, y) = ((link.left + link.right) / 2.0, (link.top + link.bottom) / 2.0);
    assert_eq!(a.hit(x, y), Hit::UpdateNotice);
    a.on_left_down(x, y, false);
    assert!(a.update_notice.is_none(), "el aviso se va al abrir");
    assert!(a.panes[a.active].tab().is_changelog());
    let view = a.panes[a.active].tab().changelog.as_ref().unwrap();
    assert_eq!(view.target.as_deref(), Some(about::VERSION), "abre en esa versión");
    a.paint();
    assert!(a.update_link.is_none());
    assert_ne!(a.hit(x, y), Hit::UpdateNotice);
}

#[test]
fn notice_for_session_from_before_changelog() {
    // Una sesión de antes de Novedades (sin `version_vista`) avisa.
    let (a, _) = app_seen(None);
    assert_eq!(a.update_notice.as_deref(), Some(about::VERSION));
}

#[test]
fn other_notice_hides_update_notice_meanwhile() {
    let (mut a, host) = app_seen(Some("0.0.1"));
    a.notice = Some("Otra cosa".into());
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(!texts.iter().any(|t| t == "Ver novedades"));
    assert!(a.update_link.is_none());
    a.notice = None;
    a.paint();
    assert!(a.update_link.is_some(), "vuelve cuando el otro aviso se va");
}

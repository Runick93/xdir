//! Pruebas del menú de ramas del pie del panel y del cambio de rama (con git de verdad en repos
//! temporales; sin git en la máquina se saltan).

use super::*;
use crate::git::{self, Head, RepoHead, SwitchOutcome};
use crate::host::MenuPos;
use crate::shell::Entry;
use crate::sys::testing::FakeHost;
use super::git_branch::GitHow;

/// Repo temporal que se borra al soltarlo.
struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo(name: &str) -> Option<Repo> {
    let r = git::temp_repo(name).map(Repo);
    if r.is_none() {
        eprintln!("git no está instalado: se salta la prueba");
    }
    r
}

/// App con `dir` abierta en el panel 0 y su `git` al día, ya dibujada.
fn app_at(dir: &std::path::Path) -> (App, Rc<FakeHost>) {
    crate::sys::testing::install_fakes();
    let host = FakeHost::new();
    let mut a = App::new(Init::blank(), host.clone());
    a.new_tab(0, Location::Path(dir.to_path_buf()), true);
    wait_loaded(&mut a);
    a.panes[0].tab_mut().git = git::read_head(dir);
    a.paint();
    (a, host)
}

/// App con un `git` fijado a mano (sin repo real).
fn app_fake(head: Head) -> (App, Rc<FakeHost>) {
    let dir = std::env::temp_dir().join("xdir-tests").join("menu-rama");
    let (mut a, host) = app_at(&dir);
    a.panes[0].tab_mut().git = Some(RepoHead { root: dir, head });
    a.paint();
    (a, host)
}

fn click_button(a: &mut App, pane: usize) {
    let r = a.panes[pane].branch_button_rect();
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
}

/// (id, label, enabled, checked) de las entradas del último menú.
fn items(host: &FakeHost) -> Vec<(u32, String, bool, bool)> {
    host.menus()
        .last()
        .expect("hubo un menú")
        .iter()
        .map(|e| match e {
            Entry::Item { id, label, enabled, checked } => (*id, label.clone(), *enabled, *checked),
            _ => panic!("solo items"),
        })
        .collect()
}

fn id_of(host: &FakeHost, label: &str) -> u32 {
    items(host).into_iter().find(|i| i.1 == label).unwrap_or_else(|| panic!("falta {label}")).0
}

/// Espera a que ninguna pestaña siga cargando.
fn wait_loaded(a: &mut App) {
    super::network_tests::wait_until(a, |a| a.panes.iter().all(|p| p.tabs.iter().all(|t| !t.loading && !t.reload_pending())));
}

fn wait_idle(a: &mut App) {
    super::network_tests::wait_until(a, |a| !a.git_busy);
}

#[test]
fn click_opens_menu_with_branches() {
    let Some(r) = repo("menu") else { return };
    let (mut a, host) = app_at(&r.0);
    click_button(&mut a, 0);
    wait_idle(&mut a);
    let it = items(&host);
    assert_eq!(it.len(), 2);
    let main = it.iter().find(|i| i.1 == "main").unwrap();
    assert!(main.3 && !main.2, "la actual: marcada y deshabilitada");
    let feature = it.iter().find(|i| i.1 == "feature").unwrap();
    assert!(feature.2 && !feature.3);
    let b = a.panes[0].branch_button_rect();
    match host.menu_positions().last() {
        Some(MenuPos::Above(x, y)) => assert_eq!((*x, *y), (b.left, b.top)),
        p => panic!("se esperaba MenuPos::Above, fue {p:?}"),
    }
}

#[test]
fn menu_caps_at_40_branches() {
    let (mut a, host) = app_fake(Head::Branch("b0".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.git_busy = true;
    a.on_git_branches(0, root, Ok((0..45).map(|i| format!("b{i}")).collect()));
    let it = items(&host);
    assert_eq!(it.len(), 41);
    assert_eq!(it[40].1, "…y 5 más");
    assert!(!it[40].2);
    assert!(!a.git_busy);
}

#[test]
fn menu_empty_repo_says_no_branches() {
    let (mut a, host) = app_fake(Head::Branch("main".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.on_git_branches(0, root, Ok(vec![]));
    let it = items(&host);
    assert_eq!(it.len(), 1);
    assert_eq!((it[0].1.as_str(), it[0].2), ("(sin ramas)", false));
}

#[test]
fn menu_detached_head_marks_none() {
    let (mut a, host) = app_fake(Head::Detached("abc1234def5678".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.on_git_branches(0, root, Ok(vec!["main".into(), "feature".into()]));
    let it = items(&host);
    assert_eq!(it.len(), 2);
    assert!(it.iter().all(|i| !i.3 && i.2));
}

#[test]
fn choosing_branch_switches_and_reloads_all_panes() {
    let Some(r) = repo("switch") else { return };
    let (mut a, host) = app_at(&r.0);
    a.new_tab(1, Location::Path(r.0.clone()), true);
    wait_loaded(&mut a);
    a.panes[1].tab_mut().git = git::read_head(&r.0);
    a.paint();
    // Primero se ve el menú; la elección se fija antes de abrirlo.
    host.set_menu_choice(Some(1));
    click_button(&mut a, 0);
    // El menú se abre al volver las ramas: se elige `feature` por su etiqueta.
    super::network_tests::wait_until(&mut a, |a| !host.menus().is_empty() || !a.git_busy);
    assert_eq!(id_of(&host, "feature"), 1, "feature (la más reciente) va primero");
    wait_idle(&mut a);
    super::network_tests::wait_until(&mut a, |a| {
        a.panes[..2].iter().all(|p| p.tab().git.as_ref().map(|g| g.head == Head::Branch("feature".into())).unwrap_or(false))
    });
    assert_eq!(git::read_head(&r.0).unwrap().head, Head::Branch("feature".into()));
}

#[test]
fn switch_failed_shows_notice() {
    let Some(r) = repo("fail") else { return };
    let (mut a, host) = app_at(&r.0);
    let root = r.0.clone();
    a.start_switch(root, "no-existe".into(), GitHow::Plain);
    wait_idle(&mut a);
    let n = a.notice.clone().expect("aviso");
    assert!(!n.is_empty());
    assert!(host.questions().is_empty());
    assert_eq!(git::read_head(&r.0).unwrap().head, Head::Branch("main".into()));
}

#[test]
fn click_ignored_while_busy() {
    let Some(r) = repo("busy") else { return };
    let (mut a, host) = app_at(&r.0);
    a.git_busy = true;
    click_button(&mut a, 0);
    assert!(host.menus().is_empty());
    assert!(a.git_busy);
}

#[test]
fn result_for_vanished_pane_does_not_panic() {
    let (mut a, host) = app_fake(Head::Branch("main".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.git_busy = true;
    a.on_git_branches(9, root, Ok(vec!["main".into()]));
    assert!(host.menus().is_empty());
    assert!(!a.git_busy);
    a.git_busy = true;
    a.on_git_switched(PathBuf::from("Z:/no/hay/repo"), "x".into(), GitHow::Plain, SwitchOutcome::Switched);
    assert!(!a.git_busy);
    assert!(host.menus().is_empty());
}

#[test]
fn cancel_menu_does_nothing() {
    let Some(r) = repo("cancel") else { return };
    let (mut a, host) = app_at(&r.0);
    host.set_menu_choice(None);
    click_button(&mut a, 0);
    super::network_tests::wait_until(&mut a, |a| !a.git_busy);
    assert_eq!(host.menus().len(), 1);
    assert!(!a.git_busy);
    assert_eq!(git::read_head(&r.0).unwrap().head, Head::Branch("main".into()));
}

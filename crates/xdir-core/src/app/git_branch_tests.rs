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

/// La pestaña está en una subcarpeta que no existe en la rama destino: la recarga falla, pero
/// el botón muestra la rama nueva (no la de antes).
#[test]
fn switch_to_branch_without_tab_folder_shows_new_branch() {
    let Some(r) = repo("sin-sub") else { return };
    let sub = r.0.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("x.txt"), "x").unwrap();
    git::git_test(&r.0, &["add", "sub/x.txt"]).unwrap();
    git::git_test(&r.0, &["commit", "-m", "sub"]).unwrap();
    let (mut a, _host) = app_at(&sub);
    assert_eq!(a.panes[0].tab().git.as_ref().map(|g| g.head.clone()), Some(Head::Branch("main".into())));
    a.start_switch(r.0.clone(), "feature".into(), GitHow::Plain);
    wait_idle(&mut a);
    assert!(!sub.exists(), "la carpeta no está en feature");
    wait_loaded(&mut a);
    let g = a.panes[0].tab().git.clone().expect("sigue en el repo");
    assert_eq!((g.root, g.head), (r.0.clone(), Head::Branch("feature".into())));
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

// ---- el cartel de cambios sin commitear ----

const Q1_TITLE: &str = "No se puede pasar a «feature»";
const Q1_TEXT: &str = "Tienes cambios sin guardar que chocan con esa rama. ¿Guardarlos en el stash y cambiar? Sí: guardar en el stash y cambiar. No: descartar los cambios. Cancelar: quedarse aquí.";

fn read(r: &Repo, f: &str) -> String {
    std::fs::read_to_string(r.0.join(f)).unwrap()
}

/// Deja `a.txt` modificado y entrega a la app el `Dirty` que daría git, como si se hubiera elegido `feature`.
fn dirty_switch(a: &mut App, r: &Repo, extra_dirty: &[&str]) {
    std::fs::write(r.0.join("a.txt"), "local").unwrap();
    for f in extra_dirty {
        std::fs::write(r.0.join(f), "local").unwrap();
    }
    let files = git::dirty_count(&r.0);
    a.git_busy = true;
    a.on_git_switched(r.0.clone(), "feature".into(), GitHow::Plain, SwitchOutcome::Dirty { files });
}

fn head(r: &Repo) -> Head {
    git::read_head(&r.0).unwrap().head
}

#[test]
fn dirty_asks_stash_first() {
    let Some(r) = repo("d-pregunta") else { return };
    let (mut a, host) = app_at(&r.0);
    host.set_answer(None);
    dirty_switch(&mut a, &r, &[]);
    assert_eq!(host.questions(), vec![(Q1_TITLE.to_string(), Q1_TEXT.to_string())]);
}

#[test]
fn dirty_yes_stashes_and_switches() {
    let Some(r) = repo("d-stash") else { return };
    let (mut a, host) = app_at(&r.0);
    host.set_answer(Some(true));
    dirty_switch(&mut a, &r, &[]);
    assert!(a.git_busy, "el worker del stash arrancó");
    wait_idle(&mut a);
    assert_eq!(head(&r), Head::Branch("feature".into()));
    assert_eq!(read(&r, "a.txt"), "feature");
    let lista = git::git_test(&r.0, &["stash", "list"]).unwrap();
    assert!(lista.contains("xdir: antes de pasar a feature"), "{lista}");
    assert_eq!(a.notice.as_deref(), Some("Tus cambios están en el stash (xdir: antes de pasar a feature)"));
}

#[test]
fn dirty_no_then_yes_discards() {
    let Some(r) = repo("d-descarta") else { return };
    let (mut a, host) = app_at(&r.0);
    std::fs::write(r.0.join("nuevo.txt"), "n").unwrap();
    host.set_answers(vec![Some(false), Some(true)]);
    dirty_switch(&mut a, &r, &[]);
    wait_idle(&mut a);
    let q = host.questions();
    assert_eq!(q.len(), 2);
    assert_eq!(q[1].0, "Descartar cambios");
    assert_eq!(q[1].1, "Se perderá 1 archivo modificado. Esto no se puede deshacer. ¿Descartar y cambiar?");
    assert_eq!(head(&r), Head::Branch("feature".into()));
    assert_eq!(read(&r, "a.txt"), "feature");
    assert!(r.0.join("nuevo.txt").exists());
}

#[test]
fn dirty_no_then_no_does_nothing() {
    let Some(r) = repo("d-no-no") else { return };
    let (mut a, host) = app_at(&r.0);
    host.set_answers(vec![Some(false), Some(false)]);
    dirty_switch(&mut a, &r, &[]);
    assert_eq!(host.questions().len(), 2);
    assert!(!a.git_busy);
    assert!(a.notice.is_none());
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "a.txt"), "local");
}

#[test]
fn dirty_cancel_does_nothing() {
    let Some(r) = repo("d-cancela") else { return };
    let (mut a, host) = app_at(&r.0);
    host.set_answer(None);
    dirty_switch(&mut a, &r, &[]);
    assert_eq!(host.questions().len(), 1);
    assert!(!a.git_busy);
    assert!(a.notice.is_none());
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "a.txt"), "local");
}

#[test]
fn dirty_count_in_plural() {
    let Some(r) = repo("d-plural") else { return };
    let (mut a, host) = app_at(&r.0);
    std::fs::write(r.0.join("c.txt"), "c").unwrap();
    git::git_test(&r.0, &["add", "c.txt"]).unwrap();
    git::git_test(&r.0, &["commit", "-m", "c"]).unwrap();
    host.set_answers(vec![Some(false), Some(false)]);
    dirty_switch(&mut a, &r, &["b.txt", "c.txt"]);
    let q = host.questions();
    assert_eq!(q[1].1, "Se perderán 3 archivos modificados. Esto no se puede deshacer. ¿Descartar y cambiar?");
}

/// Un `Dirty` sin archivos con seguimiento (no debería pasar): el aviso general, sin ventana.
#[test]
fn dirty_without_tracked_files_is_a_plain_notice() {
    let (mut a, host) = app_fake(Head::Branch("main".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.git_busy = true;
    a.on_git_switched(root, "feature".into(), GitHow::Plain, SwitchOutcome::Dirty { files: 0 });
    assert!(host.questions().is_empty());
    assert!(!a.git_busy);
    assert_eq!(a.notice.as_deref(), Some("No se pudo cambiar de rama: hay cambios sin guardar que chocan"));
}

// ---- archivos sin seguimiento que se sobrescribirían ----

const UNTRACKED_TEXT: &str = "Hay archivos sin seguimiento que se sobrescribirían al cambiar. ¿Guardarlos en el stash y cambiar?";
const UNTRACKED_NOTICE: &str = "No se pudo cambiar de rama: hay archivos sin seguimiento que se sobrescribirían";

/// `feature` gana `n.txt`; en `main` hay un `n.txt` nuevo sin seguimiento que lo tapa.
fn untracked_in_the_way(r: &Repo) {
    git::git_test(&r.0, &["switch", "feature"]).unwrap();
    std::fs::write(r.0.join("n.txt"), "feature").unwrap();
    git::git_test(&r.0, &["add", "n.txt"]).unwrap();
    git::git_test(&r.0, &["commit", "-m", "n"]).unwrap();
    git::git_test(&r.0, &["switch", "main"]).unwrap();
    std::fs::write(r.0.join("n.txt"), "local").unwrap();
}

#[test]
fn untracked_yes_stashes_and_switches() {
    let Some(r) = repo("u-stash") else { return };
    untracked_in_the_way(&r);
    let (mut a, host) = app_at(&r.0);
    host.set_answers(vec![Some(true)]);
    a.start_switch(r.0.clone(), "feature".into(), GitHow::Plain);
    wait_idle(&mut a);
    assert_eq!(host.questions(), vec![(Q1_TITLE.to_string(), UNTRACKED_TEXT.to_string())]);
    assert!(!UNTRACKED_TEXT.to_lowercase().contains("descartar"));
    assert_eq!(head(&r), Head::Branch("feature".into()));
    assert_eq!(read(&r, "n.txt"), "feature");
    let archivos = git::git_test(&r.0, &["stash", "show", "--include-untracked", "--name-only"]).unwrap();
    assert!(archivos.contains("n.txt"), "{archivos}");
    assert_eq!(a.notice.as_deref(), Some("Tus cambios están en el stash (xdir: antes de pasar a feature)"));
}

#[test]
fn untracked_no_does_nothing() {
    let Some(r) = repo("u-no") else { return };
    untracked_in_the_way(&r);
    let (mut a, host) = app_at(&r.0);
    host.set_answers(vec![Some(false)]);
    a.start_switch(r.0.clone(), "feature".into(), GitHow::Plain);
    wait_idle(&mut a);
    assert_eq!(host.questions().len(), 1, "una sola pregunta: sin la de descartar");
    assert!(!a.git_busy);
    assert!(a.notice.is_none());
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "n.txt"), "local");
}

/// Cambios con seguimiento que no chocan y un archivo nuevo que sí: la ventana del archivo
/// nuevo (no la de guardar o descartar).
#[test]
fn untracked_mixed_with_tracked_changes_asks_untracked() {
    let Some(r) = repo("u-mezcla") else { return };
    untracked_in_the_way(&r);
    std::fs::write(r.0.join("b.txt"), "local").unwrap();
    let (mut a, host) = app_at(&r.0);
    host.set_answers(vec![Some(false)]);
    a.start_switch(r.0.clone(), "feature".into(), GitHow::Plain);
    wait_idle(&mut a);
    assert_eq!(host.questions(), vec![(Q1_TITLE.to_string(), UNTRACKED_TEXT.to_string())]);
}

/// Si ya se probó guardar o descartar: un aviso, sin ventana.
#[test]
fn untracked_after_stash_or_discard_is_a_notice() {
    for how in [GitHow::Stash, GitHow::Discard] {
        let (mut a, host) = app_fake(Head::Branch("main".into()));
        let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
        a.git_busy = true;
        a.on_git_switched(root, "feature".into(), how, SwitchOutcome::UntrackedInTheWay);
        assert!(host.questions().is_empty());
        assert!(!a.git_busy);
        assert_eq!(a.notice.as_deref(), Some(UNTRACKED_NOTICE));
    }
}

#[test]
fn dirty_from_stash_attempt_does_not_reopen_dialog() {
    let (mut a, host) = app_fake(Head::Branch("main".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.git_busy = true;
    a.on_git_switched(root, "feature".into(), GitHow::Stash, SwitchOutcome::Dirty { files: 2 });
    assert!(host.questions().is_empty());
    assert!(!a.git_busy);
    assert!(a.notice.is_some());
}

#[test]
fn stash_failure_reports_error_and_keeps_changes_message() {
    let (mut a, _host) = app_fake(Head::Branch("main".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    let msg = "fatal: algo. Tus cambios quedan en el stash (xdir: antes de pasar a feature).".to_string();
    a.git_busy = true;
    a.on_git_switched(root, "feature".into(), GitHow::Stash, SwitchOutcome::Failed(msg));
    assert!(a.notice.as_deref().unwrap().contains("Tus cambios quedan en el stash"));
    assert!(!a.git_busy);
}

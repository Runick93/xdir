//! Pruebas del menú de ramas del pie del panel y del cambio de rama (con git de verdad en repos
//! temporales; sin git en la máquina se saltan).

use super::*;
use crate::git::{self, Head, RepoHead, SwitchOutcome};
use crate::host::MenuPos;
use crate::shell::Entry;
use crate::sys::testing::FakeHost;
use super::git_branch::GitHow;
use crate::keys::keys::{VK_LEFT, VK_RIGHT, VK_TAB};

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

// ---- el indicador de actividad mientras git trabaja ----

/// Dibuja y dice si el indicador tiene el trabajo de git.
fn git_activity(a: &mut App) -> bool {
    a.paint();
    a.activity.contains(&super::activity::ActivityKey::Git)
}

/// El cartel del indicador pasado su umbral (con el reloj fijo).
fn activity_tip(a: &mut App) -> Option<String> {
    let t0 = std::time::Instant::now();
    crate::clock::set(Some(t0));
    a.paint();
    crate::clock::set(Some(t0 + super::activity::SHOW_AFTER));
    a.paint();
    let tip = a.tooltip_text(Hit::Activity);
    crate::clock::set(None);
    tip
}

#[test]
fn activity_while_reading_branches_until_menu_closes() {
    let Some(r) = repo("act-ramas") else { return };
    let (mut a, host) = app_at(&r.0);
    host.set_menu_choice(None);
    assert!(!git_activity(&mut a));
    click_button(&mut a, 0);
    assert!(a.git_busy);
    assert!(git_activity(&mut a), "leyendo las ramas");
    assert_eq!(activity_tip(&mut a).as_deref(), Some("Leyendo las ramas"));
    // Se cancela el menú: se apaga.
    wait_idle(&mut a);
    assert!(!git_activity(&mut a));
}

#[test]
fn activity_while_switching_until_done() {
    let Some(r) = repo("act-cambio") else { return };
    let (mut a, _host) = app_at(&r.0);
    a.start_switch(r.0.clone(), "feature".into(), GitHow::Plain);
    assert!(git_activity(&mut a), "cambiando de rama");
    assert_eq!(activity_tip(&mut a).as_deref(), Some("Cambiando a \"feature\""));
    wait_idle(&mut a);
    assert!(!git_activity(&mut a));
    assert_eq!(head(&r), Head::Branch("feature".into()));
}

/// Todos los caminos que apagan `git_busy` apagan el indicador: error al leer las ramas, panel
/// que ya no está, y cada resultado del cambio (también el de respaldo si el hilo se cae, que
/// llega como `Err` o `Failed`).
#[test]
fn activity_ends_on_every_path() {
    let (mut a, host) = app_fake(Head::Branch("main".into()));
    host.set_answer(None);
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    let busy = |a: &mut App, text: &str| {
        a.git_busy = true;
        a.git_task = text.to_string();
        assert!(git_activity(a));
    };
    busy(&mut a, "Leyendo las ramas");
    a.on_git_branches(0, root.clone(), Err("git no respondió".into()));
    assert!(!git_activity(&mut a), "error al leer");
    busy(&mut a, "Leyendo las ramas");
    a.on_git_branches(9, root.clone(), Ok(vec!["main".into()]));
    assert!(!git_activity(&mut a), "panel que ya no está");
    let outcomes = [
        SwitchOutcome::Failed("git no respondió".into()),
        SwitchOutcome::Dirty { files: 0 },
        SwitchOutcome::Dirty { files: 2 },
        SwitchOutcome::UntrackedInTheWay,
        SwitchOutcome::Switched,
    ];
    for o in outcomes {
        busy(&mut a, "Cambiando a \"feature\"");
        a.on_git_switched(root.clone(), "feature".into(), GitHow::Plain, o.clone());
        assert!(!git_activity(&mut a), "{o:?}");
    }
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

// ---- el diálogo de cambios sin commitear (con el estilo de xdir, no un cuadro del sistema) ----

const TITLE: &str = "No se puede pasar a «feature»";
const DIRTY_TEXT: &str = "Tenés cambios sin guardar que chocan con esa rama. Podés guardarlos en el stash para recuperarlos después, o descartarlos.";
const STASH: &str = "Guardar en el stash y cambiar";
const DISCARD: &str = "Descartar…";
const DISCARD_NOW: &str = "Descartar y cambiar";
const CANCEL: &str = "Cancelar";

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

/// El diálogo abierto (falla si no hay ninguno).
fn dialog(a: &App) -> &super::confirm::Confirm {
    &a.confirm.as_ref().expect("hay un diálogo abierto").dialog
}

fn labels(a: &App) -> Vec<String> {
    dialog(a).buttons.iter().map(|b| b.label.clone()).collect()
}

/// Aprieta el botón de nombre `label` del diálogo abierto.
fn press(a: &mut App, label: &str) {
    let i = dialog(a).buttons.iter().position(|b| b.label == label).unwrap_or_else(|| panic!("no hay un botón «{label}»: {:?}", labels(a)));
    a.confirm_choose(Some(i));
}

#[test]
fn dirty_opens_an_own_dialog_with_named_buttons() {
    let Some(r) = repo("d-dialogo") else { return };
    let (mut a, host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    let d = dialog(&a);
    assert_eq!((d.title.as_str(), d.text.as_str()), (TITLE, DIRTY_TEXT));
    assert_eq!(labels(&a), [STASH, DISCARD, CANCEL]);
    assert_eq!(d.focus, 0, "el foco empieza en guardar en el stash");
    assert!(!a.git_busy, "nada corre mientras se decide");
    assert!(host.questions().is_empty(), "ya no es un cuadro del sistema");
}

#[test]
fn dirty_stash_button_stashes_and_switches() {
    let Some(r) = repo("d-stash") else { return };
    let (mut a, _host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    press(&mut a, STASH);
    assert!(a.confirm.is_none());
    assert!(a.git_busy, "el worker del stash arrancó");
    wait_idle(&mut a);
    assert_eq!(head(&r), Head::Branch("feature".into()));
    assert_eq!(read(&r, "a.txt"), "feature");
    let lista = git::git_test(&r.0, &["stash", "list"]).unwrap();
    assert!(lista.contains("xdir: antes de pasar a feature"), "{lista}");
    assert_eq!(a.notice.as_deref(), Some("Tus cambios están en el stash (xdir: antes de pasar a feature)"));
}

#[test]
fn dirty_discard_asks_again_and_then_discards() {
    let Some(r) = repo("d-descarta") else { return };
    let (mut a, _host) = app_at(&r.0);
    std::fs::write(r.0.join("nuevo.txt"), "n").unwrap();
    dirty_switch(&mut a, &r, &[]);
    press(&mut a, DISCARD);
    let d = dialog(&a);
    assert_eq!((d.title.as_str(), d.text.as_str()), ("Descartar cambios", "Se perderá 1 archivo modificado. Esto no se puede deshacer."));
    assert_eq!(labels(&a), [DISCARD_NOW, CANCEL]);
    assert!(!a.git_busy);
    assert_eq!(read(&r, "a.txt"), "local", "todavía no se descartó nada");
    press(&mut a, DISCARD_NOW);
    wait_idle(&mut a);
    assert_eq!(head(&r), Head::Branch("feature".into()));
    assert_eq!(read(&r, "a.txt"), "feature");
    assert!(r.0.join("nuevo.txt").exists(), "lo nuevo sin seguimiento no se toca");
}

/// Un Enter distraído sobre la confirmación de descartar no destruye nada: el foco empieza en «Cancelar».
#[test]
fn discard_confirmation_defaults_to_cancel_on_enter() {
    let Some(r) = repo("d-enter") else { return };
    let (mut a, _host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    press(&mut a, DISCARD);
    assert_eq!(dialog(&a).focus, 1);
    assert!(a.confirm_key(VK_RETURN, false));
    assert!(a.confirm.is_none());
    assert!(!a.git_busy);
    assert!(a.notice.is_none());
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "a.txt"), "local");
}

#[test]
fn dirty_cancel_does_nothing() {
    let Some(r) = repo("d-cancela") else { return };
    let (mut a, _host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    press(&mut a, CANCEL);
    assert!(a.confirm.is_none());
    assert!(!a.git_busy);
    assert!(a.notice.is_none());
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "a.txt"), "local");
}

#[test]
fn escape_and_close_cross_cancel() {
    let Some(r) = repo("d-esc") else { return };
    let (mut a, _host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    assert!(a.confirm_key(VK_ESCAPE, false));
    assert!(a.confirm.is_none() && !a.git_busy);
    dirty_switch(&mut a, &r, &[]);
    a.confirm_click(Some(super::confirm::ConfirmHit::Close));
    assert!(a.confirm.is_none() && !a.git_busy);
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "a.txt"), "local");
}

#[test]
fn arrows_and_tab_move_the_focus_and_enter_presses_it() {
    let Some(r) = repo("d-foco") else { return };
    let (mut a, _host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    a.confirm_key(VK_RIGHT, false);
    assert_eq!(dialog(&a).focus, 1);
    a.confirm_key(VK_TAB, true);
    assert_eq!(dialog(&a).focus, 0);
    a.confirm_key(VK_LEFT, false);
    assert_eq!(dialog(&a).focus, 2, "da la vuelta");
    a.confirm_key(VK_TAB, false);
    assert_eq!(dialog(&a).focus, 0);
    a.confirm_key(VK_RETURN, false);
    assert!(a.git_busy && a.confirm.is_none(), "Enter apretó «{STASH}»");
    wait_idle(&mut a);
    assert_eq!(head(&r), Head::Branch("feature".into()));
}

/// Con el diálogo abierto los clics solo cuentan dentro de él, y se resuelven sobre lo dibujado.
#[test]
fn clicking_the_drawn_buttons_works_and_the_dialog_is_modal() {
    let Some(r) = repo("d-clic") else { return };
    let (mut a, _host) = app_at(&r.0);
    dirty_switch(&mut a, &r, &[]);
    a.paint();
    assert_eq!(a.hit(2.0, 2.0), Hit::None, "fuera del panel no hay nada: ni la lista ni el pie");
    let nb = a.panes[0].branch_button_rect();
    assert_ne!(a.hit((nb.left + nb.right) / 2.0, (nb.top + nb.bottom) / 2.0), Hit::Pane(0, PaneHit::BranchButton), "el botón de rama de abajo no responde");
    // Busca el centro de cada botón probando puntos de la franja de abajo del panel.
    let (w, h) = a.client_dips();
    let mut found = Vec::new();
    for yy in (0..h as i32).step_by(4) {
        for xx in (0..w as i32).step_by(4) {
            if let Hit::Confirm(super::confirm::ConfirmHit::Button(i)) = a.hit(xx as f32, yy as f32) {
                if !found.iter().any(|(j, _, _)| *j == i) {
                    found.push((i, xx as f32, yy as f32));
                }
            }
        }
    }
    found.sort_by_key(|f| f.0);
    assert_eq!(found.len(), 3, "los tres botones se pueden acertar: {found:?}");
    let (_, x, y) = found[2]; // «Cancelar»
    a.on_left_down(x, y, false);
    assert!(a.confirm.is_none(), "el clic en Cancelar cerró el diálogo");
    assert_eq!(head(&r), Head::Branch("main".into()));
}

#[test]
fn dirty_count_in_plural() {
    let Some(r) = repo("d-plural") else { return };
    let (mut a, _host) = app_at(&r.0);
    std::fs::write(r.0.join("c.txt"), "c").unwrap();
    git::git_test(&r.0, &["add", "c.txt"]).unwrap();
    git::git_test(&r.0, &["commit", "-m", "c"]).unwrap();
    dirty_switch(&mut a, &r, &["b.txt", "c.txt"]);
    press(&mut a, DISCARD);
    assert_eq!(dialog(&a).text, "Se perderán 3 archivos modificados. Esto no se puede deshacer.");
}

/// Un `Dirty` sin archivos con seguimiento (no debería pasar): el aviso general, sin diálogo.
#[test]
fn dirty_without_tracked_files_is_a_plain_notice() {
    let (mut a, host) = app_fake(Head::Branch("main".into()));
    let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
    a.git_busy = true;
    a.on_git_switched(root, "feature".into(), GitHow::Plain, SwitchOutcome::Dirty { files: 0 });
    assert!(a.confirm.is_none() && host.questions().is_empty());
    assert!(!a.git_busy);
    assert_eq!(a.notice.as_deref(), Some("No se pudo cambiar de rama: hay cambios sin guardar que chocan"));
}

// ---- archivos sin seguimiento que se sobrescribirían ----

const UNTRACKED_TEXT: &str = "Hay archivos sin seguimiento que se sobrescribirían al cambiar. ¿Querés guardarlos en el stash y cambiar?";
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

/// Intenta cambiar a `feature` y espera a que git rechace y se abra el diálogo.
fn switch_until_dialog(a: &mut App, r: &Repo) {
    a.start_switch(r.0.clone(), "feature".into(), GitHow::Plain);
    super::network_tests::wait_until(a, |a| a.confirm.is_some());
}

#[test]
fn untracked_dialog_offers_only_stash_and_cancel() {
    let Some(r) = repo("u-dialogo") else { return };
    untracked_in_the_way(&r);
    let (mut a, host) = app_at(&r.0);
    switch_until_dialog(&mut a, &r);
    let d = dialog(&a);
    assert_eq!((d.title.as_str(), d.text.as_str()), (TITLE, UNTRACKED_TEXT));
    assert_eq!(labels(&a), [STASH, CANCEL], "sin la opción de descartar: no serviría");
    assert!(!a.git_busy);
    assert!(host.questions().is_empty());
}

#[test]
fn untracked_stash_button_stashes_and_switches() {
    let Some(r) = repo("u-stash") else { return };
    untracked_in_the_way(&r);
    let (mut a, _host) = app_at(&r.0);
    switch_until_dialog(&mut a, &r);
    press(&mut a, STASH);
    wait_idle(&mut a);
    assert_eq!(head(&r), Head::Branch("feature".into()));
    assert_eq!(read(&r, "n.txt"), "feature");
    let archivos = git::git_test(&r.0, &["stash", "show", "--include-untracked", "--name-only"]).unwrap();
    assert!(archivos.contains("n.txt"), "{archivos}");
    assert_eq!(a.notice.as_deref(), Some("Tus cambios están en el stash (xdir: antes de pasar a feature)"));
}

#[test]
fn untracked_cancel_does_nothing() {
    let Some(r) = repo("u-cancela") else { return };
    untracked_in_the_way(&r);
    let (mut a, _host) = app_at(&r.0);
    switch_until_dialog(&mut a, &r);
    press(&mut a, CANCEL);
    assert!(a.confirm.is_none() && !a.git_busy);
    assert!(a.notice.is_none());
    assert_eq!(head(&r), Head::Branch("main".into()));
    assert_eq!(read(&r, "n.txt"), "local");
}

/// Cambios con seguimiento que no chocan y un archivo nuevo que sí: el diálogo del archivo
/// nuevo (no el de guardar o descartar).
#[test]
fn untracked_mixed_with_tracked_changes_asks_untracked() {
    let Some(r) = repo("u-mezcla") else { return };
    untracked_in_the_way(&r);
    std::fs::write(r.0.join("b.txt"), "local").unwrap();
    let (mut a, _host) = app_at(&r.0);
    switch_until_dialog(&mut a, &r);
    assert_eq!(dialog(&a).text, UNTRACKED_TEXT);
    assert_eq!(labels(&a), [STASH, CANCEL]);
}

/// Si ya se probó guardar o descartar: un aviso, sin diálogo.
#[test]
fn untracked_after_stash_or_discard_is_a_notice() {
    for how in [GitHow::Stash, GitHow::Discard] {
        let (mut a, host) = app_fake(Head::Branch("main".into()));
        let root = a.panes[0].tab().git.as_ref().unwrap().root.clone();
        a.git_busy = true;
        a.on_git_switched(root, "feature".into(), how, SwitchOutcome::UntrackedInTheWay);
        assert!(a.confirm.is_none() && host.questions().is_empty());
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
    assert!(a.confirm.is_none() && host.questions().is_empty());
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
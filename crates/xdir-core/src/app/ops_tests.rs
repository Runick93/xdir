//! Pruebas de las operaciones chicas de la tanda D en la `App` (Duplicar, Carpeta nueva con la
//! selección, Devolver, accesos directos, Expulsar).

use super::*;
use crate::sys::testing::{FakeEnv, FakeFileOps, FakeHost, FakeShell, ShellCall};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

fn app_with_host(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakeEnv::hold_keys(&[]);
    FakeShell::take_calls();
    FakeShell::choose(None);
    FakeFileOps::take_ops();
    FakeFileOps::respond_with(OpResult::default());
    App::new(Init::blank(), host)
}

fn app() -> App {
    app_with_host(FakeHost::new())
}

/// Carpeta temporal propia de la prueba con esos archivos.
fn fixture(name: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-ops-d").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::write(dir.join(f), f.as_bytes()).unwrap();
    }
    sys::file_system().true_case(&dir)
}

fn pump(a: &mut App, pane: usize) {
    while a.panes[pane].tab().loading || a.panes[pane].tab().reload_pending() {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
}

fn go(a: &mut App, dir: &Path) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.set_active(0);
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    pump(a, 0);
}

fn pos_of(a: &App, name: &str) -> usize {
    let list = &a.panes[0].tab().list;
    (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == name).expect(name)
}

fn select(a: &mut App, names: &[&str]) {
    let positions: Vec<usize> = names.iter().map(|n| pos_of(a, n)).collect();
    let list = &mut a.panes[0].tab_mut().list;
    for (i, p) in positions.into_iter().enumerate() {
        if i == 0 {
            list.select_pos(p);
        } else {
            list.set_selected_pos(p, true);
        }
    }
}

fn deselect_all(a: &mut App) {
    let list = &mut a.panes[0].tab_mut().list;
    for p in list.selected_positions() {
        list.set_selected_pos(p, false);
    }
}

fn selected_names(a: &App) -> Vec<String> {
    let list = &a.panes[0].tab().list;
    let mut v: Vec<String> =
        list.selected_positions().into_iter().map(|p| list.entries[list.entry_of_pos(p).unwrap()].name.to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn focused_name(a: &App) -> String {
    let list = &a.panes[0].tab().list;
    list.focused_index().map(|i| list.entries[i].name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Entrega lo que respondió el `FileOps` falso (lo que haría `WM_APP_OP_DONE`).
fn drain_ops(a: &mut App) {
    while let Ok(r) = a.ops_rx.try_recv() {
        a.on_op_done(r);
    }
}

/// Las entradas propias del menú del clic derecho sobre la selección (sin elegir nada).
fn own_menu(a: &mut App) -> Vec<String> {
    FakeShell::choose(None);
    FakeShell::take_calls();
    a.on_context_menu(Some((10.0, 20.0)));
    let calls = FakeShell::take_calls();
    let Some(ShellCall::ContextMenu { own, .. }) = calls.into_iter().find(|c| matches!(c, ShellCall::ContextMenu { .. })) else {
        panic!("se esperaba un menú")
    };
    own.iter()
        .filter_map(|e| match e {
            shell::Entry::Item { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect()
}

/// Elige la entrada propia `label` del menú del clic derecho.
fn choose_menu(a: &mut App, label: &str) {
    FakeShell::choose(None);
    FakeShell::take_calls();
    a.on_context_menu(Some((10.0, 20.0)));
    let calls = FakeShell::take_calls();
    let Some(ShellCall::ContextMenu { own, .. }) = calls.into_iter().find(|c| matches!(c, ShellCall::ContextMenu { .. })) else {
        panic!("se esperaba un menú")
    };
    let id = own
        .iter()
        .find_map(|e| match e {
            shell::Entry::Item { id, label: l, .. } if l == label => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("falta «{label}»"));
    FakeShell::choose(Some(&format!("own:{id}")));
    a.on_context_menu(Some((10.0, 20.0)));
    FakeShell::choose(None);
}

// ---------- Duplicar ----------

/// Ctrl+D con dos seleccionados manda `Op::Duplicate`; sin selección o en una carpeta virtual,
/// nada.
#[test]
fn duplicate_spawns_op_with_selection() {
    use crate::keys::keys::VK_CONTROL;
    let mut a = app();
    let d = fixture("dup-op", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt", "b.txt"]);
    FakeFileOps::take_ops();
    FakeEnv::hold_keys(&[VK_CONTROL]);
    assert!(a.on_key(Key(b'D' as u16)));
    FakeEnv::hold_keys(&[]);
    let ops = FakeFileOps::take_ops();
    let [Op::Duplicate { sources }] = ops.as_slice() else { panic!("{ops:?}") };
    let mut sources = sources.clone();
    sources.sort();
    assert_eq!(sources, [d.join("a.txt"), d.join("b.txt")]);
    // Sin selección: nada.
    deselect_all(&mut a);
    a.run_command(Command::Duplicate);
    assert_eq!(FakeFileOps::take_ops(), []);
    // En una carpeta virtual (la Papelera de Windows): nada.
    let parsing = format!("::{{bin}}-dup-{}", std::process::id());
    crate::sys::testing::FakeFileSystem::set_shell_items(&parsing, &["x.txt"]);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Shell { parsing, name: "Papelera".into(), parent: None });
    pump(&mut a, 0);
    a.panes[0].tab_mut().list.select_pos(0);
    a.run_command(Command::Duplicate);
    assert_eq!(FakeFileOps::take_ops(), []);
}

/// Al terminar, las copias (`OpResult::select`) quedan seleccionadas tras la recarga, con el foco
/// en la última.
#[test]
fn duplicate_selects_copies_after_reload() {
    let mut a = app();
    let d = fixture("dup-sel", &["a.txt", "b.txt", "a copia.txt", "b copia.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt", "b.txt"]);
    let copies = vec![d.join("a copia.txt"), d.join("b copia.txt")];
    FakeFileOps::respond_with(OpResult {
        undo: Some(Undo::Created(copies.clone())),
        focus: copies.last().cloned(),
        select: copies.clone(),
        ..Default::default()
    });
    a.run_command(Command::Duplicate);
    drain_ops(&mut a);
    pump(&mut a, 0);
    assert_eq!(selected_names(&a), ["a copia.txt", "b copia.txt"]);
    assert_eq!(focused_name(&a), "b copia.txt");
    assert_eq!(a.undo.last(), Some(&Undo::Created(copies)));
}

#[test]
fn item_menu_has_duplicate() {
    let mut a = app();
    let d = fixture("dup-menu", &["a.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt"]);
    assert!(own_menu(&mut a).iter().any(|l| l == "Duplicar"));
    FakeFileOps::take_ops();
    choose_menu(&mut a, "Duplicar");
    assert_eq!(FakeFileOps::take_ops(), [Op::Duplicate { sources: vec![d.join("a.txt")] }]);
}

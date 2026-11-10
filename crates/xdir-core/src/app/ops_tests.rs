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

// ---------- Carpeta nueva con la selección ----------

/// Con dos seleccionados, Ctrl+Alt+Shift+N manda `Op::NewFolderWith` con la carpeta del panel; el
/// resultado con `rename_after` abre el renombrado sobre la carpeta nueva.
#[test]
fn new_folder_with_spawns_op() {
    use crate::host::TextKind;
    use crate::keys::keys::{VK_CONTROL, VK_MENU, VK_SHIFT};
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("nfw-op", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt", "b.txt"]);
    let folder = d.join("Carpeta nueva con 2 elementos");
    std::fs::create_dir(&folder).unwrap();
    FakeFileOps::respond_with(OpResult { focus: Some(folder.clone()), rename_after: true, ..Default::default() });
    FakeEnv::hold_keys(&[VK_CONTROL, VK_MENU, VK_SHIFT]);
    assert!(a.on_key(Key(b'N' as u16)));
    FakeEnv::hold_keys(&[]);
    let ops = FakeFileOps::take_ops();
    let [Op::NewFolderWith { parent, items }] = ops.as_slice() else { panic!("{ops:?}") };
    assert_eq!(parent, &d);
    let mut items = items.clone();
    items.sort();
    assert_eq!(items, [d.join("a.txt"), d.join("b.txt")]);
    drain_ops(&mut a);
    pump(&mut a, 0);
    a.try_pending_rename();
    assert_eq!(host.text_of(TextKind::Rename).as_deref(), Some("Carpeta nueva con 2 elementos"));
    // Sin selección: nada.
    let mut b = app();
    go(&mut b, &d);
    FakeFileOps::take_ops();
    b.run_command(Command::NewFolderWithSelection);
    assert_eq!(FakeFileOps::take_ops(), []);
}

/// Meta (⌃ en la Mac) llega a los atajos: un atajo propio con Meta se resuelve con la tecla
/// apretada y no sin ella.
#[test]
fn meta_key_reaches_shortcuts() {
    use crate::keys::keys::{VK_CONTROL, VK_LWIN};
    let mut a = app();
    let d = fixture("nfw-meta", &["a.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt"]);
    a.config.keymap.set(Command::NewFolderWithSelection, &[crate::commands::parse_combo("Meta+Ctrl+K").unwrap()]);
    FakeFileOps::take_ops();
    FakeEnv::hold_keys(&[VK_CONTROL]);
    a.on_key(Key(b'K' as u16));
    assert_eq!(FakeFileOps::take_ops(), []);
    FakeEnv::hold_keys(&[VK_CONTROL, VK_LWIN]);
    a.on_key(Key(b'K' as u16));
    FakeEnv::hold_keys(&[]);
    assert_eq!(FakeFileOps::take_ops(), [Op::NewFolderWith { parent: d.clone(), items: vec![d.join("a.txt")] }]);
}

/// El menú del clic derecho ofrece «Carpeta nueva con la selección» solo con dos o más.
#[test]
fn menu_new_folder_with_needs_two() {
    let mut a = app();
    let d = fixture("nfw-menu", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt"]);
    assert!(!own_menu(&mut a).iter().any(|l| l == "Carpeta nueva con la selección"));
    select(&mut a, &["a.txt", "b.txt"]);
    assert!(own_menu(&mut a).iter().any(|l| l == "Carpeta nueva con la selección"));
    FakeFileOps::take_ops();
    choose_menu(&mut a, "Carpeta nueva con la selección");
    let ops = FakeFileOps::take_ops();
    assert!(matches!(ops.as_slice(), [Op::NewFolderWith { items, .. }] if items.len() == 2), "{ops:?}");
}

// ---------- Devolver desde la Papelera ----------

/// Con la Papelera a la vista: «Devolver» primero en el menú; el botón de la barra está apagado
/// sin selección y con dos seleccionados manda `Op::Restore`.
#[test]
fn restore_menu_and_button_in_trash() {
    use crate::sys::testing::FakeFileSystem;
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("papelera", &["a.txt", "b.txt", "c.txt"]);
    FakeFileSystem::set_trash(Some(&d));
    go(&mut a, &d);
    a.paint();
    let r = a.panes[0].restore_button().expect("botón Devolver en la Papelera");
    assert!(host.drawn_texts().iter().any(|(_, s, _)| s == "Devolver"));
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    FakeFileOps::take_ops();
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert_eq!(FakeFileOps::take_ops(), [], "apagado sin selección");
    select(&mut a, &["a.txt", "b.txt"]);
    assert_eq!(own_menu(&mut a).first().map(String::as_str), Some("Devolver"));
    a.paint();
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    let ops = FakeFileOps::take_ops();
    let [Op::Restore { items }] = ops.as_slice() else { panic!("{ops:?}") };
    let mut items = items.clone();
    items.sort();
    assert_eq!(items, [d.join("a.txt"), d.join("b.txt")]);
    choose_menu(&mut a, "Devolver");
    assert!(matches!(FakeFileOps::take_ops().as_slice(), [Op::Restore { .. }]));
    FakeFileSystem::set_trash(None);
}

/// En la Mac, ⌘⌫ (`Recycle`) dentro de la Papelera devuelve, como el Finder; en Windows sigue
/// siendo mandar a la Papelera.
#[test]
fn recycle_in_trash_restores_on_mac() {
    use crate::sys::KeyStyle;
    use crate::sys::testing::FakeFileSystem;
    let mut a = app();
    let d = fixture("papelera-mac", &["a.txt"]);
    FakeFileSystem::set_trash(Some(&d));
    go(&mut a, &d);
    select(&mut a, &["a.txt"]);
    FakeFileOps::take_ops();
    FakeEnv::set_key_style(KeyStyle::Mac);
    a.run_command(Command::Recycle);
    FakeEnv::set_key_style(KeyStyle::Windows);
    assert_eq!(FakeFileOps::take_ops(), [Op::Restore { items: vec![d.join("a.txt")] }]);
    a.run_command(Command::Recycle);
    assert_eq!(FakeFileOps::take_ops(), [Op::Delete { paths: vec![d.join("a.txt")], recycle: true }]);
    FakeFileSystem::set_trash(None);
}

/// Fuera de la Papelera no hay «Devolver» (ni menú, ni botón, ni comando).
#[test]
fn restore_not_offered_outside_trash() {
    let mut a = app();
    let d = fixture("no-papelera", &["a.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt"]);
    assert!(!own_menu(&mut a).iter().any(|l| l == "Devolver"));
    a.paint();
    assert!(a.panes[0].restore_button().is_none());
    FakeFileOps::take_ops();
    a.run_command(Command::Restore);
    assert_eq!(FakeFileOps::take_ops(), []);
}

// ---------- Crear acceso directo y Mostrar el original ----------

/// Espera el trabajo suelto en curso y se lo entrega a la `App` (lo que haría `WM_APP_TASK`).
fn wait_task(a: &mut App) {
    let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("el trabajo termina");
    a.on_task_done(t);
}

/// Marca una fila como enlace (lo que hacen las cargas de cada sistema con `ATTR_LINK`).
fn mark_link(a: &mut App, name: &str) {
    let list = &mut a.panes[0].tab_mut().list;
    let idx = list.entries.iter().position(|e| e.name == name).unwrap();
    list.entries[idx].attrs |= crate::fs::ATTR_LINK;
}

#[test]
fn create_shortcut_spawns_op() {
    let mut a = app();
    let d = fixture("acceso", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, &["a.txt", "b.txt"]);
    assert!(own_menu(&mut a).iter().any(|l| l == "Crear acceso directo"));
    FakeFileOps::take_ops();
    choose_menu(&mut a, "Crear acceso directo");
    let ops = FakeFileOps::take_ops();
    let [Op::Shortcuts { targets }] = ops.as_slice() else { panic!("{ops:?}") };
    let mut targets = targets.clone();
    targets.sort();
    assert_eq!(targets, [d.join("a.txt"), d.join("b.txt")]);
    let s = crate::commands::ALL.iter().find(|s| s.id == "crear_acceso_directo").unwrap();
    assert_eq!((s.command, s.name, s.default_keys), (Command::CreateShortcut, "Crear acceso directo", &[][..]));
    let s = crate::commands::ALL.iter().find(|s| s.id == "mostrar_original").unwrap();
    assert_eq!((s.command, s.name, s.default_keys), (Command::ShowOriginal, "Mostrar el original", &[][..]));
}

/// «Mostrar el original» solo con un elemento que es enlace.
#[test]
fn show_original_menu_only_for_links() {
    let mut a = app();
    let d = fixture("original-menu", &["enlace", "comun.txt"]);
    go(&mut a, &d);
    mark_link(&mut a, "enlace");
    select(&mut a, &["enlace"]);
    assert!(own_menu(&mut a).iter().any(|l| l == "Mostrar el original"));
    select(&mut a, &["comun.txt"]);
    assert!(!own_menu(&mut a).iter().any(|l| l == "Mostrar el original"));
    select(&mut a, &["enlace", "comun.txt"]);
    assert!(!own_menu(&mut a).iter().any(|l| l == "Mostrar el original"));
}

/// Resuelto el original (en otro hilo), la pestaña activa va a su carpeta y lo enfoca.
#[test]
fn show_original_navigates() {
    let mut a = app();
    let d = fixture("original-va", &["enlace"]);
    let dest = fixture("original-destino", &["x.txt", "real.txt"]);
    FakeShell::set_link_target(&d.join("enlace"), Some(dest.join("real.txt")));
    go(&mut a, &d);
    mark_link(&mut a, "enlace");
    select(&mut a, &["enlace"]);
    a.run_command(Command::ShowOriginal);
    wait_task(&mut a);
    assert_eq!(a.panes[0].tab().location, Location::Path(dest.clone()));
    pump(&mut a, 0);
    assert_eq!(focused_name(&a), "real.txt");
}

/// Sin original (no se resuelve o ya no está): aviso, y la pestaña no se mueve.
#[test]
fn show_original_missing_notice() {
    let mut a = app();
    let d = fixture("original-falta", &["enlace"]);
    FakeShell::set_link_target(&d.join("enlace"), None);
    go(&mut a, &d);
    mark_link(&mut a, "enlace");
    select(&mut a, &["enlace"]);
    a.run_command(Command::ShowOriginal);
    wait_task(&mut a);
    assert_eq!(a.notice.as_deref(), Some("No se encuentra el original de \"enlace\""));
    assert_eq!(a.panes[0].tab().location, Location::Path(d.clone()));
    // Un destino que ya no existe también es «no se encuentra».
    FakeShell::set_link_target(&d.join("enlace"), Some(d.join("borrado.txt")));
    a.notice = None;
    a.run_command(Command::ShowOriginal);
    wait_task(&mut a);
    assert_eq!(a.notice.as_deref(), Some("No se encuentra el original de \"enlace\""));
}

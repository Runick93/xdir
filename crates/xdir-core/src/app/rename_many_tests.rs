//! Pruebas de Renombrar varios en la `App`: el panel de la derecha, la vista previa en la lista,
//! los choques, aplicar y deshacer (en carpetas temporales reales, con `FakeFileSystem`).

use super::preview::RightPanel;
use super::*;
use crate::keys::Key;
use crate::list::Column;
use crate::rename_many::Clash;
use crate::rename_many_draw::{Field, ModeKind, PanelHit};
use crate::sys::testing::{FakeFileOps, FakeHost, FakePreview};
use std::path::Path;
use std::time::Duration;

fn app(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakePreview::set_delay(None);
    let mut a = App::new(Init::blank(), host);
    a.set_layout(Layout::One);
    a
}

fn fixture(name: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-renvarios").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::write(dir.join(f), f.as_bytes()).unwrap();
    }
    dir
}

fn pump(a: &mut App) {
    while a.panes[0].tab().loading {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
    a.refresh_rename_many();
}

fn go(a: &mut App, dir: &Path) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    pump(a);
}

fn select(a: &mut App, names: &[&str]) {
    let list = &mut a.panes[0].tab_mut().list;
    for n in names {
        let pos = (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == *n).expect(n);
        list.set_selected_pos(pos, true);
    }
}

/// La caja del panel para ese campo.
fn field(a: &App, f: Field) -> TextBox {
    a.rename_many.as_ref().unwrap().boxes.iter().find(|(x, _)| *x == f).expect("caja del campo").1
}

fn type_in(a: &mut App, host: &FakeHost, f: Field, text: &str) {
    let b = field(a, f);
    host.text_set(b, text);
    a.on_text_changed(b.0 as isize);
}

fn preview_names(a: &App) -> Vec<(String, Option<Clash>)> {
    let list = &a.panes[0].tab().list;
    // Por nombre viejo: `entries` sigue el orden en que el disco los lista (distinto en Linux).
    let mut v: Vec<(String, Option<Clash>)> =
        list.rename_preview.as_ref().expect("vista previa").iter().map(|(i, n, c)| (format!("{}→{n}", list.entries[*i].display), *c)).collect();
    v.sort_by(|x, y| x.0.cmp(&y.0));
    v
}

/// Espera el renombrado del hilo (y procesa lo que llegue antes).
fn wait_renamed(a: &mut App) {
    loop {
        let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("el renombrado termina");
        let done = matches!(t, tasks::TaskDone::Renamed { .. });
        a.on_task_done(t);
        if done {
            return;
        }
    }
}

fn names_on_disk(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn f2_with_many_opens_panel() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("f2", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    // Con uno: el renombrado en línea, como hoy.
    select(&mut a, &["a.txt"]);
    a.run_command(Command::Rename);
    assert!(a.rename.is_some() && a.rename_many.is_none());
    a.end_rename(false);
    // Con dos: el panel.
    select(&mut a, &["b.txt"]);
    a.run_command(Command::Rename);
    assert!(a.rename.is_none());
    let st = a.rename_many.as_ref().expect("panel abierto");
    assert_eq!(st.items, ["a.txt", "b.txt"]);
    assert_eq!(host.text_boxes_of(TextKind::Field).len(), 2);
    // La paleta también lo abre («renombrar_varios»).
    assert!(crate::commands::ALL.iter().any(|s| s.command == Command::RenameMany && s.id == "renombrar_varios" && s.name == "Renombrar varios…"));
}

#[test]
fn panel_replaces_preview_and_restores_it() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("lugar", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    // Sin la vista previa prendida, el panel igual ocupa el lugar de la derecha.
    a.preview.on = false;
    assert!(a.preview_column().is_none());
    select(&mut a, &["a.txt", "b.txt"]);
    a.run_command(Command::Rename);
    assert_eq!(a.preview.owner, RightPanel::RenameMany);
    let col = a.preview_column().expect("el panel se ve");
    assert_eq!(col.right - col.left, a.preview.width);
    a.paint();
    assert!(host.drawn_texts().iter().any(|(_, t, _)| t == "Renombrar 2 elementos"));
    // Con la vista previa prendida, el panel la reemplaza (no se pide nada) y al cerrar vuelve.
    a.close_rename_many();
    assert!(a.preview_column().is_none());
    a.preview.on = true;
    FakePreview::take_requests();
    a.run_command(Command::Rename);
    a.sync_preview();
    a.flush_preview();
    assert!(FakePreview::take_requests().is_empty());
    a.run_command(Command::Rename);
    a.close_rename_many();
    assert_eq!(a.preview.owner, RightPanel::Preview);
    assert!(a.preview_column().is_some());
    a.paint();
    assert!(!host.drawn_texts().iter().any(|(_, t, _)| t.starts_with("Renombrar ")));
}

#[test]
fn typing_updates_list_preview() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("teclear", &["IMG_1.jpg", "IMG_2.jpg", "otro.txt"]);
    go(&mut a, &dir);
    select(&mut a, &["IMG_1.jpg", "IMG_2.jpg"]);
    a.run_command(Command::Rename);
    type_in(&mut a, &host, Field::Find, "IMG_");
    type_in(&mut a, &host, Field::With, "Viaje-");
    assert_eq!(preview_names(&a), [("IMG_1.jpg→Viaje-1.jpg".to_string(), None), ("IMG_2.jpg→Viaje-2.jpg".to_string(), None)]);
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    // El nombre nuevo y el viejo al lado (tachado: una línea sobre su ancho).
    for want in ["Viaje-1.jpg", "IMG_1.jpg", "Viaje-2.jpg", "2 de 2 cambian"] {
        assert!(texts.iter().any(|t| t == want), "falta {want}: {texts:?}");
    }
    let old = host.drawn_texts().into_iter().find(|(_, t, _)| t == "IMG_1.jpg").unwrap().2;
    let strike_w = 7.0 * "IMG_1.jpg".len() as f32;
    assert!(host.drawn_fills().iter().any(|(r, _)| r.left == old.left && (r.right - r.left - strike_w).abs() < 0.01 && r.bottom - r.top == 1.0));
    // «otro.txt» no se renombra.
    assert!(texts.iter().any(|t| t == "otro.txt"));
}

#[test]
fn clash_marks_row_and_disables_apply() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("choque", &["IMG_1.jpg", "IMG_2.jpg", "V-1.jpg"]);
    go(&mut a, &dir);
    select(&mut a, &["IMG_1.jpg", "IMG_2.jpg"]);
    a.run_command(Command::Rename);
    type_in(&mut a, &host, Field::Find, "IMG_");
    type_in(&mut a, &host, Field::With, "V-");
    assert_eq!(preview_names(&a), [("IMG_1.jpg→V-1.jpg".to_string(), Some(Clash::Exists)), ("IMG_2.jpg→V-2.jpg".to_string(), None)]);
    let st = a.rename_panel_state().unwrap();
    assert!(!st.can_apply);
    assert_eq!(st.clash.as_deref(), Some("1 choque: «V-1.jpg» ya existe"));
    // El nombre nuevo del choque se dibuja (en rojo; el cartel lo prueba `clash_tooltip_says_why`).
    a.paint();
    assert!(host.drawn_texts().iter().any(|(_, t, _)| t == "V-1.jpg"));
    // Renombrar está apagado: no corre nada.
    a.on_field_done(true);
    assert!(a.rename_many.is_some());
    assert_eq!(names_on_disk(&dir), ["IMG_1.jpg", "IMG_2.jpg", "V-1.jpg"]);
    assert_eq!(host.beeps(), 1);
}

#[test]
fn clash_tooltip_says_why() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("cartel", &["IMG_1.jpg", "IMG_2.jpg", "V-1.jpg"]);
    go(&mut a, &dir);
    select(&mut a, &["IMG_1.jpg", "IMG_2.jpg"]);
    a.run_command(Command::Rename);
    type_in(&mut a, &host, Field::Find, "IMG_");
    type_in(&mut a, &host, Field::With, "V-");
    a.paint();
    // El mouse sobre la fila de «IMG_1.jpg» (la primera).
    let r = a.panes[0].list;
    let (x, y) = (r.left + 60.0, r.top + crate::list::HEADER_H + crate::list::ROW_H / 2.0 + 2.0);
    a.on_mouse_move(x, y);
    assert_eq!(a.tooltip_text(Hit::Pane(0, PaneHit::List)).as_deref(), Some("Ya existe «V-1.jpg» en esta carpeta"));
}

#[test]
fn numbering_follows_visible_order() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("orden", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &dir);
    // Orden por nombre al revés: c, b, a.
    a.panes[0].tab_mut().list.set_sort(Column::Name);
    if !a.panes[0].tab().list.descending {
        a.panes[0].tab_mut().list.set_sort(Column::Name);
    }
    select(&mut a, &["a.txt", "b.txt", "c.txt"]);
    a.run_command(Command::Rename);
    a.rename_panel_click(PanelHit::Mode(ModeKind::Format));
    type_in(&mut a, &host, Field::Base, "foto");
    let mut got = preview_names(&a);
    got.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(got, [("a.txt→foto 3.txt".to_string(), None), ("b.txt→foto 2.txt".to_string(), None), ("c.txt→foto 1.txt".to_string(), None)]);
}

#[test]
fn apply_renames_and_one_undo() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("aplicar", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &dir);
    select(&mut a, &["a.txt", "b.txt"]);
    a.run_command(Command::Rename);
    a.rename_panel_click(PanelHit::Mode(ModeKind::Add));
    type_in(&mut a, &host, Field::Text, "x-");
    a.rename_panel_click(PanelHit::Apply);
    assert!(a.rename_many.is_none());
    wait_renamed(&mut a);
    assert_eq!(names_on_disk(&dir), ["c.txt", "x-a.txt", "x-b.txt"]);
    assert!(matches!(a.undo.last(), Some(Undo::Renames(v)) if v.len() == 2));
    // Ctrl+Z: todos vuelven en un paso, sin pasar por `FileOps`.
    FakeFileOps::take_ops();
    a.run_command(Command::Undo);
    wait_renamed(&mut a);
    assert_eq!(names_on_disk(&dir), ["a.txt", "b.txt", "c.txt"]);
    assert!(a.undo.is_empty());
    assert!(FakeFileOps::take_ops().is_empty());
}

#[test]
fn apply_failure_rolls_back_and_reports() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("falla", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    crate::sys::testing::FakeFileSystem::fail_rename_to(&dir.join("x-b.txt"));
    select(&mut a, &["a.txt", "b.txt"]);
    a.run_command(Command::Rename);
    a.rename_panel_click(PanelHit::Mode(ModeKind::Add));
    type_in(&mut a, &host, Field::Text, "x-");
    a.rename_panel_click(PanelHit::Apply);
    wait_renamed(&mut a);
    assert_eq!(names_on_disk(&dir), ["a.txt", "b.txt"]);
    assert!(a.undo.is_empty());
    assert!(a.notice.as_deref().is_some_and(|n| n.starts_with("No se pudo renombrar")));
}

#[test]
fn escape_cancels_and_clears_preview() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("esc", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    select(&mut a, &["a.txt", "b.txt"]);
    a.run_command(Command::Rename);
    type_in(&mut a, &host, Field::Find, "a");
    assert!(a.panes[0].tab().list.rename_preview.is_some());
    a.on_key(Key(0x1B));
    assert!(a.rename_many.is_none());
    assert!(a.panes[0].tab().list.rename_preview.is_none());
    assert!(host.text_boxes_of(TextKind::Field).is_empty());
    // Escape en una caja del panel también lo cierra.
    a.run_command(Command::Rename);
    a.on_app_message(WM_APP_FIELD_DONE, 0, 0);
    assert!(a.rename_many.is_none());
    assert_eq!(names_on_disk(&dir), ["a.txt", "b.txt"]);
}

#[test]
fn reload_drops_vanished_items() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("recarga", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &dir);
    select(&mut a, &["a.txt", "b.txt", "c.txt"]);
    a.run_command(Command::Rename);
    std::fs::remove_file(dir.join("b.txt")).unwrap();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().reload(hwnd, &tx, true);
    // La recarga silenciosa cambia la lista cuando termina de leer.
    while a.panes[0].tab().list.entries.iter().any(|e| e.name == "b.txt") {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la recarga termina");
        a.on_load_event(ev);
    }
    a.refresh_rename_many();
    assert_eq!(a.rename_many.as_ref().unwrap().items, ["a.txt", "c.txt"]);
    assert_eq!(a.rename_panel_state().unwrap().count, 2);
}

#[test]
fn rename_many_menu_label() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let dir = fixture("menu", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &dir);
    let labels = |a: &App| {
        let mut cmds = Vec::new();
        a.item_entries(&mut cmds)
            .into_iter()
            .filter_map(|e| match e {
                shell::Entry::Item { label, .. } => Some(label),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    select(&mut a, &["a.txt"]);
    assert!(!labels(&a).iter().any(|l| l.starts_with("Renombrar")));
    select(&mut a, &["b.txt", "c.txt"]);
    assert!(labels(&a).iter().any(|l| l == "Renombrar 3 elementos…"));
}

//! Pruebas de la vista rápida (barra espaciadora) en la `App`.

use super::*;
use crate::keys::keys::{VK_LEFT, VK_RIGHT, VK_SPACE};
use crate::list::Column;
use crate::quicklook::{self, QuickLookItem};
use crate::preview::{PreviewBody, PreviewInfo, PreviewTarget, WM_APP_PREVIEW};
use crate::sys::testing::{FakeEnv, FakeFileSystem, FakeHost, FakePreview, FakeShell, ShellCall};
use std::path::Path;
use std::time::Duration;

fn app_with_host(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakeEnv::hold_keys(&[]);
    FakeShell::take_calls();
    FakePreview::set_delay(None);
    FakePreview::take_requests();
    App::new(Init::blank(), host)
}

/// Lo que llegó de `sys::preview()` (lo que hace `WM_APP_PREVIEW` en la ventana): la respuesta
/// de nube de la vista rápida.
fn deliver(a: &mut App) {
    a.on_app_message(WM_APP_PREVIEW, 0, 0);
}

/// Una tecla, y después las respuestas que haya (el falso contesta enseguida).
fn press(a: &mut App, vk: crate::keys::Key) -> bool {
    let used = a.on_key(vk);
    deliver(a);
    used
}

/// Carpeta temporal propia de la prueba con esos archivos.
fn fixture(name: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-quicklook").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::write(dir.join(f), f.as_bytes()).unwrap();
    }
    dir
}

fn pump(a: &mut App, pane: usize) {
    while a.panes[pane].tab().loading {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
    deliver(a);
}

fn go(a: &mut App, dir: &Path) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    pump(a, 0);
}

/// Vuelve a leer la carpeta del panel 0 (como cuando el vigilante avisa un cambio).
fn reload(a: &mut App) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().reload(hwnd, &tx, false);
    pump(a, 0);
}

fn pos_of(a: &App, name: &str) -> usize {
    let list = &a.panes[0].tab().list;
    (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == name).expect(name)
}

fn select(a: &mut App, name: &str) {
    let p = pos_of(a, name);
    a.panes[0].tab_mut().list.select_pos(p);
}

fn focused_name(a: &App) -> String {
    let list = &a.panes[0].tab().list;
    list.focused_index().map(|i| list.entries[i].name.to_string_lossy().into_owned()).unwrap_or_default()
}

fn selected_names(a: &App) -> Vec<String> {
    let list = &a.panes[0].tab().list;
    list.selected_positions().into_iter().map(|p| list.entries[list.entry_of_pos(p).unwrap()].name.to_string_lossy().into_owned()).collect()
}

/// Lo último que se le pidió mostrar al host: nombre y «k de n».
fn last_shown(host: &FakeHost) -> (String, String) {
    let item: QuickLookItem = host.quick_look_shown().last().cloned().expect("se mostró algo");
    (item.name.clone(), quicklook::position_text(item.index, item.total))
}

fn twelve() -> Vec<String> {
    (1..=12).map(|i| format!("f{i:02}.txt")).collect()
}

#[test]
fn quick_look_command_and_menu() {
    use crate::commands::{ALL, Keymap, parse_combo};
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let s = ALL.iter().find(|s| s.id == "vista_rapida").expect("vista_rapida");
    assert_eq!((s.command, s.name, s.default_keys), (Command::QuickLook, "Vista rápida", &["Espacio"][..]));
    let c = parse_combo("Espacio").unwrap();
    assert_eq!(Keymap::default().lookup(crate::keys::Key(c.vk), false, false, false), Some(Command::QuickLook));
    assert_eq!(Keymap::default().label(Command::QuickLook), "Espacio");

    let d = fixture("menu", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, "b.txt");
    FakeShell::choose(None);
    FakeShell::take_calls();
    a.on_context_menu(Some((10.0, 20.0)));
    let calls = FakeShell::take_calls();
    let [ShellCall::ContextMenu { own, .. }] = calls.as_slice() else { panic!("se esperaba un menú: {calls:?}") };
    let shell::Entry::Item { id, label, .. } = &own[0] else { panic!("{own:?}") };
    assert_eq!(label, "Vista rápida");
    FakeShell::choose(Some(&format!("own:{id}")));
    a.on_context_menu(Some((10.0, 20.0)));
    deliver(&mut a);
    FakeShell::choose(None);
    assert_eq!(last_shown(&host), ("b.txt".to_string(), "2 de 2".to_string()));
}

#[test]
fn space_opens_quick_look() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("abre", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    assert!(press(&mut a, VK_SPACE));
    assert_eq!(last_shown(&host), ("a.txt".to_string(), "1 de 3".to_string()));
    let item = host.quick_look_shown().last().cloned().unwrap();
    assert_eq!(item.path.as_deref(), Some(d.join("a.txt").as_path()));
    assert!(!item.cloud_only);
}

#[test]
fn space_during_typeahead_types() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("tipeo", &["mi a.txt", "mi b.txt", "otro.txt"]);
    go(&mut a, &d);
    select(&mut a, "otro.txt");
    a.on_char('m');
    a.on_char('i');
    assert!(!press(&mut a, VK_SPACE), "durante la búsqueda por tipeo el espacio es parte del nombre");
    assert!(host.quick_look_shown().is_empty());
    a.on_char(' ');
    a.on_char('b');
    assert_eq!(focused_name(&a), "mi b.txt");
}

#[test]
fn space_on_group_header_folds() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("encabezado", &["a1.txt", "a2.txt", "b1.txt"]);
    go(&mut a, &d);
    a.panes[0].tab_mut().list.set_group(Some(Column::Name));
    select(&mut a, "a1.txt");
    // ← desde un elemento (Detalles) lleva al encabezado de su grupo.
    press(&mut a, VK_LEFT);
    assert!(a.panes[0].tab().list.header_focused());
    let before = a.panes[0].tab().list.count();
    assert!(press(&mut a, VK_SPACE));
    assert!(host.quick_look_shown().is_empty());
    assert!(a.panes[0].tab().list.count() < before, "el grupo se plegó");
}

#[test]
fn ctrl_space_unchanged() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("ctrl", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    FakeEnv::hold_keys(&[VK_CONTROL]);
    press(&mut a, VK_SPACE);
    FakeEnv::hold_keys(&[]);
    assert!(host.quick_look_shown().is_empty());
    assert!(selected_names(&a).is_empty(), "Ctrl+Espacio alterna la selección del enfocado");
}

#[test]
fn space_char_swallowed_after_open() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("traga", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    assert!(press(&mut a, VK_SPACE));
    // El carácter que llega después de la tecla (WM_CHAR) no entra en la búsqueda por tipeo.
    a.on_char(' ');
    assert!(press(&mut a, VK_ESCAPE));
    a.on_char('c');
    assert_eq!(focused_name(&a), "c.txt");
}

#[test]
fn esc_and_space_close() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("cierra", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    assert!(press(&mut a, VK_SPACE));
    assert!(a.quick_look.is_some());
    assert!(press(&mut a, VK_ESCAPE));
    assert!(a.quick_look.is_none());
    assert_eq!(host.quick_look_closes(), 1);
    assert!(press(&mut a, VK_SPACE));
    assert!(a.quick_look.is_some());
    assert!(press(&mut a, VK_SPACE));
    assert!(a.quick_look.is_none());
    assert_eq!(host.quick_look_closes(), 2);
    // El sistema cerró el panel por su cuenta: la `App` se entera sin volver a cerrarlo.
    a.on_char(' ');
    press(&mut a, VK_SPACE);
    a.on_quick_look_closed();
    assert!(a.quick_look.is_none());
    assert_eq!(host.quick_look_closes(), 2);
}

#[test]
fn quick_look_arrows_walk_visible_order() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("orden", &["a1.txt", "a2.jpg", "a3.txt", "b1.txt", ".oculto.txt", "c1.txt"]);
    go(&mut a, &d);
    {
        let list = &mut a.panes[0].tab_mut().list;
        list.set_group(Some(Column::Name));
        list.set_filter(Column::Extension, vec!["TXT".to_string()]);
    }
    select(&mut a, "a3.txt");
    assert!(press(&mut a, VK_SPACE));
    assert_eq!(last_shown(&host), ("a3.txt".to_string(), "2 de 4".to_string()));
    press(&mut a, VK_RIGHT);
    assert_eq!(last_shown(&host), ("b1.txt".to_string(), "3 de 4".to_string()));
    assert_eq!(selected_names(&a), ["b1.txt"], "la lista acompaña");
    press(&mut a, VK_DOWN);
    assert_eq!(last_shown(&host).0, "c1.txt");
    let n = host.quick_look_shown().len();
    press(&mut a, VK_RIGHT);
    assert_eq!(host.quick_look_shown().len(), n, "no da la vuelta");
    press(&mut a, VK_UP);
    press(&mut a, VK_LEFT);
    press(&mut a, VK_LEFT);
    assert_eq!(last_shown(&host), ("a1.txt".to_string(), "1 de 4".to_string()));
    assert_eq!(selected_names(&a), ["a1.txt"]);
}

#[test]
fn quick_look_walks_selection_only() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("seleccion", &["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"]);
    go(&mut a, &d);
    for n in ["a.txt", "c.txt", "e.txt"] {
        let p = pos_of(&a, n);
        a.panes[0].tab_mut().list.set_selected_pos(p, true);
    }
    let p = pos_of(&a, "a.txt");
    a.panes[0].tab_mut().list.set_selected_pos(p, true);
    assert!(press(&mut a, VK_SPACE));
    assert_eq!(last_shown(&host), ("a.txt".to_string(), "1 de 3".to_string()));
    press(&mut a, VK_RIGHT);
    assert_eq!(last_shown(&host), ("c.txt".to_string(), "2 de 3".to_string()));
    assert_eq!(focused_name(&a), "c.txt");
    press(&mut a, VK_RIGHT);
    assert_eq!(last_shown(&host), ("e.txt".to_string(), "3 de 3".to_string()));
    let n = host.quick_look_shown().len();
    press(&mut a, VK_RIGHT);
    assert_eq!(host.quick_look_shown().len(), n, "no da la vuelta");
    assert_eq!(selected_names(&a), ["a.txt", "c.txt", "e.txt"], "la selección no cambia");
    assert_eq!(focused_name(&a), "e.txt");
}

#[test]
fn quick_look_item_deleted_moves_next() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("borrado", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &d);
    select(&mut a, "b.txt");
    assert!(press(&mut a, VK_SPACE));
    assert_eq!(last_shown(&host), ("b.txt".to_string(), "2 de 3".to_string()));
    std::fs::remove_file(d.join("b.txt")).unwrap();
    reload(&mut a);
    assert!(a.quick_look.is_some());
    assert_eq!(last_shown(&host), ("c.txt".to_string(), "2 de 2".to_string()));
    // El último que queda también se va: el que quedó en su lugar es el anterior.
    std::fs::remove_file(d.join("c.txt")).unwrap();
    reload(&mut a);
    assert_eq!(last_shown(&host), ("a.txt".to_string(), "1 de 1".to_string()));
}

#[test]
fn quick_look_folder_emptied_closes() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("vacia", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    assert!(press(&mut a, VK_SPACE));
    std::fs::remove_file(d.join("a.txt")).unwrap();
    std::fs::remove_file(d.join("b.txt")).unwrap();
    reload(&mut a);
    assert!(a.quick_look.is_none());
    assert_eq!(host.quick_look_closes(), 1);
    // Sin vista rápida, las flechas vuelven a ser de la lista (sin pánico con la lista vacía).
    press(&mut a, VK_RIGHT);
}

#[test]
fn quick_look_cloud_item_uses_card() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("nube", &["a.jpg", "b.jpg"]);
    go(&mut a, &d);
    FakeFileSystem::set_cloud_only(&d.join("b.jpg"), true);
    select(&mut a, "b.jpg");
    assert!(press(&mut a, VK_SPACE));
    assert!(host.quick_look_shown().is_empty(), "lo solo en la nube no va al visor del sistema");
    a.paint();
    let texts = host.drawn_texts();
    assert!(texts.iter().any(|t| t.1 == "Disponible solo en línea"), "{texts:?}");
    assert!(texts.iter().any(|t| t.1 == "2 de 2"));
    assert!(host.drawn_big().iter().all(|k| k.0 != crate::thumbs::Source::Path(d.join("b.jpg"))), "no se pide su imagen");
    // ← pasa a uno local: ese sí va al sistema y la tarjeta se va.
    press(&mut a, VK_LEFT);
    assert_eq!(last_shown(&host).0, "a.jpg");
    a.paint();
    assert!(!host.drawn_texts().iter().any(|t| t.1 == "Disponible solo en línea"));
    FakeFileSystem::set_cloud_only(&d.join("b.jpg"), false);
}

/// Un host cuya ventana propia muestra también lo solo en la nube (Windows: la tarjeta va en su
/// ventana, con «Disponible solo en línea»): recibe el elemento marcado `cloud_only` y la `App` no
/// dibuja su tarjeta ni cierra el visor.
#[test]
fn quick_look_cloud_item_to_host_window() {
    let host = FakeHost::new();
    host.set_quick_look_shows_cloud(true);
    let mut a = app_with_host(host.clone());
    let d = fixture("nube-ventana", &["a.jpg", "b.jpg"]);
    go(&mut a, &d);
    FakeFileSystem::set_cloud_only(&d.join("b.jpg"), true);
    select(&mut a, "a.jpg");
    assert!(press(&mut a, VK_SPACE));
    assert!(!host.quick_look_shown().last().unwrap().cloud_only, "a.jpg es local");
    press(&mut a, VK_RIGHT);
    let shown = host.quick_look_shown();
    let last = shown.last().unwrap();
    assert_eq!((last.name.as_str(), last.cloud_only), ("b.jpg", true));
    assert_eq!(host.quick_look_closes(), 0, "el visor sigue abierto");
    a.paint();
    assert!(!host.drawn_texts().iter().any(|t| t.1 == "Disponible solo en línea"), "la tarjeta la dibuja el host");
    FakeFileSystem::set_cloud_only(&d.join("b.jpg"), false);
}

#[test]
fn quick_look_overlay_when_host_has_none() {
    let host = FakeHost::new();
    host.set_quick_look_native(false);
    let mut a = app_with_host(host.clone());
    let names = twelve();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let d = fixture("propia", &refs);
    go(&mut a, &d);
    select(&mut a, "f02.txt");
    assert!(press(&mut a, VK_SPACE));
    a.paint();
    let texts = host.drawn_texts();
    assert!(texts.iter().any(|t| t.1 == "f02.txt"));
    assert!(texts.iter().any(|t| t.1 == "2 de 12"));
    assert!(texts.iter().any(|t| t.1 == "Abrir"));
    assert!(host.drawn_big().iter().any(|k| k.0 == crate::thumbs::Source::Path(d.join("f02.txt"))), "la imagen grande");
    // Clic en «Abrir» de la barra de título: lo abre con su programa.
    let (w, h) = host.client.get();
    let card = quicklook::overlay_rect(w, h);
    let bar = geom::rect(card.left, card.top, card.right, card.top + quicklook::TITLE_H);
    let (x, y) = (bar.right - 80.0, bar.top + 20.0);
    assert_eq!(quicklook::title_hit(bar, x, y), Some(quicklook::TitleHit::Open));
    FakeShell::take_calls();
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert_eq!(FakeShell::take_calls(), [ShellCall::OpenItem(d.join("f02.txt"))]);
    // ‹ y la cruz.
    let (px, py) = (bar.right - 214.0, bar.top + 20.0);
    assert_eq!(quicklook::title_hit(bar, px, py), Some(quicklook::TitleHit::Prev));
    a.on_left_down(px, py, false);
    a.on_left_up(px, py);
    deliver(&mut a);
    assert_eq!(focused_name(&a), "f01.txt");
    let (cx, cy) = (bar.right - 22.0, bar.top + 20.0);
    a.on_left_down(cx, cy, false);
    a.on_left_up(cx, cy);
    assert!(a.quick_look.is_none());
}

/// Se pregunta si está en la nube fuera del hilo de interfaz, y al host recién se le pasa el
/// elemento con la respuesta.
#[test]
fn quick_look_waits_for_cloud_reply() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("espera", &["a.jpg", "b.jpg"]);
    go(&mut a, &d);
    select(&mut a, "a.jpg");
    FakePreview::take_requests();
    assert!(a.on_key(VK_SPACE));
    assert!(host.quick_look_shown().is_empty(), "sin respuesta no va al visor");
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::Cloud(d.join("a.jpg"))]);
    deliver(&mut a);
    assert_eq!(last_shown(&host).0, "a.jpg");
    // Una respuesta «solo en la nube» lleva a la tarjeta de xdir y el visor se cierra.
    FakePreview::serve(&d.join("b.jpg"), PreviewInfo::new(PreviewBody::CloudOnly));
    a.on_key(VK_RIGHT);
    assert_eq!(host.quick_look_shown().len(), 1);
    deliver(&mut a);
    assert_eq!(host.quick_look_shown().len(), 1, "lo de la nube no va al visor");
    assert_eq!(host.quick_look_closes(), 1);
    a.paint();
    assert!(host.drawn_texts().iter().any(|t| t.1 == "Disponible solo en línea"));
    // Volver a uno ya preguntado no vuelve a preguntar.
    a.on_key(VK_LEFT);
    assert!(FakePreview::take_requests().iter().all(|t| *t != PreviewTarget::Cloud(d.join("a.jpg"))));
    assert_eq!(last_shown(&host).0, "a.jpg");
}

/// Dibujar la tarjeta nunca pregunta al sistema de archivos.
#[test]
fn quick_look_paint_never_checks_cloud() {
    let host = FakeHost::new();
    host.set_quick_look_native(false);
    let mut a = app_with_host(host.clone());
    let d = fixture("dibujo", &["a.jpg", "b.jpg"]);
    go(&mut a, &d);
    select(&mut a, "a.jpg");
    press(&mut a, VK_SPACE);
    let before = FakeFileSystem::cloud_checks();
    for _ in 0..5 {
        a.paint();
    }
    assert_eq!(FakeFileSystem::cloud_checks(), before);
    assert!(host.drawn_texts().iter().any(|t| t.1 == "1 de 2"));
}

/// Si la columna de vista previa ya contestó por el mismo archivo, no se vuelve a preguntar.
#[test]
fn quick_look_reuses_preview_column_reply() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("columna", &["a.jpg", "b.jpg"]);
    go(&mut a, &d);
    a.preview.on = true;
    select(&mut a, "a.jpg");
    a.paint();
    a.flush_preview();
    deliver(&mut a);
    FakePreview::take_requests();
    assert!(a.on_key(VK_SPACE));
    assert!(FakePreview::take_requests().is_empty());
    assert_eq!(last_shown(&host).0, "a.jpg");
}

#[test]
fn space_opens_quick_look_in_gallery() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("galeria", &["a.jpg", "b.jpg", "c.jpg"]);
    go(&mut a, &d);
    a.set_view(0, list::View::Gallery);
    select(&mut a, "a.jpg");
    assert!(press(&mut a, VK_SPACE));
    assert_eq!(last_shown(&host), ("a.jpg".to_string(), "1 de 3".to_string()));
    press(&mut a, VK_RIGHT);
    assert_eq!(last_shown(&host), ("b.jpg".to_string(), "2 de 3".to_string()));
    assert_eq!(focused_name(&a), "b.jpg", "la tira acompaña");
}

/// Ordenar con la vista rápida abierta rearma el recorrido (sin recargar la carpeta).
#[test]
fn quick_look_follows_sort_change() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("orden-cambia", &["a.txt", "b.txt", "c.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    press(&mut a, VK_SPACE);
    a.run_command(Command::SortBy(0));
    deliver(&mut a);
    assert_eq!(last_shown(&host), ("a.txt".to_string(), "3 de 3".to_string()));
    press(&mut a, VK_LEFT);
    assert_eq!(last_shown(&host).0, "b.txt");
}

/// Abrir xtree (pantalla completa) cierra la vista rápida: quedaría encima de la vista.
#[test]
fn xtree_closes_quick_look() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let d = fixture("xtree-cierra", &["a.txt", "b.txt"]);
    go(&mut a, &d);
    select(&mut a, "a.txt");
    assert!(press(&mut a, VK_SPACE));
    assert!(a.quick_look.is_some());
    a.toggle_xtree();
    assert!(a.quick_look.is_none());
    assert_eq!(host.quick_look_closes(), 1);
}

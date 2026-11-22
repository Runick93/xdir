//! Pruebas de Configuración ▸ Lugares: la página cambia los lugares de la `App`, el árbol y
//! `lugares.toml` (en la carpeta de configuración de prueba de cada hilo).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::PlaceHit;
use crate::app::{App, Init};
use crate::places::Place;
use crate::settings::PageId;
use crate::sys::testing::{FakeFileSystem, FakeHost, FakeShell, ShellCall, install_fakes};

/// Carpeta temporal propia de la prueba.
fn temp_root(name: &str) -> PathBuf {
    install_fakes();
    let dir = std::env::temp_dir().join(format!("xdir-lugares-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::sys::file_system().true_case(&dir)
}

/// App con las seis carpetas del usuario y la Papelera bajo `root`, el árbol a la vista y la
/// configuración abierta en Lugares.
fn places_app(root: &Path) -> (App, Rc<FakeHost>) {
    install_fakes();
    let names = ["escritorio", "descargas", "documentos", "imagenes", "musica", "videos"];
    let paths: Vec<PathBuf> = names.iter().map(|n| root.join(n)).collect();
    for p in &paths {
        std::fs::create_dir_all(p).unwrap();
    }
    FakeFileSystem::set_user_folders(&paths.iter().map(PathBuf::as_path).collect::<Vec<_>>());
    FakeFileSystem::set_trash(Some(&root.join(".Trash")));
    let host = FakeHost::new();
    host.client.set((1600.0, 1200.0));
    let mut a = App::new(Init::blank(), host.clone());
    a.set_layout(crate::ui::Layout::One);
    a.tree.visible = true;
    a.open_settings(Some(PageId::Extra("lugares")));
    a.paint();
    (a, host)
}

fn texts(host: &FakeHost) -> Vec<String> {
    host.drawn_texts().into_iter().map(|t| t.1).collect()
}

fn cleanup(root: &Path) {
    FakeFileSystem::set_trash(None);
    FakeFileSystem::set_user_folders(&[]);
    let _ = std::fs::remove_dir_all(root);
}

fn click(a: &mut App, hit: PlaceHit) {
    let (r, _) = *a.places_hits.iter().find(|(_, h)| *h == hit).unwrap_or_else(|| panic!("{hit:?}: {:?}", a.places_hits));
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    a.paint();
}

/// Espera a que `lugares.toml` cumpla `ok` (se guarda en otro hilo).
fn wait_file(ok: impl Fn(&str) -> bool) {
    let file = crate::session::config_dir().unwrap().join("lugares.toml");
    let t0 = Instant::now();
    while !std::fs::read_to_string(&file).is_ok_and(|t| ok(&t)) {
        assert!(t0.elapsed() < Duration::from_secs(10), "lugares.toml no se guardó: {:?}", std::fs::read_to_string(&file));
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn index_of(a: &App, place: &Place) -> usize {
    a.places.order.iter().position(|p| p == place).unwrap()
}

fn row_y(a: &App, name: &str) -> Option<f32> {
    a.tree.row_point(name).map(|(_, y)| y)
}

#[test]
fn places_page_toggle_hides_in_tree() {
    let root = temp_root("ocultar");
    let (mut a, _host) = places_app(&root);
    assert!(row_y(&a, "Música").is_some());
    let i = index_of(&a, &Place::User(4));
    click(&mut a, PlaceHit::Toggle(i));
    assert!(a.places.is_hidden(&Place::User(4)));
    assert!(row_y(&a, "Música").is_none(), "Música sigue en el árbol");
    wait_file(|t| t.contains("ocultos = [\"musica\"]"));
    // Otra vez: vuelve a verse.
    click(&mut a, PlaceHit::Toggle(i));
    assert!(row_y(&a, "Música").is_some());
    wait_file(|t| t.contains("ocultos = []"));
    cleanup(&root);
}

#[test]
fn places_page_move_up_reorders_tree() {
    let root = temp_root("subir");
    let (mut a, _host) = places_app(&root);
    let i = index_of(&a, &Place::Trash);
    assert!(row_y(&a, "Papelera") > row_y(&a, "Vídeos"));
    click(&mut a, PlaceHit::Up(i));
    assert_eq!(a.places.order[i - 1], Place::Trash);
    assert!(row_y(&a, "Papelera") < row_y(&a, "Vídeos"), "la Papelera no subió en el árbol");
    wait_file(|t| t.contains("\"papelera\", \"videos\""));
    // ↓ la devuelve.
    click(&mut a, PlaceHit::Down(i - 1));
    assert_eq!(a.places.order[i], Place::Trash);
    assert!(row_y(&a, "Papelera") > row_y(&a, "Vídeos"));
    cleanup(&root);
}

#[test]
fn places_page_add_folder_appears() {
    let root = temp_root("agregar");
    let own = root.join("proyectos-propios");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, host) = places_app(&root);
    FakeShell::take_calls();
    FakeShell::pick(Some(own.clone()));
    click(&mut a, PlaceHit::Add);
    FakeShell::pick(None);
    assert!(FakeShell::take_calls().contains(&ShellCall::PickFolder("Agregar carpeta a Lugares".into())));
    assert_eq!(a.places.order.last(), Some(&Place::Folder(own.clone())));
    assert!(row_y(&a, "proyectos-propios").is_some(), "la carpeta no está en el árbol");
    // La página la muestra con su ruta.
    let texts = texts(&host);
    let full = own.to_string_lossy().into_owned();
    let path_shown = |t: &String| t.len() > 10 && full.starts_with(t.trim_end_matches('…'));
    assert!(texts.iter().any(|t| t == "proyectos-propios") && texts.iter().any(path_shown), "{texts:?}");
    let saved = own.to_string_lossy().replace('\\', "\\\\");
    wait_file(|t| t.contains(&saved));
    // «Quitar» la saca de la lista y del árbol.
    let i = index_of(&a, &Place::Folder(own.clone()));
    click(&mut a, PlaceHit::Remove(i));
    assert!(!a.places.order.contains(&Place::Folder(own)));
    assert!(row_y(&a, "proyectos-propios").is_none());
    // Sin elegir nada no cambia.
    let before = a.places.clone();
    click(&mut a, PlaceHit::Add);
    assert_eq!(a.places, before);
    cleanup(&root);
}

#[test]
fn places_page_search_finds_trash() {
    let root = temp_root("buscar");
    let (a, _host) = places_app(&root);
    for q in ["papelera", "lugares", "acceso rápido", "escritorio", "favoritos", "árbol"] {
        let (counts, _) = a.settings_results(q, crate::app::settings_pages::PAGES);
        assert!(counts.iter().any(|(p, n)| *p == PageId::Extra("lugares") && *n > 0), "{q}: {counts:?}");
    }
    // Va después de Árbol y el ancla de la sesión la recuerda.
    let nav = a.settings_nav();
    let tree = nav.iter().position(|p| *p == PageId::Builtin(crate::settings::Section::Tree)).unwrap();
    assert_eq!(nav[tree + 1], PageId::Extra("lugares"));
    assert_eq!(a.session().settings_page.as_deref(), Some("lugares"));
    cleanup(&root);
}

#[test]
fn places_page_rows_and_quick_access_only_on_windows() {
    let root = temp_root("filas");
    let (a, host) = places_app(&root);
    let texts = texts(&host);
    for t in ["Escritorio", "Descargas", "Documentos", "Imágenes", "Música", "Vídeos", "Papelera", "Agregar carpeta…"] {
        assert!(texts.iter().any(|x| x == t), "{t}: {texts:?}");
    }
    // El falso no tiene Acceso rápido (como la Mac y Linux): la fila no aparece.
    assert!(!texts.iter().any(|x| x == "Acceso rápido de Windows"));
    assert!(!a.places_hits.iter().any(|(_, h)| *h == PlaceHit::Toggle(index_of(&a, &Place::QuickAccess))));
    cleanup(&root);
}

/// En una ventana angosta los botones no tapan el nombre: «Quitar» y la ruta se van primero y el
/// nombre siempre tiene su lugar.
#[test]
fn places_page_narrow_keeps_names() {
    let root = temp_root("angosta");
    let own = root.join("proyectos-propios");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, _host) = places_app(&root);
    a.places.add_folder(own.clone());
    let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
    let left = 10.0;
    super::draw(&mut a, &p, crate::geom::rect(left, 0.0, left + 260.0, 2000.0), None);
    let floor = left + 52.0 + super::MIN_NAME_W;
    for (r, hit) in &a.places_hits {
        if *hit != PlaceHit::Add {
            assert!(r.left >= floor, "{hit:?} en {r:?} pisa el nombre");
        }
    }
    assert!(!a.places_hits.iter().any(|(_, h)| matches!(h, PlaceHit::Remove(_))), "sin lugar, «Quitar» no se muestra");
    let texts = p.texts();
    for name in ["Escritorio", "Papelera"] {
        assert!(texts.iter().any(|(_, s, _)| s == name), "{name}: {texts:?}");
    }
    let shown = texts.iter().find(|(_, s, _)| "proyectos-propios".starts_with(s.trim_end_matches('…')) && s.len() > 3);
    assert!(shown.is_some(), "{texts:?}");
    // Ancho: aparecen «Quitar» y la ruta.
    super::draw(&mut a, &p, crate::geom::rect(left, 0.0, left + 820.0, 2000.0), None);
    assert!(a.places_hits.iter().any(|(_, h)| matches!(h, PlaceHit::Remove(_))));
    cleanup(&root);
}

// ---------- soltar entre los lugares del árbol ----------

fn drag_dirs(a: &mut App, kind: crate::drop::DragKind, point: (f32, f32), paths: &[PathBuf]) -> crate::drop::Effect {
    use crate::drop::{DragEvent, Effect};
    let mut ev = DragEvent { kind, point, ctrl: false, shift: false, paths: Some(paths), allowed: Effect::ALL, effect: Effect::NONE };
    a.on_drag(&mut ev);
    ev.effect
}

/// El borde de arriba de la fila del árbol con ese nombre (donde va la línea de inserción).
fn row_top_edge(a: &App, name: &str) -> (f32, f32) {
    let (x, y) = a.tree.row_point(name).unwrap_or_else(|| panic!("la fila {name}"));
    (x, y - 12.0)
}

/// Mac y Linux (sin Acceso rápido): soltar una carpeta entre dos lugares la agrega ahí como
/// carpeta propia, como la barra lateral del Finder; no se copia ni se mueve nada.
#[test]
fn dropping_a_folder_between_places_adds_it_there() {
    use crate::drop::{DragKind, Effect};
    let root = temp_root("soltar-entre");
    let own = root.join("proyectos");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, _host) = places_app(&root);
    let at = row_top_edge(&a, "Descargas");
    let paths = vec![own.clone()];
    assert_eq!(drag_dirs(&mut a, DragKind::Enter, at, &paths), Effect::LINK);
    assert!(a.tree.place_drop.is_some(), "se ve la línea de inserción");
    assert_eq!(a.tree.drop_target, None, "no se resalta la carpeta: no se suelta adentro");
    assert_eq!(drag_dirs(&mut a, DragKind::Drop, at, &paths), Effect::LINK);
    a.tree.drag_end(false);
    assert_eq!(a.tree.place_drop, None);
    assert_eq!(&a.places.order[..3], &[Place::User(0), Place::Folder(own.clone()), Place::User(1)]);
    a.paint();
    assert!(a.tree.row_point("proyectos").is_some(), "aparece en el árbol");
    assert!(!FakeShell::take_calls().iter().any(|c| matches!(c, ShellCall::PinQuickAccess(_))));
    // Un archivo no es un lugar: el medio de la fila sigue siendo soltar adentro.
    let file = root.join("nota.txt");
    std::fs::write(&file, b"x").unwrap();
    let mid = a.tree.row_point("Descargas").unwrap();
    assert_eq!(drag_dirs(&mut a, DragKind::Over, mid, &[file]), Effect::MOVE);
    assert_eq!(a.tree.place_drop, None);
    cleanup(&root);
}

/// Una vigilancia de mentira: con ella la `App` cree que el sistema tiene Acceso rápido (Windows).
struct QuickWatch;
impl crate::sys::Watch for QuickWatch {}

/// Windows (con Acceso rápido): lo soltado entre los lugares también queda como carpeta propia,
/// no se ancla al Acceso rápido del Explorador.
#[test]
fn dropping_between_places_adds_a_place_also_with_quick_access() {
    use crate::drop::{DragKind, Effect};
    let root = temp_root("soltar-acceso");
    let own = root.join("proyectos");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, _host) = places_app(&root);
    a.quick_watch = Some(Box::new(QuickWatch));
    FakeShell::take_calls();
    let at = row_top_edge(&a, "Descargas");
    assert_eq!(drag_dirs(&mut a, DragKind::Drop, at, &[own.clone()]), Effect::LINK);
    assert!(!FakeShell::take_calls().iter().any(|c| matches!(c, ShellCall::PinQuickAccess(_))));
    assert_eq!(&a.places.order[..3], &[Place::User(0), Place::Folder(own), Place::User(1)]);
    cleanup(&root);
}

/// Arrastrar una pestaña entre los lugares del árbol agrega su carpeta; la pestaña se queda donde
/// estaba.
#[test]
fn dropping_a_tab_between_places_adds_its_folder() {
    let root = temp_root("soltar-pestana");
    let own = root.join("trabajo");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, _host) = places_app(&root);
    a.new_tab(0, crate::fs::Location::Path(own.clone()), true);
    a.paint();
    let tabs = a.panes[0].tabs.len();
    let index = a.panes[0].tabs.iter().position(|t| t.location == crate::fs::Location::Path(own.clone())).unwrap();
    let (tx, ty) = point_of(&a, super::Hit::Pane(0, crate::pane::PaneHit::Tab(index)));
    a.on_left_down(tx, ty, false);
    let (x, y) = row_top_edge(&a, "Escritorio");
    a.on_mouse_move(tx + 40.0, ty + 40.0);
    a.on_mouse_move(x, y);
    assert!(a.tree.place_drop.is_some(), "se ve la línea de inserción");
    a.on_left_up(x, y);
    assert_eq!(a.tree.place_drop, None);
    assert_eq!(a.places.order[0], Place::Folder(own.clone()), "va delante del Escritorio");
    assert_eq!(a.panes[0].tabs.len(), tabs, "la pestaña sigue ahí");
    cleanup(&root);
}

/// Arrastrar una carpeta del árbol entre los lugares la deja ahí, como una pestaña: una carpeta
/// propia que ya estaba se mueve. Sin mover el mouse es un clic: abre y no cambia los lugares.
#[test]
fn dragging_a_tree_folder_between_places_moves_it_there() {
    let root = temp_root("arrastrar-arbol");
    let own = root.join("trabajo");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, _host) = places_app(&root);
    a.places.add_folder(own.clone());
    a.places_changed();
    a.paint();
    let (rx, ry) = a.tree.row_point("trabajo").expect("la carpeta propia en el árbol");
    a.on_left_down(rx, ry, false);
    a.on_left_up(rx, ry);
    assert_eq!(a.places.order.last(), Some(&Place::Folder(own.clone())), "un clic no la mueve");
    a.on_left_down(rx, ry, false);
    let (x, y) = row_top_edge(&a, "Escritorio");
    a.on_mouse_move(rx, ry - 20.0);
    a.on_mouse_move(x, y);
    assert!(a.tree.place_drop.is_some(), "se ve la línea de inserción");
    assert!(a.ghost.is_some(), "la carpeta acompaña al mouse");
    a.on_left_up(x, y);
    assert_eq!(a.tree.place_drop, None);
    assert!(a.ghost.is_none());
    assert_eq!(a.places.order[0], Place::Folder(own.clone()), "va delante del Escritorio");
    assert_eq!(a.places.order.iter().filter(|p| **p == Place::Folder(own.clone())).count(), 1, "sin repetirse");
    cleanup(&root);
}

/// Esc mientras se arrastra una carpeta del árbol: se cancela y soltar después no la agrega.
#[test]
fn escape_cancels_dragging_a_tree_folder() {
    let root = temp_root("esc-arbol");
    let own = root.join("trabajo");
    std::fs::create_dir_all(&own).unwrap();
    let (mut a, _host) = places_app(&root);
    a.places.add_folder(own.clone());
    a.places_changed();
    a.paint();
    let before = a.places.order.clone();
    let (rx, ry) = a.tree.row_point("trabajo").unwrap();
    a.on_left_down(rx, ry, false);
    let (x, y) = row_top_edge(&a, "Escritorio");
    a.on_mouse_move(rx, ry - 20.0);
    a.on_mouse_move(x, y);
    assert!(a.tree.place_drop.is_some());
    assert!(a.on_key(crate::keys::keys::VK_ESCAPE));
    assert_eq!(a.tree.place_drop, None, "se va la línea de inserción");
    assert!(a.ghost.is_none());
    a.on_left_up(x, y);
    assert_eq!(a.places.order, before, "no cambió nada");
    cleanup(&root);
}

fn point_of(a: &App, want: super::Hit) -> (f32, f32) {
    let (w, h) = a.client_dips();
    let mut y = 0.0;
    while y < h {
        let mut x = 0.0;
        while x < w {
            if a.hit(x, y) == want {
                return (x, y);
            }
            x += 2.0;
        }
        y += 2.0;
    }
    panic!("no se encontró {want:?}");
}

// ---------- anclar y favoritos desde los menús ----------

/// Id y texto de los ítems de un menú (sin submenús).
fn menu_items(menu: &[crate::shell::Entry]) -> Vec<(u32, String)> {
    menu.iter()
        .filter_map(|e| match e {
            crate::shell::Entry::Item { id, label, .. } => Some((*id, label.clone())),
            _ => None,
        })
        .collect()
}

/// El menú del encabezado de columnas también agrega la carpeta del panel a favoritos y a
/// Lugares (aunque no sea el panel activo), y la vuelve a quitar.
#[test]
fn header_menu_adds_to_favorites_and_places() {
    let root = temp_root("encabezado-lugares");
    let dir = root.join("trabajo");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut a, host) = places_app(&root);
    a.new_tab(0, crate::fs::Location::Path(dir.clone()), true);
    a.show_columns_menu(0, (10.0, 10.0));
    let items = menu_items(&host.menus().pop().unwrap());
    let pick = |items: &[(u32, String)], label: &str| items.iter().find(|i| i.1 == label).unwrap_or_else(|| panic!("{label}: {items:?}")).0;
    assert!(!items.iter().any(|i| i.1.contains("Acceso rápido")), "{items:?}");
    host.set_menu_choice(Some(pick(&items, "Agregar a favoritos")));
    a.show_columns_menu(0, (10.0, 10.0));
    assert!(a.favorites.find(&crate::fs::Location::Path(dir.clone())).is_some());
    host.set_menu_choice(Some(pick(&items, "Agregar a Lugares")));
    a.show_columns_menu(0, (10.0, 10.0));
    assert!(a.places.order.contains(&Place::Folder(dir.clone())), "se agregó a Lugares");
    host.set_menu_choice(None);
    a.show_columns_menu(0, (10.0, 10.0));
    let items = menu_items(&host.menus().pop().unwrap());
    host.set_menu_choice(Some(pick(&items, "Quitar de Lugares")));
    a.show_columns_menu(0, (10.0, 10.0));
    assert!(!a.places.order.contains(&Place::Folder(dir)), "se quitó");
    host.set_menu_choice(None);
    cleanup(&root);
}

/// El menú de una carpeta del árbol y el de una carpeta elegida en la lista suman «Agregar a
/// Lugares» en las 3 plataformas; en una carpeta del usuario dice «Quitar de Lugares» y la oculta.
#[test]
fn folder_menus_offer_places() {
    let root = temp_root("menu-lugares");
    let dir = root.join("trabajo");
    std::fs::create_dir_all(&dir).unwrap();
    let (mut a, _host) = places_app(&root);
    FakeShell::take_calls();
    FakeShell::choose(Some("own:0"));
    a.tree_context_menu(dir.clone(), (10.0, 10.0));
    let own = |calls: Vec<ShellCall>| {
        calls.into_iter().find_map(|c| match c {
            ShellCall::ContextMenu { own, .. } => Some(menu_items(&own)),
            _ => None,
        })
    };
    assert_eq!(own(FakeShell::take_calls()).unwrap(), [(0, "Agregar a Lugares".to_string())]);
    assert!(a.places.order.contains(&Place::Folder(dir.clone())));
    // Una carpeta del usuario ya se ve: «Quitar» la oculta.
    let docs = root.join("documentos");
    a.tree_context_menu(docs.clone(), (10.0, 10.0));
    assert_eq!(own(FakeShell::take_calls()).unwrap(), [(0, "Quitar de Lugares".to_string())]);
    assert!(a.places.is_hidden(&Place::User(2)));
    FakeShell::choose(None);
    cleanup(&root);
}

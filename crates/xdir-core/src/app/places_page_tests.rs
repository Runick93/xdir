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

use super::*;
use crate::sys::testing::install_fakes;

fn builtins() -> Vec<Place> {
    let mut v: Vec<Place> = (0..6).map(Place::User).collect();
    v.push(Place::Trash);
    v.push(Place::QuickAccess);
    v
}

#[test]
fn places_default_order() {
    let p = Places::default();
    assert_eq!(p.order, builtins());
    assert!(p.hidden.is_empty());
}

#[test]
fn places_roundtrip() {
    let mut p = Places::default();
    // Rutas absolutas en cualquier sistema (las relativas se descartan al leer).
    let base = std::env::temp_dir();
    p.order.push(Place::Folder(base.join("Jos\u{e9}").join("Mis Proyectos \u{f1}and\u{fa}")));
    p.order.push(Place::Folder(base.join(r#"con "comillas""#)));
    p.order.swap(0, 3);
    p.hidden = vec![Place::User(4), Place::Folder(base.join(r#"con "comillas""#))];
    let text = p.to_toml();
    assert!(text.contains("\"musica\""));
    assert_eq!(Places::parse(&text), p);

    let dir = std::env::temp_dir().join(format!("xdir-lugares-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    p.save_in(&dir).unwrap();
    assert_eq!(Places::load_from(&dir), p);
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn places_unknown_lines_ignored() {
    let p = Places::parse("basura sin igual\notra = 1\n[x]\norden = [\"videos\", \"escritorio\"]\n");
    assert_eq!(p, Places::default());
    let p = Places::parse("cosa = 3\norden = [\"videos\", \"escritorio\", \"videos\", \"\"]\nocultos = [\"papelera\", \"musica\"]\n!!!\n");
    assert_eq!(&p.order[..2], &[Place::User(5), Place::User(0)]);
    assert_eq!(p.order.iter().filter(|x| **x == Place::User(5)).count(), 1);
    // `ocultos` solo cuenta lo que está en `orden`; los que faltan se suman y siguen visibles.
    assert!(p.hidden.contains(&Place::Trash) && p.hidden.contains(&Place::User(4)));
    assert_eq!(Places::parse("").order, builtins());
}

#[test]
fn places_missing_builtins_appended() {
    let srv = std::env::temp_dir().join("x");
    let old = format!(
        "orden = [\"escritorio\", \"descargas\", \"documentos\", \"imagenes\", \"musica\", \"videos\", \"acceso_rapido\", {}]\n",
        crate::toml::quote(&srv.to_string_lossy())
    );
    let p = Places::parse(&old);
    assert_eq!(p.order[6], Place::QuickAccess);
    assert_eq!(p.order[7], Place::Trash);
    assert_eq!(p.order[8], Place::Folder(srv));
    assert_eq!(p.order.len(), 9);
}

#[test]
fn places_move_toggle_add_remove() {
    install_fakes();
    let mut p = Places::default();
    p.move_up(0);
    p.move_down(7);
    assert_eq!(p.order, builtins());
    p.move_down(0);
    assert_eq!(&p.order[..2], &[Place::User(1), Place::User(0)]);
    p.move_up(1);
    assert_eq!(p.order, builtins());
    p.move_up(99);
    p.move_down(99);
    p.toggle(6);
    assert_eq!(p.hidden, vec![Place::Trash]);
    p.toggle(6);
    assert!(p.hidden.is_empty());
    p.toggle(99);
    let proy = std::env::temp_dir().join("proy");
    p.add_folder(proy.clone());
    p.add_folder(proy);
    assert_eq!(p.order.len(), 9);
    p.remove(0);
    assert_eq!(p.order.len(), 9);
    p.toggle(8);
    assert_eq!(p.hidden.len(), 1);
    p.remove(8);
    assert_eq!(p.order, builtins());
    assert!(p.hidden.is_empty());
    p.remove(99);
}

#[test]
fn places_add_folder_ignores_relative() {
    install_fakes();
    let mut p = Places::default();
    p.add_folder(PathBuf::from("papelera"));
    assert_eq!(p.order, builtins());
}

/// Varias escrituras seguidas en otro hilo: queda la última (en la carpeta de pruebas del hilo).
#[test]
fn places_save_in_background_last_wins() {
    install_fakes();
    let dir = session::config_dir().unwrap();
    let mut p = Places::default();
    for i in 0..20 {
        p.move_down(i % 7);
        p.save_in_background();
    }
    let want = p.to_toml();
    let t0 = std::time::Instant::now();
    while std::fs::read_to_string(dir.join(FILE_NAME)).ok().as_deref() != Some(want.as_str()) {
        assert!(t0.elapsed().as_secs() < 10, "no quedó la última escritura");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Y no la pisa una más vieja que llegue tarde.
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), want);
}

/// Una carpeta propia relativa en el archivo («música» a mano) se descarta: no es un lugar fijo
/// ni una ruta.
#[test]
fn places_parse_drops_relative_folders() {
    let p = Places::parse("orden = [\"música\", \"videos\"]\nocultos = [\"música\"]\n");
    assert!(!p.order.iter().any(|x| matches!(x, Place::Folder(_))));
    assert!(p.hidden.is_empty());
    assert_eq!(p.order.len(), builtins().len());
}

/// Agregar una carpeta del usuario no la repite como propia: si estaba oculta, se vuelve a ver.
#[test]
fn places_add_folder_user_folder_unhides() {
    install_fakes();
    let desk = std::env::temp_dir().join("xdir-lugares-escritorio");
    crate::sys::testing::FakeFileSystem::set_user_folders(&[desk.as_path()]);
    let mut p = Places::default();
    p.toggle(0);
    assert!(p.is_hidden(&Place::User(0)));
    p.add_folder(desk.clone());
    assert!(!p.is_hidden(&Place::User(0)), "se vuelve a ver el Escritorio");
    assert_eq!(p.order, builtins());
    p.add_folder(desk);
    assert_eq!(p.order, builtins());
    crate::sys::testing::FakeFileSystem::set_user_folders(&[]);
}

/// Soltar una carpeta entre dos lugares la agrega ahí (delante de `before`; sin él, al final). Si
/// ya era una carpeta propia, se mueve a ese lugar en vez de repetirse.
#[test]
fn places_add_folder_at_position() {
    install_fakes();
    let mut p = Places::default();
    let a = std::env::temp_dir().join("xdir-lugar-a");
    let b = std::env::temp_dir().join("xdir-lugar-b");
    p.add_folder_at(a.clone(), Some(&Place::Trash));
    assert_eq!(p.order[6], Place::Folder(a.clone()));
    assert_eq!(p.order[7], Place::Trash);
    p.add_folder_at(b.clone(), None);
    assert_eq!(p.order.last(), Some(&Place::Folder(b.clone())));
    // Ya estaba: se mueve delante del Escritorio, sin repetirse.
    p.add_folder_at(b.clone(), Some(&Place::User(0)));
    assert_eq!(p.order[0], Place::Folder(b.clone()));
    assert_eq!(p.order.iter().filter(|x| **x == Place::Folder(b.clone())).count(), 1);
    assert_eq!(p.order.len(), 10);
    // Delante de sí misma: queda donde está.
    p.add_folder_at(b.clone(), Some(&Place::Folder(b.clone())));
    assert_eq!(p.order[0], Place::Folder(b));
    // Relativa: no se agrega.
    p.add_folder_at(PathBuf::from("papelera"), None);
    assert_eq!(p.order.len(), 10);
}

/// Lo que la Mac y Linux tenían en su Acceso rápido propio (`acceso_rapido.txt`) pasa a los
/// lugares al leerlos, y el archivo se borra.
#[test]
fn old_quick_access_file_becomes_places() {
    crate::sys::testing::install_fakes();
    let dir = std::env::temp_dir().join(format!("xdir-lugares-migrar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("uno"), dir.join("dos"));
    std::fs::write(dir.join("acceso_rapido.txt"), format!("# comentario\n{}\n\n{}\n", a.display(), b.display())).unwrap();
    let places = Places::load_from(&dir);
    let n = places.order.len();
    assert_eq!(&places.order[n - 2..], &[Place::Folder(a), Place::Folder(b)]);
    assert!(!dir.join("acceso_rapido.txt").exists(), "el archivo viejo se borra");
    assert_eq!(Places::load_from(&dir), places, "quedó guardado en lugares.toml");
    let _ = std::fs::remove_dir_all(&dir);
}

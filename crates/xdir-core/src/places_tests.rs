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
    p.order.push(Place::Folder(PathBuf::from("C:\\Users\\Jos\u{e9}\\Mis Proyectos \u{f1}and\u{fa}")));
    p.order.push(Place::Folder(PathBuf::from(r#"D:\con "comillas""#)));
    p.order.swap(0, 3);
    p.hidden = vec![Place::User(4), Place::Folder(PathBuf::from(r#"D:\con "comillas""#))];
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
    let old = "orden = [\"escritorio\", \"descargas\", \"documentos\", \"imagenes\", \"musica\", \"videos\", \"acceso_rapido\", \"/srv/x\"]\n";
    let p = Places::parse(old);
    assert_eq!(p.order[6], Place::QuickAccess);
    assert_eq!(p.order[7], Place::Trash);
    assert_eq!(p.order[8], Place::Folder(PathBuf::from("/srv/x")));
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
    p.add_folder(PathBuf::from("/tmp/proy"));
    p.add_folder(PathBuf::from("/tmp/proy"));
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

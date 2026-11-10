use super::*;

fn a(s: &str) -> Address {
    Address::parse(s).unwrap()
}

#[test]
fn recents_front_dedupe_max_ten() {
    let mut p = NetPlaces::default();
    for i in 0..12 {
        p.add_recent(&a(&format!("smb://nas/r{i}")), i);
    }
    assert_eq!(p.recents.len(), 10);
    assert_eq!(p.recents[0].address, "smb://nas/r11");
    assert_eq!(p.recents[9].address, "smb://nas/r2");
    p.add_recent(&a(r"\\NAS\r5"), 100);
    assert_eq!(p.recents.len(), 10);
    assert_eq!(p.recents[0].address, "smb://NAS/r5");
    assert_eq!(p.recents[0].date, 100);
    assert_eq!(p.recents.iter().filter(|r| r.address.eq_ignore_ascii_case("smb://nas/r5")).count(), 1);
    p.remove_recent(0);
    assert_eq!(p.recents[0].address, "smb://nas/r11");
    p.clear_recents();
    assert!(p.recents.is_empty());
}

#[test]
fn favorites_order_and_toggle() {
    let mut p = NetPlaces::default();
    p.toggle_favorite(&a("smb://nas/fotos"), Some("NAS — fotos".into()));
    p.toggle_favorite(&a(r"\\srv01\compartido"), None);
    p.toggle_favorite(&a("smb://x/y"), Some("  ".into()));
    assert_eq!(p.favorites[0].name, "NAS — fotos");
    assert_eq!(p.favorites[1].name, "smb://srv01/compartido");
    assert_eq!(p.favorites[2].name, "smb://x/y");
    assert!(p.is_favorite(&a("SMB://NAS/fotos")));
    p.move_favorite(0, false);
    assert_eq!(p.favorites[1].name, "NAS — fotos");
    p.move_favorite(1, true);
    assert_eq!(p.favorites[0].name, "NAS — fotos");
    p.move_favorite(0, true);
    p.move_favorite(2, false);
    assert_eq!(p.favorites[0].name, "NAS — fotos");
    p.toggle_favorite(&a("smb://nas/fotos"), None);
    assert!(!p.is_favorite(&a("smb://nas/fotos")));
    assert_eq!(p.favorites.len(), 2);
    p.remove_favorite(0);
    p.remove_favorite(9);
    assert_eq!(p.favorites.len(), 1);
}

#[test]
fn places_roundtrip() {
    let mut p = NetPlaces::default();
    p.toggle_favorite(&a("smb://nas/fotos"), Some("NAS \"casa\" — fotos".into()));
    p.toggle_favorite(&a("sftp://ana@vps:2222/var/www"), None);
    p.add_recent(&a("afp://mac/vol"), 1_731_234_567);
    p.add_recent(&a("smb://nas/mis fotos"), 1_731_234_999);
    let text = p.write();
    assert_eq!(NetPlaces::parse(&text), p);
}

#[test]
fn invalid_entries_dropped() {
    let text = "[[favorito]]\nnombre = \"x\"\ndireccion = \"smb://na s/x\"\n[[favorito]]\ndireccion = \"smb://ok/y\"\n[[reciente]]\nfecha = 3\n[[reciente]]\ndireccion = \"nas\"\n";
    let p = NetPlaces::parse(text);
    assert_eq!(p.favorites.len(), 1);
    assert_eq!(p.favorites[0].name, "smb://ok/y");
    assert_eq!(p.recents.len(), 1);
    assert_eq!(p.recents[0].address, "smb://nas");
    assert_eq!(p.recents[0].date, 0);
}

#[test]
fn save_is_atomic_and_load_reads() {
    let dir = std::env::temp_dir().join(format!("xdir-red-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut p = NetPlaces::default();
    p.add_recent(&a("smb://nas/fotos"), 9);
    p.save_in(&dir).unwrap();
    assert!(!dir.join("red.toml.tmp").exists());
    assert_eq!(NetPlaces::load_from(&dir), p);
    let _ = std::fs::remove_dir_all(&dir);
}

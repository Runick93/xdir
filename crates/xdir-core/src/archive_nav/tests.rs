use super::temps::Temps;
use super::*;
use crate::sys::testing::{FakeArchives, FakeCopyPrims, install_fakes};

fn e(name: &str, kind: EntryKind) -> ArchiveEntry {
    ArchiveEntry { index: 0, name: name.into(), size: 10, kind, link: None, modified: 1_700_000_000, packed: 4, encrypted: false }
}

fn names(c: &[Child]) -> Vec<(String, bool)> {
    let mut v: Vec<_> = c.iter().map(|c| (c.name.clone(), c.is_dir)).collect();
    v.sort();
    v
}

/// `a/b/c.txt` sin entrada `a/` muestra la carpeta `a` (una sola vez aunque haya muchas
/// entradas adentro, y también si después viene la entrada `a/`).
#[test]
fn implicit_folders_listed_once() {
    let list = [e("a/b/c.txt", EntryKind::File), e("a/x.txt", EntryKind::File), e("a/", EntryKind::Dir), e("top.txt", EntryKind::File), e("./", EntryKind::Dir)];
    assert_eq!(names(&children(&list, "")), [("a".into(), true), ("top.txt".into(), false)]);
    assert_eq!(names(&children(&list, "a")), [("b".into(), true), ("x.txt".into(), false)]);
    let c = children(&list, "a/b");
    assert_eq!(names(&c), [("c.txt".into(), false)]);
    assert_eq!((c[0].size, c[0].packed, c[0].modified), (10, 4, 1_700_000_000));
    // La entrada «a/» le da su fecha a la carpeta implícita.
    assert_eq!(children(&list, "").iter().find(|c| c.name == "a").unwrap().modified, 1_700_000_000);
    assert!(folder_exists(&list, "a/b"));
    assert!(!folder_exists(&list, "a/c"));
    assert!(!folder_exists(&list, "top.txt"));
}

/// Los nombres que no se permiten al extraer se muestran en la raíz con su nombre crudo y marcados.
#[test]
fn invalid_names_flagged_at_root() {
    let list = [e("../evil.txt", EntryKind::File), e("/etc/passwd", EntryKind::File), e("a/CON", EntryKind::File), e("a/ok.txt", EntryKind::File)];
    let root = children(&list, "");
    let bad: Vec<&str> = root.iter().filter(|c| c.bad).map(|c| c.name.as_str()).collect();
    assert_eq!(bad, ["../evil.txt", "/etc/passwd", "a/CON"]);
    assert_eq!(names(&children(&list, "a")), [("ok.txt".into(), false)]);
}

/// Un enlace se muestra como enlace (no como carpeta) y un elemento que no se crea va marcado.
#[test]
fn symlinks_are_links_not_dirs() {
    let list = [e("l", EntryKind::Symlink), e("fifo", EntryKind::Omitted)];
    let c = children(&list, "");
    let l = c.iter().find(|c| c.name == "l").unwrap();
    assert!(l.link && !l.is_dir && !l.bad);
    assert!(c.iter().find(|c| c.name == "fifo").unwrap().bad);
    let row = entry_of(l);
    assert_ne!(row.attrs & crate::fs::ATTR_LINK, 0);
}

#[test]
fn mac_metadata_hidden() {
    let list = [e("__MACOSX/._a.txt", EntryKind::File), e("__MACOSX/", EntryKind::Dir), e("a.txt", EntryKind::File)];
    assert_eq!(names(&children(&list, "")), [("a.txt".into(), false)]);
}

#[test]
fn capped_at_big_entries() {
    let list: Vec<ArchiveEntry> = (0..MAX_ENTRIES + 5).map(|i| e(&format!("f{i}"), EntryKind::File)).collect();
    assert_eq!(children(&list, "").len(), MAX_ENTRIES);
}

/// La fecha llega como FILETIME (la que usa la lista) y las carpetas con su atributo.
#[test]
fn rows_have_filetime_and_dir_attr() {
    install_fakes();
    let c = Child { name: "d".into(), is_dir: true, size: 0, packed: 0, modified: 1, link: false, bad: false };
    let row = entry_of(&c);
    assert_eq!(row.modified, (1 + 11_644_473_600) * 10_000_000);
    assert!(row.is_dir && row.attrs & 0x10 != 0);
}

// ---------- temporales ----------

fn base(name: &str) -> PathBuf {
    let b = std::env::temp_dir().join(format!("xdir-temps-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&b);
    b
}

/// Al arrancar se borran las carpetas de ejecuciones que ya no están; la de un xdir abierto
/// (candado tomado) queda.
#[test]
fn stale_run_dirs_removed_live_kept() {
    let b = base("viejas");
    let live = Temps::new(&b);
    let live_dir = live.run_dir().to_path_buf();
    let dead = b.join("1-1");
    std::fs::create_dir_all(&dead).unwrap();
    std::fs::write(dead.join(".candado"), "").unwrap();
    std::fs::write(dead.join("foto.jpg"), "x").unwrap();
    let me = Temps::new(&b);
    me.clean_stale();
    assert!(!dead.exists());
    assert!(live_dir.exists(), "la de la otra ejecución, con su candado, queda");
    live.clean_own();
    assert!(!live_dir.exists());
    let _ = std::fs::remove_dir_all(&b);
}

/// `ensure` extrae una vez a la ruta que dice `entry_path`; si el archivo cambia, vuelve a
/// extraer.
#[test]
fn ensure_extracts_once_and_again_if_changed() {
    install_fakes();
    let b = base("una");
    let zip = b.join("datos").join("fotos.zip");
    FakeArchives::shared().add_archive(&zip, &[("a/", b""), ("a/nota.txt", b"hola")], 30);
    let t = Temps::new(&b);
    let got = t.ensure(&zip, &[], "a/nota.txt").unwrap();
    assert_eq!(Some(got.clone()), t.entry_path(&zip, &[], "a/nota.txt"));
    assert_eq!(FakeCopyPrims::shared().read_content(&got).unwrap(), b"hola");
    // Otra vez: no se toca (se cambió a mano y sigue así).
    FakeCopyPrims::shared().add_file(&got, b"tocado".to_vec());
    assert_eq!(t.ensure(&zip, &[], "a/nota.txt").unwrap(), got);
    assert_eq!(FakeCopyPrims::shared().read_content(&got).unwrap(), b"tocado");
    // El zip cambió: se vuelve a extraer.
    FakeArchives::shared().add_archive(&zip, &[("a/", b""), ("a/nota.txt", b"nueva version")], 31);
    let again = t.ensure(&zip, &[], "a/nota.txt").unwrap();
    assert_eq!(FakeCopyPrims::shared().read_content(&again).unwrap(), b"nueva version");
    assert!(t.ensure(&zip, &[], "../x").is_err());
    assert!(t.ensure(&zip, &[], "no-esta.txt").is_err());
    t.clean_own();
}

/// Un zip dentro de otro se resuelve extrayendo el de adentro a la temporal.
#[test]
fn nested_chain_resolves() {
    install_fakes();
    let b = base("anidado");
    let outer = b.join("afuera.zip");
    FakeArchives::shared().add_archive(&outer, &[("dir/adentro.zip", b"zz")], 10);
    let t = Temps::new(&b);
    let inner_path = t.entry_path(&outer, &[], "dir/adentro.zip").unwrap();
    // El de adentro también es un «zip» del falso (con sus entradas).
    let real = t.resolve(&outer, &["dir/adentro.zip".to_string()]).unwrap();
    assert_eq!(real, inner_path);
    FakeArchives::shared().add_archive(&real, &[("hondo.txt", b"!")], 1);
    let deep = t.ensure(&outer, &["dir/adentro.zip".to_string()], "hondo.txt").unwrap();
    assert_eq!(FakeCopyPrims::shared().read_content(&deep).unwrap(), b"!");
    assert_ne!(t.dir_for(&outer, &[]), t.dir_for(&outer, &["dir/adentro.zip".to_string()]));
    t.clean_own();
}

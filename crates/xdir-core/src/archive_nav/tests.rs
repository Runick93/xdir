use super::temps::Temps;
use super::*;
use crate::sys::testing::{FakeArchives, FakeCopyPrims, install_fakes};
use crate::sys::ErrKind;

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
    // La plataforma la da ya creada.
    std::fs::create_dir_all(&b).unwrap();
    b
}

/// Al arrancar se borran las carpetas de ejecuciones que ya no están; la de un xdir abierto
/// (candado tomado) queda.
#[test]
fn stale_run_dirs_removed_live_kept() {
    let b = base("viejas");
    let live = Temps::new(Some(&b));
    let live_dir = live.run_dir().unwrap().to_path_buf();
    let dead = b.join("1-1");
    std::fs::create_dir_all(&dead).unwrap();
    std::fs::write(dead.join(".candado"), "").unwrap();
    std::fs::write(dead.join("foto.jpg"), "x").unwrap();
    let me = Temps::new(Some(&b));
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
    let t = Temps::new(Some(&b));
    let got = t.ensure(&zip, &[], "a/nota.txt", &super::temps::Limits::up_to(u64::MAX)).unwrap();
    assert_eq!(Some(got.clone()), t.entry_path(&zip, &[], "a/nota.txt"));
    assert_eq!(FakeCopyPrims::shared().read_content(&got).unwrap(), b"hola");
    // Otra vez: no se toca (se cambió a mano y sigue así).
    FakeCopyPrims::shared().add_file(&got, b"tocado".to_vec());
    assert_eq!(t.ensure(&zip, &[], "a/nota.txt", &super::temps::Limits::up_to(u64::MAX)).unwrap(), got);
    assert_eq!(FakeCopyPrims::shared().read_content(&got).unwrap(), b"tocado");
    // El zip cambió: se vuelve a extraer.
    FakeArchives::shared().add_archive(&zip, &[("a/", b""), ("a/nota.txt", b"nueva version")], 31);
    let again = t.ensure(&zip, &[], "a/nota.txt", &super::temps::Limits::up_to(u64::MAX)).unwrap();
    assert_eq!(FakeCopyPrims::shared().read_content(&again).unwrap(), b"nueva version");
    assert!(t.ensure(&zip, &[], "../x", &super::temps::Limits::up_to(u64::MAX)).is_err());
    assert!(t.ensure(&zip, &[], "no-esta.txt", &super::temps::Limits::up_to(u64::MAX)).is_err());
    t.clean_own();
}

/// Un zip dentro de otro se resuelve extrayendo el de adentro a la temporal.
#[test]
fn nested_chain_resolves() {
    install_fakes();
    let b = base("anidado");
    let outer = b.join("afuera.zip");
    FakeArchives::shared().add_archive(&outer, &[("dir/adentro.zip", b"zz")], 10);
    let t = Temps::new(Some(&b));
    let inner_path = t.entry_path(&outer, &[], "dir/adentro.zip").unwrap();
    // El de adentro también es un «zip» del falso (con sus entradas).
    let real = t.resolve(&outer, &["dir/adentro.zip".to_string()], &|| false).unwrap();
    assert_eq!(real, inner_path);
    FakeArchives::shared().add_archive(&real, &[("hondo.txt", b"!")], 1);
    let deep = t.ensure(&outer, &["dir/adentro.zip".to_string()], "hondo.txt", &super::temps::Limits::up_to(u64::MAX)).unwrap();
    assert_eq!(FakeCopyPrims::shared().read_content(&deep).unwrap(), b"!");
    assert_ne!(t.dir_for(&outer, &[]), t.dir_for(&outer, &["dir/adentro.zip".to_string()]));
    t.clean_own();
}

/// Solo los errores conocidos ofrecen abrir con el sistema (no un texto que diga «dañado»).
#[test]
fn offers_system_only_known_errors() {
    for e in [crate::sys::PASSWORD, crate::sys::UNSUPPORTED, crate::sys::DAMAGED_ZIP, crate::sys::DAMAGED] {
        assert!(offers_system(e), "{e}");
    }
    assert!(!offers_system("Error de E/S: el sector está dañado"));
    assert!(!offers_system("Ya no existe"));
}

/// Sin una carpeta temporal segura (la plataforma no la dio o no pasó sus controles), no se
/// extrae nada ni hay rutas temporales.
#[test]
fn no_safe_base_refuses_extraction() {
    install_fakes();
    let zip = base("sin-base").join("a.zip");
    FakeArchives::shared().add_archive(&zip, &[("x.txt", b"x")], 3);
    let t = Temps::new(None);
    assert_eq!(t.entry_path(&zip, &[], "x.txt"), None);
    let err = t.ensure(&zip, &[], "x.txt", &super::temps::Limits::up_to(u64::MAX)).unwrap_err();
    assert_eq!(err.message(), super::temps::NO_SAFE_TEMP);
}

/// La carpeta de las temporales la da el sistema (la del falso, una por proceso de pruebas).
#[test]
fn global_base_comes_from_the_platform() {
    install_fakes();
    let base = crate::sys::env().archive_temp_dir().expect("el falso la da");
    assert!(super::temps::global().run_dir().unwrap().starts_with(&base));
}

/// Lo que se extrae a la temporal hereda el origen del archivo (marca de la web / cuarentena):
/// el archivo y las carpetas que se crearon para llegar a él.
#[test]
fn ensure_propagates_origin() {
    install_fakes();
    let b = base("origen");
    let zip = b.join("bajado.zip");
    FakeArchives::shared().add_archive(&zip, &[("d/e/malo.js", b"x")], 3);
    let t = Temps::new(Some(&b));
    let got = t.ensure(&zip, &[], "d/e/malo.js", &super::temps::Limits::up_to(u64::MAX)).unwrap();
    let dir = t.dir_for(&zip, &[]);
    let marked = FakeArchives::shared().propagated(&zip);
    for p in [got.clone(), dir.join("d"), dir.join("d").join("e")] {
        assert!(marked.contains(&p), "{p:?} no heredó el origen: {marked:?}");
    }
    t.clean_own();
}

/// Más grande que el límite, sin lugar en el disco o cancelado: no se extrae.
#[test]
fn ensure_limits_size_space_and_cancel() {
    install_fakes();
    let b = base("limites");
    let zip = b.join("grande.zip");
    FakeArchives::shared().add_archive(&zip, &[("chico.txt", b"1234"), ("grande.bin", b"")], 3);
    FakeArchives::shared().set_declared(&zip, "grande.bin", 10_000);
    let t = Temps::new(Some(&b));
    let err = t.ensure(&zip, &[], "grande.bin", &super::temps::Limits::up_to(5_000)).unwrap_err();
    assert_eq!(err.message(), super::temps::TOO_BIG);
    let err = t.ensure(&zip, &[], "chico.txt", &super::temps::Limits { max: 1_000, cancel: &|| true }).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
    assert!(FakeCopyPrims::shared().read_content(&t.entry_path(&zip, &[], "chico.txt").unwrap()).is_err());
    let tiny = crate::sys::VolumeInfo { id: "temp".into(), label: "temp".into(), case_insensitive: false, has_trash: false, max_file: None, free: 2 };
    FakeCopyPrims::shared().set_volume(t.run_dir().unwrap(), tiny);
    let err = t.ensure(&zip, &[], "chico.txt", &super::temps::Limits::up_to(1_000)).unwrap_err();
    assert_eq!(err.message(), super::temps::NO_ROOM);
    t.clean_own();
}

/// Con dos entradas del mismo nombre se abre la que muestra la lista (la última).
#[test]
fn ensure_extracts_the_listed_duplicate() {
    install_fakes();
    let b = base("dobles");
    let zip = b.join("dobles.zip");
    FakeArchives::shared().add_archive(&zip, &[("a.txt", b"uno"), ("a.txt", b"dos")], 3);
    let t = Temps::new(Some(&b));
    let got = t.ensure(&zip, &[], "a.txt", &super::temps::Limits::up_to(u64::MAX)).unwrap();
    assert_eq!(FakeCopyPrims::shared().read_content(&got).unwrap(), b"dos");
    t.clean_own();
}

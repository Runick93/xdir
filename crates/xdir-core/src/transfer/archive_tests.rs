//! Pruebas de comprimir y extraer en el motor (con `FakeCopyPrims` y `FakeArchives`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::engine::{run_with, title};
use super::{Answer, Choice, ConflictInfo, Control, JobOutcome, JobSpec, Update};
use crate::ops::Undo;
use crate::sys::testing::{FakeArchives, FakeCopyPrims, install_fakes};
use crate::sys::{NOT_ALLOWED, PASSWORD, VolumeInfo};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

struct Ran {
    updates: Vec<Update>,
    out: JobOutcome,
}

impl Ran {
    fn conflicts(&self) -> Vec<ConflictInfo> {
        self.updates.iter().filter_map(|u| if let Update::Conflict(c) = u { Some(c.clone()) } else { None }).collect()
    }
    fn messages(&self) -> Vec<String> {
        self.out.errors.iter().map(|e| e.message.clone()).collect()
    }
}

/// Corre un trabajo; responde los conflictos con `answers` y las preguntas con `confirm`.
fn go(disk: &FakeCopyPrims, arch: &FakeArchives, spec: JobSpec, answers: &[Answer], confirm: bool, hook: &mut dyn FnMut(&Update, &Arc<Control>)) -> Ran {
    install_fakes();
    let ctl = Control::new();
    let mut answers = answers.iter().copied();
    let mut updates = Vec::new();
    run_with(spec, disk, arch, &ctl, &mut |u| {
        match &u {
            Update::Conflict(_) => ctl.answer(answers.next().expect("conflicto sin respuesta")),
            Update::Confirm(_) => ctl.confirm(confirm),
            _ => {}
        }
        hook(&u, &ctl);
        updates.push(u);
    });
    let out = match updates.last() {
        Some(Update::Finished(o)) => o.clone(),
        other => panic!("el trabajo no terminó con Finished: {other:?}"),
    };
    Ran { updates, out }
}

fn simple(disk: &FakeCopyPrims, arch: &FakeArchives, spec: JobSpec) -> Ran {
    go(disk, arch, spec, &[], true, &mut |_, _| {})
}

fn no_tmp_left(disk: &FakeCopyPrims, dir: &Path) {
    let left: Vec<_> = disk.list_all(dir).into_iter().filter(|p| p.to_string_lossy().contains(".xdir")).collect();
    assert!(left.is_empty(), "quedaron temporales: {left:?}");
}

/// Lo creado del deshacer (sin identidades).
fn created(u: &Option<Undo>) -> Vec<PathBuf> {
    fn walk(u: &Undo, out: &mut Vec<PathBuf>) {
        match u {
            Undo::Batch(v) => v.iter().for_each(|u| walk(u, out)),
            Undo::Created(v) => out.extend(v.iter().cloned()),
            Undo::CreatedIds(v) => out.extend(v.iter().map(|(p, _)| p.clone())),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(u) = u {
        walk(u, &mut out);
    }
    out
}

#[test]
fn compress_one_and_many_names() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_file(&p("/z1/informe.pdf"), b"pdf".to_vec());
    disk.add_file(&p("/z1/Fotos/a.jpg"), b"jpg!".to_vec());
    disk.add_dir(&p("/z1/Fotos/sub"));
    let r = simple(&disk, &arch, JobSpec::Compress { sources: vec![p("/z1/informe.pdf")] });
    let zip = p("/z1/informe.pdf.zip");
    assert_eq!(arch.created(&zip).unwrap(), vec![("informe.pdf".to_string(), b"pdf".to_vec())]);
    assert_eq!(r.out.result.select, vec![zip.clone()]);
    assert_eq!(created(&r.out.result.undo), vec![zip]);
    assert!(r.out.errors.is_empty() && r.out.stopped.is_none());

    let spec = || JobSpec::Compress { sources: vec![p("/z1/informe.pdf"), p("/z1/Fotos")] };
    let r = simple(&disk, &arch, spec());
    assert!(r.updates.iter().any(|u| matches!(u, Update::Started { title, bytes: 7, .. } if title == "Comprimiendo 2 elementos")), "{:?}", r.updates);
    let names: Vec<String> = arch.created(&p("/z1/Archivo.zip")).unwrap().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["informe.pdf", "Fotos/", "Fotos/a.jpg", "Fotos/sub/"]);
    // El nombre libre siguiente.
    let r = simple(&disk, &arch, spec());
    assert_eq!(r.out.result.select, vec![p("/z1/Archivo 2.zip")]);
    assert!(arch.created(&p("/z1/Archivo 2.zip")).is_some());
    no_tmp_left(&disk, &p("/z1"));
    assert_eq!(title(&spec(), 2), "Comprimiendo 2 elementos");
}

#[test]
fn compress_cancel_removes_partial() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.set_block(1);
    disk.add_file(&p("/z2/big.bin"), vec![7; 20]);
    let r = go(&disk, &arch, JobSpec::Compress { sources: vec![p("/z2/big.bin")] }, &[], true, &mut |u, ctl| {
        if matches!(u, Update::Progress { bytes, .. } if *bytes > 0) {
            ctl.cancel();
        }
    });
    assert!(r.out.cancelled);
    assert!(r.out.result.undo.is_none() && r.out.result.select.is_empty());
    assert_eq!(disk.list_all(&p("/z2")), vec![p("/z2/big.bin")]);
}

#[test]
fn compress_reads_cloud_only_files() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_file(&p("/z3/nube.txt"), b"datos".to_vec());
    disk.set_cloud_only(&p("/z3/nube.txt"));
    simple(&disk, &arch, JobSpec::Compress { sources: vec![p("/z3/nube.txt")] });
    // Comprimir necesita el contenido: lo lee (y el sistema lo baja).
    assert_eq!(arch.created(&p("/z3/nube.txt.zip")).unwrap(), vec![("nube.txt".to_string(), b"datos".to_vec())]);
    assert_eq!(disk.reads(&p("/z3/nube.txt")), 1);
}

#[test]
fn compress_error_is_listed_for_retry() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_file(&p("/z4/a.txt"), b"a".to_vec());
    disk.fail(&p("/z4/a.txt"), crate::sys::ErrKind::InUse, 1);
    let r = simple(&disk, &arch, JobSpec::Compress { sources: vec![p("/z4/a.txt")] });
    assert_eq!(r.messages(), ["Lo está usando otro programa"]);
    assert!(r.out.errors[0].retry);
    assert_eq!(disk.list_all(&p("/z4")), vec![p("/z4/a.txt")]);
}

#[test]
fn extract_into_folder_numbered() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/x1/fotos.zip"), &[("a.jpg", b"aa"), ("sub/", b""), ("sub/b.jpg", b"bbb")], 5);
    disk.add_dir(&p("/x1/fotos"));
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x1/fotos.zip"), into_folder: true });
    assert!(r.conflicts().is_empty());
    assert_eq!(disk.contents(&p("/x1/fotos 2/a.jpg")).unwrap(), b"aa");
    assert_eq!(disk.contents(&p("/x1/fotos 2/sub/b.jpg")).unwrap(), b"bbb");
    assert!(disk.list_all(&p("/x1/fotos")).is_empty());
    assert_eq!(r.out.result.select, vec![p("/x1/fotos 2")]);
    assert_eq!(created(&r.out.result.undo), vec![p("/x1/fotos 2")]);
    assert!(r.updates.iter().any(|u| matches!(u, Update::Started { title, bytes: 5, .. } if title == "Extrayendo «fotos.zip»")), "{:?}", r.updates);
    no_tmp_left(&disk, &p("/x1"));
}

#[test]
fn extract_here_conflicts() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/x2/a.zip"), &[("a.txt", b"nuevo"), ("b.txt", b"b"), ("d/", b""), ("d/in.txt", b"in")], 5);
    disk.add_file(&p("/x2/a.txt"), b"viejo".to_vec());
    disk.add_file(&p("/x2/d/old.txt"), b"old".to_vec());
    let keep = Answer { choice: Choice::KeepBoth, all: false };
    let merge = Answer { choice: Choice::Merge, all: false };
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x2/a.zip"), into_folder: false }, &[keep, merge], true, &mut |_, _| {});
    let c = r.conflicts();
    assert_eq!(c.iter().map(|c| (c.name.as_str(), c.folder.as_str())).collect::<Vec<_>>(), [("a.txt", "x2"), ("d", "x2")]);
    assert_eq!(disk.contents(&p("/x2/a.txt")).unwrap(), b"viejo");
    assert_eq!(disk.contents(&p("/x2/a 2.txt")).unwrap(), b"nuevo");
    assert_eq!(disk.contents(&p("/x2/b.txt")).unwrap(), b"b");
    assert_eq!(disk.contents(&p("/x2/d/in.txt")).unwrap(), b"in");
    assert_eq!(disk.contents(&p("/x2/d/old.txt")).unwrap(), b"old");
    let mut made = created(&r.out.result.undo);
    made.sort();
    // Lo que estaba (la carpeta combinada) no es «lo creado»: deshacer no lo toca.
    assert_eq!(made, [p("/x2/a 2.txt"), p("/x2/b.txt"), p("/x2/d/in.txt")]);
    no_tmp_left(&disk, &p("/x2"));
}

#[test]
fn extract_here_replace_trashes_old() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/x8/a.zip"), &[("a.txt", b"nuevo")], 5);
    disk.add_file(&p("/x8/a.txt"), b"viejo".to_vec());
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x8/a.zip"), into_folder: false }, &[Answer { choice: Choice::Replace, all: false }], true, &mut |_, _| {});
    assert_eq!(disk.contents(&p("/x8/a.txt")).unwrap(), b"nuevo");
    assert_eq!(disk.trashed().len(), 1);
    assert!(matches!(&r.out.result.undo, Some(Undo::Batch(v)) if v.iter().any(|u| matches!(u, Undo::Recycled(_)))));
    no_tmp_left(&disk, &p("/x8"));
}

#[test]
fn extract_skips_escaping_entries() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/x3/in/m.zip"), &[("../evil.txt", b"x"), ("/etc/passwd", b"x"), ("C:\\x.txt", b"x"), ("ok.txt", b"ok")], 5);
    arch.add_link(&p("/x3/in/m.zip"), "l", Some("../../afuera"));
    arch.add_link(&p("/x3/in/m.zip"), "bien", Some("ok.txt"));
    for into_folder in [false, true] {
        let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x3/in/m.zip"), into_folder });
        assert_eq!(r.messages(), [NOT_ALLOWED; 4], "{into_folder}");
        assert!(r.out.errors.iter().all(|e| !e.retry));
        // Con el nombre que trae el archivo (la ruta del error es la del archivo).
        assert_eq!(r.out.errors[0].entry.as_deref(), Some("../evil.txt"));
        assert_eq!(r.out.errors[1].entry.as_deref(), Some("/etc/passwd"));
        assert_eq!(r.out.errors[0].src, p("/x3/in/m.zip"));
    }
    // Nada fuera de la carpeta del archivo.
    let outside: Vec<_> = disk.list_all(&p("/")).into_iter().filter(|q| !q.starts_with("/x3/in")).filter(|q| q != &p("/x3")).collect();
    assert!(outside.is_empty(), "{outside:?}");
    assert_eq!(disk.contents(&p("/x3/in/ok.txt")).unwrap(), b"ok");
    assert_eq!(disk.link_target(&p("/x3/in/bien")), Some(p("ok.txt")));
    assert_eq!(disk.contents(&p("/x3/in/m/ok.txt")).unwrap(), b"ok");
    no_tmp_left(&disk, &p("/x3"));
}

#[test]
fn extract_progress_by_bytes() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.set_block(2);
    arch.add_archive(&p("/x4/a.zip"), &[("a.bin", &[1; 6]), ("b.bin", &[2; 4])], 5);
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x4/a.zip"), into_folder: true });
    assert!(r.updates.iter().any(|u| matches!(u, Update::Started { items: 2, bytes: 10, .. })), "{:?}", r.updates);
    let bytes: Vec<u64> = r.updates.iter().filter_map(|u| if let Update::Progress { bytes, .. } = u { Some(*bytes) } else { None }).collect();
    assert!(bytes.windows(2).all(|w| w[0] <= w[1]), "{bytes:?}");
    assert_eq!(bytes.last(), Some(&10));
    assert!(matches!(r.updates.iter().rev().find(|u| matches!(u, Update::Progress { .. })), Some(Update::Progress { items: 2, .. })));
}

#[test]
fn extract_cancel_removes_what_it_extracted() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.set_block(1);
    arch.add_archive(&p("/x5/a.zip"), &[("a.txt", b"aaaa"), ("b.txt", b"bbbb")], 5);
    for into_folder in [false, true] {
        let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x5/a.zip"), into_folder }, &[], true, &mut |u, ctl| {
            // Antes del primer bloque: lo escrito hasta ahí (la carpeta del motor) se borra.
            if matches!(u, Update::Started { .. }) {
                ctl.cancel();
            }
        });
        assert!(r.out.cancelled);
        assert!(r.out.result.undo.is_none());
        assert_eq!(disk.list_all(&p("/x5")), vec![p("/x5/a.zip")]);
    }
}

#[test]
fn extract_cancel_at_conflict_takes_back_what_arrived() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/x9/a.zip"), &[("a.txt", b"a"), ("b.txt", b"b")], 5);
    disk.add_file(&p("/x9/b.txt"), b"viejo".to_vec());
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x9/a.zip"), into_folder: false }, &[Answer { choice: Choice::Skip, all: false }], true, &mut |u, ctl| {
        if matches!(u, Update::Conflict(_)) {
            ctl.cancel();
        }
    });
    assert!(r.out.cancelled);
    assert_eq!(disk.list_all(&p("/x9")), vec![p("/x9/a.zip"), p("/x9/b.txt")]);
    assert_eq!(disk.contents(&p("/x9/b.txt")).unwrap(), b"viejo");
}

#[test]
fn extract_checks_space_and_asks_for_big_expansions() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/x6/a.zip"), &[("a.bin", &[0; 600])], 5);
    disk.set_volume(&p("/x6"), VolumeInfo { id: "u".into(), label: "USB".into(), case_insensitive: false, has_trash: true, max_file: None, free: 100 });
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x6/a.zip"), into_folder: true });
    assert!(r.out.stopped.as_deref().is_some_and(|s| s.starts_with("Faltan ") && s.ends_with("en \"USB\"")), "{:?}", r.out.stopped);
    assert_eq!(disk.list_all(&p("/x6")), vec![p("/x6/a.zip")]);

    // 600 bytes de 5: más de 100 veces, pero poco: no se pregunta.
    disk.set_volume(&p("/x6"), VolumeInfo { id: "u".into(), label: "USB".into(), case_insensitive: false, has_trash: true, max_file: None, free: u64::MAX });
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x6/a.zip"), into_folder: true });
    assert!(!r.updates.iter().any(|u| matches!(u, Update::Confirm(_))));
    assert!(disk.exists(&p("/x6/a/a.bin")));

    // Más de 100 veces y más de 1 GiB: se pregunta; «No» no extrae nada.
    arch.add_archive(&p("/x6/b.zip"), &[("b.bin", b"b")], 5);
    arch.set_declared(&p("/x6/b.zip"), "b.bin", 2 << 30);
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x6/b.zip"), into_folder: true }, &[], false, &mut |_, _| {});
    let asked: Vec<&String> = r.updates.iter().filter_map(|u| if let Update::Confirm(q) = u { Some(q) } else { None }).collect();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].starts_with("Este archivo se expande a ") && asked[0].ends_with(". ¿Seguir?"), "{}", asked[0]);
    assert!(r.out.cancelled);
    assert!(!disk.exists(&p("/x6/b")));
    // «Sí» sigue.
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x6/b.zip"), into_folder: true });
    assert!(r.out.errors.is_empty() && !r.out.cancelled);
    assert!(disk.exists(&p("/x6/b/b.bin")));
    // Más de 10 GiB aunque el archivo sea grande.
    arch.add_archive(&p("/x6/c.zip"), &[("c.bin", b"c")], 5);
    arch.set_declared(&p("/x6/c.zip"), "c.bin", 11 << 30);
    disk.set_size(&p("/x6/c.zip"), 8 << 30);
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x6/c.zip"), into_folder: true }, &[], false, &mut |_, _| {});
    assert!(r.updates.iter().any(|u| matches!(u, Update::Confirm(_))));
    // Muchísimas entradas (vacías): también.
    let many: Vec<String> = (0..100_001).map(|i| format!("d/f{i}")).collect();
    let entries: Vec<(&str, &[u8])> = many.iter().map(|n| (n.as_str(), &b""[..])).collect();
    arch.add_archive(&p("/x6/m.zip"), &entries, 5);
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/x6/m.zip"), into_folder: true }, &[], false, &mut |_, _| {});
    assert!(r.updates.iter().any(|u| matches!(u, Update::Confirm(_))));
    // 20.000 vacías: no.
    let entries: Vec<(&str, &[u8])> = many.iter().take(20_000).map(|n| (n.as_str(), &b""[..])).collect();
    arch.add_archive(&p("/x6/n.zip"), &entries, 5);
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x6/n.zip"), into_folder: true });
    assert!(!r.updates.iter().any(|u| matches!(u, Update::Confirm(_))));
}

#[test]
fn extract_here_removes_links_that_end_up_outside() {
    // Un enlace del usuario apunta afuera; el del archivo era válido en la carpeta del motor,
    // pero al llegar resuelve por el del usuario.
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_file(&p("/l1/outside/secret"), b"s".to_vec());
    disk.add_symlink(&p("/l1/in/b"), Path::new("../outside"));
    arch.add_archive(&p("/l1/in/m.zip"), &[("b/", b""), ("b/secret", b"decoy"), ("d/", b"")], 5);
    arch.add_link(&p("/l1/in/m.zip"), "y", Some("b/secret"));
    arch.add_link(&p("/l1/in/m.zip"), "d/z", Some("../b/secret"));
    arch.add_link(&p("/l1/in/m.zip"), "bien", Some("d"));
    let keep = Answer { choice: Choice::KeepBoth, all: false };
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/l1/in/m.zip"), into_folder: false }, &[keep], true, &mut |_, _| {});
    assert!(!disk.exists(&p("/l1/in/y")) && !disk.exists(&p("/l1/in/d/z")));
    assert_eq!(disk.link_target(&p("/l1/in/bien")), Some(p("d")));
    assert_eq!(disk.contents(&p("/l1/in/b 2/secret")).unwrap(), b"decoy");
    let mut bad: Vec<_> = r.out.errors.iter().map(|e| (e.entry.clone().unwrap_or_default(), e.message.clone())).collect();
    bad.sort();
    assert_eq!(bad, [("d/z".to_string(), NOT_ALLOWED.to_string()), ("y".to_string(), NOT_ALLOWED.to_string())]);
    assert!(!r.out.result.select.contains(&p("/l1/in/y")));
    // Lo mismo después de combinar con una carpeta que ya estaba.
    disk.add_dir(&p("/l2/in/d"));
    disk.add_symlink(&p("/l2/in/d/b"), Path::new("../../outside"));
    arch.add_archive(&p("/l2/in/m.zip"), &[("d/", b"")], 5);
    arch.add_link(&p("/l2/in/m.zip"), "d/y", Some("b"));
    let merge = Answer { choice: Choice::Merge, all: false };
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/l2/in/m.zip"), into_folder: false }, &[merge], true, &mut |_, _| {});
    assert!(!disk.exists(&p("/l2/in/d/y")));
    assert_eq!(r.messages(), [NOT_ALLOWED]);
}

#[test]
fn extract_into_single_top_folder() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/s1/carpeta.zip"), &[("carpeta/", b""), ("carpeta/a.txt", b"a"), ("__MACOSX/", b""), ("__MACOSX/carpeta/._a.txt", b"x")], 5);
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/s1/carpeta.zip"), into_folder: true });
    assert_eq!(disk.contents(&p("/s1/carpeta/a.txt")).unwrap(), b"a");
    assert_eq!(disk.list_all(&p("/s1")), vec![p("/s1/carpeta"), p("/s1/carpeta/a.txt"), p("/s1/carpeta.zip")]);
    assert_eq!(r.out.result.select, vec![p("/s1/carpeta")]);
    // Otra vez: el nombre libre.
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/s1/carpeta.zip"), into_folder: true });
    assert_eq!(r.out.result.select, vec![p("/s1/carpeta 2")]);
    assert!(disk.exists(&p("/s1/carpeta 2/a.txt")));
    // «Extraer aquí» tampoco trae __MACOSX.
    disk.remove_all(&p("/s1/carpeta"));
    simple(&disk, &arch, JobSpec::Extract { archive: p("/s1/carpeta.zip"), into_folder: false });
    assert!(disk.exists(&p("/s1/carpeta/a.txt")) && !disk.exists(&p("/s1/__MACOSX")));
    no_tmp_left(&disk, &p("/s1"));
}

#[test]
fn extract_here_retries_a_busy_rename() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    arch.add_archive(&p("/r1/a.zip"), &[("a.txt", b"a")], 5);
    disk.fail(&p("/r1/a.txt"), crate::sys::ErrKind::InUse, 1);
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/r1/a.zip"), into_folder: false });
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert_eq!(disk.contents(&p("/r1/a.txt")).unwrap(), b"a");
}

#[test]
fn compress_walk_errors_abort_without_zip() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_file(&p("/z5/F/sub/a.txt"), b"a".to_vec());
    disk.add_file(&p("/z5/F/b.txt"), b"b".to_vec());
    disk.fail(&p("/z5/F/sub"), crate::sys::ErrKind::Denied, 1);
    let r = simple(&disk, &arch, JobSpec::Compress { sources: vec![p("/z5/F")] });
    assert_eq!(r.messages(), ["No tenés permiso"]);
    assert!(r.out.errors[0].retry);
    assert!(!disk.exists(&p("/z5/F.zip")));
    no_tmp_left(&disk, &p("/z5"));
    // Reintentar (el mismo trabajo) va al mismo nombre.
    let r = simple(&disk, &arch, JobSpec::Compress { sources: vec![p("/z5/F")] });
    assert_eq!(r.out.result.select, vec![p("/z5/F.zip")]);
}

#[test]
fn extract_reports_omitted_and_password() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::windows_style(&disk);
    arch.add_archive(&p("/x7/a.zip"), &[("a.txt", b"a")], 5);
    arch.add_link(&p("/x7/a.zip"), "enlace", Some("a.txt"));
    arch.add_link(&p("/x7/a.zip"), "duro", None);
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x7/a.zip"), into_folder: true });
    assert_eq!(r.out.omitted, 2);
    assert!(r.out.errors.is_empty());
    assert!(disk.exists(&p("/x7/a/a.txt")) && !disk.exists(&p("/x7/a/enlace")));

    arch.add_archive(&p("/x7/c.zip"), &[("s.txt", b"s")], 5);
    arch.set_password(&p("/x7/c.zip"));
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/x7/c.zip"), into_folder: false });
    assert_eq!(r.messages(), [PASSWORD]);
    assert_eq!(r.out.errors[0].src, p("/x7/c.zip"));
    assert!(!disk.exists(&p("/x7/s.txt")));
    no_tmp_left(&disk, &p("/x7"));
}

#[test]
fn extract_here_checks_each_item_when_it_lands() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_file(&p("/w1/outside/secret"), b"s".to_vec());
    disk.add_symlink(&p("/w1/in/b"), Path::new("../outside"));
    disk.add_file(&p("/w1/in/z.txt"), b"mine".to_vec());
    arch.add_archive(&p("/w1/in/m.zip"), &[("z.txt", b"theirs")], 5);
    arch.add_link(&p("/w1/in/m.zip"), "a_link", Some("b/secret"));
    arch.add_link(&p("/w1/in/m.zip"), "l", Some("q/x"));
    let d = disk.clone();
    let mut visible = None;
    let skip = Answer { choice: Choice::Skip, all: false };
    let r = go(&disk, &arch, JobSpec::Extract { archive: p("/w1/in/m.zip"), into_folder: false }, &[skip], true, &mut |u, _| {
        if matches!(u, Update::Conflict(_)) {
            // Mientras se pregunta, el enlace que sale ya no está a la vista.
            visible = Some(d.exists(&p("/w1/in/a_link")));
            // El usuario pone uno suyo con ese nombre y un enlace suyo que hace salir a «l».
            d.add_symlink(&p("/w1/in/a_link"), Path::new("../outside/secret"));
            d.add_symlink(&p("/w1/in/q"), Path::new("../outside"));
        }
    });
    assert_eq!(visible, Some(false));
    // Lo del usuario no se toca; «l» (de este trabajo) se borra al final.
    assert_eq!(disk.link_target(&p("/w1/in/a_link")), Some(p("../outside/secret")));
    assert_eq!(disk.link_target(&p("/w1/in/q")), Some(p("../outside")));
    assert!(!disk.exists(&p("/w1/in/l")));
    let mut bad: Vec<_> = r.out.errors.iter().map(|e| e.entry.clone().unwrap_or_default()).collect();
    bad.sort();
    assert_eq!(bad, ["a_link", "l"]);
}

#[test]
fn extract_here_unreadable_arrival_is_reported_not_deleted() {
    let disk = FakeCopyPrims::new();
    let arch = FakeArchives::new(&disk);
    disk.add_dir(&p("/w2/outside"));
    disk.add_symlink(&p("/w2/in/b"), Path::new("../outside"));
    arch.add_archive(&p("/w2/in/m.zip"), &[("d/", b"")], 5);
    arch.add_link(&p("/w2/in/m.zip"), "d/z", Some("../b/x"));
    // Ya en su lugar, no se puede listar.
    disk.fail_list(&p("/w2/in/d"), crate::sys::ErrKind::InUse, 1);
    let r = simple(&disk, &arch, JobSpec::Extract { archive: p("/w2/in/m.zip"), into_folder: false });
    assert!(r.out.errors.iter().any(|e| e.entry.as_deref() == Some("d") && e.message == "Lo está usando otro programa"), "{:?}", r.out.errors);
    // Lo que no se pudo mirar no se borra a ciegas (y se avisa).
    assert!(disk.exists(&p("/w2/in/d/z")));
    assert!(!r.messages().contains(&NOT_ALLOWED.to_string()));
}

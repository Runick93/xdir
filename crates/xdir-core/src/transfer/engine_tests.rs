//! Pruebas del motor de copia con el disco falso (`FakeCopyPrims`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::engine::{run, title};
use super::{Answer, Choice, ConflictInfo, Control, JobOutcome, JobSpec, Update};
use crate::ops::Undo;
use crate::sys::testing::{FakeCopyPrims, install_fakes};
use crate::sys::{ErrKind, VolumeInfo};

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

/// Corre un trabajo respondiendo los conflictos con `answers`, en orden.
fn go(fake: &FakeCopyPrims, spec: JobSpec, answers: &[Answer]) -> Ran {
    go_with(fake, spec, answers, &Control::new(), &mut |_, _| {})
}

fn go_with(fake: &FakeCopyPrims, spec: JobSpec, answers: &[Answer], ctl: &Arc<Control>, hook: &mut dyn FnMut(&Update, &Arc<Control>)) -> Ran {
    install_fakes();
    let mut answers = answers.iter().copied();
    let mut updates = Vec::new();
    run(spec, fake, ctl, &mut |u| {
        if let Update::Conflict(_) = &u {
            ctl.answer(answers.next().expect("conflicto sin respuesta"));
        }
        hook(&u, ctl);
        updates.push(u);
    });
    let out = match updates.last() {
        Some(Update::Finished(o)) => o.clone(),
        other => panic!("el trabajo no terminó con Finished: {other:?}"),
    };
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::Finished(_))).count(), 1);
    Ran { updates, out }
}

fn answer(choice: Choice) -> Answer {
    Answer { choice, all: false }
}

fn volume(id: &str, label: &str) -> VolumeInfo {
    VolumeInfo { id: id.into(), label: label.into(), case_insensitive: false, has_trash: true, max_file: None, free: u64::MAX }
}

fn no_tmp_left(fake: &FakeCopyPrims, dir: &Path) {
    let left: Vec<_> = fake.list_all(dir).into_iter().filter(|p| p.to_string_lossy().contains(".xdir-")).collect();
    assert!(left.is_empty(), "quedaron temporales: {left:?}");
}

#[test]
fn copy_tree_and_undo() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/c1/src/Fotos/a.jpg"), b"aaa".to_vec());
    fake.add_file(&p("/c1/src/Fotos/sub/b.jpg"), b"bbbb".to_vec());
    fake.add_file(&p("/c1/src/n.txt"), b"n".to_vec());
    fake.add_dir(&p("/c1/dst"));
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/c1/src/Fotos"), p("/c1/src/n.txt")], dest: p("/c1/dst") }, &[]);
    assert_eq!(fake.contents(&p("/c1/dst/Fotos/a.jpg")).unwrap(), b"aaa");
    assert_eq!(fake.contents(&p("/c1/dst/Fotos/sub/b.jpg")).unwrap(), b"bbbb");
    assert_eq!(fake.contents(&p("/c1/dst/n.txt")).unwrap(), b"n");
    // El origen sigue.
    assert!(fake.exists(&p("/c1/src/Fotos/sub/b.jpg")));
    let tops = vec![p("/c1/dst/Fotos"), p("/c1/dst/n.txt")];
    assert_eq!(r.out.result.undo, Some(Undo::Batch(vec![Undo::Created(tops.clone())])));
    assert_eq!(r.out.result.select, tops);
    assert_eq!(r.out.result.focus, Some(p("/c1/dst/n.txt")));
    assert!(r.out.errors.is_empty() && !r.out.cancelled && r.out.stopped.is_none());
    assert!(r.updates.iter().any(|u| matches!(u, Update::Started { items: 5, bytes: 8, .. })), "{:?}", r.updates);
}

#[test]
fn copy_into_same_folder_duplicates() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/c2/a.txt"), b"x".to_vec());
    fake.add_file(&p("/c2/Dir/in.txt"), b"y".to_vec());
    let spec = || JobSpec::Copy { sources: vec![p("/c2/a.txt"), p("/c2/Dir")], dest: p("/c2") };
    let r = go(&fake, spec(), &[]);
    assert!(r.conflicts().is_empty());
    assert_eq!(fake.contents(&p("/c2/a copia.txt")).unwrap(), b"x");
    assert_eq!(fake.contents(&p("/c2/Dir copia/in.txt")).unwrap(), b"y");
    go(&fake, spec(), &[]);
    assert!(fake.exists(&p("/c2/a copia 2.txt")));
    assert!(fake.exists(&p("/c2/Dir copia 2/in.txt")));
    // Duplicar usa los mismos nombres.
    let r = go(&fake, JobSpec::Duplicate { sources: vec![p("/c2/a.txt")] }, &[]);
    assert_eq!(r.out.result.select, vec![p("/c2/a copia 3.txt")]);
    assert_eq!(fake.contents(&p("/c2/a.txt")).unwrap(), b"x");
}

#[test]
fn copy_onto_itself_case_insensitive_duplicates() {
    // «Pegar» en la misma carpeta escrita con otras mayúsculas: nunca es un conflicto consigo mismo.
    let fake = FakeCopyPrims::new();
    fake.set_volume(&p("/ci0"), VolumeInfo { case_insensitive: true, ..volume("ci0", "CI") });
    fake.add_file(&p("/ci0/Docs/a.txt"), b"x".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/ci0/Docs/a.txt")], dest: p("/ci0/docs") }, &[]);
    assert!(r.conflicts().is_empty());
    assert_eq!(fake.contents(&p("/ci0/Docs/a.txt")).unwrap(), b"x");
    assert!(fake.exists(&p("/ci0/Docs/a copia.txt")));
    // Mover a la misma carpeta (con otras mayúsculas) no hace nada.
    let r = go(&fake, JobSpec::Move { sources: vec![p("/ci0/Docs/a.txt")], dest: p("/ci0/DOCS") }, &[]);
    assert!(r.conflicts().is_empty() && r.out.errors.is_empty());
    assert_eq!(fake.contents(&p("/ci0/Docs/a.txt")).unwrap(), b"x");
    assert_eq!(fake.trashed(), vec![]);
}

#[test]
fn copy_into_itself_refused() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/c3/A/sub/f.txt"), b"f".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/c3/A")], dest: p("/c3/A/sub") }, &[]);
    assert_eq!(r.messages(), vec!["No se puede copiar una carpeta dentro de sí misma".to_string()]);
    assert_eq!(fake.list_all(&p("/c3/A/sub")), vec![p("/c3/A/sub/f.txt")]);
    let r = go(&fake, JobSpec::Move { sources: vec![p("/c3/A")], dest: p("/c3/A") }, &[]);
    assert_eq!(r.messages(), vec!["No se puede copiar una carpeta dentro de sí misma".to_string()]);
    assert!(fake.exists(&p("/c3/A/sub/f.txt")));
}

#[test]
fn move_same_volume_never_copies() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/m1/a/nube.bin"), vec![7; 5000]);
    fake.set_cloud_only(&p("/m1/a/nube.bin"));
    fake.add_file(&p("/m1/a/Dir/x.bin"), vec![1; 10]);
    fake.add_dir(&p("/m1/b"));
    let r = go(&fake, JobSpec::Move { sources: vec![p("/m1/a/nube.bin"), p("/m1/a/Dir")], dest: p("/m1/b") }, &[]);
    assert_eq!(fake.reads(&p("/m1/a/nube.bin")), 0);
    assert_eq!(fake.reads(&p("/m1/a/Dir/x.bin")), 0);
    assert!(!fake.exists(&p("/m1/a/nube.bin")) && !fake.exists(&p("/m1/a/Dir")));
    assert_eq!(fake.contents(&p("/m1/b/nube.bin")).unwrap(), vec![7; 5000]);
    assert!(fake.exists(&p("/m1/b/Dir/x.bin")));
    assert_eq!(
        r.out.result.undo,
        Some(Undo::Batch(vec![Undo::Moved(vec![(p("/m1/b/nube.bin"), p("/m1/a/nube.bin")), (p("/m1/b/Dir"), p("/m1/a/Dir"))])]))
    );
    assert_eq!(r.out.result.select, vec![p("/m1/b/nube.bin"), p("/m1/b/Dir")]);
}

fn two_volumes(fake: &FakeCopyPrims) {
    fake.set_volume(&p("/v1"), volume("v1", "Interno"));
    fake.set_volume(&p("/v2"), volume("v2", "USB"));
    fake.add_dir(&p("/v2/dst"));
}

#[test]
fn move_cross_volume_verifies_then_deletes() {
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/Dir/x.txt"), b"xx".to_vec());
    fake.add_file(&p("/v1/a/Dir/sub/y.txt"), b"yyy".to_vec());
    fake.add_file(&p("/v1/a/z.txt"), b"z".to_vec());
    let r = go(&fake, JobSpec::Move { sources: vec![p("/v1/a/Dir"), p("/v1/a/z.txt")], dest: p("/v2/dst") }, &[]);
    assert!(r.out.errors.is_empty(), "{:?}", r.out.errors);
    assert_eq!(fake.contents(&p("/v2/dst/Dir/x.txt")).unwrap(), b"xx");
    assert_eq!(fake.contents(&p("/v2/dst/Dir/sub/y.txt")).unwrap(), b"yyy");
    assert_eq!(fake.contents(&p("/v2/dst/z.txt")).unwrap(), b"z");
    assert_eq!(fake.list_all(&p("/v1/a")), Vec::<PathBuf>::new());
    assert_eq!(
        r.out.result.undo,
        Some(Undo::Batch(vec![Undo::Moved(vec![(p("/v2/dst/Dir"), p("/v1/a/Dir")), (p("/v2/dst/z.txt"), p("/v1/a/z.txt"))])]))
    );
}

#[test]
fn move_cross_volume_keeps_failed_sources() {
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/Dir/ok.txt"), b"1".to_vec());
    fake.add_file(&p("/v1/a/Dir/sub/ok2.txt"), b"2".to_vec());
    fake.add_file(&p("/v1/a/Dir/abierto.txt"), b"3".to_vec());
    fake.fail(&p("/v1/a/Dir/abierto.txt"), ErrKind::InUse, 1);
    let r = go(&fake, JobSpec::Move { sources: vec![p("/v1/a/Dir")], dest: p("/v2/dst") }, &[]);
    // En el origen queda exactamente lo que no se movió (y su carpeta).
    assert_eq!(fake.list_all(&p("/v1/a")), vec![p("/v1/a/Dir"), p("/v1/a/Dir/abierto.txt")]);
    assert_eq!(fake.contents(&p("/v1/a/Dir/abierto.txt")).unwrap(), b"3");
    assert_eq!(fake.contents(&p("/v2/dst/Dir/ok.txt")).unwrap(), b"1");
    assert_eq!(fake.contents(&p("/v2/dst/Dir/sub/ok2.txt")).unwrap(), b"2");
    assert!(!fake.exists(&p("/v2/dst/Dir/abierto.txt")));
    assert_eq!(r.out.errors.len(), 1);
    assert_eq!(r.out.errors[0].message, "Lo está usando otro programa");
    assert_eq!(r.out.errors[0].src, p("/v1/a/Dir/abierto.txt"));
    assert_eq!(r.out.errors[0].dst, p("/v2/dst/Dir/abierto.txt"));
}

#[test]
fn move_size_mismatch_keeps_source() {
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/f.bin"), vec![9; 100]);
    fake.short_copy(&p("/v1/a/f.bin"), 60);
    let r = go(&fake, JobSpec::Move { sources: vec![p("/v1/a/f.bin")], dest: p("/v2/dst") }, &[]);
    assert_eq!(r.messages(), vec!["La copia no quedó igual".to_string()]);
    assert_eq!(fake.contents(&p("/v1/a/f.bin")).unwrap(), vec![9; 100]);
    assert!(!fake.exists(&p("/v2/dst/f.bin")));
    assert_eq!(r.out.result.undo, None);
}

fn conflict_setup(fake: &FakeCopyPrims) {
    fake.add_file(&p("/k/src/informe.pdf"), b"nuevo".to_vec());
    fake.add_file(&p("/k/dst/informe.pdf"), b"viejo".to_vec());
}

fn copy_informe() -> JobSpec {
    JobSpec::Copy { sources: vec![p("/k/src/informe.pdf")], dest: p("/k/dst") }
}

#[test]
fn conflict_keep_both_names() {
    let fake = FakeCopyPrims::new();
    conflict_setup(&fake);
    let r = go(&fake, copy_informe(), &[answer(Choice::KeepBoth)]);
    let c = &r.conflicts()[0];
    assert_eq!((c.name.as_str(), c.folder.as_str(), c.is_dir, c.can_merge), ("informe.pdf", "dst", false, false));
    assert_eq!((c.incoming.size, c.existing.size), (5, 5));
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"viejo");
    assert_eq!(fake.contents(&p("/k/dst/informe 2.pdf")).unwrap(), b"nuevo");
    assert_eq!(r.out.result.select, vec![p("/k/dst/informe 2.pdf")]);
}

#[test]
fn conflict_replace_trashes_old() {
    let fake = FakeCopyPrims::new();
    conflict_setup(&fake);
    let r = go(&fake, copy_informe(), &[answer(Choice::Replace)]);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"nuevo");
    let trashed = fake.trashed();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].1, p("/k/dst/informe.pdf"));
    no_tmp_left(&fake, &p("/k/dst"));
    let undo = r.out.result.undo.clone().unwrap();
    assert_eq!(
        undo,
        Undo::Batch(vec![Undo::Recycled(vec![(trashed[0].0.clone(), p("/k/dst/informe.pdf"))]), Undo::Created(vec![p("/k/dst/informe.pdf")])])
    );
    // Deshacer devuelve el viejo.
    let u = go(&fake, JobSpec::Undo(undo), &[]);
    assert!(u.out.errors.is_empty(), "{:?}", u.out.errors);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"viejo");
    assert_eq!(u.out.result.undo, None);
}

#[test]
fn conflict_replace_without_trash_deletes() {
    let fake = FakeCopyPrims::new();
    fake.set_volume(&p("/k"), VolumeInfo { has_trash: false, ..volume("k", "Red") });
    conflict_setup(&fake);
    let r = go(&fake, copy_informe(), &[answer(Choice::Replace)]);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"nuevo");
    assert!(fake.trashed().is_empty());
    no_tmp_left(&fake, &p("/k/dst"));
    assert_eq!(r.out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![p("/k/dst/informe.pdf")])])));
}

#[test]
fn replace_folder_trashes_whole_existing() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/kd/src/D/nuevo.txt"), b"n".to_vec());
    fake.add_file(&p("/kd/dst/D/viejo.txt"), b"v".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/kd/src/D")], dest: p("/kd/dst") }, &[answer(Choice::Replace)]);
    assert!(r.conflicts()[0].can_merge);
    assert_eq!(fake.list_all(&p("/kd/dst")), vec![p("/kd/dst/D"), p("/kd/dst/D/nuevo.txt")]);
    assert_eq!(fake.trashed()[0].1, p("/kd/dst/D"));
}

#[test]
fn replace_folder_without_trash_keeps_old_on_error() {
    // Sin Papelera, la carpeta vieja se aparta y se borra solo si la nueva quedó entera; si algo
    // falló, queda con otro nombre.
    let fake = FakeCopyPrims::new();
    fake.set_volume(&p("/kn"), VolumeInfo { has_trash: false, ..volume("kn", "Red") });
    fake.add_file(&p("/kn/src/D/a.txt"), b"a".to_vec());
    fake.add_file(&p("/kn/src/D/b.txt"), b"b".to_vec());
    fake.add_file(&p("/kn/dst/D/viejo.txt"), b"v".to_vec());
    fake.fail(&p("/kn/src/D/b.txt"), ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/kn/src/D")], dest: p("/kn/dst") }, &[answer(Choice::Replace)]);
    assert_eq!(fake.contents(&p("/kn/dst/D 2/viejo.txt")).unwrap(), b"v");
    assert_eq!(fake.contents(&p("/kn/dst/D/a.txt")).unwrap(), b"a");
    assert_eq!(r.out.errors.len(), 2, "{:?}", r.out.errors);
    no_tmp_left(&fake, &p("/kn/dst"));
    // Si todo sale bien, la vieja se borra.
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/kn/src/D")], dest: p("/kn/dst") }, &[answer(Choice::Replace)]);
    assert!(r.out.errors.is_empty(), "{:?}", r.out.errors);
    assert_eq!(fake.list_all(&p("/kn/dst/D")), vec![p("/kn/dst/D/a.txt"), p("/kn/dst/D/b.txt")]);
    no_tmp_left(&fake, &p("/kn/dst"));
}

#[test]
fn replace_cancel_keeps_original() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/k/src/informe.pdf"), vec![1; 10_000]);
    fake.add_file(&p("/k/dst/informe.pdf"), b"viejo".to_vec());
    fake.set_block(100);
    let ctl = Control::new();
    let r = go_with(&fake, copy_informe(), &[answer(Choice::Replace)], &ctl, &mut |u, ctl| {
        if let Update::Progress { bytes, .. } = u
            && *bytes > 0
        {
            ctl.cancel();
        }
    });
    assert!(r.out.cancelled);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"viejo");
    assert!(fake.trashed().is_empty());
    no_tmp_left(&fake, &p("/k/dst"));
    assert_eq!(r.out.result.undo, None);
}

#[test]
fn replace_disk_full_keeps_original() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/k/src/informe.pdf"), vec![1; 10_000]);
    fake.add_file(&p("/k/dst/informe.pdf"), b"viejo".to_vec());
    fake.fail_after(&p("/k/src/informe.pdf"), 4000, ErrKind::DiskFull);
    let r = go(&fake, copy_informe(), &[answer(Choice::Replace)]);
    assert_eq!(r.out.stopped.as_deref(), Some("El disco está lleno"));
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"viejo");
    assert!(fake.trashed().is_empty());
    no_tmp_left(&fake, &p("/k/dst"));
}

#[test]
fn conflict_merge_folders() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/g/s/D/a.txt"), b"a nuevo".to_vec());
    fake.add_file(&p("/g/s/D/b.txt"), b"b".to_vec());
    fake.add_file(&p("/g/d/D/a.txt"), b"a viejo".to_vec());
    fake.add_file(&p("/g/d/D/c.txt"), b"c".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/g/s/D")], dest: p("/g/d") }, &[answer(Choice::Merge), answer(Choice::Replace)]);
    let c = r.conflicts();
    assert_eq!(c.len(), 2);
    assert!(c[0].is_dir && c[0].can_merge && c[0].name == "D");
    assert!(!c[1].is_dir && c[1].name == "a.txt" && c[1].folder == "D");
    assert_eq!(fake.contents(&p("/g/d/D/a.txt")).unwrap(), b"a nuevo");
    assert_eq!(fake.contents(&p("/g/d/D/b.txt")).unwrap(), b"b");
    assert_eq!(fake.contents(&p("/g/d/D/c.txt")).unwrap(), b"c");
    let Some(Undo::Batch(steps)) = r.out.result.undo else { panic!() };
    assert!(matches!(&steps[0], Undo::Recycled(v) if v[0].1 == p("/g/d/D/a.txt")));
    assert_eq!(steps[1], Undo::Created(vec![p("/g/d/D/a.txt"), p("/g/d/D/b.txt")]));
    assert_eq!(r.out.result.select, vec![p("/g/d/D")]);
}

#[test]
fn apply_to_all_separate_for_files_and_dirs() {
    let fake = FakeCopyPrims::new();
    for top in ["D1", "D2"] {
        fake.add_file(&p(&format!("/aa/s/{top}/nuevo.txt")), b"n".to_vec());
        fake.add_dir(&p(&format!("/aa/d/{top}")));
    }
    for f in ["f1.txt", "f2.txt"] {
        fake.add_file(&p(&format!("/aa/s/{f}")), b"nuevo".to_vec());
        fake.add_file(&p(&format!("/aa/d/{f}")), b"viejo".to_vec());
    }
    let sources = ["D1", "D2", "f1.txt", "f2.txt"].iter().map(|n| p(&format!("/aa/s/{n}"))).collect();
    let all = |choice| Answer { choice, all: true };
    let r = go(&fake, JobSpec::Copy { sources, dest: p("/aa/d") }, &[all(Choice::Merge), all(Choice::KeepBoth)]);
    let c = r.conflicts();
    assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["D1", "f1.txt"]);
    assert!(fake.exists(&p("/aa/d/D2/nuevo.txt")));
    assert_eq!(fake.contents(&p("/aa/d/f2 2.txt")).unwrap(), b"nuevo");
    assert_eq!(fake.contents(&p("/aa/d/f2.txt")).unwrap(), b"viejo");
    // Quedaban conflictos por delante cuando se preguntó el primero.
    assert!(c[0].remaining >= 3, "{}", c[0].remaining);
}

#[test]
fn conflict_case_insensitive_volume() {
    let fake = FakeCopyPrims::new();
    fake.set_volume(&p("/ci"), VolumeInfo { case_insensitive: true, ..volume("ci", "APFS") });
    fake.add_file(&p("/ci/s/Foto.JPG"), b"nueva".to_vec());
    fake.add_file(&p("/ci/d/foto.jpg"), b"vieja".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/ci/s/Foto.JPG")], dest: p("/ci/d") }, &[answer(Choice::Skip)]);
    assert_eq!(r.conflicts().len(), 1);
    assert_eq!(fake.contents(&p("/ci/d/foto.jpg")).unwrap(), b"vieja");
    assert_eq!(fake.list_all(&p("/ci/d")), vec![p("/ci/d/foto.jpg")]);
    // En un disco que distingue mayúsculas son dos archivos.
    fake.add_file(&p("/cs/s/Foto.JPG"), b"nueva".to_vec());
    fake.add_file(&p("/cs/d/foto.jpg"), b"vieja".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/cs/s/Foto.JPG")], dest: p("/cs/d") }, &[]);
    assert!(r.conflicts().is_empty());
    assert_eq!(fake.list_all(&p("/cs/d")), vec![p("/cs/d/Foto.JPG"), p("/cs/d/foto.jpg")]);
}

fn three_files(fake: &FakeCopyPrims, root: &str) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = ["1.txt", "2.txt", "3.txt"].iter().map(|n| p(&format!("{root}/s/{n}"))).collect();
    for f in &files {
        fake.add_file(f, vec![5; 1000]);
    }
    fake.add_dir(&p(&format!("{root}/d")));
    files
}

#[test]
fn errors_collected_and_continue() {
    let fake = FakeCopyPrims::new();
    let files = three_files(&fake, "/e");
    fake.fail(&files[1], ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: files.clone(), dest: p("/e/d") }, &[]);
    assert!(fake.exists(&p("/e/d/1.txt")) && fake.exists(&p("/e/d/3.txt")) && !fake.exists(&p("/e/d/2.txt")));
    assert_eq!(r.out.errors.len(), 1);
    assert_eq!((r.out.errors[0].src.clone(), r.out.errors[0].dst.clone()), (files[1].clone(), p("/e/d/2.txt")));
    assert_eq!(r.out.errors[0].message, "No tenés permiso");
    assert!(r.out.stopped.is_none());
    assert_eq!(r.out.result.select, vec![p("/e/d/1.txt"), p("/e/d/3.txt")]);
}

#[test]
fn retry_only_failed_pairs() {
    let fake = FakeCopyPrims::new();
    let files = three_files(&fake, "/rt");
    fake.fail(&files[1], ErrKind::InUse, 1);
    let r = go(&fake, JobSpec::Copy { sources: files.clone(), dest: p("/rt/d") }, &[]);
    let pairs: Vec<_> = r.out.errors.iter().map(|e| (e.src.clone(), e.dst.clone())).collect();
    let again = go(&fake, JobSpec::Retry { moving: false, pairs }, &[]);
    assert!(again.out.errors.is_empty());
    assert!(fake.exists(&p("/rt/d/2.txt")));
    assert_eq!((fake.reads(&files[0]), fake.reads(&files[2])), (1, 1));
    assert_eq!(again.out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![p("/rt/d/2.txt")])])));
}

#[test]
fn disk_full_stops() {
    let fake = FakeCopyPrims::new();
    let files = three_files(&fake, "/df");
    fake.fail_after(&files[1], 300, ErrKind::DiskFull);
    let r = go(&fake, JobSpec::Copy { sources: files, dest: p("/df/d") }, &[]);
    assert_eq!(r.out.stopped.as_deref(), Some("El disco está lleno"));
    assert_eq!(fake.list_all(&p("/df/d")), vec![p("/df/d/1.txt")]);
    assert_eq!(r.out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![p("/df/d/1.txt")])])));
}

#[test]
fn not_enough_space_refuses() {
    let fake = FakeCopyPrims::new();
    const GIB: u64 = 1 << 30;
    fake.set_volume(&p("/ns/d"), VolumeInfo { free: GIB, ..volume("e", "Backup (E:)") });
    fake.add_file(&p("/ns/s/grande.iso"), vec![0; 10]);
    fake.set_size(&p("/ns/s/grande.iso"), GIB + GIB * 16 / 5 + 1);
    fake.add_dir(&p("/ns/d"));
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/ns/s/grande.iso")], dest: p("/ns/d") }, &[]);
    // `FakeEnv::format_size_short` escribe como Windows: tres cifras («3,20 GB»).
    assert_eq!(r.out.stopped.as_deref(), Some("Faltan 3,20 GB en \"Backup (E:)\""));
    assert!(!r.updates.iter().any(|u| matches!(u, Update::Started { .. })));
    assert!(fake.list_all(&p("/ns/d")).is_empty());
    assert_eq!(fake.reads(&p("/ns/s/grande.iso")), 0);
    // Mover en el mismo disco no escribe bytes: no hace falta espacio.
    fake.set_volume(&p("/ns"), VolumeInfo { free: 0, ..volume("ns", "Interno") });
    fake.add_file(&p("/ns/m/a.bin"), vec![1; 100]);
    fake.add_dir(&p("/ns/m2"));
    let r = go(&fake, JobSpec::Move { sources: vec![p("/ns/m/a.bin")], dest: p("/ns/m2") }, &[]);
    assert!(r.out.stopped.is_none());
    assert!(fake.exists(&p("/ns/m2/a.bin")));
}

#[test]
fn fat32_big_file_error_upfront() {
    let fake = FakeCopyPrims::new();
    fake.set_volume(&p("/fat"), VolumeInfo { max_file: Some((4 << 30) - 1), ..volume("fat", "USB") });
    fake.add_file(&p("/f32/s/peli.mkv"), vec![0; 10]);
    fake.set_size(&p("/f32/s/peli.mkv"), 5 << 30);
    fake.add_file(&p("/f32/s/chico.txt"), b"c".to_vec());
    fake.add_dir(&p("/fat/d"));
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/f32/s/peli.mkv"), p("/f32/s/chico.txt")], dest: p("/fat/d") }, &[]);
    assert_eq!(r.messages(), vec!["Es demasiado grande para el formato del disco de destino (FAT32)".to_string()]);
    assert_eq!(fake.reads(&p("/f32/s/peli.mkv")), 0);
    assert!(fake.exists(&p("/fat/d/chico.txt")) && !fake.exists(&p("/fat/d/peli.mkv")));
    // El error figura de entrada, antes de copiar nada.
    let first_progress = r.updates.iter().position(|u| matches!(u, Update::Progress { .. })).unwrap();
    assert!(r.updates.iter().position(|u| matches!(u, Update::Started { .. })).unwrap() < first_progress);
}

#[test]
fn pause_and_cancel_midfile() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/pc/s/1.txt"), vec![1; 50]);
    fake.add_file(&p("/pc/s/2.bin"), vec![2; 3000]);
    fake.add_dir(&p("/pc/d"));
    fake.set_block(10);
    fake.set_delay(Some(Duration::from_millis(2)));
    let ctl = Control::new();
    let count = Arc::new(AtomicUsize::new(0));
    let paused_at = Arc::new(AtomicUsize::new(usize::MAX));
    let mut helper = None;
    let r = go_with(&fake, JobSpec::Copy { sources: vec![p("/pc/s/1.txt"), p("/pc/s/2.bin")], dest: p("/pc/d") }, &[], &ctl, &mut |u, ctl| {
        if let Update::Progress { bytes, current, .. } = u {
            let n = count.fetch_add(1, Ordering::SeqCst) + 1;
            if current == "2.bin" && *bytes > 50 && helper.is_none() {
                ctl.pause();
                paused_at.store(n, Ordering::SeqCst);
                let (count, c) = (count.clone(), ctl.clone());
                helper = Some(std::thread::spawn(move || {
                    let before = count.load(Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(250));
                    let after = count.load(Ordering::SeqCst);
                    c.cancel();
                    (before, after)
                }));
            }
        }
    });
    let (before, after) = helper.unwrap().join().unwrap();
    assert_eq!(before, after, "en pausa siguieron llegando avisos");
    assert!(r.out.cancelled);
    assert!(fake.exists(&p("/pc/d/1.txt")));
    assert!(!fake.exists(&p("/pc/d/2.bin")), "quedó el archivo a medio copiar");
    assert_eq!(r.out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![p("/pc/d/1.txt")])])));
}

#[test]
fn progress_throttled_and_final() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/pt/s/a.bin"), vec![1; 4000]);
    fake.add_file(&p("/pt/s/b.bin"), vec![1; 2000]);
    fake.add_dir(&p("/pt/d"));
    fake.set_block(20);
    fake.set_delay(Some(Duration::from_millis(1)));
    let start = Instant::now();
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/pt/s/a.bin"), p("/pt/s/b.bin")], dest: p("/pt/d") }, &[]);
    let elapsed = start.elapsed().as_millis() as usize;
    let progress: Vec<_> = r.updates.iter().filter(|u| matches!(u, Update::Progress { .. })).collect();
    assert!(progress.len() <= elapsed / 100 + 3, "{} avisos en {elapsed} ms", progress.len());
    assert!(progress.len() >= 2);
    let n = r.updates.len();
    assert!(matches!(&r.updates[n - 2], Update::Progress { items: 2, bytes: 6000, .. }), "{:?}", r.updates[n - 2]);
}

#[test]
fn new_folder_with_job() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/nf/a.txt"), b"a".to_vec());
    fake.add_file(&p("/nf/b.txt"), b"b".to_vec());
    let items = vec![p("/nf/a.txt"), p("/nf/b.txt")];
    let r = go(&fake, JobSpec::NewFolderWith { parent: p("/nf"), items: items.clone() }, &[]);
    let folder = p("/nf/Carpeta nueva con 2 elementos");
    assert_eq!(fake.list_all(&p("/nf")), vec![folder.clone(), folder.join("a.txt"), folder.join("b.txt")]);
    assert!(r.out.result.rename_after);
    assert_eq!(r.out.result.focus, Some(folder.clone()));
    assert_eq!(
        r.out.result.undo,
        Some(Undo::Batch(vec![
            Undo::CreatedFolder(folder.clone()),
            Undo::Moved(vec![(folder.join("a.txt"), p("/nf/a.txt")), (folder.join("b.txt"), p("/nf/b.txt"))]),
        ]))
    );
    // Deshacer devuelve los elementos y manda la carpeta vacía a la Papelera.
    go(&fake, JobSpec::Undo(r.out.result.undo.unwrap()), &[]);
    assert_eq!(fake.list_all(&p("/nf")), items);
    assert_eq!(fake.trashed().iter().map(|t| t.1.clone()).collect::<Vec<_>>(), vec![folder.clone()]);
    // Ya existe: « 2».
    fake.add_dir(&folder);
    go(&fake, JobSpec::NewFolderWith { parent: p("/nf"), items }, &[]);
    assert!(fake.exists(&p("/nf/Carpeta nueva con 2 elementos 2/a.txt")));
}

#[test]
fn new_folder_with_nothing_moved_removes_folder() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/nf0/a.txt"), b"a".to_vec());
    fake.fail(&p("/nf0/a.txt"), ErrKind::InUse, 1);
    let r = go(&fake, JobSpec::NewFolderWith { parent: p("/nf0"), items: vec![p("/nf0/a.txt")] }, &[]);
    assert_eq!(fake.list_all(&p("/nf0")), vec![p("/nf0/a.txt")]);
    assert_eq!(r.out.result.undo, None);
    assert!(!r.out.result.rename_after);
    assert_eq!(r.messages(), vec!["Lo está usando otro programa".to_string()]);
}

#[test]
fn restore_job_uses_trash_origin() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/r/.Trash/x 3.txt"), b"x".to_vec());
    fake.set_trash_origin(&p("/r/.Trash/x 3.txt"), &p("/r/Docs/sub/x.txt"));
    fake.add_file(&p("/r/.Trash/perdido.txt"), b"?".to_vec());
    let r = go(&fake, JobSpec::Restore { items: vec![p("/r/.Trash/x 3.txt"), p("/r/.Trash/perdido.txt")] }, &[]);
    assert_eq!(fake.contents(&p("/r/Docs/sub/x.txt")).unwrap(), b"x");
    assert!(!fake.exists(&p("/r/.Trash/x 3.txt")));
    assert_eq!(r.messages(), vec!["No se sabe de dónde vino".to_string()]);
    assert!(fake.exists(&p("/r/.Trash/perdido.txt")));
    // Un paso de deshacer: vuelve a la Papelera.
    assert_eq!(r.out.result.undo, Some(Undo::Batch(vec![Undo::Created(vec![p("/r/Docs/sub/x.txt")])])));
    assert_eq!(r.out.result.select, vec![p("/r/Docs/sub/x.txt")]);
}

#[test]
fn restore_conflict_asks() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/r2/.Trash/x.txt"), b"de la papelera".to_vec());
    fake.set_trash_origin(&p("/r2/.Trash/x.txt"), &p("/r2/Docs/x.txt"));
    fake.add_file(&p("/r2/Docs/x.txt"), b"otro".to_vec());
    let r = go(&fake, JobSpec::Restore { items: vec![p("/r2/.Trash/x.txt")] }, &[answer(Choice::KeepBoth)]);
    assert_eq!(r.conflicts().len(), 1);
    assert_eq!(fake.contents(&p("/r2/Docs/x.txt")).unwrap(), b"otro");
    assert_eq!(fake.contents(&p("/r2/Docs/x 2.txt")).unwrap(), b"de la papelera");
}

#[test]
fn undo_job_moved_and_batch_reverse() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/u/a/x.txt"), b"x".to_vec());
    fake.add_dir(&p("/u/b"));
    let r = go(&fake, JobSpec::Move { sources: vec![p("/u/a/x.txt")], dest: p("/u/b") }, &[]);
    let u = go(&fake, JobSpec::Undo(r.out.result.undo.unwrap()), &[]);
    assert!(u.out.errors.is_empty());
    assert!(fake.exists(&p("/u/a/x.txt")) && !fake.exists(&p("/u/b/x.txt")));

    // Batch de Recycled + Created: primero va a la Papelera lo creado y después vuelve lo reemplazado.
    fake.add_file(&p("/u/c/r.txt"), b"viejo".to_vec());
    let name = crate::sys::CopyPrims::trash(&fake, &p("/u/c/r.txt")).unwrap();
    fake.add_file(&p("/u/c/r.txt"), b"nuevo".to_vec());
    let batch = Undo::Batch(vec![Undo::Recycled(vec![(name, p("/u/c/r.txt"))]), Undo::Created(vec![p("/u/c/r.txt")])]);
    let u = go(&fake, JobSpec::Undo(batch), &[]);
    assert!(u.out.errors.is_empty(), "{:?}", u.out.errors);
    assert_eq!(fake.contents(&p("/u/c/r.txt")).unwrap(), b"viejo");
    assert_eq!(fake.trashed().len(), 1);

    // Rename y CreatedFolder con algo adentro (se deja).
    fake.add_file(&p("/u/d/nuevo.txt"), b"n".to_vec());
    fake.add_file(&p("/u/e/con algo/f.txt"), b"f".to_vec());
    let batch = Undo::Batch(vec![Undo::Rename { path: p("/u/d/nuevo.txt"), old_name: "viejo.txt".into() }, Undo::CreatedFolder(p("/u/e/con algo"))]);
    let u = go(&fake, JobSpec::Undo(batch), &[]);
    assert!(u.out.errors.is_empty());
    assert!(fake.exists(&p("/u/d/viejo.txt")) && fake.exists(&p("/u/e/con algo/f.txt")));
    assert_eq!(u.out.result.undo, None);
}

#[test]
fn undo_moved_back_never_overwrites() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/uo/b/x.txt"), b"movido".to_vec());
    fake.add_file(&p("/uo/a/x.txt"), b"aparecio".to_vec());
    let r = go(&fake, JobSpec::Undo(Undo::Moved(vec![(p("/uo/b/x.txt"), p("/uo/a/x.txt"))])), &[answer(Choice::Skip)]);
    assert_eq!(r.conflicts().len(), 1);
    assert_eq!(fake.contents(&p("/uo/a/x.txt")).unwrap(), b"aparecio");
    assert_eq!(fake.contents(&p("/uo/b/x.txt")).unwrap(), b"movido");
}

#[test]
fn symlinks_copied_as_links() {
    let fake = FakeCopyPrims::new();
    fake.add_dir(&p("/sl/s/L"));
    fake.add_symlink(&p("/sl/s/L/enlace"), &p("/muy/lejos"));
    fake.add_dir(&p("/sl/d"));
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/sl/s/L")], dest: p("/sl/d") }, &[]);
    assert!(r.out.errors.is_empty());
    assert_eq!(fake.link_target(&p("/sl/d/L/enlace")), Some(p("/muy/lejos")));
}

#[test]
fn cancel_while_waiting_for_answer() {
    let fake = FakeCopyPrims::new();
    conflict_setup(&fake);
    let ctl = Control::new();
    let c = ctl.clone();
    let h = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        c.cancel();
    });
    install_fakes();
    let mut updates = Vec::new();
    run(copy_informe(), &fake, &ctl, &mut |u| updates.push(u));
    h.join().unwrap();
    let Some(Update::Finished(out)) = updates.last() else { panic!() };
    assert!(out.cancelled);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"viejo");
}

#[test]
fn titles() {
    let copy = JobSpec::Copy { sources: vec![p("/x/Fotos/a.jpg"), p("/x/Fotos/b.jpg")], dest: p("/y/Backup") };
    assert_eq!(title(&copy, 1240), "Copiando 1.240 elementos de \"Fotos\" a \"Backup\"");
    let one = JobSpec::Move { sources: vec![p("/x/Docs/Proyecto")], dest: p("/y/Documentos") };
    assert_eq!(title(&one, 1), "Moviendo «Proyecto» a \"Documentos\"");
    let mv = JobSpec::Move { sources: vec![p("/x/a"), p("/x/b")], dest: p("/y/z") };
    assert_eq!(title(&mv, 2), "Moviendo 2 elementos de \"x\" a \"z\"");
    assert_eq!(title(&JobSpec::Duplicate { sources: vec![p("/a"), p("/b")] }, 2), "Duplicando 2 elementos");
    assert_eq!(title(&JobSpec::Restore { items: vec![p("/a")] }, 1), "Devolviendo 1 elemento");
    assert_eq!(title(&JobSpec::Undo(Undo::Created(vec![])), 0), "Deshaciendo");
}

#[test]
fn replace_never_trashes_what_contains_the_source() {
    // Mover «a/a» a la carpeta de arriba y elegir «Reemplazar» sobre «a» (que la contiene).
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/rp/a/a/dato.txt"), b"d".to_vec());
    let r = go(&fake, JobSpec::Move { sources: vec![p("/rp/a/a")], dest: p("/rp") }, &[answer(Choice::Replace)]);
    assert_eq!(r.messages(), vec!["No se puede reemplazar una carpeta que contiene lo que se copia".to_string()]);
    assert_eq!(fake.contents(&p("/rp/a/a/dato.txt")).unwrap(), b"d");
    assert!(fake.trashed().is_empty());
}

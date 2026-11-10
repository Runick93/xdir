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
    // En la Papelera figura con el nombre y el lugar de lo que se reemplazó.
    let trashed = fake.trashed();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].1, p("/k/dst/informe.pdf"));
    assert!(trashed[0].0.starts_with("informe.pdf"));
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
    assert_eq!(fake.trashed().len(), 1);
    // Un paso de deshacer: primero se va lo nuevo y después vuelve la carpeta vieja entera.
    let undo = r.out.result.undo.clone().unwrap();
    assert!(matches!(&undo, Undo::Batch(steps) if matches!(steps[..], [Undo::Recycled(_), Undo::Created(_)])), "{undo:?}");
    go(&fake, JobSpec::Undo(undo), &[]);
    assert_eq!(fake.list_all(&p("/kd/dst")), vec![p("/kd/dst/D"), p("/kd/dst/D/viejo.txt")]);
}

#[test]
fn replace_folder_error_restores_old() {
    // La carpeta vieja se aparta y se descarta solo si la nueva quedó entera; si algo falló, la
    // nueva (copias de este trabajo) se borra y la vieja vuelve a su lugar.
    let fake = FakeCopyPrims::new();
    fake.set_volume(&p("/kn"), VolumeInfo { has_trash: false, ..volume("kn", "Red") });
    fake.add_file(&p("/kn/src/D/a.txt"), b"a".to_vec());
    fake.add_file(&p("/kn/src/D/b.txt"), b"b".to_vec());
    fake.add_file(&p("/kn/dst/D/viejo.txt"), b"v".to_vec());
    fake.fail(&p("/kn/src/D/b.txt"), ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/kn/src/D")], dest: p("/kn/dst") }, &[answer(Choice::Replace)]);
    assert_eq!(fake.list_all(&p("/kn/dst")), vec![p("/kn/dst/D"), p("/kn/dst/D/viejo.txt")]);
    assert_eq!(r.out.errors.len(), 1, "{:?}", r.out.errors);
    assert_eq!(r.out.result.undo, None);
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
    // Por delante hay otra carpeta que se puede combinar: la cuenta no sería exacta.
    assert_eq!(c[0].remaining, None);
}

#[test]
fn conflict_count_is_per_kind_and_exact() {
    // Tres archivos que chocan: el primero dice que quedan otros dos (de su clase).
    let fake = FakeCopyPrims::new();
    for f in ["f1.txt", "f2.txt", "f3.txt"] {
        fake.add_file(&p(&format!("/cc/s/{f}")), b"nuevo".to_vec());
        fake.add_file(&p(&format!("/cc/d/{f}")), b"viejo".to_vec());
    }
    let sources = ["f1.txt", "f2.txt", "f3.txt"].iter().map(|n| p(&format!("/cc/s/{n}"))).collect();
    let r = go(&fake, JobSpec::Copy { sources, dest: p("/cc/d") }, &[answer(Choice::Skip), answer(Choice::Skip), answer(Choice::Skip)]);
    assert_eq!(r.conflicts().iter().map(|c| c.remaining).collect::<Vec<_>>(), vec![Some(2), Some(1), Some(0)]);
}

#[test]
fn conflict_count_unknown_inside_or_before_a_merge() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/cm/s/D1/adentro.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/cm/d/D1/adentro.txt"), b"viejo".to_vec());
    fake.add_file(&p("/cm/s/a.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/cm/d/a.txt"), b"viejo".to_vec());
    fake.add_file(&p("/cm/s/z.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/cm/d/z.txt"), b"viejo".to_vec());
    // a.txt, D1 (se combina, adentro choca adentro.txt) y z.txt.
    let sources = vec![p("/cm/s/a.txt"), p("/cm/s/D1"), p("/cm/s/z.txt")];
    let answers = [answer(Choice::Skip), answer(Choice::Merge), answer(Choice::Skip), answer(Choice::Skip)];
    let r = go(&fake, JobSpec::Copy { sources, dest: p("/cm/d") }, &answers);
    let c = r.conflicts();
    assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["a.txt", "D1", "adentro.txt", "z.txt"]);
    // a.txt: por delante hay una carpeta que se puede combinar (no se sabe cuántos archivos chocan).
    assert_eq!(c[0].remaining, None);
    // D1: no quedan otras carpetas que choquen.
    assert_eq!(c[1].remaining, Some(0));
    // Adentro de la carpeta combinada: sin cuenta.
    assert_eq!(c[2].remaining, None);
    // z.txt: ya no hay nada que combinar por delante.
    assert_eq!(c[3].remaining, Some(0));
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
    assert_eq!(title(&copy, 1240), "Copiando 1.240 elementos de «Fotos» a «Backup»");
    let one = JobSpec::Move { sources: vec![p("/x/Docs/Proyecto")], dest: p("/y/Documentos") };
    assert_eq!(title(&one, 1), "Moviendo «Proyecto» a «Documentos»");
    let mv = JobSpec::Move { sources: vec![p("/x/a"), p("/x/b")], dest: p("/y/z") };
    assert_eq!(title(&mv, 2), "Moviendo 2 elementos de «x» a «z»");
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

// ---------- revisión 1 ----------

#[test]
fn replace_same_file_through_another_path_never_deletes() {
    // `Z:\` y `\\servidor\recurso`, o una carpeta enlazada: el mismo archivo por otro camino, y el
    // disco lo informa como otro.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/al/real/a.txt"), b"unico".to_vec());
    fake.add_alias(&p("/al/alias"), &p("/al/real"));
    fake.set_volume(&p("/al/alias"), volume("otro", "Z:"));
    let replace = [answer(Choice::Replace)];
    let r = go(&fake, JobSpec::Move { sources: vec![p("/al/real/a.txt")], dest: p("/al/alias") }, &replace);
    assert!(r.conflicts().is_empty(), "no es un conflicto consigo mismo");
    assert_eq!(fake.contents(&p("/al/real/a.txt")).unwrap(), b"unico");
    assert!(fake.trashed().is_empty());
    // Copiarlo ahí es duplicarlo.
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/al/real/a.txt")], dest: p("/al/alias") }, &replace);
    assert!(r.conflicts().is_empty());
    assert_eq!(fake.list_all(&p("/al/real")), vec![p("/al/real/a copia.txt"), p("/al/real/a.txt")]);
    // Copiar la carpeta dentro de sí misma por el otro camino: se rechaza.
    fake.add_dir(&p("/al/real/sub"));
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/al/real")], dest: p("/al/alias/sub") }, &[]);
    assert_eq!(r.messages(), vec!["No se puede copiar una carpeta dentro de sí misma".to_string()]);
}

#[test]
fn replace_keeps_new_in_place_when_trash_deletes() {
    // Una Papelera que borra definitivamente y falla: lo nuevo ya está en su lugar y se avisa.
    let fake = FakeCopyPrims::new();
    conflict_setup(&fake);
    fake.add_file(&p("/k/src/D/x"), b"x".to_vec());
    fake.add_file(&p("/k/dst/D/tesis.docx"), b"tesis".to_vec());
    fake.set_trash_deletes(true);
    let spec = JobSpec::Copy { sources: vec![p("/k/src/informe.pdf"), p("/k/src/D")], dest: p("/k/dst") };
    let r = go(&fake, spec, &[answer(Choice::Replace), answer(Choice::Replace)]);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"nuevo");
    assert_eq!(fake.contents(&p("/k/dst/D/x")).unwrap(), b"x");
    assert_eq!(r.out.errors.len(), 2, "{:?}", r.out.errors);
    assert!(r.out.errors.iter().all(|e| e.message.starts_with("Lo que estaba antes no entró en la Papelera") && !e.retry));
    no_tmp_left(&fake, &p("/k/dst"));
}

#[test]
fn replace_trash_failure_keeps_old_visible() {
    // La Papelera falla sin borrar: lo nuevo queda en su lugar y lo viejo, a la vista con otro
    // nombre.
    let fake = FakeCopyPrims::new();
    conflict_setup(&fake);
    install_fakes();
    let ctl = Control::new();
    let prims = TrashFails(fake.clone());
    let mut updates = Vec::new();
    run(copy_informe(), &prims, &ctl, &mut |u| {
        if let Update::Conflict(_) = &u {
            ctl.answer(answer(Choice::Replace));
        }
        updates.push(u);
    });
    let Some(Update::Finished(out)) = updates.last() else { panic!() };
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"nuevo");
    assert_eq!(fake.contents(&p("/k/dst/informe (reemplazado).pdf")).unwrap(), b"viejo");
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].message.contains("«informe (reemplazado).pdf»"), "{:?}", out.errors);
    no_tmp_left(&fake, &p("/k/dst"));
}

/// El disco falso con una Papelera que falla sin tocar nada.
struct TrashFails(FakeCopyPrims);

impl crate::sys::CopyPrims for TrashFails {
    fn stat(&self, p: &Path) -> Result<crate::sys::ItemStat, crate::sys::CopyError> {
        self.0.stat(p)
    }
    fn read_dir(&self, p: &Path) -> Result<Vec<std::ffi::OsString>, crate::sys::CopyError> {
        self.0.read_dir(p)
    }
    fn volume(&self, p: &Path) -> Option<VolumeInfo> {
        self.0.volume(p)
    }
    fn copy_file(&self, s: &Path, d: &Path, pr: &mut dyn FnMut(u64) -> crate::sys::Flow) -> Result<(), crate::sys::CopyError> {
        self.0.copy_file(s, d, pr)
    }
    fn copy_symlink(&self, s: &Path, d: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.copy_symlink(s, d)
    }
    fn create_dir(&self, d: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.create_dir(d)
    }
    fn finish_dir(&self, d: &Path, l: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.finish_dir(d, l)
    }
    fn rename_noreplace(&self, f: &Path, t: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.rename_noreplace(f, t)
    }
    fn replace(&self, f: &Path, t: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.replace(f, t)
    }
    fn remove(&self, p: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.remove(p)
    }
    fn trash(&self, p: &Path) -> Result<String, crate::sys::CopyError> {
        Err(crate::sys::CopyError::new(ErrKind::Denied, p))
    }
    fn untrash(&self, t: &str, o: &Path) -> Result<(), crate::sys::CopyError> {
        self.0.untrash(t, o)
    }
}

#[test]
fn replace_on_same_volume_move_sets_aside_first() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/rm/a/x.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/rm/b/x.txt"), b"viejo".to_vec());
    let r = go(&fake, JobSpec::Move { sources: vec![p("/rm/a/x.txt")], dest: p("/rm/b") }, &[answer(Choice::Replace)]);
    assert_eq!(fake.contents(&p("/rm/b/x.txt")).unwrap(), b"nuevo");
    assert_eq!(fake.reads(&p("/rm/a/x.txt")), 0);
    no_tmp_left(&fake, &p("/rm/b"));
    let undo = r.out.result.undo.unwrap();
    go(&fake, JobSpec::Undo(undo), &[]);
    assert_eq!(fake.contents(&p("/rm/a/x.txt")).unwrap(), b"nuevo");
    assert_eq!(fake.contents(&p("/rm/b/x.txt")).unwrap(), b"viejo");
}

#[test]
fn unknown_volume_tries_rename_then_copies() {
    let fake = FakeCopyPrims::new();
    fake.hide_volume(&p("/uv"));
    fake.add_file(&p("/uv/a/x.bin"), vec![1; 10]);
    fake.add_dir(&p("/uv/b"));
    // Mismo disco de verdad (aunque no se sepa): renombra, no lee.
    go(&fake, JobSpec::Move { sources: vec![p("/uv/a/x.bin")], dest: p("/uv/b") }, &[]);
    assert_eq!(fake.reads(&p("/uv/a/x.bin")), 0);
    assert!(fake.exists(&p("/uv/b/x.bin")));
    // Otro disco de verdad: renombrar no puede y se copia (verificando y borrando el original).
    fake.set_volume(&p("/uv/usb"), volume("usb", "USB"));
    fake.add_dir(&p("/uv/usb"));
    let r = go(&fake, JobSpec::Move { sources: vec![p("/uv/b/x.bin")], dest: p("/uv/usb") }, &[]);
    assert!(r.out.errors.is_empty(), "{:?}", r.out.errors);
    assert_eq!(fake.reads(&p("/uv/b/x.bin")), 1);
    assert!(!fake.exists(&p("/uv/b/x.bin")) && fake.exists(&p("/uv/usb/x.bin")));
}

#[test]
fn move_keeps_source_if_it_changed_during_copy() {
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/doc.txt"), vec![1; 1000]);
    fake.set_block(100);
    let f2 = fake.clone();
    let r = go_with(&fake, JobSpec::Move { sources: vec![p("/v1/a/doc.txt")], dest: p("/v2/dst") }, &[], &Control::new(), &mut |u, _| {
        if matches!(u, Update::Progress { bytes, .. } if *bytes > 0) {
            // Otro programa lo guarda con el mismo tamaño.
            f2.write(&p("/v1/a/doc.txt"), vec![2; 1000], 99);
        }
    });
    assert_eq!(r.messages(), vec!["El original cambió durante la copia".to_string()]);
    assert_eq!(fake.contents(&p("/v1/a/doc.txt")).unwrap(), vec![2; 1000]);
    assert!(!fake.exists(&p("/v2/dst/doc.txt")));
}

#[test]
fn move_syncs_copy_before_deleting_source() {
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/x.txt"), b"x".to_vec());
    fake.add_file(&p("/v1/a/y.txt"), b"y".to_vec());
    go(&fake, JobSpec::Move { sources: vec![p("/v1/a/x.txt")], dest: p("/v2/dst") }, &[]);
    // Se asegura la copia (todavía con su nombre parcial) antes de borrar el original.
    assert_eq!(fake.syncs(), vec![p("/v2/dst/.x.txt.xdir-part"), p("/v2/dst")]);
    assert!(!fake.exists(&p("/v1/a/x.txt")));
    // Si no se puede asegurar la copia, el original queda.
    fake.fail(&p("/v2/dst/.y.txt.xdir-part"), ErrKind::Other("No se pudo guardar".into()), 1);
    let r = go(&fake, JobSpec::Move { sources: vec![p("/v1/a/y.txt")], dest: p("/v2/dst") }, &[]);
    assert_eq!(r.messages(), vec!["No se pudo guardar".to_string()]);
    assert!(fake.exists(&p("/v1/a/y.txt")) && !fake.exists(&p("/v2/dst/y.txt")));
}

#[test]
fn skipped_are_reported_and_stale_answers_ignored() {
    let fake = FakeCopyPrims::new();
    conflict_setup(&fake);
    let ctl = Control::new();
    // Una respuesta vieja (de otro conflicto) no decide este.
    ctl.answer(answer(Choice::Replace));
    let r = go_with(&fake, copy_informe(), &[answer(Choice::Skip)], &ctl, &mut |_, _| {});
    assert_eq!(r.out.skipped, vec![p("/k/src/informe.pdf")]);
    assert_eq!(fake.contents(&p("/k/dst/informe.pdf")).unwrap(), b"viejo");
}

#[test]
fn unreadable_subfolder_does_not_stop_the_rest() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/ur/s/D/ok.txt"), b"ok".to_vec());
    fake.add_file(&p("/ur/s/D/cerrada/x.txt"), b"x".to_vec());
    fake.add_dir(&p("/ur/d"));
    fake.fail(&p("/ur/s/D/cerrada"), ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/ur/s/D")], dest: p("/ur/d") }, &[]);
    assert_eq!(fake.contents(&p("/ur/d/D/ok.txt")).unwrap(), b"ok");
    assert_eq!(r.out.errors.len(), 1);
    assert_eq!((r.out.errors[0].src.clone(), r.out.errors[0].dst.clone()), (p("/ur/s/D/cerrada"), p("/ur/d/D/cerrada")));
    assert_eq!(r.out.errors[0].message, "No tenés permiso");
}

#[test]
fn retry_keeps_the_into_itself_check() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/ri/A/f.txt"), b"f".to_vec());
    let r = go(&fake, JobSpec::Retry { moving: false, pairs: vec![(p("/ri/A"), p("/ri/A/sub/A"))] }, &[]);
    assert_eq!(r.messages(), vec!["No se puede copiar una carpeta dentro de sí misma".to_string()]);
    assert!(!r.out.errors[0].retry);
}

#[test]
fn replace_same_file_from_another_folder_never_deletes() {
    // Dos nombres del mismo archivo en carpetas distintas (un enlace duro): «Reemplazar» uno con
    // el otro lo borraría.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/hl/a/x.txt"), b"unico".to_vec());
    fake.add_dir(&p("/hl/b"));
    fake.add_alias(&p("/hl/b/x.txt"), &p("/hl/a/x.txt"));
    let replace = [answer(Choice::Replace)];
    let r = go(&fake, JobSpec::Move { sources: vec![p("/hl/a/x.txt")], dest: p("/hl/b") }, &replace);
    assert!(r.conflicts().is_empty());
    assert_eq!(fake.contents(&p("/hl/a/x.txt")).unwrap(), b"unico");
    // No se movió: se avisa como omitido.
    assert_eq!(r.out.skipped, vec![p("/hl/a/x.txt")]);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/hl/a/x.txt")], dest: p("/hl/b") }, &replace);
    assert!(r.conflicts().is_empty());
    assert_eq!(fake.contents(&p("/hl/a/x.txt")).unwrap(), b"unico");
    assert_eq!(fake.contents(&p("/hl/b/x copia.txt")).unwrap(), b"unico");
    assert!(fake.trashed().is_empty());
}

#[test]
fn replace_folder_with_unreadable_part_restores_old() {
    // Lo que no se pudo leer al recorrer también cuenta: la carpeta vieja vuelve.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/rw/s/C/a.txt"), b"a".to_vec());
    fake.add_file(&p("/rw/s/C/cerrada/x"), b"x".to_vec());
    fake.add_file(&p("/rw/d/C/v.txt"), b"v".to_vec());
    fake.fail(&p("/rw/s/C/cerrada"), ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/rw/s/C")], dest: p("/rw/d") }, &[answer(Choice::Replace)]);
    assert_eq!(r.messages(), vec!["No tenés permiso".to_string()]);
    assert_eq!(fake.list_all(&p("/rw/d")), vec![p("/rw/d/C"), p("/rw/d/C/v.txt")]);
    assert!(fake.trashed().is_empty());
    assert_eq!(r.out.result.undo, None);
}

#[test]
fn unknown_volume_cross_device_keeps_the_first_answer() {
    // Disco desconocido que resulta ser otro: se pregunta una sola vez y se copia con esa
    // respuesta (archivo con «Conservar ambos», «Reemplazar», y una carpeta combinada).
    let fake = FakeCopyPrims::new();
    fake.hide_volume(&p("/xd"));
    fake.set_volume(&p("/xd/usb"), volume("usb", "USB"));
    fake.add_file(&p("/xd/a/x.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/xd/usb/x.txt"), b"viejo".to_vec());
    let r = go(&fake, JobSpec::Move { sources: vec![p("/xd/a/x.txt")], dest: p("/xd/usb") }, &[answer(Choice::KeepBoth)]);
    assert_eq!(r.conflicts().len(), 1);
    assert_eq!(fake.contents(&p("/xd/usb/x 2.txt")).unwrap(), b"nuevo");
    assert!(!fake.exists(&p("/xd/a/x.txt")));

    fake.add_file(&p("/xd/a/y.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/xd/usb/y.txt"), b"viejo".to_vec());
    let r = go(&fake, JobSpec::Move { sources: vec![p("/xd/a/y.txt")], dest: p("/xd/usb") }, &[answer(Choice::Replace)]);
    assert_eq!(r.conflicts().len(), 1);
    assert!(r.out.errors.is_empty(), "{:?}", r.out.errors);
    assert_eq!(fake.contents(&p("/xd/usb/y.txt")).unwrap(), b"nuevo");
    assert!(!fake.exists(&p("/xd/a/y.txt")));

    // Combinar: los hijos que no se pueden renombrar se copian, con su propia respuesta.
    fake.add_file(&p("/xd/a/D/z.txt"), b"z".to_vec());
    fake.add_file(&p("/xd/a/D/w.txt"), b"w nuevo".to_vec());
    fake.add_file(&p("/xd/usb/D/w.txt"), b"w viejo".to_vec());
    let r = go(&fake, JobSpec::Move { sources: vec![p("/xd/a/D")], dest: p("/xd/usb") }, &[answer(Choice::Merge), answer(Choice::Skip)]);
    assert_eq!(r.conflicts().len(), 2);
    assert!(r.out.errors.is_empty(), "{:?}", r.out.errors);
    assert_eq!(fake.contents(&p("/xd/usb/D/z.txt")).unwrap(), b"z");
    assert_eq!(fake.contents(&p("/xd/usb/D/w.txt")).unwrap(), b"w viejo");
    assert_eq!(fake.contents(&p("/xd/a/D/w.txt")).unwrap(), b"w nuevo");
    assert!(!fake.exists(&p("/xd/a/D/z.txt")));
}

#[test]
fn new_files_are_written_under_a_partial_name() {
    // Mientras se copia, el nombre de verdad no existe: lo que se ve es «.a.bin.xdir-part». Si se
    // corta (cancelar, o cerrar xdir antes de que limpie), nunca queda algo a medias con el nombre
    // del archivo.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/pt/s/a.bin"), vec![7; 64 * 1024 * 4]);
    fake.add_dir(&p("/pt/d"));
    fake.set_block(64 * 1024);
    let mut seen = Vec::new();
    let r = go_with(&fake, JobSpec::Copy { sources: vec![p("/pt/s/a.bin")], dest: p("/pt/d") }, &[], &Control::new(), &mut |u, _| {
        if let Update::Progress { bytes, .. } = u
            && *bytes > 0
            && *bytes < 64 * 1024 * 4
        {
            seen.push((fake.exists(&p("/pt/d/a.bin")), fake.exists(&p("/pt/d/.a.bin.xdir-part"))));
        }
    });
    assert!(r.out.errors.is_empty());
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|s| *s == (false, true)), "{seen:?}");
    assert_eq!(fake.contents(&p("/pt/d/a.bin")).unwrap().len(), 64 * 1024 * 4);
    no_tmp_left(&fake, &p("/pt/d"));
    // Cancelado a mitad: ni el archivo ni el parcial.
    fake.add_file(&p("/pt/s/b.bin"), vec![7; 64 * 1024 * 4]);
    let ctl = Control::new();
    let r = go_with(&fake, JobSpec::Copy { sources: vec![p("/pt/s/b.bin")], dest: p("/pt/d") }, &[], &ctl, &mut |u, c| {
        if let Update::Progress { bytes, .. } = u
            && *bytes > 0
        {
            c.cancel();
        }
    });
    assert!(r.out.cancelled);
    assert!(!fake.exists(&p("/pt/d/b.bin")));
    no_tmp_left(&fake, &p("/pt/d"));
}

#[test]
fn leftover_partial_does_not_block_the_copy() {
    // Un parcial que quedó de otra vez (xdir se cerró a la fuerza) no impide copiar ni se pisa.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/pl/s/a.bin"), b"nuevo".to_vec());
    fake.add_file(&p("/pl/d/.a.bin.xdir-part"), b"viejo parcial".to_vec());
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/pl/s/a.bin")], dest: p("/pl/d") }, &[]);
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert_eq!(fake.contents(&p("/pl/d/a.bin")).unwrap(), b"nuevo");
    assert_eq!(fake.contents(&p("/pl/d/.a.bin.xdir-part")).unwrap(), b"viejo parcial");
}

#[test]
fn restore_refuses_a_symlinked_parent() {
    // Un disco armado: «docs» es un enlace a otra carpeta. Devolver ahí no deja el archivo del
    // otro lado: queda en la Papelera, con el error.
    let fake = FakeCopyPrims::new();
    fake.add_dir(&p("/rl/afuera"));
    fake.add_symlink(&p("/rl/vol/docs"), &p("/rl/afuera"));
    fake.add_file(&p("/rl/vol/.Trashes/evil.plist"), b"x".to_vec());
    fake.set_trash_origin(&p("/rl/vol/.Trashes/evil.plist"), &p("/rl/vol/docs/evil.plist"));
    let r = go(&fake, JobSpec::Restore { items: vec![p("/rl/vol/.Trashes/evil.plist")] }, &[]);
    assert_eq!(r.out.errors.len(), 1, "{:?}", r.messages());
    assert!(fake.exists(&p("/rl/vol/.Trashes/evil.plist")));
    assert!(!fake.exists(&p("/rl/afuera/evil.plist")));
    assert!(fake.restored().is_empty());
}

#[test]
fn restore_tells_the_platform_what_came_back() {
    // Lo devuelto sale del registro de la Papelera (papelera.tsv en la Mac, el .trashinfo en
    // Linux); lo que no se pudo devolver, no.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/rf/.Trash/a.txt"), b"a".to_vec());
    fake.set_trash_origin(&p("/rf/.Trash/a.txt"), &p("/rf/Docs/a.txt"));
    fake.add_file(&p("/rf/.Trash/b.txt"), b"b".to_vec());
    fake.set_trash_origin(&p("/rf/.Trash/b.txt"), &p("/rf/Docs/b.txt"));
    fake.add_file(&p("/rf/Docs/b.txt"), b"otro".to_vec());
    let r = go(&fake, JobSpec::Restore { items: vec![p("/rf/.Trash/a.txt"), p("/rf/.Trash/b.txt")] }, &[answer(Choice::Skip)]);
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert_eq!(fake.restored(), vec![p("/rf/.Trash/a.txt")]);
}

#[test]
fn move_flushes_destination_folder_before_deleting_source() {
    // Entre discos: la copia y el nombre nuevo quedan guardados (también la carpeta) antes de borrar
    // el original; si la carpeta no se puede asegurar, el original queda.
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/x.txt"), b"x".to_vec());
    go(&fake, JobSpec::Move { sources: vec![p("/v1/a/x.txt")], dest: p("/v2/dst") }, &[]);
    assert_eq!(fake.syncs(), vec![p("/v2/dst/.x.txt.xdir-part"), p("/v2/dst")]);
    assert!(!fake.exists(&p("/v1/a/x.txt")));
    fake.add_file(&p("/v1/a/y.txt"), b"y".to_vec());
    fake.fail(&p("/v2/dst"), ErrKind::Other("No se pudo guardar".into()), 1);
    let r = go(&fake, JobSpec::Move { sources: vec![p("/v1/a/y.txt")], dest: p("/v2/dst") }, &[]);
    assert_eq!(r.messages(), vec!["No se pudo guardar".to_string()]);
    assert!(fake.exists(&p("/v1/a/y.txt")), "el original queda");
    assert!(fake.exists(&p("/v2/dst/y.txt")), "la copia queda (deshacer la borra)");
}

#[test]
fn long_partial_name_falls_back_to_a_short_one() {
    // El disco no acepta «.nombre.xdir-part» (pasa del largo máximo): se reintenta con uno corto.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/ln/s/largo.txt"), b"datos".to_vec());
    fake.add_dir(&p("/ln/d"));
    fake.fail(&p("/ln/d/.largo.txt.xdir-part"), ErrKind::NameTooLong, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/ln/s/largo.txt")], dest: p("/ln/d") }, &[]);
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert_eq!(fake.contents(&p("/ln/d/largo.txt")).unwrap(), b"datos");
    no_tmp_left(&fake, &p("/ln/d"));
}

#[test]
fn final_rename_retries_while_the_file_is_busy() {
    // Un antivirus o el indexador abren el archivo recién escrito: el renombre final se reintenta
    // un rato antes de fallar.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/rb/s/a.txt"), b"a".to_vec());
    fake.add_dir(&p("/rb/d"));
    fake.fail(&p("/rb/d/a.txt"), ErrKind::InUse, 2);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/rb/s/a.txt")], dest: p("/rb/d") }, &[]);
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert_eq!(fake.contents(&p("/rb/d/a.txt")).unwrap(), b"a");
    // Siempre ocupado: error y no queda el parcial.
    fake.add_file(&p("/rb/s/b.txt"), b"b".to_vec());
    fake.fail(&p("/rb/d/b.txt"), ErrKind::InUse, 100);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/rb/s/b.txt")], dest: p("/rb/d") }, &[]);
    assert_eq!(r.out.errors.len(), 1);
    no_tmp_left(&fake, &p("/rb/d"));
}

#[test]
fn done_items_count_only_what_arrived() {
    // Una carpeta que no se pudo crear no cuenta sus archivos como copiados; lo omitido tampoco.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/dn/s/A/sub/1.txt"), b"1".to_vec());
    fake.add_file(&p("/dn/s/A/sub/2.txt"), b"2".to_vec());
    fake.add_file(&p("/dn/s/A/ok.txt"), b"ok".to_vec());
    fake.add_file(&p("/dn/s/z.txt"), b"z".to_vec());
    fake.add_file(&p("/dn/d/z.txt"), b"viejo".to_vec());
    fake.add_dir(&p("/dn/d"));
    fake.fail(&p("/dn/d/A/sub"), ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/dn/s/A"), p("/dn/s/z.txt")], dest: p("/dn/d") }, &[answer(Choice::Skip)]);
    // A, sub, 1, 2, ok y z: llegaron A y ok.
    assert_eq!(r.out.done_items, 2);
}

#[test]
fn restore_refusal_is_not_retried_and_retry_rechecks() {
    // Revisión del cableado (C1): el rechazo de «Devolver» no se ofrece para reintentar; y un
    // Retry con un elemento de la Papelera pasa igual por las reglas de «Devolver».
    let fake = FakeCopyPrims::new();
    fake.add_dir(&p("/rr/afuera"));
    fake.add_symlink(&p("/rr/vol/docs"), &p("/rr/afuera"));
    fake.add_file(&p("/rr/vol/.Trashes/x.txt"), b"x".to_vec());
    fake.set_trash_origin(&p("/rr/vol/.Trashes/x.txt"), &p("/rr/vol/docs/x.txt"));
    let r = go(&fake, JobSpec::Restore { items: vec![p("/rr/vol/.Trashes/x.txt")] }, &[]);
    assert_eq!(r.out.errors.len(), 1);
    assert!(!r.out.errors[0].retry);
    let pairs = vec![(p("/rr/vol/.Trashes/x.txt"), p("/rr/vol/docs/x.txt"))];
    let r = go(&fake, JobSpec::Retry { moving: true, pairs }, &[]);
    assert_eq!(r.out.errors.len(), 1, "{:?}", r.messages());
    assert!(!fake.exists(&p("/rr/afuera/x.txt")));
    assert!(fake.exists(&p("/rr/vol/.Trashes/x.txt")));
}

#[test]
fn restore_rechecks_folder_after_the_question() {
    // Revisión del cableado (I1): la carpeta cambia por un enlace mientras se espera la respuesta.
    let fake = FakeCopyPrims::new();
    fake.add_dir(&p("/rq/afuera"));
    fake.add_file(&p("/rq/vol/docs/x.txt"), b"viejo".to_vec());
    fake.add_file(&p("/rq/vol/.Trashes/x.txt"), b"x".to_vec());
    fake.set_trash_origin(&p("/rq/vol/.Trashes/x.txt"), &p("/rq/vol/docs/x.txt"));
    for choice in [Choice::KeepBoth, Choice::Replace] {
        let f2 = fake.clone();
        let r = go_with(&fake, JobSpec::Restore { items: vec![p("/rq/vol/.Trashes/x.txt")] }, &[answer(choice)], &Control::new(), &mut |u, _| {
            if let Update::Conflict(_) = u {
                crate::sys::CopyPrims::rename_noreplace(&f2, &p("/rq/vol/docs"), &p("/rq/vol/docs-real")).unwrap();
                f2.add_symlink(&p("/rq/vol/docs"), &p("/rq/afuera"));
            }
        });
        assert_eq!(r.out.errors.len(), 1, "{choice:?}: {:?}", r.messages());
        assert!(!r.out.errors[0].retry);
        assert!(fake.list_all(&p("/rq/afuera")).is_empty(), "{choice:?}: nada afuera");
        assert!(fake.exists(&p("/rq/vol/.Trashes/x.txt")));
        // Para la vuelta siguiente: la carpeta de verdad otra vez.
        crate::sys::CopyPrims::remove(&fake, &p("/rq/vol/docs")).unwrap();
        crate::sys::CopyPrims::rename_noreplace(&fake, &p("/rq/vol/docs-real"), &p("/rq/vol/docs")).unwrap();
    }
}

#[test]
fn new_folder_with_name_counts_what_arrived() {
    // Revisión del cableado (M2, el M11 de #45): de tres pedidos llegan dos → «… con 2 elementos»,
    // y el deshacer apunta a la carpeta con su nombre final.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/nc/a.txt"), b"a".to_vec());
    fake.add_file(&p("/nc/b.txt"), b"b".to_vec());
    let items = vec![p("/nc/a.txt"), p("/nc/b.txt"), p("/nc/no-esta.txt")];
    let r = go(&fake, JobSpec::NewFolderWith { parent: p("/nc"), items }, &[]);
    let folder = p("/nc/Carpeta nueva con 2 elementos");
    assert!(fake.exists(&folder.join("a.txt")) && fake.exists(&folder.join("b.txt")));
    assert!(!fake.exists(&p("/nc/Carpeta nueva con 3 elementos")));
    assert_eq!(r.out.result.focus, Some(folder.clone()));
    assert_eq!(
        r.out.result.undo,
        Some(Undo::Batch(vec![
            Undo::CreatedFolder(folder.clone()),
            Undo::Moved(vec![(folder.join("a.txt"), p("/nc/a.txt")), (folder.join("b.txt"), p("/nc/b.txt"))]),
        ]))
    );
    go(&fake, JobSpec::Undo(r.out.result.undo.unwrap()), &[]);
    assert!(fake.exists(&p("/nc/a.txt")) && fake.exists(&p("/nc/b.txt")));
    assert!(!fake.exists(&folder));
}

#[test]
fn new_folder_with_keeps_undo_for_copies_whose_source_stayed() {
    // Revisión del cableado (M2): entre discos, si no se pudo borrar el original, lo copiado queda
    // en la carpeta con su deshacer (nunca copias sueltas sin deshacer).
    let fake = FakeCopyPrims::new();
    two_volumes(&fake);
    fake.add_file(&p("/v1/a/x.txt"), b"x".to_vec());
    fake.fail(&p("/v1/a/x.txt"), ErrKind::Denied, 0);
    let r = go_with(&fake, JobSpec::NewFolderWith { parent: p("/v2/dst"), items: vec![p("/v1/a/x.txt")] }, &[], &Control::new(), &mut |u, _| {
        // Recién cuando ya se copió: borrar el original falla.
        if let Update::Progress { bytes, .. } = u
            && *bytes > 0
        {
            fake.fail(&p("/v1/a/x.txt"), ErrKind::Denied, 5);
        }
    });
    let folder = p("/v2/dst/Carpeta nueva con 1 elemento");
    assert_eq!(r.out.errors.len(), 1, "{:?}", r.messages());
    assert!(fake.exists(&folder.join("x.txt")), "la copia queda en la carpeta");
    assert!(fake.exists(&p("/v1/a/x.txt")), "y el original");
    let undo = r.out.result.undo.clone().expect("con deshacer");
    go(&fake, JobSpec::Undo(undo), &[]);
    assert!(!fake.exists(&folder.join("x.txt")), "deshacer saca la copia");
    assert!(!fake.exists(&folder), "y la carpeta");
}

#[test]
fn busy_retries_cover_replace_and_set_aside_but_not_denied() {
    // Revisión del cableado (M3): «ocupado» se reintenta también al poner el temporal de un
    // «Reemplazar» y al apartar lo que estaba; «sin permiso» no (salvo donde el sistema lo usa
    // para «ocupado», como Windows).
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/br/s/a.txt"), b"nuevo".to_vec());
    fake.add_file(&p("/br/d/a.txt"), b"viejo".to_vec());
    fake.fail(&p("/br/d/a.txt"), ErrKind::InUse, 2);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/br/s/a.txt")], dest: p("/br/d") }, &[answer(Choice::Replace)]);
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert_eq!(fake.contents(&p("/br/d/a.txt")).unwrap(), b"nuevo");
    // Sin permiso: un solo intento.
    fake.add_file(&p("/br/s/b.txt"), b"b".to_vec());
    fake.fail(&p("/br/d/b.txt"), ErrKind::Denied, 1);
    let r = go(&fake, JobSpec::Copy { sources: vec![p("/br/s/b.txt")], dest: p("/br/d") }, &[]);
    assert_eq!(r.messages(), vec!["No tenés permiso".to_string()]);
    no_tmp_left(&fake, &p("/br/d"));
}

#[test]
fn restore_merge_rechecks_every_folder_on_the_way() {
    // Ronda 2 (A): devolviendo una carpeta que se combina, la subcarpeta cambia por un enlace
    // mientras se pregunta por un archivo de adentro: nada se escribe del otro lado.
    let fake = FakeCopyPrims::new();
    fake.add_dir(&p("/rm/afuera"));
    fake.add_file(&p("/rm/vol/.Trash/X/sub/a"), b"nuevo-a".to_vec());
    fake.add_file(&p("/rm/vol/.Trash/X/sub/b"), b"nuevo-b".to_vec());
    fake.add_file(&p("/rm/vol/docs/X/sub/a"), b"viejo".to_vec());
    fake.set_trash_origin(&p("/rm/vol/.Trash/X"), &p("/rm/vol/docs/X"));
    let f2 = fake.clone();
    let mut n = 0;
    let answers = [answer(Choice::Merge), answer(Choice::Merge), answer(Choice::KeepBoth)];
    let r = go_with(&fake, JobSpec::Restore { items: vec![p("/rm/vol/.Trash/X")] }, &answers, &Control::new(), &mut |u, _| {
        if let Update::Conflict(_) = u {
            n += 1;
            if n == 3 {
                crate::sys::CopyPrims::rename_noreplace(&f2, &p("/rm/vol/docs/X/sub"), &p("/rm/vol/docs/X/sub-real")).unwrap();
                f2.add_symlink(&p("/rm/vol/docs/X/sub"), &p("/rm/afuera"));
            }
        }
    });
    assert!(fake.list_all(&p("/rm/afuera")).is_empty(), "{:?}", fake.list_all(&p("/rm/afuera")));
    assert!(!r.out.errors.is_empty() && r.out.errors.iter().all(|e| !e.retry), "{:?}", r.messages());
}

#[test]
fn retry_of_a_copy_from_the_trash_stays_a_copy() {
    // Ronda 2 (B): copiar algo de la Papelera a otra carpeta y reintentarlo no lo «devuelve».
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/rc/.Trash/f.txt"), b"x".to_vec());
    fake.set_trash_origin(&p("/rc/.Trash/f.txt"), &p("/rc/docs/f.txt"));
    fake.add_dir(&p("/rc/docs"));
    fake.add_dir(&p("/rc/Escritorio"));
    let pairs = vec![(p("/rc/.Trash/f.txt"), p("/rc/Escritorio/f.txt"))];
    let r = go(&fake, JobSpec::Retry { moving: false, pairs }, &[]);
    assert!(r.out.errors.is_empty(), "{:?}", r.messages());
    assert!(fake.exists(&p("/rc/.Trash/f.txt")) && fake.exists(&p("/rc/Escritorio/f.txt")));
    assert!(!fake.exists(&p("/rc/docs/f.txt")));
    assert!(fake.restored().is_empty());
}

#[test]
fn undo_created_refuses_a_symlinked_parent() {
    // Ronda 2: deshacer «lo creado» manda a la Papelera por ruta; si una carpeta del camino pasó a
    // ser un enlace a otro lado, no se manda a la Papelera lo que está del otro lado.
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/uc/afuera/importante.txt"), b"no".to_vec());
    fake.add_symlink(&p("/uc/docs"), &p("/uc/afuera"));
    fake.add_file(&p("/uc/otra/copia.txt"), b"c".to_vec());
    let undo = Undo::Created(vec![p("/uc/docs/importante.txt"), p("/uc/otra/copia.txt")]);
    let r = go(&fake, JobSpec::Undo(undo), &[]);
    assert_eq!(r.out.errors.len(), 1, "{:?}", r.messages());
    assert!(!r.out.errors[0].retry);
    assert!(fake.exists(&p("/uc/afuera/importante.txt")));
    assert!(!fake.exists(&p("/uc/otra/copia.txt")), "lo demás sí");
}

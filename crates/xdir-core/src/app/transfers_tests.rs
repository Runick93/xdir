//! Pruebas de los trabajos de copia en la `App` (con el disco falso de todo el proceso: cada
//! prueba usa sus propias rutas).

use super::*;
use crate::keys::keys::VK_DOWN as KEY_DOWN;
use crate::sys::testing::{FakeCopyPrims, FakeFileOps, FakeHost};
use crate::sys::{ErrKind, VolumeInfo};
use crate::transfer::{Answer, Choice, JobSpec, JobState};
use std::path::Path;
use std::time::Duration;

fn app_with_host(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    App::new(Init::blank(), host)
}

fn app() -> App {
    app_with_host(FakeHost::new())
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn fake() -> FakeCopyPrims {
    crate::sys::testing::install_fakes();
    FakeCopyPrims::shared()
}

/// Entrega los avisos de los trabajos (lo que haría `WM_APP_TRANSFER`) hasta que se cumpla `done`.
fn wait(a: &mut App, done: impl Fn(&App) -> bool) {
    for _ in 0..500 {
        a.on_transfer_updates();
        if done(a) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("los trabajos no terminaron: {:?}", a.jobs().iter().map(|j| (j.id, j.state)).collect::<Vec<_>>());
}

fn idle(a: &App) -> bool {
    a.running_jobs() == 0
}

fn volume(id: &str) -> VolumeInfo {
    VolumeInfo { id: id.into(), label: id.into(), case_insensitive: false, has_trash: true, max_file: None, free: u64::MAX }
}

#[test]
fn duplicate_job_selects_and_undo_is_one_step() {
    let f = fake();
    f.add_file(&p("/app-dup/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-dup/b.txt"), b"b".to_vec());
    let mut a = app();
    FakeFileOps::take_ops();
    a.start_transfer(JobSpec::Duplicate { sources: vec![p("/app-dup/a.txt"), p("/app-dup/b.txt")] });
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-dup/a copia.txt")) && f.exists(&p("/app-dup/b copia.txt")));
    // No pasó por `FileOps`; un solo paso de deshacer; el trabajo terminado sin errores se va.
    assert!(FakeFileOps::take_ops().is_empty());
    assert_eq!(a.undo.len(), 1);
    assert!(matches!(a.undo[0], Undo::Batch(_)));
    assert!(a.jobs().is_empty());
    assert_eq!(a.notice, None);
    // Ctrl+Z: el motor manda las copias a la Papelera, en un paso.
    a.run_command(Command::Undo);
    assert!(FakeFileOps::take_ops().is_empty(), "el deshacer de un trabajo lo hace el motor");
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-dup/a copia.txt")) && !f.exists(&p("/app-dup/b copia.txt")));
    assert!(f.exists(&p("/app-dup/a.txt")));
    assert!(a.undo.is_empty());
}

#[test]
fn new_folder_with_job_opens_rename() {
    let f = fake();
    f.add_file(&p("/app-nfw/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-nfw/b.txt"), b"b".to_vec());
    let mut a = app();
    a.start_transfer(JobSpec::NewFolderWith { parent: p("/app-nfw"), items: vec![p("/app-nfw/a.txt"), p("/app-nfw/b.txt")] });
    wait(&mut a, idle);
    let folder = p("/app-nfw/Carpeta nueva con 2 elementos");
    assert!(f.exists(&folder.join("a.txt")));
    assert_eq!(a.pending_rename, Some(folder));
    assert!(matches!(&a.undo[..], [Undo::Batch(steps)] if matches!(steps[0], Undo::CreatedFolder(_))));
}

#[test]
fn conflict_waits_only_its_job() {
    let f = fake();
    f.set_volume(&p("/app-cw1"), volume("app-cw1"));
    f.set_volume(&p("/app-cw2"), volume("app-cw2"));
    f.add_file(&p("/app-cw1/s/x.txt"), b"nuevo".to_vec());
    f.add_file(&p("/app-cw1/d/x.txt"), b"viejo".to_vec());
    f.add_file(&p("/app-cw2/s/y.txt"), b"y".to_vec());
    f.add_dir(&p("/app-cw2/d"));
    let mut a = app();
    // Con la ventana de copia se pregunta (sin ella, se omite solo).
    a.set_auto_skip(false);
    let stuck = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-cw1/s/x.txt")], dest: p("/app-cw1/d") });
    let free = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-cw2/s/y.txt")], dest: p("/app-cw2/d") });
    wait(&mut a, |a| {
        a.jobs().iter().any(|j| j.id == stuck && j.state == JobState::Conflict) && !a.jobs().iter().any(|j| j.id == free)
    });
    assert!(f.exists(&p("/app-cw2/d/y.txt")));
    let view = a.jobs().iter().find(|j| j.id == stuck).unwrap();
    assert_eq!(view.conflict.as_ref().unwrap().name, "x.txt");
    assert_eq!(a.notice.as_deref(), Some("Ya existe «x.txt» en «d»"));
    assert_eq!(a.running_jobs(), 1);
    a.answer_job(stuck, Answer { choice: Choice::KeepBoth, all: false });
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-cw1/d/x 2.txt")).unwrap(), b"nuevo");
    assert_eq!(f.contents(&p("/app-cw1/d/x.txt")).unwrap(), b"viejo");
}

#[test]
fn start_transfer_returns_immediately() {
    let f = fake();
    f.set_volume(&p("/app-slow"), volume("app-slow"));
    f.add_file(&p("/app-slow/s/grande.bin"), vec![1; 64 * 1024 * 8]);
    f.add_dir(&p("/app-slow/d"));
    let mut a = app();
    f.set_slow(&p("/app-slow/s/grande.bin"), Some(Duration::from_millis(40)));
    let t0 = std::time::Instant::now();
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-slow/s/grande.bin")], dest: p("/app-slow/d") });
    // La interfaz sigue respondiendo mientras copia.
    a.on_key(KEY_DOWN);
    a.on_transfer_updates();
    assert!(t0.elapsed() < Duration::from_millis(150), "{:?}", t0.elapsed());
    assert_eq!(a.running_jobs(), 1);
    // Pausa y reanuda desde la interfaz.
    a.pause_job(id);
    assert!(a.jobs()[0].paused);
    a.resume_job(id);
    f.set_slow(&p("/app-slow/s/grande.bin"), None);
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-slow/d/grande.bin")).unwrap().len(), 64 * 1024 * 8);
}

#[test]
fn cancel_job_cleans_partial() {
    let f = fake();
    f.set_volume(&p("/app-cancel"), volume("app-cancel"));
    f.add_file(&p("/app-cancel/s/grande.bin"), vec![1; 64 * 1024 * 20]);
    f.add_dir(&p("/app-cancel/d"));
    let mut a = app();
    f.set_slow(&p("/app-cancel/s/grande.bin"), Some(Duration::from_millis(20)));
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-cancel/s/grande.bin")], dest: p("/app-cancel/d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.cancel_job(id);
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-cancel/d/grande.bin")));
    assert!(a.undo.is_empty());
}

#[test]
fn errors_stay_for_retry() {
    let f = fake();
    f.add_file(&p("/app-retry/s/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-retry/s/b.txt"), b"b".to_vec());
    f.add_dir(&p("/app-retry/d"));
    f.fail(&p("/app-retry/s/b.txt"), ErrKind::InUse, 1);
    let mut a = app();
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-retry/s/a.txt"), p("/app-retry/s/b.txt")], dest: p("/app-retry/d") });
    wait(&mut a, idle);
    assert_eq!(a.notice.as_deref(), Some("Copiar terminó con 1 error («b.txt»: Lo está usando otro programa)"));
    let view = &a.jobs()[0];
    assert_eq!((view.id, view.state, view.errors.len()), (id, JobState::Done, 1));
    let again = a.retry_job(id).unwrap();
    assert_ne!(again, id);
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-retry/d/b.txt")));
    assert!(a.jobs().is_empty());
    assert_eq!(a.undo.len(), 2);
}

#[test]
fn conflicts_skip_on_their_own_without_window() {
    // Sin ventana de copia, un conflicto no deja el trabajo esperando para siempre: deshacer un
    // movimiento cuyo lugar original ya está ocupado termina, omite y lo dice.
    let f = fake();
    f.add_file(&p("/app-skip/b/x.txt"), b"movido".to_vec());
    f.add_file(&p("/app-skip/a/x.txt"), b"aparecio".to_vec());
    f.add_file(&p("/app-skip/b/y.txt"), b"y".to_vec());
    let mut a = app();
    a.undo.push(Undo::Batch(vec![Undo::Moved(vec![(p("/app-skip/b/x.txt"), p("/app-skip/a/x.txt")), (p("/app-skip/b/y.txt"), p("/app-skip/a/y.txt"))])]));
    a.run_command(Command::Undo);
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-skip/a/x.txt")).unwrap(), b"aparecio");
    assert_eq!(f.contents(&p("/app-skip/b/x.txt")).unwrap(), b"movido");
    assert!(f.exists(&p("/app-skip/a/y.txt")));
    assert_eq!(a.notice.as_deref(), Some("Se omitió 1 elemento que ya existía"));
}

#[test]
fn eject_waits_for_cancelled_jobs() {
    let f = fake();
    f.set_volume(&p("/app-usb"), volume("app-usb"));
    f.add_file(&p("/app-usb/s/grande.bin"), vec![1; 64 * 1024 * 20]);
    f.add_dir(&p("/app-usb/d"));
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let ejected = std::rc::Rc::new(std::cell::Cell::new(0));
    let then = |e: &std::rc::Rc<std::cell::Cell<i32>>| {
        let e = e.clone();
        Box::new(move |_: &mut App| e.set(e.get() + 1)) as transfers::AfterJobs
    };
    // Sin trabajos sobre el disco: se expulsa enseguida, sin preguntar.
    assert!(a.eject_after_jobs(Path::new("/app-usb"), "USB (E:)", then(&ejected)));
    assert_eq!(ejected.get(), 1);
    f.set_slow(&p("/app-usb/s/grande.bin"), Some(Duration::from_millis(20)));
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-usb/s/grande.bin")], dest: p("/app-usb/d") });
    assert_eq!(a.jobs_on_volume(Path::new("/app-usb")), vec![id]);
    assert!(a.jobs_on_volume(Path::new("/otro")).is_empty());
    assert_eq!(eject_question(1, "USB (E:)"), "Hay 1 operación sobre \"USB (E:)\". ¿Cancelarla y expulsar?");
    assert_eq!(eject_question(2, "USB (E:)"), "Hay 2 operaciones sobre \"USB (E:)\". ¿Cancelarlas y expulsar?");
    // «No»: no se expulsa y el trabajo sigue.
    host.set_answer(Some(false));
    assert!(!a.eject_after_jobs(Path::new("/app-usb"), "USB (E:)", then(&ejected)));
    assert_eq!((a.running_jobs(), ejected.get()), (1, 1));
    // «Sí»: se cancela y se expulsa recién cuando el trabajo terminó de limpiar.
    host.set_answer(Some(true));
    assert!(a.eject_after_jobs(Path::new("/app-usb"), "USB (E:)", then(&ejected)));
    assert_eq!(ejected.get(), 1, "no se suelta el disco antes de que termine el trabajo");
    wait(&mut a, idle);
    assert_eq!(ejected.get(), 2);
    assert!(!f.exists(&p("/app-usb/d/grande.bin")));
}

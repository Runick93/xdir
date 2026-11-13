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
        // Lo que hace la expulsión de verdad al terminar: el disco deja de estar «expulsándose».
        Box::new(move |a: &mut App| {
            e.set(e.get() + 1);
            a.eject_done(Path::new("/app-usb"));
        }) as transfers::AfterJobs
    };
    // Sin trabajos sobre el disco: se expulsa enseguida, sin preguntar.
    assert!(a.eject_after_jobs(Path::new("/app-usb"), "USB (E:)", super::transfers::Verb::Eject, then(&ejected)));
    assert_eq!(ejected.get(), 1);
    f.set_slow(&p("/app-usb/s/grande.bin"), Some(Duration::from_millis(20)));
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-usb/s/grande.bin")], dest: p("/app-usb/d") });
    assert_eq!(a.jobs_on_volume(Path::new("/app-usb")), vec![id]);
    assert!(a.jobs_on_volume(Path::new("/otro")).is_empty());
    assert_eq!(eject_question(1, "USB (E:)"), "Hay 1 operación sobre \"USB (E:)\". ¿Cancelarla y expulsar?");
    assert_eq!(eject_question(2, "USB (E:)"), "Hay 2 operaciones sobre \"USB (E:)\". ¿Cancelarlas y expulsar?");
    // «No»: no se expulsa y el trabajo sigue.
    host.set_answer(Some(false));
    assert!(!a.eject_after_jobs(Path::new("/app-usb"), "USB (E:)", super::transfers::Verb::Eject, then(&ejected)));
    assert_eq!((a.running_jobs(), ejected.get()), (1, 1));
    // «Sí»: se cancela y se expulsa recién cuando el trabajo terminó de limpiar.
    host.set_answer(Some(true));
    assert!(a.eject_after_jobs(Path::new("/app-usb"), "USB (E:)", super::transfers::Verb::Eject, then(&ejected)));
    assert_eq!(ejected.get(), 1, "no se suelta el disco antes de que termine el trabajo");
    wait(&mut a, idle);
    assert_eq!(ejected.get(), 2);
    assert!(!f.exists(&p("/app-usb/d/grande.bin")));
}

#[test]
fn second_eject_does_not_drop_the_first() {
    let f = fake();
    for usb in ["/app-ej1", "/app-ej2"] {
        f.set_volume(&p(usb), volume(&usb[1..]));
        f.add_file(&p(&format!("{usb}/s/grande.bin")), vec![1; 64 * 1024 * 20]);
        f.add_dir(&p(&format!("{usb}/d")));
        f.set_slow(&p(&format!("{usb}/s/grande.bin")), Some(Duration::from_millis(20)));
    }
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    host.set_answer(Some(true));
    let ejected = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    for usb in ["/app-ej1", "/app-ej2"] {
        a.start_transfer(JobSpec::Copy { sources: vec![p(&format!("{usb}/s/grande.bin"))], dest: p(&format!("{usb}/d")) });
    }
    for usb in ["/app-ej1", "/app-ej2"] {
        let e = ejected.clone();
        assert!(a.eject_after_jobs(Path::new(usb), usb, super::transfers::Verb::Eject, Box::new(move |a: &mut App| {
            e.borrow_mut().push(usb);
            a.eject_done(Path::new(usb));
        })));
    }
    wait(&mut a, idle);
    let mut got = ejected.borrow().clone();
    got.sort();
    assert_eq!(got, vec!["/app-ej1", "/app-ej2"]);
}

// ---------- ventana de copia ----------

use crate::host::{AuxMouse, AuxWindow};
use crate::sys::testing::FakeShell;
use crate::transfer::view::{self, ViewHit};

/// App con ventana de copia (los conflictos se preguntan).
fn app_window() -> (App, Rc<FakeHost>) {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    a.set_auto_skip(false);
    (a, host)
}

/// Un archivo grande y lento en `root`/s que se copia a `root`/d (en su propio disco).
fn slow_copy(f: &FakeCopyPrims, root: &str, delay: u64) -> JobSpec {
    f.set_volume(&p(root), volume(&root[1..]));
    let src = p(&format!("{root}/s/grande.bin"));
    f.add_file(&src, vec![1; 64 * 1024 * 20]);
    f.add_dir(&p(&format!("{root}/d")));
    f.set_slow(&src, Some(Duration::from_millis(delay)));
    JobSpec::Copy { sources: vec![src], dest: p(&format!("{root}/d")) }
}

/// Clic en el control `h` de la ventana de copia (lo busca recorriendo el área).
fn click_aux(a: &mut App, h: ViewHit) {
    let jobs = a.jobs().to_vec();
    let expanded = a.transfers_expanded();
    let r = geom::rect(0.0, 0.0, view::WIDTH, view::content_height(&jobs, expanded));
    let mut y = 0.0;
    while y < r.bottom {
        let mut x = 0.0;
        while x < r.right {
            if view::hit(r, &jobs, expanded, x, y) == Some(h) {
                a.aux_mouse(AuxWindow::Transfers, x + 1.0, y + 1.0, AuxMouse::Move);
                a.aux_mouse(AuxWindow::Transfers, x + 1.0, y + 1.0, AuxMouse::Down);
                a.aux_mouse(AuxWindow::Transfers, x + 1.0, y + 1.0, AuxMouse::Up);
                return;
            }
            x += 2.0;
        }
        y += 2.0;
    }
    panic!("no está {h:?} en la ventana");
}

fn aux_texts(a: &mut App) -> Vec<String> {
    let p = crate::sys::testing::RecordingPainter::new(a.theme);
    let h = view::content_height(a.jobs(), a.transfers_expanded());
    a.paint_aux(AuxWindow::Transfers, &p, view::WIDTH, h);
    p.texts().into_iter().map(|(_, s, _)| s).collect()
}

#[test]
fn window_appears_after_one_second() {
    let f = fake();
    let (mut a, host) = app_window();
    // Uno que termina antes del segundo: nunca muestra nada.
    f.add_file(&p("/app-w1/s/a.txt"), b"a".to_vec());
    f.add_dir(&p("/app-w1/d"));
    let quick = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-w1/s/a.txt")], dest: p("/app-w1/d") });
    wait(&mut a, idle);
    a.on_transfer_due(quick);
    assert!(host.aux_shows().is_empty());
    // Uno que sigue al segundo: aparece la ventana con su tamaño.
    let id = a.start_transfer(slow_copy(&f, "/app-w2", 20));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_app_message(crate::transfer::WM_APP_TRANSFER_DUE, 0, id as isize);
    let shows = host.aux_shows();
    assert_eq!(shows.len(), 1, "{shows:?}");
    assert_eq!((shows[0].0, shows[0].1.as_str()), (AuxWindow::Transfers, "Copiando…"));
    assert_eq!(shows[0].2, (view::WIDTH, view::content_height(a.jobs(), a.transfers_expanded())));
    // El que corre es el expandido (con el gráfico).
    assert_eq!(a.transfers_expanded(), Some(id));
    assert!(aux_texts(&mut a).iter().any(|t| t.contains("% completado")));
    a.cancel_job(id);
    wait(&mut a, idle);
    // Sin trabajos se oculta sola.
    assert!(!host.aux_visible());
}

#[test]
fn conflict_shows_window_now() {
    let f = fake();
    f.add_file(&p("/app-wc/s/x.txt"), b"nuevo".to_vec());
    f.add_file(&p("/app-wc/d/x.txt"), b"viejo".to_vec());
    let (mut a, host) = app_window();
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-wc/s/x.txt")], dest: p("/app-wc/d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.state == JobState::Conflict));
    assert_eq!(host.aux_shows().len(), 1);
    let t = aux_texts(&mut a);
    assert!(t.iter().any(|s| s == "Ya existe «x.txt» en «d»"), "{t:?}");
    assert_eq!(host.aux_shows()[0].1, "Copiando: un conflicto");
    a.cancel_job(id);
    wait(&mut a, idle);
}

#[test]
fn title_follows_the_queue() {
    let f = fake();
    f.add_file(&p("/app-wt2/s/x.txt"), b"nuevo".to_vec());
    f.add_file(&p("/app-wt2/d/x.txt"), b"viejo".to_vec());
    let (mut a, host) = app_window();
    let slow = a.start_transfer(slow_copy(&f, "/app-wt3", 20));
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-wt2/s/x.txt")], dest: p("/app-wt2/d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.state == JobState::Conflict));
    assert_eq!(host.aux_shows().last().unwrap().1, "Copiando: un conflicto");
    // Respondido, vuelve a «Copiando…» (el otro sigue).
    a.answer_job(id, Answer { choice: Choice::Skip, all: false });
    wait(&mut a, |a| !a.jobs().iter().any(|j| j.id == id));
    assert_eq!(host.aux_shows().last().unwrap().1, "Copiando…");
    a.cancel_job(slow);
    wait(&mut a, idle);
}

#[test]
fn conflict_answer_reaches_job() {
    let f = fake();
    for n in ["a", "b", "c"] {
        f.add_file(&p(&format!("/app-wa/s/{n}.txt")), b"nuevo".to_vec());
        f.add_file(&p(&format!("/app-wa/d/{n}.txt")), b"viejo".to_vec());
    }
    let (mut a, _host) = app_window();
    let sources = ["a", "b", "c"].iter().map(|n| p(&format!("/app-wa/s/{n}.txt"))).collect();
    let id = a.start_transfer(JobSpec::Copy { sources, dest: p("/app-wa/d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.state == JobState::Conflict));
    std::thread::sleep(Duration::from_millis(450));
    // «Hacer esto con todos» y «Conservar ambos»: los tres quedan con « 2» sin volver a preguntar.
    click_aux(&mut a, ViewHit::ApplyAll(id));
    assert!(a.jobs()[0].apply_all);
    click_aux(&mut a, ViewHit::Choose(id, Choice::KeepBoth));
    wait(&mut a, idle);
    for n in ["a", "b", "c"] {
        assert_eq!(f.contents(&p(&format!("/app-wa/d/{n} 2.txt"))).unwrap(), b"nuevo");
        assert_eq!(f.contents(&p(&format!("/app-wa/d/{n}.txt"))).unwrap(), b"viejo");
    }
}

#[test]
fn errors_view_and_retry_from_window() {
    let f = fake();
    f.add_file(&p("/app-we/s/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-we/s/b.txt"), b"b".to_vec());
    f.add_dir(&p("/app-we/d"));
    f.fail(&p("/app-we/s/b.txt"), ErrKind::InUse, 1);
    let (mut a, host) = app_window();
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-we/s/a.txt"), p("/app-we/s/b.txt")], dest: p("/app-we/d") });
    wait(&mut a, idle);
    // Termina con un error: la ventana aparece enseguida con la lista.
    assert_eq!(host.aux_shows().len(), 1);
    let t = aux_texts(&mut a);
    assert!(t.iter().any(|s| s == "Terminó con 1 error"), "{t:?}");
    assert!(t.iter().any(|s| s == "Lo está usando otro programa"), "{t:?}");
    click_aux(&mut a, ViewHit::Retry(id));
    wait(&mut a, |a| a.jobs().is_empty());
    assert!(f.exists(&p("/app-we/d/b.txt")));
    assert!(!host.aux_visible());
}

#[test]
fn dismiss_errors_hides_window() {
    let f = fake();
    f.add_file(&p("/app-wd/s/b.txt"), b"b".to_vec());
    f.add_dir(&p("/app-wd/d"));
    f.fail(&p("/app-wd/s/b.txt"), ErrKind::InUse, 1);
    let (mut a, host) = app_window();
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-wd/s/b.txt")], dest: p("/app-wd/d") });
    wait(&mut a, idle);
    assert!(host.aux_visible());
    click_aux(&mut a, ViewHit::Dismiss(id));
    assert!(a.jobs().is_empty());
    assert!(!host.aux_visible());
}

#[test]
fn pause_resume_cancel_buttons() {
    let f = fake();
    let (mut a, _host) = app_window();
    let id = a.start_transfer(slow_copy(&f, "/app-wp", 20));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_transfer_due(id);
    click_aux(&mut a, ViewHit::Pause(id));
    assert!(a.jobs()[0].paused);
    assert!(aux_texts(&mut a).iter().any(|t| t == "Reanudar"));
    click_aux(&mut a, ViewHit::Resume(id));
    assert!(!a.jobs()[0].paused);
    click_aux(&mut a, ViewHit::Cancel(id));
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-wp/d/grande.bin")));
}

#[test]
fn closing_window_keeps_jobs_and_indicator() {
    let f = fake();
    let (mut a, host) = app_window();
    let id = a.start_transfer(slow_copy(&f, "/app-wk", 20));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_transfer_due(id);
    host.aux_user_closed();
    a.aux_closed(AuxWindow::Transfers);
    assert_eq!(a.running_jobs(), 1, "cerrar la ventana no cancela");
    // El indicador queda en la barra de estado con el avance.
    a.paint();
    assert!(host.drawn_texts().iter().any(|(_, t, _)| t.ends_with(" %")), "{:?}", host.drawn_texts());
    // Seguir copiando no la vuelve a abrir.
    a.on_transfer_updates();
    assert_eq!(host.aux_shows().len(), 1);
    a.cancel_job(id);
    wait(&mut a, idle);
}

#[test]
fn indicator_click_reopens() {
    let f = fake();
    let (mut a, host) = app_window();
    let id = a.start_transfer(slow_copy(&f, "/app-wi", 20));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    let r = a.transfers_button_rect();
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert_eq!(host.aux_shows().len(), 1);
    // También el comando «Mostrar operaciones».
    a.aux_closed(AuxWindow::Transfers);
    a.run_command(Command::ShowTransfers);
    assert_eq!(host.aux_shows().len(), 2);
    a.cancel_job(id);
    wait(&mut a, idle);
}

#[test]
fn queue_empty_hides_window() {
    let f = fake();
    let (mut a, host) = app_window();
    let id = a.start_transfer(slow_copy(&f, "/app-wq", 5));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_transfer_due(id);
    assert!(host.aux_visible());
    f.set_slow(&p("/app-wq/s/grande.bin"), None);
    wait(&mut a, idle);
    assert!(!host.aux_visible());
    assert!(host.aux_hides() >= 1);
}

#[test]
fn without_host_window_only_indicator_and_auto_skip() {
    let f = fake();
    f.add_file(&p("/app-wn/s/x.txt"), b"nuevo".to_vec());
    f.add_file(&p("/app-wn/d/x.txt"), b"viejo".to_vec());
    let (mut a, host) = app_window();
    host.set_aux_native(false);
    let _ = a.start_transfer(slow_copy(&f, "/app-wn2", 5));
    let id = a.jobs()[0].id;
    a.on_transfer_due(id);
    // Sin ventana del sistema no hay a quién preguntar: se vuelve a «Omitir».
    let _ = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-wn/s/x.txt")], dest: p("/app-wn/d") });
    f.set_slow(&p("/app-wn2/s/grande.bin"), None);
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-wn/d/x.txt")).unwrap(), b"viejo");
    assert!(host.aux_shows().is_empty());
}

#[test]
fn paste_uses_engine() {
    let f = fake();
    f.add_file(&p("/app-paste/s/a.txt"), b"a".to_vec());
    f.add_dir(&p("/app-paste/d"));
    let (mut a, _host) = app_window();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(p("/app-paste/d")));
    FakeFileOps::take_ops();
    FakeShell::set_clipboard(Some((vec![p("/app-paste/s/a.txt")], true)));
    a.run_command(Command::Paste);
    wait(&mut a, idle);
    assert!(FakeFileOps::take_ops().is_empty(), "pegar no pasa por FileOps");
    assert!(f.exists(&p("/app-paste/d/a.txt")));
    assert!(!f.exists(&p("/app-paste/s/a.txt")), "lo cortado se mueve");
    // Copiar (no cortado) deja el original.
    f.add_file(&p("/app-paste/s/b.txt"), b"b".to_vec());
    FakeShell::set_clipboard(Some((vec![p("/app-paste/s/b.txt")], false)));
    a.run_command(Command::Paste);
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-paste/d/b.txt")) && f.exists(&p("/app-paste/s/b.txt")));
}

#[test]
fn drop_uses_engine() {
    let f = fake();
    f.add_file(&p("/app-drop/s/a.txt"), b"a".to_vec());
    f.add_dir(&p("/app-drop/d"));
    let (mut a, _host) = app_window();
    a.paint();
    let point = (300.0, 600.0);
    let Hit::Pane(pane, PaneHit::List) = a.hit(point.0, point.1) else { panic!("{:?}", a.hit(point.0, point.1)) };
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[pane].tab_mut().navigate(hwnd, &tx, Location::Path(p("/app-drop/d")));
    FakeFileOps::take_ops();
    let paths = vec![p("/app-drop/s/a.txt")];
    let mut ev = DragEvent { kind: DragKind::Drop, point, ctrl: true, shift: false, paths: Some(&paths), allowed: Effect::COPY, effect: Effect::NONE };
    a.on_drag(&mut ev);
    assert_eq!(ev.effect, Effect::COPY);
    wait(&mut a, idle);
    assert!(FakeFileOps::take_ops().is_empty(), "soltar no pasa por FileOps");
    assert!(f.exists(&p("/app-drop/d/a.txt")));
}

/// App con el árbol a la vista y `drives` como sus discos (rutas del disco falso).
#[cfg(feature = "arbol")]
fn app_with_tree_drives(drives: &[(&Path, &str)]) -> App {
    crate::sys::testing::install_fakes();
    crate::sys::testing::FakeFileSystem::set_drives(drives);
    let (mut a, _host) = app_window();
    crate::sys::testing::FakeFileSystem::set_drives(&[]);
    a.tree.visible = true;
    a.paint();
    a
}

#[cfg(feature = "arbol")]
fn drag(a: &mut App, kind: DragKind, point: (f32, f32), paths: &[PathBuf], ctrl: bool) -> Effect {
    let mut ev = DragEvent { kind, point, ctrl, shift: false, paths: Some(paths), allowed: Effect::ALL, effect: Effect::NONE };
    a.on_drag(&mut ev);
    ev.effect
}

/// #56: soltar sobre una carpeta del árbol mueve (mismo disco) con el motor de copia, y la fila
/// queda resaltada mientras se arrastra encima.
#[cfg(feature = "arbol")]
#[test]
fn drop_on_tree_folder_moves_with_engine() {
    let f = fake();
    f.add_file(&p("/app-tdrop/s/a.txt"), b"a".to_vec());
    f.add_dir(&p("/app-tdrop/d"));
    let mut a = app_with_tree_drives(&[(Path::new("/app-tdrop/d"), "Destino")]);
    FakeFileOps::take_ops();
    let point = a.tree.row_point("Destino").expect("la fila del disco");
    let paths = vec![p("/app-tdrop/s/a.txt")];
    assert_eq!(drag(&mut a, DragKind::Enter, point, &paths, false), Effect::MOVE);
    assert_eq!(a.tree.drop_target_path(), Some(p("/app-tdrop/d")), "la fila se resalta");
    assert!(a.panes.iter().all(|p| p.tab().list.drop_target.is_none()));
    // Ctrl (⌥ en la Mac) copia.
    assert_eq!(drag(&mut a, DragKind::Over, point, &paths, true), Effect::COPY);
    assert_eq!(drag(&mut a, DragKind::Drop, point, &paths, false), Effect::MOVE);
    assert_eq!(a.tree.drop_target, None, "al soltar se apaga el resaltado");
    wait(&mut a, idle);
    assert!(FakeFileOps::take_ops().is_empty(), "soltar no pasa por FileOps");
    assert!(f.exists(&p("/app-tdrop/d/a.txt")) && !f.exists(&p("/app-tdrop/s/a.txt")));
}

/// #56: lo que no es una carpeta (Este equipo, «Red»), soltar una carpeta en sí misma o un archivo
/// en su propia carpeta y la Papelera no aceptan nada ni se resaltan; salir apaga el resaltado.
#[cfg(feature = "arbol")]
#[test]
fn drop_on_tree_rejects_computer_network_and_self() {
    let f = fake();
    f.add_file(&p("/app-tdrop2/d/x.txt"), b"x".to_vec());
    f.add_dir(&p("/app-tdrop2/t"));
    let mut a = app_with_tree_drives(&[(Path::new("/app-tdrop2/d"), "Destino"), (Path::new("/app-tdrop2/t"), "Papelera")]);
    let row = |a: &App, name: &str| a.tree.row_point(name).unwrap_or_else(|| panic!("fila {name}"));
    let file = vec![p("/app-tdrop2/d/x.txt")];
    for name in ["Este equipo", "Red", "Inicio", "xtree"] {
        let at = row(&a, name);
        assert_eq!(drag(&mut a, DragKind::Over, at, &file, false), Effect::NONE, "{name}");
        assert_eq!(a.tree.drop_target, None, "{name}");
    }
    let at = row(&a, "Destino");
    assert_eq!(drag(&mut a, DragKind::Over, at, &file, false), Effect::NONE, "a su misma carpeta");
    assert_eq!(drag(&mut a, DragKind::Over, at, &[p("/app-tdrop2/d")], false), Effect::NONE, "en sí misma");
    assert_eq!(a.tree.drop_target, None);
    crate::sys::testing::FakeFileSystem::set_trash(Some(Path::new("/app-tdrop2/t")));
    let trash = row(&a, "Papelera");
    assert_eq!(drag(&mut a, DragKind::Over, trash, &file, false), Effect::NONE, "la Papelera");
    crate::sys::testing::FakeFileSystem::set_trash(None);
    assert_eq!(drag(&mut a, DragKind::Over, trash, &file, false), Effect::MOVE);
    assert!(a.tree.drop_target.is_some());
    assert_eq!(drag(&mut a, DragKind::Leave, (0.0, 0.0), &file, false), Effect::NONE);
    assert_eq!(a.tree.drop_target, None, "al salir se apaga");
    assert!(f.exists(&p("/app-tdrop2/d/x.txt")));
}

/// #56: arrastrando, quedarse sobre un disco plegado del árbol lo despliega (`DRAG_EXPAND`) y
/// sus subcarpetas aparecen cuando llega la lectura.
#[cfg(feature = "arbol")]
#[test]
fn drag_hover_expands_tree_folder() {
    let dir = std::env::temp_dir().join(format!("xdir-tdrop3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("subcarpeta")).unwrap();
    let mut a = app_with_tree_drives(&[(dir.as_path(), "Disco3")]);
    let at = a.tree.row_point("Disco3").unwrap();
    let paths = vec![p("/app-tdrop3/x.txt")];
    drag(&mut a, DragKind::Enter, at, &paths, false);
    assert!(a.tree.row_point("subcarpeta").is_none());
    a.tree.age_drag_hover(crate::tree::DRAG_EXPAND);
    drag(&mut a, DragKind::Over, at, &paths, false);
    let t0 = std::time::Instant::now();
    loop {
        a.on_app_message(crate::tree::WM_APP_TREE, 0, 0);
        a.paint();
        if a.tree.row_point("subcarpeta").is_some() {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "no se desplegó");
        std::thread::sleep(Duration::from_millis(5));
    }
    drag(&mut a, DragKind::Leave, (0.0, 0.0), &paths, false);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_move_uses_engine() {
    let f = fake();
    f.add_file(&p("/app-um/b/x.txt"), b"x".to_vec());
    f.add_dir(&p("/app-um/a"));
    let mut a = app();
    FakeFileOps::take_ops();
    a.undo.push(Undo::Moved(vec![(p("/app-um/b/x.txt"), p("/app-um/a/x.txt"))]));
    a.run_command(Command::Undo);
    assert!(FakeFileOps::take_ops().is_empty(), "deshacer un movimiento lo hace el motor");
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-um/a/x.txt")) && !f.exists(&p("/app-um/b/x.txt")));
}

#[test]
fn confirm_close_with_jobs() {
    let f = fake();
    let (mut a, host) = app_window();
    // Sin trabajos: se cierra sin preguntar.
    assert!(a.confirm_close());
    a.start_transfer(slow_copy(&f, "/app-close", 20));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    host.set_answer(None);
    assert!(!a.confirm_close());
    host.set_answer(Some(false));
    assert!(!a.confirm_close());
    assert_eq!(a.running_jobs(), 1);
    // Sí: cancela, espera que limpie y deja cerrar.
    host.set_answer(Some(true));
    assert!(a.confirm_close());
    assert_eq!(a.running_jobs(), 0);
    assert!(!f.exists(&p("/app-close/d/grande.bin")));
    assert_eq!(super::transfers::close_question(2), "Hay 2 operaciones en curso. ¿Cancelarlas y salir?");
    assert_eq!(super::transfers::close_question(1), "Hay 1 operación en curso. ¿Cancelarla y salir?");
}

#[test]
fn queued_title_counts_the_chosen_items() {
    let f = fake();
    let (mut a, _host) = app_window();
    let first = a.start_transfer(slow_copy(&f, "/app-wt", 20));
    f.add_file(&p("/app-wt/s/b.txt"), b"b".to_vec());
    let second = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-wt/s/b.txt")], dest: p("/app-wt/d") });
    let title = &a.jobs().iter().find(|j| j.id == second).unwrap().title;
    assert_eq!(title, "Copiando «b.txt» a «d»");
    a.cancel_job(first);
    a.cancel_job(second);
    wait(&mut a, idle);
}

#[test]
fn running_job_skips_conflicts_once_there_is_no_window() {
    // Un trabajo que empezó creyendo que había ventana: cuando el host no la puede abrir, sus
    // conflictos posteriores se omiten solos (no espera para siempre).
    let f = fake();
    let JobSpec::Copy { mut sources, dest } = slow_copy(&f, "/app-nw", 10) else { unreachable!() };
    f.add_file(&p("/app-nw/s/z.txt"), b"nuevo".to_vec());
    f.add_file(&p("/app-nw/d/z.txt"), b"viejo".to_vec());
    sources.push(p("/app-nw/s/z.txt"));
    let (mut a, host) = app_window();
    host.set_aux_native(false);
    let id = a.start_transfer(JobSpec::Copy { sources, dest });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_transfer_due(id);
    f.set_slow(&p("/app-nw/s/grande.bin"), None);
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-nw/d/z.txt")).unwrap(), b"viejo");
    assert_eq!(a.notice.as_deref(), Some("Se omitió 1 elemento que ya existía"));
}

#[test]
fn conflict_buttons_ignore_clicks_right_after_appearing() {
    let f = fake();
    f.add_file(&p("/app-wg/s/x.txt"), b"nuevo".to_vec());
    f.add_file(&p("/app-wg/d/x.txt"), b"viejo".to_vec());
    let (mut a, _host) = app_window();
    let id = a.start_transfer(JobSpec::Copy { sources: vec![p("/app-wg/s/x.txt")], dest: p("/app-wg/d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.state == JobState::Conflict));
    // Un clic que venía para otra cosa (el bloque recién apareció debajo del mouse): no cuenta.
    click_aux(&mut a, ViewHit::ApplyAll(id));
    click_aux(&mut a, ViewHit::Choose(id, Choice::Replace));
    assert!(!a.jobs()[0].apply_all);
    assert_eq!(a.jobs()[0].state, JobState::Conflict);
    std::thread::sleep(Duration::from_millis(450));
    click_aux(&mut a, ViewHit::Choose(id, Choice::KeepBoth));
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-wg/d/x 2.txt")).unwrap(), b"nuevo");
    assert_eq!(f.contents(&p("/app-wg/d/x.txt")).unwrap(), b"viejo");
}

#[test]
fn retry_of_a_restore_is_a_restore() {
    // Revisión del cableado (C1): «Reintentar» de un «Devolver» vuelve a ser «Devolver» (con sus
    // reglas y el aviso a la Papelera), nunca un mover común.
    let f = fake();
    f.add_file(&p("/app-rt/.Trash/x.txt"), b"x".to_vec());
    f.set_trash_origin(&p("/app-rt/.Trash/x.txt"), &p("/app-rt/Docs/x.txt"));
    f.add_dir(&p("/app-rt/Docs"));
    f.fail(&p("/app-rt/Docs/x.txt"), ErrKind::NotFound, 1);
    let (mut a, _host) = app_window();
    let id = a.start_transfer(JobSpec::Restore { items: vec![p("/app-rt/.Trash/x.txt")] });
    wait(&mut a, idle);
    assert_eq!(a.jobs()[0].errors.len(), 1, "{:?}", a.jobs()[0].errors);
    a.take_started();
    a.retry_job(id).expect("se reintenta");
    assert_eq!(a.take_started(), vec![JobSpec::Restore { items: vec![p("/app-rt/.Trash/x.txt")] }]);
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-rt/Docs/x.txt")));
    assert_eq!(f.restored(), vec![p("/app-rt/.Trash/x.txt")]);
}

#[test]
fn no_new_copies_onto_a_disk_being_ejected() {
    // Revisión del cableado (M1): mientras se espera para expulsar un disco, no empiezan copias
    // nuevas que lo usen.
    let f = fake();
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let spec = slow_copy(&f, "/app-ejn", 20);
    a.start_transfer(spec);
    host.set_answer(Some(true));
    assert!(a.eject_after_jobs(Path::new("/app-ejn"), "USB (F:)", super::transfers::Verb::Eject, Box::new(|a: &mut App| a.eject_done(Path::new("/app-ejn")))));
    f.add_file(&p("/app-ejn/s/otro.txt"), b"o".to_vec());
    let before = a.jobs().len();
    a.start_transfer(JobSpec::Copy { sources: vec![p("/app-ejn/s/otro.txt")], dest: p("/app-ejn/d") });
    assert_eq!(a.jobs().len(), before, "no se lanzó");
    assert_eq!(a.notice.as_deref(), Some("Se está expulsando \"USB (F:)\""));
    wait(&mut a, idle);
    // Ya expulsado (o no): se puede de nuevo.
    a.start_transfer(JobSpec::Copy { sources: vec![p("/app-ejn/s/otro.txt")], dest: p("/app-ejn/d") });
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-ejn/d/otro.txt")));
}

#[test]
fn eject_finds_jobs_on_the_volume_by_identity() {
    // Ronda 2: el mismo disco por otro camino (un enlace, otras mayúsculas) también cuenta, para
    // preguntar antes de expulsar y para no empezar copias nuevas mientras se espera.
    let f = fake();
    let usb = VolumeInfo { id: "app-ejid".into(), label: "USB (G:)".into(), case_insensitive: true, has_trash: true, max_file: None, free: u64::MAX };
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let spec = slow_copy(&f, "/app-ejid-enlace", 20);
    f.set_volume(&p("/app-ejid"), usb.clone());
    f.set_volume(&p("/app-ejid-enlace"), usb);
    // Lo que deja la tarea que lista los discos expulsables (en su hilo, sin tocar el disco acá).
    a.remember_volume_ids(vec![(p("/app-ejid"), "app-ejid".into())]);
    let id = a.start_transfer(spec);
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    assert_eq!(a.jobs_on_volume(Path::new("/app-ejid")), vec![id]);
    host.set_answer(Some(true));
    assert!(a.eject_after_jobs(Path::new("/app-ejid"), "USB (G:)", super::transfers::Verb::Eject, Box::new(|_: &mut App| {})));
    // Una copia nueva por el otro camino, mientras se espera: no empieza.
    f.add_file(&p("/app-ejid-enlace/s/otro.txt"), b"o".to_vec());
    a.start_transfer(JobSpec::Copy { sources: vec![p("/app-ejid-enlace/s/otro.txt")], dest: p("/app-ejid-enlace/d") });
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-ejid-enlace/d/otro.txt")));
    assert!(!f.exists(&p("/app-ejid-enlace/d/grande.bin")));
}

#[test]
fn retry_of_a_restored_folder_child_restores_its_folder() {
    // Ronda 2: algo de adentro de una carpeta devuelta que falló: «Reintentar» devuelve esa
    // carpeta de la Papelera (lo de adentro no tiene lugar original propio).
    let f = fake();
    f.add_file(&p("/app-rc/.Trash/X/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-rc/.Trash/X/b.txt"), b"b".to_vec());
    f.set_trash_origin(&p("/app-rc/.Trash/X"), &p("/app-rc/Docs/X"));
    f.add_dir(&p("/app-rc/Docs/X"));
    f.fail(&p("/app-rc/Docs/X/b.txt"), ErrKind::InUse, 1);
    let (mut a, _host) = app_window();
    let id = a.start_transfer(JobSpec::Restore { items: vec![p("/app-rc/.Trash/X")] });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.state == JobState::Conflict) || idle(a));
    if a.jobs().iter().any(|j| j.state == JobState::Conflict) {
        std::thread::sleep(Duration::from_millis(450));
        a.answer_job(id, Answer { choice: Choice::Merge, all: false });
    }
    wait(&mut a, idle);
    let errs = a.jobs().iter().find(|j| j.id == id).map(|j| j.errors.clone()).unwrap_or_default();
    assert!(errs.iter().any(|e| e.src == p("/app-rc/.Trash/X/b.txt")), "{errs:?}");
    a.take_started();
    a.retry_job(id);
    assert_eq!(a.take_started(), vec![JobSpec::Restore { items: vec![p("/app-rc/.Trash/X")] }]);
    // La carpeta ya está (con a.txt): se combina y llega b.txt.
    wait(&mut a, |a| a.jobs().iter().any(|j| j.state == JobState::Conflict));
    std::thread::sleep(Duration::from_millis(450));
    let again = a.jobs()[0].id;
    a.answer_job(again, Answer { choice: Choice::Merge, all: false });
    wait(&mut a, idle);
    assert_eq!(f.contents(&p("/app-rc/Docs/X/b.txt")).unwrap(), b"b");
}

#[test]
fn eject_without_jobs_still_blocks_new_copies_until_done() {
    // Ronda 3: sin trabajos se expulsa enseguida, pero hasta que termina (en otro hilo) no empiezan
    // copias nuevas sobre el disco.
    let f = fake();
    f.add_file(&p("/app-ejw/s/a.txt"), b"a".to_vec());
    f.add_dir(&p("/app-ejw/d"));
    let mut a = app();
    assert!(a.eject_after_jobs(Path::new("/app-ejw"), "USB (H:)", super::transfers::Verb::Eject, Box::new(|_: &mut App| {})));
    a.start_transfer(JobSpec::Copy { sources: vec![p("/app-ejw/s/a.txt")], dest: p("/app-ejw/d") });
    assert!(a.jobs().is_empty());
    assert_eq!(a.notice.as_deref(), Some("Se está expulsando \"USB (H:)\""));
    // Terminó de expulsar (o falló): se puede otra vez.
    a.eject_done(Path::new("/app-ejw"));
    a.start_transfer(JobSpec::Copy { sources: vec![p("/app-ejw/s/a.txt")], dest: p("/app-ejw/d") });
    wait(&mut a, idle);
    assert!(f.exists(&p("/app-ejw/d/a.txt")));
}

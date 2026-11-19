//! Pruebas de los trabajos de copia en la `App` (con el disco falso de todo el proceso: cada
//! prueba usa sus propias rutas).

use super::*;
use crate::keys::keys::VK_DOWN as KEY_DOWN;
use crate::sys::testing::{FakeCopyPrims, FakeFileOps, FakeHost};
use crate::sys::{ErrKind, VolumeInfo};
use crate::transfer::{Answer, Choice, JobId, JobSpec, JobState};
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
    assert_eq!(a.notice.as_deref(), Some("Copiar terminó con 1 error («b.txt»: lo está usando otro programa)"));
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

/// Ajustes 2: mientras espera a que limpien las operaciones canceladas, la barra lo dice; después,
/// «Expulsando…» hasta que termina.
#[test]
fn eject_waiting_for_jobs_text() {
    let f = fake();
    f.set_volume(&p("/app-ejtx"), volume("app-ejtx"));
    f.add_file(&p("/app-ejtx/s/grande.bin"), vec![1; 64 * 1024 * 20]);
    f.add_dir(&p("/app-ejtx/d"));
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    f.set_slow(&p("/app-ejtx/s/grande.bin"), Some(Duration::from_millis(20)));
    a.start_transfer(JobSpec::Copy { sources: vec![p("/app-ejtx/s/grande.bin")], dest: p("/app-ejtx/d") });
    assert_eq!(a.eject_status(), None);
    host.set_answer(Some(true));
    assert!(a.eject_after_jobs(Path::new("/app-ejtx"), "USB (E:)", super::transfers::Verb::Eject, Box::new(|_: &mut App| {})));
    assert_eq!(a.eject_status().as_deref(), Some("Esperando a que termine 1 operación para expulsar \"USB (E:)\"…"));
    // En el indicador de actividad, sin los puntos suspensivos.
    let now = Instant::now() + Duration::from_secs(1);
    a.sync_activity(now);
    assert_eq!(a.activity.tip(now).as_deref(), Some("Esperando a que termine 1 operación para expulsar \"USB (E:)\""));
    wait(&mut a, idle);
    assert_eq!(a.eject_status().as_deref(), Some("Expulsando \"USB (E:)\"…"));
    a.eject_done(Path::new("/app-ejtx"));
    assert_eq!(a.eject_status(), None);
    assert_eq!(super::transfers::ejecting_text(super::transfers::Verb::Eject, "USB", 2), "Esperando a que terminen 2 operaciones para expulsar \"USB\"…");
    assert_eq!(super::transfers::ejecting_text(super::transfers::Verb::Disconnect, "fotos", 0), "Desconectando \"fotos\"…");
    assert_eq!(super::transfers::ejecting_text(super::transfers::Verb::Disconnect, "fotos", 1), "Esperando a que termine 1 operación para desconectar \"fotos\"…");
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

use crate::host::{AuxMouse, AuxProgress, AuxWindow};
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

/// Dos copias lentas, con raíces distintas pero sobre el mismo disco falso. Con `first_runs` la
/// segunda se lanza recién cuando la primera ya corre (si no, los dos hilos compiten por anotarse).
fn two_same_disk_copies(a: &mut App, f: &FakeCopyPrims, tag: &str, first_runs: bool) -> (JobId, JobId) {
    let (r1, r2) = (format!("/{tag}-1"), format!("/{tag}-2"));
    let s1 = slow_copy(f, &r1, 20);
    let s2 = slow_copy(f, &r2, 20);
    f.set_volume(&p(&r1), volume(tag));
    f.set_volume(&p(&r2), volume(tag));
    let i1 = a.start_transfer(s1);
    if first_runs {
        wait(a, |a| a.jobs().iter().any(|j| j.id == i1 && j.state == JobState::Running));
    }
    (i1, a.start_transfer(s2))
}

/// De fábrica las copias sobre el mismo disco corren a la vez: ninguna pasa por «en cola».
#[test]
fn same_disk_jobs_run_together_by_default() {
    let f = fake();
    let (mut a, _host) = app_window();
    let (i1, i2) = two_same_disk_copies(&mut a, &f, "app-par", false);
    let waited = std::cell::Cell::new(false);
    wait(&mut a, |a| {
        if a.jobs().iter().any(|j| j.state == JobState::Waiting) {
            waited.set(true);
        }
        [i1, i2].iter().all(|id| a.jobs().iter().any(|j| j.id == *id && j.state == JobState::Running))
    });
    assert!(!waited.get());
    a.cancel_job(i1);
    a.cancel_job(i2);
    wait(&mut a, idle);
}

/// Con `cola_por_disco` el segundo trabajo sobre el mismo disco espera su turno.
#[test]
fn queue_by_disk_makes_second_wait() {
    let f = fake();
    let (mut a, _host) = app_window();
    a.config.queue_by_disk = true;
    let (i1, i2) = two_same_disk_copies(&mut a, &f, "app-cola", true);
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == i2 && j.state == JobState::Waiting));
    a.cancel_job(i1);
    a.cancel_job(i2);
    wait(&mut a, idle);
}

/// «Empezar ahora» saca de la cola a un trabajo que espera su turno, con el primero todavía corriendo.
#[test]
fn start_now_runs_queued_job() {
    let f = fake();
    let (mut a, _host) = app_window();
    a.config.queue_by_disk = true;
    let (i1, i2) = two_same_disk_copies(&mut a, &f, "app-ahora", true);
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == i2 && j.state == JobState::Waiting));
    click_aux(&mut a, ViewHit::StartNow(i2));
    wait(&mut a, |a| {
        let st = |id| a.jobs().iter().find(|j| j.id == id).map(|j| j.state);
        st(i2) == Some(JobState::Running) && st(i1) == Some(JobState::Running)
    });
    a.cancel_job(i1);
    a.cancel_job(i2);
    wait(&mut a, idle);
}

/// Apagar «Copias en cola por disco» con un trabajo esperando lo libera, con el primero corriendo.
#[test]
fn turning_queue_off_releases_waiting_job() {
    let f = fake();
    let (mut a, _host) = app_window();
    a.config.queue_by_disk = true;
    let (i1, i2) = two_same_disk_copies(&mut a, &f, "app-apaga", true);
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == i2 && j.state == JobState::Waiting));
    a.toggle_queue_by_disk();
    assert!(!a.config.queue_by_disk);
    wait(&mut a, |a| {
        let st = |id| a.jobs().iter().find(|j| j.id == id).map(|j| j.state);
        st(i2) == Some(JobState::Running) && st(i1) == Some(JobState::Running)
    });
    a.cancel_job(i1);
    a.cancel_job(i2);
    wait(&mut a, idle);
}

/// Dónde está el control `h` en la ventana de copia, tal como se ve (con el desplazamiento).
fn find_aux(a: &App, h: ViewHit) -> Option<(f32, f32)> {
    let jobs = a.jobs().to_vec();
    let expanded = |id| a.transfers_expanded(id);
    let (w, vh) = a.transfers_size();
    let r = geom::rect(0.0, 0.0, w, vh);
    let mut y = 0.0;
    while y < r.bottom {
        let mut x = 0.0;
        while x < r.right {
            if view::hit(r, &jobs, &expanded, a.transfers_scroll(), x, y) == Some(h) {
                return Some((x + 1.0, y + 1.0));
            }
            x += 2.0;
        }
        y += 2.0;
    }
    None
}

/// Clic en (x, y) de la ventana de copia.
fn click_aux_at(a: &mut App, x: f32, y: f32) {
    for ev in [AuxMouse::Move, AuxMouse::Down, AuxMouse::Up] {
        a.aux_mouse(AuxWindow::Transfers, x, y, ev);
    }
}

/// Clic en el control `h` de la ventana de copia (tiene que estar a la vista).
fn click_aux(a: &mut App, h: ViewHit) {
    let Some((x, y)) = find_aux(a, h) else { panic!("no está {h:?} en la ventana") };
    click_aux_at(a, x, y);
}

fn aux_texts(a: &mut App) -> Vec<String> {
    let p = crate::sys::testing::RecordingPainter::new(a.theme);
    let (w, h) = a.transfers_size();
    a.paint_aux(AuxWindow::Transfers, &p, w, h);
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
    assert_eq!(shows[0].2, (view::WIDTH, view::content_height(a.jobs(), &|id| a.transfers_expanded(id))));
    // El que corre se ve expandido (con el gráfico).
    assert!(a.transfers_expanded(id));
    assert!(aux_texts(&mut a).iter().any(|t| t.contains("% completado")));
    a.cancel_job(id);
    wait(&mut a, idle);
    // Sin trabajos se oculta sola.
    assert!(!host.aux_visible());
}

/// Ajustes 2: el clic en el indicador de actividad abre la ventana de copia solo si hay copias.
#[test]
fn activity_click_opens_copy_window_with_copies() {
    let f = fake();
    let (mut a, host) = app_window();
    let t0 = Instant::now();
    a.activity.begin(super::activity::ActivityKey::Connect(1), "Conectando con \"nas\"", t0);
    crate::clock::set(Some(t0 + Duration::from_millis(400)));
    a.paint();
    let r = a.activity_rect.expect("el indicador");
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert!(host.aux_shows().is_empty(), "sin copias, nada");
    let id = a.start_transfer(slow_copy(&f, "/app-act", 20));
    a.paint();
    // Con el indicador de copias a la vista, el de actividad se corre a su izquierda.
    let r = a.activity_rect.expect("el indicador");
    assert!(r.right <= a.transfers_button_rect().left);
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert_eq!(host.aux_shows().len(), 1);
    a.cancel_job(id);
    wait(&mut a, idle);
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
    // Con errores, el avance va en rojo.
    assert!(host.aux_progresses().contains(&AuxProgress::Error(1.0)), "{:?}", host.aux_progresses());
    click_aux(&mut a, ViewHit::Dismiss(id));
    assert!(a.jobs().is_empty());
    assert!(!host.aux_visible());
    assert_eq!(host.aux_progresses().last(), Some(&AuxProgress::None));
}

#[test]
fn pause_resume_cancel_buttons() {
    let f = fake();
    let (mut a, host) = app_window();
    let id = a.start_transfer(slow_copy(&f, "/app-wp", 20));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_transfer_due(id);
    click_aux(&mut a, ViewHit::Pause(id));
    assert!(a.jobs()[0].paused);
    // En pausa, el avance va en amarillo.
    assert!(matches!(host.aux_progresses().last(), Some(AuxProgress::Paused(_))), "{:?}", host.aux_progresses());
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
fn copy_window_reports_taskbar_progress() {
    let f = fake();
    let (mut a, host) = app_window();
    let id = a.start_transfer(slow_copy(&f, "/app-wtb", 5));
    wait(&mut a, |a| a.jobs().iter().any(|j| j.bytes.0 > 0));
    a.on_transfer_due(id);
    let reported = host.aux_progresses();
    assert!(matches!(reported.last(), Some(AuxProgress::Normal(x)) if (0.0..=1.0).contains(x)), "{reported:?}");
    // En milésimas: no se manda un avance por cada bloque copiado.
    assert!(matches!(reported.last(), Some(AuxProgress::Normal(x)) if ((x * 1000.0).round() - x * 1000.0).abs() < 1e-3), "{reported:?}");
    f.set_slow(&p("/app-wtb/s/grande.bin"), None);
    wait(&mut a, idle);
    assert_eq!(host.aux_progresses().last(), Some(&AuxProgress::None));
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

/// Soltar en la Papelera del árbol la manda a la Papelera con el motor (`Delete` con `recycle`),
/// nunca un `Move` a la carpeta de la Papelera: así «Devolver» sabe de dónde salió. El efecto es
/// mover, aun con Ctrl; lo que ya está en la Papelera no se acepta.
#[cfg(feature = "arbol")]
#[test]
fn drop_on_tree_trash_deletes() {
    let f = fake();
    f.add_file(&p("/app-ttrash/s/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-ttrash/s/b.txt"), b"b".to_vec());
    f.add_file(&p("/app-ttrash/.Trash/viejo.txt"), b"v".to_vec());
    crate::sys::testing::FakeFileSystem::set_trash(Some(Path::new("/app-ttrash/.Trash")));
    let mut a = app_with_tree_drives(&[]);
    let point = a.tree.row_point("Papelera").expect("la fila de la Papelera");
    let paths = vec![p("/app-ttrash/s/a.txt"), p("/app-ttrash/s/b.txt")];
    assert_eq!(drag(&mut a, DragKind::Enter, point, &paths, false), Effect::MOVE);
    assert!(a.tree.drop_target.is_some(), "la fila se resalta");
    assert_eq!(drag(&mut a, DragKind::Over, point, &paths, true), Effect::MOVE, "Ctrl no copia");
    // Lo que ya está en la Papelera no se acepta.
    let inside = vec![p("/app-ttrash/.Trash/viejo.txt")];
    assert_eq!(drag(&mut a, DragKind::Over, point, &inside, false), Effect::NONE);
    assert_eq!(a.tree.drop_target, None);
    a.take_started();
    assert_eq!(drag(&mut a, DragKind::Drop, point, &inside, false), Effect::NONE);
    assert!(a.take_started().is_empty());
    assert_eq!(drag(&mut a, DragKind::Drop, point, &paths, false), Effect::MOVE);
    assert_eq!(a.tree.drop_target, None, "al soltar se apaga el resaltado");
    assert_eq!(a.take_started(), vec![JobSpec::Delete { paths: paths.clone(), recycle: true }]);
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-ttrash/s/a.txt")) && !f.exists(&p("/app-ttrash/s/b.txt")));
    crate::sys::testing::FakeFileSystem::set_trash(None);
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
fn delete_retry_deletes_again() {
    // «Reintentar» de un eliminar vuelve a eliminar lo que falló (nunca copiarlo sobre sí mismo).
    let f = fake();
    f.add_file(&p("/app-dr/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-dr/b.txt"), b"b".to_vec());
    f.fail(&p("/app-dr/a.txt"), ErrKind::Denied, 1);
    let (mut a, _host) = app_window();
    let id = a.start_transfer(JobSpec::Delete { paths: vec![p("/app-dr/a.txt"), p("/app-dr/b.txt")], recycle: false });
    wait(&mut a, idle);
    assert_eq!(a.jobs()[0].errors.len(), 1, "{:?}", a.jobs()[0].errors);
    assert!(f.exists(&p("/app-dr/a.txt")) && !f.exists(&p("/app-dr/b.txt")));
    a.take_started();
    a.retry_job(id).expect("se reintenta");
    assert_eq!(a.take_started(), vec![JobSpec::Delete { paths: vec![p("/app-dr/a.txt")], recycle: false }]);
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-dr/a.txt")));
    assert!(f.list_all(&p("/app-dr")).is_empty(), "no se copió nada: {:?}", f.list_all(&p("/app-dr")));
    assert!(a.jobs().is_empty());
}

#[test]
fn delete_retry_of_a_folder_child_deletes_its_folder() {
    // Lo que falló adentro de una carpeta elegida: se vuelve a eliminar esa carpeta (si no, quedaría
    // vacía), a la Papelera si era a la Papelera.
    let f = fake();
    f.add_file(&p("/app-drf/Dir/a.txt"), b"a".to_vec());
    f.add_file(&p("/app-drf/Dir/b.txt"), b"b".to_vec());
    f.fail(&p("/app-drf/Dir/b.txt"), ErrKind::Denied, 1);
    let (mut a, _host) = app_window();
    let id = a.start_transfer(JobSpec::Delete { paths: vec![p("/app-drf/Dir")], recycle: false });
    wait(&mut a, idle);
    assert_eq!(a.jobs()[0].errors.len(), 1, "{:?}", a.jobs()[0].errors);
    a.take_started();
    a.retry_job(id).expect("se reintenta");
    assert_eq!(a.take_started(), vec![JobSpec::Delete { paths: vec![p("/app-drf/Dir")], recycle: false }]);
    wait(&mut a, idle);
    assert!(!f.exists(&p("/app-drf/Dir")));
}

#[test]
fn delete_does_not_wait_behind_a_copy() {
    // Eliminar no hace cola detrás de una copia larga en el mismo disco.
    let f = fake();
    let copy = slow_copy(&f, "/app-dq", 100);
    f.add_file(&p("/app-dq/x.txt"), b"x".to_vec());
    let (mut a, _host) = app_window();
    let slow = a.start_transfer(copy);
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == slow && j.state == JobState::Running));
    let del = a.start_transfer(JobSpec::Delete { paths: vec![p("/app-dq/x.txt")], recycle: false });
    let waited = std::cell::Cell::new(false);
    wait(&mut a, |a| {
        if a.jobs().iter().any(|j| j.id == del && j.state == JobState::Waiting) {
            waited.set(true);
        }
        !a.jobs().iter().any(|j| j.id == del)
    });
    assert!(!waited.get(), "eliminar esperó en la cola");
    assert!(!f.exists(&p("/app-dq/x.txt")));
    assert!(a.jobs().iter().any(|j| j.id == slow && j.state != JobState::Done), "la copia sigue");
    a.cancel_job(slow);
    wait(&mut a, idle);
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

// ---------- varias copias expandidas y desplazamiento ----------

/// `n` copias lentas (unos 2 s cada una), cada una en su disco, todas corriendo.
fn running_copies(a: &mut App, f: &FakeCopyPrims, tag: &str, n: usize) -> Vec<JobId> {
    let ids: Vec<JobId> = (1..=n).map(|k| a.start_transfer(slow_copy(f, &format!("/{tag}-{k}"), 100))).collect();
    wait(a, |a| ids.iter().all(|id| a.jobs().iter().any(|j| j.id == *id && j.state == JobState::Running)));
    ids
}

fn cancel_all(a: &mut App, ids: &[JobId]) {
    for id in ids {
        a.cancel_job(*id);
    }
    wait(a, idle);
}

#[test]
fn expanding_one_keeps_the_other() {
    let f = fake();
    let (mut a, _host) = app_window();
    let ids = running_copies(&mut a, &f, "app-exp", 2);
    let (i1, i2) = (ids[0], ids[1]);
    a.show_transfers();
    // De fábrica, las dos con sus detalles.
    assert!(a.transfers_expanded(i1) && a.transfers_expanded(i2));
    // «Menos detalles» del 1 pliega solo el 1.
    click_aux(&mut a, ViewHit::Expand(i1));
    assert!(!a.transfers_expanded(i1));
    assert!(a.transfers_expanded(i2));
    // Expandirlo de nuevo deja las dos expandidas.
    click_aux(&mut a, ViewHit::Expand(i1));
    assert!(a.transfers_expanded(i1) && a.transfers_expanded(i2));
    // Al terminar, el plegado no queda anotado.
    click_aux(&mut a, ViewHit::Expand(i2));
    assert!(!a.transfers_expanded(i2));
    assert_eq!(a.transfers_collapsed_len(), 1);
    cancel_all(&mut a, &ids);
    assert_eq!(a.transfers_collapsed_len(), 0);
}

#[test]
fn copy_window_caps_height_and_scrolls() {
    let f = fake();
    let (mut a, host) = app_window();
    let ids = running_copies(&mut a, &f, "app-scr", 5);
    a.show_transfers();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    assert!(content > view::MAX_H, "{content}");
    // El host recibe como mucho el alto máximo.
    let shown = host.aux_shows().last().unwrap().2;
    assert_eq!(shown, (view::WIDTH, view::MAX_H));
    assert!(host.aux_sizes().iter().all(|(_, (_, h))| *h <= view::MAX_H), "{:?}", host.aux_sizes());
    assert_eq!(a.transfers_scroll(), 0.0);
    // Lo de abajo no se ve hasta desplazar.
    let last = *ids.last().unwrap();
    assert!(find_aux(&a, ViewHit::Cancel(last)).is_none());
    // La rueda desplaza: el «Cancelar» del 3 sube 200 en la ventana y el clic le llega a ese.
    let third = ids[2];
    let (x, y) = find_aux(&a, ViewHit::Cancel(third)).expect("se ve el 3");
    a.aux_mouse(AuxWindow::Transfers, x, y, AuxMouse::Wheel { dy: 200.0 });
    assert_eq!(a.transfers_scroll(), 200.0);
    click_aux_at(&mut a, x, y - 200.0);
    wait(&mut a, |a| !a.jobs().iter().any(|j| j.id == third));
    // Más rueda que contenido: queda al fondo y se ve el último, que se cancela con su botón.
    for _ in 0..10 {
        a.aux_mouse(AuxWindow::Transfers, x, y, AuxMouse::Wheel { dy: 200.0 });
    }
    let (_, visible) = a.transfers_size();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    assert_eq!(a.transfers_scroll(), content - visible);
    click_aux(&mut a, ViewHit::Cancel(last));
    wait(&mut a, |a| !a.jobs().iter().any(|j| j.id == last));
    // Los demás siguen.
    for id in [ids[0], ids[1], ids[3]] {
        assert!(a.jobs().iter().any(|j| j.id == id && j.state == JobState::Running), "{id}");
    }
    // Hacia arriba vuelve al principio y no pasa de ahí.
    a.aux_mouse(AuxWindow::Transfers, x, y, AuxMouse::Wheel { dy: -5000.0 });
    assert_eq!(a.transfers_scroll(), 0.0);
    cancel_all(&mut a, &ids);
}

#[test]
fn scroll_is_clamped_when_content_shrinks() {
    let f = fake();
    let (mut a, _host) = app_window();
    let ids = running_copies(&mut a, &f, "app-enc", 5);
    a.show_transfers();
    a.aux_mouse(AuxWindow::Transfers, 10.0, 10.0, AuxMouse::Wheel { dy: 10_000.0 });
    assert!(a.transfers_scroll() > 0.0);
    // Terminan tres: el contenido ya entra y no queda nada desplazado.
    for id in &ids[..3] {
        a.cancel_job(*id);
    }
    wait(&mut a, |a| a.jobs().len() == 2);
    let (_, visible) = a.transfers_size();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    assert!(content <= visible, "{content} {visible}");
    assert_eq!(a.transfers_scroll(), 0.0);
    // Plegar uno también achica el contenido y vuelve a limitar.
    let more = running_copies(&mut a, &f, "app-enc2", 2);
    a.aux_mouse(AuxWindow::Transfers, 10.0, 10.0, AuxMouse::Wheel { dy: 10_000.0 });
    let (_, visible) = a.transfers_size();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    assert_eq!(a.transfers_scroll(), content - visible);
    click_aux(&mut a, ViewHit::Expand(more[1]));
    let (_, visible) = a.transfers_size();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    assert!(a.transfers_scroll() <= (content - visible).max(0.0), "{} {content} {visible}", a.transfers_scroll());
    cancel_all(&mut a, &ids);
    cancel_all(&mut a, &more);
}

#[test]
fn conflict_scrolls_into_view() {
    let f = fake();
    let (mut a, _host) = app_window();
    let ids = running_copies(&mut a, &f, "app-cv", 4);
    let root = Path::new("/app-cv-x");
    f.add_file(&root.join("s").join("x.txt"), b"nuevo".to_vec());
    f.add_file(&root.join("d").join("x.txt"), b"viejo".to_vec());
    let id = a.start_transfer(JobSpec::Copy { sources: vec![root.join("s").join("x.txt")], dest: root.join("d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == id && j.state == JobState::Conflict));
    // El conflicto está al fondo y la ventana se desplazó hasta él: se ven sus botones.
    let expanded = |k| a.transfers_expanded(k);
    let (top, bottom) = view::block_span(a.jobs(), &expanded, id).expect("el bloque del conflicto");
    let (_, visible) = a.transfers_size();
    let scroll = a.transfers_scroll();
    assert!(top >= scroll && bottom <= scroll + visible, "{top}..{bottom} con {scroll}+{visible}");
    assert!(find_aux(&a, ViewHit::Choose(id, Choice::Skip)).is_some());
    a.answer_job(id, Answer { choice: Choice::Skip, all: false });
    cancel_all(&mut a, &ids);
}

/// En una pantalla baja la ventana no pasa del alto que le entra al host, y desplazando al fondo
/// se llega a los controles del último trabajo.
#[test]
fn copy_window_fits_short_screens() {
    let f = fake();
    let (mut a, host) = app_window();
    host.set_aux_max_height(Some(400.0));
    let ids = running_copies(&mut a, &f, "app-baja", 2);
    a.show_transfers();
    assert_eq!(host.aux_shows().last().unwrap().2, (view::WIDTH, 400.0));
    assert_eq!(a.transfers_size(), (view::WIDTH, 400.0));
    let last = *ids.last().unwrap();
    assert!(find_aux(&a, ViewHit::Expand(last)).is_none());
    a.aux_mouse(AuxWindow::Transfers, 10.0, 10.0, AuxMouse::Wheel { dy: 10_000.0 });
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    assert_eq!(a.transfers_scroll(), content - 400.0);
    // «Menos detalles» del último, abajo de todo, se puede tocar adentro de los 400.
    let (_, y) = find_aux(&a, ViewHit::Expand(last)).expect("se ve el final del último");
    assert!(y < 400.0);
    assert!(host.aux_sizes().iter().all(|(_, (_, h))| *h <= 400.0), "{:?}", host.aux_sizes());
    cancel_all(&mut a, &ids);
}

/// El pulgar de la barra: se arrastra, salta de a una página (con 40 de solapamiento) y el
/// arrastre termina si el host pierde el mouse (`Cancel`) o llega otro `Down` sin `Up`.
#[test]
fn thumb_drag_pages_and_ends() {
    let f = fake();
    let (mut a, _host) = app_window();
    let ids = running_copies(&mut a, &f, "app-pulgar", 5);
    a.show_transfers();
    let (w, h) = a.transfers_size();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    let r = geom::rect(0.0, 0.0, w, h);
    let (_, thumb) = view::scrollbar(r, content, 0.0).expect("barra");
    let x = (thumb.left + thumb.right) / 2.0;
    // Clic en el riel debajo del pulgar: una página menos 40.
    a.aux_mouse(AuxWindow::Transfers, x, h - 2.0, AuxMouse::Down);
    a.aux_mouse(AuxWindow::Transfers, x, h - 2.0, AuxMouse::Up);
    assert_eq!(a.transfers_scroll(), h - 40.0);
    a.aux_key(AuxWindow::Transfers, crate::keys::keys::VK_HOME);
    assert_eq!(a.transfers_scroll(), 0.0);
    // Arrastrar el pulgar mueve el contenido.
    let y = (thumb.top + thumb.bottom) / 2.0;
    a.aux_mouse(AuxWindow::Transfers, x, y, AuxMouse::Down);
    a.aux_mouse(AuxWindow::Transfers, x, y + 100.0, AuxMouse::Move);
    let dragged = a.transfers_scroll();
    assert!(dragged > 0.0);
    // El host perdió el mouse: moverlo ya no arrastra.
    a.aux_mouse(AuxWindow::Transfers, -1.0, -1.0, AuxMouse::Cancel);
    a.aux_mouse(AuxWindow::Transfers, x, y + 200.0, AuxMouse::Move);
    assert_eq!(a.transfers_scroll(), dragged);
    // Un `Down` con el pulgar tomado (se perdió el `Up`) suelta el pulgar.
    let (_, thumb) = view::scrollbar(r, content, dragged).unwrap();
    let y = (thumb.top + thumb.bottom) / 2.0;
    a.aux_mouse(AuxWindow::Transfers, x, y, AuxMouse::Down);
    a.aux_mouse(AuxWindow::Transfers, 100.0, 5.0, AuxMouse::Down);
    a.aux_mouse(AuxWindow::Transfers, x, y + 150.0, AuxMouse::Move);
    assert_eq!(a.transfers_scroll(), dragged);
    a.aux_mouse(AuxWindow::Transfers, 100.0, 5.0, AuxMouse::Up);
    cancel_all(&mut a, &ids);
}

/// Al desplazarse solo hasta un conflicto, el clic que se soltó enseguida no le llega a lo que
/// quedó debajo (era para lo que estaba antes), y se borra lo que estaba bajo el mouse.
#[test]
fn click_right_after_reveal_is_ignored() {
    let f = fake();
    let (mut a, _host) = app_window();
    let ids = running_copies(&mut a, &f, "app-guard", 4);
    a.show_transfers();
    let (x, y) = find_aux(&a, ViewHit::Cancel(ids[0])).unwrap();
    a.aux_mouse(AuxWindow::Transfers, x, y, AuxMouse::Move);
    assert_eq!(a.transfers_hover(), Some(ViewHit::Cancel(ids[0])));
    let root = Path::new("/app-guard-x");
    f.add_file(&root.join("s").join("x.txt"), b"nuevo".to_vec());
    f.add_file(&root.join("d").join("x.txt"), b"viejo".to_vec());
    let id = a.start_transfer(JobSpec::Copy { sources: vec![root.join("s").join("x.txt")], dest: root.join("d") });
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == id && j.state == JobState::Conflict));
    assert!(a.transfers_scroll() > 0.0);
    assert_eq!(a.transfers_hover(), None);
    // El «Cancelar» del 4 quedó a la vista; un clic enseguida no hace nada.
    let target = ids[3];
    click_aux(&mut a, ViewHit::Cancel(target));
    for _ in 0..30 {
        a.on_transfer_updates();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(a.jobs().iter().any(|j| j.id == target && j.state == JobState::Running));
    // Pasado el resguardo, sí.
    crate::clock::set(Some(Instant::now() + Duration::from_secs(1)));
    click_aux(&mut a, ViewHit::Cancel(target));
    crate::clock::set(None);
    wait(&mut a, |a| !a.jobs().iter().any(|j| j.id == target));
    a.answer_job(id, Answer { choice: Choice::Skip, all: false });
    cancel_all(&mut a, &ids);
}

#[test]
fn keys_scroll_the_copy_window() {
    use crate::keys::keys::{VK_DOWN, VK_END, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP};
    let f = fake();
    let (mut a, _host) = app_window();
    let ids = running_copies(&mut a, &f, "app-teclas", 5);
    a.show_transfers();
    let (_, h) = a.transfers_size();
    let content = view::content_height(a.jobs(), &|id| a.transfers_expanded(id));
    let max = content - h;
    let mut key = |k| {
        assert!(a.aux_key(AuxWindow::Transfers, k));
        a.transfers_scroll()
    };
    assert_eq!(key(VK_NEXT), h - 40.0);
    assert_eq!(key(VK_DOWN), h);
    assert_eq!(key(VK_UP), h - 40.0);
    assert_eq!(key(VK_END), max);
    assert_eq!(key(VK_DOWN), max);
    assert_eq!(key(VK_PRIOR), max - (h - 40.0));
    assert_eq!(key(VK_HOME), 0.0);
    assert_eq!(key(VK_UP), 0.0);
    cancel_all(&mut a, &ids);
}

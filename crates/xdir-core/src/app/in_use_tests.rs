//! Pruebas de la ventana de «archivo en uso» en la `App`: cuándo se abre, qué muestra, y sus
//! botones y teclas (con el disco falso de todo el proceso: cada prueba usa sus propias rutas).

use super::*;
use crate::host::{AuxMouse, AuxWindow};
use crate::in_use::InUseDialog;
use crate::in_use_draw::{self, InUseHit};
use crate::keys::keys::VK_RETURN;
use crate::sys::testing::{AuxShow, FakeCopyPrims, FakeFileOps, FakeHost};
use crate::sys::{ErrKind, Holder};
use crate::transfer::{JobSpec, JobState};
use std::time::Duration;

/// App con ventanas secundarias (los conflictos se preguntan).
fn app_window() -> (App, Rc<FakeHost>) {
    crate::sys::testing::install_fakes();
    let host = FakeHost::new();
    let mut a = App::new(Init::blank(), host.clone());
    a.set_auto_skip(false);
    (a, host)
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn fake() -> FakeCopyPrims {
    crate::sys::testing::install_fakes();
    FakeCopyPrims::shared()
}

fn excel() -> Holder {
    Holder { name: "Microsoft Excel".into(), exe: Some("EXCEL.EXE".into()), pid: 4812, user: Some(r"PC\Leandro".into()) }
}

fn word() -> Holder {
    Holder { name: "Microsoft Word".into(), exe: Some("WINWORD.EXE".into()), pid: 77, user: None }
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

/// Entrega los resultados de las operaciones sueltas (lo que haría `WM_APP_OP_DONE`).
fn drain_ops(a: &mut App) {
    while let Ok(r) = a.ops_rx.try_recv() {
        a.on_op_done(r);
    }
}

/// Las aperturas de la ventana de «archivo en uso».
fn in_use_shows(host: &FakeHost) -> Vec<AuxShow> {
    host.aux_shows().into_iter().filter(|s| s.0 == AuxWindow::InUse).collect()
}

/// Renombra `path` como `new_name` con la operación suelta, que falla con `holders`.
fn failed_rename(a: &mut App, path: &str, new_name: &str, holders: Vec<Holder>) -> Op {
    FakeFileOps::fail_in_use(holders);
    a.commit_rename(p(path), new_name.into());
    drain_ops(a);
    FakeFileOps::respond_with(OpResult::default());
    let ops = FakeFileOps::take_ops();
    assert_eq!(ops.len(), 1, "{ops:?}");
    ops[0].clone()
}

/// Dónde está el botón `h` de la ventana abierta.
fn find_button(a: &App, h: InUseHit) -> (f32, f32) {
    let d = a.in_use_dialog().expect("la ventana está abierta");
    let r = geom::rect(0.0, 0.0, in_use_draw::WIDTH, in_use_draw::height(d));
    let mut y = 0.0;
    while y < r.bottom {
        let mut x = 0.0;
        while x < r.right {
            if in_use_draw::hit(r, d, x, y) == Some(h) {
                return (x + 1.0, y + 1.0);
            }
            x += 2.0;
        }
        y += 2.0;
    }
    panic!("no está {h:?} en la ventana")
}

/// Clic en el botón `h` de la ventana de «archivo en uso».
fn click(a: &mut App, h: InUseHit) {
    let (x, y) = find_button(a, h);
    for ev in [AuxMouse::Move, AuxMouse::Down, AuxMouse::Up] {
        a.aux_mouse(AuxWindow::InUse, x, y, ev);
    }
}

/// Los textos que dibuja la ventana `w`.
fn texts_of(a: &mut App, w: AuxWindow) -> Vec<String> {
    let p = crate::sys::testing::RecordingPainter::new(a.theme);
    let (width, height) = match w {
        AuxWindow::Transfers => a.transfers_size(),
        AuxWindow::InUse => (in_use_draw::WIDTH, in_use_draw::height(a.in_use_dialog().expect("abierta"))),
    };
    a.paint_aux(w, &p, width, height);
    p.texts().into_iter().map(|(_, s, _)| s).collect()
}

/// Borra `names` en `root` con el motor; todos fallan porque los tiene Excel.
fn locked_delete(a: &mut App, f: &FakeCopyPrims, root: &str, names: &[&str]) -> Vec<PathBuf> {
    let paths: Vec<PathBuf> = names.iter().map(|n| p(&format!("{root}/{n}"))).collect();
    for path in &paths {
        f.add_file(path, b"x".to_vec());
        f.fail(path, ErrKind::InUse, 1);
        f.set_holders(path, vec![excel()]);
    }
    a.start_transfer(JobSpec::Delete { paths: paths.clone(), recycle: true });
    paths
}

#[test]
fn failed_rename_in_use_opens_the_window() {
    let (mut a, host) = app_window();
    failed_rename(&mut a, "/iu-ren/a.txt", "b.txt", vec![excel()]);
    let shows = in_use_shows(&host);
    assert_eq!(shows.len(), 1, "{shows:?}");
    assert_eq!(shows[0].1, "No se pudo renombrar «a.txt» como «b.txt»");
    // El área mide lo que el dibujo necesita (los botones van al pie).
    let d = a.in_use_dialog().expect("la ventana está abierta").clone();
    assert_eq!(shows[0].2, (in_use_draw::WIDTH, in_use_draw::height(&d)));
    assert!(host.aux_visible_of(AuxWindow::InUse));
    // El aviso de la barra de estado sigue como siempre.
    let notice = a.notice.clone().unwrap_or_default();
    assert!(notice.contains("a.txt"), "{notice}");
    // Y la ventana dice quién lo tiene.
    let t = texts_of(&mut a, AuxWindow::InUse);
    assert!(t.iter().any(|s| s == "Microsoft Excel (EXCEL.EXE)"), "{t:?}");
    assert!(t.iter().any(|s| s == r"PID 4812 · usuario PC\Leandro"), "{t:?}");
}

#[test]
fn in_use_without_holders_opens_nothing() {
    let (mut a, host) = app_window();
    FakeFileOps::fail_with(ErrKind::InUse);
    a.commit_rename(p("/iu-sin/a.txt"), "b.txt".into());
    drain_ops(&mut a);
    FakeFileOps::respond_with(OpResult::default());
    FakeFileOps::take_ops();
    assert!(in_use_shows(&host).is_empty());
    assert!(a.in_use_dialog().is_none());
    assert!(a.notice.as_deref().is_some_and(|n| n.contains("a.txt")), "{:?}", a.notice);
}

/// Sin ventanas secundarias (Linux, o un host que todavía no tiene esta): queda solo el aviso.
#[test]
fn host_without_aux_windows_keeps_only_the_notice() {
    let (mut a, host) = app_window();
    host.set_aux_native(false);
    failed_rename(&mut a, "/iu-nat/a.txt", "b.txt", vec![excel()]);
    assert!(in_use_shows(&host).is_empty());
    assert!(a.in_use_dialog().is_none(), "no queda «abierta» una ventana que no se mostró");
    assert!(a.notice.as_deref().is_some_and(|n| n.contains("a.txt")), "{:?}", a.notice);
    // Con ventanas otra vez, el próximo fallo sí la abre (con `aux_show`, no `aux_resize`).
    host.set_aux_native(true);
    failed_rename(&mut a, "/iu-nat/c.txt", "d.txt", vec![excel()]);
    assert_eq!(in_use_shows(&host).len(), 1);
}

#[test]
fn delete_job_with_locked_files_opens_one_window_at_the_end() {
    let f = fake();
    let (mut a, host) = app_window();
    locked_delete(&mut a, &f, "/iu-del", &["a.txt", "b.txt", "c.txt"]);
    // Mientras el trabajo sigue, ninguna ventana de «en uso».
    let early = std::cell::Cell::new(false);
    let h = host.clone();
    wait(&mut a, |a| {
        if !idle(a) && !in_use_shows(&h).is_empty() {
            early.set(true);
        }
        idle(a)
    });
    assert!(!early.get(), "se abrió antes de terminar");
    assert!(a.jobs().iter().all(|j| j.state == JobState::Done));
    let shows = in_use_shows(&host);
    assert_eq!(shows.len(), 1, "una sola ventana por trabajo: {shows:?}");
    assert_eq!(shows[0].1, "3 elementos están en uso");
    let d = a.in_use_dialog().expect("abierta");
    assert_eq!(d.groups.len(), 1);
    assert_eq!(d.groups[0].files, vec!["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn cancelled_job_opens_nothing() {
    let f = fake();
    let (mut a, host) = app_window();
    let locked = p("/iu-can/s/a.txt");
    let big = p("/iu-can/s/grande.bin");
    f.add_file(&locked, b"a".to_vec());
    f.fail(&locked, ErrKind::InUse, 1);
    f.set_holders(&locked, vec![excel()]);
    f.add_file(&big, vec![1; 64 * 1024 * 20]);
    f.set_slow(&big, Some(Duration::from_millis(20)));
    f.add_dir(&p("/iu-can/d"));
    let id = a.start_transfer(JobSpec::Copy { sources: vec![locked, big], dest: p("/iu-can/d") });
    // El bloqueado ya falló: el grande está copiándose.
    wait(&mut a, |a| a.jobs().iter().any(|j| j.id == id && j.bytes.0 > 1));
    a.cancel_job(id);
    wait(&mut a, idle);
    // Terminó cancelado con el error del bloqueado (con quién lo tiene), y no se abrió nada.
    assert!(a.jobs().iter().any(|j| j.id == id && j.errors.iter().any(|e| !e.holders.is_empty())), "el error tiene que llegar");
    assert!(in_use_shows(&host).is_empty());
    assert!(a.in_use_dialog().is_none());
}

#[test]
fn second_failure_replaces_the_window() {
    let (mut a, host) = app_window();
    failed_rename(&mut a, "/iu-dos/a.txt", "b.txt", vec![excel()]);
    let resized_before = host.aux_sizes().iter().filter(|s| s.0 == AuxWindow::InUse).count();
    failed_rename(&mut a, "/iu-dos/c.txt", "d.txt", vec![excel(), word()]);
    let shows = in_use_shows(&host);
    assert_eq!(shows.len(), 1, "no abre otra: {shows:?}");
    // El título y el tamaño son los del segundo.
    assert_eq!(shows[0].1, "No se pudo renombrar «c.txt» como «d.txt»");
    let d = a.in_use_dialog().expect("abierta").clone();
    assert_eq!(d.title, "No se pudo renombrar «c.txt» como «d.txt»");
    assert_eq!(d.groups.len(), 2);
    let sizes: Vec<(f32, f32)> = host.aux_sizes().into_iter().filter(|s| s.0 == AuxWindow::InUse).map(|s| s.1).collect();
    assert_eq!(sizes.len(), resized_before + 1, "{sizes:?}");
    assert_eq!(sizes.last(), Some(&(in_use_draw::WIDTH, in_use_draw::height(&d))));
    let t = texts_of(&mut a, AuxWindow::InUse);
    assert!(t.iter().any(|s| s == "Microsoft Word (WINWORD.EXE)"), "{t:?}");
}

#[test]
fn retry_button_repeats_the_op_and_closes() {
    let (mut a, host) = app_window();
    let op = failed_rename(&mut a, "/iu-rei/a.txt", "b.txt", vec![excel()]);
    click(&mut a, InUseHit::Retry);
    assert_eq!(FakeFileOps::take_ops(), vec![op], "se repite la misma operación");
    assert_eq!(host.aux_hides_of(AuxWindow::InUse), 1);
    assert!(!host.aux_visible_of(AuxWindow::InUse));
    assert!(a.in_use_dialog().is_none());
    // Con un trabajo, «Reintentar» lanza el reintento del motor.
    let f = fake();
    let paths = locked_delete(&mut a, &f, "/iu-rej", &["a.txt"]);
    wait(&mut a, idle);
    assert!(a.in_use_dialog().is_some());
    a.take_started();
    click(&mut a, InUseHit::Retry);
    let started = a.take_started();
    assert_eq!(started.len(), 1, "{started:?}");
    assert!(matches!(&started[0], JobSpec::Delete { paths: again, recycle: true } if *again == paths), "{started:?}");
    assert_eq!(host.aux_hides_of(AuxWindow::InUse), 2);
    assert!(a.in_use_dialog().is_none());
    wait(&mut a, idle);
    assert!(!f.exists(&paths[0]), "el reintento lo borró");
}

#[test]
fn close_button_and_escape_hide_it() {
    let (mut a, host) = app_window();
    let op = failed_rename(&mut a, "/iu-cer/a.txt", "b.txt", vec![excel()]);
    // El mouse sobre un botón lo resalta (se repinta).
    let painted = host.aux_invalidations_of(AuxWindow::InUse);
    let (x, y) = find_button(&a, InUseHit::Close);
    a.aux_mouse(AuxWindow::InUse, x, y, AuxMouse::Move);
    assert!(host.aux_invalidations_of(AuxWindow::InUse) > painted);
    click(&mut a, InUseHit::Close);
    assert_eq!(host.aux_hides_of(AuxWindow::InUse), 1);
    assert!(a.in_use_dialog().is_none());
    assert!(FakeFileOps::take_ops().is_empty(), "cerrar no reintenta");
    // Esc la cierra sin reintentar.
    failed_rename(&mut a, "/iu-cer/a.txt", "b.txt", vec![excel()]);
    assert!(a.aux_key(AuxWindow::InUse, VK_ESCAPE));
    assert_eq!(host.aux_hides_of(AuxWindow::InUse), 2);
    assert!(a.in_use_dialog().is_none());
    assert!(FakeFileOps::take_ops().is_empty(), "Esc no reintenta");
    // Enter reintenta.
    failed_rename(&mut a, "/iu-cer/a.txt", "b.txt", vec![excel()]);
    assert!(a.aux_key(AuxWindow::InUse, VK_RETURN));
    assert_eq!(FakeFileOps::take_ops(), vec![op]);
    assert_eq!(host.aux_hides_of(AuxWindow::InUse), 3);
    // Cerrada por el usuario (la cruz de la ventana): queda cerrada, y la próxima vez se abre de nuevo.
    failed_rename(&mut a, "/iu-cer/a.txt", "b.txt", vec![excel()]);
    host.aux_user_closed_of(AuxWindow::InUse);
    a.aux_closed(AuxWindow::InUse);
    assert!(a.in_use_dialog().is_none());
    assert!(FakeFileOps::take_ops().is_empty());
    let shown = in_use_shows(&host).len();
    failed_rename(&mut a, "/iu-cer/a.txt", "b.txt", vec![excel()]);
    assert_eq!(in_use_shows(&host).len(), shown + 1, "se abre con `aux_show`, no se reemplaza");
    // Las demás teclas no son de esta ventana.
    assert!(!a.aux_key(AuxWindow::InUse, crate::keys::keys::VK_DOWN));
}

#[test]
fn copy_window_unchanged() {
    let f = fake();
    let locked = p("/iu-cop/s/b.txt");
    f.add_file(&p("/iu-cop/s/a.txt"), b"a".to_vec());
    f.add_file(&locked, b"b".to_vec());
    f.add_dir(&p("/iu-cop/d"));
    f.fail(&locked, ErrKind::InUse, 1);
    f.set_holders(&locked, vec![excel()]);
    let (mut a, host) = app_window();
    a.start_transfer(JobSpec::Copy { sources: vec![p("/iu-cop/s/a.txt"), locked], dest: p("/iu-cop/d") });
    wait(&mut a, idle);
    // Las dos ventanas, cada una con lo suyo.
    let shows = host.aux_shows();
    assert_eq!(shows.iter().filter(|s| s.0 == AuxWindow::Transfers).count(), 1, "{shows:?}");
    assert_eq!(shows.iter().filter(|s| s.0 == AuxWindow::InUse).count(), 1, "{shows:?}");
    assert!(host.aux_visible_of(AuxWindow::Transfers) && host.aux_visible_of(AuxWindow::InUse));
    let copy = texts_of(&mut a, AuxWindow::Transfers);
    assert!(copy.iter().any(|s| s == "Terminó con 1 error"), "{copy:?}");
    assert!(copy.iter().any(|s| s == "Lo está usando otro programa"), "{copy:?}");
    let in_use = texts_of(&mut a, AuxWindow::InUse);
    assert!(in_use.iter().any(|s| s == "«b.txt» está en uso"), "{in_use:?}");
    assert!(in_use.iter().any(|s| s == "Microsoft Excel (EXCEL.EXE)"), "{in_use:?}");
    // Cerrar una no toca la otra.
    click(&mut a, InUseHit::Close);
    assert!(host.aux_visible_of(AuxWindow::Transfers));
    assert_eq!(host.aux_hides_of(AuxWindow::Transfers), 0);
    a.aux_key(AuxWindow::Transfers, VK_ESCAPE);
    assert_eq!(host.aux_hides_of(AuxWindow::InUse), 1);
}

/// `open_in_use` se puede llamar con cualquier diálogo armado (lo que muestra es ese).
#[test]
fn open_in_use_shows_the_given_dialog() {
    let (mut a, host) = app_window();
    let d = InUseDialog::from_job(9, &[crate::transfer::JobError {
        src: p("/iu-pub/x.txt"),
        dst: PathBuf::new(),
        message: String::new(),
        retry: true,
        entry: None,
        holders: vec![excel()],
    }])
    .unwrap();
    a.open_in_use(d.clone());
    assert_eq!(a.in_use_dialog(), Some(&d));
    assert_eq!(in_use_shows(&host).len(), 1);
}

//! Pruebas de las piezas del motor: recorrido, velocidad, control y mensajes de error.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::speed::Speed;
use super::walk::{Planned, totals, walk};
use super::{Control, Flow};
use crate::sys::testing::FakeCopyPrims;
use crate::sys::{CopyError, ErrKind, ItemKind};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn rels(items: &[Planned]) -> Vec<PathBuf> {
    items.iter().map(|i| i.rel.clone()).collect()
}

#[test]
fn walk_preorder_with_totals() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/w/A/x.txt"), vec![1; 10]);
    fake.add_file(&p("/w/A/B/y.txt"), vec![2; 20]);
    fake.add_file(&p("/w/A/B/z.txt"), vec![3; 30]);
    let ctl = Control::new();
    let items = walk(&fake, &p("/w/A"), &ctl, &mut |_| {}).unwrap();
    assert_eq!(rels(&items), vec![p("A"), p("A/B"), p("A/B/y.txt"), p("A/B/z.txt"), p("A/x.txt")]);
    assert_eq!(items[0].src, p("/w/A"));
    assert_eq!(items[0].stat.kind, ItemKind::Dir);
    assert_eq!(totals(&items), (5, 60));
    // Un archivo suelto es su propia raíz.
    let one = walk(&fake, &p("/w/A/x.txt"), &ctl, &mut |_| {}).unwrap();
    assert_eq!(rels(&one), vec![p("x.txt")]);
    assert_eq!(totals(&one), (1, 10));
}

#[test]
fn walk_does_not_read_content() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/w/nube/a.bin"), vec![0; 100]);
    fake.add_file(&p("/w/nube/b.bin"), vec![0; 5000]);
    fake.set_cloud_only(&p("/w/nube/b.bin"));
    let items = walk(&fake, &p("/w/nube"), &Control::new(), &mut |_| {}).unwrap();
    assert_eq!(totals(&items), (3, 5100));
    assert!(items.iter().any(|i| i.stat.cloud_only));
    assert_eq!(fake.reads(&p("/w/nube/a.bin")), 0);
    assert_eq!(fake.reads(&p("/w/nube/b.bin")), 0);
}

#[test]
fn walk_does_not_follow_symlinks() {
    let fake = FakeCopyPrims::new();
    fake.add_file(&p("/w/otra/grande.bin"), vec![0; 1000]);
    fake.add_dir(&p("/w/L"));
    fake.add_symlink(&p("/w/L/enlace"), &p("/w/otra"));
    let items = walk(&fake, &p("/w/L"), &Control::new(), &mut |_| {}).unwrap();
    assert_eq!(rels(&items), vec![p("L"), p("L/enlace")]);
    assert_eq!(items[1].stat.kind, ItemKind::Symlink);
    assert_eq!(totals(&items).0, 2);
}

#[test]
fn walk_cancel_stops() {
    let fake = FakeCopyPrims::new();
    for i in 0..500 {
        fake.add_file(&p(&format!("/w/muchos/{i:03}.txt")), vec![0; 1]);
    }
    let ctl = Control::new();
    let mut seen = Vec::new();
    let all = walk(&fake, &p("/w/muchos"), &ctl, &mut |n| seen.push(n)).unwrap();
    assert_eq!(all.len(), 501);
    // «Preparando… N elementos» cada 200.
    assert_eq!(seen, vec![200, 400]);
    let ctl2 = Control::new();
    let c = ctl2.clone();
    let err = walk(&fake, &p("/w/muchos"), &ctl2, &mut |_| c.cancel()).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
}

#[test]
fn walk_missing_root_is_error() {
    let fake = FakeCopyPrims::new();
    let err = walk(&fake, &p("/w/no-esta"), &Control::new(), &mut |_| {}).unwrap_err();
    assert_eq!(err.kind, ErrKind::NotFound);
}

const MB: u64 = 1_000_000;

#[test]
fn speed_eta_after_two_seconds() {
    let t0 = Instant::now();
    let mut s = Speed::new(t0, 100 * MB);
    s.push(t0 + Duration::from_millis(500), 5 * MB);
    assert_eq!(s.eta(t0 + Duration::from_millis(500)), None);
    s.push(t0 + Duration::from_millis(1500), 15 * MB);
    assert_eq!(s.eta(t0 + Duration::from_millis(1500)), None);
    s.push(t0 + Duration::from_millis(2500), 25 * MB);
    let eta = s.eta(t0 + Duration::from_millis(2500)).unwrap();
    // 75 MB a 10 MB/s.
    assert!((eta.as_secs_f64() - 7.5).abs() < 0.2, "{eta:?}");
}

#[test]
fn speed_moving_average_five_seconds() {
    let t0 = Instant::now();
    let mut s = Speed::new(t0, 1000 * MB);
    let mut done = 0;
    // 10 MB/s durante 4 s y después 20 MB/s durante 6 s, en pasos de medio segundo.
    for i in 1..=20 {
        let rate = if i <= 8 { 10 } else { 20 };
        done += rate * MB / 2;
        s.push(t0 + Duration::from_millis(500 * i), done);
    }
    let rate = s.rate().unwrap();
    assert!((rate - 20.0 * MB as f64).abs() < 0.5 * MB as f64, "{rate}");
    let avg = s.average().unwrap();
    assert!((avg - 16.0 * MB as f64).abs() < 0.5 * MB as f64, "{avg}");
}

#[test]
fn speed_graph_one_point_per_percent() {
    let t0 = Instant::now();
    let total = 1000 * MB;
    let mut s = Speed::new(t0, total);
    assert!(s.graph().is_empty());
    for i in 1..=400u64 {
        s.push(t0 + Duration::from_millis(50 * i), total * i / 400);
    }
    assert_eq!(s.graph().len(), 100);
    let mut half = Speed::new(t0, total);
    for i in 1..=200u64 {
        half.push(t0 + Duration::from_millis(50 * i), total * i / 400);
    }
    assert_eq!(half.graph().len(), 50);
    assert!(half.graph().iter().all(|v| *v > 0.0));
}

#[test]
fn control_pause_blocks_until_resume() {
    let ctl = Control::new();
    ctl.pause();
    let passed = Arc::new(AtomicBool::new(false));
    let (c, flag) = (ctl.clone(), passed.clone());
    let h = std::thread::spawn(move || {
        let f = c.checkpoint();
        flag.store(true, Ordering::SeqCst);
        f
    });
    std::thread::sleep(Duration::from_millis(80));
    assert!(!passed.load(Ordering::SeqCst), "siguió estando en pausa");
    ctl.resume();
    assert_eq!(h.join().unwrap(), Flow::Continue);

    // Cancelar libera al que espera en pausa, con Stop.
    ctl.pause();
    let c = ctl.clone();
    let h = std::thread::spawn(move || c.checkpoint());
    std::thread::sleep(Duration::from_millis(50));
    ctl.cancel();
    assert_eq!(h.join().unwrap(), Flow::Stop);
    assert!(ctl.is_cancelled());
    assert_eq!(ctl.checkpoint(), Flow::Stop);
}

#[test]
fn control_answer_and_cancel_while_waiting() {
    use super::{Answer, Choice};
    let ctl = Control::new();
    let c = ctl.clone();
    let h = std::thread::spawn(move || c.wait_answer());
    std::thread::sleep(Duration::from_millis(30));
    ctl.answer(Answer { choice: Choice::KeepBoth, all: true });
    assert_eq!(h.join().unwrap(), Some(Answer { choice: Choice::KeepBoth, all: true }));
    let c = ctl.clone();
    let h = std::thread::spawn(move || c.wait_answer());
    std::thread::sleep(Duration::from_millis(30));
    ctl.cancel();
    assert_eq!(h.join().unwrap(), None);
}

#[test]
fn copy_error_messages() {
    let e = |kind| CopyError::new(kind, Path::new("/x")).message();
    assert_eq!(e(ErrKind::InUse), "Lo está usando otro programa");
    assert_eq!(e(ErrKind::Denied), "No tenés permiso");
    assert_eq!(e(ErrKind::NotFound), "Ya no existe");
    assert_eq!(e(ErrKind::DiskFull), "El disco está lleno");
    assert_eq!(e(ErrKind::TooBig), "Es demasiado grande para el formato del disco de destino (FAT32)");
    assert_eq!(e(ErrKind::Exists), "Ya existe");
    assert_eq!(e(ErrKind::Cancelled), "Cancelado");
    assert_eq!(e(ErrKind::Other("raro".into())), "raro");
}

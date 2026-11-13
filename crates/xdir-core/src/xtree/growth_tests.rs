//! Pruebas del historial de crecimiento en la vista de xtree (#58).

use super::*;
use crate::sys::testing::{FakeNetwork, install_fakes};
use crate::xtree::history;
use crate::xtree::tree::{NO_PARENT, Node};

const MB: u64 = 1024 * 1024;

fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
    Node { name: name.into(), parent, size, is_dir, small: false, children }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xdir-xtree-hist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// raíz ─ a (dir) ─ a1 (dir)
///      ├ b (dir)
///      └ f.bin
fn tree(root: &str, a: u64, a1: u64, b: u64) -> Vec<Node> {
    vec![
        node(root, NO_PARENT, a + b + 10, true, vec![1, 2, 3]),
        node("a", 0, a, true, vec![4]),
        node("b", 0, b, true, vec![]),
        node("f.bin", 0, 10, false, vec![]),
        node("a1", 1, a1, true, vec![]),
    ]
}

/// Vista sobre `root` con el historial en `dir` y hoy = `today`.
fn view(root: &Path, dir: &Path, today: i32) -> Xtree {
    install_fakes();
    let mut x = Xtree::idle(root.to_path_buf());
    x.set_history(true, 7, Some(dir.to_path_buf()));
    x.set_today(Some(today));
    x
}

/// Termina un análisis con `arena` (mismo camino que el aviso de entrega del escáner).
fn finish_with(x: &mut Xtree, arena: Vec<Node>) {
    x.scanning = true;
    *x.scan.result.lock().unwrap() = Some(arena);
    assert!(x.on_message(x.scan.id, 1));
}

/// Espera a que llegue el historial del hilo (como al repintar).
fn wait_history(x: &mut Xtree) {
    let t0 = Instant::now();
    let area = rect(0.0, 0.0, 1400.0, 790.0);
    loop {
        x.prepare(area);
        if !x.hist.pending && x.hist.loaded.is_some() {
            return;
        }
        assert!(t0.elapsed() < Duration::from_secs(5), "el historial no llegó");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Un análisis completo guarda la foto y trae el historial en otro hilo: el de la ventana nunca lee
/// ni escribe el historial (auditoría del hilo de interfaz).
#[test]
fn snapshot_saved_and_loaded_off_ui_thread() {
    for i in 0..20 {
        let dir = tmp(&format!("guardar{i}"));
        let root = tmp(&format!("raiz-guardar{i}"));
        FakeNetwork::mark_ui_thread();
        let mut x = view(&root, &dir, 500);
        finish_with(&mut x, tree("r", 300 * MB, 200 * MB, 5 * MB));
        assert!(x.hist.pending);
        wait_history(&mut x);
        let l = x.hist.loaded.clone().unwrap();
        assert_eq!(l.days, [500]);
        assert_eq!(l.value(0, "a/a1"), Some(200 * MB));
        x.set_today(Some(501));
        finish_with(&mut x, tree("r", 400 * MB, 300 * MB, 5 * MB));
        wait_history(&mut x);
        assert_eq!(x.hist.loaded.as_ref().unwrap().days, [500, 501]);
        assert!(FakeNetwork::ui_io_calls().iter().all(|c| !c.starts_with("historial")), "{:?}", FakeNetwork::ui_io_calls());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// La auditoría funciona: leer el historial en el hilo marcado queda anotado.
#[test]
fn ui_thread_audit_catches_history_io() {
    install_fakes();
    FakeNetwork::mark_ui_thread();
    let dir = tmp("auditoria");
    let _ = history::load_for(&dir, &tmp("raiz-auditoria"));
    assert!(FakeNetwork::ui_io_calls().iter().any(|c| c.starts_with("historial leer")), "{:?}", FakeNetwork::ui_io_calls());
}

/// Con el historial apagado no se guarda la foto, pero se lee lo que había.
#[test]
fn switch_off_reads_without_saving() {
    let dir = tmp("apagado");
    let root = tmp("raiz-apagado");
    history::save_snapshot(&dir, &root, 400, &[("".into(), 7)]).unwrap();
    let mut x = view(&root, &dir, 500);
    x.set_history(false, 30, Some(dir.clone()));
    assert_eq!(x.history_days(), 30);
    finish_with(&mut x, tree("r", 1, 1, 1));
    wait_history(&mut x);
    assert_eq!(x.hist.loaded.as_ref().unwrap().days, [400]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un disco de red no tiene historial: ni hilo ni archivo.
#[test]
fn network_root_has_no_history() {
    let dir = tmp("red");
    let root = tmp("raiz-red");
    let mut x = view(&root, &dir, 500);
    FakeNetwork::set_remote(&root);
    finish_with(&mut x, tree("r", 1, 1, 1));
    assert!(!x.hist.pending);
    assert!(x.hist.loaded.is_none());
    assert!(!dir.exists());
}

/// Un análisis cancelado no deja foto; el historial de otra raíz que llega tarde no se muestra.
#[test]
fn cancelled_scan_and_stale_history() {
    let dir = tmp("cancelado");
    let root = tmp("raiz-cancelado");
    let mut x = view(&root, &dir, 500);
    x.scanning = true;
    x.cancel();
    *x.scan.result.lock().unwrap() = None;
    assert!(!x.hist.pending);
    // El hilo terminó para la raíz anterior después de cambiar de raíz.
    finish_with(&mut x, tree("r", 1, 1, 1));
    x.root = tmp("raiz-otra");
    x.rescan();
    x.cancel();
    let t0 = Instant::now();
    while !dir.exists() {
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    x.prepare(rect(0.0, 0.0, 1400.0, 790.0));
    assert!(x.hist.loaded.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

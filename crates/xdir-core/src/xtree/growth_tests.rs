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

// ---------- tarjeta ----------

const AREA: Rect = Rect { left: 0.0, top: 0.0, right: 1400.0, bottom: 790.0 };
const GB: u64 = 1024 * MB;

fn texts(x: &mut Xtree) -> Vec<String> {
    let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
    x.paint(&p, AREA);
    p.texts().into_iter().map(|(_, s, _)| s).collect()
}

fn center(r: Rect) -> (f32, f32) {
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

/// Vista con historial guardado de antes (días 480 y 493) y un análisis de hoy (500).
fn with_history(name: &str) -> (Xtree, PathBuf) {
    let dir = tmp(name);
    let root = tmp(&format!("raiz-{name}"));
    history::save_snapshot(&dir, &root, 480, &[("".into(), 4 * GB), ("a".into(), GB), ("a/a1".into(), GB), ("b".into(), 2 * GB)]).unwrap();
    history::save_snapshot(&dir, &root, 493, &[("".into(), 5 * GB), ("a".into(), 2 * GB), ("a/a1".into(), GB), ("b".into(), 2 * GB)]).unwrap();
    let mut x = view(&root, &dir, 500);
    finish_with(&mut x, tree("r", 7 * GB, 3 * GB, 3 * GB));
    wait_history(&mut x);
    (x, dir)
}

/// Con historial, la tarjeta crece hacia arriba (el mapa se achica) y los botones quedan en la
/// parte de siempre; sin carpeta del historial, todo igual que antes.
#[test]
fn card_grows_with_strip() {
    let (x, dir) = with_history("crece");
    let (base, ui) = (layout_ui(AREA, 0.0), x.ui(AREA));
    assert_eq!(ui.card.top, base.card.top - growth::STRIP_H);
    assert_eq!(ui.map.bottom, base.map.bottom - growth::STRIP_H);
    assert_eq!(ui.card.bottom, base.card.bottom);
    assert!(ui.buttons.iter().all(|b| b.bottom < ui.card.top + CARD_H));
    // Una ventana baja no tiene lugar para la franja.
    let short = rect(0.0, 0.0, 1400.0, 470.0);
    assert_eq!(x.ui(short).card.top, layout_ui(short, 0.0).card.top);
    let mut plain = Xtree::idle(PathBuf::from("raiz"));
    plain.set_arena(tree("r", 1, 1, 1));
    assert_eq!(plain.ui(AREA).card.top, base.card.top);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Con datos: selector, cambio, fotos, gráfico y las que más crecieron.
#[test]
fn card_shows_change_chart_and_growers() {
    let (mut x, dir) = with_history("datos");
    let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
    x.paint(&p, AREA);
    let t: Vec<String> = p.texts().into_iter().map(|(_, s, _)| s).collect();
    for want in ["7 días", "30 días", "90 días", "+5,0 GB en 7 días", "Foto del 09/05: 5,00 GB", "Más crecieron", "a", "+5,0 GB", "b", "+1,0 GB"] {
        assert!(t.iter().any(|s| s == want), "falta «{want}»: {t:?}");
    }
    // El gráfico: una línea con la foto de comparación y hoy.
    assert!(p.paths().iter().any(|(pts, closed, _)| !closed && pts.len() == 2), "{:?}", p.paths());
    let _ = std::fs::remove_dir_all(&dir);
}

/// El selector cambia el período (y se recuerda en la vista); un clic en una de «Más crecieron»
/// entra en esa carpeta del mapa.
#[test]
fn card_period_and_grower_clicks() {
    let (mut x, dir) = with_history("clics");
    x.prepare(AREA);
    let s = growth::strip(x.ui(AREA).card);
    let (px, py) = center(s.periods[1]);
    assert!(x.on_mouse_move(AREA, px, py));
    assert_eq!(x.hist.hover, Some(StripHit::Period(1)));
    assert_eq!(x.on_click(AREA, px, py, false), Action::Redraw);
    assert_eq!(x.history_days(), 30);
    assert!(texts(&mut x).iter().any(|s| s == "+6,0 GB en 20 días"), "con la foto del día 480");
    // Primera de «Más crecieron»: «a».
    let row = rect(s.right.left, s.right.top + 20.0, s.right.right, s.right.top + 39.0);
    let (gx, gy) = center(row);
    assert_eq!(x.on_click(AREA, gx, gy, false), Action::Redraw);
    assert_eq!(&*x.arena[x.current as usize].name, "a");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sin historial, con una sola foto, una carpeta sin datos y un archivo.
#[test]
fn card_empty_states() {
    let dir = tmp("vacio");
    let root = tmp("raiz-vacio");
    let mut x = view(&root, &dir, 500);
    x.set_history(false, 7, Some(dir.clone()));
    finish_with(&mut x, tree("r", 1, 1, 1));
    wait_history(&mut x);
    assert!(texts(&mut x).iter().any(|s| s == "El historial está apagado"));
    x.set_history(true, 7, Some(dir.clone()));
    assert!(texts(&mut x).iter().any(|s| s == "Sin historial todavía: se arma cada vez que analizás"));
    finish_with(&mut x, tree("r", 1, 1, 1));
    wait_history(&mut x);
    // 500 es el 16/05/1971.
    assert!(texts(&mut x).iter().any(|s| s == "Primera foto: 16/05"), "{:?}", texts(&mut x));
    x.selected = Some(3);
    assert!(texts(&mut x).iter().any(|s| s == "Los archivos no tienen historial"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Mientras analiza, la franja lo dice (y no responde a clics).
#[test]
fn card_while_scanning() {
    let (mut x, dir) = with_history("analizando");
    x.scanning = true;
    x.scan.live.write().unwrap().extend(tree("r", 1, 1, 1));
    assert!(texts(&mut x).iter().any(|s| s == "El historial se actualiza al terminar el análisis"));
    let s = growth::strip(x.ui(AREA).card);
    let (px, py) = center(s.periods[2]);
    assert_eq!(x.on_click(AREA, px, py, false), Action::None);
    assert_eq!(x.history_days(), 7);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- ▲ en la tabla ----------

/// La ▲ va junto al tamaño de las carpetas que crecieron mucho; con el mouse quieto encima del
/// tamaño aparece el cartel con el cambio.
#[test]
fn table_marker_and_tooltip() {
    let (mut x, dir) = with_history("flecha");
    let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
    x.paint(&p, AREA);
    let tris: Vec<_> = p.paths().into_iter().filter(|(pts, closed, fill)| *closed && pts.len() == 3 && fill.is_some()).collect();
    // «a» (+5 GB) sí; «b» (+1 GB, 50 %) también; el archivo no.
    assert_eq!(tris.len(), 2, "{tris:?}");
    // El cartel es el de xdir (`App::tooltip_text`): la vista dice qué fila tiene ▲ bajo el mouse
    // (sobre su tamaño) y el texto.
    let l = x.ui(AREA).list;
    let (mx, my) = (l.right - 150.0, l.top + TABLE_H + ROW_H / 2.0);
    let a = x.mark_at_point(AREA, mx, my).expect("▲ de «a»");
    assert_eq!(&*x.arena[a as usize].name, "a");
    assert_eq!(x.mark_tip(a).as_deref(), Some("+5,0 GB en 7 días"));
    assert_eq!(x.mark_at_point(AREA, l.left + 60.0, my), None, "sobre el nombre no");
    assert_eq!(x.mark_at_point(AREA, mx, my + 2.0 * ROW_H), None, "el archivo no tiene ▲");
    // Con la búsqueda abierta la tabla no se ve: ni ▲ bajo el mouse ni cartel que quede.
    x.open_search();
    assert_eq!(x.mark_at_point(AREA, mx, my), None);
    assert_eq!(x.mark_tip(a), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Después de «Borrar historial», la tarjeta dice que no hay historial (no queda «cargando»).
#[test]
fn cleared_history_shows_empty() {
    let (mut x, dir) = with_history("borrado");
    x.clear_history();
    assert!(texts(&mut x).iter().any(|s| s == "Sin historial todavía: se arma cada vez que analizás"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Una foto que salió antes de «Borrar historial» no vuelve a crear la carpeta: el hilo ve que la
/// generación cambió y entrega un historial vacío.
#[test]
fn save_started_before_clear_is_dropped() {
    let dir = tmp("borrar-en-camino");
    let root = tmp("raiz-borrar-en-camino");
    let mut x = view(&root, &dir, 500);
    let stale = x.hist.clears.load(std::sync::atomic::Ordering::SeqCst);
    history::clear(&dir, &x.hist.clears).unwrap();
    x.scanning = true;
    *x.scan.result.lock().unwrap() = Some(tree("r", 1, 1, 1));
    x.take_result_with_generation(stale);
    wait_history(&mut x);
    assert!(x.hist.loaded.as_ref().unwrap().is_empty());
    assert!(!dir.exists(), "no se vuelve a crear");
}

/// El hilo de la ventana nunca espera al candado que tienen los hilos del historial mientras
/// guardan, leen o borran: terminar un análisis con otro guardado en curso no la traba.
#[test]
fn ui_thread_never_waits_for_history_lock() {
    let dir = tmp("candado");
    let root = tmp("raiz-candado");
    let mut x = view(&root, &dir, 500);
    let (tx, rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _g = history::hold_io_lock_for_tests();
        tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
    });
    rx.recv().unwrap();
    let t0 = Instant::now();
    finish_with(&mut x, tree("r", 1, 1, 1));
    x.prepare(rect(0.0, 0.0, 1400.0, 790.0));
    assert!(t0.elapsed() < Duration::from_millis(500), "la ventana esperó {:?}", t0.elapsed());
    holder.join().unwrap();
    wait_history(&mut x);
    let _ = std::fs::remove_dir_all(&dir);
}

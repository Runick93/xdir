//! Pruebas del historial de xtree del lado de la App (#58).

use super::*;
use crate::sys::testing::install_fakes;

fn app() -> App {
    install_fakes();
    App::new(Init::blank(), crate::sys::testing::FakeHost::new())
}

/// El período de la tarjeta sale de la sesión y vuelve a ella (lo que eligió la vista).
#[test]
fn period_comes_from_and_goes_to_session() {
    let mut a = app();
    a.xtree_days = 30;
    a.run_command(Command::Xtree);
    let x = a.xtree.as_mut().unwrap();
    assert_eq!(x.history_days(), 30);
    x.set_history(true, 90, None);
    assert_eq!(a.session().xtree_days, 90);
    a.on_closing();
}

/// La vista recibe la carpeta del historial dentro de la de configuración.
#[test]
fn view_gets_history_folder() {
    let mut a = app();
    a.run_command(Command::Xtree);
    let dir = crate::session::config_dir().unwrap().join("xtree-historial");
    assert_eq!(a.xtree.as_ref().unwrap().history_dir(), Some(dir.as_path()));
    a.on_closing();
}

// ---------- Configuración ▸ xtree ----------

/// La configuración abierta en un solo panel grande en la página xtree, ya dibujada.
fn settings_xtree() -> (App, std::rc::Rc<crate::sys::testing::FakeHost>) {
    install_fakes();
    let _ = std::fs::remove_file(config::file_path().unwrap());
    let host = crate::sys::testing::FakeHost::new();
    let mut a = App::new(Init::blank(), host.clone());
    host.client.set((1800.0, 1400.0));
    a.set_layout(Layout::One);
    a.run_command(Command::Settings);
    if let Some(v) = a.settings_view_mut(0) {
        v.set_page(crate::settings::PageId::Builtin(crate::settings::Section::Xtree));
    }
    a.paint();
    (a, host)
}

fn click(a: &mut App, id: &'static str, part: usize) {
    let r = a.part_rect(0, id, part).unwrap_or_else(|| panic!("no se dibujó {id}/{part}"));
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
    a.on_left_up((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    a.paint();
}

fn wait_config(a: &mut App) {
    while a.config_saving {
        let t = a.task_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("la escritura termina");
        a.on_task_done(t);
    }
}

fn config_text() -> String {
    std::fs::read_to_string(config::file_path().unwrap()).unwrap_or_default()
}

/// «Guardar historial de tamaños» escribe `xtree_historial = false` (y lo quita al volver a
/// encenderlo) y la vista abierta se entera.
#[test]
fn history_switch_writes_option() {
    let (mut a, _host) = settings_xtree();
    a.xtree = Some(Xtree::unscanned(a.hwnd, std::env::temp_dir(), false));
    click(&mut a, "xtree_historial", 0);
    assert!(!a.config.xtree_history);
    assert!(!a.xtree.as_ref().unwrap().history_on());
    wait_config(&mut a);
    assert!(config_text().contains("# xtree_historial = true\nxtree_historial = false\n"), "{}", config_text());
    click(&mut a, "xtree_historial", 0);
    assert!(a.config.xtree_history);
    assert!(a.xtree.as_ref().unwrap().history_on());
    wait_config(&mut a);
    assert!(!config_text().contains("\nxtree_historial = false"));
}

/// «Borrar historial» pregunta; con Sí borra la carpeta `xtree-historial` en un hilo y lo dice.
#[test]
fn clear_history_asks_and_deletes() {
    let (mut a, host) = settings_xtree();
    let dir = crate::session::config_dir().unwrap().join(crate::xtree::history::DIR_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.bin"), b"XDHT").unwrap();
    host.set_answer(None);
    click(&mut a, "xtree_borrar_historial", 0);
    assert_eq!(host.questions().last().map(|q| q.0.as_str()), Some("Borrar historial"));
    assert!(dir.exists(), "con No queda todo");
    host.set_answer(Some(true));
    click(&mut a, "xtree_borrar_historial", 0);
    super::super::network_tests::wait_until(&mut a, |a| a.xtree_history_msg.is_some());
    assert!(!dir.exists());
    assert_eq!(a.xtree_history_msg.as_deref(), Some("Historial borrado"));
    // Sin nada que borrar también sale bien.
    click(&mut a, "xtree_borrar_historial", 0);
    a.xtree_history_msg = None;
    super::super::network_tests::wait_until(&mut a, |a| a.xtree_history_msg.is_some());
    assert_eq!(a.xtree_history_msg.as_deref(), Some("Historial borrado"));
}

/// El cartel de la ▲ es el de xdir: `hit` da la fila con ▲ bajo el mouse, el cartel tiene el
/// cambio, y el clic sigue yendo a la vista (elige la fila).
#[test]
fn marker_tooltip_uses_app_tooltips() {
    let mut a = app();
    let dir = crate::session::config_dir().unwrap().join(crate::xtree::history::DIR_NAME);
    let _ = std::fs::remove_dir_all(&dir);
    let root = crate::sys::testing::FakeDiskScan::root();
    crate::xtree::history::save_snapshot(&dir, &root, 400, &[("".into(), 1), ("grande".into(), 1)]).unwrap();
    // Sin analizar al abrir: el árbol lo pone la prueba.
    a.run_command(Command::Xtree);
    let x = a.xtree.as_mut().unwrap();
    x.set_today(Some(407));
    let mut big = vec![crate::xtree::tree::Node { name: root.to_string_lossy().into(), parent: crate::xtree::tree::NO_PARENT, size: 3 << 30, is_dir: true, small: false, children: vec![1] }];
    big.push(crate::xtree::tree::Node { name: "grande".into(), parent: 0, size: 3 << 30, is_dir: true, small: false, children: vec![] });
    x.finish_for_tests(big);
    let t0 = std::time::Instant::now();
    while a.xtree.as_ref().unwrap().history_loaded().is_none_or(|l| l.days.len() < 2) {
        assert!(t0.elapsed() < std::time::Duration::from_secs(10), "el historial no llegó");
        a.paint();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let area = a.xtree_area();
    let (mx, my) = a.xtree.as_ref().unwrap().size_cell_center(area, 0);
    let Hit::XtreeMark(n) = a.hit(mx, my) else { panic!("{:?}", a.hit(mx, my)) };
    assert_eq!(a.tooltip_text(Hit::XtreeMark(n)).as_deref(), Some("+3,0 GB en 7 días"));
    a.on_left_down(mx, my, false);
    a.on_left_up(mx, my);
    assert_eq!(a.xtree.as_ref().unwrap().selected_for_tests(), Some(n));
    a.on_closing();
}

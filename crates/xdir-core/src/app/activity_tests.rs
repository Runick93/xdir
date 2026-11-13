//! Pruebas del indicador de actividad de la barra de estado (Ajustes 2): umbral, cartel, clic y
//! temporizador. El reloj se fija (`App::clock`): nada depende de cuánto tarda la prueba.

use super::activity::ActivityKey;
use super::*;
use crate::sys::testing::FakeHost;
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Una carpeta propia con una subcarpeta `a/` (con un archivo) y la `App` ya en ella.
fn app_in(name: &str) -> (App, Rc<FakeHost>) {
    let dir = std::env::temp_dir().join("xdir-test-actividad").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::write(dir.join("a").join("x.bin"), vec![0u8; 100]).unwrap();
    crate::sys::testing::install_fakes();
    let host = FakeHost::new();
    let mut a = App::new(Init::blank(), host.clone());
    a.sync_visibility();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir));
    let t0 = Instant::now();
    // Todos los paneles terminaron de leer (si no, su lectura también está en curso).
    while a.panes.iter().any(|p| p.tabs.iter().any(|t| t.loading)) {
        assert!(t0.elapsed() < Duration::from_secs(10), "la carpeta no se leyó");
        if let Ok(ev) = a.rx.recv_timeout(ms(20)) {
            a.on_load_event(ev);
        }
    }
    (a, host)
}

fn center(r: geom::Rect) -> (f32, f32) {
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

/// El tamaño de una carpeta elegida: el indicador aparece recién a los 0,3 s, con su cartel; al
/// terminar se va.
#[test]
fn indicator_after_threshold_with_tooltip() {
    let (mut a, _host) = app_in("umbral");
    let t0 = Instant::now();
    a.clock = Some(t0);
    let list = &mut a.panes[0].tab_mut().list;
    let idx = list.entries.iter().position(|e| e.name == "a").unwrap();
    let pos = (0..list.count()).find(|&p| list.entry_of_pos(p) == Some(idx)).unwrap();
    list.select_pos(pos);
    a.paint();
    assert_eq!(a.sel_size.total(), crate::tab::SelTotal::Computing);
    assert!(a.activity.contains(&ActivityKey::SelSize));
    assert_eq!(a.activity_rect, None, "antes de 0,3 s no se ve");
    a.clock = Some(t0 + ms(299));
    a.paint();
    assert_eq!(a.activity_rect, None);
    a.clock = Some(t0 + ms(300));
    a.paint();
    let r = a.activity_rect.expect("el indicador a los 0,3 s");
    let (x, y) = center(r);
    assert_eq!(a.hit(x, y), Hit::Activity);
    assert_eq!(a.tooltip_text(Hit::Activity).as_deref(), Some("Calculando el tamaño de 1 carpeta"));
    // Sin copias el clic no abre nada.
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    // Llega el tamaño: se va.
    let t = Instant::now();
    while a.sel_size.total() == crate::tab::SelTotal::Computing {
        assert!(t.elapsed() < Duration::from_secs(10), "el tamaño no llegó");
        std::thread::sleep(ms(5));
        a.on_sel_size();
    }
    a.paint();
    assert!(!a.activity.contains(&ActivityKey::SelSize));
    assert_eq!(a.activity_rect, None);
    assert_eq!(a.tooltip_text(Hit::Activity), None);
}

/// Una lectura de carpeta entra en el indicador desde los 0,5 s (como «Cargando…»).
#[test]
fn slow_load_in_indicator_from_half_second() {
    let (mut a, _host) = app_in("lectura");
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Computer);
    let t0 = Instant::now();
    a.panes[0].tab_mut().set_loading_since(Some(t0));
    a.clock = Some(t0 + ms(400));
    a.paint();
    assert_eq!(a.activity_rect, None);
    a.clock = Some(t0 + ms(500));
    a.paint();
    assert!(a.activity_rect.is_some());
    assert_eq!(a.tooltip_text(Hit::Activity).as_deref(), Some("Cargando \"Este equipo\""));
}

/// El temporizador corre solo mientras haya algo en curso.
#[test]
fn timer_only_while_active() {
    let (mut a, _host) = app_in("temporizador");
    a.paint();
    assert!(a.activity.is_empty());
    assert_eq!(a.activity.tick_due, None, "sin nada en curso, ningún temporizador");
    let now = a.now();
    a.activity.begin(ActivityKey::Extract, "Extrayendo \"x\"", now);
    a.paint();
    assert!(a.activity.tick_due.is_some());
    a.activity.end(&ActivityKey::Extract);
    a.on_activity_tick();
    a.paint();
    assert_eq!(a.activity.tick_due, None);
}

/// Varias cosas a la vez: el cartel las lista, de la más vieja a la más nueva.
#[test]
fn tooltip_lists_everything() {
    let (mut a, _host) = app_in("lista");
    let t0 = Instant::now();
    a.activity.begin(ActivityKey::Connect(1), "Conectando con \"nas\"", t0);
    a.activity.begin(ActivityKey::Extract, "Extrayendo \"foto.jpg\"", t0 + ms(100));
    a.clock = Some(t0 + ms(350));
    a.paint();
    assert_eq!(a.tooltip_text(Hit::Activity).as_deref(), Some("Conectando con \"nas\""));
    a.clock = Some(t0 + ms(450));
    a.paint();
    assert_eq!(a.tooltip_text(Hit::Activity).as_deref(), Some("Conectando con \"nas\" · Extrayendo \"foto.jpg\""));
}

/// Una lectura rápida no prende el temporizador; pasados los 0,5 s, sí (gira «Cargando…»).
#[test]
fn fast_load_starts_no_timer() {
    let (mut a, _host) = app_in("lectura-rapida");
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Computer);
    let t0 = Instant::now();
    a.panes[0].tab_mut().set_loading_since(Some(t0));
    a.clock = Some(t0 + ms(100));
    a.paint();
    assert!(!a.activity.is_empty());
    assert_eq!(a.activity.tick_due, None, "sin temporizador antes de los 0,5 s");
    a.clock = Some(t0 + ms(600));
    a.paint();
    assert!(a.activity.tick_due.is_some());
}

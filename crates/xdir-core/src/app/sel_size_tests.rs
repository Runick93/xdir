//! Pruebas del tamaño de lo elegido (carpetas incluidas) en la barra de estado.

use super::*;
use crate::sys::testing::{FakeFileSystem, FakeHost, FakeNetwork};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Carpeta propia de la prueba: `a/` con 100 bytes, `b/sub/` con 50 y `f.txt` con 10.
fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-sel-size").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b").join("sub")).unwrap();
    std::fs::write(dir.join("a").join("x.bin"), vec![0u8; 100]).unwrap();
    std::fs::write(dir.join("b").join("sub").join("y.bin"), vec![0u8; 50]).unwrap();
    std::fs::write(dir.join("f.txt"), vec![0u8; 10]).unwrap();
    dir
}

/// Una `App` con la carpeta ya leída en el panel activo.
fn app_in(dir: &Path) -> App {
    crate::sys::testing::install_fakes();
    let mut a = App::new(Init::blank(), FakeHost::new());
    a.sync_visibility();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    let t0 = Instant::now();
    while a.panes[0].tab().loading {
        assert!(t0.elapsed() < Duration::from_secs(10), "la carpeta no se leyó");
        if let Ok(ev) = a.rx.recv_timeout(Duration::from_millis(20)) {
            a.on_load_event(ev);
        }
    }
    a
}

/// Elige solo esos elementos (por nombre) y dibuja (ahí se arranca el cálculo).
fn select(a: &mut App, names: &[&str]) {
    let list = &mut a.panes[0].tab_mut().list;
    for (i, name) in names.iter().enumerate() {
        let idx = list.entries.iter().position(|e| e.name == *name).unwrap_or_else(|| panic!("no está {name}"));
        let pos = (0..list.count()).find(|&p| list.entry_of_pos(p) == Some(idx)).unwrap();
        if i == 0 {
            list.select_pos(pos);
        } else {
            list.set_selected_pos(pos, true);
        }
    }
    a.paint();
}

/// Entrega los tamaños que lleguen hasta que la barra diga `done` (o 10 s).
fn wait_status(a: &mut App, done: &str) {
    let t0 = Instant::now();
    loop {
        a.on_sel_size();
        a.paint();
        if a.status_text().ends_with(done) {
            return;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "la barra dice «{}», se esperaba «…{done}»", a.status_text());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn size(b: u64) -> String {
    crate::format::size_short(b)
}

/// Con carpetas elegidas la barra dice «calculando…» y después el total de archivos y carpetas.
#[test]
fn folders_show_calculating_then_total() {
    let dir = fixture("total");
    let mut a = app_in(&dir);
    select(&mut a, &["a", "b", "f.txt"]);
    assert!(a.status_text().ends_with("3 elementos seleccionados · calculando\u{2026}"), "{}", a.status_text());
    wait_status(&mut a, &format!("3 elementos seleccionados · {}", size(160)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Solo archivos: el tamaño sale de la lista, sin hilo.
#[test]
fn files_only_no_thread() {
    let dir = fixture("archivos");
    let mut a = app_in(&dir);
    let g = a.sel_size.generation();
    select(&mut a, &["f.txt"]);
    assert_eq!(a.sel_size.total(), crate::tab::SelTotal::Files);
    assert!(a.status_text().ends_with(&format!("1 elemento seleccionado · {}", size(10))), "{}", a.status_text());
    assert_eq!(a.sel_size.generation(), g, "nada que cortar ni arrancar");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cambiar lo elegido corta el cálculo anterior y lo que llegue de él se descarta.
#[test]
fn change_cancels_and_discards_stale() {
    let dir = fixture("cambio");
    let mut a = app_in(&dir);
    select(&mut a, &["a"]);
    let (g1, flag) = (a.sel_size.generation(), a.sel_size.cancel_flag());
    select(&mut a, &["b"]);
    assert!(flag.load(Ordering::Relaxed), "el cálculo anterior se cortó");
    assert_ne!(a.sel_size.generation(), g1);
    // Una respuesta de la generación vieja (aunque nombre a «b») no cuenta.
    a.sel_size.sender().send(sel_size::Reply { generation: g1, path: dir.join("b"), bytes: 999_999, complete: true }).unwrap();
    wait_status(&mut a, &format!("1 elemento seleccionado · {}", size(50)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// El tamaño de cada carpeta se recuerda (sumar o quitar un elemento no recalcula) hasta que el
/// vigilante de la carpeta avisa un cambio.
#[test]
fn cache_reused_and_invalidated_by_watcher() {
    let dir = fixture("cache");
    let mut a = app_in(&dir);
    select(&mut a, &["a", "b"]);
    wait_status(&mut a, &size(150));
    assert_eq!(a.sel_size.cached(&dir.join("a")), Some(100));
    std::fs::write(dir.join("a").join("z.bin"), vec![0u8; 1000]).unwrap();
    select(&mut a, &["a"]);
    assert!(a.status_text().ends_with(&size(100)), "de lo recordado, sin «calculando…»: {}", a.status_text());
    let id = a.panes[0].tab().id;
    a.on_app_message(WM_APP_CHANGED, id, 0);
    a.paint();
    assert!(a.status_text().ends_with("calculando\u{2026}"), "{}", a.status_text());
    wait_status(&mut a, &size(1100));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Si xtree ya analizó la carpeta, se usa su tamaño sin leer el disco.
#[test]
fn uses_xtree_size() {
    use crate::xtree::tree::{NO_PARENT, Node};
    let dir = fixture("xtree");
    let mut a = app_in(&dir);
    let mut x = crate::xtree::view::Xtree::idle(dir.clone());
    let node = |name: &str, parent: u32, size: u64, children: Vec<u32>| Node { name: name.into(), parent, size, is_dir: true, small: false, children };
    x.set_arena(vec![node("raiz", NO_PARENT, 7777, vec![1]), node("a", 0, 7777, vec![])]);
    a.xtree = Some(x);
    select(&mut a, &["a"]);
    assert_eq!(a.sel_size.total(), crate::tab::SelTotal::Done { bytes: 7777, partial: false });
    let _ = std::fs::remove_dir_all(&dir);
}

/// En una carpeta de red no se calcula solo: «calcular tamaño» como enlace; al tocarlo, calcula.
#[test]
fn remote_folder_waits_for_click() {
    let dir = fixture("red");
    FakeNetwork::set_remote(&dir);
    let mut a = app_in(&dir);
    select(&mut a, &["a", "f.txt"]);
    assert!(a.status_text().ends_with("2 elementos seleccionados · calcular tamaño"), "{}", a.status_text());
    let link = a.size_link.expect("se dibujó el enlace");
    let (x, y) = ((link.left + link.right) / 2.0, (link.top + link.bottom) / 2.0);
    assert_eq!(a.hit(x, y), Hit::SizeLink);
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    a.paint();
    assert!(a.status_text().ends_with("calculando\u{2026}"), "{}", a.status_text());
    wait_status(&mut a, &format!("2 elementos seleccionados · {}", size(110)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Una carpeta solo en la nube no se lee (no se baja nada): el total dice «al menos».
#[test]
fn cloud_only_folder_counts_as_partial() {
    let dir = fixture("nube");
    FakeFileSystem::set_cloud_only_everywhere(&dir.join("a"));
    let mut a = app_in(&dir);
    select(&mut a, &["a", "b"]);
    wait_status(&mut a, &format!("2 elementos seleccionados · al menos {}", size(50)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Si alguna carpeta no se pudo leer (sin permiso, o ya no está), el total lleva «al menos».
#[test]
fn unreadable_folder_says_at_least() {
    let dir = fixture("ilegible");
    let mut a = app_in(&dir);
    std::fs::remove_dir_all(dir.join("a")).unwrap();
    select(&mut a, &["a", "b"]);
    wait_status(&mut a, &format!("2 elementos seleccionados · al menos {}", size(50)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Una subcarpeta solo en la nube tampoco se lee: el total de su carpeta dice «al menos».
#[test]
fn nested_cloud_only_folder_counts_as_partial() {
    let dir = fixture("nube-anidada");
    FakeFileSystem::set_cloud_only_everywhere(&dir.join("b").join("sub"));
    let mut a = app_in(&dir);
    select(&mut a, &["b"]);
    wait_status(&mut a, &format!("1 elemento seleccionado · al menos {}", size(0)));
    let _ = std::fs::remove_dir_all(&dir);
}

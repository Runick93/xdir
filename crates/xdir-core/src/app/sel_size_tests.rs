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

/// El vigilante de la carpeta del panel avisa un cambio: se relee y se dibuja.
fn watcher_fires(a: &mut App) {
    let (id, nav) = (a.panes[0].tab().id, a.panes[0].tab().nav_id());
    a.on_app_message(WM_APP_CHANGED, id, nav as isize);
    let t0 = Instant::now();
    while a.panes[0].tab().loading || a.panes[0].tab().reload_pending() {
        assert!(t0.elapsed() < Duration::from_secs(10), "la carpeta no se releyó");
        if let Ok(ev) = a.rx.recv_timeout(Duration::from_millis(20)) {
            a.on_load_event(ev);
        }
    }
    a.paint();
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
    // Algunos discos guardan la fecha al segundo (el de Docker en la Mac): que la de «a» cambie.
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(dir.join("a").join("z.bin"), vec![0u8; 1000]).unwrap();
    select(&mut a, &["a"]);
    assert!(a.status_text().ends_with(&size(100)), "de lo recordado, sin «calculando…»: {}", a.status_text());
    // El vigilante de la carpeta avisa: se vuelve a leer y «a» trae otra fecha.
    watcher_fires(&mut a);
    assert!(a.status_text().ends_with("calculando\u{2026}"), "{}", a.status_text());
    wait_status(&mut a, &size(1100));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Aunque xtree ya haya analizado la carpeta, se suma lo que pesan los archivos (el tamaño lógico,
/// lo que pesan los archivos): xtree mide lo que ocupan en el disco y no se mezclan.
#[test]
fn logical_size_even_when_xtree_analyzed() {
    use crate::xtree::tree::{NO_PARENT, Node};
    let dir = fixture("xtree");
    let mut a = app_in(&dir);
    let mut x = crate::xtree::view::Xtree::idle(dir.clone());
    let node = |name: &str, parent: u32, size: u64, children: Vec<u32>| Node { name: name.into(), parent, size, is_dir: true, small: false, children };
    x.set_arena(vec![node("raiz", NO_PARENT, 7777, vec![1]), node("a", 0, 7777, vec![])]);
    a.xtree = Some(x);
    select(&mut a, &["a"]);
    wait_status(&mut a, &format!("1 elemento seleccionado · {}", size(100)));
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

/// Un cambio dentro de una subcarpeta (su vigilante avisa) también olvida el tamaño de las
/// carpetas que la contienen, y si están elegidas se vuelven a sumar.
#[test]
fn change_in_subfolder_drops_ancestor_cache() {
    let dir = fixture("ancestro");
    let mut a = app_in(&dir);
    select(&mut a, &["b"]);
    wait_status(&mut a, &size(50));
    std::fs::write(dir.join("b").join("sub").join("z.bin"), vec![0u8; 1000]).unwrap();
    a.sel_size.invalidate(&dir.join("b").join("sub"));
    assert_eq!(a.sel_size.cached(&dir.join("b")), None, "b quedó con el tamaño viejo");
    a.paint();
    wait_status(&mut a, &size(1050));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un cambio en un archivo suelto de la carpeta (una descarga, `.DS_Store`) no vuelve a sumar las
/// carpetas elegidas: su fecha no cambió.
#[test]
fn unrelated_change_keeps_folder_sizes() {
    let dir = fixture("suelto");
    let mut a = app_in(&dir);
    select(&mut a, &["a", "b"]);
    wait_status(&mut a, &size(150));
    let g = a.sel_size.generation();
    std::fs::write(dir.join("f.txt"), vec![0u8; 11]).unwrap();
    watcher_fires(&mut a);
    assert!(a.status_text().ends_with(&format!("2 elementos seleccionados · {}", size(150))), "{}", a.status_text());
    assert_eq!(a.sel_size.generation(), g, "no se cortó ni se volvió a arrancar");
    let _ = std::fs::remove_dir_all(&dir);
}

/// La suma no cruza a otro disco montado adentro ni entra en una carpeta de red montada adentro
/// (esa queda para «calcular tamaño»): el total dice «al menos».
#[test]
fn walk_stays_on_its_volume_and_off_network_mounts() {
    let dir = fixture("volumen");
    FakeFileSystem::set_volume_id(&dir.join("b"), 1);
    FakeFileSystem::set_volume_id(&dir.join("b").join("sub"), 2);
    std::fs::create_dir_all(dir.join("a").join("nas")).unwrap();
    std::fs::write(dir.join("a").join("nas").join("lejos.bin"), vec![0u8; 5000]).unwrap();
    FakeNetwork::set_remote(&dir.join("a").join("nas"));
    // Un disco montado adentro con el mismo `st_dev` (en la Mac, /System/Volumes/Data): se lo
    // reconoce como punto de montaje.
    std::fs::create_dir_all(dir.join("f2").join("datos")).unwrap();
    std::fs::write(dir.join("f2").join("datos").join("d.bin"), vec![0u8; 700]).unwrap();
    std::fs::write(dir.join("f2").join("propio.bin"), vec![0u8; 7]).unwrap();
    FakeFileSystem::set_mount_point(&dir.join("f2").join("datos"));
    let mut a = app_in(&dir);
    select(&mut a, &["b"]);
    wait_status(&mut a, &format!("1 elemento seleccionado · al menos {}", size(0)));
    select(&mut a, &["a"]);
    wait_status(&mut a, &format!("1 elemento seleccionado · al menos {}", size(100)));
    select(&mut a, &["f2"]);
    wait_status(&mut a, &format!("1 elemento seleccionado · al menos {}", size(7)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Una carpeta elegida que es un enlace no se sigue (podría llevar a un disco de red o a todo
/// el disco): cuenta 0 y el total dice «al menos».
#[cfg(unix)]
#[test]
fn symlinked_folder_is_not_followed() {
    let dir = fixture("enlace");
    std::os::unix::fs::symlink(dir.join("a"), dir.join("enlace")).unwrap();
    let mut a = app_in(&dir);
    select(&mut a, &["enlace", "b"]);
    wait_status(&mut a, &format!("2 elementos seleccionados · al menos {}", size(50)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Lo que quedó a medias no se recuerda (se puede volver a intentar); lo entero, sí.
#[test]
fn incomplete_results_not_cached() {
    let dir = fixture("a-medias");
    FakeFileSystem::set_cloud_only_everywhere(&dir.join("a"));
    let mut a = app_in(&dir);
    select(&mut a, &["a", "b"]);
    wait_status(&mut a, &format!("2 elementos seleccionados · al menos {}", size(50)));
    assert_eq!(a.sel_size.cached(&dir.join("a")), None);
    assert_eq!(a.sel_size.cached(&dir.join("b")), Some(50));
    let _ = std::fs::remove_dir_all(&dir);
}

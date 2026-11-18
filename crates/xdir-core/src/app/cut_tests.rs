//! Pruebas de lo cortado en la `App`: el ícono de lo cortado se dibuja atenuado (`CUT_ALPHA`)
//! mientras el portapapeles lo tenga cortado.

use super::*;
use crate::cut::CUT_ALPHA;
use crate::sys::testing::{FakeEnv, FakeHost, FakeShell};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

fn app_with_host(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakeEnv::hold_keys(&[]);
    FakeShell::set_clipboard(None);
    App::new(Init::blank(), host)
}

/// Carpeta temporal propia de la prueba con esos archivos.
fn fixture(name: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-cut").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::write(dir.join(f), f.as_bytes()).unwrap();
    }
    sys::file_system().true_case(&dir)
}

fn go(a: &mut App, dir: &Path) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.set_active(0);
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    while a.panes[0].tab().loading || a.panes[0].tab().reload_pending() {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
}

fn select(a: &mut App, name: &str) {
    let list = &mut a.panes[0].tab_mut().list;
    let pos = (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == name).expect(name);
    list.select_pos(pos);
}

/// Opacidad del ícono de la fila `name` en el último cuadro: el ícono que está a la izquierda del
/// nombre, a su altura.
fn icon_alpha(host: &FakeHost, name: &str) -> f32 {
    let texts = host.drawn_texts();
    let (_, _, r) = texts.iter().find(|(_, s, _)| s == name).unwrap_or_else(|| panic!("se dibujó {name}"));
    let icons = host.drawn_icons();
    let &(_, _, _, alpha) = icons
        .iter()
        .find(|&&(x, y, size, _)| y >= r.top && y + size <= r.bottom && x + size <= r.left && x + size > r.left - 20.0)
        .unwrap_or_else(|| panic!("se dibujó el ícono de {name}: {icons:?} {r:?}"));
    alpha
}

#[test]
fn cut_dims_icon_until_paste() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let src = fixture("pegar-src", &["a.txt", "b.txt"]);
    let dest = fixture("pegar-dest", &[]);
    go(&mut a, &src);
    select(&mut a, "a.txt");
    a.run_command(Command::Cut);
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), CUT_ALPHA);
    assert_eq!(icon_alpha(&host, "b.txt"), 1.0);

    // Pegar lo cortado vacía el portapapeles: ya no hay nada atenuado.
    go(&mut a, &dest);
    a.run_command(Command::Paste);
    go(&mut a, &src);
    a.paint();
    assert!(host.drawn_icons().iter().all(|&(.., alpha)| alpha == 1.0), "{:?}", host.drawn_icons());
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn copy_replaces_cut_dimming() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let dir = fixture("copiar", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    select(&mut a, "a.txt");
    a.run_command(Command::Cut);
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), CUT_ALPHA);
    select(&mut a, "b.txt");
    a.run_command(Command::Copy);
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), 1.0);
    assert_eq!(icon_alpha(&host, "b.txt"), 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn external_clipboard_change_updates_cut() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let dir = fixture("afuera", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    // Se cortó en otra aplicación (el Explorador): xdir se entera por el aviso del sistema.
    FakeShell::set_clipboard(Some((vec![dir.join("b.txt")], true)));
    a.on_clipboard_changed();
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), 1.0);
    assert_eq!(icon_alpha(&host, "b.txt"), CUT_ALPHA);
    // Otra aplicación copió otra cosa: ya no está cortado.
    FakeShell::set_clipboard(None);
    a.on_clipboard_changed();
    a.paint();
    assert_eq!(icon_alpha(&host, "b.txt"), 1.0);
    // Al volver a la ventana también se relee.
    FakeShell::set_clipboard(Some((vec![dir.join("a.txt")], true)));
    a.on_app_activated();
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), CUT_ALPHA);
    let _ = std::fs::remove_dir_all(&dir);
}

/// La columna de vista previa: el ícono grande de lo cortado (mientras llega la vista previa, o si
/// no tiene) también va atenuado.
#[test]
fn cut_dims_preview_icon() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let dir = fixture("vista-previa", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    a.preview.on = true;
    select(&mut a, "a.txt");
    a.run_command(Command::Cut);
    a.paint();
    let big: Vec<f32> = host.drawn_icons().iter().filter(|&&(_, _, size, _)| size == 96.0).map(|&(.., alpha)| alpha).collect();
    assert_eq!(big, [CUT_ALPHA]);
    select(&mut a, "b.txt");
    a.paint();
    let big: Vec<f32> = host.drawn_icons().iter().filter(|&&(_, _, size, _)| size == 96.0).map(|&(.., alpha)| alpha).collect();
    assert_eq!(big, [1.0]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Copiar texto («Copiar como ruta», una suma...) reemplaza lo cortado: deja de verse atenuado
/// aunque no llegue ningún aviso del sistema (macOS con xdir al frente).
#[test]
fn copy_text_clears_cut_dimming() {
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    let dir = fixture("copiar-ruta", &["a.txt", "b.txt"]);
    go(&mut a, &dir);
    select(&mut a, "a.txt");
    a.run_command(Command::Cut);
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), CUT_ALPHA);
    a.run_command(Command::CopyPaths);
    a.paint();
    assert_eq!(icon_alpha(&host, "a.txt"), 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Con muchos elegidos la vista previa muestra el abanico de los 3 primeros: se atenúa según
/// esos (sin recorrer toda la selección en cada cuadro).
#[test]
fn preview_cut_follows_the_fan() {
    let mut a = app_with_host(FakeHost::new());
    let names = ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"];
    let dir = fixture("abanico", &names);
    go(&mut a, &dir);
    {
        let list = &mut a.panes[0].tab_mut().list;
        for pos in 0..list.count() {
            list.set_selected_pos(pos, true);
        }
    }
    let selected: Vec<PathBuf> = {
        let tab = a.panes[0].tab();
        tab.list.selected_indices().map(|i| tab.entry_path(i)).collect()
    };
    assert_eq!(selected.len(), 5);
    // Solo los 3 del abanico cortados (los otros 2 no): se atenúa.
    FakeShell::set_clipboard(Some((selected[..3].to_vec(), true)));
    a.on_clipboard_changed();
    assert!(a.preview_is_cut(), "los del abanico están cortados");
    // Uno del abanico sin cortar: no se atenúa.
    FakeShell::set_clipboard(Some((selected[1..].to_vec(), true)));
    a.on_clipboard_changed();
    assert!(!a.preview_is_cut());
    let _ = std::fs::remove_dir_all(&dir);
}

//! Pruebas de la columna de vista previa en la `App`.

use super::*;
use crate::preview::{PreviewBody, PreviewInfo, PreviewReply, PreviewTarget, Shown, WM_APP_PREVIEW};
use crate::sys::testing::{FakeDiskScan, FakeFileSystem, FakeHost, FakeMetadata, FakePreview};
use std::path::Path;
use std::time::Duration;

fn app() -> App {
    crate::sys::testing::install_fakes();
    FakePreview::set_delay(None);
    FakePreview::take_requests();
    App::new(Init::blank(), FakeHost::new())
}

fn app_with_host(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakePreview::set_delay(None);
    FakePreview::take_requests();
    App::new(Init::blank(), host)
}

/// Carpeta temporal propia de la prueba con esos archivos (nombre y contenido).
fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-preview").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, text) in files {
        std::fs::write(dir.join(f), text).unwrap();
    }
    dir
}

fn pump(a: &mut App, pane: usize) {
    while a.panes[pane].tab().loading {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
}

fn go(a: &mut App, pane: usize, dir: &Path) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[pane].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    pump(a, pane);
}

/// Posición en la lista de un archivo por nombre.
fn pos_of(a: &App, pane: usize, name: &str) -> usize {
    let list = &a.panes[pane].tab().list;
    (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == name).expect(name)
}

fn select(a: &mut App, pane: usize, name: &str) {
    let p = pos_of(a, pane, name);
    a.panes[pane].tab_mut().list.select_pos(p);
}

/// Dos paneles lado a lado con la columna prendida.
fn two_panes() -> App {
    let mut a = app();
    a.set_layout(Layout::TwoSide);
    a.preview.on = true;
    a
}

/// Lo que llegó del servicio (lo que hace `WM_APP_PREVIEW` en la ventana).
fn deliver(a: &mut App) {
    a.on_app_message(WM_APP_PREVIEW, 0, 0);
}

fn rows_of(s: &Shown) -> Vec<(&'static str, String)> {
    match s {
        Shown::Item { rows, .. } => rows.clone(),
        other => panic!("no es un elemento: {other:?}"),
    }
}

#[test]
fn preview_follows_active_pane() {
    let d1 = fixture("sigue-1", &[("uno.txt", "1"), ("dos.txt", "2")]);
    let d2 = fixture("sigue-2", &[("tres.txt", "3")]);
    let mut a = two_panes();
    go(&mut a, 0, &d1);
    go(&mut a, 1, &d2);
    a.set_active(0);
    select(&mut a, 0, "dos.txt");
    a.paint();
    a.flush_preview();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::File(d1.join("dos.txt"))]);
    a.set_active(1);
    select(&mut a, 1, "tres.txt");
    a.paint();
    a.flush_preview();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::File(d2.join("tres.txt"))]);
    // Sin cambios (el foco en el árbol no cambia el panel activo) no se vuelve a pedir.
    a.paint();
    a.flush_preview();
    assert!(FakePreview::take_requests().is_empty());
}

#[test]
fn preview_stale_reply_ignored() {
    let d = fixture("vieja", &[("a.txt", "AAA"), ("b.txt", "BBB")]);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakePreview::serve(&d.join("a.txt"), PreviewInfo::new(PreviewBody::Text { lines: vec!["A".into()] }));
    FakePreview::serve(&d.join("b.txt"), PreviewInfo::new(PreviewBody::Text { lines: vec!["B".into()] }));
    FakePreview::set_delay(Some(Duration::from_millis(300)));
    select(&mut a, 0, "a.txt");
    a.paint();
    a.flush_preview();
    let old = a.preview.generation;
    FakePreview::set_delay(Some(Duration::from_millis(20)));
    select(&mut a, 0, "b.txt");
    a.paint();
    a.flush_preview();
    // Una respuesta vieja que igual llega (un servicio que no mira la generación) no pisa.
    a.preview.tx.send(PreviewReply { generation: old, info: PreviewInfo::new(PreviewBody::Text { lines: vec!["VIEJA".into()] }) }).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    deliver(&mut a);
    a.paint();
    let host_texts: Vec<String> = match a.preview_shown() {
        Shown::Item { info: Some(PreviewInfo { body: PreviewBody::Text { lines }, .. }), .. } => lines,
        other => panic!("{other:?}"),
    };
    assert_eq!(host_texts, ["B"]);
    FakePreview::set_delay(None);
}

/// Con una vista previa que tarda 5 s, 20 flechas vuelven enseguida y la lista avanza 20.
#[test]
fn arrows_never_wait_for_preview() {
    let files: Vec<(String, String)> = (0..30).map(|i| (format!("f{i:02}.txt"), format!("{i}"))).collect();
    let files: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let d = fixture("flechas", &files);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "f00.txt");
    FakePreview::set_delay(Some(Duration::from_secs(5)));
    let t = Instant::now();
    for _ in 0..20 {
        a.on_key(VK_DOWN);
        a.paint();
        // Lo peor: que cada tecla llegue a pedir.
        a.flush_preview();
    }
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(a.panes[0].tab().list.focus_pos(), Some(20));
    FakePreview::set_delay(None);
}

/// Sin dejar pasar la pausa, las flechas no piden nada; después, un solo pedido (el último).
#[test]
fn preview_debounced_to_last_subject() {
    let d = fixture("pausa", &[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    a.paint();
    for _ in 0..2 {
        a.on_key(VK_DOWN);
        a.paint();
    }
    assert!(FakePreview::take_requests().is_empty());
    std::thread::sleep(Duration::from_millis(80));
    a.on_app_message(preview::WM_APP_PREVIEW_DUE, 0, 0);
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::File(d.join("c.txt"))]);
    deliver(&mut a);
    assert!(matches!(a.preview_shown(), Shown::Item { info: Some(_), .. }));
}

#[test]
fn preview_many_selected() {
    let d = fixture("varios", &[("a.jpg", "aaaa"), ("b.jpg", "bb"), ("c.jpg", "c")]);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    a.panes[0].tab_mut().list.select_all();
    a.paint();
    a.flush_preview();
    assert!(FakePreview::take_requests().is_empty(), "con varios no se pide nada");
    match a.preview_shown() {
        Shown::Many { count, keys, bytes, .. } => {
            assert_eq!((count, keys.len(), bytes), (3, 3, 7));
        }
        other => panic!("{other:?}"),
    }
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    a.set_layout(Layout::TwoSide);
    a.preview.on = true;
    go(&mut a, 0, &d);
    a.set_active(0);
    a.panes[0].tab_mut().list.select_all();
    a.paint();
    assert!(host.drawn_texts().iter().any(|t| t.1 == "3 elementos seleccionados"));
}

/// Nada seleccionado: los detalles de la carpeta abierta y «N elementos · tamaño» con el tamaño
/// del escaneo de xtree.
#[test]
fn preview_nothing_selected_shows_folder() {
    use crate::xtree::tree::{NO_PARENT, Node};
    let d = fixture("carpeta", &[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    a.set_layout(Layout::TwoSide);
    a.preview.on = true;
    go(&mut a, 0, &d);
    a.set_active(0);
    let parent = d.parent().unwrap().to_path_buf();
    let leaf: Box<str> = d.file_name().unwrap().to_string_lossy().into();
    let mut x = Xtree::idle(parent);
    x.set_arena(vec![
        Node { name: "raíz".into(), parent: NO_PARENT, size: 5_000_000, is_dir: true, small: false, children: vec![1] },
        Node { name: leaf, parent: 0, size: 1_330_000_000, is_dir: true, small: false, children: vec![] },
    ]);
    a.xtree = Some(x);
    a.paint();
    a.flush_preview();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::Folder(d.clone())]);
    deliver(&mut a);
    let shown = a.preview_shown();
    let rows = rows_of(&shown);
    let size = crate::format::size_short(1_330_000_000);
    assert!(rows.contains(&("Contenido", "3 elementos".to_string())), "{rows:?}");
    assert!(rows.contains(&("Tamaño", size.clone())), "{rows:?}");
    assert!(rows.iter().any(|r| r.0 == "Modificado"), "{rows:?}");
    a.paint();
    assert!(host.drawn_texts().iter().any(|t| t.1 == format!("3 elementos · {size}")), "{:?}", host.drawn_texts());
}

/// En «Este equipo», el disco enfocado: espacio libre y capacidad.
#[test]
fn preview_computer_shows_drive() {
    let mut a = two_panes();
    a.set_active(0);
    pump(&mut a, 0);
    // Un disco «de verdad» para la lectura: una carpeta temporal.
    let root = fixture("disco", &[]);
    FakeFileSystem::set_drives(&[(&root, "Disco")]);
    FakeDiskScan::set_volume(Some((500_000_000_000, 120_000_000_000)), true);
    let disk = crate::fs::Entry {
        name: "Disco".into(),
        full: Some(root.clone().into_os_string()),
        group: 1,
        pidl: None,
        display: "Disco".into(),
        sort_key: "disco".into(),
        is_dir: true,
        size: 0,
        modified: 0,
        created: 0,
        attrs: 0,
        type_name: std::sync::Arc::from("Disco local"),
        icon: 0,
        overlay: 0,
        overlay_checked: true,
    };
    a.panes[0].tab_mut().list.append(vec![disk]);
    a.panes[0].tab_mut().list.select_pos(0);
    a.paint();
    a.flush_preview();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::Drive(root.clone())]);
    deliver(&mut a);
    let rows = rows_of(&a.preview_shown());
    assert!(rows.contains(&("Espacio libre", crate::format::size_short(120_000_000_000))), "{rows:?}");
    assert!(rows.contains(&("Capacidad", crate::format::size_short(500_000_000_000))), "{rows:?}");
    FakeFileSystem::set_drives(&[]);
    FakeDiskScan::set_volume(None, false);
}

#[test]
fn preview_unreadable_note() {
    let d = fixture("ilegible", &[("bloqueado.txt", "x")]);
    let host = FakeHost::new();
    let mut a = app_with_host(host.clone());
    a.set_layout(Layout::TwoSide);
    a.preview.on = true;
    go(&mut a, 0, &d);
    a.set_active(0);
    FakePreview::serve(&d.join("bloqueado.txt"), PreviewInfo::new(PreviewBody::Unreadable));
    select(&mut a, 0, "bloqueado.txt");
    a.paint();
    a.flush_preview();
    deliver(&mut a);
    a.paint();
    assert!(host.drawn_texts().iter().any(|t| t.1 == "No se puede leer este archivo"), "{:?}", host.drawn_texts());
}

/// Sin columnas de metadatos a la vista, el elemento de la columna igual pide los suyos.
#[test]
fn preview_requests_meta_for_subject() {
    let d = fixture("metadatos", &[("foto.jpg", "x"), ("otra.jpg", "y")]);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    a.paint();
    FakeMetadata::take_requests();
    select(&mut a, 0, "foto.jpg");
    a.paint();
    let reqs = FakeMetadata::take_requests();
    let paths: Vec<PathBuf> = reqs.iter().flat_map(|r| r.0.iter().map(|(_, p)| p.clone())).collect();
    assert_eq!(paths, [d.join("foto.jpg")]);
}

/// Con xtree abierto la columna no se ve, no ocupa lugar ni pide nada.
#[test]
fn preview_hidden_with_xtree() {
    let d = fixture("xtree", &[("a.txt", "a")]);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    assert!(a.preview_width() > 0.0);
    a.xtree = Some(Xtree::idle(d.clone()));
    a.xtree_open = true;
    select(&mut a, 0, "a.txt");
    a.paint();
    a.flush_preview();
    assert!(FakePreview::take_requests().is_empty());
    assert_eq!(a.preview_width(), 0.0);
    assert!(a.preview_column().is_none());
    a.xtree_open = false;
    a.paint();
    a.flush_preview();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::File(d.join("a.txt"))]);
}

/// Arrastrar el borde izquierdo de la columna cambia su ancho (y se guarda en la sesión).
#[test]
fn preview_edge_drag_resizes() {
    let host = FakeHost::new();
    host.client.set((1600.0, 800.0));
    let mut a = app_with_host(host);
    a.set_layout(Layout::TwoSide);
    a.preview.on = true;
    let col = a.preview_column().unwrap();
    assert_eq!(col.right - col.left, ui::PREVIEW_W);
    let (x, y) = (col.left - 2.0, (col.top + col.bottom) / 2.0);
    assert_eq!(a.hit(x, y), Hit::PreviewEdge);
    assert_eq!(a.cursor_at(x, y), CursorKind::SizeWE);
    a.on_left_down(x, y, false);
    a.on_mouse_move(x - 60.0, y);
    a.on_left_up(x - 60.0, y);
    let col = a.preview_column().unwrap();
    assert!((col.right - col.left - (ui::PREVIEW_W + 60.0)).abs() < 0.5, "{col:?}");
    let s = a.session();
    assert!(s.preview_on);
    assert!((s.preview_width - col.right + col.left).abs() < 0.5);
    // Más allá del máximo, se queda en 640.
    a.on_left_down(col.left - 2.0, y, false);
    a.on_mouse_move(10.0, y);
    a.on_left_up(10.0, y);
    assert_eq!(a.preview.width, ui::PREVIEW_MAX_W);
}

/// El botón de la barra de estado prende y apaga la columna; su cartel dice el atajo.
#[test]
fn preview_toggle_button_and_tooltip() {
    let mut a = app();
    let (w, h) = a.client_dips();
    let r = ui::preview_rect(w, h);
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert_eq!(a.hit(x, y), Hit::PreviewToggle);
    assert!(!a.preview.on);
    a.on_left_down(x, y, false);
    assert!(a.preview.on);
    assert_eq!(a.tooltip_text(Hit::PreviewToggle).as_deref(), Some("Vista previa (Alt+P)"));
    a.run_command(Command::TogglePreview);
    assert!(!a.preview.on);
}

/// El vigilante avisa que cambió el archivo que se muestra: se olvida su imagen grande y se
/// vuelve a pedir, sin pasar por «cargando».
#[test]
fn preview_refreshes_changed_file() {
    let d = fixture("cambia", &[("foto.png", "x")]);
    let mut a = two_panes();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "foto.png");
    a.paint();
    a.flush_preview();
    deliver(&mut a);
    FakePreview::take_requests();
    let idx = a.panes[0].tab().list.focused_index().unwrap();
    a.panes[0].tab_mut().list.entries[idx].modified += 10_000_000;
    a.sync_preview();
    assert_eq!(a.preview.forget, [crate::thumbs::Source::Path(d.join("foto.png"))]);
    assert!(a.preview.info.is_some(), "sigue lo de antes hasta que llega lo nuevo");
    a.flush_preview();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::File(d.join("foto.png"))]);
    a.paint();
    assert!(a.preview.forget.is_empty());
}

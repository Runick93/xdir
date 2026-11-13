//! Pruebas del panel de información en la `App`.

use super::*;
use crate::info::draw::InfoHit;
use crate::info::{Flag, InfoChange, InfoReply, ItemInfo, OpenWith, AppRef, Perms, Section, Tag, VolumeData, WM_APP_INFO, WM_APP_INFO_EDIT_DONE};
use crate::keys::keys::VK_RETURN;
use crate::sys::testing::{FakeDiskScan, FakeFileOps, FakeFileSystem, FakeHost, FakeInfo, FakePreview};
use std::path::Path;
use std::time::Duration;

fn app_with_host(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakePreview::set_delay(None);
    FakePreview::take_requests();
    FakeInfo::take_requests();
    FakeInfo::take_applied();
    FakeInfo::fail_with(None);
    FakeInfo::set_more_in_system(true);
    let mut a = App::new(Init::blank(), host);
    a.set_layout(Layout::TwoSide);
    a.info.size_thread = false;
    a
}

fn app() -> (App, Rc<FakeHost>) {
    let host = FakeHost::new();
    (app_with_host(host.clone()), host)
}

fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-info-app").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, text) in files {
        if let Some(d) = f.strip_suffix('/') {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        } else {
            std::fs::write(dir.join(f), text).unwrap();
        }
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

fn select(a: &mut App, pane: usize, name: &str) {
    let list = &a.panes[pane].tab().list;
    let p = (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == name).expect(name);
    a.panes[pane].tab_mut().list.select_pos(p);
}

/// Abre el panel sobre lo seleccionado y entrega la respuesta.
fn open_on(a: &mut App) {
    a.run_command(Command::Properties);
    a.paint();
    a.flush_info();
    deliver(a);
    a.paint();
}

fn deliver(a: &mut App) {
    a.on_app_message(WM_APP_INFO, 0, 0);
}

fn texts(host: &FakeHost) -> Vec<String> {
    host.drawn_texts().into_iter().map(|t| t.1).collect()
}

/// Clic en el centro de lo que el panel dibujó para `h`.
fn click(a: &mut App, h: InfoHit) {
    let r = a.info.layout.hits.iter().find(|(x, _)| *x == h).unwrap_or_else(|| panic!("{h:?} no está: {:?}", a.info.layout.hits)).1;
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
    a.on_left_up((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
}

fn full_item(path: &Path) -> ItemInfo {
    let app = AppRef { id: "fotos".into(), name: "Fotos".into(), icon: 3 };
    ItemInfo {
        path: path.to_path_buf(),
        kind_name: "Imagen JPEG".into(),
        size: 3_963_904,
        on_disk: 3_964_928,
        created: 134_000_000_000_000_000,
        modified: 134_000_000_000_000_000,
        last_opened: 134_000_000_000_000_000,
        tags: Some(vec![Tag { name: "Rojo".into(), color: Some(0xFF453A) }, Tag { name: "Viaje".into(), color: None }]),
        comment: Some("Un comentario".into()),
        hidden_ext: Some(false),
        flags: vec![(Flag::Locked, false, true), (Flag::Hidden, false, true)],
        open_with: Some(OpenWith { current: Some(app.clone()), apps: vec![app], per_file: true, system_dialog: false }),
        perms: Some(Perms { owner: "javier".into(), group: "staff".into(), mode: 0o644, editable: true, can_read: true, can_write: true }),
        more: vec![("Versión", "1.2".into())],
        ..ItemInfo::default()
    }
}

/// Alt+Enter abre el panel en el lugar de la columna; otra vez lo cierra. Con la vista previa
/// prendida, al cerrarlo vuelve la vista previa.
#[test]
fn properties_toggles_info_panel() {
    let d = fixture("abre", &[("a.txt", "x")]);
    let (mut a, _) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    assert!(!a.preview_visible());
    assert!(a.on_alt_key(VK_RETURN));
    assert!(a.info_open());
    assert!(a.preview_visible(), "el panel ocupa el lugar de la columna");
    assert!(a.on_alt_key(VK_RETURN));
    assert!(!a.info_open());
    assert!(!a.preview_visible());
    a.preview.on = true;
    a.run_command(Command::Properties);
    assert!(a.info_open() && a.preview_visible());
    a.paint();
    assert!(FakePreview::take_requests().is_empty(), "con el panel no se pide vista previa");
    a.run_command(Command::Properties);
    assert!(!a.info_open() && a.preview_visible() && a.preview.on);
    // El botón ✕ también lo cierra.
    a.run_command(Command::Properties);
    a.paint();
    click(&mut a, InfoHit::Close);
    assert!(!a.info_open());
    // Y Esc con el foco en el panel.
    a.run_command(Command::Properties);
    a.paint();
    click(&mut a, InfoHit::Toggle(Section::General));
    a.on_key(crate::keys::keys::VK_ESCAPE);
    assert!(!a.info_open());
}

#[test]
fn info_follows_active_pane() {
    let d1 = fixture("sigue-1", &[("uno.txt", "1"), ("dos.txt", "2")]);
    let d2 = fixture("sigue-2", &[("tres.txt", "3")]);
    let (mut a, _) = app();
    go(&mut a, 0, &d1);
    go(&mut a, 1, &d2);
    a.set_active(0);
    select(&mut a, 0, "dos.txt");
    a.run_command(Command::Properties);
    a.paint();
    a.flush_info();
    assert_eq!(FakeInfo::take_requests(), [vec![d1.join("dos.txt")]]);
    a.set_active(1);
    select(&mut a, 1, "tres.txt");
    a.paint();
    a.flush_info();
    assert_eq!(FakeInfo::take_requests(), [vec![d2.join("tres.txt")]]);
    a.paint();
    a.flush_info();
    assert!(FakeInfo::take_requests().is_empty());
    // Sin selección: la carpeta.
    let pos = a.panes[1].tab().list.selected_positions()[0];
    a.panes[1].tab_mut().list.set_selected_pos(pos, false);
    a.paint();
    a.flush_info();
    assert_eq!(FakeInfo::take_requests(), [vec![d2.clone()]]);
}

#[test]
fn info_stale_reply_ignored() {
    let d = fixture("vieja", &[("a.txt", "AAA"), ("b.txt", "BBB")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    open_on(&mut a);
    let old = a.info.generation;
    select(&mut a, 0, "b.txt");
    a.paint();
    a.flush_info();
    let mut stale = ItemInfo { path: d.join("a.txt"), kind_name: "VIEJA".into(), ..ItemInfo::default() };
    stale.size = 1;
    a.info.tx.send(InfoReply::Items { generation: old, items: vec![stale] }).unwrap();
    deliver(&mut a);
    a.paint();
    let t = texts(&host);
    assert!(!t.iter().any(|s| s.contains("VIEJA")), "{t:?}");
    assert!(t.iter().any(|s| s == "b.txt"), "{t:?}");
}

#[test]
fn info_draws_all_sections() {
    let d = fixture("todo", &[("IMG_3101.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakeInfo::serve(&d.join("IMG_3101.jpg"), full_item(&d.join("IMG_3101.jpg")));
    select(&mut a, 0, "IMG_3101.jpg");
    // Todas abiertas para ver su contenido.
    a.info.sections = 0x7F;
    host.client.set((1400.0, 2400.0));
    open_on(&mut a);
    let t = texts(&host);
    for s in ["Etiquetas", "General", "Más información", "Nombre y extensión", "Comentarios", "Abrir con", "Compartir y permisos", "Más en el sistema…"] {
        assert!(t.iter().any(|x| x == s), "falta {s}: {t:?}");
    }
    // El título es «Información»; el nombre va en el encabezado.
    assert!(t.iter().any(|x| x == "Información"), "{t:?}");
    assert!(!t.iter().any(|x| x.starts_with("Información de")), "{t:?}");
    assert!(t.iter().any(|x| x == "IMG_3101.jpg"), "{t:?}");
    for s in ["Rojo", "Viaje", "Bloqueado", "Oculto", "Un comentario", "Ocultar la extensión", "Fotos (predeterminada)", "Cambiar todos…", "javier", "staff", "Todos", "Versión"] {
        assert!(t.iter().any(|x| x == s), "falta {s}: {t:?}");
    }
    let bytes = format!("{} ({} bytes)", crate::format::size_short(3_963_904), crate::format::thousands(3_963_904));
    assert!(t.contains(&bytes), "{t:?}");
}

#[test]
fn info_tags_unsupported_text() {
    let d = fixture("sin-etiquetas", &[("a.txt", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakeInfo::serve(&d.join("a.txt"), ItemInfo { path: d.join("a.txt"), tags: None, ..ItemInfo::default() });
    select(&mut a, 0, "a.txt");
    open_on(&mut a);
    assert!(texts(&host).iter().any(|x| x == "Este tipo de archivo no admite etiquetas"), "{:?}", texts(&host));
}

#[test]
fn info_folder_size_calculating() {
    let d = fixture("tamano", &[("Fotos 2026/", ""), ("Fotos 2026/a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "Fotos 2026");
    open_on(&mut a);
    let folder = d.join("Fotos 2026");
    let generation = a.info.size_generation;
    let send = |a: &mut App, bytes: u64, files: u64, done: bool| {
        a.info.tx.send(InfoReply::FolderSize { generation, path: folder.clone(), bytes, files, folders: 0, skipped: 0, done }).unwrap();
        deliver(a);
        a.paint();
    };
    send(&mut a, 3_000_000, 1200, false);
    let t = texts(&host);
    assert!(t.iter().any(|s| s.starts_with("Calculando…")), "{t:?}");
    assert!(t.iter().any(|s| s.contains("1.200 elementos")), "{t:?}");
    send(&mut a, 3_500_000, 1250, true);
    let t = texts(&host);
    assert!(!t.iter().any(|s| s.starts_with("Calculando…")), "{t:?}");
    assert!(t.iter().any(|s| s == "1.250 elementos"), "{t:?}");
    assert!(t.iter().any(|s| s.contains(&crate::format::thousands(3_500_000))), "{t:?}");

    // Con xtree, al instante (sin esperar la suma).
    use crate::xtree::tree::{NO_PARENT, Node};
    let mut x = Xtree::idle(d.clone());
    x.set_arena(vec![
        Node { name: "raíz".into(), parent: NO_PARENT, size: 5_000_000, is_dir: true, small: false, children: vec![1] },
        Node { name: "Fotos 2026".into(), parent: 0, size: 1_330_000_000, is_dir: true, small: false, children: vec![] },
    ]);
    a.xtree = Some(x);
    a.run_command(Command::Properties);
    a.run_command(Command::Properties);
    a.paint();
    a.flush_info();
    deliver(&mut a);
    a.paint();
    let t = texts(&host);
    assert!(!t.iter().any(|s| s.starts_with("Calculando…")), "{t:?}");
    assert!(t.iter().any(|s| s.starts_with(&crate::format::size_short(1_330_000_000))), "{t:?}");
}

/// «Este equipo» con un disco «PENDRIVE» (carpeta temporal `tag`) seleccionado y el panel abierto.
fn drive_info(tag: &str, ejectable: bool) -> (App, Rc<FakeHost>, PathBuf) {
    let (mut a, host) = app();
    a.set_active(0);
    pump(&mut a, 0);
    let root = fixture(tag, &[]);
    FakeFileSystem::set_drives(&[(&root, "PENDRIVE")]);
    if ejectable {
        a.ejectable.insert(root.clone());
    }
    FakeInfo::serve(&root, ItemInfo {
        path: root.clone(),
        kind_name: "Disco extraíble (USB)".into(),
        is_dir: true,
        volume: Some(VolumeData { total: 58_000_000_000, free: 21_400_000_000, format: "exFAT".into() }),
        ..ItemInfo::default()
    });
    let disk = crate::fs::Entry {
        name: "PENDRIVE".into(),
        full: Some(root.clone().into_os_string()),
        group: 1,
        pidl: None,
        display: "PENDRIVE".into(),
        sort_key: "pendrive".into(),
        is_dir: true,
        size: 0,
        modified: 0,
        created: 0,
        attrs: 0,
        type_name: std::sync::Arc::from("Disco extraíble"),
        icon: 0,
        overlay: 0,
        overlay_checked: true,
        packed: 0,
    };
    a.panes[0].tab_mut().list.append(vec![disk]);
    a.panes[0].tab_mut().list.select_pos(0);
    open_on(&mut a);
    (a, host, root)
}

#[test]
fn info_drive_shows_capacity() {
    let (a, host, _root) = drive_info("disco", false);
    let t = texts(&host);
    assert!(!a.info.layout.hits.iter().any(|(h, _)| *h == InfoHit::Eject), "C: no se expulsa");
    assert!(!t.iter().any(|x| x == crate::info::draw::EJECT_TEXT), "{t:?}");
    let free = crate::format::size_short(21_400_000_000);
    let total = crate::format::size_short(58_000_000_000);
    assert!(t.contains(&format!("{free} disponibles de {total}")), "{t:?}");
    for s in ["Capacidad", "Disponible", "Usado", "Formato", "exFAT"] {
        assert!(t.iter().any(|x| x == s), "falta {s}: {t:?}");
    }
    assert!(!t.iter().any(|x| x == "Etiquetas" || x == "Abrir con"), "{t:?}");
    FakeFileSystem::set_drives(&[]);
    FakeDiskScan::set_volume(None, false);
}

/// Un disco expulsable muestra «⏏ Expulsar» en el panel; el clic lo expulsa como el ⏏ de la fila.
#[test]
fn info_drive_eject_button() {
    let (mut a, host, root) = drive_info("disco-usb", true);
    assert!(texts(&host).iter().any(|x| x == crate::info::draw::EJECT_TEXT), "{:?}", texts(&host));
    let before = crate::sys::testing::FakeShell::ejects().iter().filter(|p| **p == root).count();
    click(&mut a, InfoHit::Eject);
    // Ajustes 2: mientras expulsa, el botón gira, dice «Expulsando…» y no se puede tocar.
    a.paint();
    let t = texts(&host);
    assert!(t.iter().any(|x| x == crate::info::draw::EJECTING_TEXT) && !t.iter().any(|x| x == crate::info::draw::EJECT_TEXT), "{t:?}");
    assert!(!a.info.layout.hits.iter().any(|(h, _)| *h == InfoHit::Eject));
    // Puede llegar antes otro trabajo (qué discos se expulsan): hasta el aviso de expulsar.
    while a.notice.is_none() {
        let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("expulsar termina");
        a.on_task_done(t);
    }
    assert_eq!(crate::sys::testing::FakeShell::ejects().iter().filter(|p| **p == root).count() - before, 1);
    assert_eq!(a.notice.as_deref(), Some("Se expulsó \"PENDRIVE\": ya lo podés quitar"));
    FakeFileSystem::set_drives(&[]);
    FakeDiskScan::set_volume(None, false);
}

/// Solo en la nube: la lectura no abre nada, no se pide la imagen y la fila lo dice.
#[test]
fn info_cloud_only_not_read() {
    let d = fixture("nube", &[("foto.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakeFileSystem::set_cloud_only(&d.join("foto.jpg"), true);
    select(&mut a, 0, "foto.jpg");
    open_on(&mut a);
    let t = texts(&host);
    assert!(t.iter().any(|x| x == crate::info::draw::CLOUD_TEXT), "{t:?}");
    assert!(host.drawn_big().is_empty(), "no se pide la imagen de lo que está en la nube");
    assert!(FakePreview::take_requests().is_empty());
    FakeFileSystem::set_cloud_only(&d.join("foto.jpg"), false);
}

#[test]
fn info_sections_persist() {
    let d = fixture("secciones", &[("a.txt", "x")]);
    let (mut a, _) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    a.run_command(Command::Properties);
    a.paint();
    let before = a.session().info_sections;
    assert_eq!(before, crate::info::DEFAULT_OPEN);
    click(&mut a, InfoHit::Toggle(Section::General));
    let after = a.session().info_sections;
    assert_eq!(after, before & !Section::General.bit());
    let mut init = Init::blank();
    init.session.info_sections = after;
    let b = App::new(init, FakeHost::new());
    assert_eq!(b.info.sections, after);
}

#[test]
fn info_rename_from_panel() {
    let d = fixture("renombrar", &[("viejo.txt", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "viejo.txt");
    a.info.sections |= Section::Name.bit();
    open_on(&mut a);
    FakeFileOps::take_ops();
    click(&mut a, InfoHit::NameField);
    let (b, _) = a.info.edit.expect("se abre la caja");
    assert_eq!(host.text_get(b), "viejo.txt");
    host.text_set(b, "nuevo.txt");
    a.on_app_message(WM_APP_INFO_EDIT_DONE, 1, 0);
    assert!(a.info.edit.is_none());
    let ops = FakeFileOps::take_ops();
    assert_eq!(ops, [Op::Rename { path: d.join("viejo.txt"), new_name: "nuevo.txt".into() }]);
}

#[test]
fn info_apply_error_in_section() {
    let d = fixture("error", &[("a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakeInfo::serve(&d.join("a.jpg"), full_item(&d.join("a.jpg")));
    select(&mut a, 0, "a.jpg");
    open_on(&mut a);
    FakeInfo::fail_with(Some("No tenés permiso"));
    click(&mut a, InfoHit::Flag(Flag::Hidden));
    deliver(&mut a);
    a.paint();
    assert_eq!(FakeInfo::take_applied(), [InfoChange::Flag { paths: vec![d.join("a.jpg")], flag: Flag::Hidden, on: true }]);
    assert!(texts(&host).iter().any(|x| x == "No tenés permiso"), "{:?}", texts(&host));
    // Un cambio que anda vuelve a leer la información.
    FakeInfo::fail_with(None);
    FakeInfo::take_requests();
    click(&mut a, InfoHit::Flag(Flag::Locked));
    deliver(&mut a);
    a.paint();
    assert_eq!(FakeInfo::take_requests(), [vec![d.join("a.jpg")]]);
    assert!(!texts(&host).iter().any(|x| x == "No tenés permiso"));
}

#[test]
fn info_more_in_system_hidden_when_unsupported() {
    let d = fixture("mas", &[("a.txt", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    FakeInfo::set_more_in_system(false);
    open_on(&mut a);
    assert!(!texts(&host).iter().any(|x| x == "Más en el sistema…"));
    FakeInfo::set_more_in_system(true);
    a.paint();
    assert!(texts(&host).iter().any(|x| x == "Más en el sistema…"));
    click(&mut a, InfoHit::MoreInSystem);
    assert_eq!(FakeInfo::take_more_calls(), [vec![d.join("a.txt")]]);
}

/// Una carpeta con «Cambiar todos…» de cada sistema: macOS (por archivo) confirma la actual;
/// Linux elige la app en un menú y confirma; Windows abre su diálogo sin preguntar.
#[test]
fn info_change_all_per_platform() {
    let d = fixture("cambiar", &[("a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    let mut item = full_item(&d.join("a.jpg"));
    let other = AppRef { id: "gimp.desktop".into(), name: "GIMP".into(), icon: 0 };
    if let Some(ow) = item.open_with.as_mut() {
        ow.per_file = false;
        ow.apps.push(other.clone());
    }
    FakeInfo::serve(&d.join("a.jpg"), item.clone());
    select(&mut a, 0, "a.jpg");
    a.info.sections |= Section::OpenWith.bit();
    open_on(&mut a);
    // Linux: menú de apps, la elegida y la confirmación.
    host.set_menu_choice(Some(1));
    host.set_answer(Some(true));
    click(&mut a, InfoHit::ChangeAll);
    assert_eq!(FakeInfo::take_applied(), [InfoChange::ChangeAll { path: d.join("a.jpg"), app: "gimp.desktop".into() }]);
    host.set_menu_choice(None);
    click(&mut a, InfoHit::ChangeAll);
    assert!(FakeInfo::take_applied().is_empty(), "sin elegir, nada");
    // Windows: el diálogo del sistema, sin preguntar.
    if let Some(ow) = item.open_with.as_mut() {
        ow.system_dialog = true;
    }
    FakeInfo::serve(&d.join("a.jpg"), item);
    a.info.tx.send(InfoReply::Applied { generation: a.info.generation, section: Section::OpenWith, error: None }).unwrap();
    deliver(&mut a);
    a.paint();
    host.set_answer(None);
    click(&mut a, InfoHit::ChangeAll);
    assert_eq!(FakeInfo::take_applied().len(), 1);
}

/// Permisos de una carpeta: primero se confirma el cambio y después se pregunta por lo de adentro.
#[test]
fn info_perms_folder_confirms_then_asks() {
    let d = fixture("permisos", &[("sub/", ""), ("sub/a.txt", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    let sub = d.join("sub");
    let mut item = full_item(&sub);
    item.is_dir = true;
    item.open_with = None;
    FakeInfo::serve(&sub, item);
    a.info.size_thread = false;
    select(&mut a, 0, "sub");
    open_on(&mut a);
    host.set_menu_choice(Some(2));
    host.set_answer(Some(false));
    click(&mut a, InfoHit::Perm(0));
    assert!(FakeInfo::take_applied().is_empty(), "sin confirmar no cambia nada");
    host.set_answer(Some(true));
    click(&mut a, InfoHit::Perm(0));
    assert_eq!(FakeInfo::take_applied(), [InfoChange::Perms { paths: vec![sub.clone()], mode: 0o044, recursive: true }]);
}

/// Permisos que no se pueden cambiar: en Unix la tabla de siempre (sin tocar); en Windows el dueño
/// y lo del usuario actual.
#[test]
fn info_perms_read_only_text_per_platform() {
    let d = fixture("permisos-ver", &[("a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    let mut item = full_item(&d.join("a.jpg"));
    if let Some(p) = item.perms.as_mut() {
        p.editable = false;
    }
    FakeInfo::serve(&d.join("a.jpg"), item.clone());
    select(&mut a, 0, "a.jpg");
    a.info.sections = 0x7F;
    host.client.set((1400.0, 2400.0));
    open_on(&mut a);
    let t = texts(&host);
    assert!(t.iter().any(|x| x == "staff") && t.iter().any(|x| x == "Todos"), "{t:?}");
    assert!(!t.iter().any(|x| x == "Tu usuario" || x.contains("Más en el sistema…»")), "{t:?}");
    assert!(!a.info.layout.hits.iter().any(|(h, _)| matches!(h, InfoHit::Perm(_))));
    if let Some(p) = item.perms.as_mut() {
        p.group.clear();
        p.mode = 0;
    }
    FakeInfo::serve(&d.join("a.jpg"), item);
    a.info.tx.send(InfoReply::Applied { generation: a.info.generation, section: Section::Perms, error: None }).unwrap();
    deliver(&mut a);
    a.paint();
    assert!(texts(&host).iter().any(|x| x == "Tu usuario"), "{:?}", texts(&host));
}

/// Sin selección (la carpeta abierta) no se ofrece renombrarla.
#[test]
fn info_name_hidden_for_current_folder() {
    let d = fixture("actual", &[("a.txt", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    open_on(&mut a);
    assert!(!texts(&host).iter().any(|x| x == "Nombre y extensión"), "{:?}", texts(&host));
    select(&mut a, 0, "a.txt");
    a.paint();
    a.flush_info();
    deliver(&mut a);
    a.paint();
    assert!(texts(&host).iter().any(|x| x == "Nombre y extensión"));
}

/// Un cambio que anduvo vuelve a leer, pero no corta la suma de la carpeta; las carpetas sin
/// permiso que no se pudieron contar se avisan.
#[test]
fn info_folder_size_survives_change_and_reports_skipped() {
    let d = fixture("suma-sigue", &[("Fotos/", ""), ("Fotos/a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    let folder = d.join("Fotos");
    let mut item = full_item(&folder);
    item.is_dir = true;
    item.open_with = None;
    FakeInfo::serve(&folder, item);
    select(&mut a, 0, "Fotos");
    open_on(&mut a);
    let size_gen = a.info.size_generation;
    click(&mut a, InfoHit::Flag(Flag::Hidden));
    deliver(&mut a);
    a.paint();
    assert_eq!(a.info.size_generation, size_gen, "la suma sigue");
    a.info.tx.send(InfoReply::FolderSize { generation: size_gen, path: folder.clone(), bytes: 2_000, files: 10, folders: 2, skipped: 3, done: true }).unwrap();
    deliver(&mut a);
    a.paint();
    let t = texts(&host);
    assert!(t.iter().any(|s| s == "12 elementos (sin contar 3 carpetas sin permiso)"), "{t:?}");
}

/// «Cambiar todos…» pide confirmación y manda la app actual; elegir otra app del menú también.
#[test]
fn info_open_with_and_change_all() {
    let d = fixture("abrir", &[("a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakeInfo::serve(&d.join("a.jpg"), full_item(&d.join("a.jpg")));
    select(&mut a, 0, "a.jpg");
    a.info.sections |= Section::OpenWith.bit();
    open_on(&mut a);
    host.set_answer(None);
    click(&mut a, InfoHit::ChangeAll);
    assert!(FakeInfo::take_applied().is_empty(), "sin confirmar no cambia nada");
    host.set_answer(Some(true));
    click(&mut a, InfoHit::ChangeAll);
    assert_eq!(FakeInfo::take_applied(), [InfoChange::ChangeAll { path: d.join("a.jpg"), app: "fotos".into() }]);
    host.set_menu_choice(Some(0));
    click(&mut a, InfoHit::OpenWithMenu);
    assert_eq!(FakeInfo::take_applied(), [InfoChange::OpenWith { paths: vec![d.join("a.jpg")], app: "fotos".into() }]);
}

/// Etiquetas: quitar una y agregar otra escribiendo.
#[test]
fn info_tags_add_remove() {
    let d = fixture("etiquetas", &[("a.jpg", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    FakeInfo::serve(&d.join("a.jpg"), full_item(&d.join("a.jpg")));
    select(&mut a, 0, "a.jpg");
    open_on(&mut a);
    click(&mut a, InfoHit::TagRemove(0));
    assert_eq!(FakeInfo::take_applied(), [InfoChange::Tags { paths: vec![d.join("a.jpg")], add: vec![], remove: vec!["Rojo".into()] }]);
    deliver(&mut a);
    a.paint();
    click(&mut a, InfoHit::TagAdd);
    let (b, _) = a.info.edit.expect("caja de etiqueta");
    host.text_set(b, "Bariloche");
    a.on_app_message(WM_APP_INFO_EDIT_DONE, 1, 0);
    assert_eq!(FakeInfo::take_applied(), [InfoChange::Tags { paths: vec![d.join("a.jpg")], add: vec!["Bariloche".into()], remove: vec![] }]);
}

/// La respuesta de un cambio que llega después de otra relectura (otro cambio la pidió) igual vale
/// mientras el sujeto sea el mismo.
#[test]
fn info_late_applied_still_counts() {
    let d = fixture("tarde", &[("a.txt", "x")]);
    let (mut a, host) = app();
    go(&mut a, 0, &d);
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    open_on(&mut a);
    let g0 = a.info.generation;
    FakeInfo::take_requests();
    a.info.tx.send(InfoReply::Applied { generation: g0, section: Section::Tags, error: None }).unwrap();
    deliver(&mut a);
    assert_eq!(FakeInfo::take_requests().len(), 1, "un cambio que anduvo vuelve a leer");
    assert!(a.info.generation > g0);
    a.info.tx.send(InfoReply::Applied { generation: g0, section: Section::Comments, error: Some("Tarde".into()) }).unwrap();
    deliver(&mut a);
    a.info.sections |= Section::Comments.bit();
    a.paint();
    assert!(texts(&host).iter().any(|x| x == "Tarde"), "{:?}", texts(&host));
}

/// Expulsar el disco con el panel sumando una carpeta de ese disco: se corta la suma (la señal
/// compartida queda en `true` sin esperar al hilo), el panel suelta la carpeta y una respuesta
/// tardía de esa suma no vuelve a aparecer.
#[test]
fn eject_stops_folder_size_on_volume() {
    let root = fixture("expulsa-suma", &[("Fotos/", ""), ("Fotos/a.jpg", "x")]);
    let (mut a, _host) = app();
    FakeFileSystem::set_drives(&[(&root, "PENDRIVE")]);
    a.ejectable.insert(root.clone());
    crate::sys::testing::FakeShell::set_eject_result(&root, Ok(()));
    go(&mut a, 0, &root);
    a.set_active(0);
    select(&mut a, 0, "Fotos");
    open_on(&mut a);
    let folder = root.join("Fotos");
    assert_eq!(a.info_folder(), Some((folder.clone(), false)), "sumándose");
    // El «hilo» de la suma: lo que recibiría al arrancar.
    let cancel = a.info.size_cancel.clone();
    let generation = a.info.size_generation;
    assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed));
    a.run_command(Command::Eject);
    assert!(cancel.load(std::sync::atomic::Ordering::Relaxed), "la suma se cortó al soltar el disco");
    assert_eq!(a.info_folder(), None);
    assert!(!a.info_subject().iter().any(|p| p.starts_with(&root)), "{:?}", a.info_subject());
    a.info.tx.send(InfoReply::FolderSize { generation, path: folder.clone(), bytes: 9, files: 1, folders: 0, skipped: 0, done: true }).unwrap();
    deliver(&mut a);
    assert_eq!(a.info_folder(), None, "la respuesta tardía no cuenta");
    while a.notice.is_none() {
        let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("expulsar termina");
        a.on_task_done(t);
    }
    assert_eq!(a.notice.as_deref(), Some("Se expulsó \"PENDRIVE\": ya lo podés quitar"));
    FakeFileSystem::set_drives(&[]);
}

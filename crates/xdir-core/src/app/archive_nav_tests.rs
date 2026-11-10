//! Navegar dentro de archivos comprimidos en la `App`: entrar, abrir lo de adentro, contraseña,
//! solo lectura, copiar hacia afuera y vista previa. La carpeta del zip es real (la lista la lee
//! del disco); el zip y lo que se extrae viven en el disco falso (`FakeArchives` compartido).

use super::*;
use crate::archive_nav::{BAD_NAME, READ_ONLY};
use crate::preview::PreviewTarget;
use crate::sys::testing::{FakeArchives, FakeCopyPrims, FakeEnv, FakeHost, FakePreview, FakeShell, ShellCall};
use crate::transfer::JobSpec;
use std::path::Path;
use std::time::Duration;

fn app(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakeEnv::hold_keys(&[]);
    let mut a = App::new(Init::blank(), host);
    a.set_layout(Layout::TwoSide);
    // Sin la columna de vista previa (que extrae lo enfocado), salvo donde se la prende.
    a.preview.on = false;
    FakeShell::take_calls();
    a
}

/// Carpeta real con el zip (para la lista) y el mismo zip en el disco falso con `entries`.
fn fixture(name: &str, entries: &[(&str, &[u8])]) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join("xdir-test-zipnav").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let zip = dir.join("fotos.zip");
    std::fs::write(&zip, b"zip").unwrap();
    FakeArchives::shared().add_archive(&zip, entries, 40);
    (dir, zip)
}

fn pump(a: &mut App, pane: usize) {
    while a.panes[pane].tab().loading {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
}

fn go(a: &mut App, pane: usize, to: Location) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[pane].tab_mut().navigate(hwnd, &tx, to);
    pump(a, pane);
}

fn idx_of(a: &App, pane: usize, name: &str) -> usize {
    a.panes[pane].tab().list.entries.iter().position(|e| e.name == name).expect(name)
}

fn select(a: &mut App, pane: usize, name: &str) {
    let idx = idx_of(a, pane, name);
    let list = &mut a.panes[pane].tab_mut().list;
    let pos = (0..list.count()).find(|&p| list.entry_of_pos(p) == Some(idx)).unwrap();
    list.select_pos(pos);
}

fn names(a: &App, pane: usize) -> Vec<String> {
    let mut v: Vec<String> = a.panes[pane].tab().list.entries.iter().map(|e| e.display.clone()).collect();
    v.sort();
    v
}

/// Espera el próximo trabajo suelto de un archivo comprimido y lo procesa (los demás también).
fn task(a: &mut App) {
    loop {
        let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("el trabajo termina");
        let ours = matches!(t, tasks::TaskDone::ArchiveOpened { .. } | tasks::TaskDone::ArchivePrepared { .. });
        if !ours {
            eprintln!("otro trabajo");
        }
        a.on_task_done(t);
        if ours {
            return;
        }
    }
}

fn root(zip: &Path) -> Location {
    Location::Archive { file: zip.to_path_buf(), nested: vec![], inner: String::new() }
}

#[test]
fn double_click_zip_enters_it() {
    let mut a = app(FakeHost::new());
    let (dir, zip) = fixture("entra", &[("2024/a.jpg", b"a"), ("nota.txt", b"n")]);
    go(&mut a, 0, Location::Path(dir.clone()));
    let i = idx_of(&a, 0, "fotos.zip");
    a.apply(0, Action::Open(i));
    pump(&mut a, 0);
    assert_eq!(a.panes[0].tab().location, root(&zip));
    assert_eq!(names(&a, 0), ["2024", "nota.txt"]);
    // Las columnas de adentro.
    let cols: Vec<list::Column> = a.panes[0].tab().list.columns.iter().map(|c| c.col).collect();
    assert!(cols.contains(&list::Column::Packed), "{cols:?}");
    // Subir vuelve a la carpeta con el zip enfocado.
    a.run_command(Command::Up);
    pump(&mut a, 0);
    assert_eq!(a.panes[0].tab().location, Location::Path(dir));
    let tab = a.panes[0].tab();
    assert_eq!(tab.list.focused_index().map(|i| tab.list.entries[i].name.clone()), Some("fotos.zip".into()));
}

#[test]
fn alt_double_click_opens_with_system() {
    let mut a = app(FakeHost::new());
    let (dir, zip) = fixture("alt", &[("a.txt", b"a")]);
    go(&mut a, 0, Location::Path(dir.clone()));
    FakeEnv::hold_keys(&[VK_MENU]);
    let i = idx_of(&a, 0, "fotos.zip");
    a.apply(0, Action::Open(i));
    FakeEnv::hold_keys(&[]);
    assert_eq!(a.panes[0].tab().location, Location::Path(dir.clone()));
    assert!(FakeShell::take_calls().contains(&ShellCall::OpenFile(zip, Some(dir))));
}

#[test]
fn open_file_inside_extracts_and_opens() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("abre", &[("docs/nota.txt", b"hola")]);
    go(&mut a, 0, Location::Archive { file: zip.clone(), nested: vec![], inner: "docs".into() });
    let i = idx_of(&a, 0, "nota.txt");
    a.apply(0, Action::Open(i));
    assert_eq!(a.notice.as_deref(), Some("Abriendo «nota.txt»…   Esc cancela"));
    task(&mut a);
    let temp = crate::archive_nav::temps::global().entry_path(&zip, &[], "docs/nota.txt").unwrap();
    assert_eq!(FakeCopyPrims::shared().read_content(&temp).unwrap(), b"hola");
    let calls = FakeShell::take_calls();
    assert!(calls.contains(&ShellCall::OpenFile(temp.clone(), temp.parent().map(Path::to_path_buf))), "{calls:?}");
    assert_eq!(a.notice, None);
}

#[test]
fn nested_zip_navigable() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("anidado", &[("dir/viejo.zip", b"zz"), ("dir/b.txt", b"b")]);
    // El zip de adentro, una vez extraído a la temporal, es otro «zip» del falso.
    let inner = crate::archive_nav::temps::global().ensure(&zip, &[], "dir/viejo.zip", &crate::archive_nav::temps::Limits::up_to(u64::MAX)).unwrap();
    FakeArchives::shared().add_archive(&inner, &[("hondo/x.txt", b"x")], 2);
    go(&mut a, 0, Location::Archive { file: zip.clone(), nested: vec![], inner: "dir".into() });
    let i = idx_of(&a, 0, "viejo.zip");
    a.apply(0, Action::Open(i));
    pump(&mut a, 0);
    let here = Location::Archive { file: zip.clone(), nested: vec!["dir/viejo.zip".into()], inner: String::new() };
    assert_eq!(a.panes[0].tab().location, here);
    assert_eq!(names(&a, 0), ["hondo"]);
    let crumbs: Vec<String> = here.crumbs().into_iter().map(|c| c.0).rev().take(3).collect();
    assert_eq!(crumbs, ["viejo.zip", "dir", "fotos.zip"]);
}

#[test]
fn password_zip_offers_system() {
    let host = FakeHost::new();
    host.set_answer(Some(true));
    let mut a = app(host.clone());
    let (dir, zip) = fixture("clave", &[("a.txt", b"a")]);
    FakeArchives::shared().set_password(&zip);
    go(&mut a, 0, Location::Path(dir.clone()));
    let i = idx_of(&a, 0, "fotos.zip");
    a.apply(0, Action::Open(i));
    pump(&mut a, 0);
    pump(&mut a, 0);
    assert_eq!(a.panes[0].tab().location, Location::Path(dir.clone()));
    assert!(host.questions().iter().any(|(t, q)| t == "Abrir con el sistema" && q == "«fotos.zip»: Este zip tiene contraseña. ¿Abrir con el sistema?"), "{:?}", host.questions());
    assert!(FakeShell::take_calls().contains(&ShellCall::OpenFile(zip, Some(dir))));
}

#[test]
fn bad_name_and_link_not_opened() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("malo", &[("../fuera.txt", b"x"), ("ok.txt", b"o")]);
    FakeArchives::shared().add_link(&zip, "enlace", Some("ok.txt"));
    go(&mut a, 0, root(&zip));
    let i = idx_of(&a, 0, "../fuera.txt");
    a.apply(0, Action::Open(i));
    assert_eq!(a.notice.as_deref(), Some(BAD_NAME));
    let i = idx_of(&a, 0, "enlace");
    a.apply(0, Action::Open(i));
    assert_eq!(a.notice.as_deref(), Some(crate::archive_nav::LINK));
    assert!(!a.task_rx.try_iter().any(|t| matches!(t, tasks::TaskDone::ArchiveOpened { .. })));
}

#[test]
fn writing_commands_show_read_only_notice() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("lectura", &[("a.txt", b"a")]);
    go(&mut a, 0, root(&zip));
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    for c in [Command::Delete, Command::Recycle, Command::Rename, Command::Cut, Command::Paste, Command::NewFolder, Command::Compress, Command::Duplicate] {
        a.notice = None;
        a.run_command(c);
        assert_eq!(a.notice.as_deref(), Some(READ_ONLY), "{c:?}");
    }
    assert!(a.rename.is_none());
    assert!(a.take_started().is_empty());
    assert!(FakeShell::take_calls().iter().all(|c| !matches!(c, ShellCall::InvokeVerb { .. })));
}

#[test]
fn context_menu_disables_writing_entries() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let (_, zip) = fixture("menu", &[("a.txt", b"a")]);
    go(&mut a, 0, root(&zip));
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    host.set_menu_choice(None);
    a.on_context_menu(Some((40.0, 80.0)));
    let menu = host.menus().pop().expect("menú propio");
    let items: Vec<(String, bool)> = menu
        .into_iter()
        .filter_map(|e| match e {
            shell::Entry::Item { label, enabled, .. } => Some((label, enabled)),
            _ => None,
        })
        .collect();
    let enabled = |l: &str| items.iter().find(|(x, _)| x == l).map(|(_, e)| *e);
    assert_eq!(enabled("Copiar"), Some(true));
    for l in ["Abrir", "Abrir con…", "Extraer aquí", "Extraer en…", "Copiar como ruta de acceso"] {
        assert_eq!(enabled(l), Some(true), "{l}");
    }
    for l in ["Cortar", "Renombrar", "Eliminar", "Nueva carpeta", "Comprimir"] {
        assert_eq!(enabled(l), Some(false), "{l}");
    }
    assert!(FakeShell::take_calls().iter().all(|c| !matches!(c, ShellCall::ContextMenu { .. })));
}

/// «Extraer aquí» del menú de adentro: lo elegido va a la carpeta del archivo con el motor.
#[test]
fn context_menu_extract_here() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let (dir, zip) = fixture("menu-extraer", &[("d/a.txt", b"a")]);
    go(&mut a, 0, Location::Archive { file: zip.clone(), nested: vec![], inner: "d".into() });
    a.set_active(0);
    select(&mut a, 0, "a.txt");
    host.set_menu_choice(Some(3));
    a.on_context_menu(Some((40.0, 80.0)));
    assert_eq!(a.take_started(), [JobSpec::ExtractEntries { archive: zip, entries: vec!["d/a.txt".into()], dest: dir }]);
}

#[test]
fn copy_paste_out_of_archive_extracts() {
    let mut a = app(FakeHost::new());
    let (dir, zip) = fixture("copia", &[("fotos/a.jpg", b"a"), ("b.txt", b"b")]);
    let dest = dir.join("destino");
    std::fs::create_dir_all(&dest).unwrap();
    go(&mut a, 0, root(&zip));
    go(&mut a, 1, Location::Path(dest.clone()));
    a.set_active(0);
    select(&mut a, 0, "fotos");
    a.run_command(Command::Copy);
    assert!(FakeShell::take_calls().contains(&ShellCall::ClearClipboard));
    a.set_active(1);
    a.run_command(Command::Paste);
    assert_eq!(a.take_started(), [JobSpec::ExtractEntries { archive: zip, entries: vec!["fotos".into()], dest }]);
    // Copiar otra cosa afuera reemplaza lo copiado del zip.
    a.run_command(Command::Copy);
    assert!(a.archive.clip.is_none());
}

#[test]
fn drag_out_of_archive_stays_inside_xdir() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("arrastre", &[("a.txt", b"a")]);
    go(&mut a, 0, root(&zip));
    select(&mut a, 0, "a.txt");
    a.start_drag(0);
    assert!(matches!(a.drag, Drag::ArchiveOut(0)));
    // Soltar sobre nada: no pasa nada.
    a.drop_archive_drag(0, -50.0, -50.0);
    assert!(a.take_started().is_empty());
}

#[test]
fn preview_of_file_inside_archive_extracts_then_shows() {
    let mut a = app(FakeHost::new());
    a.preview.on = true;
    FakePreview::set_delay(None);
    let (_, zip) = fixture("vista", &[("foto.png", b"png"), ("enorme.bin", b"")]);
    FakeArchives::shared().set_declared(&zip, "enorme.bin", crate::thumbs::BIG_CACHE_BYTES as u64 + 1);
    go(&mut a, 0, root(&zip));
    a.set_active(0);
    select(&mut a, 0, "foto.png");
    FakePreview::take_requests();
    a.paint();
    a.flush_preview();
    assert!(FakePreview::take_requests().is_empty(), "todavía no está extraída");
    task(&mut a);
    a.paint();
    a.flush_preview();
    let temp = crate::archive_nav::temps::global().entry_path(&zip, &[], "foto.png").unwrap();
    assert_eq!(FakePreview::take_requests(), [PreviewTarget::File(temp)]);
    // Lo que pasa del límite de la vista previa no se extrae.
    select(&mut a, 0, "enorme.bin");
    a.paint();
    assert!(a.task_rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn restore_archive_tab_falls_back_to_folder() {
    crate::sys::testing::install_fakes();
    let gone = std::env::temp_dir().join("xdir-test-zipnav").join("no-esta.zip");
    let mut init = Init::blank();
    init.session.panes[0] = session::PaneState { tabs: vec![Location::Archive { file: gone.clone(), nested: vec![], inner: "a".into() }], active: 0 };
    let a = App::new(init, FakeHost::new());
    assert_eq!(a.panes[0].tab().location, Location::Path(gone.parent().unwrap().to_path_buf()));
}

#[test]
fn middle_click_folder_in_archive_opens_tab_inside() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("medio", &[("d/x.txt", b"x")]);
    go(&mut a, 0, root(&zip));
    let before = a.panes[0].tabs.len();
    a.run_command(Command::NewTab);
    assert_eq!(a.panes[0].tabs.len(), before + 1);
    assert_eq!(a.panes[0].tab().location, root(&zip));
}

/// La barra muestra el archivo con su ícono (X68) y, mientras se lee, la lista dice «Cargando…»
/// (X70).
#[test]
fn crumb_icon_and_loading_drawn() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let (_, zip) = fixture("dibujo", &[("d/a.txt", b"a")]);
    // En la raíz el último tramo (que siempre entra) es el del archivo.
    go(&mut a, 0, root(&zip));
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|t| t.1).collect();
    assert!(texts.iter().any(|t| t == crate::pane::ARCHIVE_GLYPH), "{texts:?}");
    assert!(texts.iter().any(|t| t == "fotos.zip"));
    assert!(!texts.iter().any(|t| t == "Cargando…"));
    // Una carga en curso (sin respuesta todavía): «Cargando…».
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Archive { file: zip.clone(), nested: vec![], inner: "d".into() });
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|t| t.1).collect();
    assert!(texts.iter().any(|t| t == "Cargando…"), "{texts:?}");
    pump(&mut a, 0);
}

/// Un nombre no permitido nunca se vuelve una ruta del disco: ni la ruta del elemento, ni la
/// miniatura, ni la selección (Copiar como ruta, información, vista rápida). Los buenos quedan
/// dentro de la temporal.
#[test]
fn unsafe_names_never_become_disk_paths() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("rutas", &[("/etc/hosts", b"h"), ("..\\x.txt", b"x"), ("\\\\srv\\s\\x.jpg", b"j"), ("C:\\x.jpg", b"c"), ("ok.jpg", b"o")]);
    go(&mut a, 0, root(&zip));
    a.set_active(0);
    let run = crate::archive_nav::temps::global().dir_for(&zip, &[]);
    let tab = a.panes[0].tab();
    for (i, e) in tab.list.entries.iter().enumerate() {
        let p = tab.entry_path(i);
        let bad = e.attrs & crate::fs::ATTR_ARCHIVE_BAD != 0;
        if bad {
            assert_eq!(p, PathBuf::new(), "{:?}", e.name);
            assert_eq!(tab.list.thumb_source(e), None, "{:?}", e.name);
        } else {
            assert!(p.starts_with(&run), "{p:?}");
            assert_eq!(tab.list.thumb_source(e), Some(crate::thumbs::Source::Path(p)));
        }
    }
    assert_eq!(tab.list.entries.iter().filter(|e| e.attrs & crate::fs::ATTR_ARCHIVE_BAD != 0).count(), 4);
    a.panes[0].tab_mut().list.select_all();
    let sel = a.selection(0);
    assert_eq!(sel.len(), 1);
    assert!(sel[0].starts_with(&run));
    assert!(a.selection_items(0).iter().all(|i| i.path.starts_with(&run)));
    // La información no lee nada de adentro.
    assert!(a.info_wanted().0.is_empty());
    // Copiar las rutas: la de adentro del archivo, nunca la del disco que eligió el autor.
    a.run_command(Command::CopyPaths);
    let copied: Vec<String> = FakeShell::take_calls().into_iter().filter_map(|c| if let ShellCall::CopyText(t) = c { Some(t) } else { None }).collect();
    assert_eq!(copied, [format!("\"{}/ok.jpg\"", zip.display())]);
}

/// Escape mientras se extrae para abrir: no se abre nada.
#[test]
fn escape_cancels_opening_from_archive() {
    let mut a = app(FakeHost::new());
    let (_, zip) = fixture("cancela", &[("nota.txt", b"hola")]);
    go(&mut a, 0, root(&zip));
    a.set_active(0);
    let i = idx_of(&a, 0, "nota.txt");
    a.apply(0, Action::Open(i));
    assert!(a.on_key(VK_ESCAPE));
    assert_eq!(a.notice.as_deref(), Some("Se canceló la apertura"));
    task(&mut a);
    assert!(FakeShell::take_calls().iter().all(|c| !matches!(c, ShellCall::OpenFile(..))));
    assert!(a.archive.opening.is_none());
}

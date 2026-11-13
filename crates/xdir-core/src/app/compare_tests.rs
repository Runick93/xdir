//! «Comparar contenido» en la `App`: qué dos archivos se toman, el resultado en otro hilo, Cancelar,
//! la red que no responde, los comprimidos y el menú (carpetas temporales reales).

use super::compare::{NEED_TWO, Status};
use super::*;
use crate::compare::Verdict;
use crate::sys::testing::{FakeArchives, FakeEnv, FakeHost, FakeNetwork, FakePreview};
use std::time::{Duration, Instant};

fn app(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    FakeEnv::hold_keys(&[]);
    FakePreview::set_delay(None);
    let mut a = App::new(Init::blank(), host);
    a.set_layout(Layout::TwoSide);
    a.preview.on = false;
    a
}

/// Carpeta real con esos archivos (nombre, contenido).
fn fixture(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-comparar-app").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, data) in files {
        if f.ends_with('/') {
            std::fs::create_dir_all(dir.join(f)).unwrap();
        } else {
            std::fs::write(dir.join(f), data).unwrap();
        }
    }
    dir
}

fn go(a: &mut App, pane: usize, to: Location) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[pane].tab_mut().navigate(hwnd, &tx, to);
    while a.panes[pane].tab().loading {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
}

fn clear(a: &mut App, pane: usize) {
    let list = &mut a.panes[pane].tab_mut().list;
    for p in 0..list.count() {
        list.set_selected_pos(p, false);
    }
}

fn select(a: &mut App, pane: usize, names: &[&str]) {
    clear(a, pane);
    let list = &mut a.panes[pane].tab_mut().list;
    for n in names {
        let pos = (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == *n).expect(n);
        list.set_selected_pos(pos, true);
    }
}

/// Espera a que la comparación termine (sin suponer el orden en que llegan los avisos).
fn wait_done(a: &mut App) -> Status {
    let t0 = Instant::now();
    loop {
        a.on_compare_message(0);
        let st = a.compare.as_ref().expect("panel de comparar").status.clone();
        if st != Status::Running {
            return st;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "la comparación no terminó");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// El comando: identificador, nombre, sin atajo de fábrica, en «Archivos» y en la tabla de la wiki.
#[test]
fn command_spec_and_wiki_table() {
    crate::sys::testing::install_fakes();
    let s = crate::commands::ALL.iter().find(|s| s.id == "comparar_contenido").expect("comando");
    assert_eq!((s.command, s.name, s.default_keys), (Command::CompareContents, "Comparar contenido", &[][..]));
    assert_eq!(crate::commands::group(Command::CompareContents), crate::commands::Group::Files);
    assert!(crate::commands::markdown_table(sys::env()).contains("| Comparar contenido |  | `comparar_contenido` |"));
    // El editor de atajos lo lista con los demás de «Archivos».
    let km = crate::commands::Keymap::default();
    assert!(km.combos(Command::CompareContents).is_empty());
}

/// Dos archivos en el panel activo: iguales.
#[test]
fn two_in_the_active_pane() {
    let mut a = app(FakeHost::new());
    let d = fixture("activo", &[("a.txt", b"hola mundo"), ("b.txt", b"hola mundo"), ("c.txt", b"hola Mundo")]);
    go(&mut a, 0, Location::Path(d.clone()));
    select(&mut a, 0, &["a.txt", "b.txt"]);
    a.run_command(Command::CompareContents);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::Same));
    let st = a.compare.as_ref().unwrap();
    assert_eq!([st.sides[0].name.as_str(), st.sides[1].name.as_str()], ["a.txt", "b.txt"]);
    assert_eq!(st.sides[0].folder, super::info::short_location(&d));
    assert!(st.sides[0].folder.ends_with(&d.file_name().unwrap().to_string_lossy().into_owned()));
    // Distintos: la posición del primer byte distinto.
    select(&mut a, 0, &["a.txt", "c.txt"]);
    a.run_command(Command::CompareContents);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::DiffersAt(5)));
    assert!(matches!(a.compare_view().unwrap().result, crate::compare::draw::ResultView::Verdict { same: false, detail } if detail == "Primer byte distinto: 5 (0x5)"));
}

/// Uno elegido en cada panel (con dos a la vista); con más paneles, no.
#[test]
fn one_in_each_visible_pane() {
    let mut a = app(FakeHost::new());
    let d1 = fixture("izq", &[("a.bin", b"1234")]);
    let d2 = fixture("der", &[("b.bin", b"1234"), ("c.bin", b"5678")]);
    go(&mut a, 0, Location::Path(d1));
    go(&mut a, 1, Location::Path(d2));
    select(&mut a, 0, &["a.bin"]);
    select(&mut a, 1, &["b.bin"]);
    a.set_active(0);
    a.run_command(Command::CompareContents);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::Same));
    // Dos elegidos en el otro panel: no se sabe con cuál.
    select(&mut a, 1, &["b.bin", "c.bin"]);
    a.compare = None;
    a.run_command(Command::CompareContents);
    assert!(a.compare.is_none());
    assert_eq!(a.notice.as_deref(), Some(NEED_TWO));
    // Con cuatro paneles a la vista, el «otro» no está claro.
    select(&mut a, 1, &["b.bin"]);
    a.set_layout(Layout::Four);
    a.notice = None;
    a.run_command(Command::CompareContents);
    assert!(a.compare.is_none());
    assert_eq!(a.notice.as_deref(), Some(NEED_TWO));
}

/// Una carpeta elegida, o uno solo: aviso y nada más.
#[test]
fn folders_or_one_file_only_warn() {
    let mut a = app(FakeHost::new());
    let d = fixture("carpeta", &[("a.txt", b"x"), ("sub/", b"")]);
    go(&mut a, 0, Location::Path(d.clone()));
    go(&mut a, 1, Location::Path(d));
    clear(&mut a, 1);
    select(&mut a, 0, &["a.txt", "sub"]);
    a.run_command(Command::CompareContents);
    assert!(a.compare.is_none());
    assert_eq!(a.notice.as_deref(), Some(NEED_TWO));
    select(&mut a, 0, &["a.txt"]);
    a.notice = None;
    a.run_command(Command::CompareContents);
    assert!(a.compare.is_none());
    assert_eq!(a.notice.as_deref(), Some(NEED_TWO));
}

/// Tamaños distintos (los de la lista): la respuesta es inmediata, sin hilo.
#[test]
fn different_sizes_answer_without_reading() {
    let mut a = app(FakeHost::new());
    let d = fixture("tamanos", &[("a.txt", b"hola"), ("b.txt", b"hola mundo")]);
    go(&mut a, 0, Location::Path(d));
    select(&mut a, 0, &["a.txt", "b.txt"]);
    a.run_command(Command::CompareContents);
    let st = a.compare.as_ref().unwrap();
    assert!(st.job.is_none(), "no se lee nada");
    assert_eq!(st.status, Status::Done(Verdict::SizeDiffers { a: 4, b: 10 }));
}

/// Cancelar corta y deja «cancelada»; el resultado que llegue después no la pisa.
#[test]
fn cancel_stops_and_ignores_late_result() {
    let mut a = app(FakeHost::new());
    let big = vec![9u8; 3 << 20];
    let d = fixture("cancelar", &[("a.bin", &big), ("b.bin", &big)]);
    FakeNetwork::set_remote(&d);
    FakeNetwork::hang(&d.to_string_lossy(), Duration::from_millis(300));
    go(&mut a, 0, Location::Path(d));
    select(&mut a, 0, &["a.bin", "b.bin"]);
    a.run_command(Command::CompareContents);
    assert_eq!(a.compare.as_ref().unwrap().status, Status::Running);
    a.cancel_compare();
    assert_eq!(a.compare.as_ref().unwrap().status, Status::Cancelled);
    std::thread::sleep(Duration::from_millis(400));
    a.on_compare_message(0);
    assert_eq!(a.compare.as_ref().unwrap().status, Status::Cancelled);
}

/// Un servidor de red que deja de responder: «no responde» al pasar el límite sin avance.
#[test]
fn stalled_network_says_not_responding() {
    let mut a = app(FakeHost::new());
    let d = fixture("colgada", &[("a.bin", b"12345678"), ("b.bin", b"12345678")]);
    FakeNetwork::set_remote(&d);
    FakeNetwork::hang(&d.to_string_lossy(), Duration::from_secs(3));
    a.jobs.limit = Duration::from_millis(150);
    go(&mut a, 0, Location::Path(d));
    select(&mut a, 0, &["a.bin", "b.bin"]);
    let t0 = Instant::now();
    a.run_command(Command::CompareContents);
    let st = wait_done(&mut a);
    assert!(matches!(&st, Status::Failed(m) if m.contains("no responde")), "{st:?}");
    assert!(t0.elapsed() < Duration::from_secs(2));
}

/// Dentro de un comprimido: dos elementos se extraen a la temporal y se comparan.
#[test]
fn inside_an_archive() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("zip", &[("fotos.zip", b"zip")]);
    let zip = d.join("fotos.zip");
    FakeArchives::shared().add_archive(&zip, &[("a.txt", b"hola mundo"), ("b.txt", b"hola mundo"), ("c.txt", b"hola Mundo")], 40);
    go(&mut a, 0, Location::Archive { file: zip.clone(), nested: Vec::new(), inner: String::new() });
    select(&mut a, 0, &["a.txt", "c.txt"]);
    a.run_command(Command::CompareContents);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::DiffersAt(5)));
    // Uno de adentro y uno de afuera, en el otro panel.
    std::fs::write(d.join("afuera.txt"), b"hola mundo").unwrap();
    go(&mut a, 1, Location::Path(d.clone()));
    select(&mut a, 0, &["b.txt"]);
    select(&mut a, 1, &["afuera.txt"]);
    a.set_active(0);
    a.run_command(Command::CompareContents);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::Same));
    // El menú de adentro lo ofrece con dos elegidos.
    host.set_menu_choice(None);
    select(&mut a, 0, &["a.txt", "b.txt"]);
    a.archive_context_menu(0);
    let labels: Vec<String> = host.menus().last().unwrap().iter().filter_map(|e| match e {
        shell::Entry::Item { label, .. } => Some(label.clone()),
        _ => None,
    }).collect();
    assert!(labels.iter().any(|l| l == "Comparar contenido"), "{labels:?}");
}

/// El menú de los elementos: «Comparar contenido» con dos archivos, no con uno.
#[test]
fn item_menu_offers_compare() {
    let mut a = app(FakeHost::new());
    let d = fixture("menu", &[("a.txt", b"x"), ("b.txt", b"y")]);
    go(&mut a, 0, Location::Path(d.clone()));
    go(&mut a, 1, Location::Path(d));
    clear(&mut a, 1);
    let labels = |a: &App| {
        let mut cmds = Vec::new();
        a.item_entries(&mut cmds)
            .into_iter()
            .filter_map(|e| match e {
                shell::Entry::Item { label, .. } => Some(label),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    select(&mut a, 0, &["a.txt"]);
    assert!(!labels(&a).iter().any(|l| l == "Comparar contenido"));
    select(&mut a, 0, &["a.txt", "b.txt"]);
    assert!(labels(&a).iter().any(|l| l == "Comparar contenido"));
}

// ---------- El panel ----------

fn texts(host: &FakeHost) -> Vec<String> {
    host.drawn_texts().into_iter().map(|t| t.1).collect()
}

/// Clic en el centro de lo que el panel dibujó para `h`.
fn click(a: &mut App, h: crate::compare::draw::CompareHit) {
    a.paint();
    let r = a.compare.as_ref().unwrap().layout.hits.iter().find(|(x, _)| *x == h).unwrap_or_else(|| panic!("{h:?} no está")).1;
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
    a.on_left_up((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
}

/// Espera a que la suma del lado `i` termine.
fn wait_hash(a: &mut App, i: usize) {
    let t0 = Instant::now();
    while a.compare.as_ref().unwrap().hashes[i].job.is_some() {
        assert!(t0.elapsed() < Duration::from_secs(10), "la suma no llegó");
        std::thread::sleep(Duration::from_millis(5));
        a.on_compare_message(0);
    }
}

/// El panel ocupa el lugar de la vista previa (uno a la vez con información y Renombrar varios),
/// muestra el resultado y se cierra con ✕ o Esc.
#[test]
fn panel_in_the_right_column() {
    use super::preview::RightPanel;
    use crate::compare::draw::CompareHit;
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("panel", &[("a.txt", b"hola"), ("b.txt", b"hola")]);
    go(&mut a, 0, Location::Path(d));
    select(&mut a, 0, &["a.txt", "b.txt"]);
    a.run_command(Command::CompareContents);
    assert_eq!(a.preview.owner, RightPanel::Compare);
    wait_done(&mut a);
    a.paint();
    let t = texts(&host);
    assert!(t.iter().any(|s| s == "Comparar contenido") && t.iter().any(|s| s == "Iguales"), "{t:?}");
    assert!(t.iter().any(|s| s == "El contenido es idéntico, byte a byte"), "{t:?}");
    // Información lo reemplaza; comparar de nuevo reemplaza a información.
    a.run_command(Command::Properties);
    assert_eq!(a.preview.owner, RightPanel::Info);
    assert!(a.compare.is_none());
    a.run_command(Command::CompareContents);
    assert_eq!(a.preview.owner, RightPanel::Compare);
    assert!(!a.info_open());
    // ✕ cierra y vuelve la columna como estaba (apagada).
    wait_done(&mut a);
    click(&mut a, CompareHit::Close);
    assert!(a.compare.is_none());
    assert_eq!(a.preview.owner, RightPanel::Preview);
    // Esc con el foco en el panel también.
    a.run_command(Command::CompareContents);
    wait_done(&mut a);
    a.paint();
    let r = a.compare.as_ref().unwrap().layout.panel;
    a.on_left_down(r.left + 5.0, r.bottom - 5.0, false);
    a.on_left_up(r.left + 5.0, r.bottom - 5.0);
    assert!(a.compare.as_ref().unwrap().focused);
    a.on_key(crate::keys::keys::VK_ESCAPE);
    assert!(a.compare.is_none());
}

/// «Calcular SHA-256» de cada uno, «Copiar» y si coinciden; «Comparar de nuevo» después de cancelar.
#[test]
fn panel_sums_copy_and_retry() {
    use crate::compare::draw::CompareHit;
    use crate::sys::testing::{FakeShell, ShellCall};
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("sumas", &[("a.txt", b"hola"), ("b.txt", b"hola")]);
    go(&mut a, 0, Location::Path(d));
    select(&mut a, 0, &["a.txt", "b.txt"]);
    a.run_command(Command::CompareContents);
    wait_done(&mut a);
    click(&mut a, CompareHit::Hash(0));
    click(&mut a, CompareHit::Hash(1));
    wait_hash(&mut a, 0);
    wait_hash(&mut a, 1);
    a.paint();
    assert!(texts(&host).iter().any(|s| s == "Las dos sumas coinciden"));
    let sum = a.compare.as_ref().unwrap().hashes[0].result.clone().unwrap().unwrap();
    assert_eq!(sum.len(), 64);
    FakeShell::take_calls();
    click(&mut a, CompareHit::Copy(0));
    assert!(FakeShell::take_calls().contains(&ShellCall::CopyText(sum)));
    // Cancelado y de nuevo.
    a.compare.as_mut().unwrap().status = Status::Cancelled;
    click(&mut a, CompareHit::Retry);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::Same));
}

// ---------- Suma de verificación en el panel de información ----------

use crate::info::Section;
use crate::info::draw::InfoHit;
use crate::sys::HashAlgo;

fn open_info(a: &mut App) {
    a.run_command(Command::Properties);
    a.paint();
    a.flush_info();
    a.on_app_message(crate::info::WM_APP_INFO, 0, 0);
    a.paint();
}

fn info_click(a: &mut App, h: InfoHit) {
    a.paint();
    let r = a.info.layout.hits.iter().find(|(x, _)| *x == h).unwrap_or_else(|| panic!("{h:?} no está: {:?}", a.info.layout.hits)).1;
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
    a.on_left_up((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
}

fn wait_sum(a: &mut App) {
    let t0 = Instant::now();
    while a.info.sum.job.is_some() {
        assert!(t0.elapsed() < Duration::from_secs(10), "la suma no llegó");
        std::thread::sleep(Duration::from_millis(5));
        a.on_compare_message(0);
    }
    a.paint();
}

/// La suma que da el falso para ese contenido (la misma cuenta que el panel).
fn sum_of(path: &std::path::Path, algo: HashAlgo) -> String {
    let p = crate::compare::Progress::default();
    crate::compare::hash_source(&crate::compare::Source::Disk(path.to_path_buf()), algo, &std::sync::atomic::AtomicBool::new(false), &p).unwrap()
}

fn mono_text(host: &FakeHost) -> String {
    host.drawn_texts().into_iter().filter(|t| t.0 == crate::paint::FontId::Mono).map(|t| t.1).collect()
}

/// Plegada de fábrica; abierta, calcula SHA-256 y MD5 en otro hilo, se ve en letra de ancho fijo y
/// se copia.
#[test]
fn info_checksum_calculates_and_copies() {
    use crate::sys::testing::{FakeShell, ShellCall};
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("info-suma", &[("a.txt", b"hola")]);
    go(&mut a, 0, Location::Path(d.clone()));
    select(&mut a, 0, &["a.txt"]);
    a.info.sections &= !Section::Checksum.bit();
    open_info(&mut a);
    assert!(a.info.layout.hits.iter().any(|(h, _)| *h == InfoHit::Toggle(Section::Checksum)));
    assert!(!a.info.layout.hits.iter().any(|(h, _)| *h == InfoHit::HashRun), "plegada");
    info_click(&mut a, InfoHit::Toggle(Section::Checksum));
    info_click(&mut a, InfoHit::HashRun);
    assert!(a.info.sum.job.is_some());
    wait_sum(&mut a);
    let sha = sum_of(&d.join("a.txt"), HashAlgo::Sha256);
    assert_eq!(mono_text(&host), sha);
    FakeShell::take_calls();
    info_click(&mut a, InfoHit::HashCopy);
    assert!(FakeShell::take_calls().contains(&ShellCall::CopyText(sha)));
    info_click(&mut a, InfoHit::HashAlgo(HashAlgo::Md5));
    info_click(&mut a, InfoHit::HashRun);
    wait_sum(&mut a);
    assert_eq!(mono_text(&host), sum_of(&d.join("a.txt"), HashAlgo::Md5));
}

/// «Comparar con…»: sin espacios ni mayúsculas, con el nombre al lado; el largo elige el
/// algoritmo y lo calcula; lo que no es una suma lo dice. También mientras se escribe en la caja.
#[test]
fn info_checksum_compare_with() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("info-comparar", &[("a.txt", b"hola")]);
    go(&mut a, 0, Location::Path(d.clone()));
    select(&mut a, 0, &["a.txt"]);
    a.info.sections |= Section::Checksum.bit();
    open_info(&mut a);
    let (sha, md5) = (sum_of(&d.join("a.txt"), HashAlgo::Sha256), sum_of(&d.join("a.txt"), HashAlgo::Md5));
    a.set_expected(&format!("{}  a.txt", sha.to_uppercase()));
    assert_eq!(a.info.sum.algo, HashAlgo::Sha256);
    wait_sum(&mut a);
    let texts = |h: &FakeHost| h.drawn_texts().into_iter().map(|t| t.1).collect::<Vec<_>>();
    assert!(texts(&host).iter().any(|t| t == "Coincide"), "{:?}", texts(&host));
    // Una MD5: cambia el algoritmo y la calcula.
    a.set_expected(&md5.replace('a', "b"));
    assert_eq!(a.info.sum.algo, HashAlgo::Md5);
    wait_sum(&mut a);
    assert!(texts(&host).iter().any(|t| t == "No coincide"));
    a.set_expected("hola");
    a.paint();
    assert!(texts(&host).iter().any(|t| t == crate::info::draw::INVALID_SUM_TEXT));
    // En la caja: se compara mientras se escribe.
    info_click(&mut a, InfoHit::Expected);
    let (b, _) = a.info.edit.expect("caja abierta");
    crate::host::WindowHost::text_set(&*host, b, &md5);
    a.on_text_changed(b.0 as isize);
    assert_eq!(a.info.sum.expected, md5);
    a.paint();
    assert!(texts(&host).iter().any(|t| t == "Coincide"));
}

/// Cambiar de archivo o cerrar el panel corta la suma en curso y la olvida.
#[test]
fn info_checksum_cancels_on_change_and_close() {
    let mut a = app(FakeHost::new());
    let d = fixture("info-cortar", &[("a.bin", b"12345678"), ("b.bin", b"x")]);
    FakeNetwork::set_remote(&d);
    FakeNetwork::hang(&d.to_string_lossy(), Duration::from_secs(2));
    go(&mut a, 0, Location::Path(d.clone()));
    select(&mut a, 0, &["a.bin"]);
    a.info.sections |= Section::Checksum.bit();
    open_info(&mut a);
    a.start_checksum(HashAlgo::Sha256);
    let flag = a.info.sum.job.as_ref().unwrap().1.cancel.clone();
    select(&mut a, 0, &["b.bin"]);
    a.paint();
    assert!(flag.load(std::sync::atomic::Ordering::Relaxed), "se cortó al cambiar de archivo");
    assert!(a.info.sum.job.is_none() && a.info.sum.results.is_empty());
    a.flush_info();
    a.on_app_message(crate::info::WM_APP_INFO, 0, 0);
    a.paint();
    a.start_checksum(HashAlgo::Md5);
    let flag = a.info.sum.job.as_ref().unwrap().1.cancel.clone();
    a.run_command(Command::Properties);
    assert!(!a.info_open());
    assert!(flag.load(std::sync::atomic::Ordering::Relaxed), "se cortó al cerrar");
}

/// Dentro de un comprimido: el panel muestra el elemento y su suma (extraído a la temporal).
#[test]
fn info_checksum_inside_an_archive() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("info-zip", &[("fotos.zip", b"zip"), ("a.txt", b"hola")]);
    let zip = d.join("fotos.zip");
    FakeArchives::shared().add_archive(&zip, &[("docs/a.txt", b"hola")], 40);
    go(&mut a, 0, Location::Archive { file: zip, nested: Vec::new(), inner: "docs".into() });
    select(&mut a, 0, &["a.txt"]);
    a.info.sections |= Section::Checksum.bit();
    open_info(&mut a);
    assert!(host.drawn_texts().iter().any(|t| t.1 == "Suma de verificación"));
    info_click(&mut a, InfoHit::HashRun);
    wait_sum(&mut a);
    assert_eq!(mono_text(&host), sum_of(&d.join("a.txt"), HashAlgo::Sha256));
}

// ---------- Expulsar ----------

/// Expulsar el disco con una comparación (o una suma del panel de información) leyendo de ahí: se
/// cierra el panel, se cortan los trabajos y se espera a que suelten los archivos antes de expulsar.
#[test]
fn eject_stops_compare_and_checksum_and_waits() {
    use crate::sys::testing::{FakeFileSystem, FakeHashes, FakeShell};
    for info in [false, true] {
        let mut a = app(FakeHost::new());
        let big = vec![4u8; 3 << 20];
        let root = fixture(if info { "expulsa-suma" } else { "expulsa-comparar" }, &[("a.bin", &big), ("b.bin", &big)]);
        FakeFileSystem::set_drives(&[(&root, "PENDRIVE")]);
        a.ejectable.insert(root.clone());
        FakeShell::set_eject_result(&root, Ok(()));
        // Cada lectura tarda: el trabajo queda leyendo cuando se expulsa.
        FakeNetwork::hang(&root.to_string_lossy(), Duration::from_millis(300));
        go(&mut a, 0, Location::Path(root.clone()));
        a.set_active(0);
        if info {
            select(&mut a, 0, &["a.bin"]);
            open_info(&mut a);
            a.start_checksum(HashAlgo::Sha256);
        } else {
            select(&mut a, 0, &["a.bin", "b.bin"]);
            a.run_command(Command::CompareContents);
        }
        let t0 = Instant::now();
        while FakeHashes::open_readers(&root) == 0 {
            assert!(t0.elapsed() < Duration::from_secs(10), "no empezó a leer");
            std::thread::sleep(Duration::from_millis(5));
        }
        a.run_command(Command::Eject);
        assert!(a.compare.is_none() && a.info.sum.job.is_none(), "se cortó al soltar el disco");
        while a.notice.is_none() {
            let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("expulsar termina");
            a.on_task_done(t);
        }
        assert_eq!(FakeHashes::open_readers(&root), 0, "se expulsó con archivos abiertos");
        assert_eq!(a.notice.as_deref(), Some("Se puede quitar \"PENDRIVE\" con seguridad"));
        FakeFileSystem::set_drives(&[]);
    }
}

/// Un enlace a un archivo (en Windows la lista muestra el tamaño del enlace, 0): no se responde
/// «tamaños distintos» con la lista; se mira el destino en el hilo.
#[test]
fn link_size_is_not_trusted() {
    let mut a = app(FakeHost::new());
    let d = fixture("enlace", &[("a.bin", b"hola mundo"), ("b.bin", b"hola mundo")]);
    go(&mut a, 0, Location::Path(d));
    let list = &mut a.panes[0].tab_mut().list;
    let i = list.entries.iter().position(|e| e.name == "b.bin").unwrap();
    list.entries[i].attrs |= crate::fs::ATTR_LINK;
    list.entries[i].size = 0;
    select(&mut a, 0, &["a.bin", "b.bin"]);
    a.run_command(Command::CompareContents);
    assert_eq!(wait_done(&mut a), Status::Done(Verdict::Same));
}

/// Escribir la suma a mano no cambia de algoritmo ni corta la suma en curso a mitad de camino (a
/// los 32 caracteres parecería una MD5): se decide al pegar, con Enter o al salir de la caja.
#[test]
fn typing_does_not_switch_algorithm_midway() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("tipeando", &[("a.bin", &[5u8; 64])]);
    FakeNetwork::hang(&d.to_string_lossy(), Duration::from_millis(200));
    go(&mut a, 0, Location::Path(d.clone()));
    select(&mut a, 0, &["a.bin"]);
    a.info.sections |= Section::Checksum.bit();
    open_info(&mut a);
    a.start_checksum(HashAlgo::Sha256);
    let flag = a.info.sum.job.as_ref().unwrap().1.cancel.clone();
    info_click(&mut a, InfoHit::Expected);
    let (b, _) = a.info.edit.expect("caja abierta");
    let sha = "ab".repeat(32);
    for n in 1..=40 {
        crate::host::WindowHost::text_set(&*host, b, &sha[..n]);
        a.on_text_changed(b.0 as isize);
    }
    assert_eq!(a.info.sum.algo, HashAlgo::Sha256);
    assert!(!flag.load(std::sync::atomic::Ordering::Relaxed), "la SHA-256 sigue");
    assert_eq!(a.info.sum.expected, sha[..40]);
    // Pegar una MD5 entera (de golpe) sí decide.
    crate::host::WindowHost::text_set(&*host, b, "");
    a.on_text_changed(b.0 as isize);
    crate::host::WindowHost::text_set(&*host, b, &"cd".repeat(16));
    a.on_text_changed(b.0 as isize);
    assert_eq!(a.info.sum.algo, HashAlgo::Md5);
}

/// Si no se puede crear el hilo, la suma (o la comparación) responde con el error que le toca y
/// no queda «calculando» para siempre.
#[test]
fn spawn_failure_answers_the_right_job() {
    let mut a = app(FakeHost::new());
    let d = fixture("sin-hilo", &[("a.bin", b"x"), ("b.bin", b"x")]);
    go(&mut a, 0, Location::Path(d));
    select(&mut a, 0, &["a.bin", "b.bin"]);
    a.jobs.fail_spawn = true;
    a.run_command(Command::CompareContents);
    let before = crate::sys::testing::FakeWaker::count(super::compare::WM_APP_COMPARE);
    a.compare_hash(0);
    assert!(crate::sys::testing::FakeWaker::count(super::compare::WM_APP_COMPARE) > before, "avisa a la ventana");
    a.on_compare_message(0);
    let st = a.compare.as_ref().unwrap();
    assert!(matches!(&st.status, Status::Failed(m) if m.contains("hilo")), "{:?}", st.status);
    assert!(st.hashes[0].job.is_none());
    assert!(matches!(&st.hashes[0].result, Some(Err(m)) if m.contains("hilo")), "{:?}", st.hashes[0].result);
}

/// En una ventana baja el panel se desplaza con la rueda; el avance no redibuja con el panel
/// oculto (xtree encima).
#[test]
fn panel_scrolls_and_no_ticks_while_hidden() {
    let host = FakeHost::new();
    let mut a = app(host.clone());
    let d = fixture("rueda", &[("a.txt", b"hola"), ("b.txt", b"hola")]);
    go(&mut a, 0, Location::Path(d.clone()));
    select(&mut a, 0, &["a.txt", "b.txt"]);
    a.run_command(Command::CompareContents);
    wait_done(&mut a);
    host.client.set((1280.0, 330.0));
    a.paint();
    let max = a.compare.as_ref().unwrap().layout.max_scroll();
    assert!(max > 0.0, "no entra en una ventana baja");
    let r = a.compare.as_ref().unwrap().layout.panel;
    a.on_wheel((r.left + r.right) / 2.0, r.top + 100.0, -50.0, 1.0);
    a.paint();
    assert_eq!(a.compare.as_ref().unwrap().scroll, max);
    // Calculando a la vista, el avance redibuja; oculto (xtree encima), no.
    host.client.set((1280.0, 800.0));
    FakeNetwork::hang(&d.to_string_lossy(), Duration::from_millis(300));
    a.compare_hash(0);
    a.paint();
    let before = host.invalidations();
    a.on_compare_message(1);
    assert!(host.invalidations() > before);
    a.toggle_xtree();
    let before = host.invalidations();
    a.on_compare_message(1);
    assert_eq!(host.invalidations(), before);
}

/// Prepara un «disco» expulsable con dos archivos grandes que tardan `hang` en cada lectura.
fn ejectable_disk(a: &mut App, name: &str, hang: Duration) -> PathBuf {
    use crate::sys::testing::{FakeFileSystem, FakeShell};
    let big = vec![4u8; 3 << 20];
    let root = fixture(name, &[("a.bin", &big), ("b.bin", &big)]);
    FakeFileSystem::set_drives(&[(&root, "PENDRIVE")]);
    a.ejectable.insert(root.clone());
    FakeShell::set_eject_result(&root, Ok(()));
    // Solo las lecturas del archivo tardan (no la lista ni el resto del disco falso).
    crate::sys::testing::FakeHashes::throttle(&root, 1 << 20, hang);
    root
}

fn eject_and_wait(a: &mut App) {
    a.run_command(Command::Eject);
    while a.notice.is_none() {
        let t = a.task_rx.recv_timeout(Duration::from_secs(10)).expect("expulsar termina");
        a.on_task_done(t);
    }
}

/// Un lector trabado (un pendrive que muere) no deja la expulsión esperando para siempre: se espera
/// como mucho el límite y se expulsa igual (el error de «en uso», si lo hay, lo da el sistema).
#[test]
fn eject_waits_at_most_the_limit() {
    use crate::sys::testing::FakeHashes;
    let mut a = app(FakeHost::new());
    let root = ejectable_disk(&mut a, "expulsa-trabado", Duration::from_secs(3));
    a.jobs.limit = Duration::from_millis(200);
    go(&mut a, 0, Location::Path(root.clone()));
    a.set_active(0);
    select(&mut a, 0, &["a.bin", "b.bin"]);
    a.run_command(Command::CompareContents);
    let t0 = Instant::now();
    while FakeHashes::open_readers(&root) == 0 {
        assert!(t0.elapsed() < Duration::from_secs(10), "no empezó a leer");
        std::thread::sleep(Duration::from_millis(5));
    }
    let t0 = Instant::now();
    eject_and_wait(&mut a);
    assert!(t0.elapsed() < Duration::from_millis(2500), "esperó al lector trabado: {:?}", t0.elapsed());
    crate::sys::testing::FakeFileSystem::set_drives(&[]);
}

/// Una comparación de un archivo del pendrive con uno de la red también se espera al expulsar el
/// pendrive (antes, por tocar la red, no se esperaba).
#[test]
fn eject_waits_for_a_job_that_also_touches_the_network() {
    use crate::sys::testing::FakeHashes;
    let mut a = app(FakeHost::new());
    let root = ejectable_disk(&mut a, "expulsa-con-red", Duration::from_millis(300));
    let net = fixture("expulsa-red-otro", &[("b.bin", &vec![4u8; 3 << 20])]);
    FakeNetwork::set_remote(&net);
    go(&mut a, 0, Location::Path(root.clone()));
    go(&mut a, 1, Location::Path(net));
    select(&mut a, 0, &["a.bin"]);
    select(&mut a, 1, &["b.bin"]);
    a.set_active(0);
    a.run_command(Command::CompareContents);
    let t0 = Instant::now();
    while FakeHashes::open_readers(&root) == 0 {
        assert!(t0.elapsed() < Duration::from_secs(10), "no empezó a leer");
        std::thread::sleep(Duration::from_millis(5));
    }
    eject_and_wait(&mut a);
    assert_eq!(FakeHashes::open_readers(&root), 0, "se expulsó con el archivo abierto");
    crate::sys::testing::FakeFileSystem::set_drives(&[]);
}

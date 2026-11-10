//! Comprimir y extraer en la `App`: menú, paleta, trabajos y avisos. La lista se lee de carpetas
//! temporales reales; los trabajos corren en el disco falso de todo el proceso (`FakeCopyPrims`
//! y `FakeArchives` compartidos), con las mismas rutas.

use super::*;
use super::fileops::archive_menu;
use crate::archive::ArchiveFormat;
use crate::sys::testing::{FakeArchives, FakeCopyPrims, FakeHost, FakeShell, ShellCall};
use crate::transfer::JobSpec;
use std::path::Path;
use std::time::Duration;

fn app(host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    let mut a = App::new(Init::blank(), host);
    a.set_layout(Layout::One);
    a
}

/// Carpeta real con esos archivos (para la lista) y los mismos en el disco falso (para el motor).
fn fixture(name: &str, files: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-zip").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::write(dir.join(f), f.as_bytes()).unwrap();
        FakeCopyPrims::shared().add_file(&dir.join(f), f.as_bytes().to_vec());
    }
    dir
}

fn go(a: &mut App, dir: &Path) {
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.to_path_buf()));
    while a.panes[0].tab().loading {
        let ev = a.rx.recv_timeout(Duration::from_secs(10)).expect("la carga termina");
        a.on_load_event(ev);
    }
}

fn select(a: &mut App, names: &[&str]) {
    let list = &mut a.panes[0].tab_mut().list;
    for p in 0..list.count() {
        list.set_selected_pos(p, false);
    }
    for n in names {
        let pos = (0..list.count()).find(|&p| list.entries[list.entry_of_pos(p).unwrap()].name == *n).expect(n);
        list.set_selected_pos(pos, true);
    }
}

fn labels(a: &App) -> Vec<String> {
    let mut cmds = Vec::new();
    a.item_entries(&mut cmds)
        .into_iter()
        .filter_map(|e| match e {
            shell::Entry::Item { label, .. } => Some(label),
            _ => None,
        })
        .collect()
}

fn wait_idle(a: &mut App) {
    for _ in 0..500 {
        a.on_transfer_updates();
        if a.running_jobs() == 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("los trabajos no terminaron");
}

#[test]
fn menu_extract_shows_real_name() {
    let mut a = app(FakeHost::new());
    let dir = fixture("menu", &["fotos.zip", "nota.txt", "b.tar.gz"]);
    go(&mut a, &dir);
    select(&mut a, &["fotos.zip"]);
    let l = labels(&a);
    assert!(l.iter().any(|l| l == "Comprimir"), "{l:?}");
    assert!(l.iter().any(|l| l == "Extraer aquí"), "{l:?}");
    assert!(l.iter().any(|l| l == "Extraer en \"fotos\""), "{l:?}");
    select(&mut a, &["nota.txt"]);
    let l = labels(&a);
    assert!(l.iter().any(|l| l == "Comprimir") && !l.iter().any(|l| l.starts_with("Extraer")), "{l:?}");
    // Con varios: solo comprimir.
    select(&mut a, &["fotos.zip", "nota.txt"]);
    let l = labels(&a);
    assert!(l.iter().any(|l| l == "Comprimir") && !l.iter().any(|l| l.starts_with("Extraer")), "{l:?}");
}

#[test]
fn targz_not_offered_on_windows_style() {
    let zip_only = [ArchiveFormat::Zip];
    let both = [ArchiveFormat::Zip, ArchiveFormat::TarGz];
    assert!(archive_menu("b.tar.gz", false, &zip_only).is_empty());
    assert_eq!(
        archive_menu("b.tar.gz", false, &both),
        vec![("Extraer aquí".to_string(), Command::ExtractHere), ("Extraer en \"b\"".to_string(), Command::ExtractInto)]
    );
    assert_eq!(archive_menu("Fotos.ZIP", false, &zip_only).len(), 2);
    // Una carpeta que se llama «x.zip» no se extrae.
    assert!(archive_menu("x.zip", true, &both).is_empty());
}

#[test]
fn commands_start_compress_and_extract_jobs() {
    let mut a = app(FakeHost::new());
    let dir = fixture("cmds", &["a.txt", "b.txt", "fotos.zip"]);
    go(&mut a, &dir);
    select(&mut a, &["a.txt", "b.txt"]);
    a.take_started();
    a.run_command(Command::Compress);
    select(&mut a, &["fotos.zip"]);
    a.run_command(Command::ExtractHere);
    a.run_command(Command::ExtractInto);
    // Extraer pide un solo archivo de formato conocido.
    select(&mut a, &["a.txt"]);
    a.run_command(Command::ExtractHere);
    let mut started = a.take_started();
    // El orden de la selección es el de la lista.
    if let Some(JobSpec::Compress { sources }) = started.first_mut() {
        sources.sort();
    }
    assert_eq!(
        started,
        vec![
            JobSpec::Compress { sources: vec![dir.join("a.txt"), dir.join("b.txt")] },
            JobSpec::Extract { archive: dir.join("fotos.zip"), into_folder: false },
            JobSpec::Extract { archive: dir.join("fotos.zip"), into_folder: true },
        ]
    );
    wait_idle(&mut a);
}

#[test]
fn compress_job_selects_zip_and_undo_is_one_step() {
    let mut a = app(FakeHost::new());
    let dir = fixture("comprimir", &["informe.pdf"]);
    go(&mut a, &dir);
    select(&mut a, &["informe.pdf"]);
    a.run_command(Command::Compress);
    wait_idle(&mut a);
    let zip = dir.join("informe.pdf.zip");
    assert!(FakeArchives::shared().created(&zip).is_some());
    assert!(a.jobs().is_empty());
    assert_eq!(a.undo.len(), 1);
}

#[test]
fn password_zip_offers_system_open() {
    let host = FakeHost::new();
    host.set_answer(Some(true));
    let mut a = app(host.clone());
    let z = PathBuf::from("/app-zip-pw/c.zip");
    FakeArchives::shared().add_archive(&z, &[("s.txt", b"s")], 5);
    FakeArchives::shared().set_password(&z);
    FakeShell::take_calls();
    a.start_transfer(JobSpec::Extract { archive: z.clone(), into_folder: true });
    wait_idle(&mut a);
    assert!(host.questions().iter().any(|(t, q)| t == "Abrir con el sistema" && q == "«c.zip»: Este zip tiene contraseña. ¿Abrir con el sistema?"), "{:?}", host.questions());
    assert!(FakeShell::take_calls().contains(&ShellCall::OpenItem(z.clone())));
    // El error queda en la ventana, sin «Reintentar» (sería lo mismo).
    assert_eq!(a.jobs()[0].errors.len(), 1);
    assert!(!a.jobs()[0].errors[0].retry);
}

#[test]
fn extract_asks_before_big_expansion_and_reports_omitted() {
    let host = FakeHost::new();
    host.set_answer(Some(false));
    let mut a = app(host.clone());
    let z = PathBuf::from("/app-zip-big/b.zip");
    FakeArchives::shared().add_archive(&z, &[("a.bin", &[0; 1000])], 5);
    FakeArchives::shared().set_declared(&z, "a.bin", 2 << 30);
    a.start_transfer(JobSpec::Extract { archive: z.clone(), into_folder: true });
    wait_idle(&mut a);
    assert!(host.questions().iter().any(|(t, q)| t == "Extraer" && q.starts_with("Este archivo se expande a ")), "{:?}", host.questions());
    assert!(!FakeCopyPrims::shared().exists(Path::new("/app-zip-big/b")));

    let z = PathBuf::from("/app-zip-om/o.zip");
    FakeArchives::shared().add_archive(&z, &[("a.txt", b"a")], 5);
    FakeArchives::shared().add_link(&z, "duro", None);
    a.start_transfer(JobSpec::Extract { archive: z, into_folder: true });
    wait_idle(&mut a);
    assert_eq!(a.notice.as_deref(), Some("Se omitió 1 elemento que no se puede crear"));
}

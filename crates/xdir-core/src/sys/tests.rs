use std::sync::OnceLock;

use super::testing::{FakeWaker, install_fakes};
use super::*;

/// Número de mensaje que solo usa este test, para que otros tests en paralelo no lo cuenten.
const MSG_MANY: u32 = crate::WM_APP + 0x7A01;

#[test]
fn waker_receives_posts_from_many_threads() {
    install_fakes();
    let before = FakeWaker::count(MSG_MANY);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                for i in 0..100 {
                    Handle(1).post(MSG_MANY, i, 0);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(FakeWaker::count(MSG_MANY) - before, 800);
}

#[test]
fn install_fakes_is_idempotent() {
    install_fakes();
    install_fakes();
    let _ = waker();
    let _ = env();
    let _ = file_ops();
}

#[test]
fn posting_to_null_handle_is_harmless() {
    install_fakes();
    assert!(Handle(0).is_null());
    assert!(!Handle(1).is_null());
    Handle(0).post(crate::WM_APP + 0x7A02, 0, 0);
}

#[test]
#[should_panic(expected = "servicios del sistema no instalados")]
fn missing_install_panics_with_message() {
    let empty: OnceLock<Services> = OnceLock::new();
    let _ = get_or_panic(&empty);
}

#[test]
fn fake_config_dir_is_per_thread() {
    install_fakes();
    let here = env().config_dir().expect("carpeta del falso");
    assert_eq!(env().config_dir(), Some(here.clone()), "el mismo hilo siempre ve la misma carpeta");
    let other = std::thread::spawn(|| {
        install_fakes();
        env().config_dir().expect("carpeta del falso")
    })
    .join()
    .unwrap();
    assert_ne!(here, other, "cada hilo tiene su carpeta");
    // Igual que el `config_dir` de pruebas de antes: temp/xdir-tests/<pid>-<id del hilo>.
    let name = format!("{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], "");
    assert_eq!(here, std::env::temp_dir().join("xdir-tests").join(name));
}

#[test]
fn fake_formats_are_deterministic() {
    install_fakes();
    let e = env();
    assert_eq!(e.format_date(0), "");
    assert_eq!(e.format_date(116_444_736_000_000_000), "1970-01-01");
    assert_eq!(e.format_date(133_000_000_000_000_000), "2022-06-18");
    assert_eq!(e.format_size_kb(0), "0 KB");
    assert_eq!(e.format_size_kb(1), "1 KB");
    assert_eq!(e.format_size_kb(1_500_000), "1.465 KB");
    assert_eq!(e.format_size_short(1023), "1023 bytes");
    assert_eq!(e.format_size_short(1_234_567_890), "1,14 GB");
    assert_eq!(e.format_size_short(12_900_000), "12,3 MB");
    assert_eq!(e.format_size_short(476 * 1024 * 1024 * 1024), "476 GB");
}

/// El `FakeShell` anota por hilo y responde lo configurado: lo copiado vuelve del portapapeles,
/// los errores se devuelven como texto y el menú elige lo pedido.
#[test]
fn fake_shell_records_per_thread_and_answers() {
    use super::testing::{FakeShell, ShellCall};
    use crate::fs::Location;
    use crate::shell::Item;
    install_fakes();
    FakeShell::take_calls();
    let a = PathBuf::from("/x/a.txt");
    shell().copy_files(&[Item::from(a.clone())], true).unwrap();
    assert_eq!(shell().clipboard_files(), Some((vec![a.clone()], true)));
    FakeShell::fail_with(Some("sin permiso"));
    assert_eq!(shell().copy_text(Handle(1), "hola"), Err("sin permiso".to_string()));
    FakeShell::fail_with(None);
    FakeShell::choose(Some("own:3"));
    let chosen = shell().context_menu(Handle(1), &Location::Computer, &[], (1.0, 2.0), false, true, &[], false);
    assert_eq!(chosen.as_deref(), Some("own:3"));
    FakeShell::choose(None);
    let calls = FakeShell::take_calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], ShellCall::CopyFiles { paths: vec![a], cut: true });
    assert_eq!(calls[1], ShellCall::CopyText("hola".into()));
    // Otro hilo no ve lo anotado en este.
    std::thread::spawn(|| assert!(FakeShell::take_calls().is_empty())).join().unwrap();
    assert_eq!(shell().volume_of(std::path::Path::new("/x/a")), shell().volume_of(std::path::Path::new("/y")));
}

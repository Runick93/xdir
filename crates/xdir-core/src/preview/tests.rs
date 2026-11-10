use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, channel};

use super::*;
use crate::sys::Handle;
use crate::sys::testing::{FakeDiskScan, install_fakes};

/// PNG real de 3×2.
pub(crate) const PNG_3X2: [u8; 73] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x41, 0xd2, 0x05, 0xfb, 0x87, 0xf0, 0xb9,
    0x48, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xdir-preview-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Pedido vigente (generación 5) para `target` y el canal donde llega la respuesta.
fn request(target: PreviewTarget) -> (PreviewRequest, Receiver<PreviewReply>) {
    let (tx, rx) = channel();
    let req = PreviewRequest { hwnd: Handle(0), tx, generation: 5, current: Arc::new(AtomicU64::new(5)), target };
    (req, rx)
}

/// Responde `target` en este hilo con el chequeo de nube `cloud` y devuelve lo que llegó.
fn answer_of(target: PreviewTarget, cloud: &dyn Fn(&Path) -> bool) -> PreviewInfo {
    install_fakes();
    let (req, rx) = request(target);
    answer(req, cloud);
    let reply = rx.try_recv().expect("respuesta");
    assert_eq!(reply.generation, 5);
    reply.info
}

fn local(_: &Path) -> bool {
    false
}

#[test]
fn preview_text_file() {
    let dir = temp_dir("texto");
    let file = dir.join("notas.txt");
    let line = format!("{}\n", "x".repeat(999));
    let text = line.repeat(70);
    assert_eq!(text.len(), 70_000);
    std::fs::write(&file, &text).unwrap();
    let info = answer_of(PreviewTarget::File(file), &local);
    let PreviewBody::Text { lines } = info.body else { panic!("se esperaba texto: {info:?}") };
    let want: Vec<&str> = text[..TEXT_MAX].split('\n').collect();
    assert_eq!(lines.len(), want.len());
    assert_eq!(lines, want);
    assert_eq!(lines.last().unwrap().len(), TEXT_MAX % 1000);
    assert!(info.accessed > 0, "último acceso como FILETIME");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_text_lines_capped() {
    let dir = temp_dir("lineas");
    let file = dir.join("muchas.log");
    std::fs::write(&file, "a\n".repeat(TEXT_LINES * 2)).unwrap();
    let PreviewBody::Text { lines } = answer_of(PreviewTarget::File(file), &local).body else { panic!("texto") };
    assert_eq!(lines.len(), TEXT_LINES);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_sniffs_unknown_ext() {
    let dir = temp_dir("olfato");
    let license = dir.join("LICENSE");
    std::fs::write(&license, "MIT License\r\n\r\nCopyright").unwrap();
    let body = answer_of(PreviewTarget::File(license), &local).body;
    assert_eq!(body, PreviewBody::Text { lines: vec!["MIT License".into(), String::new(), "Copyright".into()] });
    let png = dir.join("foto.png");
    std::fs::write(&png, PNG_3X2).unwrap();
    assert_eq!(answer_of(PreviewTarget::File(png), &local).body, PreviewBody::Image);
    // Un archivo vacío no tiene vista previa (el ícono del tipo).
    let empty = dir.join("vacio.txt");
    std::fs::write(&empty, "").unwrap();
    assert_eq!(answer_of(PreviewTarget::File(empty), &local).body, PreviewBody::Nothing);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_folder_counts_items() {
    let dir = temp_dir("carpeta");
    for name in ["a.txt", "b.txt", "c.png"] {
        std::fs::write(dir.join(name), "x").unwrap();
    }
    std::fs::create_dir(dir.join("sub")).unwrap();
    let info = answer_of(PreviewTarget::Folder(dir.clone()), &local);
    assert_eq!(info.body, PreviewBody::Folder { items: 4 });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_drive_space() {
    install_fakes();
    FakeDiskScan::set_volume(Some((100, 40)), true);
    let info = answer_of(PreviewTarget::Drive(std::env::temp_dir()), &local);
    FakeDiskScan::set_volume(None, false);
    assert_eq!(info.body, PreviewBody::Drive { total: 100, free: 40 });
}

#[test]
fn preview_cloud_only_not_read() {
    let dir = temp_dir("nube");
    let missing = dir.join("en-la-nube.docx");
    let asked = std::cell::Cell::new(0);
    let cloud = |p: &Path| {
        asked.set(asked.get() + 1);
        assert_eq!(p, missing.as_path());
        true
    };
    let info = answer_of(PreviewTarget::File(missing.clone()), &cloud);
    assert_eq!(info, PreviewInfo { body: PreviewBody::CloudOnly, accessed: 0 });
    assert!(asked.get() > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_unreadable() {
    let dir = temp_dir("ilegible");
    let info = answer_of(PreviewTarget::File(dir.join("no-existe.txt")), &local);
    assert_eq!(info, PreviewInfo { body: PreviewBody::Unreadable, accessed: 0 });
    let info = answer_of(PreviewTarget::Folder(dir.join("no-existe")), &local);
    assert_eq!(info.body, PreviewBody::Unreadable);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_stale_request_not_sent() {
    install_fakes();
    let dir = temp_dir("viejo");
    let file = dir.join("a.txt");
    std::fs::write(&file, "hola").unwrap();
    let (tx, rx) = channel();
    let current = Arc::new(AtomicU64::new(6));
    let req = PreviewRequest { hwnd: Handle(0), tx, generation: 5, current, target: PreviewTarget::File(file) };
    assert!(req.is_stale());
    answer(req, &local);
    assert!(rx.try_recv().is_err(), "un pedido viejo no responde");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Si la lectura entra en pánico, el pedido igual se responde (ilegible) y el hilo sigue.
#[test]
fn preview_panic_answers_unreadable() {
    let info = answer_of(PreviewTarget::File(PathBuf::from("x")), &|_| panic!("chequeo roto"));
    assert_eq!(info.body, PreviewBody::Unreadable);
}

/// El falso responde lo cargado con `serve`, si no lee de verdad (con su nube), anota los
/// pedidos y, con demora, no responde un pedido que quedó viejo.
#[test]
fn fake_preview_serves_reads_and_delays() {
    use crate::sys::testing::{FakeFileSystem, FakePreview};
    install_fakes();
    let dir = temp_dir("falso");
    let file = dir.join("a.txt");
    std::fs::write(&file, "hola").unwrap();
    let served = PreviewInfo { body: PreviewBody::Folder { items: 9 }, accessed: 1 };
    FakePreview::serve(&dir.join("servido"), served.clone());

    let (req, rx) = request(PreviewTarget::File(dir.join("servido")));
    crate::sys::preview().request(req);
    assert_eq!(rx.try_recv().unwrap().info, served);

    let (req, rx) = request(PreviewTarget::File(file.clone()));
    crate::sys::preview().request(req);
    assert_eq!(rx.try_recv().unwrap().info.body, PreviewBody::Text { lines: vec!["hola".into()] });

    FakeFileSystem::set_cloud_only(&file, true);
    let (req, rx) = request(PreviewTarget::File(file.clone()));
    crate::sys::preview().request(req);
    assert_eq!(rx.try_recv().unwrap().info.body, PreviewBody::CloudOnly);
    FakeFileSystem::set_cloud_only(&file, false);

    FakePreview::set_delay(Some(std::time::Duration::from_millis(50)));
    let (req, rx) = request(PreviewTarget::File(file.clone()));
    let current = req.current.clone();
    crate::sys::preview().request(req);
    assert!(rx.try_recv().is_err(), "con demora no responde enseguida");
    current.store(6, std::sync::atomic::Ordering::Relaxed);
    assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_err(), "el pedido quedó viejo");
    FakePreview::set_delay(None);

    let asked = FakePreview::take_requests();
    assert_eq!(asked.len(), 4);
    assert_eq!(asked[1], PreviewTarget::File(file));
    assert!(FakePreview::take_requests().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

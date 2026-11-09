use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use xdir_core::fs::{LoadMsg, Location};
use xdir_core::sys::testing::{FakeWaker, install_fakes};
use xdir_core::sys::{FileSystem, Handle};

use super::LinuxFileSystem;
use super::places::{mounts_from, user_dirs_from};

/// Carpeta temporal propia del test; se borra al soltarla.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("xdir-linux-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Lee `dir` con `spawn_load` y devuelve los nombres listados y el resultado final.
fn load(dir: &Path) -> (Vec<OsString>, Result<(), String>) {
    install_fakes();
    let (tx, rx) = channel();
    LinuxFileSystem.spawn_load(Handle(1), tx, 0, 7, Arc::new(AtomicU64::new(7)), Location::Path(dir.to_path_buf()));
    let mut names = Vec::new();
    loop {
        let ev = rx.recv_timeout(Duration::from_secs(5)).expect("la carga no terminó");
        assert_eq!((ev.panel, ev.generation), (0, 7));
        match ev.msg {
            LoadMsg::Entries(batch) => names.extend(batch.into_iter().map(|e| e.name)),
            LoadMsg::Listed(r) => return (names, r),
            _ => {}
        }
    }
}

#[test]
fn load_lists_files_and_folders() {
    let t = TempDir::new("load");
    std::fs::write(t.0.join("a.txt"), b"hola").unwrap();
    std::fs::write(t.0.join("b.txt"), b"").unwrap();
    std::fs::create_dir(t.0.join("sub")).unwrap();
    let (mut names, done) = load(&t.0);
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt", "sub"].map(OsString::from));
    assert_eq!(done, Ok(()));
}

#[test]
fn load_skips_nothing_with_non_utf8_name() {
    let t = TempDir::new("utf8");
    let raro = OsStr::from_bytes(b"a\xFFb");
    std::fs::write(t.0.join(raro), b"").unwrap();
    let (names, done) = load(&t.0);
    assert_eq!(names, [raro.to_os_string()]);
    assert_eq!(done, Ok(()));
}

#[test]
fn dotfiles_are_hidden() {
    let t = TempDir::new("hidden");
    let p = t.0.join(".oculto");
    std::fs::write(&p, b"").unwrap();
    let meta = std::fs::metadata(&p).unwrap();
    let fs = LinuxFileSystem;
    assert!(fs.is_hidden(&meta, OsStr::new(".oculto")));
    assert!(!fs.is_hidden(&meta, OsStr::new("visible")));
    let e = fs.disk_entry(&p, OsString::from(".oculto"), &meta);
    assert!(e.is_hidden());
    assert_eq!(e.full.as_deref(), Some(p.as_os_str()));
}

#[test]
fn mounts_from_keeps_media_and_mnt() {
    let text = "\
sysfs /sys sysfs rw,nosuid 0 0
proc /proc proc rw,nosuid 0 0
/dev/sda1 / ext4 rw,relatime 0 0
/dev/sdb1 /media/usb vfat rw 0 0
/dev/sdc1 /mnt/datos ext4 rw 0 0
tmpfs /run tmpfs rw 0 0
";
    assert_eq!(mounts_from(text), [PathBuf::from("/media/usb"), PathBuf::from("/mnt/datos")]);
}

#[test]
fn user_dirs_from_reads_xdg_file() {
    let text = "# comentario\nXDG_DOWNLOAD_DIR=\"$HOME/Descargas\"\n";
    let dirs = user_dirs_from(text, Path::new("/home/a"));
    assert_eq!(dirs, [("DOWNLOAD".to_string(), PathBuf::from("/home/a/Descargas"))]);
}

#[test]
fn watch_reports_new_file() {
    install_fakes();
    // Número de mensaje propio de este test: el contador del `FakeWaker` es de todo el proceso.
    const MSG: u32 = 0xB0B0;
    let t = TempDir::new("watch");
    let before = FakeWaker::count(MSG);
    let guard = LinuxFileSystem.watch(Handle(1), MSG, 0, 0, &t.0).expect("no se pudo vigilar");
    std::fs::write(t.0.join("nuevo.txt"), b"x").unwrap();
    let start = Instant::now();
    while FakeWaker::count(MSG) == before && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(guard);
    assert!(FakeWaker::count(MSG) > before, "no llegó el aviso del cambio");
}

#[test]
fn watch_ignores_opening_the_folder() {
    install_fakes();
    // Número de mensaje propio de este test (el contador del `FakeWaker` es de todo el proceso).
    const MSG: u32 = 0xB0B1;
    let t = TempDir::new("watch-abrir");
    std::fs::write(t.0.join("a.txt"), b"x").unwrap();
    let guard = LinuxFileSystem.watch(Handle(1), MSG, 0, 0, &t.0).expect("no se pudo vigilar");
    // Que pase cualquier aviso del arranque antes de contar.
    std::thread::sleep(Duration::from_millis(600));
    let before = FakeWaker::count(MSG);
    // Lo que hace una recarga: leer la carpeta (y abrir un archivo de adentro).
    let _ = std::fs::read_dir(&t.0).unwrap().count();
    let _ = std::fs::read(t.0.join("a.txt")).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    let after = FakeWaker::count(MSG);
    drop(guard);
    assert_eq!(after, before, "leer la carpeta no es un cambio");
}

#[test]
fn same_path_does_not_touch_disk() {
    let fs = LinuxFileSystem;
    assert!(fs.same_path(Path::new("/no/existe/a"), Path::new("/no/existe/a/")));
    assert!(!fs.same_path(Path::new("/no/existe/a"), Path::new("/no/existe/A")));
}

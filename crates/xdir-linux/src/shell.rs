//! Integración con el escritorio de Linux (`xdir_core::sys::Shell`): abrir con `xdg-open` y el
//! volumen de una ruta. Lo que necesita una interfaz (portapapeles, menú contextual, diálogos,
//! propiedades, Acceso rápido) todavía no está: devuelve `None` / `false` / "No disponible en
//! Linux", sin entrar en pánico.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use xdir_core::fs::Location;
use xdir_core::shell::{Entry, Item};
use xdir_core::sys::{Handle, Shell};

use crate::fs::places;

/// Lo que Linux todavía no puede hacer.
pub const UNAVAILABLE: &str = "No disponible en Linux";

/// Abre `path` con el programa asociado (`xdg-open`), en un proceso aparte. Un hilo lo espera para
/// que no quede zombi al terminar. Si no se puede lanzar no pasa nada (como `open` en macOS).
fn xdg_open(path: &Path, dir: Option<&Path>) {
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(path);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    if let Ok(mut c) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}

/// Punto de montaje de `drives` que contiene a `path` (el más largo).
pub fn volume_in(path: &Path, drives: &[PathBuf]) -> Option<OsString> {
    drives
        .iter()
        .filter(|d| path.starts_with(d))
        .max_by_key(|d| d.components().count())
        .map(|d| d.as_os_str().to_os_string())
}

/// (dispositivo, punto de montaje) de cada línea de `/proc/self/mounts` (`\040` → espacio).
fn mounts(text: &str) -> Vec<(String, PathBuf)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), PathBuf::from(it.next()?.replace("\\040", " "))))
        })
        .collect()
}

/// Disco de bloque de un dispositivo de partición: `/dev/sdb1` → `sdb`, `/dev/mmcblk0p1` →
/// `mmcblk0`, `/dev/nvme0n1p2` → `nvme0n1`.
fn block_of(dev: &str) -> Option<String> {
    let name = dev.strip_prefix("/dev/")?;
    if name.starts_with("mmcblk") || name.starts_with("nvme") {
        return Some(name.rsplit_once('p').map_or(name, |(b, _)| b).to_string());
    }
    Some(name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string())
}

/// De `roots`, las expulsables: montadas bajo `/media` o `/run/media`, o de un disco con
/// `removable` = 1 (`removable` lo responde quien llama: `/sys/block/<disco>/removable`).
pub fn ejectable_in(mounts_text: &str, roots: &[PathBuf], removable: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let table = mounts(mounts_text);
    roots
        .iter()
        .filter(|r| r.as_path() != Path::new("/"))
        .filter(|r| {
            r.starts_with("/media")
                || r.starts_with("/run/media")
                || table.iter().any(|(dev, m)| m == *r && block_of(dev).is_some_and(|b| removable(&b)))
        })
        .cloned()
        .collect()
}

/// Programas (`/proc/<pid>/comm`) con su carpeta de trabajo o algún archivo abierto dentro de
/// `mount`, sin repetir. `proc_root` es `/proc` (los tests usan uno falso).
pub fn holders(proc_root: &Path, mount: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let Ok(dir) = std::fs::read_dir(proc_root) else { return names };
    let mut pids: Vec<PathBuf> = dir
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
        .map(|e| e.path())
        .collect();
    pids.sort();
    for pid in pids {
        let inside = |link: &Path| std::fs::read_link(link).is_ok_and(|t| t.starts_with(mount));
        let mut uses = inside(&pid.join("cwd"));
        if !uses && let Ok(fds) = std::fs::read_dir(pid.join("fd")) {
            uses = fds.filter_map(|f| f.ok()).any(|f| inside(&f.path()));
        }
        if uses
            && let Ok(comm) = std::fs::read_to_string(pid.join("comm"))
        {
            let comm = comm.trim().to_string();
            if !comm.is_empty() && !names.contains(&comm) {
                names.push(comm);
            }
        }
    }
    names
}

/// Corre un programa y dice si terminó bien.
fn run_ok(program: &str, args: &[&std::ffi::OsStr]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Expulsa: `udisksctl unmount` + `power-off` del dispositivo; si no, `gio mount -e`; si no,
/// `umount`. Si sigue montado, busca en `/proc` quién lo usa.
pub fn eject(root: &Path) -> Result<(), xdir_core::shell::EjectError> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let dev = mounts(&text).into_iter().find(|(_, m)| m == root).map(|(d, _)| d);
    let ok = match &dev {
        Some(d) if d.starts_with("/dev/") => {
            let d = std::ffi::OsStr::new(d);
            run_ok("udisksctl", &["unmount".as_ref(), "-b".as_ref(), d]) && {
                // Apagar el disco es lo de menos: ya se desmontó.
                let _ = run_ok("udisksctl", &["power-off".as_ref(), "-b".as_ref(), d]);
                true
            }
        }
        _ => false,
    } || run_ok("gio", &["mount".as_ref(), "-e".as_ref(), root.as_os_str()])
        || run_ok("umount", &[root.as_os_str()]);
    if ok {
        return Ok(());
    }
    let users = holders(Path::new("/proc"), root);
    Err(xdir_core::shell::EjectError {
        busy_by: users.first().cloned(),
        busy: !users.is_empty(),
        message: "no se pudo desmontar".into(),
    })
}

/// Integración con el escritorio de Linux (lo que instala `sys::install`).
pub struct LinuxShell;

/// A dónde apunta un enlace simbólico (relativo a su carpeta si no es absoluto).
pub fn link_target(path: &Path) -> Option<PathBuf> {
    let to = std::fs::read_link(path).ok()?;
    Some(if to.is_absolute() { to } else { path.parent()?.join(to) })
}

impl Shell for LinuxShell {
    fn link_style(&self) -> xdir_core::names::LinkStyle {
        xdir_core::names::LinkStyle::Symlink
    }

    fn link_target(&self, path: &Path) -> Option<PathBuf> {
        link_target(path)
    }

    fn ejectable(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let removable = |block: &str| std::fs::read_to_string(format!("/sys/block/{block}/removable")).is_ok_and(|s| s.trim() == "1");
        ejectable_in(&text, roots, &removable)
    }

    fn eject(&self, root: &Path) -> Result<(), xdir_core::shell::EjectError> {
        eject(root)
    }

    fn copy_files(&self, _items: &[Item], _cut: bool) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn clipboard_files(&self) -> Option<(Vec<PathBuf>, bool)> {
        None
    }

    fn paste_virtual(&self, _owner: Handle, _dest: &Location) -> Result<bool, String> {
        Err(UNAVAILABLE.into())
    }

    fn clear_clipboard(&self) {}

    fn copy_text(&self, _owner: Handle, _text: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn pick_folder(&self, _owner: Handle, _title: &str) -> Option<PathBuf> {
        None
    }

    fn open_item(&self, _owner: Handle, item: &Item) {
        xdg_open(&item.path, None);
    }

    fn open_file(&self, _owner: Handle, file: &Path, dir: Option<&Path>) {
        xdg_open(file, dir);
    }

    fn properties(&self, _owner: Handle, _items: &[Item]) {}

    fn context_menu(
        &self,
        _owner: Handle,
        _folder: &Location,
        _items: &[Item],
        _at: (f32, f32),
        _extended: bool,
        _intercept: bool,
        _own: &[Entry],
        _pinned: bool,
    ) -> Option<String> {
        None
    }

    fn invoke_verb(&self, _owner: Handle, _folder: &Location, _items: &[Item], _verb: &str) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn pin_quick_access(&self, _owner: Handle, _path: &Path) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    fn unpin_quick_access(&self, _owner: Handle, _path: &Path) -> Result<(), String> {
        Err(UNAVAILABLE.into())
    }

    /// Sin menús nativos no hay modo que fijar.
    fn set_menu_mode(&self, _dark: Option<bool>) {}

    fn volume_of(&self, path: &Path) -> Option<OsString> {
        volume_in(path, &places::drives())
    }
}

#[cfg(test)]
mod tests {
    /// Un `/proc` falso: el que tiene su carpeta de trabajo en el disco y el que tiene un archivo
    /// abierto ahí aparecen (una vez cada uno); el que no lo usa, no.
    #[test]
    fn holders_finds_cwd_and_fd() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("xdir-linux-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (proc_root, mount) = (dir.join("proc"), dir.join("media/USB"));
        std::fs::create_dir_all(mount.join("sub")).unwrap();
        for (pid, comm) in [("100", "bash"), ("200", "vlc"), ("300", "otro"), ("400", "vlc")] {
            std::fs::create_dir_all(proc_root.join(pid).join("fd")).unwrap();
            std::fs::write(proc_root.join(pid).join("comm"), format!("{comm}\n")).unwrap();
        }
        std::fs::create_dir_all(proc_root.join("self")).unwrap();
        symlink(mount.join("sub"), proc_root.join("100/cwd")).unwrap();
        symlink(mount.join("video.mp4"), proc_root.join("200/fd/3")).unwrap();
        symlink("/tmp", proc_root.join("300/cwd")).unwrap();
        symlink(mount.join("otro.mp4"), proc_root.join("400/fd/7")).unwrap();
        assert_eq!(super::holders(&proc_root, &mount), ["bash", "vlc"]);
        assert!(super::holders(&proc_root, &dir.join("nada")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ejectable_mounts() {
        let mounts = "/dev/sda2 / ext4 rw 0 0\n/dev/sdb1 /mnt/datos ext4 rw 0 0\n/dev/mmcblk0p1 /mnt/sd vfat rw 0 0\n/dev/sdc1 /mnt/fijo ext4 rw 0 0\n";
        let removable = |b: &str| b == "sdb" || b == "mmcblk0";
        let roots: Vec<PathBuf> = ["/", "/mnt/datos", "/mnt/sd", "/mnt/fijo", "/media/ana/USB"].iter().map(PathBuf::from).collect();
        assert_eq!(
            super::ejectable_in(mounts, &roots, &removable),
            ["/mnt/datos", "/mnt/sd", "/media/ana/USB"].iter().map(PathBuf::from).collect::<Vec<_>>()
        );
        assert_eq!(super::block_of("/dev/nvme0n1p2").as_deref(), Some("nvme0n1"));
    }

    use super::*;

    #[test]
    fn shell_unsupported_returns_error() {
        let sh = LinuxShell;
        let items = [Item::from(PathBuf::from("/tmp/a"))];
        assert_eq!(sh.copy_files(&items, false), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.copy_files(&items, true), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.clipboard_files(), None);
        assert_eq!(sh.pick_folder(Handle(0), "x"), None);
        assert_eq!(sh.paste_virtual(Handle(0), &Location::Computer), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.invoke_verb(Handle(0), &Location::Computer, &items, "delete"), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.pin_quick_access(Handle(0), Path::new("/")), Err(UNAVAILABLE.to_string()));
        assert_eq!(sh.context_menu(Handle(0), &Location::Computer, &items, (0.0, 0.0), false, true, &[], false), None);
    }

    #[test]
    fn volume_is_longest_mount_point() {
        let drives = [PathBuf::from("/"), PathBuf::from("/media/usb"), PathBuf::from("/media/usb2")];
        assert_eq!(volume_in(Path::new("/media/usb/fotos/a.jpg"), &drives), Some("/media/usb".into()));
        assert_eq!(volume_in(Path::new("/media/usb2"), &drives), Some("/media/usb2".into()));
        assert_eq!(volume_in(Path::new("/home/a"), &drives), Some("/".into()));
        assert_eq!(volume_in(Path::new("relativa"), &drives), None);
    }
}

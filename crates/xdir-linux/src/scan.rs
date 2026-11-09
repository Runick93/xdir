//! xtree en Linux (`xdir_core::sys::DiskScan`): listado de carpetas con `read_dir` sin seguir
//! enlaces ni salir del disco de la raíz, los discos montados (de `crate::fs::places`) y su
//! espacio con `statvfs`. No hay escaneo rápido ni escaneo con permisos.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use xdir_core::sys::{DiskScan, Handle};
use xdir_core::xtree::colors::fmt_size;
use xdir_core::xtree::scan::{DirListing, DriveEntry, FastScan, FileEntry, ListError, Lister, Progress};
use xdir_core::xtree::tree::Node;

use crate::fs::places;

/// Aviso de la barra de datos cuando hubo carpetas sin permiso.
pub const PERMISSION_HINT: &str = "Algunas carpetas no se pudieron leer por permisos.";

/// Texto mientras corre `elevate` (que en Linux no hace nada).
pub const ELEVATING_TEXT: &str = "El análisis con permisos no está disponible en Linux.";

/// Listador de carpetas de xtree: no sigue enlaces simbólicos ni entra en otros discos.
pub struct LinuxLister {
    /// Dispositivo (`st_dev`) de la raíz; `None` si no se pudo leer.
    dev: Option<u64>,
}

impl LinuxLister {
    pub fn new(root: &Path) -> LinuxLister {
        LinuxLister { dev: std::fs::metadata(root).ok().map(|m| m.dev()) }
    }

    /// ¿Se recorre ese dispositivo? Sin raíz legible, nada.
    fn allows(&self, dev: u64) -> bool {
        self.dev == Some(dev)
    }
}

impl Lister for LinuxLister {
    /// Carpetas del mismo disco (sin enlaces) y archivos comunes con su tamaño asignado; lo demás
    /// (enlaces, sockets, dispositivos) no se cuenta. Una carpeta sin permiso es `Denied`.
    fn list(&self, path: &Path) -> Result<DirListing, ListError> {
        let rd = std::fs::read_dir(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => ListError::Denied,
            _ => ListError::Other,
        })?;
        let mut out = DirListing::default();
        // Otro disco montado que se haya colado: no se recorre. Se sigue el enlace (`metadata`): la
        // raíz puede ser un enlace a otro disco; las subcarpetas solo se encolan si `lstat` dice que
        // son carpetas de verdad, así que no hay otros enlaces que seguir.
        if !std::fs::metadata(path).is_ok_and(|m| self.allows(m.dev())) {
            return Ok(out);
        }
        for entry in rd.flatten() {
            // `DirEntry::metadata` no sigue enlaces (es `lstat`).
            let Ok(meta) = entry.metadata() else { continue };
            let ft = meta.file_type();
            if ft.is_dir() {
                if self.allows(meta.dev()) {
                    out.dirs.push(entry.file_name());
                }
            } else if ft.is_file() {
                // El id solo hace falta si hay otros enlaces duros al mismo archivo.
                let id = (meta.nlink() > 1).then(|| meta.ino());
                out.files.push(FileEntry { name: entry.file_name().to_string_lossy().into(), size: meta.blocks().saturating_mul(512), id });
            }
        }
        Ok(out)
    }
}

/// El escaneo de disco de Linux (lo que instala `sys::install`).
pub struct LinuxDiskScan;

impl DiskScan for LinuxDiskScan {
    fn lister(&self, root: &Path) -> Box<dyn Lister> {
        Box::new(LinuxLister::new(root))
    }

    /// No hay un índice del volumen que leer de una: siempre se recorre.
    fn fast_scan(&self, _root: &Path, _p: &Progress, _cancel: &AtomicBool) -> FastScan {
        FastScan::NotApplicable
    }

    fn elevate(&self, _owner: Handle, _root: &Path, _p: &Progress, _cancel: &AtomicBool) -> Option<Vec<Node>> {
        None
    }

    fn drives(&self) -> Vec<DriveEntry> {
        places::drives()
            .into_iter()
            .map(|path| {
                let detail = places::volume_size(&path).filter(|(total, _)| *total > 0).map(|(total, _)| fmt_size(total)).unwrap_or_default();
                DriveEntry { label: places::drive_label(&places::drive_name(&path)), path, detail }
            })
            .collect()
    }

    fn volume_space(&self, path: &Path) -> Option<(u64, u64)> {
        places::volume_size(path).filter(|(total, _)| *total > 0)
    }

    /// Un disco entero es uno de sus puntos de montaje.
    fn whole_volume(&self, path: &Path) -> bool {
        places::drives().iter().any(|d| d.components().eq(path.components()))
    }

    /// La carpeta personal; sin `$HOME`, la raíz.
    fn default_root(&self) -> PathBuf {
        places::home().unwrap_or_else(|| PathBuf::from("/"))
    }

    fn permission_issue(&self, p: &Progress) -> bool {
        p.denied.load(Ordering::Relaxed) > 0
    }

    fn permission_hint(&self) -> &'static str {
        PERMISSION_HINT
    }

    fn elevating_text(&self) -> &'static str {
        ELEVATING_TEXT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, RwLock};
    use std::time::Duration;
    use xdir_core::xtree::scan::walk;
    use xdir_core::xtree::tree::Tree;

    /// Carpeta temporal propia de la prueba.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-linux-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Recorre `root` en otro hilo; falla si no termina en 20 s (un ciclo lo colgaría).
    fn walk_tmp(root: &Path, progress: Arc<Progress>) -> Vec<Node> {
        let (tx, rx) = std::sync::mpsc::channel();
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let live: Tree = Arc::new(RwLock::new(Vec::new()));
            let r = walk(&root, "raiz", &LinuxLister::new(&root), &live, &progress, &AtomicBool::new(false));
            let _ = tx.send(r);
        });
        rx.recv_timeout(Duration::from_secs(20)).expect("el recorrido no terminó").expect("sin cancelar")
    }

    #[test]
    fn lister_does_not_follow_symlink_loops() {
        let dir = tmp("enlaces");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/x"), vec![1u8; 3000]).unwrap();
        std::os::unix::fs::symlink("..", dir.join("sub/loop")).unwrap();
        std::os::unix::fs::symlink("sub/x", dir.join("enlace")).unwrap();
        let l = LinuxLister::new(&dir);
        let sub = l.list(&dir.join("sub")).unwrap();
        assert!(sub.dirs.is_empty(), "el enlace a la carpeta padre no es carpeta: {:?}", sub.dirs);
        assert_eq!(sub.files.len(), 1);
        assert_eq!(&*sub.files[0].name, "x");
        assert!(sub.files[0].size >= 3000, "tamaño asignado: {}", sub.files[0].size);
        let top = l.list(&dir).unwrap();
        assert_eq!(top.dirs, ["sub"]);
        assert!(top.files.is_empty(), "el enlace no cuenta como archivo");
        let t = walk_tmp(&dir, Arc::new(Progress::default()));
        assert_eq!(t.len(), 3, "raíz, sub y x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lister_follows_symlinked_root() {
        let dir = tmp("raiz-enlace");
        std::fs::create_dir(dir.join("real")).unwrap();
        std::fs::create_dir(dir.join("real/sub")).unwrap();
        std::fs::write(dir.join("real/a"), "a").unwrap();
        std::os::unix::fs::symlink("real", dir.join("enlace")).unwrap();
        let root = dir.join("enlace");
        let top = LinuxLister::new(&root).list(&root).unwrap();
        assert_eq!(top.dirs, ["sub"]);
        assert_eq!(top.files.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lister_counts_denied_dirs() {
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("lister_counts_denied_dirs: se saltea como root (los permisos no aplican)");
            return;
        }
        let dir = tmp("denegada");
        let no = dir.join("no");
        std::fs::create_dir(&no).unwrap();
        std::fs::write(dir.join("a"), "a").unwrap();
        std::fs::set_permissions(&no, std::fs::Permissions::from_mode(0o000)).unwrap();
        let direct = LinuxLister::new(&dir).list(&no).err();
        let progress = Arc::new(Progress::default());
        let t = walk_tmp(&dir, progress.clone());
        std::fs::set_permissions(&no, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(direct, Some(ListError::Denied));
        assert_eq!(progress.denied.load(Ordering::Relaxed), 1);
        assert!(LinuxDiskScan.permission_issue(&progress));
        // El escaneo sigue: el archivo de al lado está.
        assert!(t.iter().any(|n| &*n.name == "a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_is_whole_volume_and_has_space() {
        let s = LinuxDiskScan;
        assert!(s.whole_volume(Path::new("/")));
        assert!(!s.whole_volume(&std::env::temp_dir().join("xdir-no-es-disco")));
        assert!(s.volume_space(Path::new("/")).is_some());
        assert!(s.drives().iter().any(|d| d.path == Path::new("/")));
        assert!(matches!(s.fast_scan(Path::new("/"), &Progress::default(), &AtomicBool::new(false)), FastScan::NotApplicable));
    }
}

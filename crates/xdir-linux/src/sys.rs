//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para Linux. `main` llama
//! `install` al arrancar, antes de crear la `App`.
//!
//! Cada servicio grande vive en su módulo: el sistema de archivos en `crate::fs`, las
//! operaciones de archivos en `crate::ops` (con la papelera de `crate::trash`), el escritorio en
//! `crate::shell` y el escaneo de xtree en `crate::scan`. Acá quedan los chicos.

use xdir_core::fs::{MetadataRequest, OverlayRequest};
use xdir_core::preview::PreviewRequest;
use xdir_core::sys::{Handle, Metadata, Overlays, Preview, Services, Waker};

use crate::env::LinuxEnv;
use crate::fs::LinuxFileSystem;
use crate::ops::LinuxFileOps;
use crate::scan::LinuxDiskScan;
use crate::shell::LinuxShell;

/// Sin hilo de interfaz no hay a quién despertar: `post` no hace nada. Los resultados igual
/// quedan en sus canales; cuando haya ventana, este servicio la despierta.
struct LinuxWaker;

impl Waker for LinuxWaker {
    fn post(&self, _window: Handle, _msg: u32, _wparam: usize, _lparam: isize) {}
}

/// Linux no tiene superposiciones de íconos: el pedido no se responde.
struct LinuxOverlays;

impl Overlays for LinuxOverlays {
    fn request(&self, _req: OverlayRequest) {}
}

/// Linux no tiene de dónde leer dimensiones, duración, autor ni etiquetas sin dependencias de
/// escritorio: se responde vacío (las columnas quedan sin valor).
struct LinuxMetadata;

impl Metadata for LinuxMetadata {
    fn request(&self, req: MetadataRequest) {
        req.answer_empty();
    }
}

/// Vista previa de la columna: la lectura neutra del núcleo en un hilo por pedido. Linux no tiene
/// archivos «solo en la nube» que se bajen al abrirlos.
struct LinuxPreview;

impl Preview for LinuxPreview {
    fn request(&self, req: PreviewRequest) {
        let spawned = std::thread::Builder::new().name("xdir-vista-previa".into()).spawn(move || xdir_core::preview::answer(req, &|_| false));
        debug_assert!(spawned.is_ok(), "hilo de vista previa");
    }
}

/// Instala los servicios de Linux en el registro del núcleo.
pub fn install() {
    xdir_core::sys::install(Services {
        waker: Box::new(LinuxWaker),
        env: Box::new(LinuxEnv),
        file_ops: Box::new(LinuxFileOps),
        disk_scan: Box::new(LinuxDiskScan),
        shell: Box::new(LinuxShell),
        file_system: Box::new(LinuxFileSystem),
        overlays: Box::new(LinuxOverlays),
        metadata: Box::new(LinuxMetadata),
        preview: Box::new(LinuxPreview),
        copy_prims: Box::new(crate::prims::LinuxCopyPrims),
        archives: Box::new(xdir_archive::Service { tar_gz: true, utc_offset: crate::env::utc_offset }),
        info: Box::new(crate::info::LinuxInfo),
        network: Box::new(crate::net::LinuxNetwork::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use xdir_core::preview::{PreviewBody, PreviewTarget};

    /// La vista previa responde desde su hilo con la lectura neutra.
    #[test]
    fn linux_preview_answers_from_thread() {
        xdir_core::sys::testing::install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-linux-preview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notas.md");
        std::fs::write(&file, "# Hola\nmundo\n").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let target = PreviewTarget::File(file);
        LinuxPreview.request(PreviewRequest { hwnd: Handle(0), tx, generation: 2, current: Arc::new(AtomicU64::new(2)), target });
        let reply = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("respuesta del hilo");
        assert_eq!(reply.generation, 2);
        assert_eq!(reply.info.body, PreviewBody::Text { lines: vec!["# Hola".into(), "mundo".into()] });
        let _ = std::fs::remove_dir_all(&dir);
    }
}

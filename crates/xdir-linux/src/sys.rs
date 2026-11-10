//! Implementaciones de los servicios del núcleo (`xdir_core::sys`) para Linux. `main` llama
//! `install` al arrancar, antes de crear la `App`.
//!
//! Cada servicio grande vive en su módulo: el sistema de archivos en `crate::fs`, las
//! operaciones de archivos en `crate::ops` (con la papelera de `crate::trash`), el escritorio en
//! `crate::shell` y el escaneo de xtree en `crate::scan`. Acá quedan los chicos.

use xdir_core::fs::{MetadataRequest, OverlayRequest};
use xdir_core::sys::{Handle, Metadata, Overlays, Services, Waker};

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
    });
}

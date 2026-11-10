//! Tamaño de una carpeta para el panel de información: suma recursiva con `std::fs`, sin seguir
//! enlaces y sin entrar en carpetas solo en la nube (las enumeraría el proveedor y podría bajar
//! datos). Corre en un hilo de trabajo; `step` informa el avance cada `STEP` elementos.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cada cuántos elementos se informa el avance («Calculando… N elementos»).
pub const STEP: u64 = 500;

/// Suma `path`: llama a `step(bytes, archivos, carpetas)` cada `STEP` elementos y una vez al final
/// (siempre, aunque se cancele). `ctl` en `true` corta la suma: se mira antes de abrir cada carpeta y
/// en cada elemento. Los archivos solo en la nube cuentan
/// con su tamaño lógico (sin abrirlos); las carpetas solo en la nube cuentan, pero no se entra.
/// Devuelve cuántas carpetas no se pudieron leer (sin permiso: quedan fuera de la cuenta).
pub fn folder_size(path: &Path, cloud_only: &dyn Fn(&Path) -> bool, ctl: &AtomicBool, step: &mut dyn FnMut(u64, u64, u64)) -> u64 {
    let mut skipped = 0u64;
    let (mut bytes, mut files, mut folders) = (0u64, 0u64, 0u64);
    let mut stack = vec![path.to_path_buf()];
    let mut seen = 0u64;
    'walk: while let Some(dir) = stack.pop() {
        // Cortada: no se abre ninguna carpeta más (expulsar no debe encontrar nada abierto).
        if ctl.load(Ordering::Relaxed) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            skipped += 1;
            continue;
        };
        for entry in rd.flatten() {
            if ctl.load(Ordering::Relaxed) {
                break 'walk;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let p = entry.path();
            if ft.is_dir() {
                folders += 1;
                if !cloud_only(&p) {
                    stack.push(p);
                }
            } else {
                files += 1;
                if let Ok(m) = entry.metadata() {
                    bytes += m.len();
                }
            }
            seen += 1;
            if seen.is_multiple_of(STEP) {
                step(bytes, files, folders);
            }
        }
    }
    step(bytes, files, folders);
    skipped
}

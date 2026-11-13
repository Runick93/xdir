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
    folder_size_every(path, cloud_only, &|_| {}, ctl, STEP, step)
}

/// `folder_size` informando el avance cada `every` elementos (1: con cada uno, para vigilar una
/// suma en una carpeta de red). `link` recibe cada enlace que se encuentra (no se sigue: cuenta
/// lo que pesa el enlace).
pub fn folder_size_every(
    path: &Path,
    cloud_only: &dyn Fn(&Path) -> bool,
    link: &dyn Fn(&Path),
    ctl: &AtomicBool,
    every: u64,
    step: &mut dyn FnMut(u64, u64, u64),
) -> u64 {
    let every = every.max(1);
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
                if ft.is_symlink() {
                    link(&p);
                }
                files += 1;
                if let Ok(m) = entry.metadata() {
                    bytes += m.len();
                }
            }
            seen += 1;
            if seen.is_multiple_of(every) {
                step(bytes, files, folders);
            }
        }
    }
    step(bytes, files, folders);
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Con `every = 1` el avance se informa con cada elemento (la suma de una carpeta de red se
    /// vigila elemento por elemento), y siempre una vez al final.
    #[test]
    fn folder_size_every_reports_each_entry() {
        let dir = std::env::temp_dir().join(format!("xdir-size-cada-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for (i, n) in [3usize, 4, 5].iter().enumerate() {
            std::fs::write(dir.join(format!("f{i}")), vec![0u8; *n]).unwrap();
        }
        let ctl = AtomicBool::new(false);
        let mut calls = 0;
        let mut last = (0, 0, 0);
        let skipped = folder_size_every(&dir, &|_| false, &|_| {}, &ctl, 1, &mut |b, f, d| {
            calls += 1;
            last = (b, f, d);
        });
        assert_eq!((skipped, last), (0, (12, 3, 1)));
        assert_eq!(calls, 4 + 1, "uno por elemento y uno al final");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

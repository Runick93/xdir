//! Miniaturas e íconos grandes para las vistas de cuadrícula, en un hilo propio. Se le pide la
//! imagen al Shell (`IShellItemImageFactory`), que da la miniatura real de imágenes, videos o
//! PDF y, para lo demás, el ícono del archivo en ese tamaño. Igual que los íconos chicos: la
//! interfaz pide, dibuja sin la imagen y el hilo avisa con `WM_APP_ICONS` cuando está lista.
//!
//! Se atiende primero lo último que se pidió (lo que está a la vista ahora, si se hizo scroll
//! rápido), y lo que ya no se ve no se descarta pero queda al final de la cola.
//!
//! La imagen grande (vista previa, Galería) va por la misma cola, en un escalón de
//! `thumbs::big_side`, con su propia lista de terminadas; la de un archivo solo en la nube se
//! pide solo de la caché del Shell (`SIIGBF_INCACHEONLY`), así no se baja.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromIDList, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_INCACHEONLY,
    SIIGBF_RESIZETOFIT,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, PCWSTR};

use crate::platform::Handle;
use crate::fs::wide_path;
use crate::icons::{self, Pixels, WM_APP_ICONS};

use super::*;

/// Un pedido: la imagen, si es grande (vista previa) y la ventana a la que se avisa.
type Job = ((Key, bool), usize);

struct Queue {
    /// Pila: se atiende primero lo último pedido.
    jobs: Vec<Job>,
    /// (clave, grande) encargadas y sin terminar.
    pending: HashSet<(Key, bool)>,
}

fn queue() -> &'static (Mutex<Queue>, Condvar) {
    static Q: OnceLock<(Mutex<Queue>, Condvar)> = OnceLock::new();
    Q.get_or_init(|| {
        std::thread::spawn(run);
        (Mutex::new(Queue { jobs: Vec::new(), pending: HashSet::new() }), Condvar::new())
    })
}

/// Imágenes terminadas (`None`: no se pudo obtener), hasta que el renderizador las retire.
type Done = Mutex<Vec<(Key, Option<Arc<Pixels>>)>>;

/// Resultado listo para pasar al renderizador (lo retira de acá).
fn done() -> &'static Done {
    static D: OnceLock<Done> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

fn done_big() -> &'static Done {
    static D: OnceLock<Done> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

/// Encarga una imagen (si no estaba encargada ya).
pub fn request(hwnd: Handle, key: Key) {
    enqueue(hwnd, (key, false));
}

/// Encarga una imagen grande (`key.1` es el escalón), en la misma cola.
pub fn request_big(hwnd: Handle, key: Key) {
    enqueue(hwnd, (key, true));
}

fn enqueue(hwnd: Handle, job: (Key, bool)) {
    let (lock, cv) = queue();
    let mut q = lock.lock().unwrap();
    if q.pending.insert(job.clone()) {
        q.jobs.push((job, hwnd.0 as usize));
        cv.notify_one();
    } else if let Some(i) = q.jobs.iter().position(|(j, _)| *j == job) {
        // Ya estaba en cola: pasa adelante (vuelve a estar a la vista).
        let job = q.jobs.remove(i);
        q.jobs.push(job);
    }
}

/// Imágenes que terminó el hilo desde la última vez. `None`: no se pudo obtener.
pub fn take_done() -> Vec<(Key, Option<Arc<Pixels>>)> {
    std::mem::take(&mut *done().lock().unwrap())
}

/// Imágenes grandes que terminó el hilo desde la última vez. `None`: no se pudo obtener.
pub fn take_done_big() -> Vec<(Key, Option<Arc<Pixels>>)> {
    std::mem::take(&mut *done_big().lock().unwrap())
}

/// Banderas para pedir una imagen grande: ajustada al tamaño y, si el archivo está solo en la
/// nube, solo de la caché (sin bajarlo).
pub(crate) fn big_flags(cloud: bool) -> SIIGBF {
    let flags = SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK;
    if cloud { flags | SIIGBF_INCACHEONLY } else { flags }
}

fn run() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    icons::wait_shell_ready();
    loop {
        let ((key, big), hwnd) = {
            let (lock, cv) = queue();
            let mut q = lock.lock().unwrap();
            while q.jobs.is_empty() {
                q = cv.wait(q).unwrap();
            }
            q.jobs.pop().unwrap()
        };
        let flags = match (&key.0, big) {
            (Source::Path(p), true) => big_flags(crate::metadata::cloud_only(&HSTRING::from(p.as_path()))),
            (_, true) => big_flags(false),
            (_, false) => SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK,
        };
        let pixels = image(&key, flags).map(Arc::new);
        {
            let (lock, _) = queue();
            lock.lock().unwrap().pending.remove(&(key.clone(), big));
        }
        if big { done_big() } else { done() }.lock().unwrap().push((key, pixels));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_ICONS, WPARAM(0), LPARAM(0));
        }
    }
}

fn image((source, px): &Key, flags: SIIGBF) -> Option<Pixels> {
    unsafe {
        let factory: IShellItemImageFactory = match source {
            Source::Path(p) => {
                let wide = wide_path(p);
                SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?
            }
            Source::Pidl(bytes) => SHCreateItemFromIDList(bytes.as_ptr() as *const ITEMIDLIST).ok()?,
        };
        let size = SIZE { cx: *px as i32, cy: *px as i32 };
        let bitmap = factory.GetImage(size, flags).ok()?;
        let pixels = icons::bitmap_pixels(bitmap);
        let _ = DeleteObject(bitmap.into());
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// PNG real de 3×2.
    const PNG_3X2: [u8; 73] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
        0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x41, 0xd2, 0x05, 0xfb, 0x87, 0xf0, 0xb9,
        0x48, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// Un archivo solo en la nube se pide solo de la caché; uno local (no en la nube), normal.
    #[test]
    fn cloud_big_image_cache_only() {
        assert!(big_flags(true).0 & SIIGBF_INCACHEONLY.0 != 0);
        let local = big_flags(false);
        assert_eq!(local.0 & SIIGBF_INCACHEONLY.0, 0);
        assert_eq!(local, SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK);
        let dir = std::env::temp_dir().join(format!("xdir-big-nube-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("local.png");
        std::fs::write(&png, PNG_3X2).unwrap();
        let cloud = crate::metadata::cloud_only(&HSTRING::from(png.as_path()));
        assert!(!cloud);
        assert_eq!(big_flags(cloud).0 & SIIGBF_INCACHEONLY.0, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un PNG temporal pedido como imagen grande termina con píxeles, en la lista de las grandes.
    #[test]
    fn windows_big_image_png() {
        xdir_core::sys::testing::install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("foto.png");
        std::fs::write(&png, PNG_3X2).unwrap();
        let key: Key = (Source::Path(png), xdir_core::thumbs::big_side(300));
        request_big(Handle::default(), key.clone());
        let start = Instant::now();
        let mut seen = Vec::new();
        let px = loop {
            seen.extend(take_done_big());
            if let Some(i) = seen.iter().position(|(k, _)| *k == key) {
                break seen.swap_remove(i).1;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "la imagen grande no terminó");
            std::thread::sleep(Duration::from_millis(20));
        };
        let px = px.expect("píxeles de la imagen grande");
        assert!(px.width > 0 && px.height > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Miniaturas e íconos grandes para las vistas de cuadrícula, en un hilo propio. Se le pide la
//! imagen al Shell (`IShellItemImageFactory`), que da la miniatura real de imágenes, videos o
//! PDF y, para lo demás, el ícono del archivo en ese tamaño. Igual que los íconos chicos: la
//! interfaz pide, dibuja sin la imagen y el hilo avisa con `WM_APP_ICONS` cuando está lista.
//!
//! Se atiende primero lo último que se pidió (lo que está a la vista ahora, si se hizo scroll
//! rápido), y lo que ya no se ve no se descarta pero queda al final de la cola.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::DeleteObject;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromIDList, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::PCWSTR;

use crate::fs::wide_path;
use crate::icons::{self, Pixels, WM_APP_ICONS};

/// De qué elemento: una ruta de disco o un PIDL absoluto (elementos virtuales del Shell).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Path(PathBuf),
    Pidl(Arc<[u8]>),
}

/// (elemento, lado en píxeles).
pub type Key = (Source, u32);

struct Queue {
    /// Pila: se atiende primero lo último pedido.
    jobs: Vec<(Key, usize)>,
    pending: HashSet<Key>,
}

fn queue() -> &'static (Mutex<Queue>, Condvar) {
    static Q: OnceLock<(Mutex<Queue>, Condvar)> = OnceLock::new();
    Q.get_or_init(|| {
        std::thread::spawn(run);
        (Mutex::new(Queue { jobs: Vec::new(), pending: HashSet::new() }), Condvar::new())
    })
}

/// Resultado listo para pasar al renderizador (lo retira de acá).
fn done() -> &'static Mutex<Vec<(Key, Option<Arc<Pixels>>)>> {
    static D: OnceLock<Mutex<Vec<(Key, Option<Arc<Pixels>>)>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

/// Encarga una imagen (si no estaba encargada ya).
pub fn request(hwnd: HWND, key: Key) {
    let (lock, cv) = queue();
    let mut q = lock.lock().unwrap();
    if q.pending.insert(key.clone()) {
        q.jobs.push((key, hwnd.0 as usize));
        cv.notify_one();
    } else if let Some(i) = q.jobs.iter().position(|(k, _)| *k == key) {
        // Ya estaba en cola: pasa adelante (vuelve a estar a la vista).
        let job = q.jobs.remove(i);
        q.jobs.push(job);
    }
}

/// Imágenes que terminó el hilo desde la última vez. `None`: no se pudo obtener.
pub fn take_done() -> Vec<(Key, Option<Arc<Pixels>>)> {
    std::mem::take(&mut *done().lock().unwrap())
}

fn run() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    icons::wait_shell_ready();
    loop {
        let (key, hwnd) = {
            let (lock, cv) = queue();
            let mut q = lock.lock().unwrap();
            while q.jobs.is_empty() {
                q = cv.wait(q).unwrap();
            }
            q.jobs.pop().unwrap()
        };
        let pixels = image(&key).map(Arc::new);
        {
            let (lock, _) = queue();
            lock.lock().unwrap().pending.remove(&key);
        }
        done().lock().unwrap().push((key, pixels));
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_ICONS, WPARAM(0), LPARAM(0));
        }
    }
}

fn image((source, px): &Key) -> Option<Pixels> {
    unsafe {
        let factory: IShellItemImageFactory = match source {
            Source::Path(p) => {
                let wide = wide_path(p);
                SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?
            }
            Source::Pidl(bytes) => SHCreateItemFromIDList(bytes.as_ptr() as *const ITEMIDLIST).ok()?,
        };
        let size = SIZE { cx: *px as i32, cy: *px as i32 };
        let bitmap = factory.GetImage(size, SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK).ok()?;
        let pixels = icons::bitmap_pixels(bitmap);
        let _ = DeleteObject(bitmap.into());
        pixels
    }
}

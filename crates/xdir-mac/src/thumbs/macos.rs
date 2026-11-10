//! macOS: miniaturas con QuickLook (`QLThumbnailGenerator`), que da la vista previa real de
//! imágenes, videos, PDF y documentos, o el ícono del archivo si no hay vista previa. La imagen
//! grande (vista previa, Galería) es lo mismo en un escalón de `thumbs::big_side`, con su propia
//! lista de pendientes y terminadas; los archivos solo en la nube no se piden (los bajaría).

use std::collections::HashSet;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo};
use objc2_foundation::{NSError, NSString, NSURL};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};

use super::*;
use crate::icons::{Pixels, WM_APP_ICONS};
use crate::platform::Handle;

fn pending() -> &'static Mutex<HashSet<Key>> {
    static P: OnceLock<Mutex<HashSet<Key>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Imágenes terminadas (`None`: no se pudo obtener), hasta que el renderizador las retire.
type Done = Mutex<Vec<(Key, Option<Arc<Pixels>>)>>;

/// Resultado listo para pasar al renderizador (lo retira de acá).
fn done() -> &'static Done {
    static D: OnceLock<Done> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

/// Encarga una imagen (si no estaba encargada ya).
pub fn request(hwnd: Handle, key: Key) {
    if !pending().lock().unwrap().insert(key.clone()) {
        return;
    }
    generate(key, move |key, px| finish(hwnd, key, px));
}

fn pending_big() -> &'static Mutex<HashSet<Key>> {
    static P: OnceLock<Mutex<HashSet<Key>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashSet::new()))
}

fn done_big() -> &'static Done {
    static D: OnceLock<Done> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

/// Encarga una imagen grande (`key.1` es el escalón). Un archivo solo en la nube termina sin
/// imagen y sin pedirla: QuickLook lo bajaría entero. El chequeo de nube (`lstat`/`stat`) se hace
/// en un hilo propio, no en el de interfaz (que llega acá desde el dibujo): un disco de red
/// colgado no traba la ventana.
pub fn request_big(hwnd: Handle, key: Key) {
    if !pending_big().lock().unwrap().insert(key.clone()) {
        return;
    }
    static W: OnceLock<Mutex<Sender<(Handle, Key)>>> = OnceLock::new();
    let worker = W.get_or_init(|| {
        let (tx, rx) = channel::<(Handle, Key)>();
        std::thread::Builder::new()
            .name("xdir-imagen-grande".into())
            .spawn(move || {
                while let Ok((hwnd, key)) = rx.recv() {
                    start_big(hwnd, key);
                }
            })
            .expect("hilo de imágenes grandes");
        Mutex::new(tx)
    });
    if let Err(e) = worker.lock().unwrap().send((hwnd, key)) {
        let (hwnd, key) = e.0;
        finish_big(hwnd, key, None);
    }
}

/// En el hilo de imágenes grandes: mira la nube y, si el archivo está en el disco, se lo pide a
/// QuickLook (que responde en su propia cola).
fn start_big(hwnd: Handle, key: Key) {
    if let Source::Path(path) = &key.0
        && crate::metadata::cloud_only(path)
    {
        finish_big(hwnd, key, None);
        return;
    }
    autoreleasepool(|_| generate(key, move |key, px| finish_big(hwnd, key, px)));
}

/// Imágenes grandes que terminaron desde la última vez. `None`: no se pudo obtener.
pub fn take_done_big() -> Vec<(Key, Option<Arc<Pixels>>)> {
    std::mem::take(&mut *done_big().lock().unwrap())
}

fn finish_big(hwnd: Handle, key: Key, px: Option<Arc<Pixels>>) {
    pending_big().lock().unwrap().remove(&key);
    done_big().lock().unwrap().push((key, px));
    hwnd.post(WM_APP_ICONS, 0, 0);
}

/// Le pide la imagen a QuickLook y llama `then` con el resultado (desde otro hilo).
fn generate(key: Key, then: impl Fn(Key, Option<Arc<Pixels>>) + 'static) {
    let Source::Path(path) = &key.0 else {
        then(key, None);
        return;
    };
    let side = key.1.max(16) as CGFloat;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let request = unsafe {
        QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
            QLThumbnailGenerationRequest::alloc(),
            &url,
            CGSize::new(side, side),
            1.0,
            QLThumbnailGenerationRequestRepresentationTypes::All,
        )
    };
    let job = Mutex::new(Some(key));
    let handler = RcBlock::new(move |rep: *mut QLThumbnailRepresentation, _err: *mut NSError| {
        let Some(key) = job.lock().unwrap().take() else { return };
        let px = unsafe { rep.as_ref() }.and_then(|rep| pixels_of(&*unsafe { rep.CGImage() }));
        then(key, px.map(Arc::new));
    });
    unsafe { QLThumbnailGenerator::sharedGenerator().generateBestRepresentationForRequest_completionHandler(&request, &handler) };
}

fn finish(hwnd: Handle, key: Key, px: Option<Arc<Pixels>>) {
    pending().lock().unwrap().remove(&key);
    done().lock().unwrap().push((key, px));
    hwnd.post(WM_APP_ICONS, 0, 0);
}

/// Imágenes que terminaron desde la última vez. `None`: no se pudo obtener.
pub fn take_done() -> Vec<(Key, Option<Arc<Pixels>>)> {
    std::mem::take(&mut *done().lock().unwrap())
}

/// Imagen de Core Graphics → píxeles BGRA premultiplicados.
fn pixels_of(image: &CGImage) -> Option<Pixels> {
    let (w, h) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    if w == 0 || h == 0 {
        return None;
    }
    let mut data = vec![0u8; w * h * 4];
    let space = CGColorSpace::new_device_rgb()?;
    let info = objc2_core_graphics::CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0;
    let ctx = unsafe { CGBitmapContextCreate(data.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), info) }?;
    CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as CGFloat, h as CGFloat)), Some(image));
    drop(ctx);
    Some(Pixels { width: w as u32, height: h as u32, data })
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

    /// Espera hasta 20 s a que termine la imagen grande de `key`.
    fn wait_big(key: &Key) -> Option<Arc<Pixels>> {
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < Duration::from_secs(20) {
            seen.extend(take_done_big());
            if let Some(i) = seen.iter().position(|(k, _)| k == key) {
                return seen.swap_remove(i).1;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("la imagen grande no terminó");
    }

    /// Un PNG temporal pedido como imagen grande termina con píxeles (QuickLook de verdad).
    #[test]
    fn mac_big_image_png() {
        xdir_core::sys::testing::install_fakes();
        let dir = std::env::temp_dir().join(format!("xdir-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("foto.png");
        std::fs::write(&png, PNG_3X2).unwrap();
        let key: Key = (Source::Path(png), xdir_core::thumbs::big_side(300));
        assert_eq!(key.1, 512);
        request_big(Handle::default(), key.clone());
        let px = wait_big(&key).expect("píxeles de la imagen grande");
        assert!(px.width > 0 && px.height > 0);
        assert_eq!(px.data.len(), (px.width * px.height * 4) as usize);
        // No quedó pendiente: se puede volver a pedir.
        assert!(!pending_big().lock().unwrap().contains(&key));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! macOS: miniaturas con QuickLook (`QLThumbnailGenerator`), que da la vista previa real de
//! imágenes, videos, PDF y documentos, o el ícono del archivo si no hay vista previa.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use block2::RcBlock;
use objc2::AnyThread;
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapInfo, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo};
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

/// Resultado listo para pasar al renderizador (lo retira de acá).
fn done() -> &'static Mutex<Vec<(Key, Option<Arc<Pixels>>)>> {
    static D: OnceLock<Mutex<Vec<(Key, Option<Arc<Pixels>>)>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Vec::new()))
}

/// Encarga una imagen (si no estaba encargada ya).
pub fn request(hwnd: Handle, key: Key) {
    if !pending().lock().unwrap().insert(key.clone()) {
        return;
    }
    let Source::Path(path) = &key.0 else {
        finish(hwnd, key, None);
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
        finish(hwnd, key, px.map(Arc::new));
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
    let info = CGBitmapInfo::ByteOrder32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0;
    let ctx = unsafe { CGBitmapContextCreate(data.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), info) }?;
    CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as CGFloat, h as CGFloat)), Some(image));
    drop(ctx);
    Some(Pixels { width: w as u32, height: h as u32, data })
}

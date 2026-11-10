//! macOS: metadatos de archivos para las columnas Dimensiones, Duración, Autor, Etiquetas,
//! Artista y Álbum. Se leen en un hilo propio, un pedido detrás de otro.

use std::ffi::c_void;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL};
use objc2_core_services::{MDItem, kMDItemAlbum, kMDItemAuthors, kMDItemContentTypeTree, kMDItemDurationSeconds, kMDItemPixelHeight, kMDItemPixelWidth};
use objc2_foundation::{NSArray, NSString, NSURL, NSURLTagNamesKey};
use objc2_uniform_type_identifiers::{UTType, UTTypeImage};
use xdir_core::fs::{Meta, MetadataRequest};

/// Encola el pedido para el hilo de metadatos (lo crea la primera vez) y vuelve enseguida.
pub fn queue(req: MetadataRequest) {
    static W: OnceLock<Mutex<Sender<MetadataRequest>>> = OnceLock::new();
    let worker = W.get_or_init(|| {
        let (tx, rx) = channel::<MetadataRequest>();
        std::thread::Builder::new()
            .name("xdir-metadatos".into())
            .spawn(move || {
                while let Ok(req) = rx.recv() {
                    req.answer_with(read);
                }
            })
            .expect("hilo de metadatos");
        Mutex::new(tx)
    });
    let _ = worker.lock().unwrap().send(req);
}

/// Metadatos de un archivo: lo que tenga Spotlight; las dimensiones de una imagen que Spotlight
/// no indexó (carpetas temporales, discos sin índice, archivos recién copiados), de la cabecera
/// de la imagen; las etiquetas, del archivo (`NSURLTagNamesKey`), que no depende del índice.
/// `Meta::default()` si no hay datos.
pub fn read(path: &Path) -> Meta {
    autoreleasepool(|_| {
        let mut meta = spotlight(path).unwrap_or_default();
        if meta.width.is_none() && is_image(path) && !cloud_only(path)
            && let Some((w, h)) = image_dimensions(path)
        {
            (meta.width, meta.height) = (Some(w), Some(h));
        }
        meta.tags = finder_tags(path);
        meta
    })
}

/// Lo que Spotlight sabe del archivo. `None` si no lo tiene (o no existe).
fn spotlight(path: &Path) -> Option<Meta> {
    let url = CFURL::from_file_path(path)?;
    let item = unsafe { MDItem::with_url(None, Some(&url)) }?;
    let attr = |key: Option<&CFString>| unsafe { item.attribute(key) };
    let mut meta = Meta::default();
    if let Some((w, h)) = dimensions_of(&item) {
        (meta.width, meta.height) = (Some(w), Some(h));
    }
    meta.duration_ms = attr(unsafe { kMDItemDurationSeconds })
        .and_then(|v| number(&v))
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| (s * 1000.0).round() as u64);
    let authors = attr(unsafe { kMDItemAuthors }).map(|v| strings(&v)).unwrap_or_default();
    meta.album = attr(unsafe { kMDItemAlbum }).and_then(|v| strings(&v).into_iter().next());
    // En el audio, Spotlight pone a los intérpretes en `kMDItemAuthors`: van a Artista, como en
    // el Explorador (donde Autor queda vacío para la música).
    let audio = attr(unsafe { kMDItemContentTypeTree }).is_some_and(|v| strings(&v).iter().any(|t| t == "public.audio"));
    if audio {
        meta.artist = (!authors.is_empty()).then(|| authors.join("; "));
    } else {
        meta.authors = authors;
    }
    Some(meta)
}

/// Ancho y alto según Spotlight, si lo tiene indexado (los tests avisan si no).
#[cfg(test)]
fn spotlight_dimensions(path: &Path) -> Option<(u32, u32)> {
    let url = CFURL::from_file_path(path)?;
    let item = unsafe { MDItem::with_url(None, Some(&url)) }?;
    dimensions_of(&item)
}

fn dimensions_of(item: &MDItem) -> Option<(u32, u32)> {
    let side = |key: Option<&CFString>| unsafe { item.attribute(key) }.and_then(|v| number(&v)).filter(|n| *n >= 1.0).map(|n| n as u32);
    Some((side(unsafe { kMDItemPixelWidth })?, side(unsafe { kMDItemPixelHeight })?))
}

/// `SF_DATALESS` de `chflags`: el archivo solo está en la nube (iCloud y similares).
const SF_DATALESS: u32 = 0x4000_0000;

/// ¿El archivo solo está en la nube? Leer su cabecera haría que el sistema lo baje entero, así que
/// no se busca en él lo que Spotlight no tenga. Mira el enlace y el archivo al que apunta: con
/// cualquiera de los dos sin datos locales, está en la nube.
pub(crate) fn cloud_only(path: &Path) -> bool {
    file_flags(path).iter().any(|f| f & SF_DATALESS != 0)
}

/// Banderas de `chflags` del elemento (`lstat`, no sigue enlaces) y del archivo al que lleva
/// (`stat`, los sigue); las que se pudieron leer.
fn file_flags(path: &Path) -> Vec<u32> {
    use std::os::macos::fs::MetadataExt;
    [std::fs::symlink_metadata(path), std::fs::metadata(path)].into_iter().filter_map(|m| m.ok()).map(|m| m.st_flags()).collect()
}

/// ¿La extensión es de una imagen? Así no se abre cada archivo de la carpeta para buscarle
/// dimensiones.
fn is_image(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    UTType::typeWithFilenameExtension(&NSString::from_str(ext)).is_some_and(|t| t.conformsToType(unsafe { UTTypeImage }))
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithURL(url: &CFURL, options: *const c_void) -> *mut c_void;
    fn CGImageSourceCopyPropertiesAtIndex(source: *const c_void, index: usize, options: *const c_void) -> *mut c_void;
    static kCGImagePropertyPixelWidth: &'static CFString;
    static kCGImagePropertyPixelHeight: &'static CFString;
}

/// Ancho y alto de la cabecera de una imagen (ImageIO lee solo las propiedades, sin decodificar).
fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let url = CFURL::from_file_path(path)?;
    unsafe {
        let source = NonNull::new(CGImageSourceCreateWithURL(&url, ptr::null()))?;
        let source: CFRetained<CFType> = CFRetained::from_raw(source.cast());
        let props = NonNull::new(CGImageSourceCopyPropertiesAtIndex((&*source as *const CFType).cast(), 0, ptr::null()))?;
        let props: CFRetained<CFDictionary<CFString, CFType>> = CFRetained::from_raw(props.cast());
        let side = |key: &CFString| props.get(key).and_then(|v| number(&v)).filter(|n| *n >= 1.0).map(|n| n as u32);
        Some((side(kCGImagePropertyPixelWidth)?, side(kCGImagePropertyPixelHeight)?))
    }
}

/// Etiquetas del Finder del archivo. Un `NSURL` nuevo en cada lectura: no guarda valores viejos.
fn finder_tags(path: &Path) -> Vec<String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let mut value: Option<Retained<AnyObject>> = None;
    if unsafe { url.getResourceValue_forKey_error(&mut value, NSURLTagNamesKey) }.is_err() {
        return Vec::new();
    }
    let Some(array) = value.and_then(|v| v.downcast::<NSArray>().ok()) else { return Vec::new() };
    array.iter().filter_map(|t| t.downcast::<NSString>().ok()).map(|t| t.to_string()).filter(|t| !t.is_empty()).collect()
}

fn number(v: &CFType) -> Option<f64> {
    v.downcast_ref::<CFNumber>()?.as_f64()
}

/// Un texto o una lista de textos de Spotlight, como lista.
fn strings(v: &CFType) -> Vec<String> {
    if let Some(s) = v.downcast_ref::<CFString>() {
        let s = s.to_string();
        return if s.is_empty() { Vec::new() } else { vec![s] };
    }
    let Some(array) = v.downcast_ref::<CFArray>() else { return Vec::new() };
    let array = unsafe { &*(array as *const CFArray).cast::<CFArray<CFType>>() };
    array.iter().filter_map(|x| x.downcast_ref::<CFString>().map(|s| s.to_string())).filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Un PNG de 3×2 píxeles (rojo), con bytes fijos.
    const PNG_3X2: [u8; 73] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
        0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x41, 0xd2, 0x05, 0xfb, 0x87, 0xf0, 0xb9,
        0x48, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    /// Pone etiquetas del Finder a un archivo (como el Finder: `NSURLTagNamesKey`).
    fn set_tags(path: &Path, tags: &[&str]) {
        use objc2_foundation::{NSArray, NSString, NSURL, NSURLTagNamesKey};
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let names: Vec<_> = tags.iter().map(|t| NSString::from_str(t)).collect();
        let value = NSArray::from_retained_slice(&names);
        unsafe { url.setResourceValue_forKey_error(Some(&value), NSURLTagNamesKey) }.expect("poner etiquetas");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Las dimensiones salen de Spotlight; si Spotlight no tiene el archivo (una carpeta temporal
    /// no se indexa), de la cabecera de la imagen.
    #[test]
    fn mac_reads_png_dimensions() {
        let dir = temp_dir("png");
        let png = dir.join("chica.png");
        std::fs::write(&png, PNG_3X2).unwrap();
        if spotlight_dimensions(&png).is_none() {
            eprintln!("aviso: Spotlight no tiene {} indexado; las dimensiones salen de la imagen", png.display());
        }
        let meta = read(&png);
        assert_eq!((meta.width, meta.height), (Some(3), Some(2)));
        assert_eq!(meta.duration_ms, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Las etiquetas del Finder se leen del archivo (`NSURLTagNamesKey`), sin depender de Spotlight.
    #[test]
    fn mac_reads_finder_tags() {
        let dir = temp_dir("tags");
        let file = dir.join("notas.txt");
        std::fs::write(&file, "hola").unwrap();
        assert!(read(&file).tags.is_empty());
        set_tags(&file, &["Rojo", "Trabajo"]);
        let mut tags = read(&file).tags;
        tags.sort();
        assert_eq!(tags, ["Rojo", "Trabajo"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un archivo en el disco no está «solo en la nube» (ni uno que no existe).
    #[test]
    fn local_file_is_not_cloud_only() {
        let dir = temp_dir("nube");
        let file = dir.join("local.png");
        std::fs::write(&file, PNG_3X2).unwrap();
        assert!(!cloud_only(&file));
        assert!(!cloud_only(&dir.join("no-existe.png")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Las banderas que mira `cloud_only` en un enlace incluyen las del archivo al que apunta
    /// (con `UF_HIDDEN`, que se puede poner sin la nube, en lugar de `SF_DATALESS`).
    #[test]
    fn cloud_flags_follow_symlinks() {
        const UF_HIDDEN: u32 = 0x8000;
        let dir = temp_dir("nube-enlace");
        let target = dir.join("destino.png");
        std::fs::write(&target, PNG_3X2).unwrap();
        let ok = std::process::Command::new("chflags").arg("hidden").arg(&target).status().unwrap().success();
        assert!(ok, "chflags hidden");
        let link = dir.join("enlace.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let flags = file_flags(&link);
        assert_eq!(flags.len(), 2, "lstat y stat");
        assert!(flags.iter().any(|f| f & UF_HIDDEN != 0), "{flags:x?}");
        assert!(file_flags(&dir.join("no-existe.png")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un archivo que no existe, o sin metadatos, da `Meta::default()` (y no se cuelga).
    /// Para probar a mano contra archivos indexados: `XDIR_META=<ruta> cargo test -p xdir-mac
    /// metadata_print -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn metadata_print() {
        for p in std::env::var("XDIR_META").unwrap_or_default().split(':').filter(|p| !p.is_empty()) {
            eprintln!("{p}: {:?}", read(Path::new(p)));
        }
    }

    #[test]
    fn metadata_unknown_file_is_empty() {
        let dir = temp_dir("vacio");
        assert_eq!(read(&dir.join("no-existe.png")), Meta::default());
        let raro = dir.join("datos.zzz");
        std::fs::write(&raro, [0u8, 1, 2, 3]).unwrap();
        assert_eq!(read(&raro), Meta::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El hilo de metadatos responde por el canal de la pestaña, en otro hilo.
    #[test]
    fn metadata_answers_from_worker() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicU64;
        use xdir_core::fs::{LoadMsg, MetaField};
        xdir_core::sys::testing::install_fakes();
        let dir = temp_dir("hilo");
        let png = dir.join("a.png");
        std::fs::write(&png, PNG_3X2).unwrap();
        let (tx, rx) = channel();
        queue(MetadataRequest {
            hwnd: xdir_core::sys::Handle(0),
            tx,
            panel: 1,
            generation: 4,
            current: Arc::new(AtomicU64::new(4)),
            items: vec![(9, png)],
            fields: MetaField::ALL.to_vec(),
        });
        let ev = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("respuesta del hilo");
        assert_eq!((ev.panel, ev.generation), (1, 4));
        let LoadMsg::Metadata(batch) = ev.msg else { panic!("se esperaban metadatos") };
        assert_eq!(batch.len(), 1);
        assert_eq!((batch[0].0, batch[0].1.width), (9, Some(3)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Windows: metadatos de archivos para las columnas Dimensiones, Duración, Autor, Etiquetas,
//! Artista y Álbum, leídos de `IPropertyStore` (lo mismo que muestra el Explorador). Se leen en
//! un hilo propio con COM (MTA), un pedido detrás de otro.

use std::path::Path;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_Author, PKEY_Image_HorizontalSize, PKEY_Image_VerticalSize, PKEY_Keywords, PKEY_Media_Duration, PKEY_Music_AlbumTitle,
    PKEY_Music_Artist, PKEY_Video_FrameHeight, PKEY_Video_FrameWidth,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToStringAlloc, PropVariantToStringVectorAlloc, PropVariantToUInt32, PropVariantToUInt64,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::PropertiesSystem::{GPS_BESTEFFORT, IPropertyStore, SHGetPropertyStoreFromParsingName};
use windows::core::{HSTRING, PWSTR};
use xdir_core::fs::{Meta, MetadataRequest};

/// Encola el pedido para el hilo de metadatos (lo crea la primera vez) y vuelve enseguida.
pub fn queue(req: MetadataRequest) {
    static W: OnceLock<Mutex<Sender<MetadataRequest>>> = OnceLock::new();
    let worker = W.get_or_init(|| {
        let (tx, rx) = channel::<MetadataRequest>();
        std::thread::Builder::new()
            .name("xdir-metadatos".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                while let Ok(req) = rx.recv() {
                    req.answer_with(read);
                }
            })
            .expect("hilo de metadatos");
        Mutex::new(tx)
    });
    let _ = worker.lock().unwrap().send(req);
}

/// Metadatos de un archivo (con COM ya inicializado en el hilo). `Meta::default()` si no hay
/// datos. `GPS_BESTEFFORT` y no `GPS_FASTPROPERTIESONLY`: con este último el Shell no carga el
/// manejador de propiedades del archivo y no hay dimensiones ni duración. Sin `GPS_OPENSLOWITEM`,
/// los archivos que están solo en la nube (OneDrive) no se descargan para leerlos.
pub fn read(path: &Path) -> Meta {
    let Ok(store) = (unsafe { SHGetPropertyStoreFromParsingName::<_, _, IPropertyStore>(&HSTRING::from(path), None, GPS_BESTEFFORT) }) else {
        return Meta::default();
    };
    let get = |key: &PROPERTYKEY| unsafe { store.GetValue(key) }.ok().map(Value);
    let u32_of = |key: &PROPERTYKEY| get(key).and_then(|v| unsafe { PropVariantToUInt32(&v.0) }.ok()).filter(|n| *n > 0);
    let strings_of = |key: &PROPERTYKEY| get(key).map(|v| v.strings()).unwrap_or_default();
    let mut meta = Meta::default();
    let image = u32_of(&PKEY_Image_HorizontalSize).zip(u32_of(&PKEY_Image_VerticalSize));
    let video = || u32_of(&PKEY_Video_FrameWidth).zip(u32_of(&PKEY_Video_FrameHeight));
    if let Some((w, h)) = image.or_else(video) {
        (meta.width, meta.height) = (Some(w), Some(h));
    }
    // System.Media.Duration viene en unidades de 100 ns.
    meta.duration_ms = get(&PKEY_Media_Duration).and_then(|v| unsafe { PropVariantToUInt64(&v.0) }.ok()).filter(|d| *d > 0).map(|d| d / 10_000);
    meta.authors = strings_of(&PKEY_Author);
    meta.tags = strings_of(&PKEY_Keywords);
    let artists = strings_of(&PKEY_Music_Artist);
    meta.artist = (!artists.is_empty()).then(|| artists.join("; "));
    meta.album = get(&PKEY_Music_AlbumTitle).and_then(|v| v.string());
    meta
}

/// Un PROPVARIANT que se libera al salir.
struct Value(PROPVARIANT);

impl Drop for Value {
    fn drop(&mut self) {
        unsafe {
            let _ = PropVariantClear(&mut self.0);
        }
    }
}

impl Value {
    /// Lista de textos (un texto solo también vale), sin los vacíos.
    fn strings(&self) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let mut array: *mut PWSTR = std::ptr::null_mut();
            let mut count = 0u32;
            if PropVariantToStringVectorAlloc(&self.0, &mut array, &mut count).is_err() || array.is_null() {
                return out;
            }
            for i in 0..count as usize {
                let p = *array.add(i);
                if let Ok(s) = p.to_string()
                    && !s.is_empty()
                {
                    out.push(s);
                }
                CoTaskMemFree(Some(p.0 as *const _));
            }
            CoTaskMemFree(Some(array as *const _));
        }
        out
    }

    fn string(&self) -> Option<String> {
        unsafe {
            let p = PropVariantToStringAlloc(&self.0).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s.filter(|s| !s.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Un PNG de 3×2 píxeles (rojo), con bytes fijos (el mismo que en la prueba de macOS).
    const PNG_3X2: [u8; 73] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
        0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x41, 0xd2, 0x05, 0xfb, 0x87, 0xf0, 0xb9,
        0x48, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xdir-meta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn com() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
    }

    #[test]
    fn windows_reads_png_dimensions() {
        com();
        let dir = temp_dir("png");
        let png = dir.join("chica.png");
        std::fs::write(&png, PNG_3X2).unwrap();
        let meta = read(&png);
        assert_eq!((meta.width, meta.height), (Some(3), Some(2)));
        assert_eq!(meta.duration_ms, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un archivo que no existe, o sin metadatos, da `Meta::default()`.
    #[test]
    fn metadata_unknown_file_is_empty() {
        com();
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
        let ev = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("respuesta del hilo");
        assert_eq!((ev.panel, ev.generation), (1, 4));
        let LoadMsg::Metadata(batch) = ev.msg else { panic!("se esperaban metadatos") };
        assert_eq!((batch[0].0, batch[0].1.width), (9, Some(3)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! macOS: íconos de archivos de `NSWorkspace`. El índice de cada ícono (`Entry::icon`) sale de un
//! registro de claves ("carpeta", una extensión o una ruta con ícono propio) que se puede llenar
//! desde cualquier hilo; la imagen se pide recién al dibujar, en el hilo principal, y se cachea.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use objc2::rc::Retained;
use objc2_app_kit::{NSImage, NSWorkspace};
use objc2_foundation::NSString;

use crate::fs::LoadEvent;
use crate::platform::Handle;

/// De dónde sale un ícono.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum IconKey {
    /// Carpeta común.
    Folder,
    /// Archivo sin extensión conocida.
    File,
    /// Por extensión ("pdf").
    Ext(String),
    /// Ícono propio de ese elemento (apps, carpetas del usuario, discos).
    Path(PathBuf),
}

struct Registry {
    keys: Vec<IconKey>,
    index: HashMap<IconKey, i32>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry { keys: Vec::new(), index: HashMap::new() }))
}

/// Índice del ícono para esa clave (se registra la primera vez).
pub fn index_of(key: IconKey) -> i32 {
    let mut r = registry().lock().unwrap();
    if let Some(&i) = r.index.get(&key) {
        return i;
    }
    let i = r.keys.len() as i32;
    r.keys.push(key.clone());
    r.index.insert(key, i);
    i
}

thread_local! {
    static IMAGES: RefCell<HashMap<i32, Option<Retained<NSImage>>>> = RefCell::new(HashMap::new());
}

/// Imagen del ícono `index` (solo en el hilo principal).
pub fn image(index: i32) -> Option<Retained<NSImage>> {
    if let Some(hit) = IMAGES.with(|m| m.borrow().get(&index).cloned()) {
        return hit;
    }
    let key = registry().lock().unwrap().keys.get(index as usize).cloned();
    let ws = NSWorkspace::sharedWorkspace();
    #[allow(deprecated)]
    let img = key.map(|key| match key {
        IconKey::Folder => ws.iconForFileType(&NSString::from_str("public.folder")),
        IconKey::File => ws.iconForFileType(&NSString::from_str("public.data")),
        IconKey::Ext(ext) => ws.iconForFileType(&NSString::from_str(&ext)),
        IconKey::Path(p) => ws.iconForFile(&NSString::from_str(&p.to_string_lossy())),
    });
    IMAGES.with(|m| m.borrow_mut().insert(index, img.clone()));
    img
}

/// Pedido de superposiciones de íconos: en macOS no hay (las etiquetas de color del Finder van
/// aparte), así que no se hace nada.
#[allow(dead_code)]
pub struct OverlayRequest {
    pub hwnd: Handle,
    pub tx: Sender<LoadEvent>,
    pub panel: usize,
    pub generation: u64,
    /// (posición del elemento, ruta).
    pub items: Vec<(u32, PathBuf)>,
}

pub fn request_overlays(_req: OverlayRequest) {}

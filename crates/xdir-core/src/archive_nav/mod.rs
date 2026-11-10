//! Navegar dentro de un archivo comprimido (`Location::Archive`), solo lectura: qué hay en cada
//! carpeta de adentro (`children`, con las carpetas implícitas), su lectura en un hilo
//! (`spawn_load`, que nunca corre en el de interfaz) y las extracciones temporales de un solo
//! elemento para abrirlo o verlo (`temps`).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::archive::{ArchiveEntry, EntryKind, format_of, is_root_entry, safe_entry_path};
use crate::fs::{ATTR_ARCHIVE_BAD, ATTR_LINK, Entry, LoadEvent, LoadMsg, Location, WM_APP_LOAD};
use crate::sys::{self, CopyError, Flow, Handle};

pub mod temps;

#[cfg(test)]
mod tests;

/// El aviso de lo que escribe dentro de un archivo comprimido.
pub const READ_ONLY: &str = "El contenido de un archivo comprimido es solo de lectura";
/// Un elemento con nombre no permitido (sale de la carpeta, absoluto, reservado…).
pub const BAD_NAME: &str = "Ese elemento tiene un nombre no permitido y no se puede abrir ni copiar";
/// Los enlaces de adentro se muestran pero no se siguen.
pub const LINK: &str = "Los enlaces dentro de un archivo comprimido no se siguen";
/// La carpeta interior ya no está (el archivo cambió).
pub const NO_FOLDER: &str = "Esa carpeta ya no está en el archivo";

/// Más entradas que esto no se listan (el mismo límite con el que el motor pregunta antes de
/// extraer): un archivo con millones de entradas vacías no infla la memoria de la lista.
pub const MAX_ENTRIES: usize = crate::transfer::engine::BIG_ENTRIES;

/// Atributo de carpeta (`FILE_ATTRIBUTE_DIRECTORY`), como lo ponen las cargas del disco.
const ATTR_DIRECTORY: u32 = 0x10;
/// Segundos entre 1601 y 1970 (FILETIME).
const EPOCH_DIFF: u64 = 11_644_473_600;

/// Lo que se muestra en una carpeta de adentro.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Child {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub packed: u64,
    /// Segundos desde 1970 (0 si no figura o es una carpeta implícita).
    pub modified: u64,
    /// Enlace simbólico: se muestra, no se sigue.
    pub link: bool,
    /// Nombre no permitido (o un tipo que no se crea): se muestra en la raíz con su nombre crudo
    /// y no se abre ni se copia.
    pub bad: bool,
}

/// ¿Es de `__MACOSX/` (lo que agrega el Finder al comprimir)? No se muestra, como no se extrae.
fn is_mac_metadata(name: &str) -> bool {
    let n = name.trim_start_matches(['.', '/']).replace('\\', "/");
    n == "__MACOSX" || n.starts_with("__MACOSX/")
}

/// Componentes de una ruta interior saneada (`safe_entry_path`), como texto.
fn parts_of(rel: &Path) -> Vec<String> {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Lo que hay en la carpeta interior `inner` (con `/`, vacía en la raíz): una fila por nombre,
/// con las carpetas implícitas (entradas `a/b/c.txt` sin `a/`), sin `__MACOSX`. Las entradas con
/// nombre no permitido van a la raíz con su nombre crudo y `bad`. Mira como mucho `MAX_ENTRIES`
/// entradas.
pub fn children(entries: &[ArchiveEntry], inner: &str) -> Vec<Child> {
    let base: Vec<&str> = inner.split('/').filter(|p| !p.is_empty()).collect();
    let mut out: Vec<Child> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for e in entries.iter().take(MAX_ENTRIES) {
        if is_mac_metadata(&e.name) || (e.kind == EntryKind::Dir && is_root_entry(&e.name)) {
            continue;
        }
        let Some(rel) = safe_entry_path(&e.name) else {
            if base.is_empty() {
                out.push(Child { name: e.name.clone(), is_dir: false, size: e.size, packed: e.packed, modified: e.modified, link: false, bad: true });
            }
            continue;
        };
        let parts = parts_of(&rel);
        if parts.len() <= base.len() || parts.iter().zip(&base).any(|(a, b)| a != b) {
            continue;
        }
        let name = &parts[base.len()];
        let implicit = parts.len() > base.len() + 1;
        let child = if implicit || e.kind == EntryKind::Dir {
            Child { name: name.clone(), is_dir: true, size: 0, packed: 0, modified: if implicit { 0 } else { e.modified }, link: false, bad: false }
        } else {
            Child {
                name: name.clone(),
                is_dir: false,
                size: e.size,
                packed: e.packed,
                modified: e.modified,
                link: e.kind == EntryKind::Symlink,
                bad: e.kind == EntryKind::Omitted,
            }
        };
        match at.get(name) {
            // Una carpeta gana (una entrada «a/» y otras «a/x»: una sola fila); entre dos iguales,
            // la que trae fecha.
            Some(&i) => {
                let old = &mut out[i];
                if child.is_dir && !old.is_dir {
                    *old = child;
                } else if child.is_dir && old.is_dir && old.modified == 0 {
                    old.modified = child.modified;
                } else if !child.is_dir && !old.is_dir {
                    *old = child;
                }
            }
            None => {
                at.insert(name.clone(), out.len());
                out.push(child);
            }
        }
    }
    out
}

/// ¿Hay algo dentro de `inner`? (la raíz siempre existe)
pub fn folder_exists(entries: &[ArchiveEntry], inner: &str) -> bool {
    if inner.is_empty() {
        return true;
    }
    let base: Vec<&str> = inner.split('/').collect();
    entries.iter().take(MAX_ENTRIES).any(|e| {
        safe_entry_path(&e.name).is_some_and(|rel| {
            let parts = parts_of(&rel);
            parts.len() >= base.len() && parts.iter().zip(&base).all(|(a, b)| a == b) && (parts.len() > base.len() || e.kind == EntryKind::Dir)
        })
    })
}

/// ¿El nombre es de un formato que se sabe abrir como carpeta (un zip dentro de otro)?
pub fn is_navigable_archive(name: &str) -> bool {
    format_of(name).is_some_and(|f| sys::archives().formats().contains(&f))
}

/// ¿Ese error de listar se arregla abriéndolo con el sistema? (contraseña, formato que no se
/// sabe leer o archivo dañado).
pub fn offers_system(err: &str) -> bool {
    err == sys::PASSWORD || err == sys::UNSUPPORTED || err.contains("dañado")
}

/// La fila de la lista de un elemento de adentro.
pub fn entry_of(c: &Child) -> Entry {
    let (icon, type_name) = sys::file_system().archive_look(&c.name, c.is_dir);
    let mut attrs = if c.is_dir { ATTR_DIRECTORY } else { 0 };
    if c.link {
        attrs |= ATTR_LINK;
    }
    if c.bad {
        attrs |= ATTR_ARCHIVE_BAD;
    }
    Entry {
        name: OsString::from(&c.name),
        full: None,
        group: 0,
        pidl: None,
        display: c.name.clone(),
        sort_key: c.name.to_lowercase(),
        is_dir: c.is_dir,
        size: c.size,
        modified: if c.modified == 0 { 0 } else { (c.modified + EPOCH_DIFF) * 10_000_000 },
        created: 0,
        attrs,
        type_name,
        icon,
        overlay: 0,
        overlay_checked: true,
        packed: c.packed,
    }
}

// ---------- listado en un hilo ----------

/// Los últimos listados, por archivo (con su tamaño y fecha: si cambió, se vuelve a leer). Entrar
/// y salir de carpetas de un zip grande no lo vuelve a leer cada vez.
type Listing = (PathBuf, (u64, u64), Arc<Vec<ArchiveEntry>>);
static LISTINGS: Mutex<Vec<Listing>> = Mutex::new(Vec::new());
const LISTINGS_KEPT: usize = 4;

/// Las entradas del archivo `real` (del disco o una extracción temporal), con la caché.
pub fn list_cached(real: &Path, cancel: &mut dyn FnMut() -> Flow) -> Result<Arc<Vec<ArchiveEntry>>, CopyError> {
    let stat = sys::copy_prims().stat(real)?;
    let stamp = (stat.size, stat.modified);
    {
        let cache = LISTINGS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, _, list)) = cache.iter().find(|(p, s, _)| p == real && *s == stamp) {
            return Ok(list.clone());
        }
    }
    let list = Arc::new(sys::archives().list(real, cancel)?);
    let mut cache = LISTINGS.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(p, _, _)| p != real);
    cache.insert(0, (real.to_path_buf(), stamp, list.clone()));
    cache.truncate(LISTINGS_KEPT);
    Ok(list)
}

/// Lee la carpeta de adentro `location` en un hilo y manda sus filas como una carga más
/// (`LoadMsg::Entries` y `Listed`). Un archivo anidado se extrae primero a la temporal. Se
/// abandona si `current` deja de valer `generation`. Con contraseña, el error es
/// `sys::PASSWORD` (la `App` ofrece abrirlo con el sistema).
pub fn spawn_load(hwnd: Handle, tx: Sender<LoadEvent>, panel: usize, generation: u64, current: Arc<AtomicU64>, location: Location) {
    std::thread::spawn(move || {
        let send = |msg: LoadMsg| {
            if tx.send(LoadEvent { panel, generation, msg }).is_ok() {
                hwnd.post(WM_APP_LOAD, panel, 0);
            }
        };
        let Location::Archive { file, nested, inner } = &location else {
            return send(LoadMsg::Listed(Err("Esta ubicación no existe".into())));
        };
        match read(file, nested, inner, &|| current.load(Ordering::Relaxed) != generation) {
            Ok(rows) => {
                if current.load(Ordering::Relaxed) != generation {
                    return;
                }
                // Los buenos llevan su ruta temporal (de ahí salen miniaturas, vista previa y vista
                // rápida); los de nombre no permitido no tienen ninguna ruta del disco.
                let temps = temps::global();
                let with_path = |c: &Child| {
                    let mut e = entry_of(c);
                    if !c.bad {
                        e.full = temps.entry_path(file, nested, &crate::fs::inner_join(inner, &c.name)).map(PathBuf::into_os_string);
                    }
                    e
                };
                for chunk in rows.chunks(4096) {
                    send(LoadMsg::Entries(chunk.iter().map(with_path).collect()));
                }
                send(LoadMsg::Listed(Ok(())));
            }
            Err(e) if e.kind == sys::ErrKind::Cancelled => {}
            Err(e) => send(LoadMsg::Listed(Err(e.message()))),
        }
    });
}

/// Resuelve el archivo (con sus anidados), lo lista y arma la carpeta `inner`.
fn read(file: &Path, nested: &[String], inner: &str, cancelled: &dyn Fn() -> bool) -> Result<Vec<Child>, CopyError> {
    let real = temps::global().resolve(file, nested)?;
    let entries = list_cached(&real, &mut || if cancelled() { Flow::Stop } else { Flow::Continue })?;
    if entries.iter().any(|e| e.encrypted) {
        return Err(CopyError::new(sys::ErrKind::Other(sys::PASSWORD.into()), &real));
    }
    if !folder_exists(&entries, inner) {
        return Err(CopyError::new(sys::ErrKind::Other(NO_FOLDER.into()), &real));
    }
    Ok(children(&entries, inner))
}

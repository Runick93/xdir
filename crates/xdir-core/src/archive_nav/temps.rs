//! Carpeta temporal propia de xdir para lo que se extrae de un archivo comprimido sin que el
//! usuario lo pida (abrir un archivo de adentro, verlo en la vista previa, entrar en un zip
//! anidado). Una carpeta por ejecución, con un candado (`File::try_lock`) que dura lo que dura
//! xdir: al arrancar se borran las de ejecuciones que ya no están (su candado se puede tomar) y
//! al salir, la propia. Adentro, una carpeta por archivo comprimido (por su ruta y su cadena de
//! anidados) con lo extraído en su ruta interior, así una ruta temporal se sabe sin tocar el
//! disco (`entry_path`).
//!
//! Lo que se extrae pasa por `sys::archives()` y se mira con `sys::copy_prims()` (en las pruebas,
//! el disco falso); la carpeta de la ejecución y su candado son del disco real.

use std::collections::HashMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::archive::safe_entry_path;
use crate::sys::{self, CopyError, ErrKind, Flow, ItemKind};

/// Nombre del candado dentro de la carpeta de cada ejecución.
const LOCK: &str = ".candado";

pub struct Temps {
    base: PathBuf,
    run: PathBuf,
    /// El candado de esta ejecución (abierto y tomado mientras viva).
    lock: Mutex<Option<File>>,
    /// Tamaño y fecha de cada archivo cuando se extrajo de él: si cambió, lo viejo se borra.
    stamps: Mutex<HashMap<PathBuf, (u64, u64)>>,
}

/// Las temporales del proceso, en `<temp del sistema>/xdir-comprimidos`.
pub fn global() -> &'static Temps {
    static GLOBAL: OnceLock<Temps> = OnceLock::new();
    GLOBAL.get_or_init(|| Temps::new(&std::env::temp_dir().join("xdir-comprimidos")))
}

/// Al arrancar: borra lo que quedó de ejecuciones anteriores (en un hilo: no frena el arranque).
pub fn clean_at_start() {
    std::thread::spawn(|| global().clean_stale());
}

/// Al salir: borra la carpeta de esta ejecución.
pub fn clean_at_exit() {
    global().clean_own();
}

impl Temps {
    /// Las temporales bajo `base` (se crean al primer uso).
    pub fn new(base: &Path) -> Temps {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let run = base.join(format!("{}-{nanos}", std::process::id()));
        Temps { base: base.to_path_buf(), run, lock: Mutex::new(None), stamps: Mutex::new(HashMap::new()) }
    }

    /// La carpeta de esta ejecución, creada y con su candado tomado.
    pub fn run_dir(&self) -> &Path {
        let mut lock = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if lock.is_none() {
            let _ = std::fs::create_dir_all(&self.run);
            if let Ok(f) = File::create(self.run.join(LOCK)) {
                let _ = f.try_lock();
                *lock = Some(f);
            }
        }
        &self.run
    }

    /// Borra las carpetas de ejecuciones que ya no están: las que tienen un candado que se puede
    /// tomar (las de un xdir abierto, no). Una sin candado se borra si tiene más de un minuto (la
    /// de una ejecución que justo está arrancando, no).
    pub fn clean_stale(&self) {
        let Ok(dirs) = std::fs::read_dir(&self.base) else { return };
        for d in dirs.flatten() {
            let path = d.path();
            if path == self.run || !path.is_dir() {
                continue;
            }
            let stale = match File::open(path.join(LOCK)) {
                Ok(f) => {
                    let free = f.try_lock().is_ok();
                    drop(f);
                    free
                }
                Err(_) => d.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e.as_secs() > 60)),
            };
            if stale {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }

    /// Borra la carpeta de esta ejecución (suelta el candado antes: en Windows no se borra un
    /// archivo abierto).
    pub fn clean_own(&self) {
        let mut lock = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        *lock = None;
        let _ = std::fs::remove_dir_all(&self.run);
        self.stamps.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// La carpeta de lo extraído del archivo `file` con la cadena `nested` (sin tocar el disco).
    pub fn dir_for(&self, file: &Path, nested: &[String]) -> PathBuf {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        file.hash(&mut h);
        nested.hash(&mut h);
        self.run.join(format!("{:016x}", h.finish()))
    }

    /// Dónde queda (o quedaría) extraído el elemento `rel` (ruta interior con `/`), sin tocar el
    /// disco. `None` si el nombre no es seguro.
    pub fn entry_path(&self, file: &Path, nested: &[String], rel: &str) -> Option<PathBuf> {
        Some(self.dir_for(file, nested).join(safe_entry_path(rel)?))
    }

    /// El archivo comprimido de verdad: `file`, o la extracción temporal del último anidado.
    /// Bloqueante (hilos de trabajo).
    pub fn resolve(&self, file: &Path, nested: &[String]) -> Result<PathBuf, CopyError> {
        match nested.split_last() {
            None => Ok(file.to_path_buf()),
            Some((last, outer)) => self.ensure(file, outer, last),
        }
    }

    /// Extrae (si no estaba ya, o si el archivo cambió) el elemento `rel` del archivo `file` con
    /// la cadena `nested` y devuelve su ruta temporal. Bloqueante (hilos de trabajo); de a una
    /// extracción a la vez, así dos pedidos del mismo elemento no se pisan.
    pub fn ensure(&self, file: &Path, nested: &[String], rel: &str) -> Result<PathBuf, CopyError> {
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure_locked(file, nested, rel)
    }

    fn ensure_locked(&self, file: &Path, nested: &[String], rel: &str) -> Result<PathBuf, CopyError> {
        let real = match nested.split_last() {
            None => file.to_path_buf(),
            Some((last, outer)) => self.ensure_locked(file, outer, last)?,
        };
        let not_allowed = || CopyError::new(ErrKind::Other(sys::NOT_ALLOWED.into()), Path::new(rel));
        let target = safe_entry_path(rel).ok_or_else(not_allowed)?;
        self.run_dir();
        let prims = sys::copy_prims();
        let dir = self.dir_for(file, nested);
        let stat = prims.stat(&real)?;
        let stamp = (stat.size, stat.modified);
        {
            let mut stamps = self.stamps.lock().unwrap_or_else(|e| e.into_inner());
            if stamps.get(&dir) != Some(&stamp) {
                remove_tree(&dir);
                stamps.insert(dir.clone(), stamp);
            }
        }
        let dst = dir.join(&target);
        if prims.stat(&dst).is_ok() {
            return Ok(dst);
        }
        if prims.stat(&dir).is_err() {
            // La de la ejecución ya está en el disco real; en el falso de las pruebas se crea acá.
            for up in dir.ancestors().skip(1).collect::<Vec<_>>().into_iter().rev() {
                if prims.stat(up).is_err() {
                    prims.create_dir(up)?;
                }
            }
            prims.create_private_dir(&dir)?;
        }
        let mut failed = None;
        let want = |name: &str| safe_entry_path(name).as_deref() == Some(target.as_path());
        sys::archives().extract_some(&real, &dir, &want, &mut |_| Flow::Continue, &mut |_, e| failed = Some(e))?;
        match prims.stat(&dst) {
            Ok(_) => Ok(dst),
            Err(e) => Err(failed.unwrap_or(e)),
        }
    }
}

/// Borra `p` y lo de adentro (sin seguir enlaces), con las primitivas del sistema.
fn remove_tree(p: &Path) {
    let prims = sys::copy_prims();
    let Ok(st) = prims.stat(p) else { return };
    if st.kind == ItemKind::Dir {
        for name in prims.read_dir(p).unwrap_or_default() {
            remove_tree(&p.join(name));
        }
    }
    let _ = prims.remove(p);
}

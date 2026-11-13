//! Carpeta temporal propia de xdir para lo que se extrae de un archivo comprimido sin que el
//! usuario lo pida (abrir un archivo de adentro, verlo en la vista previa, entrar en un zip
//! anidado). Una carpeta por ejecución, con un candado (`File::try_lock`) que dura lo que dura
//! xdir: al arrancar se borran las de ejecuciones que ya no están (su candado se puede tomar) y
//! al salir, la propia. Adentro, una carpeta por archivo comprimido (por su ruta y su cadena de
//! anidados) con lo extraído en su ruta interior, así una ruta temporal se sabe sin tocar el
//! disco (`entry_path`).
//!
//! Lo que se extrae pasa por `sys::archives()` y se mira con `sys::copy_prims()` (en las pruebas,
//! el disco falso; o los que se le den con `Temps::with_services`); la carpeta de la ejecución y su
//! candado son del disco real.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::archive::safe_entry_path;
use crate::sys::{self, Archives, CopyError, CopyPrims, ErrKind, Flow, ItemKind, Pick};

/// Nombre del candado dentro de la carpeta de cada ejecución.
const LOCK: &str = ".candado";

/// Lo más que se extrae para abrir un archivo de adentro o entrar en un archivo anidado (la vista
/// previa usa su propio límite, `thumbs::BIG_CACHE_BYTES`).
pub const OPEN_MAX: u64 = 4 << 30;
/// Más que el límite.
pub const TOO_BIG: &str = "Es demasiado grande para abrirlo desde adentro: usá «Extraer en…»";
/// No entra en el disco de la temporal.
pub const NO_ROOM: &str = "No hay lugar en el disco para extraerlo";

/// Límites de una extracción temporal.
pub struct Limits<'a> {
    /// Bytes que puede declarar (y escribir) el elemento.
    pub max: u64,
    /// `true` corta (se dejó la carpeta, se movió el foco, se canceló).
    pub cancel: &'a dyn Fn() -> bool,
}

fn never() -> bool {
    false
}

impl Limits<'static> {
    /// Hasta `max`, sin cancelar.
    pub fn up_to(max: u64) -> Limits<'static> {
        Limits { max, cancel: &never }
    }
}

/// Texto del error cuando no hay una carpeta temporal segura.
pub const NO_SAFE_TEMP: &str = "No hay una carpeta temporal segura para extraer";

pub struct Temps {
    /// La carpeta del usuario que da la plataforma (`None`: no hay una segura).
    base: Option<PathBuf>,
    run: PathBuf,
    /// El candado de esta ejecución (abierto y tomado mientras viva).
    lock: Mutex<Option<File>>,
    /// Tamaño y fecha de cada archivo cuando se extrajo de él: si cambió, lo viejo se borra.
    stamps: Mutex<HashMap<PathBuf, (u64, u64)>>,
    /// Lo que extrajo esta ejecución (lo único que se reusa).
    done: Mutex<HashSet<PathBuf>>,
    /// Un candado por carpeta de archivo comprimido.
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    /// Primitivas y servicio propios (pruebas de una plataforma); sin ellos, los del sistema.
    services: Option<(&'static dyn CopyPrims, &'static dyn Archives)>,
}

/// Las temporales del proceso, en la carpeta que da la plataforma
/// (`SystemEnv::archive_temp_dir`: propia del usuario, privada y verificada).
pub fn global() -> &'static Temps {
    static GLOBAL: OnceLock<Temps> = OnceLock::new();
    GLOBAL.get_or_init(|| Temps::new(sys::env().archive_temp_dir().as_deref()))
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
    /// Las temporales bajo `base`, que ya tiene que estar verificada (se crean al primer uso).
    /// Sin `base` no hay temporales: todo pedido falla con `NO_SAFE_TEMP`.
    pub fn new(base: Option<&Path>) -> Temps {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let run = base.map(|b| b.join(format!("{}-{nanos}", std::process::id()))).unwrap_or_default();
        Temps {
            base: base.map(Path::to_path_buf),
            run,
            lock: Mutex::new(None),
            stamps: Mutex::new(HashMap::new()),
            done: Mutex::new(HashSet::new()),
            locks: Mutex::new(HashMap::new()),
            services: None,
        }
    }

    /// Como `new`, con primitivas y servicio propios en lugar de los del sistema (para probar las
    /// de una plataforma sin instalarlas).
    pub fn with_services(base: Option<&Path>, prims: &'static dyn CopyPrims, archives: &'static dyn Archives) -> Temps {
        Temps { services: Some((prims, archives)), ..Temps::new(base) }
    }

    fn prims(&self) -> &dyn CopyPrims {
        self.services.map_or(sys::copy_prims(), |s| s.0)
    }

    fn archives(&self) -> &dyn Archives {
        self.services.map_or(sys::archives(), |s| s.1)
    }

    fn unsafe_base() -> CopyError {
        CopyError::new(ErrKind::Other(NO_SAFE_TEMP.into()), Path::new(""))
    }

    /// La carpeta de esta ejecución, creada (nueva: `create_dir` no sigue un enlace que ya esté
    /// ahí) y con su candado tomado. `None` sin carpeta segura o si no se pudo crear.
    pub fn run_dir(&self) -> Option<&Path> {
        self.base.as_ref()?;
        let mut lock = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if lock.is_none() {
            std::fs::create_dir(&self.run).ok()?;
            let f = File::create_new(self.run.join(LOCK)).ok()?;
            let _ = f.try_lock();
            *lock = Some(f);
        }
        Some(&self.run)
    }

    /// Borra las carpetas de ejecuciones que ya no están: las que tienen un candado que se puede
    /// tomar (las de un xdir abierto, no) o no tienen candado. Nunca una de menos de un minuto:
    /// puede ser de una ejecución que está arrancando y todavía no tomó el suyo.
    pub fn clean_stale(&self) {
        self.clean_stale_older_than(std::time::Duration::from_secs(60));
    }

    /// `clean_stale` con otra edad mínima (las pruebas).
    pub fn clean_stale_older_than(&self, min_age: std::time::Duration) {
        let Some(base) = &self.base else { return };
        let Ok(dirs) = std::fs::read_dir(base) else { return };
        for d in dirs.flatten() {
            let path = d.path();
            if path == self.run || !path.is_dir() {
                continue;
            }
            let old = d.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e >= min_age));
            if !old {
                continue;
            }
            let stale = match File::open(path.join(LOCK)) {
                Ok(f) => {
                    let free = f.try_lock().is_ok();
                    drop(f);
                    free
                }
                Err(_) => true,
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
        if lock.take().is_none() {
            return;
        }
        // Lo extraído pasó por las primitivas del sistema (en las pruebas, el disco falso).
        self.remove_tree(&self.run);
        let _ = std::fs::remove_dir_all(&self.run);
        self.stamps.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.done.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
        self.base.as_ref()?;
        Some(self.dir_for(file, nested).join(safe_entry_path(rel)?))
    }

    /// El archivo comprimido de verdad: `file`, o la extracción temporal del último anidado
    /// (hasta `OPEN_MAX`). Bloqueante (hilos de trabajo); `cancel` corta.
    pub fn resolve(&self, file: &Path, nested: &[String], cancel: &dyn Fn() -> bool) -> Result<PathBuf, CopyError> {
        match nested.split_last() {
            None => Ok(file.to_path_buf()),
            Some((last, outer)) => self.ensure(file, outer, last, &Limits { max: OPEN_MAX, cancel }),
        }
    }

    /// Extrae (si no estaba ya, o si el archivo cambió) el elemento `rel` del archivo `file` con
    /// la cadena `nested` y devuelve su ruta temporal. Bloqueante (hilos de trabajo). Se extrae la
    /// entrada que muestra la lista (la última con ese nombre), solo si declara hasta `lim.max`
    /// bytes y entra en el disco, cortando si escribe más o si `lim.cancel` lo pide; un tar.gz se
    /// deja de leer apenas llega. De a una extracción por archivo comprimido (dos pedidos del
    /// mismo elemento no se pisan; los de otros archivos no esperan).
    pub fn ensure(&self, file: &Path, nested: &[String], rel: &str, lim: &Limits) -> Result<PathBuf, CopyError> {
        let real = match nested.split_last() {
            None => file.to_path_buf(),
            Some((last, outer)) => self.ensure(file, outer, last, &Limits { max: OPEN_MAX, cancel: lim.cancel })?,
        };
        let not_allowed = || CopyError::new(ErrKind::Other(sys::NOT_ALLOWED.into()), Path::new(rel));
        let target = safe_entry_path(rel).ok_or_else(not_allowed)?;
        self.run_dir().ok_or_else(Temps::unsafe_base)?;
        let prims = self.prims();
        let dir = self.dir_for(file, nested);
        let lock = self.lock_for(&dir);
        let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
        let stat = prims.stat(&real)?;
        let stamp = (stat.size, stat.modified);
        {
            let mut stamps = self.stamps.lock().unwrap_or_else(|e| e.into_inner());
            if stamps.get(&dir) != Some(&stamp) {
                self.remove_tree(&dir);
                // Y las carpetas de los gemelos (`<hash>~<n>`).
                let prefix = format!("{}~", dir.file_name().unwrap_or_default().to_string_lossy());
                for name in prims.read_dir(&self.run).unwrap_or_default() {
                    if name.to_string_lossy().starts_with(&prefix) {
                        self.remove_tree(&self.run.join(name));
                    }
                }
                self.done.lock().unwrap_or_else(|e| e.into_inner()).retain(|p| !p.starts_with(&dir) && !p.to_string_lossy().contains(&prefix));
                stamps.insert(dir.clone(), stamp);
            }
        }
        let dst = dir.join(&target);
        // Solo lo que extrajo esta ejecución (en un disco sin mayúsculas, «A.txt» no es «a.txt»).
        if self.done.lock().unwrap_or_else(|e| e.into_inner()).contains(&dst) && prims.stat(&dst).is_ok() {
            return Ok(dst);
        }
        let entries = super::list_cached_with(prims, self.archives(), &real, &mut || if (lim.cancel)() { Flow::Stop } else { Flow::Continue })?;
        let Some(entry) = entries.iter().rev().find(|e| safe_entry_path(&e.name).as_deref() == Some(target.as_path())) else {
            return Err(CopyError::new(ErrKind::NotFound, &dst));
        };
        if entry.size > lim.max {
            return Err(CopyError::new(ErrKind::Other(TOO_BIG.into()), &dst));
        }
        if let Some(v) = prims.volume(&self.run)
            && entry.size > v.free
        {
            return Err(CopyError::new(ErrKind::Other(NO_ROOM.into()), &dst));
        }
        // En la carpeta del archivo ya hay otra cosa con este nombre (un gemelo que solo cambia en
        // mayúsculas o en la forma de los acentos, en un disco que no los distingue): este va a una
        // carpeta propia de la entrada, así cada uno se abre con lo suyo.
        let index = entry.index;
        let (root, dst) = if prims.stat(&dst).is_ok() {
            let alt = self.twin_dir(&dir, index);
            let at = alt.join(&target);
            if self.done.lock().unwrap_or_else(|e| e.into_inner()).contains(&at) && prims.stat(&at).is_ok() {
                return Ok(at);
            }
            (alt, at)
        } else {
            (dir.clone(), dst)
        };
        let dir = root;
        if prims.stat(&dir).is_err() {
            // La de la ejecución ya está en el disco real; en el falso de las pruebas se crea acá.
            // Otra extracción a la vez (otro archivo, otro candado) puede crear la misma carpeta de
            // arriba entre `stat` y `create_dir`: si ya es una carpeta, sigue.
            for up in dir.ancestors().skip(1).collect::<Vec<_>>().into_iter().rev() {
                if prims.stat(up).is_err()
                    && let Err(e) = prims.create_dir(up)
                    && !(e.kind == ErrKind::Exists && prims.stat(up).is_ok_and(|st| st.kind == ItemKind::Dir))
                {
                    return Err(e);
                }
            }
            prims.create_private_dir(&dir)?;
        }
        let mut failed = None;
        let mut too_big = false;
        let res = self.archives().extract_some(
            &real,
            &dir,
            &|i, _| if i == index { Pick::Last } else { Pick::Skip },
            &mut |b| {
                too_big |= b > lim.max;
                if too_big || (lim.cancel)() { Flow::Stop } else { Flow::Continue }
            },
            &mut |_, e| failed = Some(e),
        );
        if too_big {
            self.remove_tree(&dst);
            return Err(CopyError::new(ErrKind::Other(TOO_BIG.into()), &dst));
        }
        // Cortada o fallida (también justo después de escribir el archivo): no queda nada. Si
        // quedara, la próxima vez estaría en su lugar sin estar anotada y se la tomaría por un
        // gemelo (otra carpeta, otra ruta).
        if let Err(e) = res {
            self.remove_tree(&dst);
            return Err(e);
        }
        // Una entrada que no se pudo escribir (ya había algo, ruta no permitida…) es un error, nunca
        // lo que haya en ese lugar.
        if let Some(e) = failed {
            return Err(e);
        }
        match prims.stat(&dst) {
            Ok(_) => {
                self.done.lock().unwrap_or_else(|e| e.into_inner()).insert(dst.clone());
                // El origen del archivo (marca de la web, cuarentena) pasa a lo extraído y a las
                // carpetas que se crearon para llegar.
                let written: Vec<PathBuf> = dst.ancestors().take_while(|p| *p != dir).map(Path::to_path_buf).collect();
                self.archives().propagate_origin(&real, &written);
                Ok(dst)
            }
            Err(e) => Err(e),
        }
    }

    /// La carpeta propia de la entrada `index` cuando su nombre choca con otro (al lado de la del
    /// archivo, `<hash>~<index>`).
    fn twin_dir(&self, dir: &Path, index: usize) -> PathBuf {
        let mut name = dir.file_name().unwrap_or_default().to_os_string();
        name.push(format!("~{index}"));
        dir.with_file_name(name)
    }

    /// El candado de las extracciones de la carpeta `dir` (una por archivo comprimido).
    fn lock_for(&self, dir: &Path) -> Arc<Mutex<()>> {
        self.locks.lock().unwrap_or_else(|e| e.into_inner()).entry(dir.to_path_buf()).or_default().clone()
    }

    /// Borra `p` y lo de adentro (sin seguir enlaces), con las primitivas del sistema.
    fn remove_tree(&self, p: &Path) {
        let prims = self.prims();
        let Ok(st) = prims.stat(p) else { return };
        if st.kind == ItemKind::Dir {
            for name in prims.read_dir(p).unwrap_or_default() {
                self.remove_tree(&p.join(name));
            }
        }
        let _ = prims.remove(p);
    }
}

//! `FakeCopyPrims`: un disco en memoria para probar el motor de copia sin tocar el disco real.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use crate::sys::{CopyError, CopyPrims, ErrKind, Flow, ItemKind, ItemStat, VolumeInfo};

#[derive(Clone, Debug)]
enum Node {
    Dir { modified: u64 },
    /// `size`: el tamaño que informa `stat` si no es el del contenido (archivos «grandes»).
    File { data: Vec<u8>, modified: u64, cloud: bool, size: Option<u64> },
    Link { target: PathBuf },
}

/// Algo mandado a la Papelera: su nombre ahí, de dónde vino y el subárbol (rutas relativas a él;
/// la vacía es el elemento mismo).
type Trashed = (String, PathBuf, Vec<(PathBuf, Node)>);

#[derive(Default)]
struct FakeFs {
    nodes: BTreeMap<PathBuf, Node>,
    volumes: Vec<(PathBuf, VolumeInfo)>,
    fails: HashMap<PathBuf, (ErrKind, u32)>,
    fail_after: HashMap<PathBuf, (u64, ErrKind)>,
    short: HashMap<PathBuf, u64>,
    block: Option<u64>,
    delay: Option<Duration>,
    slow: HashMap<PathBuf, Duration>,
    reads: HashMap<PathBuf, u32>,
    trash: Vec<Trashed>,
    origins: HashMap<PathBuf, PathBuf>,
    trash_count: u32,
}

fn default_volume() -> VolumeInfo {
    VolumeInfo { id: "fake".into(), label: "Disco".into(), case_insensitive: false, has_trash: true, max_file: None, free: u64::MAX }
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

impl FakeFs {
    fn volume(&self, p: &Path) -> VolumeInfo {
        self.volumes
            .iter()
            .filter(|(prefix, _)| p.starts_with(prefix))
            .max_by_key(|(prefix, _)| prefix.components().count())
            .map_or_else(default_volume, |(_, v)| v.clone())
    }

    /// La ruta guardada que corresponde a `p` (sin distinguir mayúsculas en los discos que no
    /// las distinguen).
    fn key(&self, p: &Path) -> Option<PathBuf> {
        if self.nodes.contains_key(p) {
            return Some(p.to_path_buf());
        }
        if !self.volume(p).case_insensitive {
            return None;
        }
        let want = lower(p);
        self.nodes.keys().find(|k| lower(k) == want).cloned()
    }

    /// Un fallo pendiente para `p`.
    fn check_fail(&mut self, p: &Path) -> Result<(), CopyError> {
        if let Some((kind, times)) = self.fails.get_mut(p)
            && *times > 0
        {
            *times -= 1;
            return Err(CopyError::new(kind.clone(), p));
        }
        Ok(())
    }

    fn need_parent_dir(&self, p: &Path) -> Result<(), CopyError> {
        let Some(parent) = p.parent() else { return Ok(()) };
        match self.key(parent).and_then(|k| self.nodes.get(&k)) {
            Some(Node::Dir { .. }) => Ok(()),
            _ => Err(CopyError::new(ErrKind::NotFound, p)),
        }
    }

    /// `p` y todo lo que tiene adentro (claves reales).
    fn subtree(&self, key: &Path) -> Vec<PathBuf> {
        self.nodes.keys().filter(|k| k.starts_with(key)).cloned().collect()
    }

    fn move_tree(&mut self, from: &Path, to: &Path) {
        for k in self.subtree(from) {
            let node = self.nodes.remove(&k).unwrap();
            let rel = k.strip_prefix(from).unwrap();
            let dst = if rel.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rel) };
            self.nodes.insert(dst, node);
        }
    }

    fn add_parents(&mut self, p: &Path) {
        let mut cur = p.parent();
        while let Some(dir) = cur {
            if dir.as_os_str().is_empty() {
                break;
            }
            self.nodes.entry(dir.to_path_buf()).or_insert(Node::Dir { modified: 0 });
            cur = dir.parent();
        }
    }
}

/// Primitivas de copia falsas: un disco en memoria. `new()` arma uno propio (para probar el motor
/// directamente) y `shared()` es el de todo el proceso, el que instala `install_fakes` (las
/// pruebas de la `App` usan rutas propias, como con `FakeDiskScan::serve`). Los volúmenes se fijan
/// por prefijo de ruta; sin ninguno, todo está en un mismo disco con Papelera y espacio de sobra.
#[derive(Clone, Default)]
pub struct FakeCopyPrims {
    fs: Arc<Mutex<FakeFs>>,
}

impl FakeCopyPrims {
    pub fn new() -> FakeCopyPrims {
        FakeCopyPrims::default()
    }

    /// El disco falso de todo el proceso (el de `sys::copy_prims()` en las pruebas).
    pub fn shared() -> FakeCopyPrims {
        static SHARED: OnceLock<FakeCopyPrims> = OnceLock::new();
        SHARED.get_or_init(FakeCopyPrims::new).clone()
    }

    fn fs(&self) -> MutexGuard<'_, FakeFs> {
        self.fs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Crea la carpeta (y las de arriba).
    pub fn add_dir(&self, path: &Path) {
        let mut fs = self.fs();
        fs.add_parents(path);
        fs.nodes.insert(path.to_path_buf(), Node::Dir { modified: 0 });
    }

    /// Crea el archivo con ese contenido (y las carpetas de arriba).
    pub fn add_file(&self, path: &Path, bytes: Vec<u8>) {
        let mut fs = self.fs();
        fs.add_parents(path);
        fs.nodes.insert(path.to_path_buf(), Node::File { data: bytes, modified: 0, cloud: false, size: None });
    }

    pub fn add_symlink(&self, path: &Path, target: &Path) {
        let mut fs = self.fs();
        fs.add_parents(path);
        fs.nodes.insert(path.to_path_buf(), Node::Link { target: target.to_path_buf() });
    }

    pub fn set_modified(&self, path: &Path, modified: u64) {
        let mut fs = self.fs();
        match fs.nodes.get_mut(path) {
            Some(Node::File { modified: m, .. }) | Some(Node::Dir { modified: m }) => *m = modified,
            _ => {}
        }
    }

    /// El tamaño que informa `stat` (sin guardar tanto contenido); copiarlo copia el contenido.
    pub fn set_size(&self, path: &Path, bytes: u64) {
        if let Some(Node::File { size, .. }) = self.fs().nodes.get_mut(path) {
            *size = Some(bytes);
        }
    }

    pub fn set_cloud_only(&self, path: &Path) {
        if let Some(Node::File { cloud, .. }) = self.fs().nodes.get_mut(path) {
            *cloud = true;
        }
    }

    /// El disco de las rutas que empiezan con `prefix`.
    pub fn set_volume(&self, prefix: &Path, info: VolumeInfo) {
        self.fs().volumes.push((prefix.to_path_buf(), info));
    }

    /// Las próximas `times` operaciones sobre `path` fallan con `kind` (copiar desde o hacia,
    /// crear, renombrar, reemplazar, borrar, Papelera o listar; no `stat`).
    pub fn fail(&self, path: &Path, kind: ErrKind, times: u32) {
        self.fs().fails.insert(path.to_path_buf(), (kind, times));
    }

    /// Copiar desde `path` escribe `bytes` y falla con `kind` (borrando lo escrito).
    pub fn fail_after(&self, path: &Path, bytes: u64, kind: ErrKind) {
        self.fs().fail_after.insert(path.to_path_buf(), (bytes, kind));
    }

    /// Copiar desde `path` «termina bien» pero escribe solo `bytes` (una copia que no quedó
    /// igual, para probar la verificación al mover).
    pub fn short_copy(&self, path: &Path, bytes: u64) {
        self.fs().short.insert(path.to_path_buf(), bytes);
    }

    /// Tamaño de bloque de la copia (un aviso de progreso por bloque). Por defecto 64 KiB.
    pub fn set_block(&self, n: u64) {
        self.fs().block = Some(n.max(1));
    }

    /// Demora por bloque copiado.
    pub fn set_delay(&self, delay: Option<Duration>) {
        self.fs().delay = delay;
    }

    /// Demora por bloque al copiar desde `path` (para el disco de todo el proceso, donde
    /// `set_delay` alcanzaría a las otras pruebas). `None` la quita.
    pub fn set_slow(&self, path: &Path, delay: Option<Duration>) {
        let mut fs = self.fs();
        match delay {
            Some(d) => fs.slow.insert(path.to_path_buf(), d),
            None => fs.slow.remove(path),
        };
    }

    /// Lugar original de un elemento de la Papelera (para «Devolver»).
    pub fn set_trash_origin(&self, in_trash: &Path, original: &Path) {
        self.fs().origins.insert(in_trash.to_path_buf(), original.to_path_buf());
    }

    pub fn contents(&self, path: &Path) -> Option<Vec<u8>> {
        let fs = self.fs();
        match fs.key(path).and_then(|k| fs.nodes.get(&k)) {
            Some(Node::File { data, .. }) => Some(data.clone()),
            _ => None,
        }
    }

    pub fn exists(&self, path: &Path) -> bool {
        self.fs().key(path).is_some()
    }

    pub fn is_dir(&self, path: &Path) -> bool {
        let fs = self.fs();
        matches!(fs.key(path).and_then(|k| fs.nodes.get(&k)), Some(Node::Dir { .. }))
    }

    pub fn link_target(&self, path: &Path) -> Option<PathBuf> {
        match self.fs().nodes.get(path) {
            Some(Node::Link { target }) => Some(target.clone()),
            _ => None,
        }
    }

    /// Lo que hay debajo de `dir` (rutas completas, en orden).
    pub fn list_all(&self, dir: &Path) -> Vec<PathBuf> {
        self.fs().nodes.keys().filter(|k| k.starts_with(dir) && k.as_path() != dir).cloned().collect()
    }

    /// Veces que se leyó el contenido de `path`.
    pub fn reads(&self, path: &Path) -> u32 {
        self.fs().reads.get(path).copied().unwrap_or(0)
    }

    /// Lo que está en la Papelera: (nombre ahí, de dónde vino).
    pub fn trashed(&self) -> Vec<(String, PathBuf)> {
        self.fs().trash.iter().map(|(name, from, _)| (name.clone(), from.clone())).collect()
    }
}

impl CopyPrims for FakeCopyPrims {
    fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
        let fs = self.fs();
        let key = fs.key(p).ok_or_else(|| CopyError::new(ErrKind::NotFound, p))?;
        Ok(match &fs.nodes[&key] {
            Node::Dir { modified } => ItemStat { kind: ItemKind::Dir, size: 0, modified: *modified, cloud_only: false },
            Node::File { data, modified, cloud, size } => {
                ItemStat { kind: ItemKind::File, size: size.unwrap_or(data.len() as u64), modified: *modified, cloud_only: *cloud }
            }
            Node::Link { target } => {
                ItemStat { kind: ItemKind::Symlink, size: target.as_os_str().len() as u64, modified: 0, cloud_only: false }
            }
        })
    }

    fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
        let mut fs = self.fs();
        fs.check_fail(p)?;
        let key = fs.key(p).ok_or_else(|| CopyError::new(ErrKind::NotFound, p))?;
        if !matches!(fs.nodes[&key], Node::Dir { .. }) {
            return Err(CopyError::new(ErrKind::Other("No es una carpeta".into()), p));
        }
        Ok(fs.nodes.keys().filter(|k| k.parent() == Some(key.as_path())).filter_map(|k| k.file_name().map(|n| n.to_os_string())).collect())
    }

    fn volume(&self, p: &Path) -> Option<VolumeInfo> {
        Some(self.fs().volume(p))
    }

    fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        let (data, block, delay, fail_after, short) = {
            let mut fs = self.fs();
            fs.check_fail(src)?;
            fs.check_fail(dst)?;
            let (data, modified) = match fs.key(src).and_then(|k| fs.nodes.get(&k)) {
                Some(Node::File { data, modified, .. }) => (data.clone(), *modified),
                Some(_) => return Err(CopyError::new(ErrKind::Other("No es un archivo".into()), src)),
                None => return Err(CopyError::new(ErrKind::NotFound, src)),
            };
            if fs.key(dst).is_some() {
                return Err(CopyError::new(ErrKind::Exists, dst));
            }
            fs.need_parent_dir(dst)?;
            *fs.reads.entry(src.to_path_buf()).or_insert(0) += 1;
            fs.nodes.insert(dst.to_path_buf(), Node::File { data: Vec::new(), modified, cloud: false, size: None });
            (data, fs.block.unwrap_or(64 * 1024), fs.slow.get(src).copied().or(fs.delay), fs.fail_after.get(src).cloned(), fs.short.get(src).copied())
        };
        let total = short.unwrap_or(data.len() as u64).min(data.len() as u64);
        let mut done = 0u64;
        let cleanup = |this: &Self| {
            this.fs().nodes.remove(dst);
        };
        loop {
            if let Some((limit, kind)) = &fail_after
                && done >= *limit
            {
                cleanup(self);
                return Err(CopyError::new(kind.clone(), dst));
            }
            if done >= total {
                break;
            }
            let mut end = (done + block).min(total);
            if let Some((limit, _)) = &fail_after
                && done < *limit
            {
                end = end.min(*limit);
            }
            if let Some(d) = delay {
                std::thread::sleep(d);
            }
            {
                let mut fs = self.fs();
                match fs.nodes.get_mut(dst) {
                    Some(Node::File { data: out, .. }) => out.extend_from_slice(&data[done as usize..end as usize]),
                    _ => return Err(CopyError::new(ErrKind::NotFound, dst)),
                }
            }
            done = end;
            if progress(done) == Flow::Stop {
                cleanup(self);
                return Err(CopyError::new(ErrKind::Cancelled, dst));
            }
        }
        Ok(())
    }

    fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        fs.check_fail(src)?;
        fs.check_fail(dst)?;
        let Some(Node::Link { target }) = fs.key(src).and_then(|k| fs.nodes.get(&k)).cloned() else {
            return Err(CopyError::new(ErrKind::NotFound, src));
        };
        if fs.key(dst).is_some() {
            return Err(CopyError::new(ErrKind::Exists, dst));
        }
        fs.need_parent_dir(dst)?;
        fs.nodes.insert(dst.to_path_buf(), Node::Link { target });
        Ok(())
    }

    fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        fs.check_fail(dst)?;
        if fs.key(dst).is_some() {
            return Err(CopyError::new(ErrKind::Exists, dst));
        }
        fs.need_parent_dir(dst)?;
        fs.nodes.insert(dst.to_path_buf(), Node::Dir { modified: 0 });
        Ok(())
    }

    fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        let modified = match fs.key(like).and_then(|k| fs.nodes.get(&k)) {
            Some(Node::Dir { modified }) => *modified,
            _ => return Err(CopyError::new(ErrKind::NotFound, like)),
        };
        match fs.nodes.get_mut(dst) {
            Some(Node::Dir { modified: m }) => {
                *m = modified;
                Ok(())
            }
            _ => Err(CopyError::new(ErrKind::NotFound, dst)),
        }
    }

    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        fs.check_fail(from)?;
        fs.check_fail(to)?;
        let from_key = fs.key(from).ok_or_else(|| CopyError::new(ErrKind::NotFound, from))?;
        if let Some(existing) = fs.key(to) {
            // El mismo elemento con otras mayúsculas: se puede.
            if existing != from_key {
                return Err(CopyError::new(ErrKind::Exists, to));
            }
        }
        fs.need_parent_dir(to)?;
        fs.move_tree(&from_key, to);
        Ok(())
    }

    fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        fs.check_fail(from)?;
        fs.check_fail(to)?;
        let from_key = fs.key(from).ok_or_else(|| CopyError::new(ErrKind::NotFound, from))?;
        let target = match fs.key(to) {
            Some(k) if matches!(fs.nodes[&k], Node::Dir { .. }) => {
                return Err(CopyError::new(ErrKind::Other("No se puede reemplazar una carpeta".into()), to));
            }
            Some(k) => {
                fs.nodes.remove(&k);
                k
            }
            None => to.to_path_buf(),
        };
        fs.need_parent_dir(&target)?;
        fs.move_tree(&from_key, &target);
        Ok(())
    }

    fn remove(&self, p: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        fs.check_fail(p)?;
        let key = fs.key(p).ok_or_else(|| CopyError::new(ErrKind::NotFound, p))?;
        if fs.subtree(&key).len() > 1 {
            return Err(CopyError::new(ErrKind::Other("La carpeta no está vacía".into()), p));
        }
        fs.nodes.remove(&key);
        Ok(())
    }

    fn trash(&self, p: &Path) -> Result<String, CopyError> {
        let mut fs = self.fs();
        fs.check_fail(p)?;
        let key = fs.key(p).ok_or_else(|| CopyError::new(ErrKind::NotFound, p))?;
        let mut tree = Vec::new();
        for k in fs.subtree(&key) {
            let node = fs.nodes.remove(&k).unwrap();
            tree.push((k.strip_prefix(&key).unwrap().to_path_buf(), node));
        }
        fs.trash_count += 1;
        let base = key.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = format!("{base} {}", fs.trash_count);
        fs.trash.push((name.clone(), key, tree));
        Ok(name)
    }

    fn trash_origin(&self, in_trash: &Path) -> Option<PathBuf> {
        self.fs().origins.get(in_trash).cloned()
    }

    fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError> {
        let mut fs = self.fs();
        fs.check_fail(original)?;
        let Some(i) = fs.trash.iter().position(|(name, ..)| name == in_trash) else {
            return Err(CopyError::new(ErrKind::NotFound, original));
        };
        if fs.key(original).is_some() {
            return Err(CopyError::new(ErrKind::Exists, original));
        }
        fs.need_parent_dir(original)?;
        let (_, _, tree) = fs.trash.remove(i);
        for (rel, node) in tree {
            let p = if rel.as_os_str().is_empty() { original.to_path_buf() } else { original.join(rel) };
            fs.nodes.insert(p, node);
        }
        Ok(())
    }
}

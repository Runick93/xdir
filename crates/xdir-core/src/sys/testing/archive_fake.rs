//! `FakeArchives`: archivos comprimidos falsos sobre el disco en memoria de `FakeCopyPrims`.
//! Un «zip» es una lista de entradas guardada por ruta (el archivo existe en el disco falso con
//! un contenido cualquiera); extraer las escribe con las mismas reglas que la crate de verdad
//! (rutas seguras, sin pasar por enlaces, enlaces al final y solo si apuntan adentro).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::archive::{ArchiveEntry, ArchiveFormat, EntryKind, format_of, is_root_entry, safe_entry_path, symlink_inside};
use crate::sys::{Archives, CopyError, CopyPrims, ErrKind, Flow, ItemKind, NOT_ALLOWED, PASSWORD, UNSUPPORTED};

use super::FakeCopyPrims;

#[derive(Clone, Debug)]
struct FakeEntry {
    name: String,
    kind: EntryKind,
    data: Vec<u8>,
    link: Option<String>,
    /// Tamaño declarado si no es el del contenido.
    size: Option<u64>,
}

#[derive(Default)]
struct State {
    /// Por identidad del archivo en el disco falso (renombrarlo la conserva).
    archives: HashMap<(u64, u64), Vec<FakeEntry>>,
    password: Vec<PathBuf>,
    /// Bytes por aviso de progreso al extraer y comprimir.
    block: Option<usize>,
}

/// Comprimir y extraer en memoria. `formats` simula el sistema (Windows: solo zip) y `links`, si
/// el sistema crea enlaces simbólicos (en Windows se omiten).
#[derive(Clone)]
pub struct FakeArchives {
    disk: FakeCopyPrims,
    formats: &'static [ArchiveFormat],
    links: bool,
    state: Arc<Mutex<State>>,
}

impl FakeArchives {
    pub fn new(disk: &FakeCopyPrims) -> FakeArchives {
        FakeArchives { disk: disk.clone(), formats: &[ArchiveFormat::Zip, ArchiveFormat::TarGz], links: true, state: Arc::default() }
    }

    /// Los del proceso (el de `sys::archives()` en las pruebas), sobre `FakeCopyPrims::shared()`.
    pub fn shared() -> FakeArchives {
        static SHARED: std::sync::OnceLock<FakeArchives> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| FakeArchives::new(&FakeCopyPrims::shared())).clone()
    }

    /// Como en Windows: solo zip y sin enlaces simbólicos.
    pub fn windows_style(disk: &FakeCopyPrims) -> FakeArchives {
        FakeArchives { formats: &[ArchiveFormat::Zip], links: false, ..FakeArchives::new(disk) }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Un archivo con esas entradas: (nombre, contenido); los nombres que terminan en `/` son
    /// carpetas. El archivo queda en el disco falso con un contenido de `packed` bytes.
    pub fn add_archive(&self, path: &Path, entries: &[(&str, &[u8])], packed: usize) {
        let list = entries
            .iter()
            .map(|(n, d)| FakeEntry {
                name: n.to_string(),
                kind: if n.ends_with('/') { EntryKind::Dir } else { EntryKind::File },
                data: d.to_vec(),
                link: None,
                size: None,
            })
            .collect();
        self.disk.add_file(path, vec![0; packed]);
        let id = self.id(path).expect("recién creado");
        self.st().archives.insert(id, list);
    }

    /// Suma un enlace simbólico (o, con `target` = None, un elemento que no se puede crear).
    pub fn add_link(&self, archive: &Path, name: &str, target: Option<&str>) {
        let kind = if target.is_some() { EntryKind::Symlink } else { EntryKind::Omitted };
        let e = FakeEntry { name: name.into(), kind, data: Vec::new(), link: target.map(String::from), size: None };
        let id = self.id(archive).expect("primero add_archive");
        self.st().archives.entry(id).or_default().push(e);
    }

    /// El tamaño que declara la entrada `name` (sin guardar tanto contenido).
    pub fn set_declared(&self, archive: &Path, name: &str, size: u64) {
        let id = self.id(archive).expect("primero add_archive");
        if let Some(e) = self.st().archives.get_mut(&id).and_then(|v| v.iter_mut().find(|e| e.name == name)) {
            e.size = Some(size);
        }
    }

    pub fn set_password(&self, archive: &Path) {
        self.st().password.push(archive.to_path_buf());
    }

    pub fn set_block(&self, n: usize) {
        self.st().block = Some(n);
    }

    /// Lo que guardó `create` en `archive`: (nombre adentro, contenido).
    pub fn created(&self, archive: &Path) -> Option<Vec<(String, Vec<u8>)>> {
        let id = self.id(archive)?;
        self.st().archives.get(&id).map(|v| v.iter().map(|e| (e.name.clone(), e.data.clone())).collect())
    }

    fn id(&self, p: &Path) -> Option<(u64, u64)> {
        self.disk.stat(p).ok().and_then(|s| s.file_id)
    }

    fn entries(&self, archive: &Path) -> Result<Vec<FakeEntry>, CopyError> {
        let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !format_of(&name).is_some_and(|f| self.formats.contains(&f)) {
            return Err(CopyError::new(ErrKind::Other(UNSUPPORTED.into()), archive));
        }
        let id = self.disk.stat(archive)?.file_id.unwrap_or_default();
        self.st().archives.get(&id).cloned().ok_or_else(|| CopyError::new(ErrKind::Other("El zip está dañado".into()), archive))
    }

    /// Adónde lleva el enlace `link` siguiendo los enlaces del disco falso (`None`: un ciclo).
    fn resolve(&self, link: &Path, depth: u32) -> Option<PathBuf> {
        let target = self.disk.link_target(link)?;
        if depth > 20 {
            return None;
        }
        if target.is_absolute() {
            return Some(target);
        }
        let mut cur = link.parent()?.to_path_buf();
        for c in target.components() {
            match c {
                std::path::Component::ParentDir => {
                    cur.pop();
                }
                std::path::Component::Normal(n) => {
                    cur.push(n);
                    if self.disk.link_target(&cur).is_some() {
                        cur = self.resolve(&cur, depth + 1)?;
                    }
                }
                _ => {}
            }
        }
        Some(cur)
    }

    /// Baja de `root` a la carpeta de `rel` creando las que faltan, sin pasar por enlaces.
    fn walk_down(&self, root: &Path, rel: &Path) -> Result<PathBuf, CopyError> {
        let mut cur = root.to_path_buf();
        let comps: Vec<_> = rel.components().collect();
        let (last, parents) = comps.split_last().ok_or_else(|| CopyError::new(ErrKind::Other(NOT_ALLOWED.into()), root))?;
        for c in parents {
            cur.push(c);
            match self.disk.stat(&cur) {
                Ok(s) if s.kind == ItemKind::Dir => {}
                Ok(s) if s.kind == ItemKind::Symlink => return Err(CopyError::new(ErrKind::Other(NOT_ALLOWED.into()), &cur)),
                Ok(_) => return Err(CopyError::new(ErrKind::Exists, &cur)),
                Err(_) => self.disk.create_dir(&cur)?,
            }
        }
        cur.push(last);
        Ok(cur)
    }
}

impl Archives for FakeArchives {
    fn formats(&self) -> &'static [ArchiveFormat] {
        self.formats
    }

    fn list(&self, archive: &Path, _cancel: &mut dyn FnMut() -> Flow) -> Result<Vec<ArchiveEntry>, CopyError> {
        let encrypted = self.st().password.iter().any(|p| p == archive);
        Ok(self
            .entries(archive)?
            .into_iter()
            .enumerate()
            .map(|(index, e)| {
                let size = e.size.unwrap_or(e.data.len() as u64);
                ArchiveEntry { index, name: e.name, size, kind: e.kind, link: e.link, modified: 0, packed: size / 2, encrypted }
            })
            .collect())
    }

    fn extract_all(
        &self,
        archive: &Path,
        root: &Path,
        progress: &mut dyn FnMut(u64) -> Flow,
        failed: &mut dyn FnMut(&str, CopyError),
    ) -> Result<usize, CopyError> {
        self.extract_some(archive, root, &|_| true, progress, failed)
    }

    fn extract_some(
        &self,
        archive: &Path,
        root: &Path,
        want: &dyn Fn(&str) -> bool,
        progress: &mut dyn FnMut(u64) -> Flow,
        failed: &mut dyn FnMut(&str, CopyError),
    ) -> Result<usize, CopyError> {
        let mut entries = self.entries(archive)?;
        entries.retain(|e| want(&e.name));
        if self.st().password.iter().any(|p| p == archive) {
            return Err(CopyError::new(ErrKind::Other(PASSWORD.into()), archive));
        }
        let block = self.st().block.unwrap_or(usize::MAX);
        let (links, rest): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| e.kind == EntryKind::Symlink);
        let mut done = 0u64;
        let mut omitted = 0;
        for e in rest.into_iter().chain(links) {
            if e.kind == EntryKind::Omitted || (e.kind == EntryKind::Symlink && !self.links) {
                omitted += 1;
                continue;
            }
            if e.kind == EntryKind::Dir && is_root_entry(&e.name) {
                continue;
            }
            let Some(rel) = safe_entry_path(&e.name) else {
                failed(&e.name, CopyError::new(ErrKind::Other(NOT_ALLOWED.into()), Path::new(&e.name)));
                continue;
            };
            let dst = match self.walk_down(root, &rel) {
                Ok(d) => d,
                Err(err) => {
                    failed(&e.name, err);
                    continue;
                }
            };
            match e.kind {
                EntryKind::Dir => {
                    if self.disk.stat(&dst).is_err()
                        && let Err(err) = self.disk.create_dir(&dst)
                    {
                        failed(&e.name, err);
                    }
                }
                EntryKind::Symlink => {
                    let target = e.link.clone().unwrap_or_default();
                    if !symlink_inside(&rel, &target) {
                        failed(&e.name, CopyError::new(ErrKind::Other(NOT_ALLOWED.into()), &dst));
                    } else if self.disk.stat(&dst).is_ok() {
                        failed(&e.name, CopyError::new(ErrKind::Exists, &dst));
                    } else {
                        self.disk.add_symlink(&dst, Path::new(&target));
                    }
                }
                EntryKind::File => {
                    if self.disk.stat(&dst).is_ok() {
                        failed(&e.name, CopyError::new(ErrKind::Exists, &dst));
                        continue;
                    }
                    let mut written = 0usize;
                    for chunk in e.data.chunks(block.min(e.data.len().max(1))) {
                        written += chunk.len();
                        if progress(done + written as u64) == Flow::Stop {
                            return Err(CopyError::new(ErrKind::Cancelled, &dst));
                        }
                    }
                    self.disk.add_file(&dst, e.data.clone());
                    done += e.data.len() as u64;
                }
                EntryKind::Omitted => {}
            }
            if progress(done) == Flow::Stop {
                return Err(CopyError::new(ErrKind::Cancelled, archive));
            }
        }
        Ok(omitted)
    }

    fn links_outside(&self, root: &Path, links: &[PathBuf]) -> Vec<PathBuf> {
        links.iter().filter(|l| !self.resolve(l, 0).is_some_and(|r| r.starts_with(root))).cloned().collect()
    }

    fn create(&self, out: &Path, items: &[(PathBuf, String)], progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        if self.disk.stat(out).is_ok() {
            return Err(CopyError::new(ErrKind::Exists, out));
        }
        self.disk.add_file(out, Vec::new());
        let block = self.st().block.unwrap_or(usize::MAX);
        let mut list = Vec::new();
        let mut done = 0u64;
        for (src, name) in items {
            let data = if name.ends_with('/') {
                Vec::new()
            } else {
                // Lee el contenido (un archivo solo en la nube se baja).
                match self.disk.read_content(src) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = self.disk.remove(out);
                        return Err(e);
                    }
                }
            };
            for chunk in data.chunks(block.min(data.len().max(1))) {
                done += chunk.len() as u64;
                if progress(done) == Flow::Stop {
                    let _ = self.disk.remove(out);
                    return Err(CopyError::new(ErrKind::Cancelled, out));
                }
            }
            let kind = if name.ends_with('/') { EntryKind::Dir } else { EntryKind::File };
            list.push(FakeEntry { name: name.clone(), kind, data, link: None, size: None });
            if progress(done) == Flow::Stop {
                let _ = self.disk.remove(out);
                return Err(CopyError::new(ErrKind::Cancelled, out));
            }
        }
        let id = self.id(out).expect("recién creado");
        self.st().archives.insert(id, list);
        Ok(())
    }
}

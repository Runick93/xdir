//! Primitivas de copia de cada sistema (`CopyPrims`): lo mínimo que necesita el motor de copia
//! (`transfer`) para copiar, mover, reemplazar y mandar a la Papelera archivo por archivo. Todo lo
//! que decide (conflictos, nombres, verificar, deshacer) es del motor; cada sistema solo pone lo
//! que lo hace fiel (clonar, conservar metadatos, renombrar sin pisar, Papelera).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Qué es un elemento, sin seguir enlaces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    File,
    Dir,
    Symlink,
}

/// Datos de un elemento leídos sin seguir enlaces ni leer (ni bajar de la nube) su contenido.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemStat {
    pub kind: ItemKind,
    /// Bytes (0 en carpetas).
    pub size: u64,
    /// Fecha de modificación como FILETIME (centenas de ns desde 1601; la de `format_date`).
    pub modified: u64,
    /// Está solo en la nube: copiarlo lo baja; recorrerlo o moverlo en el mismo disco, no.
    pub cloud_only: bool,
    /// Identidad del elemento (disco, número de archivo): dos rutas con la misma son el mismo
    /// elemento aunque se escriban distinto (`Z:\` y `\\servidor\recurso`, un enlace a la carpeta,
    /// un disco montado dos veces). `None` si el sistema no la da.
    pub file_id: Option<(u64, u64)>,
}

impl ItemStat {
    /// ¿Son el mismo elemento? (solo si los dos tienen identidad).
    pub fn same_item(&self, other: &ItemStat) -> bool {
        self.file_id.is_some() && self.file_id == other.file_id
    }
}

/// El disco de una ruta.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VolumeInfo {
    /// Identifica el disco: dos rutas con el mismo `id` están en el mismo.
    pub id: OsString,
    /// Nombre para mostrar («Backup (E:)», «USB»).
    pub label: String,
    /// No distingue mayúsculas en los nombres (NTFS, APFS por defecto, FAT).
    pub case_insensitive: bool,
    /// Tiene Papelera (lo reemplazado va ahí; si no, se borra).
    pub has_trash: bool,
    /// Tamaño máximo de un archivo (FAT32: 4 GiB - 1).
    pub max_file: Option<u64>,
    /// Bytes libres.
    pub free: u64,
}

/// Lo que devuelve el aviso de progreso: seguir o cortar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    Continue,
    Stop,
}

/// Por qué falló una primitiva.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ErrKind {
    InUse,
    Denied,
    NotFound,
    DiskFull,
    TooBig,
    Exists,
    Cancelled,
    /// Renombrar no puede llevarlo a otro disco (hay que copiar).
    CrossDevice,
    Other(String),
}

/// Un error de una primitiva, con la ruta que lo causó.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CopyError {
    pub kind: ErrKind,
    pub path: PathBuf,
}

impl CopyError {
    pub fn new(kind: ErrKind, path: &Path) -> CopyError {
        CopyError { kind, path: path.to_path_buf() }
    }

    /// El motivo en castellano, para la lista de errores.
    pub fn message(&self) -> String {
        match &self.kind {
            ErrKind::InUse => "Lo está usando otro programa".into(),
            ErrKind::Denied => "No tenés permiso".into(),
            ErrKind::NotFound => "Ya no existe".into(),
            ErrKind::DiskFull => "El disco está lleno".into(),
            ErrKind::TooBig => "Es demasiado grande para el formato del disco de destino (FAT32)".into(),
            ErrKind::Exists => "Ya existe".into(),
            ErrKind::Cancelled => "Cancelado".into(),
            ErrKind::CrossDevice => "Está en otro disco".into(),
            ErrKind::Other(text) => text.clone(),
        }
    }
}

/// Las primitivas de copia de un sistema. Corren en el hilo del trabajo (nunca en el de
/// interfaz). Ninguna pisa nada salvo `replace`, que es atómico.
pub trait CopyPrims: Send + Sync {
    /// Datos de `p` sin seguir enlaces ni leer contenido.
    fn stat(&self, p: &Path) -> Result<ItemStat, CopyError>;
    /// Nombres de lo que hay en la carpeta `p` (sin «.» ni «..»).
    fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError>;
    /// El disco de `p` (`None` si no se sabe).
    fn volume(&self, p: &Path) -> Option<VolumeInfo>;
    /// Crea `dst` (falla con Exists si ya está) con el contenido y los metadatos de `src`;
    /// `progress(bytes_copiados)` en cada bloque; si devuelve `Flow::Stop`, corta, borra `dst` y
    /// devuelve `ErrKind::Cancelled`. Si falla por otro motivo después de crear `dst`, también lo
    /// borra: nunca queda un archivo a medio copiar.
    fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError>;
    /// Copia un enlace simbólico como enlace (no lo que apunta).
    fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError>;
    /// Crea la carpeta `dst` (falla con Exists si ya está).
    fn create_dir(&self, dst: &Path) -> Result<(), CopyError>;
    /// Pone a `dst` los atributos y las fechas de `like` (al terminar su contenido).
    fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError>;
    /// Renombra o mueve dentro del mismo disco; falla con Exists si `to` ya está (salvo que sea
    /// el mismo elemento con otras mayúsculas).
    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError>;
    /// Pone `from` en lugar de `to` en un solo paso (atómico). El motor no lo usa para
    /// «Reemplazar» (aparta lo que está con `rename_noreplace` y recién después lo manda a la
    /// Papelera): queda para quien necesite pisar a propósito.
    fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError>;
    /// Deja en el disco lo escrito en `p` (antes de borrar el original al mover).
    fn sync(&self, _p: &Path) -> Result<(), CopyError> {
        Ok(())
    }
    /// Borra un archivo, un enlace o una carpeta vacía.
    fn remove(&self, p: &Path) -> Result<(), CopyError>;
    /// Manda a la Papelera; devuelve cómo encontrarlo ahí (como `Undo::Recycled`; el texto es de
    /// la plataforma). Si el sistema lo borró en vez de mandarlo a la Papelera, es un error.
    fn trash(&self, p: &Path) -> Result<String, CopyError>;
    /// Manda `p` a la Papelera como si fuera `shown` (lo que reemplazó: nombre y lugar para
    /// «Devolver» o «Poner de nuevo»), donde el sistema lo permite. Por defecto, `trash(p)`.
    fn trash_as(&self, p: &Path, _shown: &Path) -> Result<String, CopyError> {
        self.trash(p)
    }
    /// Lugar original de un elemento de la Papelera, si se sabe.
    fn trash_origin(&self, _in_trash: &Path) -> Option<PathBuf> {
        None
    }
    /// «Devolver»: deja lista la carpeta `dir` del lugar original de `in_trash` antes de moverlo
    /// ahí, sin seguir enlaces (un disco armado podría tener «docs» apuntando a otro lado): las
    /// que faltan se crean una por una y ninguna del camino puede ser un enlace. Cada sistema
    /// pone sus reglas (en macOS y Linux, solo los enlaces de root del primer nivel del disco de
    /// arranque, como `/tmp`). Por defecto: ningún enlace en el camino.
    fn prepare_restore(&self, _in_trash: &Path, dir: &Path) -> Result<(), CopyError> {
        let refuse = |p: &Path| CopyError::new(ErrKind::Other(format!("«{}» no es una carpeta de ese disco (es un enlace o está afuera): no se devuelve ahí", p.display())), p);
        // Las que faltan (desde abajo) hasta la primera que está; desde ahí, todas carpetas.
        let mut missing = Vec::new();
        let mut found = false;
        for d in dir.ancestors().filter(|d| !d.as_os_str().is_empty()) {
            match self.stat(d) {
                Ok(s) if s.kind == ItemKind::Dir => found = true,
                Ok(_) => return Err(refuse(d)),
                Err(e) if e.kind == ErrKind::NotFound && !found => missing.push(d.to_path_buf()),
                Err(e) => return Err(e),
            }
        }
        for d in missing.iter().rev() {
            self.create_dir(d)?;
        }
        Ok(())
    }
    /// «Devolver» terminó con `in_trash`: el sistema olvida su registro (el `.trashinfo` en Linux,
    /// la línea de `papelera.tsv` en macOS).
    fn restored(&self, _in_trash: &Path) {}
    /// Deshace `trash`: devuelve `in_trash` a `original` (que puede no ser donde estaba al
    /// mandarlo: el motor manda a la Papelera lo apartado con un nombre temporal).
    fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError>;
}

/// Primitivas de un sistema que todavía no las tiene: todo responde «sin implementar».
pub struct UnimplementedCopyPrims;

fn unimplemented<T>(p: &Path) -> Result<T, CopyError> {
    Err(CopyError::new(ErrKind::Other("sin implementar".into()), p))
}

impl CopyPrims for UnimplementedCopyPrims {
    fn stat(&self, p: &Path) -> Result<ItemStat, CopyError> {
        unimplemented(p)
    }
    fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError> {
        unimplemented(p)
    }
    fn volume(&self, _p: &Path) -> Option<VolumeInfo> {
        None
    }
    fn copy_file(&self, src: &Path, _dst: &Path, _progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError> {
        unimplemented(src)
    }
    fn copy_symlink(&self, src: &Path, _dst: &Path) -> Result<(), CopyError> {
        unimplemented(src)
    }
    fn create_dir(&self, dst: &Path) -> Result<(), CopyError> {
        unimplemented(dst)
    }
    fn finish_dir(&self, dst: &Path, _like: &Path) -> Result<(), CopyError> {
        unimplemented(dst)
    }
    fn rename_noreplace(&self, from: &Path, _to: &Path) -> Result<(), CopyError> {
        unimplemented(from)
    }
    fn replace(&self, from: &Path, _to: &Path) -> Result<(), CopyError> {
        unimplemented(from)
    }
    fn remove(&self, p: &Path) -> Result<(), CopyError> {
        unimplemented(p)
    }
    fn trash(&self, p: &Path) -> Result<String, CopyError> {
        unimplemented(p)
    }
    fn untrash(&self, _in_trash: &str, original: &Path) -> Result<(), CopyError> {
        unimplemented(original)
    }
}

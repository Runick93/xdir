//! Primitivas de copia de cada sistema (`CopyPrims`): lo mínimo que necesita el motor de copia
//! (`transfer`) para copiar, mover, reemplazar y mandar a la Papelera archivo por archivo. Todo lo
//! que decide (conflictos, nombres, verificar, deshacer) es del motor; cada sistema solo pone lo
//! que lo hace fiel (clonar, conservar metadatos, renombrar sin pisar, Papelera).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Los tramos de una ruta en minúsculas, para comparar rutas en discos que no distinguen
/// mayúsculas. Por tramos (no el texto): en Windows `C:/a/b` y `C:\a\b` (o lo armado con `join`,
/// que pone `\`) son la misma ruta.
pub fn folded(p: &Path) -> Vec<String> {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect()
}

/// ¿Es la misma ruta sin distinguir mayúsculas? (por tramos).
pub fn same_path_folded(a: &Path, b: &Path) -> bool {
    folded(a) == folded(b)
}

/// ¿`inner` está dentro de `outer` (o es la misma) sin distinguir mayúsculas? (por tramos).
pub fn starts_with_folded(inner: &Path, outer: &Path) -> bool {
    let (i, o) = (folded(inner), folded(outer));
    i.len() >= o.len() && i[..o.len()] == o[..]
}

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
    /// Un tramo de la ruta (el nombre) es más largo de lo que acepta el disco.
    NameTooLong,
    /// La ruta entera es más larga de lo que acepta el sistema (cada nombre entra).
    PathTooLong,
    /// El nombre tiene caracteres que el disco no acepta o es reservado (CON, NUL…).
    InvalidName,
    /// El disco es de solo lectura (protegido contra escritura, un CD, montado así).
    ReadOnly,
    /// El disco o la red no responde (desconectado, se cortó la red, no está listo).
    Offline,
    /// macOS: el elemento está bloqueado (Obtener información ▸ Bloqueado, `uchg`).
    Locked,
    /// Windows: el acceso controlado a carpetas (protección contra ransomware) no deja a xdir
    /// escribir ahí. `sure`: se pudo leer que xdir no está permitido (si no, se dice con «puede
    /// que»).
    Blocked { sure: bool },
    /// macOS: la privacidad del sistema (TCC) no le dio permiso a xdir para esa carpeta. `sure`:
    /// al leer una carpeta (al escribir, `EPERM` también puede ser otra cosa: «puede que»).
    Privacy { sure: bool },
    Other(String),
}

/// Un programa que tiene abierto un archivo (lo que `CopyPrims::holders` encuentra cuando algo
/// falla con `ErrKind::InUse`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Holder {
    /// El nombre que se le muestra al usuario (el de la aplicación o el del proceso).
    pub name: String,
    /// El ejecutable, si se sabe.
    pub exe: Option<String>,
    pub pid: u32,
    /// El usuario dueño del proceso, si se sabe.
    pub user: Option<String>,
}

/// Un error de una primitiva, con la ruta que lo causó.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CopyError {
    pub kind: ErrKind,
    pub path: PathBuf,
    /// Quiénes tienen la ruta abierta (solo lo llena el motor, para `ErrKind::InUse`; las
    /// primitivas lo dejan vacío).
    pub holders: Vec<Holder>,
}

/// El nombre que se muestra de una ruta: el último tramo (o la ruta entera si no tiene, como
/// `C:\` o `/`).
pub fn shown_name(p: &Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => p.display().to_string(),
    }
}

/// Un error que el sistema no explica mejor: una frase propia y, entre paréntesis, el texto del
/// sistema y su código («Error inesperado del sistema (El dispositivo no está listo; Windows:
/// 0x80070015)»), para poder diagnosticarlo.
pub fn unexpected(system: &str, text: &str, code: &str) -> ErrKind {
    let text = text.trim().trim_end_matches('.').trim();
    if text.is_empty() {
        ErrKind::Other(format!("Error inesperado del sistema ({system}: {code})"))
    } else {
        ErrKind::Other(format!("Error inesperado del sistema ({text}; {system}: {code})"))
    }
}

/// Un error de E/S del sistema que no tiene una explicación mejor (`unexpected` con el texto y
/// el código: `Windows: 0x80070015` en Windows, `macOS: errno 5` o `Linux: errno 5` en los demás).
pub fn io_unexpected(e: &std::io::Error) -> ErrKind {
    let text = e.to_string();
    // Sin el «(os error N)» que agrega Rust: el código va aparte.
    let text = text.split(" (os error").next().unwrap_or(&text);
    match e.raw_os_error() {
        Some(n) => unexpected(SYSTEM, text, &os_code(n)),
        None => unexpected(SYSTEM, text, "sin código"),
    }
}

/// Un error de E/S de Rust, sin la plataforma a mano (lo usan el núcleo y los comprimidos): por su
/// tipo y, en Windows, por los códigos que Rust no distingue. Lo que no se reconoce, con el texto y
/// el código (`io_unexpected`).
pub fn io_kind(e: &std::io::Error) -> ErrKind {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::PermissionDenied => ErrKind::Denied,
        K::NotFound => ErrKind::NotFound,
        K::AlreadyExists => ErrKind::Exists,
        K::StorageFull | K::QuotaExceeded => ErrKind::DiskFull,
        K::FileTooLarge => ErrKind::TooBig,
        K::ResourceBusy | K::ExecutableFileBusy => ErrKind::InUse,
        K::ReadOnlyFilesystem => ErrKind::ReadOnly,
        K::CrossesDevices => ErrKind::CrossDevice,
        K::NetworkDown | K::NetworkUnreachable | K::HostUnreachable | K::TimedOut | K::NotConnected | K::StaleNetworkFileHandle => ErrKind::Offline,
        // Rust junta ahí «nombre demasiado largo» (ENAMETOOLONG, ERROR_FILENAME_EXCED_RANGE) y
        // «nombre no válido».
        K::InvalidFilename if e.raw_os_error() == Some(NAME_TOO_LONG) => ErrKind::NameTooLong,
        K::InvalidFilename => ErrKind::InvalidName,
        _ => match e.raw_os_error() {
            // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_HANDLE_DISK_FULL,
            // ERROR_WRITE_PROTECT.
            Some(32 | 33) if cfg!(windows) => ErrKind::InUse,
            Some(39 | 112) if cfg!(windows) => ErrKind::DiskFull,
            Some(19) if cfg!(windows) => ErrKind::ReadOnly,
            _ => io_unexpected(e),
        },
    }
}

/// El código de «nombre demasiado largo» (ENAMETOOLONG, ERROR_FILENAME_EXCED_RANGE).
const NAME_TOO_LONG: i32 = if cfg!(windows) { 206 } else if cfg!(target_os = "linux") { 36 } else { 63 };

/// El nombre del sistema para los códigos de error.
const SYSTEM: &str = if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" };

/// El código de un error del sistema como se busca: el HRESULT en Windows, el `errno` en los demás.
fn os_code(n: i32) -> String {
    if cfg!(windows) { format!("0x{:08X}", 0x8007_0000u32 | (n as u32 & 0xFFFF)) } else { format!("errno {n}") }
}

/// «Demasiado largo»: el nombre (algún tramo pasa de 255 caracteres, el máximo de casi todos los
/// discos) o la ruta entera.
pub fn too_long(p: &Path) -> ErrKind {
    if p.components().any(|c| c.as_os_str().to_string_lossy().chars().count() > 255) {
        ErrKind::NameTooLong
    } else {
        ErrKind::PathTooLong
    }
}

/// La primera letra en minúscula (para seguir una frase después de «:»), salvo que la primera
/// palabra sea un nombre propio («Windows», «macOS») o una sigla.
pub fn lower_first(text: &str) -> String {
    let word = text.split([' ', ':', ',']).next().unwrap_or("");
    let proper = ["Windows", "macOS", "Linux", "Mac", "xdir"].contains(&word) || word.chars().skip(1).any(char::is_uppercase);
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if !proper => c.to_lowercase().chain(chars).collect(),
        _ => text.to_string(),
    }
}

impl CopyError {
    pub fn new(kind: ErrKind, path: &Path) -> CopyError {
        CopyError { kind, path: path.to_path_buf(), holders: Vec::new() }
    }

    /// El motivo corto (para la fila de la lista de errores, ~30 caracteres) y, si hay algo
    /// concreto, qué hacer (solo en el aviso).
    fn parts(&self) -> (String, Option<&'static str>) {
        let (why, todo): (&str, Option<&'static str>) = match &self.kind {
            ErrKind::InUse => ("Lo está usando otro programa", Some("cerralo y reintentá")),
            ErrKind::Denied => ("No tenés permiso", Some("pedí acceso a quien administra esa carpeta")),
            ErrKind::NotFound => ("Ya no existe", None),
            ErrKind::DiskFull => ("El disco está lleno", Some("liberá espacio y reintentá")),
            ErrKind::TooBig => ("Es demasiado grande para FAT32", Some("ese formato admite hasta 4 GB por archivo: usá otro disco")),
            ErrKind::Exists => ("Ya existe", None),
            ErrKind::Cancelled => ("Cancelado", None),
            ErrKind::CrossDevice => ("Está en otro disco", None),
            ErrKind::NameTooLong => ("El nombre es demasiado largo", Some("acortalo")),
            ErrKind::PathTooLong => ("La ruta es demasiado larga", Some("acortá el nombre de alguna carpeta o llevalo más arriba")),
            ErrKind::InvalidName => {
                ("Nombre no válido para ese disco", Some("sacale caracteres como : * ? \" < > | o no uses nombres reservados como CON o NUL"))
            }
            ErrKind::ReadOnly => ("El disco es de solo lectura", Some("quitale la protección contra escritura o usá otro disco")),
            ErrKind::Offline => ("El disco o la red no responde", Some("revisá la conexión y reintentá")),
            ErrKind::Locked => ("Está bloqueado", Some("desbloquealo en Obtener información ▸ Bloqueado")),
            ErrKind::Blocked { sure: true } => ("Bloqueado: acceso controlado", None),
            ErrKind::Blocked { sure: false } => ("¿Acceso controlado de Windows?", None),
            ErrKind::Privacy { sure: true } => ("Falta permiso de Privacidad", None),
            ErrKind::Privacy { sure: false } => ("¿Falta permiso de Privacidad?", None),
            ErrKind::Other(text) => return (text.clone(), None),
        };
        (why.to_string(), todo)
    }

    /// El motivo corto, para la fila de la lista de errores (el nombre va al lado): «Lo está
    /// usando otro programa». Qué hacer va en `notice`.
    pub fn message(&self) -> String {
        self.parts().0
    }

    /// El aviso de la barra de estado: qué no se pudo hacer (`what`, con el nombre: «crear la
    /// carpeta nueva en «Docs»»), por qué y qué hacer, corto. «No se pudo renombrar «a.txt»: lo
    /// está usando otro programa. Cerralo y reintentá.» Lo bloqueado por el sistema dice dónde y
    /// cómo permitirlo; la explicación entera está en `detail`.
    pub fn notice(&self, what: &str) -> String {
        let name = shown_name(&self.path);
        match &self.kind {
            ErrKind::Blocked { sure: true } => format!(
                "Windows bloqueó a xdir en «{name}» (acceso controlado a carpetas): permitilo en Seguridad de Windows ▸ Protección contra ransomware."
            ),
            ErrKind::Blocked { sure: false } => format!(
                "No se pudo {what}: puede que lo haya bloqueado el acceso controlado a carpetas de Windows. \
                 Si es así, permití xdir en Seguridad de Windows ▸ Protección contra ransomware."
            ),
            ErrKind::Privacy { sure: true } => {
                format!("macOS no deja que xdir use «{name}»: dale permiso en Configuración del Sistema ▸ Privacidad y seguridad.")
            }
            ErrKind::Privacy { sure: false } => format!(
                "No se pudo {what}: puede que macOS no deje a xdir usar esa carpeta. \
                 Si es así, dale permiso en Configuración del Sistema ▸ Privacidad y seguridad."
            ),
            _ => {
                let (why, todo) = self.parts();
                let why = lower_first(why.trim_end_matches('.'));
                match todo {
                    Some(todo) => {
                        let mut todo = todo.chars();
                        let first: String = todo.next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                        format!("No se pudo {what}: {why}. {first}{}.", todo.as_str())
                    }
                    None => format!("No se pudo {what}: {why}."),
                }
            }
        }
    }

    /// La explicación entera, para el cartel al pasar el mouse sobre el aviso (lo bloqueado por
    /// el sistema: dónde se permite xdir paso a paso). `None` si el aviso ya lo dice todo.
    pub fn detail(&self, what: &str) -> Option<String> {
        let name = shown_name(&self.path);
        const CFA_STEPS: &str = "Permití xdir en Seguridad de Windows ▸ Protección contra virus y amenazas ▸ Protección contra ransomware ▸ \
                                 Permitir una aplicación a través del acceso controlado a carpetas (en una PC de empresa, lo hace sistemas).";
        const TCC_STEPS: &str = "Configuración del Sistema ▸ Privacidad y seguridad ▸ Archivos y carpetas (o Acceso total al disco)";
        Some(match &self.kind {
            ErrKind::Blocked { sure: true } => format!(
                "Windows bloqueó a xdir: el acceso controlado a carpetas (protección contra ransomware) no lo deja escribir en «{name}». {CFA_STEPS}"
            ),
            ErrKind::Blocked { sure: false } => format!(
                "Puede que el acceso controlado a carpetas (protección contra ransomware) no deje a xdir escribir en «{name}»: \
                 no se pudo {what}. Si es así, {}{}",
                CFA_STEPS[..1].to_lowercase(),
                &CFA_STEPS[1..]
            ),
            ErrKind::Privacy { sure: true } => format!("macOS no deja que xdir use «{name}». Dale permiso en {TCC_STEPS} y reintentá."),
            ErrKind::Privacy { sure: false } => format!(
                "No se pudo {what}: puede que macOS no deje a xdir usar «{name}». Si es así, dale permiso en {TCC_STEPS} y reintentá."
            ),
            _ => return None,
        })
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
    /// Crea la carpeta `dst` solo para el usuario (0700 donde hay permisos): la carpeta oculta
    /// donde el motor extrae antes de llevar las cosas a su lugar. Por defecto, `create_dir`.
    fn create_private_dir(&self, dst: &Path) -> Result<(), CopyError> {
        self.create_dir(dst)
    }
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
    /// ¿Es un error de «ocupado por un momento» (un antivirus o el indexador abrieron lo recién
    /// escrito)? El motor reintenta un rato los renombres finales. Por defecto, solo «ocupado»;
    /// Windows también cuenta «sin permiso» (lo da con un archivo abierto para borrar).
    fn transient(&self, kind: &ErrKind) -> bool {
        *kind == ErrKind::InUse
    }
    /// Deja en el disco la carpeta `dir` (sus nombres: lo recién renombrado ahí adentro), antes de
    /// borrar el original al mover.
    fn sync_dir(&self, _dir: &Path) -> Result<(), CopyError> {
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
    /// Qué programas tienen abierto `p` (bloqueante). Por defecto, ninguno: no se sabe.
    fn holders(&self, _path: &Path) -> Vec<Holder> {
        Vec::new()
    }
    /// Lugar original de un elemento de la Papelera, si se sabe.
    fn trash_origin(&self, _in_trash: &Path) -> Option<PathBuf> {
        None
    }
    /// «Devolver»: deja lista la carpeta `dir` del lugar original de `in_trash` antes de moverlo
    /// ahí, sin seguir enlaces (un disco armado podría tener «docs» apuntando a otro lado): las
    /// que faltan se crean una por una y ninguna del camino puede ser un enlace. Cada sistema
    /// pone sus reglas (en macOS y Linux, solo los enlaces de root del primer nivel del disco de
    /// arranque, como `/tmp`). Por defecto: ningún enlace en el camino. Con `create` = false solo
    /// se vuelve a mirar (justo antes de escribir ahí): lo que falte es error.
    fn prepare_restore(&self, _in_trash: &Path, dir: &Path, create: bool) -> Result<(), CopyError> {
        let refuse = |p: &Path| CopyError::new(ErrKind::Other(format!("«{}» no es una carpeta de ese disco (es un enlace o está afuera): no se devuelve ahí", p.display())), p);
        // Las que faltan (desde abajo) hasta la primera que está; desde ahí, todas carpetas.
        let mut missing = Vec::new();
        let mut found = false;
        for d in dir.ancestors().filter(|d| !d.as_os_str().is_empty()) {
            match self.stat(d) {
                Ok(s) if s.kind == ItemKind::Dir => found = true,
                Ok(_) => return Err(refuse(d)),
                Err(e) if e.kind == ErrKind::NotFound && !found && create => missing.push(d.to_path_buf()),
                Err(e) => return Err(e),
            }
        }
        for d in missing.iter().rev() {
            self.create_dir(d)?;
        }
        Ok(())
    }
    /// Antes de mandar a la Papelera algo por su ruta (deshacer «lo creado»): ninguna carpeta del
    /// camino de `dir` puede ser un enlace a otro lado (salvo los del sistema, como `/tmp`).
    /// Por defecto, no se mira.
    fn check_parent(&self, _dir: &Path) -> Result<(), CopyError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_paths_compare_by_components() {
        assert!(same_path_folded(Path::new("/ci0/Docs/a.txt"), &Path::new("/ci0/docs").join("A.TXT")));
        assert!(!same_path_folded(Path::new("/ci0/Docs/a.txt"), Path::new("/ci0/Docs/b.txt")));
        assert!(starts_with_folded(&Path::new("/X/Y").join("z"), Path::new("/x/y")));
        assert!(!starts_with_folded(Path::new("/x/yz"), Path::new("/x/y")));
        #[cfg(windows)]
        assert!(same_path_folded(Path::new(r"C:\Docs\a.txt"), Path::new("C:/docs/A.txt")));
    }
}

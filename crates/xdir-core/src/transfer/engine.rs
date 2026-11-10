//! El motor: corre un `JobSpec` con las primitivas del sistema y manda `Update`s.
//!
//! Reglas que cuidan los datos:
//! - nada se pisa sin preguntar, tampoco el mismo elemento por otro camino (se compara la
//!   identidad de los archivos, no solo las rutas);
//! - «Reemplazar» nunca toca lo que está hasta que lo nuevo está listo: un archivo se copia a un
//!   temporal oculto en la carpeta destino; recién entonces lo que estaba se aparta (renombrado sin
//!   pisar), el temporal toma su nombre y lo apartado va a la Papelera (si el disco no tiene, se
//!   borra). Una carpeta se aparta antes de llenar la nueva y, si algo falla o se cancela, vuelve;
//! - mover entre discos es copiar, verificar el tamaño, ver que el original no cambió, dejar la
//!   copia guardada en el disco y recién ahí borrar el original, archivo por archivo; las carpetas
//!   de origen se borran solo si quedaron vacías;
//! - mover dentro del mismo disco es renombrar (no lee contenido);
//! - lo a medio copiar lo borra la primitiva; el temporal de un reemplazo lo borra el motor;
//! - cada error se anota y se sigue, salvo «El disco está lleno», que detiene el trabajo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::archive::{EntryKind, archive_name, declared_total, extract_folder_name};
use crate::names::{copy_name, folder_with_name, free_name, numbered_name, replaced_name};
use super::walk::{Planned, totals, walk};
use super::{Answer, Choice, ConflictInfo, Control, Flow, JobError, JobOutcome, JobSpec, Update};
use crate::format::thousands;
use crate::ops::{OpResult, Undo};
use crate::sys::{self, Archives, CopyError, CopyPrims, ErrKind, ItemKind, ItemStat, NoArchives, PASSWORD, UNSUPPORTED};

/// Como mucho un aviso de progreso cada tanto por trabajo.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

const INTO_ITSELF: &str = "No se puede copiar una carpeta dentro de sí misma";
const NOT_SAME: &str = "La copia no quedó igual";
const NO_ORIGIN: &str = "No se sabe de dónde vino";
/// Deshacer «lo creado»: en esa ruta ahora hay otra cosa (no se toca).
const NOT_THE_SAME: &str = "Ahí ya no está lo que se había creado: no se manda a la Papelera";
const REPLACE_PARENT: &str = "No se puede reemplazar una carpeta que contiene lo que se copia";
const CHANGED: &str = "El original cambió durante la copia";

/// Extraer más que esto (o más de 100 veces lo que ocupa el archivo) se pregunta antes.
const BIG_EXPANSION: u64 = 10 * 1024 * 1024 * 1024;
const BIG_RATIO: u64 = 100;

/// Título del trabajo para la ventana de copia; `items`: cuántos elementos de arriba.
pub fn title(spec: &JobSpec, items: u64) -> String {
    let name = |p: &Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    let folder = |p: &Path| p.parent().map(name).unwrap_or_default();
    let count = |verb: &str| if items == 1 { format!("{verb} 1 elemento") } else { format!("{verb} {} elementos", thousands(items)) };
    let between = |verb: &str, first: Option<&PathBuf>, dest: &Path| match first {
        Some(one) if items == 1 => format!("{verb} «{}» a «{}»", name(one), name(dest)),
        _ => format!("{} de «{}» a «{}»", count(verb), first.map(|f| folder(f)).unwrap_or_default(), name(dest)),
    };
    match spec {
        JobSpec::Copy { sources, dest } => between("Copiando", sources.first(), dest),
        JobSpec::Move { sources, dest } => between("Moviendo", sources.first(), dest),
        JobSpec::Retry { moving, pairs } => {
            let verb = if *moving { "Moviendo" } else { "Copiando" };
            let dest = pairs.first().and_then(|(_, d)| d.parent()).map(Path::to_path_buf).unwrap_or_default();
            between(verb, pairs.first().map(|(s, _)| s), &dest)
        }
        JobSpec::Duplicate { .. } => count("Duplicando"),
        JobSpec::NewFolderWith { .. } => count("Moviendo"),
        JobSpec::Restore { .. } => count("Devolviendo"),
        JobSpec::Undo(_) => "Deshaciendo".into(),
        JobSpec::Compress { .. } => count("Comprimiendo"),
        JobSpec::Extract { archive, .. } => format!("Extrayendo «{}»", name(archive)),
    }
}

/// Corre el trabajo hasta el final (o hasta que se cancele) y manda `Update::Finished`. Comprimir
/// y extraer usan `sys::archives()`.
pub fn run(spec: JobSpec, prims: &dyn CopyPrims, ctl: &Control, out: &mut dyn FnMut(Update)) {
    if matches!(spec, JobSpec::Compress { .. } | JobSpec::Extract { .. }) {
        run_with(spec, prims, sys::archives(), ctl, out);
    } else {
        run_with(spec, prims, &NoArchives, ctl, out);
    }
}

/// `run` con el servicio de archivos comprimidos dado.
pub fn run_with(spec: JobSpec, prims: &dyn CopyPrims, archives: &dyn Archives, ctl: &Control, out: &mut dyn FnMut(Update)) {
    let mut job = Job::new(prims, ctl, out);
    job.archives = archives;
    match spec {
        JobSpec::Copy { sources, dest } => {
            let t = title(&JobSpec::Copy { sources: sources.clone(), dest: dest.clone() }, sources.len() as u64);
            let items = job.plan_into(&sources, &dest, false);
            job.run_items(t, items, false);
        }
        JobSpec::Move { sources, dest } => {
            let t = title(&JobSpec::Move { sources: sources.clone(), dest: dest.clone() }, sources.len() as u64);
            let items = job.plan_into(&sources, &dest, true);
            job.run_items(t, items, true);
        }
        JobSpec::Duplicate { sources } => {
            let t = title(&JobSpec::Duplicate { sources: vec![] }, sources.len() as u64);
            let items = sources
                .iter()
                .filter_map(|s| s.parent().map(|parent| Item::new(s, Target::Duplicate(parent.to_path_buf()), How::Walk(None))))
                .collect();
            job.run_items(t, items, false);
        }
        JobSpec::Retry { moving, pairs } => {
            let t = title(&JobSpec::Retry { moving, pairs: pairs.clone() }, pairs.len() as u64);
            let items = job.plan_retry(&pairs, moving);
            job.run_items(t, items, moving);
        }
        JobSpec::NewFolderWith { parent, items } => job.new_folder_with(&parent, &items),
        JobSpec::Compress { sources } => job.compress(&sources),
        JobSpec::Extract { archive, into_folder } => job.extract(&archive, into_folder),
        JobSpec::Restore { items } => job.restore(&items),
        JobSpec::Undo(undo) => {
            job.start("Deshaciendo".into(), count_steps(&undo), 0);
            job.undo(undo);
            job.discard_result();
        }
    }
    job.finish();
}

fn count_steps(u: &Undo) -> u64 {
    match u {
        Undo::Batch(v) => v.iter().map(count_steps).sum(),
        Undo::Created(v) => v.len() as u64,
        Undo::CreatedIds(v) => v.len() as u64,
        Undo::Moved(v) => v.len() as u64,
        Undo::Recycled(v) => v.len() as u64,
        Undo::Renames(v) => v.len() as u64,
        Undo::Rename { .. } | Undo::CreatedFolder(_) => 1,
    }
}

/// Adónde va un elemento de arriba.
enum Target {
    /// A esa ruta (si existe, conflicto).
    Fixed(PathBuf),
    /// A esa carpeta con el primer nombre «copia» libre (Duplicar, pegar en la misma carpeta).
    Duplicate(PathBuf),
}

/// Un conflicto de arriba que se ve por delante.
#[derive(Clone, Copy)]
struct Ahead {
    is_dir: bool,
    /// Carpeta sobre carpeta: si se combina, puede haber más conflictos adentro.
    can_merge: bool,
}

/// Cómo se lleva.
enum How {
    /// Renombrar (mover en el mismo disco).
    Rename,
    /// Copiar elemento por elemento (con el recorrido, si ya se hizo).
    Walk(Option<Vec<Planned>>),
}

struct Item {
    src: PathBuf,
    target: Target,
    how: How,
    /// «Devolver»: la carpeta del lugar original, ya preparada; se vuelve a mirar
    /// (`CopyPrims::prepare_restore` sin crear) justo antes de escribir ahí.
    restore_dir: Option<PathBuf>,
}

impl Item {
    fn new(src: &Path, target: Target, how: How) -> Item {
        Item { src: src.to_path_buf(), target, how, restore_dir: None }
    }
}

/// Lo que se decidió para un destino.
enum Decision {
    Skip,
    /// Escribir en esa ruta (libre).
    Fresh(PathBuf),
    /// Entrar en la carpeta que ya está.
    Merge(PathBuf),
    /// Reemplazar lo que está en esa ruta: se aparta (renombrado sin pisar) recién cuando lo nuevo
    /// está listo para ponerse (archivos) o antes de empezar a llenar la carpeta nueva; lo
    /// apartado va a la Papelera cuando lo nuevo ya está en su lugar, o vuelve si algo falla.
    Replace(PathBuf),
}

/// Una carpeta abierta mientras se copia su contenido.
struct Frame {
    rel: PathBuf,
    src: PathBuf,
    dst: PathBuf,
    created: bool,
    errors_before: usize,
    /// «Reemplazar» una carpeta: dónde quedó apartada la que estaba.
    aside: Option<PathBuf>,
    /// Anotar el deshacer al cerrar (después de lo apartado, así se deshace en el orden justo).
    record_later: bool,
}

/// Resultado de mover renombrando.
enum Renamed {
    Done(Option<PathBuf>),
    /// El sistema dice que es otro disco: hay que copiar, con lo que ya se decidió (no se vuelve
    /// a preguntar).
    CrossDevice(Decision),
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Nombre oculto para un temporal del motor en `dir`.
fn tmp_in(dir: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".xdir-{}-{n}.tmp", std::process::id()))
}

/// Nombre con que se escribe un archivo nuevo hasta que está entero: «.informe.pdf.xdir-part» al
/// lado del destino (oculto donde los puntos ocultan, y reconocible). Si el nombre es muy largo o
/// ese parcial ya existe (quedó de otra vez), uno único del motor.
fn part_name(dst: &Path, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let dir = dst.parent().unwrap_or(Path::new(""));
    let name = name_of(dst);
    let part = dir.join(format!(".{name}.xdir-part"));
    if name.len() <= 200 && !taken(&part) {
        return part;
    }
    short_part_name(dst)
}

/// Lo que estaba en `from` ahora está en `to` (la carpeta cambió de nombre): el deshacer apunta ahí.
fn remap_undo(u: &mut Undo, from: &Path, to: &Path) {
    let fix = |p: &mut PathBuf| {
        if let Ok(rest) = p.strip_prefix(from) {
            *p = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
        }
    };
    match u {
        Undo::Batch(v) => v.iter_mut().for_each(|u| remap_undo(u, from, to)),
        Undo::Created(v) => v.iter_mut().for_each(fix),
        Undo::CreatedIds(v) => v.iter_mut().for_each(|(p, _)| fix(p)),
        Undo::Moved(v) => v.iter_mut().for_each(|(now, _)| fix(now)),
        Undo::Recycled(v) => v.iter_mut().for_each(|(_, p)| fix(p)),
        Undo::CreatedFolder(p) => fix(p),
        Undo::Rename { path, .. } => fix(path),
        Undo::Renames(v) => v.iter_mut().for_each(|(p, _)| fix(p)),
    }
}

/// Un parcial corto y único del motor (si el disco no acepta el nombre largo).
fn short_part_name(dst: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dst.parent().unwrap_or(Path::new("")).join(format!(".xdir-{}-{n}.xdir-part", std::process::id()))
}

/// Reintentos del renombre final mientras el archivo está ocupado, y la pausa entre ellos.
const RENAME_RETRIES: u32 = 5;
#[cfg(not(test))]
const RENAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(200);
#[cfg(test)]
const RENAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

struct Job<'a> {
    prims: &'a dyn CopyPrims,
    archives: &'a dyn Archives,
    ctl: &'a Control,
    out: &'a mut dyn FnMut(Update),
    items_done: u64,
    bytes_done: u64,
    current: String,
    last_progress: Option<Instant>,
    errors: Vec<JobError>,
    undo: Vec<Undo>,
    select: Vec<PathBuf>,
    focus: Option<PathBuf>,
    rename_after: bool,
    file_all: Option<Choice>,
    dir_all: Option<Choice>,
    cancelled: bool,
    stopped: Option<String>,
    skipped: Vec<PathBuf>,
    /// Lo que el recorrido no pudo leer (cuenta como error de la carpeta que lo contiene).
    walk_failed: Vec<PathBuf>,
    /// Conflictos de arriba que se ven por delante (por índice de elemento): `None` si no choca.
    ahead: Vec<Option<Ahead>>,
    /// Se está resolviendo adentro de una carpeta que se combina (los conflictos de adentro no
    /// se cuentan por delante).
    merging: usize,
    /// Elementos que llegaron (`JobOutcome::done_items`).
    ok_items: u64,
    /// «Devolver» del elemento actual: (lo de la Papelera, la carpeta del lugar original).
    restore_guard: Option<(PathBuf, PathBuf)>,
    index: usize,
    /// «Devolver»: lo movido se deshace mandándolo de nuevo a la Papelera.
    moves_as_created: bool,
    /// Extraer: lo que no se pudo crear (`JobOutcome::omitted`).
    omitted: usize,
}

impl<'a> Job<'a> {
    fn new(prims: &'a dyn CopyPrims, ctl: &'a Control, out: &'a mut dyn FnMut(Update)) -> Job<'a> {
        Job {
            prims,
            archives: &NoArchives,
            ctl,
            out,
            items_done: 0,
            bytes_done: 0,
            current: String::new(),
            last_progress: None,
            errors: Vec::new(),
            undo: Vec::new(),
            select: Vec::new(),
            focus: None,
            rename_after: false,
            file_all: None,
            dir_all: None,
            cancelled: false,
            stopped: None,
            skipped: Vec::new(),
            walk_failed: Vec::new(),
            ahead: Vec::new(),
            merging: 0,
            ok_items: 0,
            restore_guard: None,
            index: 0,
            moves_as_created: false,
            omitted: 0,
        }
    }

    fn halted(&self) -> bool {
        self.cancelled || self.stopped.is_some()
    }

    /// ¿Hay que cortar? (cancelado, disco lleno o en pausa hasta que siga).
    fn should_stop(&mut self) -> bool {
        if self.halted() {
            return true;
        }
        if self.ctl.checkpoint() == Flow::Stop {
            self.cancelled = true;
            return true;
        }
        false
    }

    // ---------- avisos ----------

    fn start(&mut self, title: String, items: u64, bytes: u64) {
        (self.out)(Update::Started { title, items, bytes });
    }

    fn progress(&mut self, partial: u64, force: bool) {
        let now = Instant::now();
        if !force && self.last_progress.is_some_and(|t| now.duration_since(t) < PROGRESS_EVERY) {
            return;
        }
        self.last_progress = Some(now);
        let update = Update::Progress { items: self.items_done, bytes: self.bytes_done + partial, current: self.current.clone() };
        (self.out)(update);
    }

    fn finish(mut self) {
        self.progress(0, true);
        let undo = (!self.undo.is_empty()).then(|| Undo::Batch(std::mem::take(&mut self.undo)));
        let focus = self.focus.take().or_else(|| self.select.last().cloned());
        let result = OpResult { undo, focus, rename_after: self.rename_after, error: None, select: std::mem::take(&mut self.select) };
        let outcome = JobOutcome {
            result,
            errors: std::mem::take(&mut self.errors),
            stopped: self.stopped.take(),
            cancelled: self.cancelled,
            skipped: std::mem::take(&mut self.skipped),
            done_items: self.ok_items,
            omitted: self.omitted,
        };
        (self.out)(Update::Finished(outcome));
    }

    /// Deshacer no se deshace: sin deshacer ni selección.
    fn discard_result(&mut self) {
        self.undo.clear();
        self.select.clear();
        self.focus = None;
    }

    // ---------- errores y deshacer ----------

    fn fail(&mut self, src: &Path, dst: &Path, e: CopyError) {
        match e.kind {
            ErrKind::Cancelled => self.cancelled = true,
            ErrKind::DiskFull => {
                self.stopped = Some(e.message());
                self.errors.push(JobError { src: src.to_path_buf(), dst: dst.to_path_buf(), message: e.message(), retry: true });
            }
            _ => self.errors.push(JobError { src: src.to_path_buf(), dst: dst.to_path_buf(), message: e.message(), retry: true }),
        }
    }

    /// Un error con su texto; `retry`: «Reintentar» lo vuelve a intentar (no, si es un aviso).
    fn fail_msg(&mut self, src: &Path, dst: &Path, message: &str, retry: bool) {
        self.errors.push(JobError { src: src.to_path_buf(), dst: dst.to_path_buf(), message: message.into(), retry });
    }

    /// Anota un paso de deshacer, juntándolo con el anterior si es del mismo tipo.
    fn record(&mut self, step: Undo) {
        let step = match step {
            Undo::Moved(pairs) if self.moves_as_created => Undo::Created(pairs.into_iter().map(|(now, _)| now).collect()),
            other => other,
        };
        // Lo creado se recuerda con su identidad (si el disco la da): deshacer solo toca eso.
        let step = match step {
            Undo::Created(paths) => {
                let ids: Option<Vec<_>> = paths.iter().map(|p| self.prims.stat(p).ok().and_then(|s| s.file_id).map(|id| (p.clone(), id))).collect();
                match ids {
                    Some(ids) => Undo::CreatedIds(ids),
                    None => Undo::Created(paths),
                }
            }
            other => other,
        };
        match (self.undo.last_mut(), step) {
            (Some(Undo::Created(a)), Undo::Created(b)) => a.extend(b),
            (Some(Undo::CreatedIds(a)), Undo::CreatedIds(b)) => a.extend(b),
            (Some(Undo::Moved(a)), Undo::Moved(b)) => a.extend(b),
            (Some(Undo::Recycled(a)), Undo::Recycled(b)) => a.extend(b),
            (_, step) => self.undo.push(step),
        }
    }

    fn record_done(&mut self, src: &Path, dst: &Path, moving: bool) {
        if moving {
            self.record(Undo::Moved(vec![(dst.to_path_buf(), src.to_path_buf())]));
        } else {
            self.record(Undo::Created(vec![dst.to_path_buf()]));
        }
    }

    // ---------- discos y nombres ----------

    fn exists(&self, p: &Path) -> Option<ItemStat> {
        self.prims.stat(p).ok()
    }

    /// ¿Mismo disco? `None` si no se sabe (se intenta renombrar y, si el sistema dice que es otro
    /// disco, se copia).
    fn same_volume(&self, a: &Path, b: &Path) -> Option<bool> {
        match (self.prims.volume(a), self.prims.volume(b)) {
            (Some(x), Some(y)) => Some(x.id == y.id),
            _ => None,
        }
    }

    /// Sin saber del disco, se compara sin mayúsculas (más conflictos, nunca menos).
    fn case_insensitive(&self, p: &Path) -> bool {
        self.prims.volume(p).is_none_or(|v| v.case_insensitive)
    }

    /// ¿Es la misma ruta? (sin distinguir mayúsculas donde el disco no las distingue).
    fn same_path(&self, a: &Path, b: &Path) -> bool {
        a == b || (self.case_insensitive(a) && sys::same_path_folded(a, b))
    }

    /// ¿Es la misma carpeta? (por la ruta o por la identidad: otro camino a la misma).
    fn same_dir(&self, a: &Path, b: &Path) -> bool {
        self.same_path(a, b) || matches!((self.exists(a), self.exists(b)), (Some(x), Some(y)) if x.same_item(&y))
    }

    /// ¿`inner` está dentro de `outer` (o es la misma)?
    fn inside(&self, inner: &Path, outer: &Path) -> bool {
        if inner.starts_with(outer) {
            return true;
        }
        if self.case_insensitive(outer) && sys::starts_with_folded(inner, outer) {
            return true;
        }
        // Otro camino a la misma carpeta: alguna de arriba de `inner` es `outer`.
        let Some(target) = self.exists(outer).filter(|s| s.kind == ItemKind::Dir && s.file_id.is_some()) else { return false };
        inner.ancestors().any(|a| self.exists(a).is_some_and(|s| s.same_item(&target)))
    }

    /// ¿El disco tiene Papelera? `None` si no se sabe.
    fn has_trash(&self, p: &Path) -> Option<bool> {
        let dir = p.parent().unwrap_or(p);
        self.prims.volume(dir).map(|v| v.has_trash)
    }

    fn free_numbered(&self, dst: &Path, is_dir: bool) -> PathBuf {
        let name = name_of(dst);
        let parent = dst.parent().unwrap_or(Path::new(""));
        free_name(parent, &|n| numbered_name(&name, is_dir, n.max(2)), &|p| self.exists(p).is_some())
    }

    fn free_copy(&self, dir: &Path, name: &str, is_dir: bool) -> PathBuf {
        free_name(dir, &|n| copy_name(name, is_dir, n), &|p| self.exists(p).is_some())
    }

    // ---------- armar los trabajos ----------

    /// Copiar o mover `sources` a `dest`: lo que va, con las comprobaciones de entrada.
    fn plan_into(&mut self, sources: &[PathBuf], dest: &Path, moving: bool) -> Vec<Item> {
        let mut items = Vec::new();
        for src in sources {
            let Some(name) = src.file_name() else { continue };
            if self.inside(dest, src) {
                self.fail_msg(src, &dest.join(name), INTO_ITSELF, false);
                continue;
            }
            if src.parent().is_some_and(|p| self.same_dir(p, dest)) {
                // Mover a la misma carpeta no hace nada; copiar ahí es duplicar.
                if !moving {
                    items.push(Item::new(src, Target::Duplicate(dest.to_path_buf()), How::Walk(None)));
                }
                continue;
            }
            items.push(self.item_to(src, dest.join(name), moving));
        }
        items
    }

    fn item_to(&self, src: &Path, dst: PathBuf, moving: bool) -> Item {
        let rename = moving && dst.parent().is_some_and(|parent| self.same_volume(src, parent) != Some(false));
        Item::new(src, Target::Fixed(dst), if rename { How::Rename } else { How::Walk(None) })
    }

    /// «Reintentar»: los pares que fallaron, con las mismas comprobaciones de entrada.
    fn plan_retry(&mut self, pairs: &[(PathBuf, PathBuf)], moving: bool) -> Vec<Item> {
        let mut items = Vec::new();
        for (src, dst) in pairs {
            if dst.parent().is_some_and(|d| self.inside(d, src)) {
                self.fail_msg(src, dst, INTO_ITSELF, false);
                continue;
            }
            // Un «Devolver» (mover algo de la Papelera a su lugar original): siempre con sus reglas.
            // Copiar o mover a otro lado desde la Papelera sigue siendo eso.
            if moving && self.prims.trash_origin(src).as_deref() == Some(dst.as_path()) {
                if let Some(item) = self.restore_item(src, dst) {
                    items.push(item);
                }
                continue;
            }
            items.push(self.item_to(src, dst.clone(), moving));
        }
        items
    }

    /// Como `restore_still_safe`, pero de la carpeta `dir` misma (antes de ponerle permisos y
    /// fechas al cerrarla, o de renombrarla): ni ella ni ninguna del camino puede ser un enlace.
    fn restore_dir_still_safe(&mut self, src: &Path, dir: &Path) -> bool {
        let Some((in_trash, root)) = self.restore_guard.clone() else { return true };
        if !dir.starts_with(&root) {
            return true;
        }
        match self.prims.prepare_restore(&in_trash, dir, false) {
            Ok(()) => true,
            Err(e) => {
                self.fail_msg(src, dir, &e.message(), false);
                false
            }
        }
    }

    /// «Devolver» `src` a `origin`: prepara la carpeta sin seguir enlaces (reglas de cada sistema).
    /// Un rechazo no se ofrece para «Reintentar» (sería lo mismo, o un camino sin controles).
    fn restore_item(&mut self, src: &Path, origin: &Path) -> Option<Item> {
        let dir = origin.parent()?.to_path_buf();
        if let Err(e) = self.prims.prepare_restore(src, &dir, true) {
            self.fail_msg(src, origin, &e.message(), false);
            return None;
        }
        let mut item = self.item_to(src, origin.to_path_buf(), true);
        item.restore_dir = Some(dir);
        Some(item)
    }

    /// Justo antes de escribir en `dst`: si se está devolviendo ahí, se vuelve a mirar el camino
    /// entero hasta la carpeta de `dst` (también las de adentro que se combinan o que creó este
    /// trabajo): ninguna puede ser un enlace (pudo cambiar mientras se esperaba una respuesta). Si
    /// no, error (sin reintento) y no se escribe nada.
    fn restore_still_safe(&mut self, src: &Path, dst: &Path) -> bool {
        let Some((in_trash, dir)) = self.restore_guard.clone() else { return true };
        if !dst.starts_with(&dir) {
            return true;
        }
        let parent = dst.parent().unwrap_or(&dir);
        match self.prims.prepare_restore(&in_trash, parent, false) {
            Ok(()) => true,
            Err(e) => {
                self.fail_msg(src, dst, &e.message(), false);
                false
            }
        }
    }

    /// Anota los errores del recorrido de `item`, con el destino que les tocaba.
    fn walk_errors(&mut self, item: &Item, errors: Vec<CopyError>) {
        let hint = self.target_hint(item);
        let (base, dst_base) = (item.src.parent().map(Path::to_path_buf).unwrap_or_default(), hint.parent().map(Path::to_path_buf).unwrap_or_default());
        for e in errors {
            let dst = e.path.strip_prefix(&base).map(|rel| dst_base.join(rel)).unwrap_or_else(|_| hint.clone());
            let src = e.path.clone();
            self.walk_failed.push(src.clone());
            self.fail(&src, &dst, e);
        }
    }

    /// Recorre, cuenta, comprueba espacio y tamaños, y lleva cada elemento.
    fn run_items(&mut self, title: String, mut items: Vec<Item>, moving: bool) {
        // Recorrido con metadatos.
        let mut seen_before = 0u64;
        let mut keep = Vec::with_capacity(items.len());
        for mut item in items.drain(..) {
            if matches!(item.how, How::Walk(None)) {
                let mut on_seen = |n: u64| (self.out)(Update::Preparing { items: seen_before + n });
                match walk(self.prims, &item.src, self.ctl, &mut on_seen) {
                    Ok((plan, errors)) => {
                        seen_before += plan.len() as u64;
                        item.how = How::Walk(Some(plan));
                        self.walk_errors(&item, errors);
                    }
                    Err(e) => {
                        let dst = self.target_hint(&item);
                        self.fail(&item.src, &dst, e);
                        if self.halted() {
                            break;
                        }
                        continue;
                    }
                }
            }
            keep.push(item);
        }
        let mut items = keep;
        if self.halted() {
            return;
        }

        // Archivos demasiado grandes para el disco de destino: error de entrada.
        let mut needed: HashMap<std::ffi::OsString, (u64, String, u64)> = HashMap::new();
        let (mut total_items, mut total_bytes) = (0u64, 0u64);
        for item in &mut items {
            let hint = self.target_hint(item);
            let dir = hint.parent().map(Path::to_path_buf).unwrap_or_default();
            let vol = self.prims.volume(&dir);
            match &mut item.how {
                How::Rename => total_items += 1,
                How::Walk(Some(plan)) => {
                    if let Some(max) = vol.as_ref().and_then(|v| v.max_file) {
                        let mut big = Vec::new();
                        plan.retain(|p| {
                            let too_big = p.stat.kind == ItemKind::File && p.stat.size > max;
                            if too_big {
                                big.push(p.clone());
                            }
                            !too_big
                        });
                        for b in big {
                            let dst = dir.join(&b.rel);
                            self.fail(&b.src, &dst, CopyError::new(ErrKind::TooBig, &b.src));
                        }
                    }
                    let (n, bytes) = totals(plan);
                    total_items += n;
                    total_bytes += bytes;
                    if let Some(v) = vol {
                        needed.entry(v.id.clone()).or_insert((0, v.label.clone(), v.free)).0 += bytes;
                    }
                }
                How::Walk(None) => {}
            }
        }

        // Espacio: si no alcanza, no empieza.
        if let Some((bytes, label, free)) = needed.values().find(|(bytes, _, free)| bytes > free) {
            let missing = sys::env().format_size_short(bytes - free);
            self.stopped = Some(format!("Faltan {missing} en \"{label}\""));
            return;
        }

        self.start(title, total_items, total_bytes);
        self.ahead = items
            .iter()
            .map(|i| match &i.target {
                Target::Fixed(dst) if !self.same_path(&i.src, dst) => self.ahead_of(&i.src, dst),
                _ => None,
            })
            .collect();
        for (i, item) in items.into_iter().enumerate() {
            self.index = i;
            if self.should_stop() {
                break;
            }
            self.process(item, moving);
        }
    }

    /// Si `dst` ya existe, qué conflicto va a ser (sin leer contenido).
    fn ahead_of(&self, src: &Path, dst: &Path) -> Option<Ahead> {
        let existing = self.exists(dst)?;
        let is_dir = self.prims.stat(src).is_ok_and(|s| s.kind == ItemKind::Dir);
        Some(Ahead { is_dir, can_merge: is_dir && existing.kind == ItemKind::Dir })
    }

    /// Cuántos conflictos de la misma clase quedan por delante, si se puede saber.
    fn remaining(&self, is_dir: bool) -> Option<usize> {
        if self.merging > 0 {
            return None;
        }
        let rest: Vec<Ahead> = self.ahead.iter().skip(self.index + 1).flatten().copied().collect();
        if rest.iter().any(|a| a.can_merge) {
            return None;
        }
        Some(rest.iter().filter(|a| a.is_dir == is_dir).count())
    }

    /// Destino aproximado de un elemento (para los errores de antes de decidir).
    fn target_hint(&self, item: &Item) -> PathBuf {
        match &item.target {
            Target::Fixed(p) => p.clone(),
            Target::Duplicate(dir) => dir.join(item.src.file_name().unwrap_or_default()),
        }
    }

    /// Lleva un elemento de arriba.
    fn process(&mut self, item: Item, moving: bool) {
        let Some(dir) = item.restore_dir.clone() else { return self.process_item(item, moving) };
        let (src, origin) = (item.src.clone(), self.target_hint(&item));
        self.restore_guard = Some((src.clone(), dir));
        self.process_item(item, moving);
        self.restore_guard = None;
        if self.exists(&src).is_none() && self.exists(&origin).is_some() {
            self.prims.restored(&src);
        }
    }

    fn process_item(&mut self, item: Item, moving: bool) {
        self.current = name_of(&item.src);
        if let (How::Rename, Target::Fixed(dst)) = (&item.how, &item.target) {
            match self.rename_tree(&item.src, dst) {
                Renamed::Done(done) => {
                    self.ok_items += done.is_some() as u64;
                    self.select.extend(done);
                    self.items_done += 1;
                    self.progress(0, false);
                    return;
                }
                // No era el mismo disco: se copia (y se borra el original archivo por archivo),
                // con lo que ya se decidió.
                Renamed::CrossDevice(decision) => {
                    self.copy_with(&item.src, decision, moving);
                    return;
                }
            }
        }
        let plan = match item.how {
            How::Walk(Some(p)) => p,
            _ => match walk(self.prims, &item.src, self.ctl, &mut |_| {}) {
                Ok((p, errors)) => {
                    self.walk_errors(&item, errors);
                    p
                }
                Err(e) => {
                    let hint = self.target_hint(&item);
                    self.fail(&item.src, &hint, e);
                    return;
                }
            },
        };
        let Some(first) = plan.first() else { return };
        let decision = match &item.target {
            Target::Duplicate(dir) => {
                // Elegido libre: nunca es conflicto.
                Decision::Fresh(self.free_copy(dir, &name_of(&item.src), first.stat.kind == ItemKind::Dir))
            }
            Target::Fixed(dst) => {
                let (src, stat) = (first.src.clone(), first.stat);
                self.decide(&src, &stat, dst, moving)
            }
        };
        self.copy_tree(plan, decision, moving);
    }

    /// Copia (o mueve copiando) `src` con la decisión ya tomada para su destino.
    fn copy_with(&mut self, src: &Path, decision: Decision, moving: bool) {
        let hint = match &decision {
            Decision::Fresh(p) | Decision::Merge(p) | Decision::Replace(p) => p.clone(),
            Decision::Skip => return,
        };
        let item = Item::new(src, Target::Fixed(hint.clone()), How::Walk(None));
        match walk(self.prims, src, self.ctl, &mut |_| {}) {
            Ok((plan, errors)) => {
                self.walk_errors(&item, errors);
                if !plan.is_empty() {
                    self.copy_tree(plan, decision, moving);
                }
            }
            Err(e) => self.fail(src, &hint, e),
        }
    }

    // ---------- conflictos ----------

    /// Decide qué hacer con `dst` (si existe, pregunta o usa «Hacer esto con todos»).
    fn decide(&mut self, src: &Path, incoming: &ItemStat, dst: &Path, moving: bool) -> Decision {
        let Some(existing) = self.exists(dst) else { return Decision::Fresh(dst.to_path_buf()) };
        let is_dir = incoming.kind == ItemKind::Dir;
        if self.same_path(src, dst) || incoming.same_item(&existing) {
            // El mismo elemento (quizás por otro camino): mover no hace nada; copiar duplica.
            if moving {
                // Otro nombre del mismo archivo (un enlace duro): se avisa como omitido.
                if !self.same_path(src, dst) {
                    self.skipped.push(src.to_path_buf());
                }
                return Decision::Skip;
            }
            let dir = dst.parent().unwrap_or(Path::new(""));
            return Decision::Fresh(self.free_copy(dir, &name_of(dst), is_dir));
        }
        let can_merge = is_dir && existing.kind == ItemKind::Dir;
        let stored = if is_dir { self.dir_all } else { self.file_all };
        let choice = match stored {
            Some(Choice::Merge) if !can_merge => self.ask(src, dst, incoming, existing, is_dir, can_merge),
            Some(c) => Some(c),
            None => self.ask(src, dst, incoming, existing, is_dir, can_merge),
        };
        let Some(choice) = choice else { return Decision::Skip };
        match choice {
            Choice::Skip => {
                self.skipped.push(src.to_path_buf());
                Decision::Skip
            }
            Choice::Merge if can_merge => Decision::Merge(dst.to_path_buf()),
            Choice::KeepBoth | Choice::Merge => Decision::Fresh(self.free_numbered(dst, is_dir)),
            Choice::Replace => {
                if self.inside(src, dst) {
                    self.fail_msg(src, dst, REPLACE_PARENT, false);
                    return Decision::Skip;
                }
                Decision::Replace(dst.to_path_buf())
            }
        }
    }

    fn ask(&mut self, src: &Path, dst: &Path, incoming: &ItemStat, existing: ItemStat, is_dir: bool, can_merge: bool) -> Option<Choice> {
        let remaining = self.remaining(is_dir);
        let folder = dst.parent().map(name_of).unwrap_or_default();
        let info = ConflictInfo { name: name_of(src), folder, is_dir, can_merge, incoming: *incoming, existing, remaining };
        // Una respuesta que llegó tarde a otro conflicto no vale para este.
        self.ctl.clear_answer();
        (self.out)(Update::Conflict(info));
        let Some(Answer { choice, all }) = self.ctl.wait_answer() else {
            self.cancelled = true;
            return None;
        };
        if all {
            if is_dir {
                self.dir_all = Some(choice);
            } else {
                self.file_all = Some(choice);
            }
        }
        // Lo que tardó en contestar no cuenta para la velocidad del aviso siguiente.
        self.last_progress = None;
        Some(choice)
    }

    // ---------- lo reemplazado ----------

    /// Aparta lo que está en `dst` (renombrado sin pisar a un temporal oculto al lado).
    fn set_aside(&mut self, src: &Path, dst: &Path) -> Option<PathBuf> {
        if !self.restore_still_safe(src, dst) {
            return None;
        }
        // A la vista y con un nombre que lo explica («informe (reemplazado).pdf»): si xdir se
        // corta antes de tirarlo, queda ahí; y en la Papelera se reconoce.
        let is_dir = self.exists(dst).is_some_and(|s| s.kind == ItemKind::Dir);
        let name = name_of(dst);
        let parent = dst.parent().unwrap_or(Path::new(""));
        let aside = free_name(parent, &|n| replaced_name(&name, is_dir, n), &|p| self.exists(p).is_some());
        match self.rename_final(dst, &aside) {
            Ok(()) => Some(aside),
            Err(e) => {
                self.fail(src, dst, e);
                None
            }
        }
    }

    /// Devuelve lo apartado a `dst`; si no se puede, lo deja a la vista con otro nombre.
    fn put_back(&mut self, aside: &Path, dst: &Path) {
        if self.prims.rename_noreplace(aside, dst).is_err() {
            self.keep_visible(aside, dst, "Lo que estaba antes quedó como");
        }
    }

    /// Lo apartado se queda donde está (a la vista, «… (reemplazado)»): se avisa.
    fn keep_visible(&mut self, aside: &Path, dst: &Path, why: &str) {
        self.fail_msg(dst, aside, &format!("{why} «{}»", name_of(aside)), false);
    }

    /// Lo nuevo ya está en `dst`: lo apartado va a la Papelera (si el disco no tiene, se borra,
    /// que es lo que pidió «Reemplazar»). Si la Papelera falla, queda a la vista y se avisa.
    fn dispose_aside(&mut self, aside: &Path, dst: &Path) {
        if self.has_trash(dst) == Some(false) {
            self.remove_tree(aside);
            return;
        }
        // En la Papelera, con el nombre y el lugar de lo que reemplazó donde el sistema lo permite.
        match self.prims.trash_as(aside, dst) {
            Ok(name) => self.record(Undo::Recycled(vec![(name, dst.to_path_buf())])),
            Err(e) => {
                if self.exists(aside).is_some() {
                    self.keep_visible(aside, dst, "No se pudo mandar a la Papelera lo que estaba antes: quedó como");
                } else {
                    let msg = format!("Lo que estaba antes no entró en la Papelera y se borró ({})", e.message());
                    self.fail_msg(dst, dst, &msg, false);
                }
            }
        }
    }

    // ---------- copiar (o mover entre discos) elemento por elemento ----------

    fn copy_tree(&mut self, plan: Vec<Planned>, first: Decision, moving: bool) {
        // Destino de cada carpeta ya resuelta: (ruta, la creó este trabajo). `None`: omitida.
        let mut dests: HashMap<PathBuf, Option<(PathBuf, bool)>> = HashMap::new();
        let mut stack: Vec<Frame> = Vec::new();
        let mut first = Some(first);
        for item in plan {
            while let Some(top) = stack.last() {
                if item.rel.starts_with(&top.rel) {
                    break;
                }
                let frame = stack.pop().unwrap();
                self.close_dir(frame, moving);
            }
            if self.should_stop() {
                break;
            }
            self.current = name_of(&item.src);
            let is_root = first.is_some();
            let (decision, parent_created) = match first.take() {
                Some(d) => (d, false),
                None => {
                    let parent = item.rel.parent().map(Path::to_path_buf).unwrap_or_default();
                    match dests.get(&parent) {
                        Some(Some((dir, created))) => {
                            let dst = dir.join(item.src.file_name().unwrap_or_default());
                            // Adentro de una carpeta creada por este trabajo no hay nada.
                            let d = if *created {
                                Decision::Fresh(dst)
                            } else {
                                // Adentro de una carpeta que se combina.
                                self.merging += 1;
                                let d = self.decide(&item.src, &item.stat, &dst, moving);
                                self.merging -= 1;
                                d
                            };
                            (d, *created)
                        }
                        _ => (Decision::Skip, false),
                    }
                }
            };
            let record = !parent_created;
            let size = if item.stat.kind == ItemKind::File { item.stat.size } else { 0 };
            let is_dir = item.stat.kind == ItemKind::Dir;
            match decision {
                Decision::Skip => {
                    if is_dir {
                        dests.insert(item.rel.clone(), None);
                    }
                }
                Decision::Merge(dst) => {
                    self.ok_items += 1;
                    dests.insert(item.rel.clone(), Some((dst.clone(), false)));
                    if is_root {
                        self.select.push(dst.clone());
                    }
                    let errors_before = self.errors.len();
                    stack.push(Frame { rel: item.rel.clone(), src: item.src.clone(), dst, created: false, errors_before, aside: None, record_later: false });
                }
                Decision::Fresh(dst) if is_dir => self.open_dir(&item, dst, None, moving, record, is_root, &mut dests, &mut stack),
                Decision::Replace(dst) if is_dir => {
                    // La carpeta que estaba se aparta entera antes de llenar la nueva; vuelve si
                    // algo falla.
                    match self.set_aside(&item.src, &dst) {
                        Some(aside) => self.open_dir(&item, dst, Some(aside), moving, record, is_root, &mut dests, &mut stack),
                        None => {
                            dests.insert(item.rel.clone(), None);
                        }
                    }
                }
                Decision::Fresh(dst) => {
                    let ok = self.copy_one(&item, &dst, moving, record);
                    self.ok_items += ok as u64;
                    if ok && is_root {
                        self.select.push(dst);
                    }
                }
                Decision::Replace(dst) => {
                    let ok = self.replace_one(&item, &dst, moving, record);
                    self.ok_items += ok as u64;
                    if ok && is_root {
                        self.select.push(dst);
                    }
                }
            }
            self.items_done += 1;
            self.bytes_done += size;
            self.progress(0, false);
        }
        while let Some(frame) = stack.pop() {
            self.close_dir(frame, moving);
        }
    }

    /// Crea la carpeta `dst` (libre) para `item` y la abre para su contenido. `aside`: la que
    /// estaba, apartada por «Reemplazar».
    #[allow(clippy::too_many_arguments)]
    fn open_dir(
        &mut self,
        item: &Planned,
        dst: PathBuf,
        aside: Option<PathBuf>,
        moving: bool,
        record: bool,
        is_root: bool,
        dests: &mut HashMap<PathBuf, Option<(PathBuf, bool)>>,
        stack: &mut Vec<Frame>,
    ) {
        if !self.restore_still_safe(&item.src, &dst) {
            dests.insert(item.rel.clone(), None);
            if let Some(aside) = aside {
                self.put_back(&aside, &dst);
            }
            return;
        }
        match self.prims.create_dir(&dst) {
            Ok(()) => {
                self.ok_items += 1;
                // Con algo apartado, el deshacer se anota al cerrar (después de la Papelera).
                let record_later = record && aside.is_some();
                if record && aside.is_none() {
                    self.record_done(&item.src, &dst, moving);
                }
                if is_root {
                    self.select.push(dst.clone());
                }
                dests.insert(item.rel.clone(), Some((dst.clone(), true)));
                let errors_before = self.errors.len();
                stack.push(Frame { rel: item.rel.clone(), src: item.src.clone(), dst, created: true, errors_before, aside, record_later });
            }
            Err(e) => {
                self.fail(&item.src, &dst, e);
                dests.insert(item.rel.clone(), None);
                if let Some(aside) = aside {
                    self.put_back(&aside, &dst);
                }
            }
        }
    }

    /// Copia (o mueve) un archivo o un enlace a `dst`, que está libre. `record`: anota el
    /// deshacer (no hace falta dentro de una carpeta creada por este trabajo).
    fn copy_one(&mut self, item: &Planned, dst: &Path, moving: bool, record: bool) -> bool {
        if !self.restore_still_safe(&item.src, dst) {
            return false;
        }
        // Se escribe con un nombre parcial y recién entero toma el suyo: si xdir se corta a mitad,
        // nunca queda algo a medias con el nombre del archivo.
        let mut part = part_name(dst, &|p| self.exists(p).is_some());
        let mut written = self.write(item, &part);
        if matches!(&written, Err(e) if e.kind == ErrKind::NameTooLong) {
            // El disco no acepta el nombre largo del parcial: uno corto del motor.
            part = short_part_name(dst);
            written = self.write(item, &part);
        }
        if let Err(e) = written {
            if e.kind != ErrKind::Exists {
                let _ = self.prims.remove(&part);
            }
            self.fail(&item.src, dst, e);
            return false;
        }
        if moving && !self.verify_move(item, &part, dst) {
            return false;
        }
        if self.halted() || self.ctl.is_cancelled() {
            let _ = self.prims.remove(&part);
            self.cancelled = true;
            return false;
        }
        if let Err(e) = self.rename_final(&part, dst) {
            // Algo ocupó el nombre mientras se copiaba: no se pisa.
            let _ = self.prims.remove(&part);
            self.fail(&item.src, dst, e);
            return false;
        }
        // Al mover: el nombre nuevo también queda guardado antes de borrar el original.
        if moving
            && let Err(e) = self.prims.sync_dir(dst.parent().unwrap_or(Path::new("")))
        {
            self.fail(&item.src, dst, e);
            if record {
                self.record(Undo::Created(vec![dst.to_path_buf()]));
            }
            return false;
        }
        if moving
            && let Err(e) = self.prims.remove(&item.src)
        {
            // Quedó copiado y el original sigue: se anota como copia (deshacer la borra).
            self.fail(&item.src, dst, e);
            if record {
                self.record(Undo::Created(vec![dst.to_path_buf()]));
            }
            return false;
        }
        if record {
            self.record_done(&item.src, dst, moving);
        }
        true
    }

    /// El parcial toma su nombre de verdad. Si el archivo está ocupado o sin permiso por un momento
    /// (un antivirus o el indexador lo abrieron recién escrito), se reintenta un rato.
    fn rename_final(&self, part: &Path, dst: &Path) -> Result<(), CopyError> {
        let mut tries = 0;
        loop {
            match self.prims.rename_noreplace(part, dst) {
                Err(e) if self.prims.transient(&e.kind) && tries < RENAME_RETRIES && !self.ctl.is_cancelled() => {
                    tries += 1;
                    std::thread::sleep(RENAME_PAUSE);
                }
                other => return other,
            }
        }
    }

    /// Escribe el contenido de `item` en `to` (que no existe), con progreso.
    fn write(&mut self, item: &Planned, to: &Path) -> Result<(), CopyError> {
        let prims = self.prims;
        match item.stat.kind {
            ItemKind::Symlink => prims.copy_symlink(&item.src, to),
            _ => {
                let mut on_block = |copied: u64| {
                    self.progress(copied, false);
                    self.ctl.checkpoint()
                };
                prims.copy_file(&item.src, to, &mut on_block)
            }
        }
    }

    /// Al mover entre discos, antes de borrar el original: la copia en `written` quedó del mismo
    /// tamaño, el original no cambió mientras se copiaba y la copia está guardada en el disco. Si
    /// algo no da, se borra la copia (el original queda) y se anota con `dst`.
    fn verify_move(&mut self, item: &Planned, written: &Path, dst: &Path) -> bool {
        if item.stat.kind != ItemKind::File {
            return true;
        }
        let problem = match (self.prims.stat(written), self.prims.stat(&item.src)) {
            (Ok(_), Ok(now)) if now.size != item.stat.size || now.modified != item.stat.modified => Some(CHANGED.to_string()),
            (Ok(copy), Ok(now)) if copy.size != now.size => Some(NOT_SAME.to_string()),
            (Ok(_), Ok(_)) => self.prims.sync(written).err().map(|e| e.message()),
            (Err(_), _) => Some(NOT_SAME.to_string()),
            (_, Err(e)) => Some(e.message()),
        };
        match problem {
            None => true,
            Some(msg) => {
                let _ = self.prims.remove(written);
                self.fail_msg(&item.src, dst, &msg, true);
                false
            }
        }
    }

    /// «Reemplazar» con un archivo (o enlace): se copia a un temporal en la carpeta destino; recién
    /// cuando está entero se aparta lo que estaba, se pone el temporal en su lugar y lo apartado va
    /// a la Papelera. Si algo falla, lo que estaba vuelve.
    fn replace_one(&mut self, item: &Planned, dst: &Path, moving: bool, record: bool) -> bool {
        if !self.restore_still_safe(&item.src, dst) {
            return false;
        }
        let tmp = tmp_in(dst.parent().unwrap_or(Path::new("")));
        if let Err(e) = self.write(item, &tmp) {
            if e.kind != ErrKind::Exists {
                let _ = self.prims.remove(&tmp);
            }
            self.fail(&item.src, dst, e);
            return false;
        }
        if moving && !self.verify_move(item, &tmp, dst) {
            return false;
        }
        if self.halted() || self.ctl.is_cancelled() {
            let _ = self.prims.remove(&tmp);
            self.cancelled = true;
            return false;
        }
        let Some(aside) = self.set_aside(&item.src, dst) else {
            let _ = self.prims.remove(&tmp);
            return false;
        };
        if let Err(e) = self.rename_final(&tmp, dst) {
            self.put_back(&aside, dst);
            let _ = self.prims.remove(&tmp);
            self.fail(&item.src, dst, e);
            return false;
        }
        self.dispose_aside(&aside, dst);
        if moving && let Err(e) = self.prims.remove(&item.src) {
            self.fail(&item.src, dst, e);
            if record {
                self.record(Undo::Created(vec![dst.to_path_buf()]));
            }
            return false;
        }
        if record {
            self.record_done(&item.src, dst, moving);
        }
        true
    }

    /// Termina una carpeta: sus fechas y atributos, borra el origen vacío al mover y resuelve
    /// lo apartado por «Reemplazar» (a la Papelera si todo salió bien; si no, vuelve).
    fn close_dir(&mut self, frame: Frame, moving: bool) {
        // Devolviendo: la carpeta tiene que seguir siendo una carpeta de verdad del disco (pudo
        // cambiar por un enlace mientras se copiaba lo de adentro). Si no, no se le pone nada ni
        // se la renombra: se avisa.
        let safe = self.restore_dir_still_safe(&frame.src, &frame.dst);
        if frame.created && safe {
            let _ = self.prims.finish_dir(&frame.dst, &frame.src);
        }
        if moving {
            // Solo si quedó vacía: si algo no se movió, queda con ella.
            let _ = self.prims.remove(&frame.src);
        }
        let Some(aside) = frame.aside else { return };
        // Un error adentro, también del recorrido (anotado antes de abrir la carpeta), hace que
        // vuelva la carpeta vieja.
        let ok = self.errors.len() == frame.errors_before && !self.halted() && !self.walk_failed.iter().any(|p| p.starts_with(&frame.src));
        if ok {
            self.dispose_aside(&aside, &frame.dst);
            if frame.record_later {
                self.record_done(&frame.src, &frame.dst, moving);
            }
            return;
        }
        // Algo falló o se canceló: vuelve la carpeta que estaba.
        self.select.retain(|p| p != &frame.dst);
        if !safe {
            self.keep_visible(&aside, &frame.dst, "Lo que estaba antes quedó como");
            return;
        }
        if moving {
            // Lo que ya se movió no tiene otra copia: queda a la vista con otro nombre.
            let keep = self.free_numbered(&frame.dst, true);
            match self.prims.rename_noreplace(&frame.dst, &keep) {
                Ok(()) => {
                    self.fail_msg(&frame.src, &keep, &format!("Lo que se llegó a mover quedó en «{}»", name_of(&keep)), false);
                    self.record_done(&frame.src, &keep, true);
                }
                Err(_) => {
                    self.keep_visible(&aside, &frame.dst, "Lo que estaba antes quedó como");
                    return;
                }
            }
        } else {
            // Solo copias de este trabajo: se borran.
            self.remove_tree(&frame.dst);
        }
        self.put_back(&aside, &frame.dst);
    }

    /// Borra un árbol (un temporal del motor o lo copiado a medias), de adentro hacia afuera.
    fn remove_tree(&mut self, root: &Path) {
        let never = Control::default();
        match walk(self.prims, root, &never, &mut |_| {}) {
            Ok((items, errors)) => {
                for e in errors {
                    let p = e.path.clone();
                    self.fail(&p, &p, e);
                }
                for i in items.iter().rev() {
                    if let Err(e) = self.prims.remove(&i.src) {
                        self.fail(&i.src, &i.src, e);
                    }
                }
            }
            Err(e) => self.fail(root, root, e),
        }
    }

    // ---------- mover en el mismo disco ----------

    /// Renombra `src` a `dst` (mismo disco), con conflictos.
    fn rename_tree(&mut self, src: &Path, dst: &Path) -> Renamed {
        let incoming = match self.prims.stat(src) {
            Ok(s) => s,
            Err(e) => {
                self.fail(src, dst, e);
                return Renamed::Done(None);
            }
        };
        match self.decide(src, &incoming, dst, true) {
            Decision::Skip => Renamed::Done(None),
            Decision::Fresh(to) if !self.restore_still_safe(src, &to) => Renamed::Done(None),
            Decision::Fresh(to) => match self.prims.rename_noreplace(src, &to) {
                Ok(()) => {
                    self.record(Undo::Moved(vec![(to.clone(), src.to_path_buf())]));
                    Renamed::Done(Some(to))
                }
                Err(e) if e.kind == ErrKind::CrossDevice => Renamed::CrossDevice(Decision::Fresh(to)),
                Err(e) => {
                    self.fail(src, &to, e);
                    Renamed::Done(None)
                }
            },
            Decision::Merge(to) => {
                match self.prims.read_dir(src) {
                    Ok(mut names) => {
                        names.sort();
                        self.merging += 1;
                        for n in names {
                            if self.should_stop() {
                                break;
                            }
                            let (child, child_dst) = (src.join(&n), to.join(&n));
                            if let Renamed::CrossDevice(decision) = self.rename_tree(&child, &child_dst) {
                                self.copy_with(&child, decision, true);
                            }
                        }
                        self.merging -= 1;
                        let _ = self.prims.remove(src);
                    }
                    Err(e) => self.fail(src, &to, e),
                }
                Renamed::Done(Some(to))
            }
            Decision::Replace(to) => {
                let Some(aside) = self.set_aside(src, &to) else { return Renamed::Done(None) };
                if !self.restore_still_safe(src, &to) {
                    self.put_back(&aside, &to);
                    return Renamed::Done(None);
                }
                match self.prims.rename_noreplace(src, &to) {
                    Ok(()) => {
                        self.dispose_aside(&aside, &to);
                        self.record(Undo::Moved(vec![(to.clone(), src.to_path_buf())]));
                        Renamed::Done(Some(to))
                    }
                    Err(e) => {
                        self.put_back(&aside, &to);
                        if e.kind == ErrKind::CrossDevice {
                            // Otro disco: se reemplaza copiando (sin volver a preguntar).
                            return Renamed::CrossDevice(Decision::Replace(to));
                        }
                        self.fail(src, &to, e);
                        Renamed::Done(None)
                    }
                }
            }
        }
    }

    // ---------- carpeta nueva con la selección ----------

    fn new_folder_with(&mut self, parent: &Path, items: &[PathBuf]) {
        let name = folder_with_name(items.len());
        let folder = free_name(parent, &|n| numbered_name(&name, true, n), &|p| self.exists(p).is_some());
        self.start(title(&JobSpec::NewFolderWith { parent: parent.into(), items: vec![] }, items.len() as u64), items.len() as u64, 0);
        if let Err(e) = self.prims.create_dir(&folder) {
            self.fail(parent, &folder, e);
            return;
        }
        self.record(Undo::CreatedFolder(folder.clone()));
        let list: Vec<Item> = items
            .iter()
            .filter_map(|src| src.file_name().map(|n| (src, folder.join(n))))
            .map(|(src, dst)| self.item_to(src, dst, true))
            .collect();
        for (i, item) in list.into_iter().enumerate() {
            self.index = i;
            if self.should_stop() {
                break;
            }
            self.process(item, true);
        }
        self.select.clear();
        // Lo que de verdad quedó adentro (movido, o copiado entre discos aunque el original no se
        // haya podido borrar: eso tiene su deshacer como copia).
        let arrived = self.prims.read_dir(&folder).map_or(0, |v| v.len());
        if arrived == 0 {
            // No llegó nada: la carpeta vacía no queda.
            let _ = self.prims.remove(&folder);
            self.undo.clear();
            return;
        }
        // El nombre cuenta lo que de verdad quedó adentro (sin pisar nada).
        let mut folder = folder;
        if arrived != items.len() {
            let name = folder_with_name(arrived);
            let renamed = free_name(parent, &|n| numbered_name(&name, true, n), &|p| self.exists(p).is_some());
            if self.prims.rename_noreplace(&folder, &renamed).is_ok() {
                for u in &mut self.undo {
                    remap_undo(u, &folder, &renamed);
                }
                folder = renamed;
            }
        }
        self.focus = Some(folder);
        self.rename_after = true;
    }

    // ---------- devolver de la Papelera ----------

    fn restore(&mut self, items: &[PathBuf]) {
        self.moves_as_created = true;
        let mut list = Vec::new();
        for src in items {
            let Some(origin) = self.prims.trash_origin(src) else {
                self.fail_msg(src, src, NO_ORIGIN, false);
                continue;
            };
            list.push((src.clone(), origin));
        }
        self.start(title(&JobSpec::Restore { items: vec![] }, items.len() as u64), list.len() as u64, 0);
        self.ahead = list.iter().map(|(s, o)| self.ahead_of(s, o)).collect();
        for (i, (src, origin)) in list.into_iter().enumerate() {
            self.index = i;
            if self.should_stop() {
                break;
            }
            let Some(item) = self.restore_item(&src, &origin) else { continue };
            self.process(item, true);
        }
    }

    // ---------- comprimir ----------

    /// Un zip con `sources` en la carpeta del primero: se recorre (solo metadatos), se escribe con
    /// un nombre parcial y recién entero toma su nombre libre. Leer el contenido baja lo que está
    /// solo en la nube.
    fn compress(&mut self, sources: &[PathBuf]) {
        let t = title(&JobSpec::Compress { sources: vec![] }, sources.len() as u64);
        let Some(dir) = sources.first().and_then(|s| s.parent()).map(Path::to_path_buf) else { return };
        let names: Vec<String> = sources.iter().map(|s| name_of(s)).collect();
        let name = archive_name(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let hint = dir.join(&name);
        // (ruta, nombre adentro, tamaño) de cada elemento, en preorden.
        let mut items: Vec<(PathBuf, String)> = Vec::new();
        let mut sizes: Vec<u64> = Vec::new();
        let mut seen = 0u64;
        for src in sources {
            let mut on_seen = |n: u64| (self.out)(Update::Preparing { items: seen + n });
            match walk(self.prims, src, self.ctl, &mut on_seen) {
                Ok((plan, errors)) => {
                    seen += plan.len() as u64;
                    for e in errors {
                        let at = e.path.clone();
                        self.fail(&at, &hint, e);
                    }
                    for pl in plan {
                        let mut inside = pl.rel.iter().map(|c| c.to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                        if pl.stat.kind == ItemKind::Dir {
                            inside.push('/');
                        }
                        sizes.push(if pl.stat.kind == ItemKind::File { pl.stat.size } else { 0 });
                        items.push((pl.src, inside));
                    }
                }
                Err(e) => self.fail(src, &hint, e),
            }
            if self.halted() {
                return;
            }
        }
        if items.is_empty() {
            return;
        }
        let total: u64 = sizes.iter().sum();
        self.start(t, items.len() as u64, total);
        // Dónde termina cada elemento en los bytes leídos (para el nombre del actual).
        let ends: Vec<u64> = sizes.iter().scan(0u64, |acc, s| {
            *acc += s;
            Some(*acc)
        }).collect();
        let zip = free_name(&dir, &|n| numbered_name(&name, false, n), &|p| self.exists(p).is_some());
        let archives = self.archives;
        let mut part = part_name(&zip, &|p| self.exists(p).is_some());
        let mut written = Err(CopyError::new(ErrKind::Cancelled, &part));
        for attempt in 0..2 {
            if attempt == 1 {
                // El disco no acepta el nombre largo del parcial: uno corto del motor.
                part = short_part_name(&zip);
            }
            let mut on_bytes = |b: u64| {
                let i = ends.partition_point(|&e| e < b).min(items.len() - 1);
                self.current = name_of(&items[i].0);
                self.items_done = ends.partition_point(|&e| e <= b) as u64;
                self.progress(b, false);
                self.ctl.checkpoint()
            };
            written = archives.create(&part, &items, &mut on_bytes);
            if !matches!(&written, Err(e) if e.kind == ErrKind::NameTooLong) {
                break;
            }
        }
        if let Err(e) = written {
            if e.kind != ErrKind::Exists {
                let _ = self.prims.remove(&part);
            }
            // El error del parcial se muestra con el nombre del zip.
            let at = if e.path == part { zip.clone() } else { e.path.clone() };
            self.fail(&at, &zip, e);
            return;
        }
        if self.halted() || self.ctl.is_cancelled() {
            let _ = self.prims.remove(&part);
            self.cancelled = true;
            return;
        }
        // Si algo ocupó el nombre mientras se escribía, el siguiente libre.
        let mut zip = zip;
        let mut renamed = self.rename_final(&part, &zip);
        for _ in 0..3 {
            if !matches!(&renamed, Err(e) if e.kind == ErrKind::Exists) {
                break;
            }
            zip = free_name(&dir, &|n| numbered_name(&name, false, n), &|p| self.exists(p).is_some());
            renamed = self.rename_final(&part, &zip);
        }
        if let Err(e) = renamed {
            let _ = self.prims.remove(&part);
            self.fail(&zip, &zip, e);
            return;
        }
        self.items_done = items.len() as u64;
        self.bytes_done = total;
        self.ok_items = sources.len() as u64;
        self.record(Undo::Created(vec![zip.clone()]));
        self.select.push(zip);
    }

    // ---------- extraer ----------

    /// Un error de todo el archivo (no se puede leer, tiene contraseña…): se ofrece «Reintentar»
    /// salvo cuando sería lo mismo.
    fn fail_archive(&mut self, archive: &Path, dir: &Path, e: CopyError) {
        let again = !matches!(&e.kind, ErrKind::Other(m) if m == PASSWORD || m == UNSUPPORTED);
        let before = self.errors.len();
        self.fail(archive, dir, e);
        if let Some(err) = self.errors.get_mut(before) {
            err.retry = again;
        }
    }

    /// Extrae `archive`. Todo se escribe primero en una carpeta oculta del motor al lado del
    /// archivo (así lo que está a medias nunca se ve con su nombre y cancelar lo borra entero);
    /// después esa carpeta toma el nombre libre («fotos», «fotos 2») o, para «Extraer aquí», lo de
    /// adentro se mueve a la carpeta del archivo con los conflictos de siempre (renombrando: no se
    /// vuelve a escribir).
    fn extract(&mut self, archive: &Path, into_folder: bool) {
        let t = title(&JobSpec::Extract { archive: archive.to_path_buf(), into_folder }, 1);
        let Some(dir) = archive.parent().map(Path::to_path_buf) else { return };
        let archives = self.archives;
        let ctl = self.ctl;
        let entries = match archives.list(archive, &mut || ctl.checkpoint()) {
            Ok(v) => v,
            Err(e) if e.kind == ErrKind::Cancelled => {
                self.cancelled = true;
                return;
            }
            Err(e) => {
                self.fail_archive(archive, &dir, e);
                return;
            }
        };
        let declared = declared_total(&entries);
        let count = entries.iter().filter(|e| e.kind != EntryKind::Omitted).count() as u64;
        // Espacio: si no alcanza lo que declara, no empieza.
        if let Some(v) = self.prims.volume(&dir)
            && declared > v.free
        {
            let missing = sys::env().format_size_short(declared - v.free);
            self.stopped = Some(format!("Faltan {missing} en \"{}\"", v.label));
            return;
        }
        // Un archivo que se expande muchísimo (o a muchos GB) se pregunta antes.
        let packed = self.prims.stat(archive).map(|s| s.size).unwrap_or(0);
        if declared > BIG_EXPANSION || declared > packed.saturating_mul(BIG_RATIO) {
            let q = format!("Este archivo se expande a {}. ¿Seguir?", sys::env().format_size_short(declared));
            (self.out)(Update::Confirm(q));
            if !self.ctl.wait_confirm() {
                self.cancelled = true;
                return;
            }
        }
        self.start(t, count, declared);
        self.current = name_of(archive);
        let staging = tmp_in(&dir);
        if let Err(e) = self.prims.create_dir(&staging) {
            self.fail_archive(archive, &dir, e);
            return;
        }
        let mut failed: Vec<(String, CopyError)> = Vec::new();
        let mut written = 0u64;
        let res = {
            let mut on_bytes = |b: u64| {
                written = b;
                self.progress(b, false);
                self.ctl.checkpoint()
            };
            archives.extract_all(archive, &staging, &mut on_bytes, &mut |name, e| failed.push((name.to_string(), e)))
        };
        // Cada entrada que no se pudo extraer: con su nombre (sin reintento: sería lo mismo).
        for (name, e) in failed {
            let rel = name.trim_start_matches(['/', '\\']);
            let message = e.message();
            self.errors.push(super::JobError { src: archive.join(rel), dst: dir.join(rel), message, retry: false });
        }
        match res {
            Ok(n) => self.omitted = n,
            Err(e) => {
                self.remove_tree(&staging);
                if e.kind == ErrKind::Cancelled {
                    self.cancelled = true;
                } else {
                    self.fail_archive(archive, &dir, e);
                }
                return;
            }
        }
        if self.ctl.is_cancelled() {
            self.remove_tree(&staging);
            self.cancelled = true;
            return;
        }
        self.items_done = count;
        self.bytes_done = written;
        self.progress(0, true);
        if into_folder {
            self.extract_into_folder(archive, &dir, &staging, count);
        } else {
            self.extract_here(&dir, &staging);
        }
        self.items_done = count;
    }

    /// «Extraer en "fotos"»: la carpeta del motor toma el primer nombre libre.
    fn extract_into_folder(&mut self, archive: &Path, dir: &Path, staging: &Path, count: u64) {
        let base = extract_folder_name(&name_of(archive));
        let mut renamed = Err(CopyError::new(ErrKind::Exists, staging));
        let mut folder = PathBuf::new();
        for _ in 0..5 {
            folder = free_name(dir, &|n| numbered_name(&base, true, n), &|p| self.exists(p).is_some());
            renamed = self.rename_final(staging, &folder);
            if !matches!(&renamed, Err(e) if e.kind == ErrKind::Exists) {
                break;
            }
        }
        if let Err(e) = renamed {
            self.remove_tree(staging);
            self.fail_archive(archive, dir, e);
            return;
        }
        self.ok_items = count;
        self.record(Undo::Created(vec![folder.clone()]));
        self.select.push(folder);
    }

    /// «Extraer aquí»: lo de adentro de la carpeta del motor va a `dir` renombrando, con los
    /// conflictos de siempre (Reemplazar, Conservar ambos, Omitir, Combinar). Lo que queda en la
    /// carpeta del motor (lo omitido) se borra. Si se cancela, lo que ya llegó se saca y lo
    /// reemplazado vuelve.
    fn extract_here(&mut self, dir: &Path, staging: &Path) {
        let mut names = self.prims.read_dir(staging).unwrap_or_default();
        names.sort();
        self.moves_as_created = true;
        let items: Vec<Item> = names.iter().map(|n| Item::new(&staging.join(n), Target::Fixed(dir.join(n)), How::Rename)).collect();
        self.ahead = items.iter().map(|i| self.ahead_of(&i.src, &self.target_hint(i))).collect();
        for (i, item) in items.into_iter().enumerate() {
            self.index = i;
            if self.should_stop() {
                break;
            }
            self.process(item, true);
        }
        if self.cancelled {
            self.take_back();
        }
        self.remove_tree(staging);
    }

    /// Cancelar «Extraer aquí» a mitad: lo que llegó (solo lo de este trabajo) se borra y lo
    /// reemplazado vuelve de la Papelera, en orden inverso.
    fn take_back(&mut self) {
        for step in std::mem::take(&mut self.undo).into_iter().rev() {
            match step {
                Undo::Created(paths) => paths.iter().rev().for_each(|p| self.remove_tree(p)),
                Undo::CreatedIds(items) => items.iter().rev().for_each(|(p, _)| self.remove_tree(p)),
                Undo::Recycled(pairs) => {
                    for (name, original) in pairs.into_iter().rev() {
                        if let Err(e) = self.prims.untrash(&name, &original) {
                            self.fail(&original, &original, e);
                        }
                    }
                }
                _ => {}
            }
        }
        self.select.clear();
        self.ok_items = 0;
    }

    // ---------- deshacer ----------

    fn undo(&mut self, undo: Undo) {
        if self.should_stop() {
            return;
        }
        match undo {
            Undo::Batch(steps) => {
                for step in steps.into_iter().rev() {
                    self.undo(step);
                }
            }
            Undo::Moved(pairs) => {
                for (now, before) in pairs {
                    if self.halted() {
                        break;
                    }
                    self.current = name_of(&now);
                    let item = self.item_to(&now, before, true);
                    self.process(item, true);
                }
            }
            Undo::Created(paths) => {
                for p in paths {
                    self.current = name_of(&p);
                    // Por ruta: si una carpeta del camino pasó a ser un enlace, no se toca lo que
                    // está del otro lado.
                    if let Err(e) = self.prims.check_parent(p.parent().unwrap_or(Path::new(""))) {
                        self.fail_msg(&p, &p, &e.message(), false);
                    } else if self.exists(&p).is_some()
                        && let Err(e) = self.prims.trash(&p)
                    {
                        self.fail(&p, &p, e);
                    }
                    self.items_done += 1;
                    self.progress(0, false);
                }
            }
            Undo::CreatedIds(items) => {
                for (p, id) in items {
                    self.current = name_of(&p);
                    match self.prims.stat(&p) {
                        // Ya no está: nada que hacer.
                        Err(e) if e.kind == ErrKind::NotFound => {}
                        Ok(s) if s.file_id == Some(id) => {
                            if let Err(e) = self.prims.trash(&p) {
                                self.fail(&p, &p, e);
                            }
                        }
                        Ok(_) => self.fail_msg(&p, &p, NOT_THE_SAME, false),
                        Err(e) => self.fail(&p, &p, e),
                    }
                    self.items_done += 1;
                    self.progress(0, false);
                }
            }
            Undo::CreatedFolder(dir) => {
                if self.prims.read_dir(&dir).is_ok_and(|n| n.is_empty())
                    && let Err(e) = self.prims.trash(&dir)
                {
                    self.fail(&dir, &dir, e);
                }
                self.items_done += 1;
            }
            Undo::Recycled(pairs) => {
                for (name, original) in pairs {
                    self.current = name_of(&original);
                    if let Err(e) = self.prims.untrash(&name, &original) {
                        self.fail(&original, &original, e);
                    }
                    self.items_done += 1;
                    self.progress(0, false);
                }
            }
            Undo::Rename { path, old_name } => {
                let to = path.with_file_name(&old_name);
                if let Err(e) = self.prims.rename_noreplace(&path, &to) {
                    self.fail(&path, &to, e);
                }
                self.items_done += 1;
            }
            // Renombrar varios se deshace en el núcleo (`App::undo_renames`, con su cruce seguro) y
            // no llega acá; si llegara, uno por uno sin pisar nada (un intercambio queda en error).
            Undo::Renames(pairs) => {
                for (path, old_name) in pairs {
                    if self.halted() {
                        break;
                    }
                    self.current = name_of(&path);
                    let to = path.with_file_name(&old_name);
                    if let Err(e) = self.prims.rename_noreplace(&path, &to) {
                        self.fail(&path, &to, e);
                    }
                    self.items_done += 1;
                }
            }
        }
    }
}

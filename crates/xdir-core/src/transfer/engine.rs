//! El motor: corre un `JobSpec` con las primitivas del sistema y manda `Update`s.
//!
//! Reglas que cuidan los datos:
//! - nada se pisa sin preguntar; «Reemplazar» un archivo copia a un temporal oculto en la carpeta
//!   destino y recién al terminar bien lo pone en lugar del otro (atómico); lo reemplazado va a la
//!   Papelera si el disco tiene (si no, lo borra el reemplazo);
//! - mover entre discos es copiar, verificar el tamaño y recién ahí borrar el original, archivo
//!   por archivo; las carpetas de origen se borran solo si quedaron vacías;
//! - mover dentro del mismo disco es renombrar (no lee contenido);
//! - lo a medio copiar lo borra la primitiva; el temporal de un reemplazo lo borra el motor;
//! - cada error se anota y se sigue, salvo «El disco está lleno», que detiene el trabajo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::naming::{copy_name, folder_with_name, free_name, numbered_name};
use super::walk::{Planned, totals, walk};
use super::{Answer, Choice, ConflictInfo, Control, Flow, JobError, JobOutcome, JobSpec, Update};
use crate::format::thousands;
use crate::ops::{OpResult, Undo};
use crate::sys::{self, CopyError, CopyPrims, ErrKind, ItemKind, ItemStat};

/// Como mucho un aviso de progreso cada tanto por trabajo.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

const INTO_ITSELF: &str = "No se puede copiar una carpeta dentro de sí misma";
const NOT_SAME: &str = "La copia no quedó igual";
const NO_ORIGIN: &str = "No se sabe de dónde vino";
const REPLACE_PARENT: &str = "No se puede reemplazar una carpeta que contiene lo que se copia";

/// Título del trabajo para la ventana de copia; `items`: cuántos elementos de arriba.
pub fn title(spec: &JobSpec, items: u64) -> String {
    let name = |p: &Path| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
    let folder = |p: &Path| p.parent().map(name).unwrap_or_default();
    let count = |verb: &str| if items == 1 { format!("{verb} 1 elemento") } else { format!("{verb} {} elementos", thousands(items)) };
    let between = |verb: &str, first: Option<&PathBuf>, dest: &Path| match first {
        Some(one) if items == 1 => format!("{verb} «{}» a \"{}\"", name(one), name(dest)),
        _ => format!("{} de \"{}\" a \"{}\"", count(verb), first.map(|f| folder(f)).unwrap_or_default(), name(dest)),
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
    }
}

/// Corre el trabajo hasta el final (o hasta que se cancele) y manda `Update::Finished`.
pub fn run(spec: JobSpec, prims: &dyn CopyPrims, ctl: &Control, out: &mut dyn FnMut(Update)) {
    let mut job = Job::new(prims, ctl, out);
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
            let items = pairs.iter().map(|(src, dst)| job.item_to(src, dst.clone(), moving)).collect();
            job.run_items(t, items, moving);
        }
        JobSpec::NewFolderWith { parent, items } => job.new_folder_with(&parent, &items),
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
        Undo::Moved(v) => v.len() as u64,
        Undo::Recycled(v) => v.len() as u64,
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
}

impl Item {
    fn new(src: &Path, target: Target, how: How) -> Item {
        Item { src: src.to_path_buf(), target, how }
    }
}

/// Lo que se decidió para un destino.
enum Decision {
    Skip,
    /// Escribir en esa ruta (libre).
    Fresh(PathBuf),
    /// Entrar en la carpeta que ya está.
    Merge(PathBuf),
    /// Reemplazar el archivo que ya está (por un temporal).
    ReplaceFile(PathBuf),
    /// Escribir en esa ruta, que se liberó apartando lo que había (a un temporal oculto, en
    /// discos sin Papelera): al terminar se borra si salió todo bien o se devuelve.
    FreshAside(PathBuf, PathBuf),
}

/// Una carpeta abierta mientras se copia su contenido.
struct Frame {
    rel: PathBuf,
    src: PathBuf,
    dst: PathBuf,
    created: bool,
    errors_before: usize,
    aside: Option<PathBuf>,
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Nombre oculto para un temporal del motor en `dir`.
fn tmp_in(dir: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".xdir-{}-{n}.tmp", std::process::id()))
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

struct Job<'a> {
    prims: &'a dyn CopyPrims,
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
    /// Conflictos de arriba que se ven por delante (por índice de elemento).
    ahead: Vec<bool>,
    index: usize,
    /// «Devolver»: lo movido se deshace mandándolo de nuevo a la Papelera.
    moves_as_created: bool,
}

impl<'a> Job<'a> {
    fn new(prims: &'a dyn CopyPrims, ctl: &'a Control, out: &'a mut dyn FnMut(Update)) -> Job<'a> {
        Job {
            prims,
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
            ahead: Vec::new(),
            index: 0,
            moves_as_created: false,
        }
    }

    fn halted(&self) -> bool {
        self.cancelled || self.stopped.is_some()
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
        let outcome = JobOutcome { result, errors: std::mem::take(&mut self.errors), stopped: self.stopped.take(), cancelled: self.cancelled };
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
                self.errors.push(JobError { src: src.to_path_buf(), dst: dst.to_path_buf(), message: e.message() });
            }
            _ => self.errors.push(JobError { src: src.to_path_buf(), dst: dst.to_path_buf(), message: e.message() }),
        }
    }

    fn fail_msg(&mut self, src: &Path, dst: &Path, message: &str) {
        self.errors.push(JobError { src: src.to_path_buf(), dst: dst.to_path_buf(), message: message.into() });
    }

    /// Anota un paso de deshacer, juntándolo con el anterior si es del mismo tipo.
    fn record(&mut self, step: Undo) {
        let step = match step {
            Undo::Moved(pairs) if self.moves_as_created => Undo::Created(pairs.into_iter().map(|(now, _)| now).collect()),
            other => other,
        };
        match (self.undo.last_mut(), step) {
            (Some(Undo::Created(a)), Undo::Created(b)) => a.extend(b),
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

    fn same_volume(&self, a: &Path, b: &Path) -> bool {
        match (self.prims.volume(a), self.prims.volume(b)) {
            (Some(x), Some(y)) => x.id == y.id,
            _ => false,
        }
    }

    fn case_insensitive(&self, p: &Path) -> bool {
        self.prims.volume(p).is_some_and(|v| v.case_insensitive)
    }

    /// ¿Es la misma ruta? (sin distinguir mayúsculas donde el disco no las distingue).
    fn same_path(&self, a: &Path, b: &Path) -> bool {
        a == b || (self.case_insensitive(a) && a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase())
    }

    /// ¿`inner` está dentro de `outer` (o es la misma)?
    fn inside(&self, inner: &Path, outer: &Path) -> bool {
        if inner.starts_with(outer) {
            return true;
        }
        if !self.case_insensitive(outer) {
            return false;
        }
        let (i, o) = (inner.to_string_lossy().to_lowercase(), outer.to_string_lossy().to_lowercase());
        Path::new(&i).starts_with(Path::new(&o))
    }

    fn has_trash(&self, p: &Path) -> bool {
        self.prims.volume(p).is_some_and(|v| v.has_trash)
    }

    fn free_numbered(&self, dst: &Path, is_dir: bool) -> PathBuf {
        let name = name_of(dst);
        let parent = dst.parent().unwrap_or(Path::new(""));
        free_name(parent, &|n| numbered_name(&name, is_dir, n.max(2)), &|p| self.exists(p).is_some())
    }

    // ---------- armar los trabajos ----------

    /// Copiar o mover `sources` a `dest`: lo que va, con las comprobaciones de entrada.
    fn plan_into(&mut self, sources: &[PathBuf], dest: &Path, moving: bool) -> Vec<Item> {
        let mut items = Vec::new();
        for src in sources {
            let Some(name) = src.file_name() else { continue };
            if self.inside(dest, src) {
                self.fail_msg(src, &dest.join(name), INTO_ITSELF);
                continue;
            }
            let same_folder = src.parent().is_some_and(|p| self.same_path(p, dest));
            if same_folder {
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
        let rename = moving && dst.parent().is_some_and(|parent| self.same_volume(src, parent));
        Item::new(src, Target::Fixed(dst), if rename { How::Rename } else { How::Walk(None) })
    }

    /// Recorre, cuenta, comprueba espacio y tamaños, y lleva cada elemento.
    fn run_items(&mut self, title: String, mut items: Vec<Item>, moving: bool) {
        // Recorrido con metadatos.
        let mut seen_before = 0u64;
        let mut keep = Vec::with_capacity(items.len());
        for mut item in items.drain(..) {
            if let How::Walk(plan @ None) = &mut item.how {
                let mut on_seen = |n: u64| (self.out)(Update::Preparing { items: seen_before + n });
                match walk(self.prims, &item.src, self.ctl, &mut on_seen) {
                    Ok(p) => {
                        seen_before += p.len() as u64;
                        *plan = Some(p);
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
        let mut needed: HashMap<std::ffi::OsString, (u64, String, u64, PathBuf)> = HashMap::new();
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
                        let e = needed.entry(v.id.clone()).or_insert((0, v.label.clone(), v.free, dir.clone()));
                        e.0 += bytes;
                    }
                }
                How::Walk(None) => {}
            }
        }

        // Espacio: si no alcanza, no empieza.
        if let Some((bytes, label, free, _)) = needed.values().find(|(bytes, _, free, _)| bytes > free) {
            let missing = sys::env().format_size_short(bytes - free);
            self.stopped = Some(format!("Faltan {missing} en \"{label}\""));
            return;
        }

        self.start(title, total_items, total_bytes);
        self.ahead = items
            .iter()
            .map(|i| matches!(&i.target, Target::Fixed(dst) if !self.same_path(&i.src, dst) && self.exists(dst).is_some()))
            .collect();
        for (i, item) in items.into_iter().enumerate() {
            self.index = i;
            if self.halted() || self.ctl.checkpoint() == Flow::Stop {
                self.cancelled |= self.ctl.is_cancelled();
                break;
            }
            self.process(item, moving);
        }
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
        self.current = name_of(&item.src);
        let hint = self.target_hint(&item);
        match item.how {
            How::Rename => {
                let Target::Fixed(dst) = item.target else { return };
                if let Some(done) = self.rename_tree(&item.src, &dst) {
                    self.select.push(done);
                }
                self.items_done += 1;
                self.progress(0, false);
            }
            How::Walk(plan) => {
                let plan = match plan {
                    Some(p) => p,
                    None => match walk(self.prims, &item.src, self.ctl, &mut |_| {}) {
                        Ok(p) => p,
                        Err(e) => {
                            self.fail(&item.src, &hint, e);
                            return;
                        }
                    },
                };
                if plan.is_empty() {
                    return;
                }
                let first = match &item.target {
                    Target::Fixed(dst) => Some(dst.clone()),
                    Target::Duplicate(dir) => {
                        let (name, is_dir) = (name_of(&item.src), plan[0].stat.kind == ItemKind::Dir);
                        let dst = free_name(dir, &|n| copy_name(&name, is_dir, n), &|p| self.exists(p).is_some());
                        // Elegido libre: nunca es conflicto.
                        self.copy_tree(plan, Decision::Fresh(dst), moving);
                        return;
                    }
                };
                if let Some(dst) = first {
                    let decision = self.decide(&plan[0].src, &plan[0].stat, &dst, moving);
                    self.copy_tree(plan, decision, moving);
                }
            }
        }
    }

    // ---------- conflictos ----------

    /// Decide qué hacer con `dst` (si existe, pregunta o usa «Hacer esto con todos»).
    fn decide(&mut self, src: &Path, incoming: &ItemStat, dst: &Path, moving: bool) -> Decision {
        let Some(existing) = self.exists(dst) else { return Decision::Fresh(dst.to_path_buf()) };
        if self.same_path(src, dst) {
            // El mismo elemento: mover no hace nada; copiar sería pisarlo.
            if !moving {
                self.fail_msg(src, dst, "No se puede copiar un elemento sobre sí mismo");
            }
            return Decision::Skip;
        }
        let is_dir = incoming.kind == ItemKind::Dir;
        let can_merge = is_dir && existing.kind == ItemKind::Dir;
        let stored = if is_dir { self.dir_all } else { self.file_all };
        let choice = match stored {
            Some(Choice::Merge) if !can_merge => self.ask(src, dst, incoming, existing, is_dir, can_merge),
            Some(c) => Some(c),
            None => self.ask(src, dst, incoming, existing, is_dir, can_merge),
        };
        let Some(choice) = choice else { return Decision::Skip };
        match choice {
            Choice::Skip => Decision::Skip,
            Choice::Merge if can_merge => Decision::Merge(dst.to_path_buf()),
            Choice::KeepBoth | Choice::Merge => Decision::Fresh(self.free_numbered(dst, is_dir)),
            Choice::Replace => {
                if self.inside(src, dst) {
                    self.fail_msg(src, dst, REPLACE_PARENT);
                    return Decision::Skip;
                }
                if !is_dir && existing.kind != ItemKind::Dir {
                    return Decision::ReplaceFile(dst.to_path_buf());
                }
                // Hay una carpeta en juego: lo que está se aparta entero.
                if self.has_trash(dst) {
                    match self.prims.trash(dst) {
                        Ok(name) => {
                            self.record(Undo::Recycled(vec![(name, dst.to_path_buf())]));
                            Decision::Fresh(dst.to_path_buf())
                        }
                        Err(e) => {
                            self.fail(src, dst, e);
                            Decision::Skip
                        }
                    }
                } else {
                    let aside = tmp_in(dst.parent().unwrap_or(Path::new("")));
                    match self.prims.rename_noreplace(dst, &aside) {
                        Ok(()) => Decision::FreshAside(dst.to_path_buf(), aside),
                        Err(e) => {
                            self.fail(src, dst, e);
                            Decision::Skip
                        }
                    }
                }
            }
        }
    }

    fn ask(&mut self, src: &Path, dst: &Path, incoming: &ItemStat, existing: ItemStat, is_dir: bool, can_merge: bool) -> Option<Choice> {
        let remaining = self.ahead.iter().skip(self.index + 1).filter(|c| **c).count();
        let folder = dst.parent().map(name_of).unwrap_or_default();
        let info = ConflictInfo { name: name_of(src), folder, is_dir, can_merge, incoming: *incoming, existing, remaining };
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
            if self.halted() || self.ctl.checkpoint() == Flow::Stop {
                self.cancelled |= self.ctl.is_cancelled();
                // Lo que queda no se cuenta como hecho.
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
                            let d = if *created { Decision::Fresh(dst) } else { self.decide(&item.src, &item.stat, &dst, moving) };
                            (d, *created)
                        }
                        _ => (Decision::Skip, false),
                    }
                }
            };
            let record = !parent_created;
            let size = if item.stat.kind == ItemKind::File { item.stat.size } else { 0 };
            match decision {
                Decision::Skip => {
                    if item.stat.kind == ItemKind::Dir {
                        dests.insert(item.rel.clone(), None);
                    }
                }
                Decision::Merge(dst) => {
                    dests.insert(item.rel.clone(), Some((dst.clone(), false)));
                    if is_root {
                        self.select.push(dst.clone());
                    }
                    let errors_before = self.errors.len();
                    stack.push(Frame { rel: item.rel.clone(), src: item.src.clone(), dst, created: false, errors_before, aside: None });
                }
                Decision::Fresh(dst) if item.stat.kind == ItemKind::Dir => {
                    self.open_dir(&item, dst, None, moving, record, is_root, &mut dests, &mut stack);
                }
                Decision::FreshAside(dst, aside) if item.stat.kind == ItemKind::Dir => {
                    // Lo apartado se resuelve al cerrar la carpeta.
                    self.open_dir(&item, dst, Some(aside), moving, record, is_root, &mut dests, &mut stack);
                }
                Decision::Fresh(dst) => {
                    if self.copy_one(&item, &dst, moving, record) && is_root {
                        self.select.push(dst);
                    }
                }
                Decision::FreshAside(dst, aside) => {
                    // Un archivo en lugar de una carpeta, en un disco sin Papelera.
                    let ok = self.copy_one(&item, &dst, moving, record);
                    if ok && is_root {
                        self.select.push(dst.clone());
                    }
                    self.settle_aside(&dst, &aside, ok);
                }
                Decision::ReplaceFile(dst) => {
                    if self.replace_one(&item, &dst, moving, record) && is_root {
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

    /// Crea la carpeta `dst` (libre) para `item` y la abre para su contenido.
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
        match self.prims.create_dir(&dst) {
            Ok(()) => {
                if record {
                    self.record_done(&item.src, &dst, moving);
                }
                if is_root {
                    self.select.push(dst.clone());
                }
                dests.insert(item.rel.clone(), Some((dst.clone(), true)));
                let errors_before = self.errors.len();
                stack.push(Frame { rel: item.rel.clone(), src: item.src.clone(), dst, created: true, errors_before, aside });
            }
            Err(e) => {
                self.fail(&item.src, &dst, e);
                dests.insert(item.rel.clone(), None);
                if let Some(aside) = aside {
                    self.settle_aside(&dst, &aside, false);
                }
            }
        }
    }

    /// Copia (o mueve) un archivo o un enlace a `dst`, que está libre. `record`: anota el
    /// deshacer (no hace falta dentro de una carpeta creada por este trabajo).
    fn copy_one(&mut self, item: &Planned, dst: &Path, moving: bool, record: bool) -> bool {
        if let Err(e) = self.write(item, dst) {
            self.fail(&item.src, dst, e);
            return false;
        }
        if moving && !self.verified_remove(item, dst) {
            return false;
        }
        if record {
            self.record_done(&item.src, dst, moving);
        }
        true
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

    /// Al mover entre discos: comprueba que la copia en `dst` quedó igual y borra el original.
    /// Si no quedó igual, borra la copia y el original queda.
    fn verified_remove(&mut self, item: &Planned, dst: &Path) -> bool {
        if item.stat.kind == ItemKind::File {
            let copied = self.prims.stat(dst).map(|s| s.size).ok();
            let original = self.prims.stat(&item.src).map(|s| s.size).ok();
            if copied.is_none() || copied != original {
                let _ = self.prims.remove(dst);
                self.fail_msg(&item.src, dst, NOT_SAME);
                return false;
            }
        }
        if let Err(e) = self.prims.remove(&item.src) {
            // Quedó copiado y el original sigue: se anota como copia (deshacer la borra).
            self.fail(&item.src, dst, e);
            self.record(Undo::Created(vec![dst.to_path_buf()]));
            return false;
        }
        true
    }

    /// «Reemplazar» un archivo: copia a un temporal en la carpeta destino y recién al terminar
    /// bien pone el temporal en lugar del que estaba (que va a la Papelera si el disco tiene).
    fn replace_one(&mut self, item: &Planned, dst: &Path, moving: bool, record: bool) -> bool {
        let tmp = tmp_in(dst.parent().unwrap_or(Path::new("")));
        if let Err(e) = self.write(item, &tmp) {
            if e.kind != ErrKind::Exists {
                let _ = self.prims.remove(&tmp);
            }
            self.fail(&item.src, dst, e);
            return false;
        }
        if moving && item.stat.kind == ItemKind::File {
            let copied = self.prims.stat(&tmp).map(|s| s.size).ok();
            let original = self.prims.stat(&item.src).map(|s| s.size).ok();
            if copied.is_none() || copied != original {
                let _ = self.prims.remove(&tmp);
                self.fail_msg(&item.src, dst, NOT_SAME);
                return false;
            }
        }
        if self.halted() || self.ctl.is_cancelled() {
            let _ = self.prims.remove(&tmp);
            self.cancelled = true;
            return false;
        }
        if self.has_trash(dst) {
            let name = match self.prims.trash(dst) {
                Ok(name) => name,
                Err(e) => {
                    let _ = self.prims.remove(&tmp);
                    self.fail(&item.src, dst, e);
                    return false;
                }
            };
            if let Err(e) = self.prims.replace(&tmp, dst) {
                // Lo de antes vuelve a su lugar; si no se puede, queda en la Papelera (y en el
                // deshacer).
                if self.prims.untrash(&name, dst).is_err() {
                    self.record(Undo::Recycled(vec![(name, dst.to_path_buf())]));
                }
                let _ = self.prims.remove(&tmp);
                self.fail(&item.src, dst, e);
                return false;
            }
            self.record(Undo::Recycled(vec![(name, dst.to_path_buf())]));
        } else if let Err(e) = self.prims.replace(&tmp, dst) {
            let _ = self.prims.remove(&tmp);
            self.fail(&item.src, dst, e);
            return false;
        }
        if moving && let Err(e) = self.prims.remove(&item.src) {
            self.fail(&item.src, dst, e);
            self.record(Undo::Created(vec![dst.to_path_buf()]));
            return false;
        }
        if record {
            self.record_done(&item.src, dst, moving);
        }
        true
    }

    /// Termina una carpeta: sus fechas y atributos, borra el origen vacío al mover y resuelve
    /// lo apartado.
    fn close_dir(&mut self, frame: Frame, moving: bool) {
        if frame.created {
            let _ = self.prims.finish_dir(&frame.dst, &frame.src);
        }
        if moving {
            // Solo si quedó vacía: si algo no se movió, queda con ella.
            let _ = self.prims.remove(&frame.src);
        }
        if let Some(aside) = &frame.aside {
            let ok = self.errors.len() == frame.errors_before && !self.halted();
            self.settle_aside(&frame.dst, aside, ok);
        }
    }

    /// Lo apartado (disco sin Papelera): si lo nuevo quedó bien, se borra; si no, vuelve a su
    /// lugar o, si ya hay algo ahí, queda con un nombre libre.
    fn settle_aside(&mut self, dst: &Path, aside: &Path, ok: bool) {
        if ok {
            self.remove_tree(aside);
            return;
        }
        if self.exists(dst).is_none() && self.prims.rename_noreplace(aside, dst).is_ok() {
            return;
        }
        let is_dir = self.exists(aside).is_some_and(|s| s.kind == ItemKind::Dir);
        let keep = self.free_numbered(dst, is_dir);
        match self.prims.rename_noreplace(aside, &keep) {
            Ok(()) => self.fail_msg(dst, &keep, &format!("Lo que estaba antes quedó como «{}»", name_of(&keep))),
            Err(_) => self.fail_msg(dst, aside, &format!("Lo que estaba antes quedó como «{}»", name_of(aside))),
        }
    }

    /// Borra un árbol (un temporal del motor), de adentro hacia afuera.
    fn remove_tree(&mut self, root: &Path) {
        let never = Control::default();
        match walk(self.prims, root, &never, &mut |_| {}) {
            Ok(items) => {
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

    /// Renombra `src` a `dst` (mismo disco), con conflictos. Devuelve dónde quedó.
    fn rename_tree(&mut self, src: &Path, dst: &Path) -> Option<PathBuf> {
        if self.same_path(src, dst) {
            return None;
        }
        let incoming = match self.prims.stat(src) {
            Ok(s) => s,
            Err(e) => {
                self.fail(src, dst, e);
                return None;
            }
        };
        match self.decide(src, &incoming, dst, true) {
            Decision::Skip => None,
            Decision::Fresh(to) => self.rename_one(src, &to),
            Decision::Merge(to) => {
                match self.prims.read_dir(src) {
                    Ok(mut names) => {
                        names.sort();
                        for n in names {
                            if self.halted() {
                                break;
                            }
                            self.rename_tree(&src.join(&n), &to.join(&n));
                        }
                        let _ = self.prims.remove(src);
                    }
                    Err(e) => self.fail(src, &to, e),
                }
                Some(to)
            }
            Decision::ReplaceFile(to) => {
                if self.has_trash(&to) {
                    let name = match self.prims.trash(&to) {
                        Ok(name) => name,
                        Err(e) => {
                            self.fail(src, &to, e);
                            return None;
                        }
                    };
                    match self.prims.rename_noreplace(src, &to) {
                        Ok(()) => {
                            self.record(Undo::Recycled(vec![(name, to.clone())]));
                            self.record(Undo::Moved(vec![(to.clone(), src.to_path_buf())]));
                            Some(to)
                        }
                        Err(e) => {
                            if self.prims.untrash(&name, &to).is_err() {
                                self.record(Undo::Recycled(vec![(name, to.clone())]));
                            }
                            self.fail(src, &to, e);
                            None
                        }
                    }
                } else {
                    match self.prims.replace(src, &to) {
                        Ok(()) => {
                            self.record(Undo::Moved(vec![(to.clone(), src.to_path_buf())]));
                            Some(to)
                        }
                        Err(e) => {
                            self.fail(src, &to, e);
                            None
                        }
                    }
                }
            }
            Decision::FreshAside(to, aside) => {
                let done = self.rename_one(src, &to);
                self.settle_aside(&to, &aside, done.is_some());
                done
            }
        }
    }

    fn rename_one(&mut self, src: &Path, to: &Path) -> Option<PathBuf> {
        match self.prims.rename_noreplace(src, to) {
            Ok(()) => {
                self.record(Undo::Moved(vec![(to.to_path_buf(), src.to_path_buf())]));
                Some(to.to_path_buf())
            }
            Err(e) => {
                self.fail(src, to, e);
                None
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
            if self.halted() || self.ctl.checkpoint() == Flow::Stop {
                self.cancelled |= self.ctl.is_cancelled();
                break;
            }
            self.process(item, true);
        }
        let moved = self.undo.iter().any(|u| matches!(u, Undo::Moved(_)));
        self.select.clear();
        if moved {
            self.focus = Some(folder);
            self.rename_after = true;
        } else {
            // No se movió nada: la carpeta vacía no queda.
            let _ = self.prims.remove(&folder);
            self.undo.clear();
        }
    }

    // ---------- devolver de la Papelera ----------

    fn restore(&mut self, items: &[PathBuf]) {
        self.moves_as_created = true;
        let mut list = Vec::new();
        for src in items {
            let Some(origin) = self.prims.trash_origin(src) else {
                self.fail_msg(src, src, NO_ORIGIN);
                continue;
            };
            list.push((src.clone(), origin));
        }
        self.start(title(&JobSpec::Restore { items: vec![] }, items.len() as u64), list.len() as u64, 0);
        self.ahead = list.iter().map(|(_, o)| self.exists(o).is_some()).collect();
        for (i, (src, origin)) in list.into_iter().enumerate() {
            self.index = i;
            if self.halted() || self.ctl.checkpoint() == Flow::Stop {
                self.cancelled |= self.ctl.is_cancelled();
                break;
            }
            if let Some(dir) = origin.parent()
                && let Err(e) = self.make_dirs(dir)
            {
                self.fail(&src, &origin, e);
                continue;
            }
            let item = self.item_to(&src, origin, true);
            self.process(item, true);
        }
    }

    /// Crea `dir` y las carpetas de arriba que falten.
    fn make_dirs(&self, dir: &Path) -> Result<(), CopyError> {
        let mut missing = Vec::new();
        let mut cur = Some(dir);
        while let Some(d) = cur {
            if d.as_os_str().is_empty() || self.exists(d).is_some() {
                break;
            }
            missing.push(d.to_path_buf());
            cur = d.parent();
        }
        for d in missing.iter().rev() {
            match self.prims.create_dir(d) {
                Ok(()) => {}
                Err(e) if e.kind == ErrKind::Exists => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    // ---------- deshacer ----------

    fn undo(&mut self, undo: Undo) {
        if self.halted() || self.ctl.checkpoint() == Flow::Stop {
            self.cancelled |= self.ctl.is_cancelled();
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
                    self.items_done += 1;
                }
            }
            Undo::Created(paths) => {
                for p in paths {
                    self.current = name_of(&p);
                    if self.exists(&p).is_some()
                        && let Err(e) = self.prims.trash(&p)
                    {
                        self.fail(&p, &p, e);
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
        }
    }
}

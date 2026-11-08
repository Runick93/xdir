//! Escáner neutro de xtree: cola de carpetas, hilos de trabajo y armado del árbol.
//! El listado de cada carpeta (`Lister`) y el escaneo rápido los pone cada sistema (`fs`).

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

use super::tree::{NO_PARENT, Node, Tree, sort_children};
use crate::fs;
use crate::platform::{Handle, WM_APP};

/// Avance del escaneo (`wparam` = id del escaneo, `lparam` = 1 si terminó, 0 si es solo avance).
pub const WM_APP_XTREE: u32 = WM_APP + 22;

/// Archivos que se guardan como nodos propios por carpeta; el resto se suma en un solo nodo.
const MAX_FILES_PER_DIR: usize = 1500;
const ID_SHARDS: usize = 64;

pub struct FileEntry {
    pub name: Box<str>,
    pub size: u64,
    /// Identificador del archivo; solo si puede tener enlaces duros (se cuenta una sola vez).
    pub id: Option<u64>,
}

#[derive(Default)]
pub struct DirListing {
    pub dirs: Vec<OsString>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListError {
    Denied,
    Other,
}

/// Los listadores pueden devolver archivos de tamaño 0: el núcleo los descarta.
pub trait Lister: Sync {
    fn list(&self, path: &Path) -> Result<DirListing, ListError>;
}

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    /// 0 recorriendo, 1 leyendo la MFT, 2 armando el árbol.
    pub phase: AtomicU32,
    pub mft_done: AtomicU64,
    pub mft_total: AtomicU64,
    pub denied: AtomicU64,
    pub need_admin: AtomicBool,
}

pub struct Scan {
    pub id: usize,
    pub live: Tree,
    pub progress: Arc<Progress>,
    pub cancel: Arc<AtomicBool>,
    pub result: Arc<Mutex<Option<Vec<Node>>>>,
}

pub enum FastScan {
    NotApplicable,
    Done(Vec<Node>),
    NeedsAdmin,
}

pub struct DriveEntry {
    pub path: PathBuf,
    pub label: String,
    pub detail: String,
}

struct Job {
    node: u32,
    path: PathBuf,
}

struct Ctx<'a> {
    tree: &'a RwLock<Vec<Node>>,
    queue: Mutex<(VecDeque<Job>, usize)>,
    cv: Condvar,
    /// Ids de archivo ya contados: un archivo con varios enlaces duros ocupa espacio una sola vez.
    seen: Vec<Mutex<HashSet<u64>>>,
    cancel: &'a AtomicBool,
    progress: &'a Progress,
    lister: &'a dyn Lister,
}

impl Ctx<'_> {
    fn first_time(&self, id: u64) -> bool {
        let shard = ((id ^ (id >> 20)) as usize) % ID_SHARDS;
        self.seen[shard].lock().unwrap().insert(id)
    }
}

/// Agrega el listado al árbol compartido y encola las subcarpetas.
fn insert_listing(ctx: &Ctx, job: &Job, l: DirListing) {
    // Enlaces duros ya contados no suman; el resto cuenta para el avance.
    let mut files: Vec<(Box<str>, u64)> = Vec::with_capacity(l.files.len());
    let (mut n_files, mut n_bytes) = (0u64, 0u64);
    for f in l.files {
        // Los archivos vacíos no ocupan nada: no son nodos ni entran en la deduplicación.
        if f.size == 0 {
            continue;
        }
        if let Some(id) = f.id
            && !ctx.first_time(id)
        {
            continue;
        }
        n_files += 1;
        n_bytes += f.size;
        files.push((f.name, f.size));
    }
    ctx.progress.files.fetch_add(n_files, Ordering::Relaxed);
    ctx.progress.bytes.fetch_add(n_bytes, Ordering::Relaxed);

    // Solo los más pesados entran como nodos propios; el resto se agrupa.
    let (mut small_size, mut small_count) = (0u64, 0u32);
    if files.len() > MAX_FILES_PER_DIR {
        files.select_nth_unstable_by(MAX_FILES_PER_DIR, |a, b| b.1.cmp(&a.1));
        for (_, size) in files.drain(MAX_FILES_PER_DIR..) {
            small_size += size;
            small_count += 1;
        }
    }

    let mut nodes: Vec<Node> = Vec::with_capacity(l.dirs.len() + files.len() + 1);
    for d in &l.dirs {
        nodes.push(Node { name: d.to_string_lossy().into(), parent: job.node, size: 0, is_dir: true, small: false, children: Vec::new() });
    }
    let mut total = small_size;
    for (name, size) in files {
        total += size;
        nodes.push(Node { name, parent: job.node, size, is_dir: false, small: false, children: Vec::new() });
    }
    if small_count > 0 {
        let name = format!("\u{2026} {small_count} archivos pequeños");
        nodes.push(Node { name: name.into(), parent: job.node, size: small_size, is_dir: false, small: true, children: Vec::new() });
    }
    let count = nodes.len() as u32;
    let base = {
        let mut t = ctx.tree.write().unwrap();
        let base = t.len() as u32;
        t.extend(nodes);
        t[job.node as usize].children = (base..base + count).collect();
        // El tamaño sube por toda la cadena de padres: el árbol siempre está al día.
        let mut cur = job.node;
        loop {
            let n = &mut t[cur as usize];
            n.size += total;
            if n.parent == NO_PARENT {
                break;
            }
            cur = n.parent;
        }
        base
    };
    if !l.dirs.is_empty() {
        let mut q = ctx.queue.lock().unwrap();
        for (i, d) in l.dirs.iter().enumerate() {
            q.0.push_back(Job { node: base + i as u32, path: job.path.join(d) });
        }
    }
}

fn worker(ctx: &Ctx) {
    loop {
        let job = {
            let mut st = ctx.queue.lock().unwrap();
            loop {
                if ctx.cancel.load(Ordering::Relaxed) {
                    ctx.cv.notify_all();
                    return;
                }
                if let Some(j) = st.0.pop_front() {
                    st.1 += 1;
                    break j;
                }
                if st.1 == 0 {
                    ctx.cv.notify_all();
                    return;
                }
                st = ctx.cv.wait_timeout(st, Duration::from_millis(100)).unwrap().0;
            }
        };
        // Un fallo en una carpeta no debe colgar todo el análisis.
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.lister.list(&job.path))).unwrap_or(Err(ListError::Other));
        match res {
            Ok(l) => insert_listing(ctx, &job, l),
            Err(ListError::Denied) => {
                ctx.progress.denied.fetch_add(1, Ordering::Relaxed);
            }
            Err(ListError::Other) => {}
        }
        ctx.queue.lock().unwrap().1 -= 1;
        ctx.cv.notify_all();
    }
}

/// Recorre `root` con varios hilos y arma el árbol (bloqueante). `live` se va llenando mientras
/// tanto para que la vista lo muestre. `None` si se canceló.
pub fn walk(root: &Path, label: &str, lister: &dyn Lister, live: &Tree, progress: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>> {
    live.write().unwrap().push(Node { name: label.into(), parent: NO_PARENT, size: 0, is_dir: true, small: false, children: Vec::new() });
    let ctx = Ctx {
        tree: live,
        queue: Mutex::new((VecDeque::from([Job { node: 0, path: root.to_path_buf() }]), 0)),
        cv: Condvar::new(),
        seen: (0..ID_SHARDS).map(|_| Mutex::new(HashSet::new())).collect(),
        cancel,
        progress,
        lister,
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| worker(&ctx));
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    // Orden final: hijos de mayor a menor, para la tabla y el mapa.
    let mut t = live.write().unwrap();
    sort_children(&mut t);
    Some(std::mem::take(&mut *t))
}

/// Empieza un escaneo en segundo plano. Al terminar deja el árbol en `result` y avisa a la
/// ventana con `WM_APP_XTREE` (`lparam` = 1); mientras corre avisa cada 250 ms (`lparam` = 0).
pub fn start(hwnd: Handle, id: usize, root: PathBuf) -> Scan {
    let scan = Scan {
        id,
        live: Arc::new(RwLock::new(Vec::new())),
        progress: Arc::new(Progress::default()),
        cancel: Arc::new(AtomicBool::new(false)),
        result: Arc::new(Mutex::new(None)),
    };
    let finished = Arc::new(AtomicBool::new(false));
    {
        let (cancel, finished) = (scan.cancel.clone(), finished.clone());
        let _ = std::thread::Builder::new().spawn(move || {
            while !finished.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                hwnd.post(WM_APP_XTREE, id, 0);
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    }
    {
        let (live, progress, cancel, result) = (scan.live.clone(), scan.progress.clone(), scan.cancel.clone(), scan.result.clone());
        let _ = std::thread::Builder::new().spawn(move || {
            let text = root.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
            let label = if text.is_empty() { "/".to_string() } else { text };
            progress.phase.store(0, Ordering::Relaxed);
            let tree = match fs::xtree_fast_scan(&root, &progress, &cancel) {
                FastScan::Done(arena) => Some(arena),
                other => {
                    if matches!(other, FastScan::NeedsAdmin) {
                        progress.need_admin.store(true, Ordering::Relaxed);
                    }
                    progress.phase.store(0, Ordering::Relaxed);
                    progress.files.store(0, Ordering::Relaxed);
                    progress.bytes.store(0, Ordering::Relaxed);
                    if cancel.load(Ordering::Relaxed) {
                        None
                    } else {
                        walk(&root, &label, &fs::xtree_lister(&root), &live, &progress, &cancel)
                    }
                }
            };
            finished.store(true, Ordering::Relaxed);
            // Un resultado que llega después de cancelar se descarta.
            if let Some(arena) = tree
                && !cancel.load(Ordering::Relaxed)
            {
                *result.lock().unwrap() = Some(arena);
                hwnd.post(WM_APP_XTREE, id, 1);
            }
        });
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;

    #[derive(Default)]
    struct FakeLister {
        dirs: HashMap<PathBuf, Result<(Vec<&'static str>, Vec<(String, u64, Option<u64>)>), ListError>>,
        delay: Duration,
    }

    impl Lister for FakeLister {
        fn list(&self, path: &Path) -> Result<DirListing, ListError> {
            std::thread::sleep(self.delay);
            match self.dirs.get(path) {
                None => Ok(DirListing::default()),
                Some(Err(e)) => Err(*e),
                Some(Ok((dirs, files))) => Ok(DirListing {
                    dirs: dirs.iter().map(OsString::from).collect(),
                    files: files.iter().map(|(n, s, id)| FileEntry { name: n.as_str().into(), size: *s, id: *id }).collect(),
                }),
            }
        }
    }

    fn run(l: &FakeLister) -> (Option<Vec<Node>>, Progress) {
        let live: Tree = Arc::new(RwLock::new(Vec::new()));
        let p = Progress::default();
        let r = walk(Path::new("/r"), "/r", l, &live, &p, &AtomicBool::new(false));
        (r, p)
    }

    fn f(n: &str, s: u64) -> (String, u64, Option<u64>) {
        (n.to_string(), s, None)
    }

    #[test]
    fn walk_builds_tree_and_totals() {
        let mut l = FakeLister::default();
        l.dirs.insert("/r".into(), Ok((vec!["a"], vec![f("b.bin", 5)])));
        l.dirs.insert("/r/a".into(), Ok((vec![], vec![f("x", 10), f("y", 20)])));
        let (r, p) = run(&l);
        let t = r.unwrap();
        assert_eq!(t[0].size, 35);
        let a = t.iter().find(|n| &*n.name == "a").unwrap();
        assert_eq!(a.size, 30);
        let kids: Vec<u64> = t[0].children.iter().map(|&c| t[c as usize].size).collect();
        assert_eq!(kids, vec![30, 5]);
        assert_eq!(p.files.load(Ordering::Relaxed), 3);
        assert_eq!(p.bytes.load(Ordering::Relaxed), 35);
    }

    #[test]
    fn walk_groups_small_files() {
        let mut l = FakeLister::default();
        let files = (0..1503u64).map(|i| f(&format!("f{i}"), i + 1)).collect();
        l.dirs.insert("/r".into(), Ok((vec![], files)));
        let t = run(&l).0.unwrap();
        let own = t[0].children.iter().filter(|&&c| !t[c as usize].small).count();
        assert_eq!(own, 1500);
        let small: Vec<&Node> = t.iter().filter(|n| n.small).collect();
        assert_eq!(small.len(), 1);
        assert_eq!(&*small[0].name, "\u{2026} 3 archivos pequeños");
        assert_eq!(small[0].size, 6);
    }

    #[test]
    fn walk_counts_hard_links_once() {
        let mut l = FakeLister::default();
        l.dirs.insert("/r".into(), Ok((vec![], vec![("a".into(), 100, Some(7)), ("b".into(), 100, Some(7))])));
        assert_eq!(run(&l).0.unwrap()[0].size, 100);
    }

    #[test]
    fn walk_counts_denied_folders() {
        let mut l = FakeLister::default();
        l.dirs.insert("/r".into(), Ok((vec!["ok", "no"], vec![f("z", 1)])));
        l.dirs.insert("/r/ok".into(), Ok((vec![], vec![f("q", 9)])));
        l.dirs.insert("/r/no".into(), Err(ListError::Denied));
        let (r, p) = run(&l);
        assert_eq!(r.unwrap()[0].size, 10);
        assert_eq!(p.denied.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cancel_stops_workers_and_late_result_is_ignored() {
        let mut l = FakeLister { delay: Duration::from_millis(50), ..Default::default() };
        let names: Vec<&'static str> = (0..200).map(|i| &*Box::leak(format!("d{i}").into_boxed_str())).collect();
        l.dirs.insert("/r".into(), Ok((names, vec![])));
        let live: Tree = Arc::new(RwLock::new(Vec::new()));
        let p = Progress::default();
        let cancel = AtomicBool::new(false);
        let t0 = Instant::now();
        let r = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(20));
                cancel.store(true, Ordering::Relaxed);
            });
            walk(Path::new("/r"), "/r", &l, &live, &p, &cancel)
        });
        assert!(r.is_none());
        assert!(t0.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn walk_ignores_empty_files() {
        let mut l = FakeLister::default();
        let mut files: Vec<(String, u64, Option<u64>)> = (0..1400u64).map(|i| f(&format!("f{i}"), i + 1)).collect();
        files.extend((0..200).map(|i| f(&format!("e{i}"), 0)));
        // Un archivo vacío con el mismo id que uno con datos no debe taparlo.
        files.push(("vacio".into(), 0, Some(9)));
        files.push(("lleno".into(), 50, Some(9)));
        l.dirs.insert("/r".into(), Ok((vec![], files)));
        let (r, p) = run(&l);
        let t = r.unwrap();
        assert!(t.iter().all(|n| !n.small));
        assert_eq!(t[0].children.len(), 1401);
        let expected: u64 = (1..=1400u64).sum::<u64>() + 50;
        assert_eq!(t[0].size, expected);
        assert_eq!(p.files.load(Ordering::Relaxed), 1401);
    }
}

//! Renombrar varios: el cálculo de los nombres nuevos (Reemplazar texto, Agregar texto y Formato),
//! los choques que impiden aplicarlos y el renombrado seguro de todos juntos (`rename_all`): primero
//! cada uno a un nombre temporal y después al final, así los nombres que se cruzan (a→b y b→a) no
//! se pisan; si algo falla en el medio, se vuelve todo atrás. Lógica pura: el sistema solo
//! renombra (`RenamePrims`).
//!
//! Los nombres no se normalizan (NFC/NFD): se comparan tal como los da el sistema. macOS los
//! guarda descompuestos y la lista los muestra igual, así que un «ñ» escrito en el panel y uno del
//! disco pueden no coincidir al buscar; el choque con un nombre que ya existe lo atrapa igual el
//! sistema al renombrar (`rename_noreplace` no pisa), y entonces se vuelve todo atrás.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ops::Undo;

/// Caracteres que no puede tener un nombre en Windows.
pub const INVALID_WINDOWS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
/// Caracteres que no puede tener un nombre en macOS y Linux.
pub const INVALID_UNIX: &[char] = &['/'];

/// Las reglas de nombres del sistema (las da `FileSystem::name_rules`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameRules {
    /// Caracteres que no puede tener (además de los de control).
    pub chars: &'static [char],
    /// Windows quita el punto o el espacio del final: «a.» sería «a».
    pub no_trailing_dot_space: bool,
    /// Los nombres de dispositivo de DOS (CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM1–9, LPT1–9), con o
    /// sin extensión (`names::is_dos_reserved`).
    pub reserved_dos: bool,
    /// Largo máximo de un nombre: en unidades UTF-16 si `utf16` (Windows), si no en bytes UTF-8.
    pub max_len: usize,
    pub utf16: bool,
}

impl NameRules {
    pub const WINDOWS: NameRules = NameRules { chars: INVALID_WINDOWS, no_trailing_dot_space: true, reserved_dos: true, max_len: 255, utf16: true };
    pub const UNIX: NameRules = NameRules { chars: INVALID_UNIX, no_trailing_dot_space: false, reserved_dos: false, max_len: 255, utf16: false };

    fn too_long(&self, name: &str) -> bool {
        let len = if self.utf16 { name.encode_utf16().count() } else { name.len() };
        len > self.max_len
    }
}

/// Qué se le hace al nombre (siempre sin la extensión, que queda igual).
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// Todas las apariciones de `find` cambian por `with`. `match_case`: «Distinguir mayúsculas»
    /// (el diseño X51 la trae prendida).
    Replace { find: String, with: String, match_case: bool },
    /// `text` antes o después del nombre.
    Add { text: String, before: bool },
    /// «base 1», «base 001» o «base 2026-11-10» (`before`: «1 base»); `base` vacío = el nombre
    /// de cada uno.
    Format { style: FormatStyle, base: String, before: bool, start: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatStyle {
    /// «Nombre y número»: 1, 2, 3…
    Number,
    /// «Nombre y contador»: 001, 002… (al menos 3 cifras, más si hacen falta).
    Counter,
    /// «Nombre y fecha»: la de modificación, AAAA-MM-DD.
    Date,
}

/// Un elemento a renombrar: su nombre completo, si es carpeta y el día (local) de modificación.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub name: String,
    pub is_dir: bool,
    pub modified_day: (i32, u8, u8),
}

/// Por qué un nombre nuevo no se puede aplicar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    /// Otro de los que se renombran quedaría con el mismo nombre.
    Duplicate,
    /// Ya hay un elemento con ese nombre en la carpeta (que no se renombra).
    Exists,
    /// El nombre queda vacío (o solo la extensión).
    Empty,
    /// Empezaría con punto (y quedaría oculto) sin haberlo estado antes.
    Hidden,
    /// Tiene un carácter que el sistema no permite.
    Invalid(char),
    /// Termina en punto o en espacio (Windows los quita).
    TrailingDotSpace,
    /// Es un nombre de dispositivo de Windows (CON, NUL, COM1…).
    Reserved,
    /// Pasa del largo máximo del sistema.
    TooLong,
}

impl Clash {
    /// El cartel de la fila con el choque.
    pub fn tip(self, new_name: &str) -> String {
        match self {
            Clash::Duplicate => format!("Otro elemento también se llamaría «{new_name}»"),
            Clash::Exists => format!("Ya existe «{new_name}» en esta carpeta"),
            Clash::Empty => "El nombre no puede quedar vacío".to_string(),
            Clash::Hidden => "Empezaría con punto y quedaría oculto".to_string(),
            Clash::Invalid(c) => format!("El nombre no puede tener «{c}»"),
            Clash::TrailingDotSpace => "El nombre no puede terminar en punto ni en espacio".to_string(),
            Clash::Reserved => format!("«{new_name}» es un nombre reservado de Windows"),
            Clash::TooLong => "El nombre es demasiado largo (más de 255 caracteres)".to_string(),
        }
    }

    /// El resumen corto del panel (««x» ya existe»).
    pub fn short(self, new_name: &str) -> String {
        match self {
            Clash::Duplicate => format!("«{new_name}» se repite"),
            Clash::Exists => format!("«{new_name}» ya existe"),
            Clash::Empty => "un nombre queda vacío".to_string(),
            Clash::Hidden => format!("«{new_name}» quedaría oculto"),
            Clash::Invalid(c) => format!("«{c}» no se permite en un nombre"),
            Clash::TrailingDotSpace => format!("«{new_name}» termina en punto o espacio"),
            Clash::Reserved => format!("«{new_name}» es un nombre reservado"),
            Clash::TooLong => "un nombre es demasiado largo".to_string(),
        }
    }
}

/// (nombre, extensión con el punto). Las carpetas y los nombres que empiezan con punto no tienen
/// extensión; solo cuenta la última («a.tar.gz» → («a.tar», «.gz»)).
// Igual que `names::split_ext` de la tanda D (otra rama): al juntarlas, usar esa.
fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if !is_dir && dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

/// Reemplaza todas las apariciones de `find` en `s` (sin distinguir mayúsculas si `match_case`
/// es false; el resto del texto queda como estaba).
fn replace_all(s: &str, find: &str, with: &str, match_case: bool) -> String {
    if find.is_empty() {
        return s.to_string();
    }
    if match_case {
        return s.replace(find, with);
    }
    // Sin distinguir mayúsculas: se busca carácter por carácter comparando en minúsculas.
    let find_low: Vec<char> = find.chars().flat_map(char::to_lowercase).collect();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = String::new();
    let mut i = 0;
    'outer: while i < chars.len() {
        let mut low = Vec::new();
        let mut j = i;
        while j < chars.len() && low.len() < find_low.len() {
            low.extend(chars[j].1.to_lowercase());
            j += 1;
            if low == find_low {
                out.push_str(with);
                i = j;
                continue 'outer;
            }
        }
        out.push(chars[i].1);
        i += 1;
    }
    out
}

/// Los nombres nuevos, en el orden de `items`.
pub fn compute(items: &[Source], mode: &Mode) -> Vec<String> {
    let last = match mode {
        Mode::Format { start, .. } => start.saturating_add(items.len().saturating_sub(1) as u32),
        _ => 0,
    };
    let width = last.to_string().len().max(3);
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let (stem, ext) = split_ext(&it.name, it.is_dir);
            let new_stem = match mode {
                Mode::Replace { find, with, match_case } => replace_all(stem, find, with, *match_case),
                Mode::Add { text, before: true } => format!("{text}{stem}"),
                Mode::Add { text, before: false } => format!("{stem}{text}"),
                Mode::Format { style, base, before, start } => {
                    let base = if base.is_empty() { stem } else { base.as_str() };
                    let n = start.saturating_add(i as u32);
                    let tag = match style {
                        FormatStyle::Number => n.to_string(),
                        FormatStyle::Counter => format!("{n:0width$}"),
                        FormatStyle::Date => {
                            let (y, m, d) = it.modified_day;
                            format!("{y:04}-{m:02}-{d:02}")
                        }
                    };
                    if *before { format!("{tag} {base}") } else { format!("{base} {tag}") }
                }
            };
            format!("{new_stem}{ext}")
        })
        .collect()
}

/// El choque de cada nombre nuevo (`None`: se puede aplicar). `old` y `new` van en el mismo orden;
/// `others` son los nombres de la carpeta que no se renombran; `case_insensitive`: el disco no
/// distingue mayúsculas (un nombre que solo cambia mayúsculas no choca consigo mismo). Los que no
/// cambian no tienen choque (los demás sí chocan con ellos: siguen ocupando su nombre).
pub fn clashes(old: &[String], new: &[String], others: &[String], case_insensitive: bool, rules: &NameRules) -> Vec<Option<Clash>> {
    let key = |s: &str| if case_insensitive { s.to_lowercase() } else { s.to_string() };
    let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for n in new {
        *count.entry(key(n)).or_insert(0) += 1;
    }
    let others: std::collections::HashSet<String> = others.iter().map(|o| key(o)).collect();
    old.iter()
        .zip(new)
        .map(|(o, n)| {
            if o == n {
                return None;
            }
            // Vacío, o solo la extensión que tenía («a.txt» → «.txt»).
            let (_, ext) = split_ext(o, false);
            if n.trim().is_empty() || n == "." || n == ".." || (!ext.is_empty() && n == ext) {
                return Some(Clash::Empty);
            }
            if n.starts_with('.') && !o.starts_with('.') {
                return Some(Clash::Hidden);
            }
            if let Some(c) = n.chars().find(|c| rules.chars.contains(c) || c.is_control()) {
                return Some(Clash::Invalid(c));
            }
            if rules.no_trailing_dot_space && (n.ends_with('.') || n.ends_with(' ')) {
                return Some(Clash::TrailingDotSpace);
            }
            if rules.reserved_dos && crate::names::is_dos_reserved(n) {
                return Some(Clash::Reserved);
            }
            if rules.too_long(n) {
                return Some(Clash::TooLong);
            }
            let k = key(n);
            if count.get(&k).copied().unwrap_or(0) > 1 {
                return Some(Clash::Duplicate);
            }
            if others.contains(&k) {
                return Some(Clash::Exists);
            }
            None
        })
        .collect()
}

/// Lo que `rename_all` necesita del sistema: renombrar sin pisar nunca lo que ya está.
pub trait RenamePrims {
    /// Renombra `from` a `to`; falla (con el texto para el usuario) si `to` ya existe.
    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), String>;
}

/// Las primitivas del sistema instalado (`sys::file_system()`).
pub struct SysRenames;

impl RenamePrims for SysRenames {
    fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), String> {
        crate::sys::file_system().rename_noreplace(from, to)
    }
}

/// Texto para el usuario de un error de E/S al renombrar `path` (lo usan las plataformas).
pub fn io_message(e: &std::io::Error, path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("Ya existe «{name}»"),
        std::io::ErrorKind::PermissionDenied => format!("No tenés permiso para renombrar «{name}»"),
        std::io::ErrorKind::NotFound => format!("«{name}» ya no existe"),
        _ => format!("«{name}»: {e}"),
    }
}

/// Número de llamada a `rename_all` (con el pid, hace únicos los temporales aunque corran dos a la
/// vez en la misma carpeta).
static SEQ: AtomicU64 = AtomicU64::new(0);

/// El nombre temporal del elemento `i` de la llamada `seq`: visible (sin punto adelante) y con el
/// nombre original, para que alguien lo reconozca y lo arregle a mano si quedara así:
/// «IMG_1 (xdir renombrando 4321-7-0).jpg». Si no entra en 255 bytes, se acorta el nombre.
fn temp_name(original: &str, seq: u64, i: usize) -> String {
    let (stem, ext) = split_ext(original, false);
    let tag = format!(" (xdir renombrando {}-{seq}-{i})", std::process::id());
    let room = 255usize.saturating_sub(tag.len() + ext.len());
    let mut cut = stem.len().min(room);
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{tag}{ext}", &stem[..cut])
}

/// Dónde está cada elemento durante `rename_all`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum At {
    Original,
    Temp,
    Final,
}

/// Renombra cada `(ruta, nombre nuevo)` en su carpeta, en dos pasos: todos a un temporal visible
/// (`temp_name`, único por llamada) y después al nombre final. Si un paso falla, deshace lo que
/// hizo esta llamada (solo mueve lo que ella movió) y devuelve el error; si algo no se pudo
/// devolver, el error dice cómo quedó cada uno. Devuelve cómo deshacerlo todo junto
/// (`Undo::Renames`).
pub fn rename_all(prims: &dyn RenamePrims, pairs: &[(PathBuf, String)]) -> Result<Undo, String> {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name_of = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temps: Vec<PathBuf> = pairs.iter().enumerate().map(|(i, (p, _))| p.with_file_name(temp_name(&name_of(p), seq, i))).collect();
    let finals: Vec<PathBuf> = pairs.iter().map(|(p, n)| p.with_file_name(n)).collect();
    let mut at = vec![At::Original; pairs.len()];
    // Paso 1: a los temporales; paso 2: a los finales.
    for (i, (path, _)) in pairs.iter().enumerate() {
        if let Err(e) = prims.rename_noreplace(path, &temps[i]) {
            return Err(with_stranded(e, &rollback(prims, pairs, &temps, &finals, &mut at)));
        }
        at[i] = At::Temp;
    }
    for i in 0..pairs.len() {
        if let Err(e) = prims.rename_noreplace(&temps[i], &finals[i]) {
            return Err(with_stranded(e, &rollback(prims, pairs, &temps, &finals, &mut at)));
        }
        at[i] = At::Final;
    }
    let undo = finals.into_iter().zip(pairs).map(|(now, (old, _))| (now, name_of(old))).collect();
    Ok(Undo::Renames(undo))
}

/// Vuelve atrás lo que hizo esta llamada, del último al primero: los finales a su temporal (así
/// quedan libres los nombres originales aunque se crucen) y los temporales al original. Devuelve
/// los que no volvieron: (dónde quedaron, nombre original).
fn rollback(prims: &dyn RenamePrims, pairs: &[(PathBuf, String)], temps: &[PathBuf], finals: &[PathBuf], at: &mut [At]) -> Vec<(PathBuf, String)> {
    for i in (0..at.len()).rev() {
        if at[i] == At::Final && prims.rename_noreplace(&finals[i], &temps[i]).is_ok() {
            at[i] = At::Temp;
        }
    }
    for i in (0..at.len()).rev() {
        if at[i] == At::Temp && prims.rename_noreplace(&temps[i], &pairs[i].0).is_ok() {
            at[i] = At::Original;
        }
    }
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (0..at.len())
        .filter_map(|i| match at[i] {
            At::Original => None,
            At::Temp => Some((temps[i].clone(), name(&pairs[i].0))),
            At::Final => Some((finals[i].clone(), name(&pairs[i].0))),
        })
        .collect()
}

/// El error con los que quedaron sin volver: «…; no se pudo volver atrás y quedaron así: «x» (era
/// «1»)».
fn with_stranded(e: String, stranded: &[(PathBuf, String)]) -> String {
    if stranded.is_empty() {
        return e;
    }
    let list: Vec<String> = stranded
        .iter()
        .map(|(now, was)| format!("«{}» (era «{was}»)", now.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()))
        .collect();
    format!("{e}; no se pudo volver atrás y quedaron así: {}", list.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeSet;

    fn src(name: &str) -> Source {
        Source { name: name.into(), is_dir: false, modified_day: (2026, 11, 10) }
    }

    fn names(items: &[&str], mode: &Mode) -> Vec<String> {
        compute(&items.iter().map(|n| src(n)).collect::<Vec<_>>(), mode)
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn replace_text_all_occurrences() {
        let m = Mode::Replace { find: "IMG_".into(), with: "Viaje-".into(), match_case: true };
        assert_eq!(names(&["IMG_2041.jpg", "img_2042.jpg", "IMG_IMG_1.png"], &m), ["Viaje-2041.jpg", "img_2042.jpg", "Viaje-Viaje-1.png"]);
        // Sin distinguir mayúsculas, también la de minúsculas.
        let m = Mode::Replace { find: "IMG_".into(), with: "Viaje-".into(), match_case: false };
        assert_eq!(names(&["img_2042.jpg", "ÁimG_x"], &m), ["Viaje-2042.jpg", "ÁViaje-x"]);
        // La extensión no se toca aunque coincida.
        let m = Mode::Replace { find: "jpg".into(), with: "png".into(), match_case: true };
        assert_eq!(names(&["jpg.jpg"], &m), ["png.jpg"]);
        // Buscar vacío: sin cambios.
        let m = Mode::Replace { find: String::new(), with: "x".into(), match_case: true };
        assert_eq!(names(&["a.txt"], &m), ["a.txt"]);
    }

    #[test]
    fn add_text_before_after() {
        assert_eq!(names(&["foto.jpg"], &Mode::Add { text: "2026 ".into(), before: true }), ["2026 foto.jpg"]);
        assert_eq!(names(&["foto.jpg"], &Mode::Add { text: " (copia)".into(), before: false }), ["foto (copia).jpg"]);
    }

    #[test]
    fn format_number_counter_date() {
        let items: Vec<&str> = (0..12).map(|_| "x.jpg").collect();
        let m = Mode::Format { style: FormatStyle::Counter, base: "base".into(), before: false, start: 1 };
        let out = names(&items, &m);
        assert_eq!(out[0], "base 001.jpg");
        assert_eq!(out[11], "base 012.jpg");
        let m = Mode::Format { style: FormatStyle::Counter, base: "base".into(), before: false, start: 998 };
        assert_eq!(names(&["a", "b", "c", "d", "e"], &m), ["base 0998", "base 0999", "base 1000", "base 1001", "base 1002"]);
        let m = Mode::Format { style: FormatStyle::Number, base: "base".into(), before: false, start: 1 };
        assert_eq!(names(&["a.txt", "b.txt"], &m), ["base 1.txt", "base 2.txt"]);
        let m = Mode::Format { style: FormatStyle::Number, base: "base".into(), before: true, start: 1 };
        assert_eq!(names(&["a.txt"], &m), ["1 base.txt"]);
        let m = Mode::Format { style: FormatStyle::Date, base: "base".into(), before: false, start: 1 };
        assert_eq!(names(&["a.txt"], &m), ["base 2026-11-10.txt"]);
        // Base vacía: el nombre de cada uno.
        let m = Mode::Format { style: FormatStyle::Number, base: String::new(), before: false, start: 5 };
        assert_eq!(names(&["a.txt", "b.txt"], &m), ["a 5.txt", "b 6.txt"]);
    }

    #[test]
    fn extension_kept_and_dirs_without_ext() {
        let dir = Source { name: "Fotos.2024".into(), is_dir: true, modified_day: (2026, 1, 2) };
        let file = src("a.tar.gz");
        let dot = src(".bashrc");
        let out = compute(&[dir, file, dot], &Mode::Add { text: "-x".into(), before: false });
        assert_eq!(out, ["Fotos.2024-x", "a.tar-x.gz", ".bashrc-x"]);
    }

    #[test]
    fn clash_duplicate_and_exists() {
        let old = strings(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
        let new = strings(&["x.jpg", "x.jpg", "otro.jpg", "d.jpg"]);
        let c = clashes(&old, &new, &strings(&["otro.jpg"]), false, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::Duplicate), Some(Clash::Duplicate), Some(Clash::Exists), None]);
        // Con uno que no cambia: choca el que va a su nombre, no él.
        let c = clashes(&strings(&["a", "b"]), &strings(&["b", "b"]), &[], false, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::Duplicate), None]);
        // Sin distinguir mayúsculas, «X.jpg» y «x.jpg» son el mismo.
        let c = clashes(&strings(&["a", "b"]), &strings(&["X.jpg", "x.jpg"]), &[], true, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::Duplicate), Some(Clash::Duplicate)]);
        let c = clashes(&strings(&["a", "b"]), &strings(&["X.jpg", "x.jpg"]), &[], false, &NameRules::UNIX);
        assert_eq!(c, [None, None]);
        let c = clashes(&strings(&["a"]), &strings(&["OTRO.jpg"]), &strings(&["otro.jpg"]), true, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::Exists)]);
    }

    #[test]
    fn clash_empty_and_invalid() {
        let c = clashes(&strings(&["a.txt", "b.txt", "c"]), &strings(&[".txt", "a:b.txt", " "]), &[], false, &NameRules::WINDOWS);
        assert_eq!(c, [Some(Clash::Empty), Some(Clash::Invalid(':')), Some(Clash::Empty)]);
        // En macOS y Linux «:» se permite; «/» nunca.
        let c = clashes(&strings(&["a", "b"]), &strings(&["a:b", "a/b"]), &[], false, &NameRules::UNIX);
        assert_eq!(c, [None, Some(Clash::Invalid('/'))]);
    }

    #[test]
    fn rename_many_case_only_change() {
        // «a.txt» → «A.txt» en un disco que no distingue mayúsculas: no choca consigo mismo.
        let c = clashes(&strings(&["a.txt"]), &strings(&["A.txt"]), &strings(&["b.txt"]), true, &NameRules::UNIX);
        assert_eq!(c, [None]);
        // Y con otro elemento que no se renombra llamado «A.txt» sí.
        let c = clashes(&strings(&["a.txt"]), &strings(&["B.txt"]), &strings(&["b.txt"]), true, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::Exists)]);
        // Por el disco: con el falso que no distingue mayúsculas, el cambio pasa por el temporal.
        let fake = FakeRenames::new(&["/d/a.txt"], true);
        rename_all(&fake, &[(PathBuf::from("/d/a.txt"), "A.txt".into())]).unwrap();
        assert_eq!(fake.names(), ["/d/A.txt"]);
    }

    /// Disco falso en memoria: rutas que existen, si distingue mayúsculas y rutas que fallan.
    struct FakeRenames {
        files: RefCell<BTreeSet<String>>,
        ci: bool,
        fail_to: RefCell<Vec<String>>,
        /// Fallan los destinos que empiezan con alguno de estos nombres (sin la carpeta).
        fail_names: RefCell<Vec<String>>,
        /// Los renombrados hechos (desde, hasta), en orden.
        log: RefCell<Vec<(String, String)>>,
    }

    impl FakeRenames {
        fn new(files: &[&str], ci: bool) -> FakeRenames {
            FakeRenames {
                files: RefCell::new(files.iter().map(|f| f.to_string()).collect()),
                ci,
                fail_to: RefCell::new(Vec::new()),
                fail_names: RefCell::new(Vec::new()),
                log: RefCell::new(Vec::new()),
            }
        }
        fn key(&self, s: &str) -> String {
            if self.ci { s.to_lowercase() } else { s.to_string() }
        }
        fn names(&self) -> Vec<String> {
            self.files.borrow().iter().cloned().collect()
        }
    }

    impl RenamePrims for FakeRenames {
        fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), String> {
            // `rename_all` arma las rutas con `with_file_name`, que en Windows junta con «\»
            // («/d/1» → «/d\x»): el falso guarda las rutas siempre con «/».
            let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");
            let (f, t) = (norm(from), norm(to));
            let short = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if self.fail_to.borrow().contains(&t) || self.fail_names.borrow().iter().any(|p| short.starts_with(p.as_str())) {
                return Err(format!("falla {t}"));
            }
            let mut files = self.files.borrow_mut();
            let Some(existing) = files.iter().find(|x| self.key(x) == self.key(&f)).cloned() else { return Err(format!("no existe {f}")) };
            if files.iter().any(|x| self.key(x) == self.key(&t)) {
                return Err(format!("ya existe {t}"));
            }
            files.remove(&existing);
            self.log.borrow_mut().push((f, t.clone()));
            files.insert(t);
            Ok(())
        }
    }

    #[test]
    fn rename_all_cross_names() {
        let fake = FakeRenames::new(&["/d/a", "/d/b"], false);
        let undo = rename_all(&fake, &[(PathBuf::from("/d/a"), "b".into()), (PathBuf::from("/d/b"), "a".into())]).unwrap();
        assert_eq!(fake.names(), ["/d/a", "/d/b"]);
        assert_eq!(undo, Undo::Renames(vec![(PathBuf::from("/d/b"), "a".into()), (PathBuf::from("/d/a"), "b".into())]));
        // Sin temporales sueltos.
        assert!(fake.names().iter().all(|n| !n.contains("xdir renombrando")));
    }

    #[test]
    fn rename_all_rolls_back() {
        let fake = FakeRenames::new(&["/d/1", "/d/2", "/d/3"], false);
        fake.fail_to.borrow_mut().push("/d/z".into());
        let pairs = [(PathBuf::from("/d/1"), "x".into()), (PathBuf::from("/d/2"), "y".into()), (PathBuf::from("/d/3"), "z".into())];
        assert!(rename_all(&fake, &pairs).is_err());
        assert_eq!(fake.names(), ["/d/1", "/d/2", "/d/3"]);
        // Falla en el primer paso (el temporal del tercero).
        let fake = FakeRenames::new(&["/d/1", "/d/2", "/d/3"], false);
        fake.fail_names.borrow_mut().push("3 (xdir renombrando".into());
        assert!(rename_all(&fake, &pairs).is_err());
        assert_eq!(fake.names(), ["/d/1", "/d/2", "/d/3"]);
    }

    #[test]
    fn undo_renames_roundtrip() {
        let fake = FakeRenames::new(&["/d/IMG_1.jpg", "/d/IMG_2.jpg"], false);
        let Undo::Renames(back) =
            rename_all(&fake, &[(PathBuf::from("/d/IMG_1.jpg"), "Viaje-1.jpg".into()), (PathBuf::from("/d/IMG_2.jpg"), "Viaje-2.jpg".into())]).unwrap()
        else {
            panic!("se esperaba Renames")
        };
        assert_eq!(fake.names(), ["/d/Viaje-1.jpg", "/d/Viaje-2.jpg"]);
        // Deshacer es otro `rename_all` con esos mismos pares.
        rename_all(&fake, &back).unwrap();
        assert_eq!(fake.names(), ["/d/IMG_1.jpg", "/d/IMG_2.jpg"]);
    }

    #[test]
    fn rollback_failure_names_what_stayed() {
        // Falla el paso 2 del tercero y, al volver, también el original del primero: el error lo
        // dice y el temporal es visible y lleva el nombre original.
        let fake = FakeRenames::new(&["/d/1.jpg", "/d/2.jpg", "/d/3.jpg"], false);
        fake.fail_to.borrow_mut().extend(["/d/z.jpg".to_string(), "/d/1.jpg".to_string()]);
        let pairs = [(PathBuf::from("/d/1.jpg"), "x.jpg".into()), (PathBuf::from("/d/2.jpg"), "y.jpg".into()), (PathBuf::from("/d/3.jpg"), "z.jpg".into())];
        let e = rename_all(&fake, &pairs).unwrap_err();
        let names = fake.names();
        let stuck = names.iter().find(|n| n.contains("xdir renombrando")).expect("queda un temporal");
        let stuck = stuck.trim_start_matches("/d/");
        assert!(stuck.starts_with("1 (xdir renombrando ") && stuck.ends_with(").jpg"), "{stuck}");
        assert!(e.starts_with("falla /d/z.jpg; no se pudo volver atrás y quedaron así: "), "{e}");
        assert!(e.contains(&format!("«{stuck}» (era «1.jpg»)")), "{e}");
        assert!(names.contains(&"/d/2.jpg".to_string()) && names.contains(&"/d/3.jpg".to_string()));
    }

    #[test]
    fn temp_names_unique_per_call_and_rollback_owns_only_its_own() {
        let fake = FakeRenames::new(&["/d/a", "/d/b", "/d/c"], false);
        rename_all(&fake, &[(PathBuf::from("/d/a"), "a2".into())]).unwrap();
        rename_all(&fake, &[(PathBuf::from("/d/b"), "b2".into())]).unwrap();
        let temps: Vec<String> = fake.log.borrow().iter().map(|(_, t)| t.clone()).filter(|t| t.contains("renombrando")).collect();
        assert_eq!(temps.len(), 2);
        let tag = |t: &str| t.split("renombrando ").nth(1).unwrap().to_string();
        assert_ne!(tag(&temps[0]), tag(&temps[1]), "cada llamada tiene su número: {temps:?}");
        // Falla el temporal del segundo: solo se devuelve el primero (nunca se toca otro temporal).
        fake.log.borrow_mut().clear();
        fake.fail_names.borrow_mut().push("b2 (xdir".into());
        assert!(rename_all(&fake, &[(PathBuf::from("/d/a2"), "x".into()), (PathBuf::from("/d/b2"), "y".into())]).is_err());
        let log = fake.log.borrow().clone();
        assert_eq!(log.len(), 2, "{log:?}");
        assert_eq!(log[1].1, "/d/a2");
        assert_eq!(fake.names(), ["/d/a2", "/d/b2", "/d/c"]);
    }

    #[test]
    fn temp_name_is_visible_and_fits() {
        let t = temp_name("IMG_1.jpg", 7, 0);
        assert_eq!(t, format!("IMG_1 (xdir renombrando {}-7-0).jpg", std::process::id()));
        let long = "ñ".repeat(127) + ".txt";
        let t = temp_name(&long, 1, 2);
        assert!(t.len() <= 255 && t.ends_with(".txt") && t.starts_with("ññ"), "{t}");
    }

    #[test]
    fn clash_windows_rules() {
        let old = strings(&["a.txt", "b.txt", "c.txt", "d.txt", "e.txt", "f.txt"]);
        let long = "x".repeat(252) + ".txt";
        let new = vec!["a.".to_string(), "b ".into(), "CON.txt".into(), "lpt1.tar.gz".into(), long.clone(), "COM10.txt".into()];
        let c = clashes(&old, &new, &[], true, &NameRules::WINDOWS);
        assert_eq!(c, [Some(Clash::TrailingDotSpace), Some(Clash::TrailingDotSpace), Some(Clash::Reserved), Some(Clash::Reserved), Some(Clash::TooLong), None]);
        // Las consolas también (la misma lista que al extraer: `names::is_dos_reserved`).
        let c = clashes(&strings(&["a", "b"]), &strings(&["CONIN$", "conout$.txt"]), &[], true, &NameRules::WINDOWS);
        assert_eq!(c, [Some(Clash::Reserved), Some(Clash::Reserved)]);
        // En macOS y Linux esos nombres valen; el largo se cuenta en bytes.
        let c = clashes(&old[..4], &new[..4], &[], false, &NameRules::UNIX);
        assert_eq!(c, [None, None, None, None]);
        let c = clashes(&strings(&["a"]), &["é".repeat(128)], &[], false, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::TooLong)]);
        let c = clashes(&strings(&["a"]), &["é".repeat(128)], &[], false, &NameRules::WINDOWS);
        assert_eq!(c, [None]);
    }

    #[test]
    fn clash_hidden_is_its_own_reason() {
        let c = clashes(&strings(&["a.txt", ".env"]), &strings(&[".a.txt", ".env2"]), &[], false, &NameRules::UNIX);
        assert_eq!(c, [Some(Clash::Hidden), None]);
        assert_eq!(Clash::Hidden.tip(".a.txt"), "Empezaría con punto y quedaría oculto");
    }

    #[test]
    fn clash_texts() {
        assert_eq!(Clash::Exists.tip("Bariloche-3090.jpg"), "Ya existe «Bariloche-3090.jpg» en esta carpeta");
        assert_eq!(Clash::Exists.short("Bariloche-3090.jpg"), "«Bariloche-3090.jpg» ya existe");
    }
}

//! Renombrar varios: el cálculo de los nombres nuevos (Reemplazar texto, Agregar texto y Formato),
//! los choques que impiden aplicarlos y el renombrado seguro de todos juntos (`rename_all`): primero
//! cada uno a un nombre temporal y después al final, así los nombres que se cruzan (a→b y b→a) no
//! se pisan; si algo falla en el medio, se vuelve todo atrás. Lógica pura: el sistema solo
//! renombra (`RenamePrims`).

use std::path::{Path, PathBuf};

use crate::ops::Undo;

/// Caracteres que no puede tener un nombre en Windows.
pub const INVALID_WINDOWS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
/// Caracteres que no puede tener un nombre en macOS y Linux.
pub const INVALID_UNIX: &[char] = &['/'];

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
    /// Tiene un carácter que el sistema no permite.
    Invalid(char),
}

impl Clash {
    /// El cartel de la fila con el choque.
    pub fn tip(self, new_name: &str) -> String {
        match self {
            Clash::Duplicate => format!("Otro elemento también se llamaría «{new_name}»"),
            Clash::Exists => format!("Ya existe «{new_name}» en esta carpeta"),
            Clash::Empty => "El nombre no puede quedar vacío".to_string(),
            Clash::Invalid(c) => format!("El nombre no puede tener «{c}»"),
        }
    }

    /// El resumen corto del panel (««x» ya existe»).
    pub fn short(self, new_name: &str) -> String {
        match self {
            Clash::Duplicate => format!("«{new_name}» se repite"),
            Clash::Exists => format!("«{new_name}» ya existe"),
            Clash::Empty => "un nombre queda vacío".to_string(),
            Clash::Invalid(c) => format!("«{c}» no se permite en un nombre"),
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
pub fn clashes(old: &[String], new: &[String], others: &[String], case_insensitive: bool, invalid: &[char]) -> Vec<Option<Clash>> {
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
            if n.trim().is_empty() || n == "." || n == ".." || (n.starts_with('.') && !o.starts_with('.')) {
                return Some(Clash::Empty);
            }
            if let Some(c) = n.chars().find(|c| invalid.contains(c) || c.is_control()) {
                return Some(Clash::Invalid(c));
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

/// Renombra cada `(ruta, nombre nuevo)` en su carpeta, en dos pasos: todos a un temporal
/// `.xdir-ren-<pid>-<i>` y después al nombre final. Si un paso falla, deshace lo hecho en orden
/// inverso y devuelve el error. Devuelve cómo deshacerlo todo junto (`Undo::Renames`).
pub fn rename_all(prims: &dyn RenamePrims, pairs: &[(PathBuf, String)]) -> Result<Undo, String> {
    let pid = std::process::id();
    let temps: Vec<PathBuf> = pairs.iter().enumerate().map(|(i, (p, _))| p.with_file_name(format!(".xdir-ren-{pid}-{i}"))).collect();
    let finals: Vec<PathBuf> = pairs.iter().map(|(p, n)| p.with_file_name(n)).collect();
    // Paso 1: a los temporales.
    for (i, (path, _)) in pairs.iter().enumerate() {
        if let Err(e) = prims.rename_noreplace(path, &temps[i]) {
            back(prims, &temps[..i], &pairs[..i].iter().map(|(p, _)| p.clone()).collect::<Vec<_>>());
            return Err(e);
        }
    }
    // Paso 2: a los finales.
    for i in 0..pairs.len() {
        if let Err(e) = prims.rename_noreplace(&temps[i], &finals[i]) {
            back(prims, &finals[..i], &temps[..i]);
            let originals: Vec<PathBuf> = pairs.iter().map(|(p, _)| p.clone()).collect();
            back(prims, &temps, &originals);
            return Err(e);
        }
    }
    let undo = finals
        .into_iter()
        .zip(pairs)
        .map(|(now, (old, _))| (now, old.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
        .collect();
    Ok(Undo::Renames(undo))
}

/// Vuelve cada `from[i]` a `to[i]`, del último al primero (lo que falle acá ya no tiene arreglo:
/// queda con el nombre temporal, que se ve en la carpeta).
fn back(prims: &dyn RenamePrims, from: &[PathBuf], to: &[PathBuf]) {
    for (f, t) in from.iter().zip(to).rev() {
        let _ = prims.rename_noreplace(f, t);
    }
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
        let c = clashes(&old, &new, &strings(&["otro.jpg"]), false, INVALID_UNIX);
        assert_eq!(c, [Some(Clash::Duplicate), Some(Clash::Duplicate), Some(Clash::Exists), None]);
        // Con uno que no cambia: choca el que va a su nombre, no él.
        let c = clashes(&strings(&["a", "b"]), &strings(&["b", "b"]), &[], false, INVALID_UNIX);
        assert_eq!(c, [Some(Clash::Duplicate), None]);
        // Sin distinguir mayúsculas, «X.jpg» y «x.jpg» son el mismo.
        let c = clashes(&strings(&["a", "b"]), &strings(&["X.jpg", "x.jpg"]), &[], true, INVALID_UNIX);
        assert_eq!(c, [Some(Clash::Duplicate), Some(Clash::Duplicate)]);
        let c = clashes(&strings(&["a", "b"]), &strings(&["X.jpg", "x.jpg"]), &[], false, INVALID_UNIX);
        assert_eq!(c, [None, None]);
        let c = clashes(&strings(&["a"]), &strings(&["OTRO.jpg"]), &strings(&["otro.jpg"]), true, INVALID_UNIX);
        assert_eq!(c, [Some(Clash::Exists)]);
    }

    #[test]
    fn clash_empty_and_invalid() {
        let c = clashes(&strings(&["a.txt", "b.txt", "c"]), &strings(&[".txt", "a:b.txt", " "]), &[], false, INVALID_WINDOWS);
        assert_eq!(c, [Some(Clash::Empty), Some(Clash::Invalid(':')), Some(Clash::Empty)]);
        // En macOS y Linux «:» se permite; «/» nunca.
        let c = clashes(&strings(&["a", "b"]), &strings(&["a:b", "a/b"]), &[], false, INVALID_UNIX);
        assert_eq!(c, [None, Some(Clash::Invalid('/'))]);
    }

    #[test]
    fn rename_many_case_only_change() {
        // «a.txt» → «A.txt» en un disco que no distingue mayúsculas: no choca consigo mismo.
        let c = clashes(&strings(&["a.txt"]), &strings(&["A.txt"]), &strings(&["b.txt"]), true, INVALID_UNIX);
        assert_eq!(c, [None]);
        // Y con otro elemento que no se renombra llamado «A.txt» sí.
        let c = clashes(&strings(&["a.txt"]), &strings(&["B.txt"]), &strings(&["b.txt"]), true, INVALID_UNIX);
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
    }

    impl FakeRenames {
        fn new(files: &[&str], ci: bool) -> FakeRenames {
            FakeRenames { files: RefCell::new(files.iter().map(|f| f.to_string()).collect()), ci, fail_to: RefCell::new(Vec::new()) }
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
            let (f, t) = (from.to_string_lossy().into_owned(), to.to_string_lossy().into_owned());
            if self.fail_to.borrow().contains(&t) {
                return Err(format!("falla {t}"));
            }
            let mut files = self.files.borrow_mut();
            let Some(existing) = files.iter().find(|x| self.key(x) == self.key(&f)).cloned() else { return Err(format!("no existe {f}")) };
            if files.iter().any(|x| self.key(x) == self.key(&t)) {
                return Err(format!("ya existe {t}"));
            }
            files.remove(&existing);
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
        assert!(fake.names().iter().all(|n| !n.contains(".xdir-ren-")));
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
        let pid = std::process::id();
        fake.fail_to.borrow_mut().push(format!("/d/.xdir-ren-{pid}-2"));
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
    fn clash_texts() {
        assert_eq!(Clash::Exists.tip("Bariloche-3090.jpg"), "Ya existe «Bariloche-3090.jpg» en esta carpeta");
        assert_eq!(Clash::Exists.short("Bariloche-3090.jpg"), "«Bariloche-3090.jpg» ya existe");
    }
}

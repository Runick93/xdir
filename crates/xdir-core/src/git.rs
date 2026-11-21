//! Lectura de la rama actual de un repositorio git, sin lanzar procesos.

use std::path::{Path, PathBuf};

/// Estado de `HEAD`: una rama, o un commit suelto.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Head {
    /// Nombre de la rama (sin `refs/heads/`).
    Branch(String),
    /// Hash completo del commit con el que está desacoplado.
    Detached(String),
}

impl Head {
    /// Texto a mostrar: el nombre de la rama, o los 7 primeros caracteres del hash.
    pub fn label(&self) -> &str {
        match self {
            Head::Branch(n) => n,
            Head::Detached(h) => h.get(..7).unwrap_or(h),
        }
    }
}

/// Repositorio encontrado y el estado de su `HEAD`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RepoHead {
    /// Carpeta que contiene el `.git`.
    pub root: PathBuf,
    pub head: Head,
}

/// Interpreta el contenido de un archivo `HEAD`.
fn parse_head(texto: &str) -> Option<Head> {
    let linea = texto.lines().next()?.trim();
    if let Some(r) = linea.strip_prefix("ref:") {
        let nombre = r.trim().strip_prefix("refs/heads/")?;
        return (!nombre.is_empty()).then(|| Head::Branch(nombre.to_string()));
    }
    let es_hash = (linea.len() == 40 || linea.len() == 64)
        && linea.chars().all(|c| c.is_ascii_hexdigit());
    es_hash.then(|| Head::Detached(linea.to_string()))
}

/// Resuelve la carpeta de git de `raiz/.git` (carpeta, o archivo `gitdir: ...`).
fn gitdir_de(raiz: &Path) -> Option<PathBuf> {
    let punto_git = raiz.join(".git");
    if punto_git.is_dir() {
        return Some(punto_git);
    }
    if punto_git.is_file() {
        let texto = std::fs::read_to_string(&punto_git).ok()?;
        let ruta = texto.lines().next()?.trim().strip_prefix("gitdir:")?.trim();
        if ruta.is_empty() {
            return None;
        }
        let ruta = Path::new(ruta);
        return Some(if ruta.is_absolute() {
            ruta.to_path_buf()
        } else {
            raiz.join(ruta)
        });
    }
    None
}

/// Busca el repositorio que contiene `dir` (subiendo por los padres) y lee su `HEAD`.
/// Devuelve `None` si no hay repositorio o si `HEAD` no se puede leer ni entender.
pub fn read_head(dir: &Path) -> Option<RepoHead> {
    for raiz in dir.ancestors() {
        if let Some(gitdir) = gitdir_de(raiz) {
            let texto = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
            let head = parse_head(&texto)?;
            return Some(RepoHead { root: raiz.to_path_buf(), head });
        }
    }
    None
}

// ---- Comandos: se delegan al `git` de la consola ----

use std::process::{Command, Stdio};
use std::sync::OnceLock;

#[cfg(test)]
thread_local! {
    /// Fuerza el resultado de `available()` en las pruebas de este hilo.
    static DISPONIBLE_PRUEBAS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Solo para pruebas: fija (o con `None` restaura) lo que devuelve `available()` en este hilo.
#[cfg(test)]
pub fn set_available_for_tests(v: Option<bool>) {
    DISPONIBLE_PRUEBAS.with(|c| c.set(v));
}

/// Prepara un `git` sin ventana, sin preguntas por terminal y con mensajes en inglés.
fn comando(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Ejecuta `git` y devuelve la salida estándar, o el error recortado si falla.
fn ejecutar(mut cmd: Command, args: &[&str]) -> Result<String, String> {
    let out = cmd
        .args(args)
        .output()
        .map_err(|e| format!("No se pudo ejecutar git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("git terminó con {}", out.status)
        } else {
            err
        })
    }
}

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    ejecutar(comando(root), args)
}

/// Indica si hay un `git` utilizable. Se calcula una sola vez.
pub fn available() -> bool {
    #[cfg(test)]
    if let Some(v) = DISPONIBLE_PRUEBAS.with(|c| c.get()) {
        return v;
    }
    static DISPONIBLE: OnceLock<bool> = OnceLock::new();
    *DISPONIBLE.get_or_init(|| {
        let mut cmd = Command::new("git");
        cmd.arg("--version")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
}

/// Calienta `available()` en segundo plano para que el primer uso no espere.
pub fn warm_up() {
    std::thread::spawn(|| {
        available();
    });
}

/// Ramas locales, la de commit más reciente primero.
pub fn branches(root: &Path) -> Result<Vec<String>, String> {
    let salida = run(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )?;
    Ok(salida
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Resultado de intentar cambiar de rama.
#[derive(Clone, PartialEq, Debug)]
pub enum SwitchOutcome {
    Switched,
    /// Hay cambios sin guardar que chocan con la rama destino.
    Dirty { files: usize },
    Failed(String),
}

/// Cantidad de archivos con cambios sobre los que ya sigue git (ignora los nuevos sin seguimiento).
pub fn dirty_count(root: &Path) -> usize {
    run(root, &["status", "--porcelain", "-uno"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Mensaje del stash que se crea antes de pasar a `dest`.
pub fn stash_message(dest: &str) -> String {
    format!("xdir: antes de pasar a {dest}")
}

/// Cambia de rama sin tocar nada más; si hay choque con cambios locales devuelve `Dirty`.
pub fn switch(root: &Path, name: &str) -> SwitchOutcome {
    match run(root, &["switch", name]) {
        Ok(_) => SwitchOutcome::Switched,
        Err(e) => {
            if e.contains("would be overwritten") || e.contains("commit your changes or stash them")
            {
                SwitchOutcome::Dirty { files: dirty_count(root) }
            } else {
                SwitchOutcome::Failed(e)
            }
        }
    }
}

/// Guarda los cambios (incluidos los archivos nuevos) en un stash y cambia de rama.
/// Nunca recupera el stash: los cambios quedan guardados.
pub fn stash_and_switch(root: &Path, name: &str) -> SwitchOutcome {
    let mensaje = stash_message(name);
    if let Err(e) = run(root, &["stash", "push", "-u", "-m", &mensaje]) {
        return SwitchOutcome::Failed(e);
    }
    match run(root, &["switch", name]) {
        Ok(_) => SwitchOutcome::Switched,
        Err(e) => SwitchOutcome::Failed(format!(
            "{e} Tus cambios quedan en el stash ({mensaje})."
        )),
    }
}

/// Cambia de rama descartando los cambios sobre archivos con seguimiento.
/// Los archivos nuevos sin seguimiento no se tocan.
pub fn discard_and_switch(root: &Path, name: &str) -> SwitchOutcome {
    match run(root, &["switch", "--discard-changes", name]) {
        Ok(_) => SwitchOutcome::Switched,
        Err(e) => SwitchOutcome::Failed(e),
    }
}

/// Solo para pruebas: `git` con identidad fija para poder hacer commits.
#[cfg(test)]
pub(crate) fn git_test(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = comando(root);
    cmd.args(["-c", "user.name=xdir", "-c", "user.email=xdir@example.com"]);
    ejecutar(cmd, args)
}

/// Solo para pruebas: repo temporal con `main` (a.txt y b.txt) y `feature`
/// (un commit más que cambia solo a.txt). Quien lo pide debe borrar la carpeta.
/// Devuelve `None` si no hay git.
#[cfg(test)]
pub(crate) fn temp_repo(name: &str) -> Option<PathBuf> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    if !available() {
        return None;
    }
    let p = std::env::temp_dir().join(format!(
        "xdir-gitcmd-{}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        name
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).ok()?;
    let paso = |args: &[&str]| git_test(&p, args).ok();
    paso(&["init", "-b", "main"])?;
    std::fs::write(p.join("a.txt"), "main").ok()?;
    std::fs::write(p.join("b.txt"), "main").ok()?;
    paso(&["add", "a.txt", "b.txt"])?;
    paso(&["commit", "-m", "inicial"])?;
    paso(&["switch", "-c", "feature"])?;
    std::fs::write(p.join("a.txt"), "feature").ok()?;
    paso(&["commit", "-am", "feature"])?;
    paso(&["switch", "main"])?;
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Carpeta temporal única por prueba; se borra al soltarla.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(name: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("xdir-git-{}-{}", std::process::id(), name));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn escribir(ruta: &Path, texto: &str) {
        fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        fs::write(ruta, texto).unwrap();
    }

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn read_head_branch() {
        let t = Tmp::new("branch");
        escribir(&t.0.join(".git/HEAD"), "ref: refs/heads/feature/x\n");
        let r = read_head(&t.0).unwrap();
        assert_eq!(r.head, Head::Branch("feature/x".into()));
        assert_eq!(r.head.label(), "feature/x");
        assert_eq!(r.root, t.0);
    }

    #[test]
    fn read_head_detached() {
        let t = Tmp::new("detached");
        escribir(&t.0.join(".git/HEAD"), &format!("{HASH}\n"));
        let r = read_head(&t.0).unwrap();
        assert!(matches!(r.head, Head::Detached(_)));
        assert_eq!(r.head.label(), "0123456");
    }

    #[test]
    fn read_head_from_subfolder() {
        let t = Tmp::new("subfolder");
        escribir(&t.0.join(".git/HEAD"), "ref: refs/heads/main\n");
        let sub = t.0.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        let r = read_head(&sub).unwrap();
        assert_eq!(r.root, t.0);
        assert_eq!(r.head, Head::Branch("main".into()));
    }

    #[test]
    fn read_head_gitfile_relative() {
        let t = Tmp::new("gitfile-rel");
        escribir(
            &t.0.join("real/.git/worktrees/w/HEAD"),
            "ref: refs/heads/wt\n",
        );
        escribir(
            &t.0.join("wt/.git"),
            "gitdir: ../real/.git/worktrees/w\n",
        );
        let r = read_head(&t.0.join("wt")).unwrap();
        assert_eq!(r.head, Head::Branch("wt".into()));
        assert_eq!(r.root, t.0.join("wt"));
    }

    #[test]
    fn read_head_gitfile_absolute() {
        let t = Tmp::new("gitfile-abs");
        let gitdir = t.0.join("real/.git/worktrees/w");
        escribir(&gitdir.join("HEAD"), "ref: refs/heads/abs\n");
        escribir(
            &t.0.join("wt/.git"),
            &format!("gitdir: {}\n", gitdir.display()),
        );
        let r = read_head(&t.0.join("wt")).unwrap();
        assert_eq!(r.head, Head::Branch("abs".into()));
    }

    #[test]
    fn read_head_unborn_repo() {
        let t = Tmp::new("unborn");
        escribir(&t.0.join(".git/HEAD"), "ref: refs/heads/main\n");
        let r = read_head(&t.0).unwrap();
        assert_eq!(r.head, Head::Branch("main".into()));
    }

    #[test]
    fn read_head_none() {
        let t = Tmp::new("none");
        // Sin `.git` en la carpeta (los padres del temporal tampoco deberían tenerlo).
        let sin_git = t.0.join("vacia");
        fs::create_dir_all(&sin_git).unwrap();
        if read_head(&sin_git).is_none() {
            // HEAD vacío o ilegible.
            escribir(&t.0.join("r1/.git/HEAD"), "");
            assert!(read_head(&t.0.join("r1")).is_none());
            escribir(&t.0.join("r2/.git/HEAD"), "basura\n");
            assert!(read_head(&t.0.join("r2")).is_none());
            // HEAD es una carpeta: no se puede leer como texto.
            fs::create_dir_all(t.0.join("r3/.git/HEAD")).unwrap();
            assert!(read_head(&t.0.join("r3")).is_none());
        }
    }

    #[test]
    fn read_head_path_with_spaces_and_unicode() {
        let t = Tmp::new("unicode");
        let repo = t.0.join("carpeta con ñ");
        escribir(&repo.join(".git/HEAD"), "ref: refs/heads/ñandú\n");
        let r = read_head(&repo).unwrap();
        assert_eq!(r.head, Head::Branch("ñandú".into()));
        assert_eq!(r.root, repo);
    }

    // ---- Pruebas de los comandos de git (usan el git de la consola) ----

    /// Repo temporal que se borra al soltarlo.
    struct Repo(PathBuf);

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Crea el repo de prueba o avisa y devuelve `None` si no hay git.
    fn repo(name: &str) -> Option<Repo> {
        let r = temp_repo(name).map(Repo);
        if r.is_none() {
            eprintln!("git no está instalado: se salta la prueba");
        }
        r
    }

    fn git_ok(root: &Path, args: &[&str]) -> String {
        git_test(root, args).unwrap_or_else(|e| panic!("git {args:?} falló: {e}"))
    }

    fn leer(root: &Path, f: &str) -> String {
        fs::read_to_string(root.join(f)).unwrap()
    }

    #[test]
    fn branches_lists_recent_first() {
        let Some(r) = repo("ramas") else { return };
        git_ok(&r.0, &["switch", "feature"]);
        fs::write(r.0.join("c.txt"), "c").unwrap();
        git_ok(&r.0, &["add", "c.txt"]);
        git_ok(&r.0, &["commit", "-m", "nuevo"]);
        assert_eq!(branches(&r.0), Ok(vec!["feature".to_string(), "main".to_string()]));
    }

    #[test]
    fn branches_empty_repo() {
        if !available() {
            eprintln!("git no está instalado: se salta la prueba");
            return;
        }
        let p = std::env::temp_dir().join(format!("xdir-gitcmd-{}-vacio", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        let r = Repo(p);
        git_ok(&r.0, &["init", "-b", "main"]);
        assert_eq!(branches(&r.0), Ok(vec![]));
    }

    #[test]
    fn switch_clean() {
        let Some(r) = repo("limpio") else { return };
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::Switched);
        assert_eq!(read_head(&r.0).unwrap().head, Head::Branch("feature".into()));
    }

    #[test]
    fn switch_branch_with_slash() {
        let Some(r) = repo("barra") else { return };
        git_ok(&r.0, &["branch", "feat/x"]);
        assert_eq!(switch(&r.0, "feat/x"), SwitchOutcome::Switched);
        assert_eq!(read_head(&r.0).unwrap().head, Head::Branch("feat/x".into()));
    }

    #[test]
    fn switch_missing_branch() {
        let Some(r) = repo("inexistente") else { return };
        match switch(&r.0, "no-existe") {
            SwitchOutcome::Failed(t) => assert!(!t.is_empty()),
            o => panic!("se esperaba Failed, llegó {o:?}"),
        }
    }

    #[test]
    fn switch_dirty_conflict() {
        let Some(r) = repo("conflicto") else { return };
        fs::write(r.0.join("a.txt"), "local").unwrap();
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::Dirty { files: 1 });
        assert_eq!(leer(&r.0, "a.txt"), "local");
        assert_eq!(read_head(&r.0).unwrap().head, Head::Branch("main".into()));
    }

    #[test]
    fn switch_dirty_without_conflict_carries_changes() {
        let Some(r) = repo("arrastra") else { return };
        fs::write(r.0.join("b.txt"), "local").unwrap();
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::Switched);
        assert_eq!(leer(&r.0, "b.txt"), "local");
    }

    #[test]
    fn stash_and_switch_saves_changes() {
        let Some(r) = repo("stash") else { return };
        fs::write(r.0.join("a.txt"), "local").unwrap();
        fs::write(r.0.join("nuevo.txt"), "n").unwrap();
        assert_eq!(stash_and_switch(&r.0, "feature"), SwitchOutcome::Switched);
        let lista = git_ok(&r.0, &["stash", "list"]);
        assert!(lista.contains("xdir: antes de pasar a feature"), "{lista}");
        assert!(!r.0.join("nuevo.txt").exists());
        let archivos = git_ok(&r.0, &["stash", "show", "--include-untracked", "--name-only"]);
        assert!(archivos.contains("nuevo.txt"), "{archivos}");
    }

    #[test]
    fn discard_and_switch_keeps_untracked() {
        let Some(r) = repo("descartar") else { return };
        fs::write(r.0.join("a.txt"), "local").unwrap();
        fs::write(r.0.join("nuevo.txt"), "n").unwrap();
        assert_eq!(discard_and_switch(&r.0, "feature"), SwitchOutcome::Switched);
        assert_eq!(leer(&r.0, "a.txt"), "feature");
        assert!(r.0.join("nuevo.txt").exists());
    }

    #[test]
    fn dirty_count_ignores_untracked() {
        let Some(r) = repo("conteo") else { return };
        fs::write(r.0.join("a.txt"), "x").unwrap();
        fs::write(r.0.join("b.txt"), "y").unwrap();
        fs::write(r.0.join("nuevo.txt"), "n").unwrap();
        assert_eq!(dirty_count(&r.0), 2);
    }

    #[test]
    fn works_in_path_with_spaces_and_unicode() {
        let Some(r) = repo("carpeta con ñ") else { return };
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::Switched);
    }

    #[test]
    fn available_override() {
        set_available_for_tests(Some(false));
        assert!(!available());
        set_available_for_tests(Some(true));
        assert!(available());
        set_available_for_tests(None);
    }
}

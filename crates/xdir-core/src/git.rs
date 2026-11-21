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

/// El programa de git elegido por `warm_up` (`None`: no hay git). Solo lo escribe `warm_up`, que
/// corre en otro hilo; la interfaz solo lo lee.
static GIT: OnceLock<Option<&'static str>> = OnceLock::new();

/// El `git` a lanzar: el que eligió `warm_up`, o el del PATH si todavía no se eligió.
fn programa() -> &'static str {
    GIT.get().copied().flatten().unwrap_or("git")
}

/// Prepara un `git` sin ventana, sin preguntas por terminal y con mensajes en inglés.
/// `GIT_OPTIONAL_LOCKS=0` en todos: así `status` no toma `index.lock` para refrescar el índice
/// (no le gana el candado a un IDE abierto). Solo afecta a los candados opcionales: `switch` y
/// `stash` siguen tomando los que necesitan.
fn comando(root: &Path) -> Command {
    let mut cmd = Command::new(programa());
    cmd.current_dir(root)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
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

/// Lo ya calculado, sin esperar: si nadie lo calculó (o se está calculando), `false`.
fn leer_cache(cell: &OnceLock<Option<&'static str>>) -> bool {
    cell.get().is_some_and(|g| g.is_some())
}

/// Indica si hay un `git` utilizable. Nunca lanza procesos ni espera: solo lee lo que calculó
/// `warm_up`; hasta que termine, `false` (la `App` recibe `TaskDone::GitReady` al terminar
/// y las pestañas releen su rama).
pub fn available() -> bool {
    #[cfg(test)]
    if let Some(v) = DISPONIBLE_PRUEBAS.with(|c| c.get()) {
        return v;
    }
    leer_cache(&GIT)
}

/// Qué `git` usar en macOS sin lanzar el de `/usr/bin` cuando no hay herramientas de línea de
/// comandos: ahí `/usr/bin/git` es el atajo de `xcode-select`, que abre el cartel de Apple para
/// instalarlas (y sin git no se avisa de nada). `herramientas`: `xcode-select -p` funcionó;
/// `existe`: si hay un archivo en esa ruta.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn elegir_git_mac(herramientas: bool, existe: impl Fn(&str) -> bool) -> Option<&'static str> {
    if herramientas {
        return Some("git");
    }
    // Las apps abiertas desde el Finder tienen un PATH mínimo: se usa la ruta completa.
    ["/opt/homebrew/bin/git", "/usr/local/bin/git"].into_iter().find(|p| existe(p))
}

/// El programa a probar (`None`: no hay git, sin haber lanzado nada).
fn candidato() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        // `xcode-select` es una herramienta de verdad (no un atajo): `-p` solo dice dónde están
        // las herramientas, o falla si no hay, sin abrir ningún cartel.
        let herramientas = Command::new("/usr/bin/xcode-select")
            .arg("-p")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        elegir_git_mac(herramientas, |p| Path::new(p).is_file())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some("git")
    }
}

/// Prueba `git --version`. Bloquea: solo desde otro hilo.
fn probar() -> Option<&'static str> {
    let prog = candidato()?;
    let mut cmd = Command::new(prog);
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
    cmd.status().is_ok_and(|s| s.success()).then_some(prog)
}

/// Averigua (una sola vez) si hay git y devuelve si lo hay. Lanza `git --version` y lo espera:
/// nunca desde el hilo de la interfaz (la `App` lo corre con `spawn_task`).
pub fn warm_up() -> bool {
    GIT.get_or_init(probar).is_some()
}

/// Ramas locales, la de commit más reciente primero.
pub fn branches(root: &Path) -> Result<Vec<String>, String> {
    let salida = run(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            // `lstrip=2` y no `short`: con una etiqueta del mismo nombre, `short` da `heads/v1`.
            "--format=%(refname:lstrip=2)",
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
    /// Hay archivos nuevos (sin seguimiento) que la rama destino sobrescribiría.
    UntrackedInTheWay,
    Failed(String),
}

/// Cantidad de archivos con cambios sobre los que ya sigue git (ignora los nuevos sin seguimiento).
pub fn dirty_count(root: &Path) -> usize {
    run(root, &["status", "--porcelain", "-uno"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Cuántos archivos tienen cambios sin commitear (modificados, borrados, nuevos sin seguimiento);
/// `None` si git no pudo decirlo.
pub fn changes_count(root: &Path) -> Option<usize> {
    run(root, &["status", "--porcelain"]).ok().map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
}

/// Mensaje del stash que se crea antes de pasar a `dest`.
pub fn stash_message(dest: &str) -> String {
    format!("xdir: antes de pasar a {dest}")
}

/// Qué quiso decir git al negarse a cambiar (los mensajes salen en inglés: `LC_ALL=C`).
/// Lo de los archivos sin seguimiento va antes: su mensaje también dice «would be overwritten».
fn clasificar(root: &Path, e: String) -> SwitchOutcome {
    // `switch` dice «The following untracked working tree files…»; `--discard-changes`,
    // «Untracked working tree file 'x'…».
    if e.to_ascii_lowercase().contains("untracked working tree file") {
        SwitchOutcome::UntrackedInTheWay
    } else if e.contains("would be overwritten") || e.contains("commit your changes or stash them") {
        SwitchOutcome::Dirty { files: dirty_count(root) }
    } else {
        SwitchOutcome::Failed(e)
    }
}

/// Cambia de rama sin tocar nada más; si hay choque con cambios locales devuelve `Dirty`, y si
/// lo que estorba son archivos nuevos, `UntrackedInTheWay`.
pub fn switch(root: &Path, name: &str) -> SwitchOutcome {
    match run(root, &["switch", name]) {
        Ok(_) => SwitchOutcome::Switched,
        Err(e) => clasificar(root, e),
    }
}

/// Guarda los cambios (incluidos los archivos nuevos) en un stash y cambia de rama.
/// Nunca recupera el stash: los cambios quedan guardados. No devuelve `UntrackedInTheWay`: el
/// stash (`-u`) ya se llevó los archivos nuevos, y si el cambio igual falla el aviso tiene que
/// decir que los cambios quedaron en el stash.
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
/// Los archivos nuevos sin seguimiento no se tocan: si estorban, `UntrackedInTheWay`.
pub fn discard_and_switch(root: &Path, name: &str) -> SwitchOutcome {
    match run(root, &["switch", "--discard-changes", name]) {
        Ok(_) => SwitchOutcome::Switched,
        Err(e) => clasificar(root, e),
    }
}

/// Cuánto se adelantó y cuánto se atrasó la rama actual respecto de la remota que sigue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Upstream {
    /// Commits locales que el remoto no tiene (para un `push`).
    pub ahead: usize,
    /// Commits del remoto que faltan acá (para un `pull`).
    pub behind: usize,
}

/// Lo que el botón de sincronizar de una pestaña tiene que mostrar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SyncView {
    /// Commits que hay para bajar: 0 es «al día» (verde), más es «hay para bajar» (amarillo).
    pub behind: usize,
    /// Gira mientras se baja o se consulta el remoto.
    pub spinning: bool,
}

/// Cuánto adelanta o atrasa la rama actual respecto de su remota, según lo último que se trajo
/// (no consulta la red). `None` si la rama no sigue a ninguna remota o `HEAD` está suelto.
pub fn upstream_state(root: &Path) -> Option<Upstream> {
    let out = run(root, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"]).ok()?;
    let mut n = out.split_whitespace();
    Some(Upstream { ahead: n.next()?.parse().ok()?, behind: n.next()?.parse().ok()? })
}

/// El nombre del remoto que sigue la rama actual (`origin`, casi siempre); `None` si no sigue a ninguno.
pub fn upstream_remote(root: &Path) -> Option<String> {
    let head = run(root, &["symbolic-ref", "-q", "HEAD"]).ok()?;
    let out = run(root, &["for-each-ref", "--format=%(upstream:remotename)", head.trim()]).ok()?;
    Some(out.trim().to_string()).filter(|s| !s.is_empty())
}

/// Trae lo nuevo del remoto sin tocar la copia de trabajo. Corre en segundo plano, así que pide
/// que el administrador de credenciales no abra ventanas (`GCM_INTERACTIVE=never`): si hace falta
/// contraseña, falla en silencio.
pub fn fetch(root: &Path) -> Result<(), String> {
    ejecutar(comando_fetch(root), &["fetch", "--quiet"]).map(|_| ())
}

/// El `git` de `fetch`: el de siempre, más el pedido de no abrir ventanas de credenciales.
fn comando_fetch(root: &Path) -> Command {
    let mut cmd = comando(root);
    cmd.env("GCM_INTERACTIVE", "never");
    cmd
}

/// Baja lo nuevo avanzando la rama actual. Solo avanza en línea recta (`--ff-only`): nunca crea
/// un commit de fusión ni deja conflictos a medias; si no se puede, git lo dice y no toca nada.
pub fn pull(root: &Path) -> Result<(), String> {
    ejecutar(comando(root), &["pull", "--ff-only", "--quiet"]).map(|_| ())
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
    // Las pruebas averiguan si hay git en su hilo (la interfaz nunca lo hace).
    if !warm_up() {
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

/// Solo para pruebas: un remoto de verdad en la carpeta temporal. Devuelve `base` con
/// `origin.git` (sin copia de trabajo), `work` (clon con `main` ya subido y siguiendo a `origin/main`) y
/// `other` (otro clon, para empujar cambios que `work` todavía no vio). Quien lo pide borra `base`.
/// Devuelve `None` si no hay git.
#[cfg(test)]
pub(crate) fn temp_remote(name: &str) -> Option<PathBuf> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    if !warm_up() {
        return None;
    }
    let base = std::env::temp_dir().join(format!("xdir-gitremote-{}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), name));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).ok()?;
    git_test(&base, &["init", "--bare", "-b", "main", "origin.git"]).ok()?;
    git_test(&base, &["clone", "origin.git", "work"]).ok()?;
    let work = base.join("work");
    git_test(&work, &["checkout", "-B", "main"]).ok()?;
    std::fs::write(work.join("a.txt"), "uno").ok()?;
    git_test(&work, &["add", "a.txt"]).ok()?;
    git_test(&work, &["commit", "-m", "inicial"]).ok()?;
    git_test(&work, &["push", "-u", "origin", "main"]).ok()?;
    git_test(&base, &["clone", "origin.git", "other"]).ok()?;
    Some(base)
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
        if !warm_up() {
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

    /// Una rama y una etiqueta con el mismo nombre: la lista dice `v1` (no `heads/v1`) y se
    /// puede cambiar a ella.
    #[test]
    fn branch_and_tag_with_same_name() {
        let Some(r) = repo("rama-etiqueta") else { return };
        git_ok(&r.0, &["branch", "v1"]);
        git_ok(&r.0, &["tag", "v1"]);
        let mut lista = branches(&r.0).unwrap();
        lista.sort();
        assert_eq!(lista, vec!["feature".to_string(), "main".to_string(), "v1".to_string()]);
        assert_eq!(switch(&r.0, "v1"), SwitchOutcome::Switched);
        assert_eq!(read_head(&r.0).unwrap().head, Head::Branch("v1".into()));
    }

    /// `feature` gana `n.txt`; en `main` hay un `n.txt` nuevo sin seguimiento que lo tapa.
    fn untracked_in_the_way(r: &Repo) {
        git_ok(&r.0, &["switch", "feature"]);
        fs::write(r.0.join("n.txt"), "feature").unwrap();
        git_ok(&r.0, &["add", "n.txt"]);
        git_ok(&r.0, &["commit", "-m", "n"]);
        git_ok(&r.0, &["switch", "main"]);
        fs::write(r.0.join("n.txt"), "local").unwrap();
    }

    #[test]
    fn switch_blocked_by_untracked_file() {
        let Some(r) = repo("sin-seguimiento") else { return };
        untracked_in_the_way(&r);
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::UntrackedInTheWay);
        assert_eq!(leer(&r.0, "n.txt"), "local");
        assert_eq!(read_head(&r.0).unwrap().head, Head::Branch("main".into()));
    }

    /// Cambios con seguimiento que no chocan y un archivo nuevo que sí: es lo del archivo nuevo.
    #[test]
    fn switch_mixed_untracked_and_tracked_changes() {
        let Some(r) = repo("mezcla") else { return };
        untracked_in_the_way(&r);
        fs::write(r.0.join("b.txt"), "local").unwrap();
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::UntrackedInTheWay);
    }

    /// Descartar no toca los archivos sin seguimiento: sigue sin poder cambiar.
    #[test]
    fn discard_blocked_by_untracked_file() {
        let Some(r) = repo("descartar-nuevo") else { return };
        untracked_in_the_way(&r);
        fs::write(r.0.join("a.txt"), "local").unwrap();
        assert_eq!(discard_and_switch(&r.0, "feature"), SwitchOutcome::UntrackedInTheWay);
        assert_eq!(leer(&r.0, "n.txt"), "local");
    }

    /// Guardar en el stash (`-u`) se lleva el archivo que tapaba: cambia.
    #[test]
    fn stash_takes_untracked_in_the_way() {
        let Some(r) = repo("stash-nuevo") else { return };
        untracked_in_the_way(&r);
        assert_eq!(stash_and_switch(&r.0, "feature"), SwitchOutcome::Switched);
        assert_eq!(leer(&r.0, "n.txt"), "feature");
    }

    #[test]
    fn works_in_path_with_spaces_and_unicode() {
        let Some(r) = repo("carpeta con ñ") else { return };
        assert_eq!(switch(&r.0, "feature"), SwitchOutcome::Switched);
    }

    /// `available()` solo lee lo ya calculado: mientras la prueba de `git --version` sigue en
    /// curso (en otro hilo) contesta `false` enseguida, sin esperarla.
    #[test]
    fn available_never_waits_for_probe() {
        use std::sync::mpsc::channel;
        use std::time::{Duration, Instant};
        let cell: &'static OnceLock<Option<&'static str>> = Box::leak(Box::new(OnceLock::new()));
        assert!(!leer_cache(cell), "sin calcular: false");
        let (empezo_tx, empezo_rx) = channel();
        let (seguir_tx, seguir_rx) = channel::<()>();
        let h = std::thread::spawn(move || {
            cell.get_or_init(|| {
                empezo_tx.send(()).unwrap();
                let _ = seguir_rx.recv();
                Some("git")
            });
        });
        empezo_rx.recv().unwrap();
        let t0 = Instant::now();
        assert!(!leer_cache(cell), "calculándose: false");
        assert!(t0.elapsed() < Duration::from_secs(1));
        seguir_tx.send(()).unwrap();
        h.join().unwrap();
        assert!(leer_cache(cell));
    }

    #[test]
    fn mac_git_choice_never_runs_the_shim() {
        // Con las herramientas de línea de comandos (o Xcode), el `git` del PATH.
        assert_eq!(elegir_git_mac(true, |_| false), Some("git"));
        // Sin ellas, solo un git de verdad fuera de /usr/bin (el de /usr/bin es el instalador).
        assert_eq!(elegir_git_mac(false, |p| p == "/opt/homebrew/bin/git"), Some("/opt/homebrew/bin/git"));
        assert_eq!(elegir_git_mac(false, |p| p == "/usr/local/bin/git"), Some("/usr/local/bin/git"));
        assert_eq!(elegir_git_mac(false, |p| p == "/usr/bin/git"), None);
        assert_eq!(elegir_git_mac(false, |_| false), None);
    }

    #[test]
    fn commands_skip_optional_locks() {
        let cmd = comando(Path::new("."));
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(
            envs.contains(&(std::ffi::OsStr::new("GIT_OPTIONAL_LOCKS"), Some(std::ffi::OsStr::new("0")))),
            "{envs:?}"
        );
    }

    // ---- sincronizar con el remoto ----

    /// Remoto temporal (ver `temp_remote`) que se borra al soltarlo.
    struct Remote(PathBuf);

    impl Remote {
        fn work(&self) -> PathBuf {
            self.0.join("work")
        }
        fn other(&self) -> PathBuf {
            self.0.join("other")
        }
        /// Un commit nuevo en `other` que se sube al remoto, sin que `work` se entere.
        fn push_from_other(&self, file: &str) {
            fs::write(self.other().join(file), "otro").unwrap();
            git_test(&self.other(), &["add", file]).unwrap();
            git_test(&self.other(), &["commit", "-m", file]).unwrap();
            git_test(&self.other(), &["push"]).unwrap();
        }
    }

    impl Drop for Remote {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn remote(name: &str) -> Option<Remote> {
        let r = temp_remote(name).map(Remote);
        if r.is_none() {
            eprintln!("git no está instalado: se salta la prueba");
        }
        r
    }

    #[test]
    fn upstream_remote_names_the_followed_remote() {
        let Some(r) = repo("up-nombre-sin") else { return };
        assert_eq!(upstream_remote(&r.0), None);
        let Some(r) = remote("up-nombre") else { return };
        assert_eq!(upstream_remote(&r.work()).as_deref(), Some("origin"));
    }

    #[test]
    fn upstream_is_none_without_remote() {
        let Some(r) = repo("up-sin-remoto") else { return };
        assert_eq!(upstream_state(&r.0), None);
    }

    #[test]
    fn upstream_up_to_date_after_push() {
        let Some(r) = remote("up-al-dia") else { return };
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 0, behind: 0 }));
    }

    #[test]
    fn upstream_does_not_see_the_remote_until_fetch() {
        let Some(r) = remote("up-fetch") else { return };
        r.push_from_other("b.txt");
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 0, behind: 0 }), "sin traer, lo local no cambia");
        fetch(&r.work()).unwrap();
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 0, behind: 1 }));
        assert!(!r.work().join("b.txt").exists(), "fetch no toca la copia de trabajo");
    }

    #[test]
    fn upstream_counts_ahead() {
        let Some(r) = remote("up-adelante") else { return };
        fs::write(r.work().join("c.txt"), "c").unwrap();
        git_test(&r.work(), &["add", "c.txt"]).unwrap();
        git_test(&r.work(), &["commit", "-m", "c"]).unwrap();
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 1, behind: 0 }));
    }

    #[test]
    fn upstream_is_none_with_detached_head() {
        let Some(r) = remote("up-suelto") else { return };
        git_test(&r.work(), &["switch", "--detach"]).unwrap();
        assert_eq!(upstream_state(&r.work()), None);
    }

    #[test]
    fn pull_fast_forwards_and_brings_the_files() {
        let Some(r) = remote("pull-ff") else { return };
        r.push_from_other("b.txt");
        fetch(&r.work()).unwrap();
        pull(&r.work()).unwrap();
        assert_eq!(fs::read_to_string(r.work().join("b.txt")).unwrap(), "otro");
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 0, behind: 0 }));
    }

    #[test]
    fn pull_refuses_to_merge_diverged_branches() {
        let Some(r) = remote("pull-diverge") else { return };
        fs::write(r.work().join("c.txt"), "c").unwrap();
        git_test(&r.work(), &["add", "c.txt"]).unwrap();
        git_test(&r.work(), &["commit", "-m", "c"]).unwrap();
        r.push_from_other("b.txt");
        fetch(&r.work()).unwrap();
        let before = git_test(&r.work(), &["rev-parse", "HEAD"]).unwrap();
        let err = pull(&r.work()).unwrap_err();
        assert!(err.to_lowercase().contains("fast-forward"), "{err}");
        assert_eq!(git_test(&r.work(), &["rev-parse", "HEAD"]).unwrap(), before, "no se tocó nada");
        assert!(!r.work().join("b.txt").exists());
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 1, behind: 1 }));
    }

    #[test]
    fn fetch_fails_quietly_without_the_remote() {
        let Some(r) = remote("fetch-sin-remoto") else { return };
        fs::remove_dir_all(r.0.join("origin.git")).unwrap();
        let err = fetch(&r.work()).unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(upstream_state(&r.work()), Some(Upstream { ahead: 0, behind: 0 }), "se queda con lo último que se sabía");
    }

    #[test]
    fn background_fetch_never_asks_for_credentials_in_a_window() {
        // El pull lo pide la persona: ese sí puede abrir la ventana del administrador de credenciales.
        let pull_cmd = comando(Path::new("."));
        assert!(!pull_cmd.get_envs().any(|(k, _)| k == std::ffi::OsStr::new("GCM_INTERACTIVE")));
        let cmd = comando_fetch(Path::new("."));
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&(std::ffi::OsStr::new("GCM_INTERACTIVE"), Some(std::ffi::OsStr::new("never")))));
        assert!(envs.contains(&(std::ffi::OsStr::new("GIT_TERMINAL_PROMPT"), Some(std::ffi::OsStr::new("0")))));
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

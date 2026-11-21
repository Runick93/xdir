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
}

//! Nombres de los elementos que crean las operaciones («informe copia.pdf», «informe 2.pdf»…),
//! iguales en los tres sistemas. Funciones puras: quien llama dice qué nombres están tomados.

use std::path::{Path, PathBuf};

/// Nombre y extensión (con el punto). Las carpetas y los nombres que empiezan con punto no tienen
/// extensión; solo cuenta la última («a.tar.gz» → («a.tar», «.gz»)).
pub fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

/// «informe copia.pdf» (n = 1), «informe copia 2.pdf» (n ≥ 2): el nombre de Duplicar.
pub fn copy_name(name: &str, is_dir: bool, n: u32) -> String {
    let (stem, ext) = split_ext(name, is_dir);
    if n <= 1 { format!("{stem} copia{ext}") } else { format!("{stem} copia {n}{ext}") }
}

/// El mismo nombre (n = 1) o «informe 2.pdf» (n ≥ 2).
pub fn numbered_name(name: &str, is_dir: bool, n: u32) -> String {
    if n <= 1 {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name, is_dir);
    format!("{stem} {n}{ext}")
}

/// Primer `dir/make(n)` (n = 1, 2, …) que no está tomado según `taken` (en las plataformas,
/// `symlink_metadata().is_ok()`).
pub fn free_name(dir: &Path, make: &dyn Fn(u32) -> String, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let mut n = 1;
    loop {
        let candidate = dir.join(make(n));
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn copy_names() {
        assert_eq!(copy_name("informe.pdf", false, 1), "informe copia.pdf");
        assert_eq!(copy_name("informe.pdf", false, 3), "informe copia 3.pdf");
        assert_eq!(copy_name("Fotos.2024", true, 1), "Fotos.2024 copia");
        assert_eq!(copy_name(".bashrc", false, 1), ".bashrc copia");
        assert_eq!(copy_name("a.tar.gz", false, 1), "a.tar copia.gz");
        assert_eq!(split_ext("a.tar.gz", false), ("a.tar", ".gz"));
        assert_eq!(split_ext("Fotos.2024", true), ("Fotos.2024", ""));
        assert_eq!(split_ext(".bashrc", false), (".bashrc", ""));
        assert_eq!(split_ext("sin", false), ("sin", ""));
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered_name("informe.pdf", false, 1), "informe.pdf");
        assert_eq!(numbered_name("informe.pdf", false, 2), "informe 2.pdf");
        assert_eq!(numbered_name("Fotos.2024", true, 3), "Fotos.2024 3");
    }

    #[test]
    fn free_name_skips_taken() {
        let dir = Path::new("/d");
        let taken: HashSet<PathBuf> = [dir.join("a copia.txt"), dir.join("a copia 2.txt")].into();
        let got = free_name(dir, &|n| copy_name("a.txt", false, n), &|p| taken.contains(p));
        assert_eq!(got, dir.join("a copia 3.txt"));
        let none: HashSet<PathBuf> = HashSet::new();
        assert_eq!(free_name(dir, &|n| numbered_name("a.txt", false, n), &|p| none.contains(p)), dir.join("a.txt"));
    }
}

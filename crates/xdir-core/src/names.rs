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

/// Lo apartado por «Reemplazar»: «informe (reemplazado).pdf» (n = 1), «informe (reemplazado
/// 2).pdf» (n ≥ 2). A la vista y reconocible: si xdir se corta antes de tirarlo, queda ahí.
pub fn replaced_name(name: &str, is_dir: bool, n: u32) -> String {
    let (stem, ext) = split_ext(name, is_dir);
    if n <= 1 { format!("{stem} (reemplazado){ext}") } else { format!("{stem} (reemplazado {n}){ext}") }
}

/// Cómo hace cada sistema sus accesos directos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStyle {
    /// Windows: `.lnk` («informe.pdf - Acceso directo.lnk»).
    Lnk,
    /// macOS: alias del Finder («informe.pdf alias»).
    Alias,
    /// Linux: enlace simbólico («Enlace a informe.pdf»).
    Symlink,
}

/// Nombre del acceso directo `n` (1, 2, …) a `name`, como lo nombra cada sistema.
pub fn shortcut_name(name: &str, style: LinkStyle, n: u32) -> String {
    match (style, n) {
        (LinkStyle::Lnk, 0 | 1) => format!("{name} - Acceso directo.lnk"),
        (LinkStyle::Lnk, n) => format!("{name} - Acceso directo ({n}).lnk"),
        (LinkStyle::Alias, 0 | 1) => format!("{name} alias"),
        (LinkStyle::Alias, n) => format!("{name} alias {n}"),
        (LinkStyle::Symlink, 0 | 1) => format!("Enlace a {name}"),
        (LinkStyle::Symlink, n) => format!("Enlace a {name} {n}"),
    }
}

/// «Carpeta nueva con 3 elementos» («con 1 elemento»; si existe, `numbered_name` le suma « 2»…).
pub fn folder_with_name(count: usize) -> String {
    if count == 1 { "Carpeta nueva con 1 elemento".to_string() } else { format!("Carpeta nueva con {count} elementos") }
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
    fn replaced_names() {
        assert_eq!(replaced_name("informe.pdf", false, 1), "informe (reemplazado).pdf");
        assert_eq!(replaced_name("informe.pdf", false, 2), "informe (reemplazado 2).pdf");
        assert_eq!(replaced_name("Fotos.2024", true, 1), "Fotos.2024 (reemplazado)");
    }

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
    fn folder_with_name_text() {
        assert_eq!(folder_with_name(3), "Carpeta nueva con 3 elementos");
        assert_eq!(folder_with_name(1), "Carpeta nueva con 1 elemento");
        assert_eq!(numbered_name(&folder_with_name(3), true, 2), "Carpeta nueva con 3 elementos 2");
    }

    #[test]
    fn shortcut_names() {
        assert_eq!(shortcut_name("informe.pdf", LinkStyle::Lnk, 1), "informe.pdf - Acceso directo.lnk");
        assert_eq!(shortcut_name("informe.pdf", LinkStyle::Lnk, 2), "informe.pdf - Acceso directo (2).lnk");
        assert_eq!(shortcut_name("informe.pdf", LinkStyle::Alias, 1), "informe.pdf alias");
        assert_eq!(shortcut_name("informe.pdf", LinkStyle::Alias, 2), "informe.pdf alias 2");
        assert_eq!(shortcut_name("informe.pdf", LinkStyle::Symlink, 1), "Enlace a informe.pdf");
        assert_eq!(shortcut_name("informe.pdf", LinkStyle::Symlink, 2), "Enlace a informe.pdf 2");
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

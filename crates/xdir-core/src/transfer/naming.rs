//! Nombres que necesita el motor («copia», « 2», «Carpeta nueva con N elementos»).
//!
//! TODO(merge tanda D): son los de `crate::names` (Tasks 1–2, en otra rama); al juntar las ramas
//! se borra este archivo y el motor usa `crate::names::{copy_name, numbered_name, free_name,
//! folder_with_name}` (mismas firmas y reglas).

use std::path::{Path, PathBuf};

/// (nombre, extensión con el punto); carpetas y nombres que empiezan con punto no tienen
/// extensión; solo la última («a.tar.gz» → («a.tar», «.gz»)).
pub fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

/// n = 1 «informe copia.pdf»; n ≥ 2 «informe copia 2.pdf».
pub fn copy_name(name: &str, is_dir: bool, n: u32) -> String {
    let (stem, ext) = split_ext(name, is_dir);
    if n <= 1 { format!("{stem} copia{ext}") } else { format!("{stem} copia {n}{ext}") }
}

/// n = 1 el mismo; n ≥ 2 «informe 2.pdf».
pub fn numbered_name(name: &str, is_dir: bool, n: u32) -> String {
    if n <= 1 {
        return name.to_string();
    }
    let (stem, ext) = split_ext(name, is_dir);
    format!("{stem} {n}{ext}")
}

/// La primera ruta libre `dir/make(n)` con n = 1, 2, …
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

/// «Carpeta nueva con 3 elementos».
pub fn folder_with_name(count: usize) -> String {
    if count == 1 { "Carpeta nueva con 1 elemento".into() } else { format!("Carpeta nueva con {count} elementos") }
}

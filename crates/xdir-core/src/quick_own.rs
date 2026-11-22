//! Acceso rápido propio de xdir, para los sistemas que no lo tienen (Mac y Linux): las carpetas
//! ancladas van en `acceso_rapido.txt` (carpeta de configuración, una ruta por línea) y se ven en
//! el árbol como el Acceso rápido de Windows. Cada plataforma lo conecta con sus funciones del
//! Acceso rápido (`FileSystem::quick_access`, `is_quick_pinned`, `quick_access_watch_dir` y
//! `Shell::pin_quick_access` / `unpin_quick_access`); así el árbol, los menús, la chincheta y
//! soltar entre lugares funcionan igual en los tres sistemas.

use std::path::{Path, PathBuf};

use crate::fs::QuickItem;
use crate::session;

pub const FILE_NAME: &str = "acceso_rapido.txt";

const HEADER: &str = "# Acceso rápido de xdir: una carpeta por línea, en el orden del árbol.\n\
                      # Lo escribe xdir (Anclar a Acceso rápido); se puede editar a mano.\n";

/// Las carpetas ancladas en `dir` (las líneas vacías, los comentarios y las rutas relativas no
/// cuentan). Bloqueante.
pub fn read_from(dir: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE_NAME)) else { return Vec::new() };
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(PathBuf::from).filter(|p| p.is_absolute()).collect()
}

/// Escribe la lista en `dir` (temporal propio y renombrar, para no dejarla a medias).
pub fn write_in(dir: &Path, paths: &[PathBuf]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut text = HEADER.to_string();
    for p in paths {
        text.push_str(&p.to_string_lossy());
        text.push('\n');
    }
    let tmp = dir.join(format!("{FILE_NAME}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(FILE_NAME)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// El nombre que se ve: el último tramo (o la ruta entera, como «/»).
fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// Lo anclado, para el árbol (todas ancladas: no hay «frecuentes»).
pub fn items() -> Vec<QuickItem> {
    let Some(dir) = session::config_dir() else { return Vec::new() };
    read_from(&dir).into_iter().map(|path| QuickItem { name: name_of(&path), path, pinned: true }).collect()
}

/// ¿Está anclada? (compara como el sistema: sin distinguir mayúsculas donde corresponde).
pub fn is_pinned(path: &Path) -> bool {
    let fs = crate::sys::file_system();
    session::config_dir().is_some_and(|dir| read_from(&dir).iter().any(|p| fs.same_path(p, path)))
}

/// Ancla `path` al final (si ya estaba, no hace nada).
pub fn pin(path: &Path) -> Result<(), String> {
    let dir = session::config_dir().ok_or("no hay carpeta de configuración")?;
    let fs = crate::sys::file_system();
    let mut paths = read_from(&dir);
    if paths.iter().any(|p| fs.same_path(p, path)) {
        return Ok(());
    }
    paths.push(path.to_path_buf());
    write_in(&dir, &paths).map_err(|e| e.to_string())
}

/// Desancla `path` (si no estaba, no hace nada).
pub fn unpin(path: &Path) -> Result<(), String> {
    let dir = session::config_dir().ok_or("no hay carpeta de configuración")?;
    let fs = crate::sys::file_system();
    let mut paths = read_from(&dir);
    let before = paths.len();
    paths.retain(|p| !fs.same_path(p, path));
    if paths.len() == before {
        return Ok(());
    }
    write_in(&dir, &paths).map_err(|e| e.to_string())
}

/// La carpeta que se vigila para releer la lista (la de configuración; se crea si falta, así la
/// vigilancia arranca aunque todavía no se haya anclado nada).
pub fn watch_dir() -> Option<PathBuf> {
    let dir = session::config_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::install_fakes;

    /// Anclar agrega al final sin repetir, desanclar quita, y la lista se lee igual que se escribió;
    /// lo que no es una ruta absoluta se ignora.
    #[test]
    fn quick_own_pin_unpin_roundtrip() {
        install_fakes();
        let dir = session::config_dir().unwrap();
        let _ = std::fs::remove_file(dir.join(FILE_NAME));
        assert!(items().is_empty());
        let (a, b) = (std::env::temp_dir().join("xdir-ar-a"), std::env::temp_dir().join("xdir-ar-b"));
        pin(&a).unwrap();
        pin(&b).unwrap();
        pin(&a).unwrap();
        let got: Vec<PathBuf> = items().into_iter().map(|i| i.path).collect();
        assert_eq!(got, [a.clone(), b.clone()]);
        assert!(items().iter().all(|i| i.pinned));
        assert_eq!(items()[0].name, "xdir-ar-a");
        assert!(is_pinned(&a) && is_pinned(&b));
        unpin(&a).unwrap();
        assert!(!is_pinned(&a));
        assert_eq!(read_from(&dir), [b.clone()]);
        // Editado a mano: comentarios, vacías y relativas no cuentan.
        std::fs::write(dir.join(FILE_NAME), format!("# hola\n\nrelativa\n{}\n", b.display())).unwrap();
        assert_eq!(read_from(&dir), [b]);
        assert_eq!(watch_dir(), Some(dir.clone()));
        let _ = std::fs::remove_file(dir.join(FILE_NAME));
    }
}

//! Reglas puras de archivos comprimidos: formatos, nombres y seguridad de las rutas de adentro.
//! La lectura y escritura de zip y tar vive en la crate `xdir-archive`.

use std::path::PathBuf;

/// Formatos que sabe tratar la crate `xdir-archive`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

/// Formato según la extensión (sin distinguir mayúsculas): `.zip`, `.tar.gz` o `.tgz`.
pub fn format_of(name: &str) -> Option<ArchiveFormat> {
    let lower = name.to_lowercase();
    if lower.ends_with(".zip") {
        Some(ArchiveFormat::Zip)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else {
        None
    }
}

fn has_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Ruta relativa y segura de una entrada, o `None` si es absoluta, con unidad o UNC, con `..`
/// que sale de la carpeta, con un byte nulo o vacía. `\` cuenta como separador; `.` y los
/// separadores repetidos se descartan.
pub fn safe_entry_path(name: &str) -> Option<PathBuf> {
    if name.contains('\0') {
        return None;
    }
    let n = name.replace('\\', "/");
    if n.starts_with('/') || has_drive(&n) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for c in n.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.iter().collect())
}

/// ¿El destino de un enlace simbólico (relativo a la carpeta de `entry`) queda dentro de la
/// carpeta de extracción? Las rutas absolutas, con unidad o vacías no.
pub fn symlink_inside(entry: &std::path::Path, target: &str) -> bool {
    let t = target.replace('\\', "/");
    if t.is_empty() || t.contains('\0') || t.starts_with('/') || has_drive(&t) {
        return false;
    }
    let mut depth = entry.components().count().saturating_sub(1) as i64;
    for c in t.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    true
}

/// Nombre del zip al comprimir: uno solo → «informe.pdf.zip» / «Fotos.zip» (como el Finder);
/// varios → «Archivo.zip».
pub fn archive_name(names: &[&str]) -> String {
    match names {
        [one] => format!("{one}.zip"),
        _ => "Archivo.zip".to_string(),
    }
}

/// Carpeta donde se extrae: «fotos.zip» → «fotos», «a.tar.gz» → «a». Si no tiene extensión
/// conocida queda igual.
pub fn extract_folder_name(archive: &str) -> String {
    let lower = archive.to_lowercase();
    for ext in [".tar.gz", ".tgz", ".zip"] {
        if lower.ends_with(ext) {
            let cut = archive.len() - ext.len();
            if cut > 0 {
                return archive[..cut].to_string();
            }
        }
    }
    archive.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn safe_entry_rejects_escapes() {
        for bad in ["/etc/x", "C:\\x", "c:/x", "\\\\srv\\x", "a/../../x", "..", "", ".", "a\\..\\..\\x", "a\0b"] {
            assert_eq!(safe_entry_path(bad), None, "{bad:?}");
        }
        assert_eq!(safe_entry_path("a/./b"), Some(PathBuf::from("a/b")));
        assert_eq!(safe_entry_path("a/b/../c"), Some(PathBuf::from("a/c")));
        assert_eq!(safe_entry_path("a\\b"), Some(PathBuf::from("a/b")));
        assert_eq!(safe_entry_path("dir/"), Some(PathBuf::from("dir")));
    }

    #[test]
    fn symlink_inside_only() {
        let e = Path::new("a/b/link");
        assert!(symlink_inside(e, "x"));
        assert!(symlink_inside(e, "../c"));
        assert!(symlink_inside(e, "../../c"));
        assert!(!symlink_inside(e, "../../../c"));
        assert!(!symlink_inside(e, "/etc/passwd"));
        assert!(!symlink_inside(e, "C:\\x"));
        assert!(!symlink_inside(e, ""));
        assert!(!symlink_inside(Path::new("link"), ".."));
        assert!(symlink_inside(Path::new("link"), "a/../b"));
    }

    #[test]
    fn archive_and_folder_names() {
        assert_eq!(archive_name(&["informe.pdf"]), "informe.pdf.zip");
        assert_eq!(archive_name(&["Fotos"]), "Fotos.zip");
        assert_eq!(archive_name(&["a", "b"]), "Archivo.zip");
        assert_eq!(extract_folder_name("fotos.zip"), "fotos");
        assert_eq!(extract_folder_name("a.tar.gz"), "a");
        assert_eq!(extract_folder_name("B.TGZ"), "B");
        assert_eq!(extract_folder_name("x.rar"), "x.rar");
    }

    #[test]
    fn format_detection() {
        assert_eq!(format_of("a.ZIP"), Some(ArchiveFormat::Zip));
        assert_eq!(format_of("a.tar.gz"), Some(ArchiveFormat::TarGz));
        assert_eq!(format_of("a.TGZ"), Some(ArchiveFormat::TarGz));
        assert_eq!(format_of("a.tar"), None);
        assert_eq!(format_of("zip"), None);
    }
}

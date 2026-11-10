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

/// Nombres de dispositivo de Windows: no se pueden usar como archivo, con o sin extensión.
fn is_reserved(c: &str) -> bool {
    let stem = c.split('.').next().unwrap_or("").trim_end_matches(' ').to_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let n = stem.chars().count();
    n == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.chars().last().is_some_and(|d| matches!(d, '1'..='9' | '¹' | '²' | '³'))
}

/// Componentes de la ruta sin `.` ni separadores repetidos, o `None` si es insegura (absoluta,
/// con unidad, UNC, `..` que sale, byte nulo, `:`, solo puntos y espacios, nombre reservado).
/// Puede devolver una lista vacía: la entrada es la propia carpeta de extracción.
fn sanitize(name: &str) -> Option<Vec<String>> {
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
            // «C:x», «C:/x» y «a:flujo» son unidades o flujos alternativos en Windows; Windows
            // también ignora los puntos y espacios del final («...» o «. » serían «» o «..»).
            c if c.contains(':') || c.trim_end_matches(['.', ' ']).is_empty() || is_reserved(c) => return None,
            c => parts.push(c),
        }
    }
    Some(parts.into_iter().map(String::from).collect())
}

/// Ruta relativa y segura de una entrada, o `None` si es absoluta, con unidad o UNC, con `..`
/// que sale de la carpeta, con un byte nulo o vacía; también si algún componente tiene `:`, está
/// hecho solo de puntos y espacios o es un nombre de dispositivo de Windows (CON, NUL, COM1…),
/// igual en todos los sistemas para que un archivo se comporte igual en todos. `\` cuenta como
/// separador; `.` y los separadores repetidos se descartan.
pub fn safe_entry_path(name: &str) -> Option<PathBuf> {
    let parts = sanitize(name)?;
    if parts.is_empty() {
        return None;
    }
    Some(parts.iter().collect())
}

/// ¿La entrada es la propia carpeta de extracción («./», «a/../»)? Los tar hechos con
/// `tar czf x -C dir .` empiezan así: se salta, no es un error.
pub fn is_root_entry(name: &str) -> bool {
    !name.is_empty() && sanitize(name).is_some_and(|p| p.is_empty())
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
        for bad in ["/etc/x", "C:\\x", "c:/x", "\\\\srv\\x", "a/../../x", "..", "", ".", "a\\..\\..\\x", "a\0b", "a/C:x", "a/C:/x", "a/b:stream", "a/.../b", "a/ /b", "a/.. /b", "a/. ./b", "CON", "con.txt", "a/Nul.tar.gz", "PRN", "aux", "COM1", "lpt9.x", "a/com5 /b", "a/con ./b", "a/CON.", "COM¹"] {
            assert_eq!(safe_entry_path(bad), None, "{bad:?}");
        }
        assert_eq!(safe_entry_path("a/./b"), Some(PathBuf::from("a/b")));
        assert_eq!(safe_entry_path("a/b/../c"), Some(PathBuf::from("a/c")));
        assert_eq!(safe_entry_path("a\\b"), Some(PathBuf::from("a/b")));
        assert_eq!(safe_entry_path("dir/"), Some(PathBuf::from("dir")));
        // Parecidos a los reservados, pero no lo son.
        for ok in ["console", "COM0", "COM10", "LPT", "a/auxiliar.txt", "nul_x", "com"] {
            assert!(safe_entry_path(ok).is_some(), "{ok}");
        }
    }

    #[test]
    fn root_entries() {
        for r in ["./", ".", "a/../", "./.", "a/./.."] {
            assert!(is_root_entry(r), "{r}");
        }
        for n in ["", "/", "../", "a", "C:", "a/../..", "CON"] {
            assert!(!is_root_entry(n), "{n}");
        }
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

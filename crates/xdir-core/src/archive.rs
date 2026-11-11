//! Reglas puras de archivos comprimidos: formatos, nombres y seguridad de las rutas de adentro.
//! La lectura y escritura de zip y tar vive en la crate `xdir-archive`.

use std::path::PathBuf;

/// Formatos que sabe tratar la crate `xdir-archive`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

/// Qué es una entrada de un archivo comprimido.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    /// Enlace duro, dispositivo o FIFO de un tar: se lista pero no se crea (se avisa como omitido).
    Omitted,
}

/// Una entrada del archivo, tal como figura adentro (sin sanear: quien extrae la valida con
/// `safe_entry_path`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArchiveEntry {
    pub index: usize,
    pub name: String,
    pub size: u64,
    pub kind: EntryKind,
    /// Destino del enlace simbólico.
    pub link: Option<String>,
    /// Segundos desde 1970 (0 si no figura).
    pub modified: u64,
    /// Bytes que ocupa comprimida dentro del archivo (0 si el formato no lo dice: un tar.gz se
    /// comprime entero).
    pub packed: u64,
    /// Cifrada (un zip con contraseña): se lista pero no se puede leer.
    pub encrypted: bool,
}

/// Bytes que declara el archivo para sus archivos (lo que ocupará extraído). No se confía (quien
/// extrae corta si una entrada escribe más), pero sirve para avisar antes de empezar.
pub fn declared_total(entries: &[ArchiveEntry]) -> u64 {
    entries.iter().filter(|e| e.kind == EntryKind::File).map(|e| e.size).sum()
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
            c if c.contains(':') || c.trim_end_matches(['.', ' ']).is_empty() || crate::names::is_dos_reserved(c) => return None,
            c => parts.push(c),
        }
    }
    Some(parts.into_iter().map(String::from).collect())
}

/// Ruta relativa y segura de una entrada, o `None` si es absoluta, con unidad o UNC, con `..`
/// que sale de la carpeta, con un byte nulo o vacía; también si algún componente tiene `:`, está
/// hecho solo de puntos y espacios o es un nombre de dispositivo de Windows
/// (`names::is_dos_reserved`), igual en todos los sistemas para que un archivo se comporte igual
/// en todos. `\` cuenta como separador; `.` y los separadores repetidos se descartan.
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
        for bad in ["/etc/x", "C:\\x", "c:/x", "\\\\srv\\x", "a/../../x", "..", "", ".", "a\\..\\..\\x", "a\0b", "a/C:x", "a/C:/x", "a/b:stream", "a/.../b", "a/ /b", "a/.. /b", "a/. ./b", "CON", "con.txt", "a/Nul.tar.gz", "PRN", "aux", "COM1", "lpt9.x", "a/com5 /b", "a/con ./b", "a/CON.", "COM¹", "CONIN$", "a/conout$.txt"] {
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

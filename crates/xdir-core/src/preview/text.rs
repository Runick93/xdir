//! Qué es texto para la vista previa: extensiones conocidas y contenido que es UTF-8 válido.

/// Extensiones que se muestran como texto sin mirar el contenido (en minúsculas). SVG no: el
/// sistema lo dibuja como imagen.
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "yaml", "yml", "xml", "html", "htm", "css", "js", "ts", "tsx", "jsx", "py", "c", "h", "cpp",
    "hpp", "cs", "java", "kt", "swift", "go", "rb", "php", "sh", "ps1", "bat", "cmd", "ini", "cfg", "conf", "log", "csv", "tsv",
    "sql", "gitignore",
];

/// ¿La extensión (sin el punto, sin distinguir mayúsculas) es de un archivo de texto?
pub fn is_text_ext(ext: &str) -> bool {
    TEXT_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// Las líneas de `bytes` si son texto: UTF-8 válido y sin NUL. Con `cut` (se leyó hasta un
/// límite y el archivo sigue), un carácter incompleto al final se descarta en vez de invalidar
/// el texto. Partido por `\n`, sin el `\r` final de cada línea; un `\n` al final no agrega una
/// línea vacía.
pub fn decode_text(bytes: &[u8], cut: bool) -> Option<Vec<String>> {
    if bytes.contains(&0) {
        return None;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        // `error_len() == None`: la secuencia quedó incompleta al final (el corte).
        Err(e) if cut && e.error_len().is_none() => std::str::from_utf8(&bytes[..e.valid_up_to()]).ok()?,
        Err(_) => return None,
    };
    if text.is_empty() {
        return Some(Vec::new());
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    Some(body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_text_utf8_and_cut() {
        assert_eq!(decode_text(b"a\r\nb", false), Some(vec!["a".to_string(), "b".to_string()]));
        // «ñ» (C3 B1) partida al final: con `cut` cuenta como texto; sin `cut`, no.
        let cut = [b'a', b'\n', b'n', b'i', 0xC3];
        assert_eq!(decode_text(&cut, true), Some(vec!["a".to_string(), "ni".to_string()]));
        assert_eq!(decode_text(&cut, false), None);
        assert_eq!(decode_text(b"a\0b", false), None);
        assert_eq!(decode_text(b"a\0b", true), None);
        // Un error en el medio no es un corte.
        assert_eq!(decode_text(&[b'a', 0xFF, b'b'], true), None);
        assert_eq!(decode_text("señal\n".as_bytes(), false), Some(vec!["señal".to_string()]));
    }

    #[test]
    fn text_ext_list() {
        assert!(is_text_ext("RS"));
        assert!(is_text_ext("toml"));
        assert!(is_text_ext("gitignore"));
        assert!(!is_text_ext("svg"));
        assert!(!is_text_ext("jpg"));
        assert!(!is_text_ext(""));
    }
}

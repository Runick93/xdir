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
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
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

/// Las líneas de `bytes` en la codificación que tengan: UTF-16 con BOM (LE o BE), UTF-8 (con o
/// sin BOM) o, si `text_ext` (el archivo es de texto por su extensión), Windows-1252, que mapea
/// cada byte a un carácter sin perder ninguno. `None` si no es texto.
pub fn decode(bytes: &[u8], cut: bool, text_ext: bool) -> Option<Vec<String>> {
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return decode_utf16(rest, cut, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return decode_utf16(rest, cut, u16::from_be_bytes);
    }
    decode_text(bytes, cut).or_else(|| if text_ext { decode_cp1252(bytes) } else { None })
}

/// UTF-16 sin el BOM; con `cut`, un byte suelto o un par sustituto partido al final se descartan.
fn decode_utf16(bytes: &[u8], cut: bool, unit: fn([u8; 2]) -> u16) -> Option<Vec<String>> {
    if bytes.len() % 2 == 1 && !cut {
        return None;
    }
    let mut units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| unit(*c)).collect();
    if cut && units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
        units.pop();
    }
    let text = String::from_utf16(&units).ok()?;
    decode_text(text.as_bytes(), false)
}

/// Windows-1252 → texto: 0x80–0x9F según la tabla de Windows (los 5 sin definir quedan como su
/// control C1) y lo demás igual que Latin-1. `None` si hay NUL (binario).
fn decode_cp1252(bytes: &[u8]) -> Option<Vec<String>> {
    const HIGH: [u16; 32] = [
        0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F,
        0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
    ];
    if bytes.contains(&0) {
        return None;
    }
    let text: String = bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('\u{FFFD}'),
            _ => b as char,
        })
        .collect();
    decode_text(text.as_bytes(), false)
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

    #[test]
    fn decode_text_strips_bom() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhola\nchau", false), Some(vec!["hola".to_string(), "chau".to_string()]));
        assert_eq!(decode(b"\xEF\xBB\xBFhola", false, false), Some(vec!["hola".to_string()]));
    }

    /// Con extensión de texto, lo que no es UTF-8 se lee como Windows-1252 (sin perder bytes);
    /// sin extensión de texto, no (podría ser binario).
    #[test]
    fn decode_falls_back_to_cp1252_for_text_ext() {
        let latin = b"ma\xF1ana \x80 \x93ok\x94";
        assert_eq!(decode_text(latin, false), None);
        assert_eq!(decode(latin, false, true), Some(vec!["mañana € \u{201C}ok\u{201D}".to_string()]));
        assert_eq!(decode(latin, false, false), None);
        // Los 5 sin definir quedan como su control C1 (cada byte da un carácter).
        assert_eq!(decode(b"\x81\x8D\x8F\x90\x9D\xFF", false, true), Some(vec!["\u{81}\u{8D}\u{8F}\u{90}\u{9D}ÿ".to_string()]));
        assert_eq!(decode(b"a\0\xF1", false, true), None, "con NUL es binario");
    }

    /// UTF-16 con BOM (LE o BE), con o sin extensión de texto; un corte al final se descarta.
    #[test]
    fn decode_utf16_with_bom() {
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("año\r\nb".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode(&le, false, false), Some(vec!["año".to_string(), "b".to_string()]));
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("hola".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode(&be, false, true), Some(vec!["hola".to_string()]));
        // Un byte suelto al final: con corte se descarta; sin corte no es texto.
        let mut odd = le.clone();
        odd.push(b'x');
        assert_eq!(decode(&odd, true, false), Some(vec!["año".to_string(), "b".to_string()]));
        assert_eq!(decode(&odd, false, false), None);
        // Un par sustituto partido al final, con corte.
        let mut split: Vec<u8> = [0xFF, 0xFE].into_iter().chain("a".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        split.extend(0xD83Du16.to_le_bytes());
        assert_eq!(decode(&split, true, false), Some(vec!["a".to_string()]));
        assert_eq!(decode(&split, false, false), None);
    }
}

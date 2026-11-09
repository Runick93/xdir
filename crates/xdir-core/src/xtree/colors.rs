//! Colores y textos del treemap: categorías de archivo, rueda de matices y formato de tamaños.

use crate::geom::Color;

fn color(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

fn rgb((r, g, b): (f32, f32, f32)) -> Color {
    color(r, g, b)
}

/// Categorías de archivo: nombre, color y extensiones. La última recibe todo lo demás.
pub const CATS: [(&str, (f32, f32, f32), &[&str]); 8] = [
    ("Video", (0.60, 0.35, 0.95), &["mp4", "mkv", "avi", "mov", "wmv", "webm", "vhdx", "vhd", "vmdk"]),
    ("Imágenes", (0.95, 0.40, 0.72), &["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "psd", "heic"]),
    ("Audio", (0.98, 0.62, 0.25), &["mp3", "wav", "flac", "m4a", "ogg"]),
    ("Comprimidos", (0.55, 0.58, 0.64), &["zip", "rar", "7z", "iso", "tar", "gz", "cab", "msi", "img", "etl", "qcow2"]),
    ("Documentos", (0.38, 0.64, 0.98), &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "json", "xml", "log"]),
    ("Código", (0.70, 0.48, 0.96), &["rs", "js", "ts", "cs", "py", "java", "c", "cpp", "h", "html", "css", "go", "toml", "lock"]),
    ("Programas", (0.86, 0.38, 0.64), &["exe", "dll", "lib", "rlib", "rmeta", "pdb", "obj", "o", "sys", "a", "so", "d"]),
    ("Otros", (0.98, 0.80, 0.28), &[]),
];

/// Índice en `CATS` de la categoría de un archivo según su extensión.
pub fn category(name: &str) -> usize {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    CATS.iter().position(|(_, _, exts)| exts.contains(&ext.as_str())).unwrap_or(CATS.len() - 1)
}

pub fn file_color(name: &str) -> Color {
    rgb(CATS[category(name)].1)
}

pub fn lighten(c: Color, k: f32) -> Color {
    color((c.r + k).min(1.0), (c.g + k).min(1.0), (c.b + k).min(1.0))
}

/// Rueda de color: azul, violeta, magenta, rosa, naranja, amarillo, lima, verde, turquesa, celeste.
const WHEEL: [(f32, f32, f32); 11] = [
    (0.224, 0.420, 1.000),
    (0.545, 0.302, 1.000),
    (0.851, 0.275, 0.937),
    (1.000, 0.310, 0.604),
    (1.000, 0.439, 0.353),
    (1.000, 0.702, 0.278),
    (1.000, 0.894, 0.361),
    (0.722, 0.941, 0.290),
    (0.298, 0.878, 0.416),
    (0.184, 0.847, 0.753),
    (0.227, 0.784, 1.000),
];

/// Color de la rueda en `t` (0..1, cíclico), mezclando entre los dos puntos vecinos.
fn wheel_at(t: f32) -> (f32, f32, f32) {
    let x = t.rem_euclid(1.0) * WHEEL.len() as f32;
    let i = (x.floor() as usize) % WHEEL.len();
    let (a, b, f) = (WHEEL[i], WHEEL[(i + 1) % WHEEL.len()], x.fract());
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f)
}

fn to_hsl((r, g, b): (f32, f32, f32)) -> (f32, f32, f32) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

/// Color de un bloque según el matiz heredado y la profundidad: las carpetas son un marco más
/// oscuro que se aclara al anidarse, y los archivos son las teselas vivas de adentro.
pub fn position_color(hue: f32, depth: u8, dir: bool) -> Color {
    let (h, s, l) = to_hsl(wheel_at(hue));
    let d = depth as f32;
    let (s2, l2) = if dir { (s * 0.78, l * 0.5 + d * 0.035) } else { (s * 0.88, l + 0.03 + d * 0.01) };
    rgb(from_hsl(h, s2.clamp(0.0, 1.0), l2.clamp(0.1, 0.86)))
}

/// Negro azulado o blanco, el que mejor se lee sobre `c`.
pub fn text_on(c: Color) -> Color {
    let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    if lum > 0.62 { color(0.07, 0.06, 0.12) } else { color(1.0, 1.0, 1.0) }
}

pub fn fmt_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.2} {}", UNITS[u]).replace('.', ",") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_size_matches_windows() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1536), "1,50 KB");
        assert_eq!(fmt_size(5 * 1024u64.pow(3)), "5,00 GB");
    }

    #[test]
    fn category_by_extension() {
        assert_eq!(CATS[category("a.MP4")].0, "Video");
        assert_eq!(category("sin_ext"), CATS.len() - 1);
    }
}

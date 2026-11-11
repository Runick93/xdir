//! Novedades: el `CHANGELOG.md` de la raíz del repo, incluido en el binario al compilar, y lo que
//! hace falta para mostrarlo (un parser del markdown restringido que usa ese archivo y el corte en
//! líneas). El formato está en `docs/superpowers/specs/2026-11-10-novedades-design.md`: títulos
//! `## <versión> — <AAAA-MM-DD>` o `## Sin publicar`, subtítulos `### …`, viñetas `- ` (con
//! sub-viñetas de dos espacios), `**negrita**` y `` `código` ``. Lo demás se muestra como texto.

/// El CHANGELOG del repo (la ruta es relativa a este archivo: `crates/xdir-core/src`).
pub const TEXT: &str = include_str!("../../../CHANGELOG.md");

/// Título de la sección que todavía no salió en una versión.
const UNRELEASED: &str = "Sin publicar";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Plain,
    Bold,
    Code,
}

/// Un tramo de texto con su estilo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub style: Style,
    pub text: String,
}

impl Span {
    pub fn new(style: Style, text: impl Into<String>) -> Self {
        Self { style, text: text.into() }
    }
}

/// Lo que se dibuja, de arriba abajo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Título de una versión; `version` vacía es «Sin publicar» (se muestra «En desarrollo»).
    /// `date` como viene en el archivo (`AAAA-MM-DD`), o vacía.
    Version { version: String, date: String },
    /// Subtítulo (`### Nuevo`).
    Heading(String),
    /// Viñeta; `level` 0 es la de primer nivel.
    Bullet { level: u8, spans: Vec<Span> },
    /// Cualquier otra línea, como texto.
    Para(Vec<Span>),
}

/// Lee el CHANGELOG. Tolera BOM, CRLF, secciones vacías y líneas que no entiende (van como texto).
/// La sección «Sin publicar» vacía no aparece.
pub fn parse(text: &str) -> Vec<Block> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut out: Vec<Block> = Vec::new();
    let mut seen_title = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        // El título del archivo («# Novedades de xdir»), antes de cualquier versión.
        if !seen_title && out.is_empty() && line.starts_with("# ") {
            seen_title = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            out.push(version_block(rest.trim()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            out.push(Block::Heading(rest.trim().to_string()));
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let body = line.trim_start();
        if let Some(item) = body.strip_prefix("- ") {
            let level = (indent / 2).min(2) as u8;
            out.push(Block::Bullet { level, spans: spans(item.trim()) });
            continue;
        }
        // Una línea con sangría que sigue a una viñeta o un párrafo es su continuación.
        if indent > 0
            && let Some(Block::Bullet { spans: prev, .. } | Block::Para(prev)) = out.last_mut()
        {
            let mut joined = String::new();
            for s in prev.iter() {
                joined.push_str(&raw_of(s));
            }
            joined.push(' ');
            joined.push_str(body);
            *prev = spans(&joined);
            continue;
        }
        out.push(Block::Para(spans(body)));
    }
    drop_empty_unreleased(out)
}

/// El texto de un tramo como estaba en el archivo (para volver a leerlo al juntar líneas).
fn raw_of(s: &Span) -> String {
    match s.style {
        Style::Plain => s.text.clone(),
        Style::Bold => format!("**{}**", s.text),
        Style::Code => format!("`{}`", s.text),
    }
}

/// `0.1.24 — 2026-11-10` → versión y fecha; `Sin publicar` → versión vacía.
fn version_block(title: &str) -> Block {
    if title.eq_ignore_ascii_case(UNRELEASED) {
        return Block::Version { version: String::new(), date: String::new() };
    }
    let (version, date) = match title.split_once('—') {
        Some((v, d)) => (v.trim(), d.trim()),
        None => (title, ""),
    };
    Block::Version { version: version.to_string(), date: date.to_string() }
}

/// Saca «Sin publicar» si no tiene nada hasta la versión siguiente.
fn drop_empty_unreleased(blocks: Vec<Block>) -> Vec<Block> {
    let empty: Vec<bool> = (0..blocks.len())
        .map(|i| {
            matches!(&blocks[i], Block::Version { version, .. } if version.is_empty())
                && blocks.get(i + 1).is_none_or(|b| matches!(b, Block::Version { .. }))
        })
        .collect();
    blocks.into_iter().zip(empty).filter_map(|(b, e)| (!e).then_some(b)).collect()
}

/// Tramos de una línea: `**negrita**` y `` `código` ``; lo que no cierra queda como texto.
pub fn spans(s: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
            && end > 0
        {
            flush(&mut out, &mut plain);
            out.push(Span::new(Style::Bold, &after[..end]));
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
            && end > 0
        {
            flush(&mut out, &mut plain);
            out.push(Span::new(Style::Code, &after[..end]));
            rest = &after[end + 1..];
            continue;
        }
        let ch = rest.chars().next().unwrap_or(' ');
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush(&mut out, &mut plain);
    out
}

fn flush(out: &mut Vec<Span>, plain: &mut String) {
    if !plain.is_empty() {
        out.push(Span::new(Style::Plain, std::mem::take(plain)));
    }
}

/// `AAAA-MM-DD` → `DD/MM/AAAA`; otra cosa queda como está.
pub fn display_date(date: &str) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    match parts[..] {
        [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 && date.chars().all(|c| c.is_ascii_digit() || c == '-') => format!("{d}/{m}/{y}"),
        _ => date.to_string(),
    }
}

/// `0.1.24` → `[0, 1, 24]`; `None` si no es una versión de números con puntos.
pub fn version_parts(v: &str) -> Option<Vec<u64>> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

/// ¿Hay que avisar que xdir se actualizó? `seen`: la última versión vista guardada en la sesión
/// (`None`: una sesión de antes de Novedades). No avisa si la de ahora no es más nueva (bajar de
/// versión, la misma) ni si alguna no se entiende.
pub fn should_notify(seen: Option<&str>, current: &str) -> bool {
    let Some(now) = version_parts(current) else { return false };
    match seen {
        None => true,
        Some(s) => version_parts(s).is_some_and(|old| now > old),
    }
}

/// La más nueva de dos versiones (la que se guarda como vista); si una no se entiende, la otra.
pub fn newest<'a>(a: &'a str, b: &'a str) -> &'a str {
    match (version_parts(a), version_parts(b)) {
        (Some(x), Some(y)) => {
            if y > x {
                b
            } else {
                a
            }
        }
        (None, Some(_)) => b,
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> Vec<Span> {
        vec![Span::new(Style::Plain, s)]
    }

    #[test]
    fn parses_versions_headings_and_bullets() {
        let text = "# Novedades de xdir\n\n## Sin publicar\n### Nuevo\n- Uno.\n  - Detalle.\n\n## 0.1.23 — 2026-11-08\n- Antes.\n";
        assert_eq!(
            parse(text),
            vec![
                Block::Version { version: String::new(), date: String::new() },
                Block::Heading("Nuevo".into()),
                Block::Bullet { level: 0, spans: plain("Uno.") },
                Block::Bullet { level: 1, spans: plain("Detalle.") },
                Block::Version { version: "0.1.23".into(), date: "2026-11-08".into() },
                Block::Bullet { level: 0, spans: plain("Antes.") },
            ]
        );
    }

    #[test]
    fn empty_unreleased_is_dropped() {
        let text = "# Novedades de xdir\n\n## Sin publicar\n\n## 0.1.24 — 2026-11-10\n- Algo.\n";
        assert_eq!(
            parse(text),
            vec![Block::Version { version: "0.1.24".into(), date: "2026-11-10".into() }, Block::Bullet { level: 0, spans: plain("Algo.") }]
        );
        // Al final del archivo también.
        assert_eq!(parse("## Sin publicar\n\n"), vec![]);
    }

    #[test]
    fn bom_and_crlf() {
        let text = "\u{FEFF}# Novedades\r\n\r\n## 1.0.0 — 2027-01-02\r\n### Arreglos\r\n- **Uno**.\r\n";
        assert_eq!(
            parse(text),
            vec![
                Block::Version { version: "1.0.0".into(), date: "2027-01-02".into() },
                Block::Heading("Arreglos".into()),
                Block::Bullet { level: 0, spans: vec![Span::new(Style::Bold, "Uno"), Span::new(Style::Plain, ".")] },
            ]
        );
    }

    #[test]
    fn missing_sections_and_odd_input() {
        assert_eq!(parse(""), vec![]);
        assert_eq!(parse("\n\n   \n"), vec![]);
        // Sin títulos de versión: lo que haya, como viene.
        assert_eq!(parse("- suelto"), vec![Block::Bullet { level: 0, spans: plain("suelto") }]);
        // Versión sin fecha.
        assert_eq!(parse("## 0.2.0"), vec![Block::Version { version: "0.2.0".into(), date: String::new() }]);
    }

    #[test]
    fn unknown_markup_is_plain_text() {
        let text = "## 1.0.0 — 2027-01-02\n> cita\n1. numerada\n#### chico\n- [enlace](https://x) y _cursiva_\n- **sin cerrar y `tampoco\n# otro título\n";
        assert_eq!(
            parse(text),
            vec![
                Block::Version { version: "1.0.0".into(), date: "2027-01-02".into() },
                Block::Para(plain("> cita")),
                Block::Para(plain("1. numerada")),
                Block::Para(plain("#### chico")),
                Block::Bullet { level: 0, spans: plain("[enlace](https://x) y _cursiva_") },
                Block::Bullet { level: 0, spans: plain("**sin cerrar y `tampoco") },
                Block::Para(plain("# otro título")),
            ]
        );
    }

    #[test]
    fn spans_bold_and_code() {
        assert_eq!(
            spans("**Duplicar** (`Ctrl+D` / `⌘D`) y más"),
            vec![
                Span::new(Style::Bold, "Duplicar"),
                Span::new(Style::Plain, " ("),
                Span::new(Style::Code, "Ctrl+D"),
                Span::new(Style::Plain, " / "),
                Span::new(Style::Code, "⌘D"),
                Span::new(Style::Plain, ") y más"),
            ]
        );
        // Marcadores vacíos quedan como texto.
        assert_eq!(spans("a ** b `` c"), plain("a ** b `` c"));
    }

    #[test]
    fn indented_continuation_joins_the_bullet() {
        let text = "## 1.0.0 — 2027-01-02\n- Una viñeta larga que\n  sigue en **otra\n  línea** acá.\n";
        assert_eq!(
            parse(text)[1],
            Block::Bullet {
                level: 0,
                spans: vec![Span::new(Style::Plain, "Una viñeta larga que sigue en "), Span::new(Style::Bold, "otra línea"), Span::new(Style::Plain, " acá.")]
            }
        );
    }

    #[test]
    fn dates_and_versions() {
        assert_eq!(display_date("2026-11-08"), "08/11/2026");
        assert_eq!(display_date("ayer"), "ayer");
        assert_eq!(display_date(""), "");
        assert_eq!(version_parts("0.1.24"), Some(vec![0, 1, 24]));
        assert_eq!(version_parts("0.1.x"), None);
        assert!(should_notify(Some("0.1.23"), "0.1.24"));
        assert!(should_notify(Some("0.1.9"), "0.1.10"), "por número, no por texto");
        assert!(!should_notify(Some("0.1.24"), "0.1.24"));
        assert!(!should_notify(Some("0.2.0"), "0.1.24"), "bajar de versión no avisa");
        assert!(!should_notify(Some("basura"), "0.1.24"));
        assert!(should_notify(None, "0.1.24"), "sesión de antes de Novedades");
        assert_eq!(newest("0.1.9", "0.1.10"), "0.1.10");
        assert_eq!(newest("0.2.0", "0.1.24"), "0.2.0");
        assert_eq!(newest("basura", "0.1.24"), "0.1.24");
    }

    /// El CHANGELOG del repo se lee entero: una versión por título y nada como texto suelto (todo
    /// lo que tiene es del formato).
    #[test]
    fn bundled_changelog_parses() {
        let blocks = parse(TEXT);
        assert!(blocks.iter().any(|b| matches!(b, Block::Version { version, .. } if version == "0.1.23")), "{blocks:?}");
        assert!(!blocks.iter().any(|b| matches!(b, Block::Para(_))), "algo del CHANGELOG no es del formato: {blocks:?}");
        let titles = TEXT.lines().filter(|l| l.starts_with("## ")).count();
        let versions = blocks.iter().filter(|b| matches!(b, Block::Version { .. })).count();
        assert!(versions == titles || versions + 1 == titles, "{versions} de {titles}");
        for b in &blocks {
            if let Block::Version { version, date } = b
                && !version.is_empty()
            {
                assert!(version_parts(version).is_some(), "{version}");
                assert_eq!(display_date(date).len(), 10, "{date}");
            }
        }
    }
}

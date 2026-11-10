//! Cambiar `config.toml` por líneas sin perder lo escrito a mano: solo cambian las líneas de lo
//! que se tocó; comentarios, orden, claves y secciones desconocidas y líneas con error quedan
//! byte por byte. Se conserva el fin de línea del archivo (`\n` o `\r\n`), la falta de salto final
//! y la marca de orden de bytes (BOM) si la había.
//!
//! Las líneas se clasifican con el mismo lector de `toml.rs` (`strip_comment`, `parse_entry`), así
//! una clave «activa» es lo mismo que `toml::parse` lee. Lo que el lector no entiende (una lista o
//! un string de varias líneas) se trata entero como opaco: no se busca nada adentro ni se inserta
//! en el medio.

use crate::toml::{self, parse_entry, strip_comment};

/// Cambia (o agrega) `key = value` en `[section]`. `value` va ya escrito en TOML
/// (`"\"Ctrl+K\""`, `"true"`, `"[\"Ctrl+K\", \"F3\"]"`). En orden:
/// 1. la última línea activa de la clave en esa sección (si la sección se repite, en cualquiera
///    de ellas): cambia solo el valor; sangría, comillas de la clave y comentario final quedan;
/// 2. si no, justo debajo del último ejemplo comentado `# key = …` de la sección (queda);
/// 3. si no, después de la última línea activa de la última `[section]`, o de su encabezado;
/// 4. sin la sección, al final: una línea en blanco, `[section]` y la línea.
pub fn set(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut doc = Doc::read(text);
    let kinds = doc.classify();
    let ranges = sections(&kinds, section);
    let in_section = |i: &usize| ranges.iter().any(|r| r.contains(i));
    let line = format!("{} = {value}", write_key(key));
    let active = (0..kinds.len()).filter(in_section).rfind(|&i| matches!(&kinds[i], Kind::Active(k) if k == key));
    if let Some(i) = active {
        let raw = &doc.lines[i].text;
        if let Some((start, end)) = value_span(raw) {
            doc.lines[i].text = format!("{}{value}{}", &raw[..start], &raw[end..]);
        }
        return doc.write();
    }
    let example = (0..kinds.len()).filter(in_section).rfind(|&i| matches!(&kinds[i], Kind::Example(k) if k == key));
    if let Some(i) = example {
        doc.insert_after(i, line);
        return doc.write();
    }
    if let Some(last) = ranges.last() {
        // `last.start` es la línea después del encabezado (la sección raíz no tiene: va arriba).
        match last.clone().rfind(|&i| matches!(kinds[i], Kind::Active(_))) {
            Some(i) => doc.insert_after(i, line),
            None if last.start > 0 => doc.insert_after(last.start - 1, line),
            None => doc.lines.insert(0, Line { text: line, eol: doc.eol }),
        }
        return doc.write();
    }
    doc.append_section(section, line);
    doc.write()
}

/// Quita todas las líneas activas de la clave en `[section]`; el ejemplo comentado queda.
pub fn remove(text: &str, section: &str, key: &str) -> String {
    remove_where(text, section, |k| k == key)
}

/// Quita todas las líneas activas de `[section]`; comentarios, encabezado y líneas con error quedan.
pub fn remove_all(text: &str, section: &str) -> String {
    remove_where(text, section, |_| true)
}

/// Cuántas líneas activas tiene `[section]` (p. ej. los atajos propios de `[atajos]`).
pub fn count_active(text: &str, section: &str) -> usize {
    let doc = Doc::read(text);
    let kinds = doc.classify();
    sections(&kinds, section).into_iter().flatten().filter(|&i| matches!(kinds[i], Kind::Active(_))).count()
}

/// Lista de strings como valor TOML: 0 → `""`, 1 → `"x"`, varios → `["a", "b"]`.
pub fn str_value(items: &[String]) -> String {
    match items {
        [] => "\"\"".to_string(),
        [one] => toml::quote(one),
        many => format!("[{}]", many.iter().map(|s| toml::quote(s)).collect::<Vec<_>>().join(", ")),
    }
}

fn remove_where(text: &str, section: &str, pick: impl Fn(&str) -> bool) -> String {
    let mut doc = Doc::read(text);
    let kinds = doc.classify();
    let ranges = sections(&kinds, section);
    let doomed: Vec<usize> =
        ranges.into_iter().flatten().filter(|&i| matches!(&kinds[i], Kind::Active(k) if pick(k))).collect();
    if doomed.is_empty() {
        return text.to_string();
    }
    for &i in doomed.iter().rev() {
        doc.remove(i);
    }
    doc.write()
}

/// La clave como la entiende `toml::parse`: suelta si es `[A-Za-z0-9_-]+`, si no entre comillas.
fn write_key(key: &str) -> String {
    if !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        key.to_string()
    } else {
        toml::quote(key)
    }
}

/// Dónde está el valor de una línea activa (sin el comentario final ni los espacios antes de él).
fn value_span(raw: &str) -> Option<(usize, usize)> {
    let end = strip_comment(raw).trim_end().len();
    let mut it = raw.char_indices().skip_while(|(_, c)| c.is_whitespace()).peekable();
    let (_, first) = *it.peek()?;
    if first == '"' || first == '\'' {
        it.next();
        let mut escaped = false;
        loop {
            let (_, c) = it.next()?;
            if first == '"' && escaped {
                escaped = false;
            } else if first == '"' && c == '\\' {
                escaped = true;
            } else if c == first {
                break;
            }
        }
    } else {
        while it.next_if(|(_, c)| *c != '=' && !c.is_whitespace()).is_some() {}
    }
    while it.next_if(|(_, c)| c.is_whitespace()).is_some() {}
    if it.next()?.1 != '=' {
        return None;
    }
    while it.next_if(|(_, c)| c.is_whitespace()).is_some() {}
    let start = it.peek().map(|(i, _)| *i).unwrap_or(raw.len());
    (start <= end).then_some((start, end))
}

#[derive(Debug, PartialEq)]
enum Kind {
    Blank,
    /// Comentario; con la clave si es un ejemplo `# clave = valor`.
    Comment,
    Example(String),
    Header { name: String, array: bool },
    Active(String),
    /// Una línea que `toml::parse` no entiende: nunca cambia.
    Error,
    /// Dentro de una lista o string de varias líneas: no se mira ni se toca.
    Opaque,
}

/// Rangos de líneas (sin el encabezado) de cada `[section]` (no `[[section]]`), en orden.
fn sections(kinds: &[Kind], section: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut current: Option<usize> = if section.is_empty() { Some(0) } else { None };
    for (i, k) in kinds.iter().enumerate() {
        if let Kind::Header { name, array } = k {
            if let Some(start) = current.take() {
                out.push(start..i);
            }
            if name == section && !array {
                current = Some(i + 1);
            }
        }
    }
    if let Some(start) = current {
        out.push(start..kinds.len());
    }
    out
}

struct Line {
    text: String,
    /// `""` solo en la última línea de un archivo sin salto final.
    eol: &'static str,
}

struct Doc {
    bom: bool,
    lines: Vec<Line>,
    /// Fin de línea para las líneas nuevas: el de la primera línea.
    eol: &'static str,
}

impl Doc {
    fn read(text: &str) -> Self {
        let (bom, text) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let lines: Vec<Line> = text
            .split_inclusive('\n')
            .map(|piece| {
                if let Some(t) = piece.strip_suffix("\r\n") {
                    Line { text: t.to_string(), eol: "\r\n" }
                } else if let Some(t) = piece.strip_suffix('\n') {
                    Line { text: t.to_string(), eol: "\n" }
                } else {
                    Line { text: piece.to_string(), eol: "" }
                }
            })
            .collect();
        let eol = lines.first().map(|l| l.eol).filter(|e| !e.is_empty()).unwrap_or("\n");
        Self { bom, lines, eol }
    }

    fn write(&self) -> String {
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        for l in &self.lines {
            out += &l.text;
            out += l.eol;
        }
        out
    }

    fn classify(&self) -> Vec<Kind> {
        let mut kinds = Vec::with_capacity(self.lines.len());
        // Dentro de algo de varias líneas: profundidad de corchetes o el delimitador del string.
        let mut depth = 0i32;
        let mut long_string: Option<&str> = None;
        for l in &self.lines {
            if let Some(delim) = long_string {
                if !l.text.matches(delim).count().is_multiple_of(2) {
                    long_string = None;
                }
                kinds.push(Kind::Opaque);
                continue;
            }
            if depth > 0 {
                depth += bracket_balance(&l.text);
                kinds.push(Kind::Opaque);
                continue;
            }
            let content = strip_comment(&l.text).trim();
            if content.is_empty() {
                let t = l.text.trim_start();
                kinds.push(match t.strip_prefix('#') {
                    None => Kind::Blank,
                    Some(body) => {
                        let body = strip_comment(body.trim_start_matches('#')).trim();
                        match parse_entry(body) {
                            Ok((k, _)) => Kind::Example(k),
                            Err(_) => Kind::Comment,
                        }
                    }
                });
                continue;
            }
            if let Some(name) = content.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
                kinds.push(Kind::Header { name: name.trim().to_string(), array: true });
                continue;
            }
            if let Some(name) = content.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                kinds.push(Kind::Header { name: name.trim().to_string(), array: false });
                continue;
            }
            match parse_entry(content) {
                Ok((k, _)) => kinds.push(Kind::Active(k)),
                Err(_) => {
                    for delim in ["\"\"\"", "'''"] {
                        if !l.text.matches(delim).count().is_multiple_of(2) {
                            long_string = Some(delim);
                        }
                    }
                    if long_string.is_none() {
                        depth = bracket_balance(&l.text).max(0);
                    }
                    kinds.push(Kind::Error);
                }
            }
        }
        kinds
    }

    fn insert_after(&mut self, i: usize, text: String) {
        let eol = if self.lines[i].eol.is_empty() {
            self.lines[i].eol = self.eol;
            ""
        } else {
            self.eol
        };
        self.lines.insert(i + 1, Line { text, eol });
    }

    fn remove(&mut self, i: usize) {
        if self.lines[i].eol.is_empty() && i > 0 {
            self.lines[i - 1].eol = "";
        }
        self.lines.remove(i);
    }

    fn append_section(&mut self, section: &str, line: String) {
        let header = format!("[{}]", if write_key(section) == section { section.to_string() } else { toml::quote(section) });
        let eol = self.eol;
        let Some(last) = self.lines.last_mut() else {
            self.lines.push(Line { text: header, eol });
            self.lines.push(Line { text: line, eol });
            return;
        };
        let final_eol = if last.eol.is_empty() {
            last.eol = eol;
            ""
        } else {
            eol
        };
        self.lines.push(Line { text: String::new(), eol });
        self.lines.push(Line { text: header, eol });
        self.lines.push(Line { text: line, eol: final_eol });
    }
}

/// Corchetes abiertos menos cerrados fuera de strings y del comentario.
fn bracket_balance(line: &str) -> i32 {
    let mut n = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in strip_comment(line).chars() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '[' => n += 1,
            None if c == ']' => n -= 1,
            None => {}
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{self, Command};
    use crate::config::{Config, template};

    #[test]
    fn set_replaces_value_keeps_comment() {
        let t = "[opciones]\nanimaciones = true   # mío\n";
        assert_eq!(set(t, "opciones", "animaciones", "false"), "[opciones]\nanimaciones = false   # mío\n");
        // Sangría, espacios alrededor del '=' y un '#' dentro del string anterior quedan.
        let t = "[atajos]\n   duplicar   =   \"Ctrl+#\"\t# el mío\n";
        assert_eq!(set(t, "atajos", "duplicar", "\"Ctrl+K\""), "[atajos]\n   duplicar   =   \"Ctrl+K\"\t# el mío\n");
        // Sin comentario.
        assert_eq!(set("[opciones]\nanimaciones=true\n", "opciones", "animaciones", "false"), "[opciones]\nanimaciones=false\n");
    }

    #[test]
    fn set_quoted_key() {
        let t = "[colores]\n\"*.pdf\" = \"#E5534B\" # rojo\n'@oculto' = \"#768390\"\n\"a=b\" = \"#000000\"\n";
        let out = set(t, "colores", "*.pdf", "\"#112233\"");
        assert_eq!(out, "[colores]\n\"*.pdf\" = \"#112233\" # rojo\n'@oculto' = \"#768390\"\n\"a=b\" = \"#000000\"\n");
        let out = set(&out, "colores", "@oculto", "\"#FFFFFF\"");
        assert!(out.contains("'@oculto' = \"#FFFFFF\"\n"), "{out}");
        let out = set(&out, "colores", "a=b", "\"#010101\"");
        assert!(out.contains("\"a=b\" = \"#010101\"\n"), "{out}");
        // Una clave nueva con caracteres raros va entre comillas.
        let out = set(&out, "colores", "*.log", "\"#445566\"");
        assert!(out.ends_with("\"a=b\" = \"#010101\"\n\"*.log\" = \"#445566\"\n"), "{out}");
        let c = Config::parse(&out);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
    }

    #[test]
    fn set_below_commented_example() {
        crate::sys::testing::install_fakes();
        let t = template();
        let out = set(&t, "atajos", "duplicar", "\"Ctrl+Shift+K\"");
        let lines: Vec<&str> = out.lines().collect();
        let ex = lines.iter().position(|l| l.starts_with("# duplicar ")).unwrap();
        assert_eq!(lines[ex + 1], "duplicar = \"Ctrl+Shift+K\"");
        assert_eq!(out.lines().count(), t.lines().count() + 1);
        // También en [opciones].
        let out = set(&t, "opciones", "animaciones", "true");
        assert!(out.contains("# animaciones = true\nanimaciones = true\n"), "{out}");
    }

    #[test]
    fn set_appends_in_section() {
        let t = "[opciones]\nanimaciones = true\n# fin\n\n[colores]\n";
        assert_eq!(
            set(t, "opciones", "arbol_sigue_panel", "false"),
            "[opciones]\nanimaciones = true\narbol_sigue_panel = false\n# fin\n\n[colores]\n"
        );
        // Sección sin líneas activas: debajo del encabezado.
        assert_eq!(set("[opciones]\n# nada\n[colores]\n", "opciones", "x", "1"), "[opciones]\nx = 1\n# nada\n[colores]\n");
        // Un [[opciones]] no es la sección.
        assert_eq!(set("[[opciones]]\nx = 1\n", "opciones", "x", "2"), "[[opciones]]\nx = 1\n\n[opciones]\nx = 2\n");
    }

    #[test]
    fn set_creates_section_at_end() {
        assert_eq!(set("[atajos]\n", "opciones", "animaciones", "true"), "[atajos]\n\n[opciones]\nanimaciones = true\n");
        assert_eq!(set("[atajos]", "opciones", "animaciones", "true"), "[atajos]\n\n[opciones]\nanimaciones = true");
        assert_eq!(set("", "opciones", "animaciones", "true"), "[opciones]\nanimaciones = true\n");
        // Lo de antes de la primera sección no es la sección.
        assert_eq!(set("x = 1\n", "opciones", "x", "2"), "x = 1\n\n[opciones]\nx = 2\n");
    }

    #[test]
    fn crlf_and_no_final_newline_kept() {
        let t = "[atajos]\r\n# duplicar = \"Ctrl+D\"\r\nx = 1";
        let out = set(t, "atajos", "duplicar", "\"Ctrl+K\"");
        assert_eq!(out, "[atajos]\r\n# duplicar = \"Ctrl+D\"\r\nduplicar = \"Ctrl+K\"\r\nx = 1");
        assert_eq!(remove(&out, "atajos", "duplicar"), t);
        // Agregar después de la última línea, que no tiene salto.
        let out = set(t, "atajos", "y", "2");
        assert_eq!(out, "[atajos]\r\n# duplicar = \"Ctrl+D\"\r\nx = 1\r\ny = 2");
        assert_eq!(remove(&out, "atajos", "y"), t);
        let out = set(t, "opciones", "a", "true");
        assert_eq!(out, "[atajos]\r\n# duplicar = \"Ctrl+D\"\r\nx = 1\r\n\r\n[opciones]\r\na = true");
        assert_eq!(set(t, "atajos", "x", "3"), "[atajos]\r\n# duplicar = \"Ctrl+D\"\r\nx = 3");
    }

    #[test]
    fn bom_kept() {
        let t = "\u{feff}[opciones]\r\nanimaciones = true\r\n";
        assert_eq!(set(t, "opciones", "animaciones", "false"), "\u{feff}[opciones]\r\nanimaciones = false\r\n");
        assert_eq!(remove(t, "opciones", "animaciones"), "\u{feff}[opciones]\r\n");
        assert_eq!(Config::parse(&set(t, "opciones", "animaciones", "false")).animations, Some(false));
    }

    #[test]
    fn repeated_section_edits_last() {
        let t = "[atajos]\nduplicar = \"Ctrl+1\"\n[colores]\n[atajos]\nduplicar = \"Ctrl+2\"\n";
        assert_eq!(
            set(t, "atajos", "duplicar", "\"Ctrl+3\""),
            "[atajos]\nduplicar = \"Ctrl+1\"\n[colores]\n[atajos]\nduplicar = \"Ctrl+3\"\n"
        );
        // remove quita las dos.
        assert_eq!(remove(t, "atajos", "duplicar"), "[atajos]\n[colores]\n[atajos]\n");
        // Una clave nueva va a la última.
        assert_eq!(set(t, "atajos", "copiar", "\"F3\""), "[atajos]\nduplicar = \"Ctrl+1\"\n[colores]\n[atajos]\nduplicar = \"Ctrl+2\"\ncopiar = \"F3\"\n");
    }

    #[test]
    fn error_lines_untouched() {
        let t = "[atajos]\nbasura sin igual\nx = 1\n";
        assert_eq!(set(t, "atajos", "x", "2"), "[atajos]\nbasura sin igual\nx = 2\n");
        assert_eq!(set(t, "atajos", "y", "2"), "[atajos]\nbasura sin igual\nx = 1\ny = 2\n");
        // Una línea de la clave mal escrita no se toca; la buena va aparte.
        let t = "[atajos]\nduplicar = Ctrl+D\n";
        assert_eq!(set(t, "atajos", "duplicar", "\"Ctrl+K\""), "[atajos]\nduplicar = \"Ctrl+K\"\nduplicar = Ctrl+D\n");
        assert_eq!(remove(t, "atajos", "duplicar"), t);
        // Secciones y claves desconocidas quedan.
        let t = "[mia]\nduplicar = 1 # x\n[atajos]\n";
        assert_eq!(set(t, "atajos", "duplicar", "\"F1\""), "[mia]\nduplicar = 1 # x\n[atajos]\nduplicar = \"F1\"\n");
    }

    #[test]
    fn multiline_array_untouched() {
        let t = "[atajos]\ncopiar = [\n  \"Ctrl+C\",\n  # duplicar = \"x\"\n  [\"raro\"]\n]\npegar = \"Ctrl+V\"\n";
        // El ejemplo comentado de adentro no cuenta, ni el "[\"raro\"]" es una sección.
        assert_eq!(
            set(t, "atajos", "duplicar", "\"F1\""),
            "[atajos]\ncopiar = [\n  \"Ctrl+C\",\n  # duplicar = \"x\"\n  [\"raro\"]\n]\npegar = \"Ctrl+V\"\nduplicar = \"F1\"\n"
        );
        assert_eq!(remove_all(t, "atajos"), "[atajos]\ncopiar = [\n  \"Ctrl+C\",\n  # duplicar = \"x\"\n  [\"raro\"]\n]\n");
        let t = "[opciones]\nx = \"\"\"\n[atajos]\nduplicar = 1\n\"\"\"\n";
        assert_eq!(set(t, "atajos", "duplicar", "2"), format!("{t}\n[atajos]\nduplicar = 2\n"));
    }

    #[test]
    fn remove_then_original() {
        crate::sys::testing::install_fakes();
        let t = template();
        for (s, k, v) in [("atajos", "duplicar", "\"Ctrl+Shift+K\""), ("opciones", "animaciones", "false"), ("colores", "*.log", "\"#112233\"")] {
            assert_eq!(remove(&set(&t, s, k, v), s, k), t, "{s}.{k}");
        }
        assert_eq!(remove(&t, "atajos", "duplicar"), t, "sin la línea no cambia nada");
    }

    #[test]
    fn remove_all_keeps_comments() {
        let t = "# arriba\n[atajos]\n# duplicar = \"Ctrl+D\"\nduplicar = \"F1\" # mío\n\nbasura\ncopiar = \"F2\"\n[colores]\n\"*.x\" = \"#000000\"\n";
        assert_eq!(remove_all(t, "atajos"), "# arriba\n[atajos]\n# duplicar = \"Ctrl+D\"\n\nbasura\n[colores]\n\"*.x\" = \"#000000\"\n");
        assert_eq!(count_active(t, "atajos"), 2);
        assert_eq!(count_active(t, "opciones"), 0);
    }

    #[test]
    fn template_roundtrip_one_change() {
        crate::sys::testing::install_fakes();
        let before = Config::parse(&template());
        let after = Config::parse(&set(&template(), "atajos", "duplicar", "\"Ctrl+Shift+K\""));
        assert!(after.errors.is_empty(), "{:?}", after.errors);
        assert_eq!(after.colors, before.colors);
        assert_eq!(after.animations, before.animations);
        assert_eq!(after.tree_follows_pane, before.tree_follows_pane);
        for s in commands::ALL {
            if s.command == Command::Duplicate {
                assert_eq!(after.keymap.label(s.command), commands::shortcut_label("Ctrl+Shift+K"));
            } else {
                assert_eq!(after.keymap.label(s.command), before.keymap.label(s.command), "{}", s.id);
            }
        }
    }

    #[test]
    fn str_value_forms() {
        assert_eq!(str_value(&[]), "\"\"");
        assert_eq!(str_value(&["Ctrl+K".into()]), "\"Ctrl+K\"");
        assert_eq!(str_value(&["Ctrl+K".into(), "F3".into()]), "[\"Ctrl+K\", \"F3\"]");
        assert_eq!(str_value(&["a\"b".into()]), "\"a\\\"b\"");
    }
}

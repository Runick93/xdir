//! Subconjunto de TOML para los archivos que se editan a mano (`config.toml`, `grupos.toml`),
//! leído sin dependencias. Soporta:
//!
//! - comentarios `#` (también al final de la línea), líneas en blanco;
//! - secciones `[nombre]` y listas de tablas `[[nombre]]`;
//! - `clave = valor`, con la clave suelta (`pestana_nueva`) o entre comillas (`"*.pdf"`);
//! - strings `"..."` (con `\\`, `\"`, `\n`, `\t`) y literales `'...'` (sin escapes: cómodos para
//!   rutas de Windows), enteros, decimales, booleanos y listas en una línea (anidables).
//!
//! Lo que no entiende lo informa con el número de línea, sin cortar la lectura del resto.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Un string suelto o una lista de strings, como lista.
    pub fn as_str_list(&self) -> Option<Vec<&str>> {
        match self {
            Value::Str(s) => Some(vec![s.as_str()]),
            Value::List(items) => items.iter().map(Value::as_str).collect(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    /// Línea en el archivo (desde 1), para los mensajes de error.
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Vacío para lo que va antes de la primera sección.
    pub name: String,
    /// `[[nombre]]`: un elemento de una lista de tablas.
    pub array: bool,
    pub line: usize,
    pub entries: Vec<Entry>,
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|e| e.key == key).map(|e| &e.value)
    }
}

#[derive(Debug, Default)]
pub struct Document {
    pub sections: Vec<Section>,
    /// (línea, problema).
    pub errors: Vec<(usize, String)>,
}

impl Document {
    /// Secciones con ese nombre (una para `[x]`, varias para `[[x]]`).
    pub fn sections<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections.iter().filter(move |s| s.name == name)
    }
}

pub fn parse(text: &str) -> Document {
    let mut doc = Document { sections: vec![Section { name: String::new(), array: false, line: 0, entries: Vec::new() }], errors: Vec::new() };
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
            doc.sections.push(Section { name: name.trim().to_string(), array: true, line: line_no, entries: Vec::new() });
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            doc.sections.push(Section { name: name.trim().to_string(), array: false, line: line_no, entries: Vec::new() });
            continue;
        }
        match parse_entry(line) {
            Ok((key, value)) => doc.sections.last_mut().unwrap().entries.push(Entry { key, value, line: line_no }),
            Err(e) => doc.errors.push((line_no, e)),
        }
    }
    doc
}

/// Saca el comentario del final, respetando los `#` dentro de strings.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' => return &line[..i],
            None => {}
        }
    }
    line
}

fn parse_entry(line: &str) -> Result<(String, Value), String> {
    let mut chars = Chars::new(line);
    let key = if matches!(chars.peek(), Some('"') | Some('\'')) {
        chars.string()?
    } else {
        let k = chars.take_while(|c| c != '=' && !c.is_whitespace());
        if k.is_empty() {
            return Err("falta el nombre antes de '='".into());
        }
        k
    };
    chars.skip_ws();
    if chars.next() != Some('=') {
        return Err(format!("se esperaba '=' después de «{key}»"));
    }
    chars.skip_ws();
    let value = chars.value()?;
    chars.skip_ws();
    if let Some(c) = chars.peek() {
        return Err(format!("sobra texto después del valor: «{c}…»"));
    }
    Ok((key, value))
}

struct Chars<'a> {
    it: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Chars<'a> {
    fn new(s: &'a str) -> Self {
        Self { it: s.chars().peekable() }
    }

    fn peek(&mut self) -> Option<char> {
        self.it.peek().copied()
    }

    fn next(&mut self) -> Option<char> {
        self.it.next()
    }

    fn skip_ws(&mut self) {
        while self.it.next_if(|c| c.is_whitespace()).is_some() {}
    }

    fn take_while(&mut self, f: impl Fn(char) -> bool) -> String {
        let mut s = String::new();
        while let Some(c) = self.it.next_if(|c| f(*c)) {
            s.push(c);
        }
        s
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.next().unwrap();
        let mut s = String::new();
        loop {
            match self.next() {
                None => return Err("falta cerrar las comillas".into()),
                Some(c) if c == quote => return Ok(s),
                Some('\\') if quote == '"' => match self.next() {
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(c) => return Err(format!("escape desconocido «\\{c}» (para rutas, usá comillas simples: 'C:\\carpeta')")),
                    None => return Err("falta cerrar las comillas".into()),
                },
                Some(c) => s.push(c),
            }
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.peek() {
            None => Err("falta el valor después de '='".into()),
            Some('"') | Some('\'') => self.string().map(Value::Str),
            Some('[') => {
                self.next();
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        Some(']') => {
                            self.next();
                            return Ok(Value::List(items));
                        }
                        None => return Err("falta cerrar la lista con ']'".into()),
                        _ => {}
                    }
                    items.push(self.value()?);
                    self.skip_ws();
                    match self.next() {
                        Some(',') => {}
                        Some(']') => return Ok(Value::List(items)),
                        _ => return Err("en una lista, los elementos van separados por comas".into()),
                    }
                }
            }
            Some(_) => {
                let word = self.take_while(|c| c != ',' && c != ']' && !c.is_whitespace());
                match word.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    w => {
                        let w = w.replace('_', "");
                        if let Ok(i) = w.parse::<i64>() {
                            Ok(Value::Int(i))
                        } else if let Ok(f) = w.parse::<f64>() {
                            Ok(Value::Float(f))
                        } else {
                            Err(format!("valor no reconocido «{word}» (¿faltan las comillas?)"))
                        }
                    }
                }
            }
        }
    }
}

/// String TOML entre comillas dobles, con escapes.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_subset() {
        let doc = parse(
            r##"
# comentario
suelto = 1
[atajos]
pestana_nueva = "Ctrl+T"   # al final
cerrar = ["Ctrl+W", "Ctrl+F4"]
apagado = ""
[colores]
"*.pdf" = "#E5534B"
'@oculto' = "#768390 # no es comentario"
[[grupo]]
nombre = "Trabajo"
layout = 4
division = 0.35
paneles = [['C:\dev'], ["D:\\x", 'E:\y'], []]
activo = true
"##,
        );
        assert!(doc.errors.is_empty(), "{:?}", doc.errors);
        assert_eq!(doc.sections[0].get("suelto"), Some(&Value::Int(1)));
        let atajos = doc.sections("atajos").next().unwrap();
        assert_eq!(atajos.get("pestana_nueva").and_then(Value::as_str), Some("Ctrl+T"));
        assert_eq!(atajos.get("cerrar").and_then(Value::as_str_list), Some(vec!["Ctrl+W", "Ctrl+F4"]));
        let colores = doc.sections("colores").next().unwrap();
        assert_eq!(colores.entries[1].key, "@oculto");
        assert_eq!(colores.entries[1].value, Value::Str("#768390 # no es comentario".into()));
        let g = doc.sections("grupo").next().unwrap();
        assert!(g.array);
        assert_eq!(g.get("division").and_then(Value::as_float), Some(0.35));
        let paneles = g.get("paneles").unwrap();
        assert_eq!(
            paneles,
            &Value::List(vec![
                Value::List(vec![Value::Str(r"C:\dev".into())]),
                Value::List(vec![Value::Str(r"D:\x".into()), Value::Str(r"E:\y".into())]),
                Value::List(vec![]),
            ])
        );
    }

    #[test]
    fn reports_errors_by_line() {
        let doc = parse("ok = 1\nmal\nruta = \"C:\\dev\"\nlista = [1, 2\nsobra = 1 2\n");
        let lines: Vec<usize> = doc.errors.iter().map(|(l, _)| *l).collect();
        assert_eq!(lines, [2, 3, 4, 5]);
        assert!(doc.errors[1].1.contains("comillas simples"));
        assert_eq!(doc.sections[0].get("ok"), Some(&Value::Int(1)));
    }

    #[test]
    fn quote_round_trips() {
        let s = "C:\\a \"b\"\tc";
        let doc = parse(&format!("x = {}", quote(s)));
        assert_eq!(doc.sections[0].get("x").and_then(Value::as_str), Some(s));
    }
}

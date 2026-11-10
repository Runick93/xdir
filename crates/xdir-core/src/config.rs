//! `config.toml`: atajos y colores por tipo de archivo, editables a mano. Va junto a la sesión.
//! La primera vez se escribe una plantilla comentada con todo lo que se puede cambiar; después
//! xdir solo lo lee (nunca lo reescribe, para no perder lo que se editó) y lo vuelve a leer
//! cuando cambia (al volver a la ventana), así los cambios se aplican sin reiniciar.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::commands::{self, Keymap};
use crate::geom::Color;
use crate::session;
use crate::toml;

const FILE_NAME: &str = "config.toml";

/// Atributos de archivo que se pueden colorear con `"@nombre"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attr {
    Folder,
    Hidden,
    System,
    ReadOnly,
    Compressed,
    Encrypted,
}

const ATTRS: &[(&str, Attr)] = &[
    ("carpeta", Attr::Folder),
    ("oculto", Attr::Hidden),
    ("sistema", Attr::System),
    ("solo_lectura", Attr::ReadOnly),
    ("comprimido", Attr::Compressed),
    ("cifrado", Attr::Encrypted),
];

#[derive(Clone, Debug, PartialEq)]
pub enum ColorMatch {
    /// Patrones de nombre (`*.pdf`, `informe*`), en minúsculas.
    Names(Vec<String>),
    Attr(Attr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorRule {
    pub matches: ColorMatch,
    pub color: Color,
}

#[derive(Debug)]
pub struct Config {
    pub keymap: Keymap,
    /// En orden: gana la primera regla que coincide.
    pub colors: Vec<ColorRule>,
    /// `[opciones] animaciones`: forzar las animaciones sutiles (scroll suave, transición de layout)
    /// o apagarlas. Sin la opción, se sigue la preferencia de Windows.
    pub animations: Option<bool>,
    /// `[opciones] arbol_sigue_panel`: el árbol de carpetas despliega y muestra la carpeta del
    /// panel activo cuando cambia (por defecto, sí).
    pub tree_follows_pane: bool,
    /// Problemas al leer el archivo, para avisar en la barra de estado.
    pub errors: Vec<String>,
    /// Fecha del archivo leído, para saber si cambió.
    modified: Option<SystemTime>,
}

impl Default for Config {
    fn default() -> Self {
        Self { keymap: Keymap::default(), colors: Vec::new(), animations: None, tree_follows_pane: true, errors: Vec::new(), modified: None }
    }
}

pub fn file_path() -> Option<PathBuf> {
    session::config_dir().map(|d| d.join(FILE_NAME))
}

fn modified_time() -> Option<SystemTime> {
    std::fs::metadata(file_path()?).and_then(|m| m.modified()).ok()
}

impl Config {
    /// Lee `config.toml`; si no existe, escribe la plantilla.
    pub fn load() -> Self {
        let Some(path) = file_path() else { return Self::default() };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, template());
        }
        let mut c = match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(e) => Self { errors: vec![format!("no se pudo leer: {e}")], ..Self::default() },
        };
        c.modified = modified_time();
        c
    }

    /// ¿Cambió el archivo desde la última lectura?
    pub fn changed_on_disk(&self) -> bool {
        modified_time() != self.modified
    }

    pub fn parse(text: &str) -> Self {
        let mut c = Self::parse_rules(text);
        // Las reglas propias van primero (gana la primera que coincide); las de fábrica, al final.
        c.colors.extend(Self::parse_rules(DEFAULT_COLORS).colors);
        c
    }

    fn parse_rules(text: &str) -> Self {
        let doc = toml::parse(text);
        let mut c = Self::default();
        c.errors.extend(doc.errors.iter().map(|(l, e)| format!("línea {l}: {e}")));
        for section in &doc.sections {
            match section.name.as_str() {
                "" => {
                    for e in &section.entries {
                        c.errors.push(format!("línea {}: «{}» va dentro de [atajos] o [colores]", e.line, e.key));
                    }
                }
                "atajos" => {
                    for e in &section.entries {
                        let Some(spec) = commands::ALL.iter().find(|s| s.id == e.key) else {
                            c.errors.push(format!("línea {}: acción desconocida «{}»", e.line, e.key));
                            continue;
                        };
                        let Some(keys) = e.value.as_str_list() else {
                            c.errors.push(format!("línea {}: el atajo va entre comillas, o una lista de atajos", e.line));
                            continue;
                        };
                        let mut combos = Vec::new();
                        let mut ok = true;
                        for k in keys.into_iter().filter(|k| !k.trim().is_empty()) {
                            match commands::parse_combo(k) {
                                Ok(combo) => combos.push(combo),
                                Err(err) => {
                                    c.errors.push(format!("línea {}: {err}", e.line));
                                    ok = false;
                                }
                            }
                        }
                        // Con un atajo mal escrito la acción conserva los que tenía.
                        if ok {
                            c.keymap.set(spec.command, &combos);
                        }
                    }
                }
                "colores" => {
                    for e in &section.entries {
                        let Some(color) = e.value.as_str().and_then(parse_color) else {
                            c.errors.push(format!("línea {}: el color va como \"#RRGGBB\"", e.line));
                            continue;
                        };
                        let key = e.key.trim();
                        let matches = if let Some(name) = key.strip_prefix('@') {
                            match ATTRS.iter().find(|(n, _)| *n == name) {
                                Some((_, a)) => ColorMatch::Attr(*a),
                                None => {
                                    let known: Vec<&str> = ATTRS.iter().map(|(n, _)| *n).collect();
                                    c.errors.push(format!("línea {}: atributo desconocido «@{name}» (hay: {})", e.line, known.join(", ")));
                                    continue;
                                }
                            }
                        } else {
                            ColorMatch::Names(key.split(';').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect())
                        };
                        c.colors.push(ColorRule { matches, color });
                    }
                }
                "opciones" => {
                    for e in &section.entries {
                        match (e.key.as_str(), e.value.as_bool()) {
                            ("animaciones", Some(on)) => c.animations = Some(on),
                            ("animaciones", None) => c.errors.push(format!("línea {}: animaciones va como true o false", e.line)),
                            ("arbol_sigue_panel", Some(on)) => c.tree_follows_pane = on,
                            ("arbol_sigue_panel", None) => c.errors.push(format!("línea {}: arbol_sigue_panel va como true o false", e.line)),
                            (other, _) => c.errors.push(format!("línea {}: opción desconocida «{other}»", e.line)),
                        }
                    }
                }
                name => c.errors.push(format!("línea {}: sección desconocida «{name}»", section.line.max(1))),
            }
        }
        c
    }
}

/// Color del nombre según la primera regla que coincide. `name` en minúsculas.
pub fn color_for(rules: &[ColorRule], name: &str, is_dir: bool, attrs: u32) -> Option<Color> {
    rules
        .iter()
        .find(|r| match &r.matches {
            ColorMatch::Names(patterns) => patterns.iter().any(|p| {
                if p.contains(['*', '?']) { wildcard(p, name) } else { p == name }
            }),
            ColorMatch::Attr(a) => match a {
                Attr::Folder => is_dir,
                Attr::Hidden => attrs & 0x2 != 0,
                Attr::System => attrs & 0x4 != 0,
                Attr::ReadOnly => attrs & 0x1 != 0,
                Attr::Compressed => attrs & 0x800 != 0,
                Attr::Encrypted => attrs & 0x4000 != 0,
            },
        })
        .map(|r| r.color)
}

/// Comodines de Windows: `*` cualquier secuencia, `?` un carácter. Sin recursión: vuelve al
/// último `*` cuando algo no coincide.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// "#RRGGBB" → color.
fn parse_color(s: &str) -> Option<Color> {
    let hex = s.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(Color {
        r: ((v >> 16) & 0xFF) as f32 / 255.0,
        g: ((v >> 8) & 0xFF) as f32 / 255.0,
        b: (v & 0xFF) as f32 / 255.0,
        a: 1.0,
    })
}

/// Colores por tipo de archivo de fábrica, detrás de los de `[colores]`. El switch "Colores por
/// tipo" de la rueda dentada los apaga junto con los propios.
const DEFAULT_COLORS: &str = r##"[colores]
"*.pdf" = "#E5534B"
"*.jpg;*.jpeg;*.png;*.gif;*.bmp;*.webp;*.svg;*.ico;*.heic" = "#57AB5A"
"*.zip;*.7z;*.rar;*.tar;*.gz;*.iso" = "#C69026"
"*.exe;*.msi;*.bat;*.cmd;*.ps1;*.com" = "#B083F0"
"*.mp3;*.wav;*.flac;*.ogg;*.m4a;*.mp4;*.mkv;*.avi;*.mov" = "#39C5CF"
"*.doc;*.docx;*.odt;*.rtf" = "#4F8CE8"
"*.xls;*.xlsx;*.csv;*.ods" = "#3FA45F"
"*.ppt;*.pptx;*.odp" = "#E8833A"
"*.rs;*.py;*.js;*.ts;*.c;*.cpp;*.h;*.cs;*.java;*.go;*.json;*.toml;*.html;*.css" = "#8FB4E8"
"@oculto" = "#768390"
"##;

/// Plantilla de la primera vez: todos los atajos de fábrica, comentados, y ejemplos de colores.
pub fn template() -> String {
    let mut out = String::from(
        "# Configuración de xdir. Se aplica al guardar el archivo y volver a la ventana de xdir.\n\
         # Si algo está mal escrito, la barra de estado avisa en qué línea.\n\
         \n\
         [atajos]\n\
         # Para cambiar un atajo, sacá el '#' de su línea y escribí el nuevo. Se pueden poner\n\
         # varios: pestana_nueva = [\"Ctrl+T\", \"Ctrl+N\"]. Con \"\" la acción queda sin atajo.\n\
         # Teclas: letras, números, F1..F24, Tab, Enter, Supr, Insert, Retroceso, Esc, Espacio,\n\
         # Izquierda, Derecha, Arriba, Abajo, Inicio, Fin, RePag, AvPag; con Ctrl+, Shift+ y Alt+.\n\
         # Si una tecla ya la usaba otra acción, pasa a la nueva.\n",
    );
    let width = commands::ALL.iter().map(|s| s.id.len()).max().unwrap_or(0);
    for s in commands::ALL.iter().filter(|s| commands::available(s.command)) {
        let keys: Vec<String> = s.keys().iter().map(|k| toml::quote(k)).collect();
        let value = match keys.len() {
            0 => "\"\"".to_string(),
            1 => keys[0].clone(),
            _ => format!("[{}]", keys.join(", ")),
        };
        out += &format!("# {:width$} = {:<24} # {}\n", s.id, value, s.title(), width = width);
    }
    out += "\n\
            [colores]\n\
            # Color del nombre según el tipo de archivo: patrón = \"#RRGGBB\". Gana la primera regla\n\
            # que coincide. Patrones con * y ?, varios separados por ';'. Con @ se elige por\n\
            # atributo: @carpeta, @oculto, @sistema, @solo_lectura, @comprimido, @cifrado.\n\
            # Ya vienen colores de fábrica (PDF, imágenes, comprimidos, ejecutables, audio y video,\n\
            # Office, código); lo que pongas acá tiene prioridad. Ejemplos:\n\
            # \"*.pdf\"                      = \"#E5534B\"\n\
            # \"*.jpg;*.jpeg;*.png;*.gif\"   = \"#57AB5A\"\n\
            # \"*.zip;*.7z;*.rar\"           = \"#C69026\"\n\
            # \"*.exe;*.msi;*.bat;*.cmd\"    = \"#B083F0\"\n\
            # \"@oculto\"                    = \"#768390\"\n\
            \n\
            [opciones]\n\
            # Animaciones sutiles (scroll suave y transición al cambiar de layout). Sin esta opción\n\
            # se sigue a Windows (Accesibilidad → Efectos visuales → Mostrar animaciones);\n\
            # true o false lo fuerza.\n\
            # animaciones = true\n\
            # El árbol de carpetas despliega y muestra la carpeta del panel activo cuando cambia;\n\
            # false lo apaga.\n\
            # arbol_sigue_panel = true\n";
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Key;

    #[test]
    fn template_parses_clean_and_keeps_defaults() {
        crate::sys::testing::install_fakes();
        let c = Config::parse(&template());
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        // Sin reglas propias quedan solo las de fábrica.
        assert_eq!(c.colors.len(), Config::parse_rules(DEFAULT_COLORS).colors.len());
        assert_eq!(c.keymap.label(commands::Command::NewTab), commands::shortcut_label("Ctrl+T"));
        // Sacar los '#' de las líneas de atajos tiene que dar exactamente lo mismo.
        let uncommented: String = template()
            .lines()
            .map(|l| {
                // Solo las líneas de ejemplo ("# clave = valor"), no los textos explicativos.
                let is_example = l.starts_with("# ") && l[2..].split_once(" = ").is_some_and(|(k, _)| !k.trim_end().contains(' '));
                if is_example { &l[2..] } else { l }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let c2 = Config::parse(&uncommented);
        assert!(c2.errors.is_empty(), "{:?}", c2.errors);
        assert_eq!(c2.colors.len(), 5 + Config::parse_rules(DEFAULT_COLORS).colors.len());
        for s in commands::ALL {
            assert_eq!(c2.keymap.label(s.command), c.keymap.label(s.command), "{}", s.id);
        }
    }

    #[test]
    fn animations_option() {
        crate::sys::testing::install_fakes();
        assert_eq!(Config::parse("").animations, None, "sin la opción se sigue a Windows");
        let off = Config::parse("[opciones]\nanimaciones = false\n");
        assert!(off.animations == Some(false) && off.errors.is_empty());
        assert_eq!(Config::parse("[opciones]\nanimaciones = true\n").animations, Some(true));
        let bad = Config::parse("[opciones]\nanimaciones = \"no\"\nzoom = 2\n");
        assert_eq!(bad.animations, None);
        assert_eq!(bad.errors.len(), 2, "{:?}", bad.errors);
    }

    #[test]
    fn tree_follows_pane_option() {
        crate::sys::testing::install_fakes();
        assert!(Config::parse("").tree_follows_pane, "por defecto el árbol sigue al panel");
        let off = Config::parse("[opciones]\narbol_sigue_panel = false\n");
        assert!(!off.tree_follows_pane && off.errors.is_empty(), "{:?}", off.errors);
        let bad = Config::parse("[opciones]\narbol_sigue_panel = 0\n");
        assert!(bad.tree_follows_pane);
        assert_eq!(bad.errors.len(), 1, "{:?}", bad.errors);
    }

    /// Un atajo con Meta en `[atajos]` se lee y no choca con el mismo sin Meta.
    #[test]
    fn meta_shortcut_in_config() {
        crate::sys::testing::install_fakes();
        let c = Config::parse("[atajos]\ncarpeta_con_seleccion = \"Meta+Ctrl+K\"\n");
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        let k = Key(b'K' as u16);
        assert_eq!(c.keymap.lookup(k, true, false, false, true), Some(commands::Command::NewFolderWithSelection));
        // Ctrl+K sin Meta sigue siendo «Conectar a un servidor…».
        assert_eq!(c.keymap.lookup(k, true, false, false, false), Some(commands::Command::ConnectServer));
    }

    #[test]
    fn user_changes_and_errors() {
        crate::sys::testing::install_fakes();
        let c = Config::parse(
            "[atajos]\npestana_nueva = \"Ctrl+K\"\ninexistente = \"Ctrl+J\"\nrecargar = \"Ctrl+Hiper\"\n\
             [colores]\n\"*.PDF; *.ps\" = \"#FF0000\"\n'@oculto' = \"#00FF00\"\n'@raro' = \"#000000\"\n\"*.x\" = \"rojo\"\n[otra]\n",
        );
        assert_eq!(c.keymap.lookup(Key(b'K' as u16), true, false, false, false), Some(commands::Command::NewTab));
        let own = 2;
        assert_eq!(c.colors.len(), own + Config::parse_rules(DEFAULT_COLORS).colors.len());
        assert_eq!(c.colors[0].matches, ColorMatch::Names(vec!["*.pdf".into(), "*.ps".into()]));
        assert_eq!(c.colors[1].matches, ColorMatch::Attr(Attr::Hidden));
        // Un color propio gana sobre el de fábrica del mismo tipo.
        assert_eq!(color_for(&c.colors, "a.pdf", false, 0).map(|c| c.r), Some(1.0));
        let lines: Vec<&str> = c.errors.iter().map(|e| e.split(':').next().unwrap()).collect();
        assert_eq!(lines, ["línea 3", "línea 4", "línea 8", "línea 9", "línea 10"], "{:?}", c.errors);
    }

    #[test]
    fn color_for_matches_wildcards() {
        crate::sys::testing::install_fakes();
        let c = Config::parse("[colores]\n\"informe*;img?.png\" = \"#FF0000\"\n\"datos.csv\" = \"#00FF00\"\n");
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        let red = |name: &str| color_for(&c.colors, name, false, 0).is_some_and(|c| c.r == 1.0 && c.g == 0.0);
        assert!(red("informe final.docx"));
        assert!(red("img2.png"));
        assert!(!red("img10.png"), "? es un solo carácter");
        assert!(!red("mi informe.docx"), "sin * al principio tiene que empezar igual");
        // Sin comodines el nombre tiene que ser exacto.
        assert_eq!(color_for(&c.colors, "datos.csv", false, 0).map(|c| c.g), Some(1.0));
        assert_eq!(color_for(&c.colors, "datos.csv.bak", false, 0), None);
    }
}

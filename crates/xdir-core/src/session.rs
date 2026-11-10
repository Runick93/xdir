//! Sesión: layout, divisores, paneles con sus pestañas y posición de la ventana.
//!
//! Se guarda en un subconjunto chico de TOML, escrito y leído a mano para no sumar
//! dependencias: `clave = valor`, tablas `[[panel]]`, strings, números, booleanos y listas en
//! una línea. Va en `%APPDATA%\xdir\`, o junto al .exe si existe `xdir.portable` (modo portable).

use std::path::{Path, PathBuf};

use crate::folder_views::ViewSettings;
use crate::fs::Location;
use crate::list::{Column, ColumnSpec, View, default_columns};
use crate::theme::{Accent, DEFAULT_ACCENT};

const COMPUTER: &str = "Este equipo";
/// Carpetas virtuales: "shell:<nombre de análisis>|<nombre visible>".
const SHELL_PREFIX: &str = "shell:";

/// Pestañas de un panel y cuál está activa.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneState {
    pub tabs: Vec<Location>,
    pub active: usize,
}

/// Carpeta de configuración: junto al ejecutable en modo portable (si ahí existe
/// `xdir.portable`), si no la del usuario que da la plataforma.
pub fn config_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    resolve_config_dir(exe_dir.as_deref(), || crate::sys::env().config_dir())
}

/// La regla de `config_dir`, sin efectos: `exe_dir` es la carpeta del ejecutable y `platform`
/// la carpeta del usuario (solo se pide si no hay modo portable).
pub fn resolve_config_dir(exe_dir: Option<&Path>, platform: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    match exe_dir {
        Some(dir) if dir.join("xdir.portable").exists() => Some(dir.to_path_buf()),
        _ => platform(),
    }
}

pub fn location_str(l: &Location) -> String {
    match l {
        Location::Computer => COMPUTER.to_string(),
        Location::Path(p) => p.to_string_lossy().into_owned(),
        Location::Shell { parsing, name, .. } => format!("{SHELL_PREFIX}{parsing}|{name}"),
    }
}

pub fn location_from(s: &str) -> Location {
    if s == COMPUTER {
        return Location::Computer;
    }
    if let Some((parsing, name)) = s.strip_prefix(SHELL_PREFIX).and_then(|r| r.rsplit_once('|')) {
        return Location::Shell { parsing: parsing.to_string(), name: name.to_string(), parent: None };
    }
    Location::Path(PathBuf::from(s))
}

const FILE_NAME: &str = "sesion.toml";

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// Posición y tamaño de la ventana restaurada, en píxeles.
    pub window: Option<[i32; 4]>,
    pub maximized: bool,
    pub layout: usize,
    pub split_x: f32,
    pub split_y: f32,
    pub active_pane: usize,
    pub show_hidden: bool,
    pub show_favorites: bool,
    /// Barra de pestañas de grupos habilitada (aparece solo si hay grupos).
    pub group_bar: bool,
    pub accent: Accent,
    /// Modo fijado por el usuario; None sigue a Windows.
    pub dark_mode: Option<bool>,
    /// Colores por tipo de archivo (reglas de `config.toml`) encendidos.
    pub file_colors: bool,
    /// Nombres de archivo con su extensión (switch «Mostrar extensiones»).
    pub show_extensions: bool,
    /// Casillas de selección en las listas (switch «Casillas de selección»).
    pub checkboxes: bool,
    /// Switch «El árbol sigue al panel», si el usuario lo tocó alguna vez; `None` usa
    /// `arbol_sigue_panel` de `config.toml`.
    pub tree_follow: Option<bool>,
    /// Árbol de carpetas lateral visible (solo con la feature `arbol`).
    pub show_tree: bool,
    /// Árbol fijo al lado de los paneles (si no, queda la franja de íconos y se expande al asomar el mouse).
    pub tree_pinned: bool,
    /// Grupo "Acceso rápido" del árbol desplegado (se puede plegar para no scrollear tanto).
    pub quick_open: bool,
    /// Grupo de paneles activo (el que se guarda solo con cada cambio).
    pub group: Option<String>,
    /// Columna de vista previa prendida, y su ancho (240–640).
    pub preview_on: bool,
    pub preview_width: f32,
    /// Secciones abiertas del panel de información (un bit por `info::Section`).
    pub info_sections: u32,
    pub panes: Vec<PaneState>,
    /// Columnas de las listas, de izquierda a derecha. Desde `carpetas.toml` (memoria por
    /// carpeta) solo se leen para la migración (`legacy_view`); se siguen escribiendo con las de
    /// la vista por defecto para que una versión anterior las entienda.
    pub columns: Vec<ColumnSpec>,
    /// Vista de cada panel (detalles, íconos, miniaturas), en el orden de `panes`. Igual que
    /// `columns`: solo para la migración y para versiones anteriores.
    pub views: Vec<View>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            window: None,
            maximized: false,
            layout: 4,
            split_x: 0.5,
            split_y: 0.5,
            active_pane: 0,
            show_hidden: false,
            show_favorites: true,
            group_bar: false,
            accent: DEFAULT_ACCENT,
            dark_mode: None,
            file_colors: false,
            show_extensions: true,
            checkboxes: false,
            tree_follow: None,
            show_tree: true,
            tree_pinned: true,
            quick_open: true,
            group: None,
            preview_on: false,
            preview_width: crate::ui::PREVIEW_W,
            info_sections: crate::info::DEFAULT_OPEN,
            panes: Vec::new(),
            columns: default_columns(),
            views: Vec::new(),
        }
    }
}

impl Session {
    /// Vista por defecto que sale de una sesión de antes de `carpetas.toml` (migración): la vista
    /// del panel activo y las columnas, con el nombre sin ancho fijo (la sesión no lo guardaba).
    pub fn legacy_view(&self) -> ViewSettings {
        let view = self.views.get(self.active_pane).or(self.views.first()).copied().unwrap_or(View::Details);
        let columns = self.columns.iter().map(|c| if c.col == Column::Name { ColumnSpec { col: c.col, width: 0.0 } } else { *c }).collect();
        ViewSettings { view, columns, ..ViewSettings::default() }
    }
}

pub fn load() -> Option<Session> {
    let text = std::fs::read_to_string(config_dir()?.join(FILE_NAME)).ok()?;
    Some(parse(&text))
}

pub fn save(s: &Session) -> std::io::Result<()> {
    let dir = config_dir().ok_or(std::io::ErrorKind::NotFound)?;
    std::fs::create_dir_all(&dir)?;
    // Escritura atómica: si se corta a la mitad, la sesión anterior queda intacta.
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, write(s))?;
    std::fs::rename(&tmp, dir.join(FILE_NAME))
}

fn write(s: &Session) -> String {
    let mut out = String::from("# Sesión de xdir: se reescribe al cerrar.\n");
    if let Some([x, y, w, h]) = s.window {
        out += &format!("ventana = [{x}, {y}, {w}, {h}]\n");
    }
    out += &format!("maximizada = {}\n", s.maximized);
    out += &format!("layout = {}\n", s.layout);
    out += &format!("division_x = {:.4}\n", s.split_x);
    out += &format!("division_y = {:.4}\n", s.split_y);
    out += &format!("panel_activo = {}\n", s.active_pane);
    out += &format!("ocultos = {}\n", s.show_hidden);
    out += &format!("barra_favoritos = {}\n", s.show_favorites);
    out += &format!("barra_grupos = {}\n", s.group_bar);
    out += &format!("colores_por_tipo = {}
", s.file_colors);
    out += &format!("extensiones = {}\n", s.show_extensions);
    out += &format!("casillas = {}\n", s.checkboxes);
    if let Some(on) = s.tree_follow {
        out += &format!("arbol_sigue = {on}\n");
    }
    out += &format!("arbol = {}
", s.show_tree);
    out += &format!("arbol_fijo = {}
", s.tree_pinned);
    out += &format!("arbol_rapido = {}
", s.quick_open);
    out += &format!("vista_previa = {}\n", s.preview_on);
    out += &format!("vista_previa_ancho = {}\n", s.preview_width.round());
    out += &format!("informacion_secciones = {}\n", s.info_sections);
    if let Some(g) = &s.group {
        out += &format!("grupo = {}
", quote(g));
    }
    out += &match s.accent {
        Accent::System => "acento = \"sistema\"\n".to_string(),
        Accent::Rgb(c) => format!("acento = \"#{c:06X}\"\n"),
    };
    out += match s.dark_mode {
        None => "modo = \"sistema\"\n",
        Some(true) => "modo = \"oscuro\"\n",
        Some(false) => "modo = \"claro\"\n",
    };
    // "nombre", "modificado:160", ...: el nombre no tiene ancho (ocupa lo que sobra).
    let cols: Vec<String> = s
        .columns
        .iter()
        .map(|c| if c.col == Column::Name { quote(c.col.id()) } else { quote(&format!("{}:{}", c.col.id(), c.width.round())) })
        .collect();
    out += &format!("columnas = [{}]\n", cols.join(", "));
    let views: Vec<String> = s.views.iter().map(|v| quote(v.id())).collect();
    out += &format!("vistas = [{}]\n", views.join(", "));
    for pane in &s.panes {
        out += "\n[[panel]]\n";
        out += &format!("pestana_activa = {}\n", pane.active);
        let tabs: Vec<String> = pane.tabs.iter().map(|t| quote(&location_str(t))).collect();
        out += &format!("pestanas = [{}]\n", tabs.join(", "));
    }
    out
}

fn parse(text: &str) -> Session {
    let mut s = Session::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[panel]]" {
            s.panes.push(PaneState::default());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        let int = || value.parse::<i64>().ok();
        let float = || value.parse::<f32>().ok();
        let boolean = || value.parse::<bool>().ok();
        match (key, s.panes.last_mut()) {
            ("pestana_activa", Some(p)) => p.active = int().unwrap_or(0).max(0) as usize,
            ("pestanas", Some(p)) => {
                p.tabs = parse_list(value).iter().map(|v| location_from(v)).collect();
            }
            ("ventana", None) => {
                let nums: Vec<i32> = parse_list(value).iter().filter_map(|v| v.parse().ok()).collect();
                if let [x, y, w, h] = nums[..] {
                    s.window = Some([x, y, w, h]);
                }
            }
            ("maximizada", None) => s.maximized = boolean().unwrap_or(false),
            ("vistas", None) => {
                s.views = parse_list(value).iter().map(|v| View::from_id(v).unwrap_or(View::Details)).collect();
            }
            ("columnas", None) => {
                let mut cols: Vec<ColumnSpec> = parse_list(value)
                    .iter()
                    .filter_map(|item| {
                        let (id, width) = item.split_once(':').unwrap_or((item, ""));
                        let col = Column::from_id(id.trim())?;
                        let width = width.trim().parse::<f32>().unwrap_or(0.0).max(48.0);
                        Some(ColumnSpec { col, width })
                    })
                    .collect();
                cols.dedup_by_key(|c| c.col);
                // Sin el nombre la lista no sirve: se vuelve a las de fábrica.
                if cols.iter().any(|c| c.col == Column::Name) {
                    s.columns = cols;
                }
            }
            ("layout", None) => s.layout = int().unwrap_or(4).clamp(0, 4) as usize,
            ("division_x", None) => s.split_x = float().unwrap_or(0.5),
            ("division_y", None) => s.split_y = float().unwrap_or(0.5),
            ("panel_activo", None) => s.active_pane = int().unwrap_or(0).max(0) as usize,
            ("ocultos", None) => s.show_hidden = boolean().unwrap_or(false),
            ("colores_por_tipo", None) => s.file_colors = boolean().unwrap_or(false),
            ("extensiones", None) => s.show_extensions = boolean().unwrap_or(true),
            ("casillas", None) => s.checkboxes = boolean().unwrap_or(false),
            ("arbol_sigue", None) => s.tree_follow = boolean(),
            ("arbol", None) => s.show_tree = boolean().unwrap_or(true),
            ("arbol_fijo", None) => s.tree_pinned = boolean().unwrap_or(true),
            ("vista_previa", None) => s.preview_on = boolean().unwrap_or(false),
            ("vista_previa_ancho", None) => {
                s.preview_width = float().filter(|w| w.is_finite()).unwrap_or(crate::ui::PREVIEW_W).clamp(crate::ui::PREVIEW_MIN_W, crate::ui::PREVIEW_MAX_W);
            }
            ("informacion_secciones", None) => s.info_sections = int().filter(|n| (0..128).contains(n)).map_or(crate::info::DEFAULT_OPEN, |n| n as u32),
            ("arbol_rapido", None) => s.quick_open = boolean().unwrap_or(true),
            ("grupo", None) => s.group = Some(value.trim_matches('"').to_string()).filter(|g| !g.is_empty()),
            ("barra_favoritos", None) => s.show_favorites = boolean().unwrap_or(true),
            ("barra_grupos", None) => s.group_bar = boolean().unwrap_or(false),
            ("modo", None) => {
                s.dark_mode = match value.trim_matches('"') {
                    "oscuro" => Some(true),
                    "claro" => Some(false),
                    _ => None,
                };
            }
            ("acento", None) => {
                let v = value.trim_matches('"');
                s.accent = match v.strip_prefix('#').and_then(|h| u32::from_str_radix(h, 16).ok()) {
                    Some(c) if v.len() == 7 => Accent::Rgb(c),
                    _ if v == "sistema" => Accent::System,
                    _ => DEFAULT_ACCENT,
                };
            }
            _ => {}
        }
    }
    s.panes.retain(|p| !p.tabs.is_empty());
    s
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Lista de una línea: `["a", "b"]` o `[1, 2]`. Devuelve los elementos sin comillas.
fn parse_list(value: &str) -> Vec<String> {
    let inner = value.trim().trim_start_matches('[').trim_end_matches(']');
    let mut items = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
        match chars.peek() {
            None => break,
            Some('"') => {
                chars.next();
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some(other) => {
                                s.push('\\');
                                s.push(other);
                            }
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                items.push(s);
            }
            Some(_) => {
                let mut s = String::new();
                while let Some(c) = chars.next_if(|c| *c != ',') {
                    s.push(c);
                }
                items.push(s.trim().to_string());
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_mode_wins() {
        let exe_dir = std::env::temp_dir().join(format!("xdir-portable-{}", std::process::id()));
        std::fs::create_dir_all(&exe_dir).unwrap();
        // Sin la marca, manda la plataforma.
        let platform = PathBuf::from("/plataforma/xdir");
        assert_eq!(resolve_config_dir(Some(&exe_dir), || Some(platform.clone())), Some(platform.clone()));
        // Con `xdir.portable` junto al ejecutable, esa carpeta, sin preguntarle a la plataforma.
        std::fs::write(exe_dir.join("xdir.portable"), "").unwrap();
        let got = resolve_config_dir(Some(&exe_dir), || panic!("no debía llamar a la plataforma"));
        assert_eq!(got.as_deref(), Some(exe_dir.as_path()));
        // Sin ejecutable conocido, la plataforma.
        assert_eq!(resolve_config_dir(None::<&Path>, || Some(platform.clone())), Some(platform));
        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    /// Guardar y leer con `config_dir` del falso (carpeta temporal por hilo) da la misma sesión.
    #[test]
    fn session_roundtrip_in_core() {
        crate::sys::testing::install_fakes();
        let dir = config_dir().expect("carpeta de configuración del falso");
        // El falso arma la carpeta con `temp_dir()` tal cual: si no cuelga de ahí (un
        // `xdir.portable` junto al binario de pruebas), no se borra nada.
        assert!(dir.starts_with(std::env::temp_dir()), "carpeta de configuración fuera de temp: {}", dir.display());
        let _ = std::fs::remove_dir_all(&dir);
        let s = Session {
            accent: Accent::Rgb(0xF85149),
            panes: vec![
                PaneState { tabs: vec![Location::Path(std::env::temp_dir()), Location::Computer], active: 1 },
                PaneState { tabs: vec![Location::Path(PathBuf::from("a b, c"))], active: 0 },
            ],
            columns: vec![
                ColumnSpec { col: Column::Name, width: 48.0 },
                ColumnSpec { col: Column::Size, width: 120.0 },
            ],
            views: vec![View::Icons, View::Details],
            ..Session::default()
        };
        assert_ne!(s.columns, default_columns());
        save(&s).unwrap();
        assert_eq!(load(), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip() {
        let s = Session {
            window: Some([10, 20, 1200, 800]),
            maximized: true,
            layout: 3,
            split_x: 0.3,
            split_y: 0.7,
            active_pane: 2,
            show_hidden: true,
            show_favorites: false,
            group_bar: false,
            accent: Accent::Rgb(0xA371F7),
            dark_mode: Some(false),
            file_colors: false,
            show_extensions: false,
            checkboxes: true,
            tree_follow: Some(false),
            show_tree: true,
            tree_pinned: false,
            quick_open: true,
            group: Some("Trabajo".into()),
            preview_on: false,
            preview_width: 320.0,
            info_sections: 0b101,
            panes: vec![
                PaneState {
                    tabs: vec![
                        Location::Path(r#"C:\Users\x "raro""#.into()),
                        Location::Computer,
                        Location::Shell {
                            parsing: "::{645FF040-5081-101B-9F08-00AA002F954E}".into(),
                            name: "Papelera de reciclaje".into(),
                            parent: None,
                        },
                    ],
                    active: 1,
                },
                PaneState { tabs: vec![Location::Path(r"D:\a, b\c".into())], active: 0 },
            ],
            columns: vec![
                ColumnSpec { col: Column::Size, width: 90.0 },
                ColumnSpec { col: Column::Name, width: 48.0 },
                ColumnSpec { col: Column::Created, width: 150.0 },
            ],
            views: vec![View::Thumbnails, View::Details, View::Icons],
        };
        assert_eq!(parse(&write(&s)), s);
        // Sin la columna del nombre, las de fábrica.
        assert_eq!(parse("columnas = [\"tamano:90\"]\n").columns, default_columns());
        // Las sesiones de antes (sin la clave) muestran las extensiones.
        assert!(parse("layout = 2\n").show_extensions);
    }

    /// Las vistas Lista y Contenido se guardan con sus ids («lista», «contenido») y vuelven igual,
    /// tanto en la sesión como en la vista de una carpeta (`carpetas.toml`).
    #[test]
    fn view_ids_roundtrip_session_and_folders() {
        assert_eq!((View::List.id(), View::Content.id()), ("lista", "contenido"));
        assert_eq!((View::List.name(), View::Content.name()), ("Lista", "Contenido"));
        let s = Session { views: vec![View::List, View::Content, View::Details], ..Session::default() };
        let text = write(&s);
        assert!(text.contains("vistas = [\"lista\", \"contenido\", \"detalles\"]"), "{text}");
        assert_eq!(parse(&text).views, s.views);

        use crate::folder_views::{FolderViews, ViewSettings};
        let mut fv = FolderViews::default();
        fv.apply_to_all(ViewSettings { view: View::Content, ..ViewSettings::default() });
        fv.set("/a", ViewSettings { view: View::List, ..ViewSettings::default() });
        let text = crate::folder_views::write(&fv);
        let (back, errors) = crate::folder_views::parse(&text);
        assert!(errors.is_empty(), "{errors:?}\n{text}");
        assert_eq!(back.default_view().view, View::Content);
        assert_eq!(back.get("/a").view, View::List);
        assert!(text.contains("\"lista\"") && text.contains("\"contenido\""), "{text}");

        // La Galería (al final de `VIEWS`), igual.
        assert_eq!(View::Gallery.id(), "galeria");
        let s = Session { views: vec![View::Gallery, View::Details], ..Session::default() };
        let text = write(&s);
        assert!(text.contains("vistas = [\"galeria\", \"detalles\"]"), "{text}");
        assert_eq!(parse(&text).views, s.views);
        let mut fv = FolderViews::default();
        fv.set("/fotos", ViewSettings { view: View::Gallery, ..ViewSettings::default() });
        let text = crate::folder_views::write(&fv);
        assert!(text.contains("\"galeria\""), "{text}");
        let (back, errors) = crate::folder_views::parse(&text);
        assert!(errors.is_empty(), "{errors:?}\n{text}");
        assert_eq!(back.get("/fotos").view, View::Gallery);
    }

    /// La columna de vista previa: prendida y su ancho se guardan y vuelven; un ancho fuera de los
    /// límites se recorta al leer; sin la clave, apagada y de 320.
    /// Secciones abiertas del panel de información: se guardan como bits y faltando se usan las
    /// de fábrica.
    #[test]
    fn info_sections_persist() {
        let s = Session { info_sections: 0b0100_0011, ..Session::default() };
        let text = write(&s);
        assert!(text.contains("informacion_secciones = 67"), "{text}");
        assert_eq!(parse(&text).info_sections, 0b0100_0011);
        assert_eq!(parse("").info_sections, crate::info::DEFAULT_OPEN);
        assert_eq!(Session::default().info_sections, crate::info::DEFAULT_OPEN);
    }

    #[test]
    fn preview_session_roundtrip() {
        let s = Session { preview_on: true, preview_width: 410.0, ..Session::default() };
        let text = write(&s);
        assert!(text.contains("vista_previa = true") && text.contains("vista_previa_ancho = 410"), "{text}");
        let back = parse(&text);
        assert_eq!((back.preview_on, back.preview_width), (true, 410.0));
        assert_eq!(parse("vista_previa_ancho = 9999\n").preview_width, 640.0);
        assert_eq!(parse("vista_previa_ancho = 10\n").preview_width, 240.0);
        let d = parse("layout = 2\n");
        assert_eq!((d.preview_on, d.preview_width), (false, 320.0));
    }

    #[test]
    fn tolerates_garbage() {
        let s = parse("esto no es toml\nlayout = 99\n[[panel]]\npestanas = []\n");
        assert_eq!(s.layout, 4);
        assert!(s.panes.is_empty());
    }
}

//! Memoria de vista por carpeta, como el Explorador: cada carpeta recuerda su vista, su orden,
//! sus columnas con los anchos (y, desde las tareas de agrupar y filtrar, la agrupación, los
//! grupos plegados y los filtros). Las carpetas nunca tocadas usan la vista por defecto.
//!
//! Se guarda en `carpetas.toml`, en la carpeta de configuración (aparte de `sesion.toml`, que se
//! reescribe seguido), con el parser `toml` del núcleo. Guarda como mucho `MAX_FOLDERS` carpetas:
//! cuando se pasa, se descartan las usadas hace más tiempo. Un archivo roto no impide arrancar:
//! lo que no se entiende se ignora y se informa por línea.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs::Location;
use crate::list::{Column, ColumnSpec, View, default_columns};
use crate::session::Session;
use crate::{sys, toml};

/// Cuántas carpetas se recuerdan como mucho.
pub const MAX_FOLDERS: usize = 2000;

const FILE_NAME: &str = "carpetas.toml";
const COMPUTER_KEY: &str = "equipo";
const SHELL_PREFIX: &str = "shell:";
const SEARCH_PREFIX: &str = "busqueda:";

/// Cómo se ve una carpeta.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewSettings {
    pub view: View,
    pub sort: Column,
    pub descending: bool,
    /// Columnas de izquierda a derecha, con sus anchos. El ancho del nombre es 0 mientras ocupa
    /// lo que sobra (no se lo fijó a mano).
    pub columns: Vec<ColumnSpec>,
    /// Columna por la que se agrupa (tarea de agrupar).
    pub group_by: Option<Column>,
    /// Grupos plegados, por nombre (tarea de agrupar).
    pub collapsed: Vec<String>,
    /// Filtros por columna: los grupos tildados de cada una (tarea de filtros).
    pub filters: Vec<(Column, Vec<String>)>,
}

impl Default for ViewSettings {
    /// La vista de fábrica: Detalles, por nombre, sin agrupar, las columnas de siempre.
    fn default() -> Self {
        let mut columns = default_columns();
        for c in &mut columns {
            if c.col == Column::Name {
                c.width = 0.0;
            }
        }
        Self { view: View::Details, sort: Column::Name, descending: false, columns, group_by: None, collapsed: Vec::new(), filters: Vec::new() }
    }
}

/// Clave de una ubicación en `carpetas.toml`: la ruta normalizada (sin separador final y, donde
/// el sistema no distingue mayúsculas, en minúsculas: misma clave para las rutas que
/// `same_path` considera iguales); `equipo` para «Este equipo»; `shell:<nombre de análisis>`
/// para las ubicaciones del Shell; `busqueda:<carpeta buscada>` para los resultados de una
/// búsqueda (`search_root`).
pub fn folder_key(loc: &Location, search_root: Option<&Path>) -> String {
    if let Some(root) = search_root {
        return format!("{SEARCH_PREFIX}{}", path_key(root));
    }
    match loc {
        Location::Computer => COMPUTER_KEY.to_string(),
        // Nunca se guarda vista para ella (ver `Tab::sync_view`).
        Location::Settings => "configuracion".to_string(),
        Location::Shell { parsing, .. } => format!("{SHELL_PREFIX}{parsing}"),
        Location::Path(p) => path_key(p),
    }
}

/// Clave de los resultados de una búsqueda en «Este equipo» (todas las unidades).
pub fn computer_search_key() -> String {
    format!("{SEARCH_PREFIX}{COMPUTER_KEY}")
}

/// Ruta normalizada como la compara el sistema: se le pregunta a `same_path` si distingue
/// mayúsculas y si la barra invertida es separador, así la clave sigue su misma regla sin
/// depender de `cfg` de sistema.
fn path_key(p: &Path) -> String {
    let fs = sys::file_system();
    let backslash = fs.same_path(Path::new("x\\"), Path::new("x"));
    let caseless = fs.same_path(Path::new("X"), Path::new("x"));
    let s = p.to_string_lossy();
    let trimmed = if backslash { s.trim_end_matches(['/', '\\']) } else { s.trim_end_matches('/') };
    // La raíz ("/") queda como está.
    let t = if trimmed.is_empty() { &s[..s.len().min(1)] } else { trimmed };
    if caseless { t.to_lowercase() } else { t.to_string() }
}

/// Las vistas recordadas de cada carpeta y la vista por defecto.
#[derive(Clone, Debug)]
pub struct FolderViews {
    default: ViewSettings,
    /// Vista de cada carpeta y cuándo se usó por última vez (reloj propio, creciente).
    entries: HashMap<String, (ViewSettings, u64)>,
    clock: u64,
    dirty: bool,
    /// No había `carpetas.toml`: la primera vez se copia la vista de la sesión (`migrate`).
    fresh: bool,
    /// Dónde se guarda; `None` no guarda (pruebas, `--probe`).
    path: Option<PathBuf>,
    /// Problemas al leer el archivo, uno por línea («línea N: …»).
    pub errors: Vec<String>,
}

impl Default for FolderViews {
    fn default() -> Self {
        Self { default: ViewSettings::default(), entries: HashMap::new(), clock: 0, dirty: false, fresh: true, path: None, errors: Vec::new() }
    }
}

impl FolderViews {
    /// Lee `carpetas.toml` de la carpeta de configuración. Sin archivo, queda «primera vez».
    pub fn load() -> FolderViews {
        let Some(path) = crate::session::config_dir().map(|d| d.join(FILE_NAME)) else { return FolderViews::default() };
        Self::load_from(path)
    }

    /// Lee las vistas de `path` y guarda ahí en adelante.
    fn load_from(path: PathBuf) -> FolderViews {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let (mut fv, errors) = parse(&text);
                fv.errors = errors;
                fv.path = Some(path);
                fv
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FolderViews { path: Some(path), ..FolderViews::default() },
            // Existe pero no se puede leer: se arranca con la de fábrica, se avisa y no se guarda
            // (pisarlo perdería la configuración del usuario).
            Err(e) => FolderViews { fresh: false, errors: vec![format!("no se pudo leer: {e}")], ..FolderViews::default() },
        }
    }

    /// ¿No había `carpetas.toml`? (primera vez: hay que copiar la vista de la sesión).
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }

    /// Primera vez: la vista y las columnas de la sesión de antes pasan a ser la vista por
    /// defecto; los paneles que tenían otra vista la guardan para sus carpetas.
    pub fn migrate(&mut self, s: &Session) {
        let default = s.legacy_view();
        for (i, pane) in s.panes.iter().enumerate() {
            let Some(&view) = s.views.get(i) else { continue };
            if view == default.view {
                continue;
            }
            for loc in &pane.tabs {
                self.set(&folder_key(loc, None), ViewSettings { view, ..default.clone() });
            }
        }
        self.default = default;
        self.fresh = false;
        self.dirty = true;
    }

    /// La vista de la carpeta, o la por defecto si nunca se la tocó.
    pub fn get(&self, key: &str) -> ViewSettings {
        self.entries.get(key).map_or_else(|| self.default.clone(), |e| e.0.clone())
    }

    /// ¿La carpeta tiene vista propia?
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Cuántas carpetas tienen vista propia.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Guarda la vista de una carpeta (la más recién usada). Si se pasa de `MAX_FOLDERS`, se
    /// descarta la usada hace más tiempo.
    pub fn set(&mut self, key: &str, v: ViewSettings) {
        self.clock += 1;
        self.entries.insert(key.to_string(), (v, self.clock));
        self.dirty = true;
        if self.entries.len() > MAX_FOLDERS {
            self.evict();
        }
    }

    /// Anota que se usó la vista de la carpeta (para el orden de descarte), sin cambiarla.
    pub fn touch(&mut self, key: &str) {
        if let Some(e) = self.entries.get_mut(key) {
            self.clock += 1;
            e.1 = self.clock;
        }
    }

    fn evict(&mut self) {
        while self.entries.len() > MAX_FOLDERS {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| k.clone()) else { return };
            self.entries.remove(&oldest);
        }
    }

    pub fn default_view(&self) -> &ViewSettings {
        &self.default
    }

    /// «Usar esta vista en todas las carpetas»: pasa a ser la por defecto (sin grupos plegados
    /// ni filtros) y se borran las vistas de todas las carpetas.
    pub fn apply_to_all(&mut self, v: ViewSettings) {
        self.default = ViewSettings { collapsed: Vec::new(), filters: Vec::new(), ..v };
        self.entries.clear();
        self.dirty = true;
    }

    /// «Restablecer la vista de esta carpeta»: vuelve a la por defecto.
    pub fn reset(&mut self, key: &str) {
        if self.entries.remove(key).is_some() {
            self.dirty = true;
        }
    }

    /// ¿Hay cambios sin guardar?
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Escribe `carpetas.toml` si hubo cambios (de forma atómica: si se corta a la mitad, el
    /// anterior queda intacto). Sin carpeta de configuración no hace nada; si falla, queda
    /// pendiente para la próxima.
    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.dirty = false;
            return;
        };
        let text = write(self);
        let saved = (|| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &path)
        })();
        if saved.is_ok() {
            self.dirty = false;
        }
    }

    /// Las carpetas con vista propia, de la usada hace más tiempo a la más reciente.
    pub fn entries(&self) -> Vec<(&str, &ViewSettings)> {
        let mut v: Vec<(&str, &ViewSettings, u64)> = self.entries.iter().map(|(k, e)| (k.as_str(), &e.0, e.1)).collect();
        v.sort_by_key(|e| e.2);
        v.into_iter().map(|(k, s, _)| (k, s)).collect()
    }
}

/// `carpetas.toml` como texto: la vista por defecto y una tabla por carpeta, de la usada hace
/// más tiempo a la más reciente.
pub fn write(fv: &FolderViews) -> String {
    let mut out = String::from("# Vista de cada carpeta: xdir lo reescribe solo. Borrarlo vuelve todas a la vista de fábrica.\n");
    out += "\n[defecto]\n";
    write_settings(&mut out, &fv.default);
    for (key, v) in fv.entries() {
        out += "\n[[carpeta]]\n";
        out += &format!("clave = {}\n", toml::quote(key));
        write_settings(&mut out, v);
    }
    out
}

fn write_settings(out: &mut String, v: &ViewSettings) {
    *out += &format!("vista = {}\n", toml::quote(v.view.id()));
    *out += &format!("orden = {}\n", toml::quote(v.sort.id()));
    *out += &format!("descendente = {}\n", v.descending);
    // "nombre" (ocupa lo que sobra) o "nombre:300" (fijado), "modificado:160", ...
    let cols: Vec<String> = v
        .columns
        .iter()
        .map(|c| if c.col == Column::Name && c.width <= 0.0 { toml::quote(c.col.id()) } else { toml::quote(&format!("{}:{}", c.col.id(), c.width.round())) })
        .collect();
    *out += &format!("columnas = [{}]\n", cols.join(", "));
    if let Some(g) = v.group_by {
        *out += &format!("agrupar = {}\n", toml::quote(g.id()));
    }
    if !v.collapsed.is_empty() {
        let items: Vec<String> = v.collapsed.iter().map(|s| toml::quote(s)).collect();
        *out += &format!("plegados = [{}]\n", items.join(", "));
    }
    if !v.filters.is_empty() {
        // Cada filtro: la columna y después los grupos tildados.
        let items: Vec<String> = v
            .filters
            .iter()
            .map(|(col, groups)| {
                let parts: Vec<String> = std::iter::once(col.id()).chain(groups.iter().map(String::as_str)).map(toml::quote).collect();
                format!("[{}]", parts.join(", "))
            })
            .collect();
        *out += &format!("filtros = [{}]\n", items.join(", "));
    }
}

/// Lee `carpetas.toml`. Lo que no se entiende se saltea: una línea rota descarta la tabla en la
/// que está (la por defecto vuelve a la de fábrica) y una carpeta con una columna o vista
/// desconocida se ignora; cada problema va a la lista de errores con su línea.
pub fn parse(text: &str) -> (FolderViews, Vec<String>) {
    let doc = toml::parse(text);
    let mut fv = FolderViews { fresh: false, ..FolderViews::default() };
    let mut errors: Vec<String> = doc.errors.iter().map(|(line, e)| format!("línea {line}: {e}")).collect();
    // Rango de líneas de cada tabla, para saber en cuál cayó cada error.
    let starts: Vec<usize> = doc.sections.iter().map(|s| s.line).collect();
    let broken = |i: usize| {
        let (from, to) = (starts[i], starts.get(i + 1).copied().unwrap_or(usize::MAX));
        doc.errors.iter().any(|(l, _)| *l > from && *l < to)
    };
    for (i, section) in doc.sections.iter().enumerate() {
        match (section.name.as_str(), section.array) {
            ("defecto", false) => {
                if broken(i) {
                    continue;
                }
                match settings_from(section) {
                    Ok(v) => fv.default = v,
                    Err(e) => errors.push(format!("línea {}: {e}", section.line)),
                }
            }
            ("carpeta", true) => {
                if broken(i) {
                    errors.push(format!("línea {}: carpeta ignorada (tiene una línea que no se entiende)", section.line));
                    continue;
                }
                let Some(key) = section.get("clave").and_then(toml::Value::as_str).filter(|k| !k.is_empty()) else {
                    errors.push(format!("línea {}: carpeta sin clave", section.line));
                    continue;
                };
                match settings_from(section) {
                    Ok(v) => {
                        fv.clock += 1;
                        fv.entries.insert(key.to_string(), (v, fv.clock));
                    }
                    Err(e) => errors.push(format!("línea {}: carpeta ignorada: {e}", section.line)),
                }
            }
            ("", false) if section.entries.is_empty() => {}
            (name, _) => errors.push(format!("línea {}: tabla desconocida «{name}»", section.line)),
        }
    }
    fv.evict();
    (fv, errors)
}

/// Vista de una tabla; los campos que faltan quedan como los de fábrica.
fn settings_from(section: &toml::Section) -> Result<ViewSettings, String> {
    let mut v = ViewSettings::default();
    let column = |id: &str| Column::from_id(id).ok_or_else(|| format!("columna desconocida «{id}»"));
    for entry in &section.entries {
        let bad = || format!("valor no válido para «{}»", entry.key);
        match entry.key.as_str() {
            "clave" => {}
            "vista" => {
                let id = entry.value.as_str().ok_or_else(bad)?;
                v.view = View::from_id(id).ok_or_else(|| format!("vista desconocida «{id}»"))?;
            }
            "orden" => v.sort = column(entry.value.as_str().ok_or_else(bad)?)?,
            "descendente" => v.descending = entry.value.as_bool().ok_or_else(bad)?,
            "columnas" => {
                let mut cols = Vec::new();
                for item in entry.value.as_str_list().ok_or_else(bad)? {
                    let (id, width) = item.split_once(':').unwrap_or((item, ""));
                    let col = column(id.trim())?;
                    let width = width.trim().parse::<f32>().ok().filter(|w| w.is_finite()).unwrap_or(0.0);
                    let width = if col == Column::Name { width.max(0.0) } else { width.max(48.0) };
                    if !cols.iter().any(|c: &ColumnSpec| c.col == col) {
                        cols.push(ColumnSpec { col, width });
                    }
                }
                if !cols.iter().any(|c| c.col == Column::Name) {
                    return Err("faltan la columna del nombre".into());
                }
                v.columns = cols;
            }
            "agrupar" => v.group_by = Some(column(entry.value.as_str().ok_or_else(bad)?)?),
            "plegados" => v.collapsed = entry.value.as_str_list().ok_or_else(bad)?.into_iter().map(String::from).collect(),
            "filtros" => {
                let toml::Value::List(items) = &entry.value else { return Err(bad()) };
                let mut filters = Vec::new();
                for item in items {
                    let parts = item.as_str_list().ok_or_else(bad)?;
                    let (first, groups) = parts.split_first().ok_or_else(bad)?;
                    filters.push((column(first)?, groups.iter().map(|s| s.to_string()).collect()));
                }
                v.filters = filters;
            }
            // Campos de versiones más nuevas: se ignoran.
            _ => {}
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FolderViews {
        let mut fv = FolderViews::default();
        fv.default = ViewSettings {
            view: View::Icons,
            sort: Column::Size,
            descending: true,
            columns: vec![
                ColumnSpec { col: Column::Name, width: 0.0 },
                ColumnSpec { col: Column::Size, width: 120.0 },
                ColumnSpec { col: Column::Created, width: 150.0 },
            ],
            ..ViewSettings::default()
        };
        fv.set(
            r"c:\users\a",
            ViewSettings {
                view: View::Thumbnails,
                columns: vec![ColumnSpec { col: Column::Type, width: 90.0 }, ColumnSpec { col: Column::Name, width: 300.0 }],
                group_by: Some(Column::Type),
                collapsed: vec!["Carpeta de archivos".into(), "Hoy".into()],
                filters: vec![(Column::Type, vec!["PDF".into(), "Texto \"plano\"".into()]), (Column::Size, vec![])],
                ..ViewSettings::default()
            },
        );
        fv.set("/home/x/fotos, raras", ViewSettings { sort: Column::Modified, descending: true, ..ViewSettings::default() });
        fv
    }

    fn same(a: &FolderViews, b: &FolderViews) {
        assert_eq!(a.default_view(), b.default_view());
        assert_eq!(a.entries(), b.entries());
    }

    #[test]
    fn folder_views_roundtrip() {
        let fv = sample();
        let text = write(&fv);
        let (back, errors) = parse(&text);
        assert!(errors.is_empty(), "{errors:?}\n{text}");
        same(&back, &fv);
        assert_eq!(back.len(), 2);
        assert!(!back.is_fresh());
    }

    /// Las columnas de metadatos (con sus ids `dimensiones`, `duracion`, `autor`, `etiquetas`,
    /// `artista`, `album`) se guardan y se leen como las demás, también al agrupar y filtrar.
    #[test]
    fn folder_views_roundtrip_metadata_columns() {
        let meta = [Column::Dimensions, Column::Duration, Column::Authors, Column::Tags, Column::Artist, Column::Album];
        assert_eq!(meta.map(Column::id), ["dimensiones", "duracion", "autor", "etiquetas", "artista", "album"]);
        let mut fv = FolderViews::default();
        let mut columns = vec![ColumnSpec { col: Column::Name, width: 0.0 }];
        columns.extend(meta.iter().map(|&col| ColumnSpec { col, width: 120.0 }));
        let v = ViewSettings {
            columns,
            sort: Column::Duration,
            group_by: Some(Column::Dimensions),
            filters: vec![(Column::Tags, vec!["viaje".into()])],
            ..ViewSettings::default()
        };
        fv.set("/fotos", v.clone());
        let (back, errors) = parse(&write(&fv));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(back.get("/fotos"), v);
    }

    #[test]
    fn folder_views_ignores_bad_lines() {
        let text = write(&sample());
        // Una línea que no se entiende (en la primera carpeta) y una carpeta con una columna desconocida.
        let text = text.replacen("vista = \"miniaturas\"", "vista = miniaturas sin comillas", 1)
            + "\n[[carpeta]]\nclave = \"/otra\"\ncolumnas = [\"nombre\", \"inventada:80\"]\n"
            + "\n[[carpeta]]\nclave = \"/buena\"\nvista = \"iconos_grandes\"\n";
        let (fv, errors) = parse(&text);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().all(|e| e.starts_with("línea ")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("inventada")), "{errors:?}");
        assert!(!fv.contains(r"c:\users\a") && !fv.contains("/otra"));
        assert!(fv.contains("/home/x/fotos, raras"));
        assert_eq!(fv.get("/buena").view, View::LargeIcons);
        assert_eq!(fv.default_view(), sample().default_view());
    }

    /// Un `carpetas.toml` que existe pero no se puede leer (acá, una carpeta con ese nombre) no se
    /// pisa: la vista de fábrica, con el aviso, y sin dónde guardar.
    #[test]
    fn folder_views_unreadable_file_is_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("xdir-carpetas-ilegible-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(FILE_NAME);
        std::fs::create_dir_all(&path).unwrap();
        let mut fv = FolderViews::load_from(path.clone());
        assert_eq!(fv.errors.len(), 1, "{:?}", fv.errors);
        assert!(!fv.is_fresh());
        assert_eq!(fv.path, None);
        fv.set("/x", ViewSettings { view: View::Icons, ..ViewSettings::default() });
        fv.save_if_dirty();
        assert!(path.is_dir());
        // Sin archivo: «primera vez», y se guarda ahí.
        let other = dir.join("otro.toml");
        let fv = FolderViews::load_from(other.clone());
        assert!(fv.is_fresh() && fv.errors.is_empty());
        assert_eq!(fv.path, Some(other));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un ancho no finito (inf, NaN) se ignora: la columna queda con el ancho mínimo.
    #[test]
    fn folder_views_ignores_non_finite_widths() {
        for w in ["inf", "-inf", "NaN"] {
            let text = format!("[[carpeta]]\nclave = \"/a\"\ncolumnas = [\"nombre:{w}\", \"tamano:{w}\"]\n");
            let (fv, errors) = parse(&text);
            assert!(errors.is_empty(), "{errors:?}");
            let cols = fv.get("/a").columns;
            assert!(cols.iter().all(|c| c.width.is_finite()), "{w}: {cols:?}");
            assert_eq!(cols.iter().map(|c| c.width).collect::<Vec<_>>(), [0.0, 48.0], "{w}");
        }
    }

    #[test]
    fn folder_views_truncated_file_loads() {
        let mut fv = FolderViews::default();
        for i in 0..40 {
            fv.set(&format!("/carpeta/{i}"), ViewSettings { view: View::Icons, ..ViewSettings::default() });
        }
        let text = write(&fv);
        for cut in [text.len() / 2, text.len() / 3, 10, text.len() - 3] {
            let mut cut = cut;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            let (back, _errors) = parse(&text[..cut]);
            assert!(back.len() <= 40);
            // Las carpetas anteriores al corte, enteras; la última puede haber quedado sin sus
            // últimos campos (los que faltan son los de fábrica), pero nunca con valores rotos.
            let loaded = back.entries();
            for (k, v) in loaded.iter().take(loaded.len().saturating_sub(1)) {
                assert_eq!(Some(*v), fv.entries.get(*k).map(|e| &e.0), "carpeta {k} a medias");
            }
        }
        assert!(parse(&text[..text.len() / 2]).0.len() >= 15);
    }

    #[test]
    fn folder_views_lru_caps_at_2000() {
        let mut fv = FolderViews::default();
        for i in 0..5000 {
            fv.set(&format!("/c/{i}"), ViewSettings::default());
            // La primera se sigue usando: no se descarta.
            if i % 100 == 0 {
                fv.touch("/c/0");
            }
        }
        assert_eq!(fv.len(), MAX_FOLDERS);
        assert!(fv.contains("/c/0"));
        assert!(fv.contains("/c/4999") && fv.contains("/c/3001"));
        assert!(!fv.contains("/c/1") && !fv.contains("/c/3000"));
        // Al leerlo se respeta el mismo tope.
        let (back, _) = parse(&write(&fv));
        assert_eq!(back.len(), MAX_FOLDERS);
    }

    #[test]
    fn folder_key_same_for_equal_paths() {
        sys::testing::install_fakes();
        let (a, b) = (Path::new(r"C:\Users\A"), Path::new(r"c:\users\a\"));
        assert!(sys::file_system().same_path(a, b));
        let (ka, kb) = (folder_key(&Location::Path(a.into()), None), folder_key(&Location::Path(b.into()), None));
        assert_eq!(ka, kb);
        assert_ne!(ka, folder_key(&Location::Path(r"c:\users\b".into()), None));
        assert_eq!(folder_key(&Location::Computer, None), "equipo");
        let trash = Location::Shell { parsing: "::{645FF040}".into(), name: "Papelera".into(), parent: None };
        assert_eq!(folder_key(&trash, None), "shell:::{645FF040}");
        // Los resultados de buscar en una carpeta tienen su propia clave.
        let search = folder_key(&Location::Path(a.into()), Some(b));
        assert_eq!(search, format!("busqueda:{kb}"));
        assert_ne!(search, ka);
    }

    #[test]
    fn migration_copies_session_view() {
        sys::testing::install_fakes();
        let dir = crate::session::config_dir().unwrap();
        let _ = std::fs::remove_file(dir.join(FILE_NAME));
        let cols = vec![ColumnSpec { col: Column::Name, width: 48.0 }, ColumnSpec { col: Column::Size, width: 120.0 }];
        let s = Session {
            columns: cols,
            views: vec![View::Icons, View::Thumbnails],
            panes: vec![
                crate::session::PaneState { tabs: vec![Location::Computer], active: 0 },
                crate::session::PaneState { tabs: vec![Location::Path("/fotos".into())], active: 0 },
            ],
            ..Session::default()
        };
        let mut fv = FolderViews::load();
        assert!(fv.is_fresh());
        fv.migrate(&s);
        let d = fv.default_view();
        assert_eq!(d.view, View::Icons);
        // El nombre sin ancho fijo (la sesión lo guardaba sin ancho).
        assert_eq!(d.columns, vec![ColumnSpec { col: Column::Name, width: 0.0 }, ColumnSpec { col: Column::Size, width: 120.0 }]);
        // El panel con otra vista la conserva en su carpeta.
        assert_eq!(fv.get(&folder_key(&Location::Path("/fotos".into()), None)).view, View::Thumbnails);
        // Queda guardado: la próxima vez ya no es la primera.
        fv.save_if_dirty();
        let again = FolderViews::load();
        assert!(!again.is_fresh());
        assert_eq!(again.default_view(), fv.default_view());
        let _ = std::fs::remove_file(dir.join(FILE_NAME));
    }
}

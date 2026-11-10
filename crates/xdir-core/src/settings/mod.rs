//! La pestaña Configuración (`Location::Settings`): secciones, el estado de la vista que viaja con
//! la pestaña y el modelo de lo que se dibuja (filas de opciones con su control). La `App` arma
//! las filas con sus valores (`app/settings.rs`) y `draw` las dibuja y anota dónde quedó cada
//! parte, así el clic se resuelve con lo mismo que se dibujó.

pub mod controls;
pub mod draw;
pub mod keys;
pub mod options;

use crate::geom::{Color, Rect};

/// Secciones propias, en el orden de la lista de la izquierda.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    Appearance,
    ListViews,
    Tree,
    Preview,
    Keys,
    Xtree,
    Advanced,
}

impl Section {
    pub const ALL: [Section; 7] =
        [Section::Appearance, Section::ListViews, Section::Tree, Section::Preview, Section::Keys, Section::Xtree, Section::Advanced];

    /// Ancla de la sesión (`xdir:configuracion#atajos`).
    pub fn id(self) -> &'static str {
        match self {
            Section::Appearance => "apariencia",
            Section::ListViews => "lista",
            Section::Tree => "arbol",
            Section::Preview => "vista_previa",
            Section::Keys => "atajos",
            Section::Xtree => "xtree",
            Section::Advanced => "avanzado",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Appearance => "Apariencia",
            Section::ListViews => "Lista y vistas",
            Section::Tree => "Árbol",
            Section::Preview => "Vista previa",
            Section::Keys => "Atajos de teclado",
            Section::Xtree => "xtree",
            Section::Advanced => "Avanzado",
        }
    }

    pub fn from_id(id: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// Una página de la lista de la izquierda: una sección propia o una de otro módulo (Red).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PageId {
    Builtin(Section),
    Extra(&'static str),
}

impl Default for PageId {
    fn default() -> Self {
        PageId::Builtin(Section::Appearance)
    }
}

/// Algo buscable: una opción, un comando de Atajos o una fila de una página extra.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchItem {
    pub title: String,
    pub desc: String,
    pub keywords: Vec<String>,
}

/// Parte de la vista bajo el mouse o cliqueada.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsHit {
    /// Una página de la lista de la izquierda.
    Nav(PageId),
    /// Una parte del control de una fila: (id de la fila, cuál: segmento, color, botón…).
    Part(&'static str, usize),
    /// El contenido de una página extra (lo atiende su `click`).
    Extra,
    /// El contenido sin nada debajo.
    Body,
}

/// Estado de la vista: va en la pestaña, así se mueve con ella a otro panel.
#[derive(Debug, Default)]
pub struct SettingsView {
    pub page: PageId,
    /// Desplazamiento del contenido (DIPs desde arriba).
    pub scroll: f32,
    /// Lo escrito en el buscador (la caja de búsqueda del panel).
    pub query: String,
    /// Grabando un atajo (Atajos de teclado).
    pub recording: Option<keys::Recording>,
    /// Alto del contenido dibujado y del área visible (para limitar el desplazamiento).
    pub content_h: f32,
    pub view_h: f32,
    /// Dónde quedó cada parte en el último cuadro (ya desplazada y dentro del área visible).
    pub hits: Vec<(Rect, SettingsHit)>,
    /// Área del contenido en el último cuadro.
    pub content: Rect,
    /// Nombre de la página, para las migas de la barra de dirección (lo pone la `App`).
    pub page_title: String,
}

impl SettingsView {
    pub fn new(page: PageId) -> Self {
        Self { page, ..Default::default() }
    }

    /// Lo que hay en `(x, y)` según el último cuadro.
    pub fn hit_at(&self, x: f32, y: f32) -> Option<SettingsHit> {
        self.hits.iter().rev().find(|(r, _)| crate::geom::contains(r, x, y)).map(|(_, h)| *h)
    }

    /// Mueve el contenido `dy` DIPs (positivo: hacia abajo), sin pasarse.
    pub fn scroll_by(&mut self, dy: f32) -> bool {
        let max = (self.content_h - self.view_h).max(0.0);
        let to = (self.scroll + dy).clamp(0.0, max);
        let changed = (to - self.scroll).abs() > 0.1;
        self.scroll = to;
        changed
    }

    /// Cambia de página: arriba de todo y sin grabación.
    pub fn set_page(&mut self, page: PageId) {
        if self.page != page {
            self.page = page;
            self.scroll = 0.0;
            self.recording = None;
        }
    }
}

/// Control de una fila de opción.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// Solo el texto (y lo que diga la descripción).
    None,
    Switch(bool),
    /// Opciones y la elegida.
    Segmented(Vec<&'static str>, Option<usize>),
    /// Desplegable: el texto de lo elegido (el menú lo abre la `App`).
    Dropdown(String),
    /// Deslizador con su valor (y el número a la derecha).
    Slider { value: f32, min: f32, max: f32 },
    /// Botones de 180 a la derecha (de derecha a izquierda el último primero); `false`: apagado.
    Buttons(Vec<(String, bool)>),
    /// Color de acento: los presets, «Del sistema» y «Otro…».
    Accent(AccentChoice),
    /// Reglas de color, solo para ver: (patrón, color, propia).
    ColorRules(Vec<(String, Color, bool)>),
    /// Tabla de dos columnas (archivo y qué guarda; tecla y qué hace).
    Table(Vec<(String, String)>),
}

/// Qué está elegido en el color de acento.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccentChoice {
    Preset(usize),
    System,
    Other(u32),
}

/// Una fila de opción: tarjeta con título, descripción y control.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: &'static str,
    pub title: String,
    pub desc: String,
    pub control: Control,
    pub enabled: bool,
}

impl Row {
    pub fn new(id: &'static str, title: impl Into<String>, desc: impl Into<String>, control: Control) -> Self {
        Self { id, title: title.into(), desc: desc.into(), control, enabled: true }
    }
}

/// Lo que se dibuja en el contenido, de arriba abajo.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Encabezado de grupo (SmallBold, `text_secondary`).
    Heading(String),
    Row(Row),
    /// Texto chico al pie.
    Note(String),
    /// Aviso en rojo (no se pudo guardar, error de una línea), con un botón opcional a la derecha.
    Error(String, Option<(&'static str, String)>),
    /// Botón suelto a la derecha (p. ej. «Restablecer todos»), con su id.
    Action(&'static str, String),
    /// Fila de un comando en Atajos de teclado.
    Shortcut(keys::ShortcutRow),
}

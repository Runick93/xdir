//! Dibujo y hit-test del panel de información (diseño `info::draw`, `info::draw_header` e
//! `info::draw_section`, pantallas X48–X50): barra superior de 40 con el título y ✕, encabezado de
//! 100 con la vista previa de 72, secciones plegables (cabecera de 32, filas de 22 con el valor a 96
//! del margen) y «Más en el sistema…» (150×28) al final del contenido, que se desplaza debajo de
//! la barra superior.
//!
//! `layout` arma, sin pintar, lo que se dibuja (primitivas) y dónde está cada cosa que se puede
//! tocar; `draw` pinta ese resultado y `hit` lo consulta. Así lo dibujado y lo que responde al
//! mouse salen de la misma cuenta. Es neutro: lo que se muestra llega armado en `Panel`.

use super::{Flag, Section, Tag, Tri};
use crate::sys::HashAlgo;
use crate::geom::{Color, Rect, contains, rect};
use crate::paint::{FontId, Painter};
use crate::thumbs;

pub const MARGIN: f32 = 16.0;
/// Barra superior (título y ✕).
pub const TOP_H: f32 = 40.0;
/// Encabezado con la vista previa.
pub const HEADER_H: f32 = 100.0;
pub const PREVIEW: f32 = 72.0;
/// Cabecera de cada sección y filas de datos.
pub const SECTION_H: f32 = 32.0;
pub const ROW_H: f32 = 22.0;
/// Dónde empieza el valor de una fila, desde el margen.
pub const VALUE_X: f32 = 96.0;
/// «Más en el sistema…».
pub const MORE_W: f32 = 150.0;
pub const MORE_H: f32 = 28.0;
/// Alto de una caja de texto (nombre, comentario) y del desplegable de «Abrir con».
pub const FIELD_H: f32 = 28.0;
/// Columna «Privilegio» de la tabla de permisos, desde el margen.
const PRIV_X: f32 = 130.0;

pub const CLOUD_TEXT: &str = crate::preview::draw::CLOUD_TEXT;
pub const NO_TAGS_TEXT: &str = "Este tipo de archivo no admite etiquetas";
pub const NO_COMMENT_TEXT: &str = "Este tipo de archivo no admite comentarios";
pub const ADD_TAG_TEXT: &str = "Agregar etiqueta";
pub const MORE_TEXT: &str = "Más en el sistema…";
pub const CHANGE_ALL_TEXT: &str = "Cambiar todos…";
pub const EJECT_TEXT: &str = "⏏ Expulsar";
/// El mismo botón mientras se expulsa (con el indicador girando a la izquierda).
pub const EJECTING_TEXT: &str = "Expulsando…";
/// Lo que dice la caja «Comparar con…» vacía.
pub const EXPECTED_HINT: &str = "Pegá la suma publicada";
pub const INVALID_SUM_TEXT: &str = "No es una suma SHA-256 (64 caracteres) ni MD5 (32)";
/// Ancho de cada opción del selector de algoritmo.
const ALGO_W: f32 = 88.0;

/// Lo que muestra el panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub title: String,
    pub header: Header,
    pub sections: Vec<SectionView>,
    /// Botón «Más en el sistema…» (no en Linux).
    pub more_button: bool,
    /// Botón «⏏ Expulsar» (un disco expulsable).
    pub eject_button: bool,
    /// Ese disco se está expulsando: el botón gira, dice «Expulsando…» y no se puede tocar.
    pub ejecting: bool,
}

/// Encabezado: la imagen de la vista previa (si `image`, la grande de `key`; si no, el ícono), el
/// nombre y hasta tres líneas (la primera en Ui, el resto chicas); una línea con `true` lleva el
/// indicador de «Calculando…».
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub key: Option<thumbs::Key>,
    pub icon: i32,
    pub image: bool,
    pub name: String,
    pub lines: Vec<(String, bool)>,
}

/// Una sección: abierta o plegada, el error del último cambio y su contenido.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionView {
    pub section: Section,
    pub open: bool,
    pub error: Option<String>,
    pub body: Body,
}

/// Una fila de datos: rótulo y valor; `link` se dibuja en acento y se puede tocar (Ubicación);
/// `spinner`, con el indicador de «Calculando…».
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub link: bool,
    pub spinner: bool,
}

impl Row {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Row {
        Row { label: label.into(), value: value.into(), link: false, spinner: false }
    }
}

/// Contenido de cada sección.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// `None`: el formato no admite etiquetas.
    Tags(Option<Vec<Tag>>),
    /// General: la barra de un disco (usado, libre y sus textos), las filas y las casillas.
    General { capacity: Option<(u64, u64, String, String)>, rows: Vec<Row>, flags: Vec<(Flag, Tri, bool)> },
    /// Más información.
    Rows { rows: Vec<Row>, note: Option<String> },
    /// Nombre (editable) y «Ocultar la extensión» (solo macOS).
    Name { name: String, hide_ext: Option<bool> },
    /// `None`: el formato no admite comentarios.
    Comment(Option<String>),
    /// La app actual (o ninguna), su ícono y la nota sobre «Cambiar todos…».
    OpenWith { app: Option<String>, icon: i32, note: String },
    /// Filas (quién, acceso); `editable`: el acceso se toca para cambiarlo.
    Perms { rows: Vec<(String, String)>, editable: bool, note: Option<String> },
    /// Suma de verificación de un archivo.
    Checksum(ChecksumView),
}

/// Cómo va la suma del algoritmo elegido.
#[derive(Clone, Debug, PartialEq)]
pub enum SumState {
    Idle,
    /// Calculando (la parte hecha).
    Running(f32),
    Done(String),
    Failed(String),
    /// Solo en la nube: calcularla lo bajaría (no se ofrece).
    CloudOnly,
}

/// Lo que dice «Comparar con…».
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SumCheck {
    Matches,
    Differs,
    /// Lo pegado no es una suma que se entienda.
    Invalid,
}

/// La sección «Suma de verificación»: el algoritmo elegido, cómo va, lo pegado en «Comparar con…»
/// y si coincide (`None`: no hay nada pegado o falta calcular).
#[derive(Clone, Debug, PartialEq)]
pub struct ChecksumView {
    pub algo: HashAlgo,
    pub state: SumState,
    pub expected: String,
    pub check: Option<SumCheck>,
    /// Con varias sumas pegadas: qué línea se usó.
    pub note: Option<String>,
}

/// Lo que hay bajo el mouse en el panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InfoHit {
    Close,
    /// La cabecera de una sección: la pliega o la abre.
    Toggle(Section),
    TagRemove(usize),
    TagAdd,
    Flag(Flag),
    HideExt,
    /// La ubicación (ir a la carpeta).
    Location,
    NameField,
    CommentField,
    OpenWithMenu,
    ChangeAll,
    /// El acceso de una fila de permisos (0 dueño, 1 grupo, 2 todos).
    Perm(usize),
    MoreInSystem,
    /// «⏏ Expulsar» del disco.
    Eject,
    /// Una opción del selector de la suma de verificación.
    HashAlgo(HashAlgo),
    /// «Calcular» la suma (o Cancelar, mientras calcula).
    HashRun,
    /// «Copiar» la suma.
    HashCopy,
    /// La caja «Comparar con…».
    Expected,
    /// El panel, sin nada que tocar.
    Body,
}

/// Color de una primitiva, del tema.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Text,
    Dim,
    Secondary,
    Accent,
    Danger,
    /// Verde de «Iguales» / «Coincide».
    Ok,
    Border,
    Background,
    SurfaceAlt,
    Rgb(u32),
}

/// Lo que se dibuja, ya ubicado.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Text { font: FontId, s: String, r: Rect, tone: Tone },
    Fill { r: Rect, radius: f32, tone: Tone },
    Stroke { r: Rect, radius: f32, tone: Tone },
    /// Flecha de la sección (▾ abierta, ▸ plegada) centrada en `(x, y)`.
    Chevron { x: f32, y: f32, open: bool },
    /// Flecha de un desplegable (hacia abajo).
    Down { x: f32, y: f32 },
    Check { r: Rect, state: Tri, enabled: bool },
    Image { key: Option<thumbs::Key>, icon: i32, r: Rect, image: bool },
    Icon { icon: i32, x: f32, y: f32, size: f32 },
    Dot { x: f32, y: f32, radius: f32, tone: Tone },
    Cross { x: f32, y: f32, size: f32, tone: Tone },
    Plus { x: f32, y: f32 },
    Spinner { x: f32, y: f32 },
}

/// Resultado de `layout`: el panel, la parte que se desplaza, el alto del contenido, el
/// desplazamiento (ya limitado), las primitivas fijas y las del contenido (ya desplazadas) y lo
/// que se puede tocar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub panel: Rect,
    pub view: Rect,
    pub content_h: f32,
    pub scroll: f32,
    pub fixed: Vec<Prim>,
    pub content: Vec<Prim>,
    pub hits: Vec<(InfoHit, Rect)>,
    /// Dónde va cada caja de texto (para montar la del host encima).
    pub fields: Vec<(InfoHit, Rect)>,
}

impl Layout {
    /// Desplazamiento máximo.
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - (self.view.bottom - self.view.top)).max(0.0)
    }
}

/// Arma el panel en `r` con el desplazamiento `scroll` (se limita a lo que hay). `measure` da el
/// ancho de un texto en la fuente de la interfaz.
pub fn layout(r: Rect, panel: &Panel, scroll: f32, measure: &dyn Fn(&str) -> f32) -> Layout {
    let mut l = Layout { panel: r, view: rect(r.left, r.top + TOP_H, r.right, r.bottom - 1.0), ..Layout::default() };
    let (x0, x1) = (r.left + MARGIN, r.right - MARGIN);
    // Barra superior.
    l.fixed.push(Prim::Text { font: FontId::Ui, s: panel.title.clone(), r: rect(x0, r.top + 12.0, x1 - 28.0, r.top + 28.0), tone: Tone::Text });
    let close = rect(r.right - 33.0, r.top + 8.0, r.right - 9.0, r.top + 32.0);
    l.fixed.push(Prim::Cross { x: (close.left + close.right) / 2.0, y: (close.top + close.bottom) / 2.0, size: 5.0, tone: Tone::Secondary });
    l.hits.push((InfoHit::Close, close));

    // El contenido se arma desde y = 0 (relativo) y después se corre.
    let top = r.top + TOP_H;
    let mut c = Builder { prims: Vec::new(), hits: Vec::new(), fields: Vec::new(), measure, x0, x1 };
    let mut y = top;
    y = c.header(&panel.header, y);
    for s in &panel.sections {
        y = c.section(s, y);
    }
    // Abajo, en una fila: «⏏ Expulsar» a la izquierda y «Más en el sistema…» a la derecha.
    if panel.more_button || panel.eject_button {
        y += 16.0;
        if panel.eject_button && panel.ejecting {
            // Gira en lugar del ⏏, el texto tenue y sin lugar para el clic.
            let w = (c.measure)(EJECTING_TEXT) + 40.0;
            let b = rect(x0, y, x0 + w, y + MORE_H);
            c.prims.push(Prim::Fill { r: b, radius: 4.0, tone: Tone::SurfaceAlt });
            c.prims.push(Prim::Stroke { r: b, radius: 4.0, tone: Tone::Border });
            let cy = (b.top + b.bottom) / 2.0;
            c.prims.push(Prim::Spinner { x: b.left + 16.0, y: cy });
            c.text(FontId::Ui, EJECTING_TEXT, rect(b.left + 28.0, cy - 8.0, b.right - 4.0, cy + 8.0), Tone::Dim);
        } else if panel.eject_button {
            let w = (c.measure)(EJECT_TEXT) + 28.0;
            let b = rect(x0, y, x0 + w, y + MORE_H);
            c.button(b, EJECT_TEXT);
            c.hits.push((InfoHit::Eject, b));
        }
        if panel.more_button {
            let b = rect(x1 - MORE_W, y, x1, y + MORE_H);
            c.button(b, MORE_TEXT);
            c.hits.push((InfoHit::MoreInSystem, b));
        }
        y += MORE_H;
    }
    y += 16.0;
    l.content_h = y - top;
    l.scroll = scroll.clamp(0.0, l.max_scroll());
    let dy = -l.scroll;
    l.content = c.prims.into_iter().map(|p| shift(p, dy)).collect();
    let view = l.view;
    let visible = |rc: &Rect| rc.bottom > view.top && rc.top < view.bottom;
    l.hits.extend(c.hits.into_iter().map(|(h, rc)| (h, moved(rc, dy))).filter(|(_, rc)| visible(rc)));
    l.fields = c.fields.into_iter().map(|(h, rc)| (h, moved(rc, dy))).collect();
    l
}

/// Lo que hay en `(x, y)` (None: fuera del panel).
pub fn hit(l: &Layout, x: f32, y: f32) -> Option<InfoHit> {
    if !contains(&l.panel, x, y) {
        return None;
    }
    // Lo de la barra superior primero; lo del contenido, solo dentro de la parte que se desplaza.
    let in_view = contains(&l.view, x, y);
    l.hits
        .iter()
        .find(|(h, rc)| contains(rc, x, y) && (*h == InfoHit::Close || in_view))
        .map(|(h, _)| *h)
        .or(Some(InfoHit::Body))
}

pub(crate) fn moved(r: Rect, dy: f32) -> Rect {
    rect(r.left, r.top + dy, r.right, r.bottom + dy)
}

pub(crate) fn shift(p: Prim, dy: f32) -> Prim {
    match p {
        Prim::Text { font, s, r, tone } => Prim::Text { font, s, r: moved(r, dy), tone },
        Prim::Fill { r, radius, tone } => Prim::Fill { r: moved(r, dy), radius, tone },
        Prim::Stroke { r, radius, tone } => Prim::Stroke { r: moved(r, dy), radius, tone },
        Prim::Chevron { x, y, open } => Prim::Chevron { x, y: y + dy, open },
        Prim::Down { x, y } => Prim::Down { x, y: y + dy },
        Prim::Check { r, state, enabled } => Prim::Check { r: moved(r, dy), state, enabled },
        Prim::Image { key, icon, r, image } => Prim::Image { key, icon, r: moved(r, dy), image },
        Prim::Icon { icon, x, y, size } => Prim::Icon { icon, x, y: y + dy, size },
        Prim::Dot { x, y, radius, tone } => Prim::Dot { x, y: y + dy, radius, tone },
        Prim::Cross { x, y, size, tone } => Prim::Cross { x, y: y + dy, size, tone },
        Prim::Plus { x, y } => Prim::Plus { x, y: y + dy },
        Prim::Spinner { x, y } => Prim::Spinner { x, y: y + dy },
    }
}

struct Builder<'a> {
    prims: Vec<Prim>,
    hits: Vec<(InfoHit, Rect)>,
    fields: Vec<(InfoHit, Rect)>,
    measure: &'a dyn Fn(&str) -> f32,
    x0: f32,
    x1: f32,
}

impl Builder<'_> {
    fn text(&mut self, font: FontId, s: &str, r: Rect, tone: Tone) {
        self.prims.push(Prim::Text { font, s: s.to_string(), r, tone });
    }

    fn button(&mut self, b: Rect, label: &str) {
        self.prims.push(Prim::Fill { r: b, radius: 4.0, tone: Tone::SurfaceAlt });
        self.prims.push(Prim::Stroke { r: b, radius: 4.0, tone: Tone::Border });
        self.text(FontId::UiCenter, label, rect(b.left + 4.0, (b.top + b.bottom) / 2.0 - 8.0, b.right - 4.0, (b.top + b.bottom) / 2.0 + 8.0), Tone::Text);
    }

    fn header(&mut self, h: &Header, y: f32) -> f32 {
        let (x0, x1) = (self.x0, self.x1);
        self.prims.push(Prim::Image { key: h.key.clone(), icon: h.icon, r: rect(x0, y + 13.0, x0 + PREVIEW, y + 13.0 + PREVIEW), image: h.image });
        let tx = x0 + PREVIEW + 12.0;
        self.text(FontId::Title, &h.name, rect(tx, y + 15.0, x1, y + 34.0), Tone::Text);
        let mut ly = y + 37.0;
        for (i, (line, spinner)) in h.lines.iter().take(3).enumerate() {
            let (font, tone, lh) = if i == 0 { (FontId::Ui, Tone::Dim, 19.0) } else { (FontId::Small, Tone::Dim, 18.0) };
            let mut x = tx;
            if *spinner {
                self.prims.push(Prim::Spinner { x: x + 5.0, y: ly + 7.0 });
                x += 14.0;
            }
            self.text(font, line, rect(x, ly, x1, ly + 16.0), tone);
            ly += lh;
        }
        y + HEADER_H
    }

    fn row(&mut self, row: &Row, y: f32) {
        let (x0, x1) = (self.x0, self.x1);
        let vx = x0 + VALUE_X;
        self.text(FontId::Ui, &row.label, rect(x0, y + 3.0, vx - 6.0, y + 19.0), Tone::Dim);
        let mut x = vx;
        if row.spinner {
            self.prims.push(Prim::Spinner { x: x + 6.0, y: y + 11.0 });
            x += 16.0;
        }
        let tone = if row.link { Tone::Accent } else { Tone::Text };
        self.text(FontId::Ui, &row.value, rect(x, y + 3.0, x1, y + 19.0), tone);
        if row.link {
            let w = (self.measure)(&row.value).min(x1 - x);
            self.hits.push((InfoHit::Location, rect(x, y, x + w, y + ROW_H)));
        }
    }

    fn section(&mut self, s: &SectionView, y: f32) -> f32 {
        let (x0, x1) = (self.x0, self.x1);
        self.prims.push(Prim::Fill { r: rect(x0, y, x1, y + 1.0), radius: 0.0, tone: Tone::Border });
        let head = rect(x0, y, x1, y + SECTION_H);
        self.prims.push(Prim::Chevron { x: x0 + 6.0, y: y + SECTION_H / 2.0, open: s.open });
        self.text(FontId::UiBold, s.section.title(), rect(x0 + 18.0, y + 8.0, x1, y + 24.0), Tone::Text);
        self.hits.push((InfoHit::Toggle(s.section), head));
        let mut y = y + SECTION_H;
        if !s.open {
            return y;
        }
        if let Some(e) = &s.error {
            self.text(FontId::Small, e, rect(x0, y, x1, y + 16.0), Tone::Danger);
            y += 20.0;
        }
        y = match &s.body {
            Body::Tags(tags) => self.tags(tags.as_deref(), y),
            Body::General { capacity, rows, flags } => {
                if let Some((used, free, used_text, free_text)) = capacity {
                    let bar = rect(x0, y + 2.0, x1, y + 10.0);
                    self.prims.push(Prim::Fill { r: bar, radius: 4.0, tone: Tone::SurfaceAlt });
                    let total = (used + free).max(1) as f32;
                    let frac = (*used as f32 / total).clamp(0.0, 1.0);
                    if frac > 0.0 {
                        self.prims.push(Prim::Fill { r: rect(x0, bar.top, x0 + (x1 - x0) * frac, bar.bottom), radius: 4.0, tone: Tone::Accent });
                    }
                    let ly = y + 16.0;
                    self.prims.push(Prim::Dot { x: x0 + 4.0, y: ly + 8.0, radius: 3.0, tone: Tone::Accent });
                    self.text(FontId::Small, used_text, rect(x0 + 14.0, ly, (x0 + x1) / 2.0, ly + 16.0), Tone::Secondary);
                    let mx = (x0 + x1) / 2.0;
                    self.prims.push(Prim::Dot { x: mx + 4.0, y: ly + 8.0, radius: 3.0, tone: Tone::Border });
                    self.text(FontId::Small, free_text, rect(mx + 14.0, ly, x1, ly + 16.0), Tone::Secondary);
                    y += 40.0;
                }
                for row in rows {
                    self.row(row, y);
                    y += ROW_H;
                }
                if !flags.is_empty() {
                    y += 6.0;
                    let mut x = x0;
                    for &(flag, state, enabled) in flags {
                        let label = flag.label();
                        let w = 20.0 + (self.measure)(label) + 16.0;
                        if x + w > x1 + 16.0 && x > x0 {
                            x = x0;
                            y += 24.0;
                        }
                        self.prims.push(Prim::Check { r: rect(x, y + 4.0, x + 14.0, y + 18.0), state, enabled });
                        self.text(FontId::Ui, label, rect(x + 20.0, y + 3.0, x + w, y + 19.0), if enabled { Tone::Text } else { Tone::Dim });
                        if enabled {
                            self.hits.push((InfoHit::Flag(flag), rect(x, y, x + w - 10.0, y + ROW_H)));
                        }
                        x += w;
                    }
                    y += 26.0;
                }
                y
            }
            Body::Rows { rows, note } => {
                for row in rows {
                    self.row(row, y);
                    y += ROW_H;
                }
                if let Some(n) = note {
                    self.text(FontId::Small, n, rect(x0, y + 2.0, x1, y + 18.0), Tone::Dim);
                    y += 22.0;
                }
                y
            }
            Body::Name { name, hide_ext } => {
                let f = rect(x0, y, x1, y + FIELD_H);
                self.field(f, name, false);
                self.hits.push((InfoHit::NameField, f));
                self.fields.push((InfoHit::NameField, f));
                y += FIELD_H + 6.0;
                if let Some(on) = hide_ext {
                    let label = "Ocultar la extensión";
                    let state = if *on { Tri::On } else { Tri::Off };
                    self.prims.push(Prim::Check { r: rect(x0, y + 4.0, x0 + 14.0, y + 18.0), state, enabled: true });
                    self.text(FontId::Ui, label, rect(x0 + 20.0, y + 3.0, x1, y + 19.0), Tone::Text);
                    self.hits.push((InfoHit::HideExt, rect(x0, y, x0 + 20.0 + (self.measure)(label), y + ROW_H)));
                    y += 26.0;
                }
                y
            }
            Body::Comment(None) => {
                self.text(FontId::Ui, NO_COMMENT_TEXT, rect(x0, y + 3.0, x1, y + 19.0), Tone::Dim);
                y + 26.0
            }
            Body::Comment(Some(text)) => {
                let f = rect(x0, y, x1, y + FIELD_H);
                if text.is_empty() {
                    self.field(f, "Agregar un comentario", true);
                } else {
                    self.field(f, text, false);
                }
                self.hits.push((InfoHit::CommentField, f));
                self.fields.push((InfoHit::CommentField, f));
                y + FIELD_H + 6.0
            }
            Body::OpenWith { app, icon, note } => {
                let f = rect(x0, y, x1, y + FIELD_H);
                self.prims.push(Prim::Fill { r: f, radius: 4.0, tone: Tone::Background });
                self.prims.push(Prim::Stroke { r: f, radius: 4.0, tone: Tone::Border });
                let label = app.clone().unwrap_or_else(|| "Ninguna".to_string());
                self.prims.push(Prim::Icon { icon: *icon, x: x0 + 8.0, y: y + 6.0, size: 16.0 });
                self.text(FontId::Ui, &label, rect(x0 + 32.0, y + 6.0, x1 - 24.0, y + 22.0), Tone::Text);
                self.prims.push(Prim::Down { x: x1 - 14.0, y: y + FIELD_H / 2.0 });
                self.hits.push((InfoHit::OpenWithMenu, f));
                y += FIELD_H + 6.0;
                if !note.is_empty() {
                    self.text(FontId::Small, note, rect(x0, y, x1, y + 16.0), Tone::Dim);
                    y += 22.0;
                }
                let w = (self.measure)(CHANGE_ALL_TEXT) + 28.0;
                let b = rect(x0, y, x0 + w, y + 26.0);
                self.button(b, CHANGE_ALL_TEXT);
                self.hits.push((InfoHit::ChangeAll, b));
                y + 32.0
            }
            Body::Checksum(v) => self.checksum(v, y),
            Body::Perms { rows, editable, note } => {
                self.prims.push(Prim::Fill { r: rect(x0, y, x1, y + 24.0), radius: 0.0, tone: Tone::SurfaceAlt });
                self.text(FontId::SmallBold, "Nombre", rect(x0 + 8.0, y + 5.0, x0 + PRIV_X - 4.0, y + 19.0), Tone::Secondary);
                self.text(FontId::SmallBold, "Privilegio", rect(x0 + PRIV_X, y + 5.0, x1, y + 19.0), Tone::Secondary);
                y += 24.0;
                for (i, (who, access)) in rows.iter().enumerate() {
                    self.text(FontId::Ui, who, rect(x0 + 8.0, y + 4.0, x0 + PRIV_X - 4.0, y + 20.0), Tone::Text);
                    self.text(FontId::Ui, access, rect(x0 + PRIV_X, y + 4.0, x1 - 16.0, y + 20.0), Tone::Text);
                    if *editable {
                        self.prims.push(Prim::Down { x: x1 - 8.0, y: y + 12.0 });
                        self.hits.push((InfoHit::Perm(i), rect(x0 + PRIV_X - 4.0, y, x1, y + 24.0)));
                    }
                    y += 24.0;
                }
                if let Some(n) = note {
                    self.text(FontId::Small, n, rect(x0, y + 4.0, x1, y + 20.0), Tone::Dim);
                    y += 24.0;
                }
                y
            }
        };
        y + 8.0
    }

    /// La sección «Suma de verificación»: selector y «Calcular» (o el avance con Cancelar), la suma
    /// con «Copiar», y la caja «Comparar con…» con lo que dice.
    fn checksum(&mut self, v: &ChecksumView, mut y: f32) -> f32 {
        let (x0, x1) = (self.x0, self.x1);
        if v.state == SumState::CloudOnly {
            self.text(FontId::Ui, CLOUD_TEXT, rect(x0, y + 3.0, x1, y + 19.0), Tone::Dim);
            return y + 26.0;
        }
        // Selector (88 por opción) y, a la derecha, «Calcular» / «Cancelar».
        let run_label = if matches!(v.state, SumState::Running(_)) { "Cancelar" } else { "Calcular" };
        let run_w = (self.measure)(run_label) + 28.0;
        let seg_w = (ALGO_W * HashAlgo::ALL.len() as f32).min(x1 - x0 - run_w - 8.0);
        let all = rect(x0, y, x0 + seg_w, y + FIELD_H);
        self.prims.push(Prim::Fill { r: all, radius: 6.0, tone: Tone::SurfaceAlt });
        self.prims.push(Prim::Stroke { r: all, radius: 6.0, tone: Tone::Border });
        let w = seg_w / HashAlgo::ALL.len() as f32;
        for (i, a) in HashAlgo::ALL.into_iter().enumerate() {
            let b = rect(x0 + w * i as f32, y, x0 + w * (i + 1) as f32, y + FIELD_H);
            if a == v.algo {
                self.prims.push(Prim::Fill { r: rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0), radius: 4.0, tone: Tone::Accent });
            }
            let tone = if a == v.algo { Tone::Rgb(0xFFFFFF) } else { Tone::Secondary };
            self.text(FontId::UiCenter, a.label(), rect(b.left + 2.0, y + 6.0, b.right - 2.0, y + 22.0), tone);
            self.hits.push((InfoHit::HashAlgo(a), b));
        }
        let run = rect(x1 - run_w, y, x1, y + FIELD_H);
        self.button(run, run_label);
        self.hits.push((InfoHit::HashRun, run));
        y += FIELD_H + 8.0;
        match &v.state {
            SumState::Running(f) => {
                self.text(FontId::Small, &format!("Calculando… {} %", (f * 100.0).floor() as u32), rect(x0, y, x1, y + 16.0), Tone::Dim);
                let bar = rect(x0, y + 20.0, x1, y + 24.0);
                self.prims.push(Prim::Fill { r: bar, radius: 2.0, tone: Tone::SurfaceAlt });
                let w = (x1 - x0) * f.clamp(0.0, 1.0);
                if w > 0.5 {
                    self.prims.push(Prim::Fill { r: rect(x0, bar.top, x0 + w.max(4.0), bar.bottom), radius: 2.0, tone: Tone::Accent });
                }
                y += 32.0;
            }
            SumState::Done(hex) => {
                for line in crate::compare::hex_lines(hex, x1 - x0) {
                    self.text(FontId::Mono, &line, rect(x0, y, x1, y + 16.0), Tone::Text);
                    y += 16.0;
                }
                y += 4.0;
                let w = (self.measure)("Copiar") + 28.0;
                let b = rect(x0, y, x0 + w, y + 26.0);
                self.button(b, "Copiar");
                self.hits.push((InfoHit::HashCopy, b));
                y += 34.0;
            }
            SumState::Failed(m) => {
                self.text(FontId::Small, m, rect(x0, y, x1, y + 16.0), Tone::Danger);
                y += 22.0;
            }
            SumState::Idle | SumState::CloudOnly => {}
        }
        // «Comparar con…».
        self.text(FontId::Small, "Comparar con…", rect(x0, y, x1, y + 16.0), Tone::Secondary);
        y += 18.0;
        let f = rect(x0, y, x1, y + FIELD_H);
        if v.expected.is_empty() {
            self.field(f, EXPECTED_HINT, true);
        } else {
            self.field(f, &v.expected, false);
        }
        self.hits.push((InfoHit::Expected, f));
        self.fields.push((InfoHit::Expected, f));
        y += FIELD_H + 6.0;
        match v.check {
            Some(SumCheck::Matches) | Some(SumCheck::Differs) => {
                let (text, tone) = if v.check == Some(SumCheck::Matches) { ("Coincide", Tone::Ok) } else { ("No coincide", Tone::Danger) };
                self.prims.push(Prim::Dot { x: x0 + 5.0, y: y + 10.0, radius: 5.0, tone });
                self.text(FontId::UiBold, text, rect(x0 + 16.0, y + 2.0, x1, y + 18.0), tone);
                y += 24.0;
            }
            Some(SumCheck::Invalid) => {
                self.text(FontId::Small, INVALID_SUM_TEXT, rect(x0, y + 2.0, x1, y + 18.0), Tone::Dim);
                y += 24.0;
            }
            None => {}
        }
        if let Some(n) = &v.note {
            self.text(FontId::Small, n, rect(x0, y, x1, y + 16.0), Tone::Dim);
            y += 22.0;
        }
        y
    }

    /// Caja de texto dibujada (la del host se monta encima al editar).
    fn field(&mut self, f: Rect, text: &str, placeholder: bool) {
        self.prims.push(Prim::Fill { r: f, radius: 4.0, tone: Tone::Background });
        self.prims.push(Prim::Stroke { r: f, radius: 4.0, tone: Tone::Border });
        self.text(FontId::Ui, text, rect(f.left + 8.0, f.top + 6.0, f.right - 8.0, f.top + 22.0), if placeholder { Tone::Dim } else { Tone::Text });
    }

    fn tags(&mut self, tags: Option<&[Tag]>, y: f32) -> f32 {
        let (x0, x1) = (self.x0, self.x1);
        let Some(tags) = tags else {
            self.text(FontId::Ui, NO_TAGS_TEXT, rect(x0, y + 3.0, x1, y + 19.0), Tone::Dim);
            return y + 26.0;
        };
        if tags.is_empty() {
            self.prims.push(Prim::Plus { x: x0 + 10.0, y: y + 11.0 });
            self.text(FontId::Ui, ADD_TAG_TEXT, rect(x0 + 30.0, y + 3.0, x1, y + 19.0), Tone::Dim);
            let w = 30.0 + (self.measure)(ADD_TAG_TEXT) + 8.0;
            let add = rect(x0, y, (x0 + w).min(x1), y + ROW_H);
            self.hits.push((InfoHit::TagAdd, add));
            self.fields.push((InfoHit::TagAdd, rect(x0, y - 3.0, x1, y + FIELD_H - 3.0)));
            return y + 28.0;
        }
        let (mut x, mut ty) = (x0, y);
        for (i, t) in tags.iter().enumerate() {
            let dot = if t.color.is_some() { 12.0 } else { 0.0 };
            let w = (8.0 + dot + (self.measure)(&t.name) + 6.0 + 14.0 + 4.0).min(x1 - x0);
            if x + w > x1 && x > x0 {
                x = x0;
                ty += 28.0;
            }
            let pill = rect(x, ty, x + w, ty + 22.0);
            self.prims.push(Prim::Fill { r: pill, radius: 11.0, tone: Tone::SurfaceAlt });
            self.prims.push(Prim::Stroke { r: pill, radius: 11.0, tone: Tone::Border });
            if let Some(c) = t.color {
                self.prims.push(Prim::Dot { x: x + 12.0, y: ty + 11.0, radius: 3.5, tone: Tone::Rgb(c) });
            }
            self.text(FontId::Small, &t.name, rect(x + 8.0 + dot, ty + 4.0, pill.right - 20.0, ty + 18.0), Tone::Text);
            let (cx, cy) = (pill.right - 11.0, ty + 11.0);
            self.prims.push(Prim::Cross { x: cx, y: cy, size: 3.5, tone: Tone::Secondary });
            self.hits.push((InfoHit::TagRemove(i), rect(cx - 8.0, ty, cx + 8.0, ty + 22.0)));
            x += w + 6.0;
        }
        if x + 22.0 > x1 {
            x = x0;
            ty += 28.0;
        }
        self.prims.push(Prim::Plus { x: x + 11.0, y: ty + 11.0 });
        let add = rect(x, ty, x + 22.0, ty + 22.0);
        self.hits.push((InfoHit::TagAdd, add));
        // La caja para escribir una etiqueta nueva va en una línea propia debajo.
        self.fields.push((InfoHit::TagAdd, rect(x0, ty + 26.0, x1, ty + 26.0 + FIELD_H)));
        ty + 28.0
    }
}

fn rgb(c: u32) -> Color {
    Color { r: ((c >> 16) & 0xFF) as f32 / 255.0, g: ((c >> 8) & 0xFF) as f32 / 255.0, b: (c & 0xFF) as f32 / 255.0, a: 1.0 }
}

/// Pinta el panel ya armado. `hover`: lo que está bajo el mouse (se resalta si es un botón).
pub fn draw(p: &dyn Painter, l: &Layout, hover: Option<InfoHit>) {
    let th = p.theme();
    let r = l.panel;
    p.round_fill(r, 8.0, th.surface);
    p.round_stroke(r, 8.0, th.border);
    if r.right - r.left < 120.0 {
        return;
    }
    if let Some(h) = hover.filter(|h| matches!(h, InfoHit::Close | InfoHit::MoreInSystem | InfoHit::ChangeAll | InfoHit::TagAdd | InfoHit::OpenWithMenu | InfoHit::HashRun | InfoHit::HashCopy))
        && let Some((_, rc)) = l.hits.iter().find(|(x, _)| *x == h)
        && (h == InfoHit::Close || rc.top >= l.view.top)
    {
        p.round_fill(*rc, 4.0, th.hover);
    }
    p.clip(r, &mut || {
        for prim in &l.fixed {
            paint(p, prim);
        }
    });
    p.clip(l.view, &mut || {
        for prim in &l.content {
            paint(p, prim);
        }
    });
    // Barra de desplazamiento (4 de ancho) si el contenido no entra.
    let vh = l.view.bottom - l.view.top;
    if l.content_h > vh + 0.5 {
        let thumb_h = (vh * vh / l.content_h).max(24.0);
        let y = l.view.top + (vh - thumb_h) * (l.scroll / l.max_scroll().max(1.0));
        p.round_fill(rect(r.right - 7.0, y, r.right - 3.0, y + thumb_h), 2.0, th.scrollbar);
    }
}

/// Pinta una primitiva (también la usa el panel de comparar, `compare::draw`).
pub(crate) fn paint(p: &dyn Painter, prim: &Prim) {
    let th = p.theme();
    let color = |t: Tone| match t {
        Tone::Text => th.text,
        Tone::Dim => th.text_dim,
        Tone::Secondary => th.text_secondary,
        Tone::Accent => th.accent,
        Tone::Danger => th.danger,
        Tone::Ok => th.ok,
        Tone::Border => th.border,
        Tone::Background => th.background,
        Tone::SurfaceAlt => th.surface_alt,
        Tone::Rgb(c) => rgb(c),
    };
    match prim {
        Prim::Text { font, s, r, tone } => p.text(*font, s, *r, color(*tone)),
        Prim::Fill { r, radius, tone } if *radius > 0.0 => p.round_fill(*r, *radius, color(*tone)),
        Prim::Fill { r, tone, .. } => p.fill(*r, color(*tone)),
        Prim::Stroke { r, radius, tone } => p.round_stroke(*r, *radius, color(*tone)),
        Prim::Chevron { x, y, open } => {
            let pts: [(f32, f32, f32); 3] = if *open {
                [(x - 4.0, y - 2.0, 0.0), (*x, y + 2.0, 0.0), (x + 4.0, y - 2.0, 0.0)]
            } else {
                [(x - 2.0, y - 4.0, 0.0), (x + 2.0, *y, 0.0), (x - 2.0, y + 4.0, 0.0)]
            };
            p.rounded_path(&pts, false, None, Some((th.text_secondary, 1.5)));
        }
        Prim::Down { x, y } => {
            p.rounded_path(&[(x - 4.0, y - 2.0, 0.0), (*x, y + 2.0, 0.0), (x + 4.0, y - 2.0, 0.0)], false, None, Some((th.text_secondary, 1.5)));
        }
        Prim::Check { r, state, enabled } => {
            let a = if *enabled { 1.0 } else { 0.45 };
            match state {
                Tri::Off => p.round_stroke(*r, 3.0, crate::theme::alpha(th.text_secondary, a)),
                Tri::On | Tri::Mixed => {
                    p.round_fill(*r, 3.0, crate::theme::alpha(th.accent, a));
                    let white = Color { r: 1.0, g: 1.0, b: 1.0, a };
                    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    if *state == Tri::On {
                        p.rounded_path(&[(cx - 3.5, cy, 0.0), (cx - 1.0, cy + 2.5, 0.0), (cx + 3.5, cy - 2.5, 0.0)], false, None, Some((white, 1.5)));
                    } else {
                        p.fill(rect(cx - 3.5, cy - 0.75, cx + 3.5, cy + 0.75), white);
                    }
                }
            }
        }
        Prim::Image { key, icon, r, image } => {
            let shown = *image && key.as_ref().is_some_and(|k| p.big_image(&(k.0.clone(), thumbs::big_side(p.thumb_px(PREVIEW))), *r));
            if !shown {
                p.shell_icon(*icon, 0, r.left, r.top, r.right - r.left);
            }
        }
        Prim::Icon { icon, x, y, size } => p.shell_icon(*icon, 0, *x, *y, *size),
        Prim::Dot { x, y, radius, tone } => p.circle(*x, *y, *radius, color(*tone)),
        Prim::Cross { x, y, size, tone } => {
            let c = color(*tone);
            p.rounded_path(&[(x - size, y - size, 0.0), (x + size, y + size, 0.0)], false, None, Some((c, 1.5)));
            p.rounded_path(&[(x - size, y + size, 0.0), (x + size, y - size, 0.0)], false, None, Some((c, 1.5)));
        }
        Prim::Plus { x, y } => {
            p.circle_stroke(*x, *y, 10.0, th.border, 1.0);
            p.fill(rect(x - 4.5, y - 0.75, x + 4.5, y + 0.75), th.text_secondary);
            p.fill(rect(x - 0.75, y - 4.5, x + 0.75, y + 4.5), th.text_secondary);
        }
        Prim::Spinner { x, y } => {
            p.circle_stroke(*x, *y, 5.0, th.border, 1.5);
            // Tres cuartos de vuelta en acento.
            let pts: Vec<(f32, f32, f32)> = (0..=12)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 / 12.0 * 1.5 * std::f32::consts::PI;
                    (x + 5.0 * a.cos(), y + 5.0 * a.sin(), 0.0)
                })
                .collect();
            p.rounded_path(&pts, false, None, Some((th.accent, 1.5)));
        }
    }
}

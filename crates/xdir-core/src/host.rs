//! La ventana de la aplicación vista desde el núcleo: repintar, tamaño, título, foco, mouse,
//! diálogos, tema y arrastre. Cada sistema la implementa sobre su ventana real (`Win32Host`,
//! `MacHost`) y las pruebas con `sys::testing::FakeHost`.
//!
//! A diferencia de los servicios de `sys`, la ventana es una por `App` y solo se usa desde el
//! hilo de interfaz: no es `Send` ni vive en el registro global. Los métodos toman `&self` (el
//! estado propio va en `Cell`/`RefCell`), así se puede llamar con la `App` prestada clonando
//! antes el `Rc`.

use std::path::Path;

use crate::WM_APP;
use crate::config::ColorRule;
use crate::geom::{Color, Rect};
use crate::paint::Painter;
use crate::shell::{Entry, Item};
use crate::sys::Handle;
use crate::theme::Theme;

// Mensajes que las cajas de texto del host le mandan a la ventana.

/// La barra de dirección terminó de editarse. `wparam` = 1 si hay que navegar.
pub const WM_APP_EDIT_DONE: u32 = WM_APP + 10;
/// El renombrado en línea terminó. `wparam` = 1 si hay que aplicarlo.
pub const WM_APP_RENAME_DONE: u32 = WM_APP + 11;
/// Terminó el renombrado de un favorito. `wparam` = 1 si hay que aplicarlo.
pub const WM_APP_FAVEDIT_DONE: u32 = WM_APP + 12;
/// Terminó la edición del filtro rápido. `wparam` = 1 si se conserva, 0 si se quita (Escape).
pub const WM_APP_FILTER_DONE: u32 = WM_APP + 13;
/// Tecla de la paleta: `wparam` = tecla virtual (0: perdió el foco), `lparam` = 1 si con Ctrl.
pub const WM_APP_PALETTE_KEY: u32 = WM_APP + 14;
/// Flecha / Re Pág / Av Pág en la barra de dirección: recorren el desplegable de rutas.
pub const WM_APP_SUGGEST_KEY: u32 = WM_APP + 15;
/// Cambió el texto de una caja de edición. `lparam` = el número de la `TextBox`.
pub const WM_APP_TEXT_CHANGED: u32 = WM_APP + 16;

/// Para qué es una caja de edición: decide qué mensaje manda al terminar y qué teclas reenvía.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextKind {
    /// Barra de dirección: las flechas recorren el desplegable de rutas; perder el foco cancela.
    Address,
    /// Renombrado en línea: perder el foco aplica (como en el Explorador).
    Rename,
    /// Renombrado de un favorito.
    FavRename,
    /// Filtro rápido.
    Filter,
    /// Paleta: flechas, Re Pág / Av Pág, Enter, Escape y Ctrl+P van a la ventana.
    Palette,
}

impl TextKind {
    /// El mensaje que la caja le manda a la ventana al terminar (Enter, Escape o perder el foco).
    pub fn done_msg(self) -> u32 {
        match self {
            TextKind::Address => WM_APP_EDIT_DONE,
            TextKind::Rename => WM_APP_RENAME_DONE,
            TextKind::FavRename => WM_APP_FAVEDIT_DONE,
            TextKind::Filter => WM_APP_FILTER_DONE,
            TextKind::Palette => WM_APP_PALETTE_KEY,
        }
    }
}

/// Caja de texto nativa de una línea, creada por el host. El número es propio del host (es el
/// `lparam` de `WM_APP_TEXT_CHANGED`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextBox(pub u32);

/// Dónde abrir un menú emergente.
#[derive(Clone, Copy, Debug)]
pub enum MenuPos {
    /// Esquina superior izquierda en este punto (DIPs del área de la ventana).
    At(f32, f32),
    /// Esquina inferior izquierda en este punto: el menú crece hacia arriba.
    Above(f32, f32),
    /// Junto al mouse (menú de clic derecho).
    Cursor,
    /// Junto al mouse, abierto desde otro menú todavía abierto.
    CursorNested,
}

/// La ventana principal. Los tamaños y posiciones están en DIPs: el host lee el DPI de su ventana.
pub trait WindowHost {
    /// La ventana ya existe (en Windows el `HWND` llega después de armar la `App`).
    fn attach(&self, window: Handle);
    /// Pide repintar toda la ventana (sin ventana todavía, no hace nada).
    fn invalidate(&self);
    /// Tamaño del área de la ventana.
    fn client_size(&self) -> (f32, f32);
    /// Posición del mouse en el área de la ventana.
    fn cursor_dips(&self) -> (f32, f32);
    fn set_title(&self, title: &str);
    /// Pide cerrar la ventana (como el botón de cerrar).
    fn request_close(&self);
    /// Devuelve el foco del teclado a la ventana principal.
    fn focus_main(&self);
    /// Devuelve el foco a la ventana solo si nadie lo tiene (se cerró el control que lo tenía).
    fn focus_main_if_lost(&self);
    /// El mouse sigue llegando a la ventana aunque salga de ella (arrastres).
    fn capture_mouse(&self);
    fn release_mouse(&self);
    /// Pide el aviso de que el mouse salió de la ventana.
    fn track_mouse_leave(&self);
    /// Sonido de "no se puede".
    fn beep(&self);
    /// Pregunta Sí / No / Cancelar. Some(true): sí; Some(false): no; None: cancelar.
    fn ask_yes_no_cancel(&self, title: &str, text: &str) -> Option<bool>;
    /// Abre un enlace en el navegador predeterminado.
    fn open_url(&self, url: &str);
    /// Abre un archivo de configuración en el editor de texto.
    fn open_in_editor(&self, path: &Path);
    /// Filas por "clic" de la rueda del mouse.
    fn wheel_lines(&self) -> f32;
    /// Expande variables de entorno (y `~` donde corresponde).
    fn expand_env(&self, s: &str) -> String;
    /// Ajusta la ventana al tema (barra de título, fondo). `follow_system`: el modo claro u
    /// oscuro sigue al del sistema. Devuelve si quedó el fondo translúcido (Mica).
    fn apply_theme(&self, theme: &Theme, follow_system: bool) -> bool;
    /// Selector de color "Personalizado…" a partir de `current` (0xRRGGBB). Devuelve el elegido si
    /// el selector es modal (Windows), o `None` si se canceló o si el color llega después por
    /// mensaje (macOS).
    fn choose_color(&self, current: u32) -> Option<u32>;
    /// Arrastra estos elementos hacia otras ventanas (o dentro de la misma). Cancelar no es error.
    fn start_drag(&self, items: &[Item]) -> Result<(), String>;
    /// Posición y tamaño normales de la ventana (en coordenadas de pantalla) y si está maximizada.
    fn placement(&self) -> Option<([i32; 4], bool)>;
    /// Muestra la ventana con la posición guardada (o el tamaño inicial).
    fn show(&self, saved: Option<([i32; 4], bool)>);
    /// Dibuja un cuadro: `paint` recibe el pintor y el tamaño del área en DIPs. El primer cuadro
    /// crea el destino de dibujo; si el destino se perdió o no se pudo crear, devuelve el error
    /// (sin llamar a `paint` si no llegó a dibujar). `file_colors`: reglas de color por tipo de
    /// archivo de `config.toml`.
    fn draw(&self, theme: &Theme, file_colors: &[ColorRule], paint: &mut dyn FnMut(&dyn Painter, f32, f32)) -> Result<(), String>;
    /// Ancho de un texto con la fuente de la interfaz (o su negrita, la de los encabezados), sin
    /// necesitar un cuadro en curso.
    fn text_width(&self, s: &str, bold: bool) -> f32;

    /// Crea una caja sobre `r` (DIPs del área de la ventana) con `text`, sin foco ni selección.
    /// Al terminar le manda a la ventana `kind.done_msg()`; al cambiar el texto,
    /// `WM_APP_TEXT_CHANGED`.
    fn text_create(&self, kind: TextKind, r: Rect, text: &str, theme: &Theme) -> Option<TextBox>;
    fn text_get(&self, b: TextBox) -> String;
    fn text_set(&self, b: TextBox, text: &str);
    /// Selecciona de `from` a `to` (en unidades UTF-16; `to` = -1: hasta el final).
    fn text_select(&self, b: TextBox, from: usize, to: isize);
    fn text_focus(&self, b: TextBox);
    fn text_has_focus(&self, b: TextBox) -> bool;
    /// Cierra la caja (sin volver a avisar que terminó).
    fn text_destroy(&self, b: TextBox);
    /// Las cajas siguen al tema y al DPI (cambió alguno de los dos).
    fn text_restyle(&self, theme: &Theme);
    /// Muestra un menú y espera la elección. `dot`: el ítem con ese id lleva un punto de ese
    /// color. Devuelve el id elegido (None si se cerró sin elegir). `on_context` se llama con el
    /// camino de posiciones dentro del menú (por ejemplo `[2, 0]`: primer elemento de la tercera
    /// entrada) al hacer clic derecho sobre un elemento; si devuelve `true`, el menú se cierra y
    /// el resultado es `None`. Donde el sistema no avisa ese clic (macOS), no se llama.
    fn popup_menu(&self, entries: &[Entry], pos: MenuPos, dot: Option<(u32, Color)>, on_context: Option<&mut dyn FnMut(&[usize]) -> bool>) -> Option<u32>;
}

//! La ventana de la aplicación vista desde el núcleo: repintar, tamaño, título, foco, mouse,
//! diálogos, tema y arrastre. Cada sistema la implementa sobre su ventana real (`Win32Host`,
//! `MacHost`) y las pruebas con `sys::testing::FakeHost`.
//!
//! A diferencia de los servicios de `sys`, la ventana es una por `App` y solo se usa desde el
//! hilo de interfaz: no es `Send` ni vive en el registro global. Los métodos toman `&self` (el
//! estado propio va en `Cell`/`RefCell`), así se puede llamar con la `App` prestada clonando
//! antes el `Rc`.

use std::path::Path;

use crate::config::ColorRule;
use crate::paint::Painter;
use crate::shell::Item;
use crate::sys::Handle;
use crate::theme::Theme;

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
}

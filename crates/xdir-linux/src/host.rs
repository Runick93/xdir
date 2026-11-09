//! Ventana de la aplicación en Linux, por ahora sin ventana: el núcleo arranca y se puede medir,
//! pero no se dibuja nada ni hay cajas de texto, menús ni diálogos.

use std::path::Path;

use xdir_core::config::ColorRule;
use xdir_core::geom::{Color, Rect};
use xdir_core::host::{MenuPos, OnContext, TextBox, TextKind, WindowHost};
use xdir_core::paint::Painter;
use xdir_core::shell::{Entry, Item};
use xdir_core::sys::Handle;
use xdir_core::theme::Theme;

/// Host sin ventana: tamaño fijo, no dibuja y lo que necesita interfaz no hace nada.
pub struct HeadlessHost;

/// Expande `$VAR` y `${VAR}` con las variables del proceso; las que no existen quedan como estaban.
fn expand_vars(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (name, len) = match after.strip_prefix('{') {
            Some(inner) => inner.find('}').map_or(("", 0), |end| (&inner[..end], end + 2)),
            None => {
                let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
                (&after[..end], end)
            }
        };
        match std::env::var(name).ok().filter(|_| !name.is_empty()) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[i..i + 1 + len]),
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}

impl WindowHost for HeadlessHost {
    fn attach(&self, _window: Handle) {}
    fn invalidate(&self) {}

    fn client_size(&self) -> (f32, f32) {
        (1280.0, 800.0)
    }

    fn cursor_dips(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn set_title(&self, _title: &str) {}
    fn request_close(&self) {}
    fn focus_main(&self) {}
    fn focus_main_if_lost(&self) {}
    fn capture_mouse(&self) {}
    fn release_mouse(&self) {}
    fn track_mouse_leave(&self) {}
    fn beep(&self) {}

    /// Sin diálogo: como cancelar.
    fn ask_yes_no_cancel(&self, _title: &str, _text: &str) -> Option<bool> {
        None
    }

    fn open_url(&self, _url: &str) {}
    fn open_in_editor(&self, _path: &Path) {}

    fn wheel_lines(&self) -> f32 {
        3.0
    }

    fn expand_env(&self, s: &str) -> String {
        expand_vars(s)
    }

    fn apply_theme(&self, _theme: &Theme, _follow_system: bool) -> bool {
        false
    }

    fn choose_color(&self, _current: u32) -> Option<u32> {
        None
    }

    fn start_drag(&self, _items: &[Item]) -> Result<(), String> {
        Err("No disponible en Linux".into())
    }

    fn placement(&self) -> Option<([i32; 4], bool)> {
        None
    }

    fn show(&self, _saved: Option<([i32; 4], bool)>) {}

    /// No hay dónde dibujar: no llama a `paint`.
    fn draw(&self, _theme: &Theme, _file_colors: &[ColorRule], _paint: &mut dyn FnMut(&dyn Painter, f32, f32)) -> Result<(), String> {
        Ok(())
    }

    /// Ancho aproximado: 7 DIPs por carácter.
    fn text_width(&self, s: &str, _bold: bool) -> f32 {
        7.0 * s.chars().count() as f32
    }

    fn text_create(&self, _kind: TextKind, _r: Rect, _text: &str, _theme: &Theme) -> Option<TextBox> {
        None
    }

    fn text_get(&self, _b: TextBox) -> String {
        String::new()
    }

    fn text_set(&self, _b: TextBox, _text: &str) {}
    fn text_select(&self, _b: TextBox, _from: usize, _to: isize) {}
    fn text_focus(&self, _b: TextBox) {}

    fn text_has_focus(&self, _b: TextBox) -> bool {
        false
    }

    fn text_destroy(&self, _b: TextBox) {}
    fn text_restyle(&self, _theme: &Theme) {}

    fn popup_menu(&self, _entries: &[Entry], _pos: MenuPos, _dot: Option<(u32, Color)>, _on_context: Option<OnContext<'_>>) -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use xdir_core::geom::Rect;
    use xdir_core::host::{TextKind, WindowHost};
    use xdir_core::theme::{DEFAULT_ACCENT, Theme};

    use super::HeadlessHost;

    #[test]
    fn headless_host_has_fixed_size_and_no_text_boxes() {
        let host = HeadlessHost;
        assert_eq!(host.client_size(), (1280.0, 800.0));
        let theme = Theme::from_system(DEFAULT_ACCENT, Some(false));
        assert!(host.text_create(TextKind::Address, Rect::default(), "hola", &theme).is_none());
    }
}

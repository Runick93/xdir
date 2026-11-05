//! Tipos y servicios que no dependen del sistema operativo: color, rectángulo, teclas y el
//! "buzón" de la ventana (mensajes que los hilos de trabajo le mandan al hilo de interfaz).
//!
//! Cada sistema implementa lo suyo en un submódulo (`win32`, `macos`) con la misma interfaz;
//! el resto del programa solo usa lo que se reexporta acá.

#[cfg(windows)]
mod win32;
#[cfg(windows)]
pub use self::win32::*;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

/// Color RGBA con componentes de 0 a 1 (sin premultiplicar).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// Rectángulo en DIPs (1 DIP = 1 px a 96 DPI en Windows; 1 punto en macOS).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect { left, top, right, bottom }
}

pub fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Primer número de mensaje libre para la aplicación (igual que `WM_APP` de Windows).
pub const WM_APP: u32 = 0x8000;

/// La ventana principal, vista desde cualquier hilo: sirve para mandarle mensajes de la
/// aplicación (`WM_APP + n`) que se procesan en el hilo de interfaz.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Handle(pub isize);

impl Handle {
    /// Encola un mensaje para el hilo de interfaz y vuelve enseguida.
    pub fn post(self, msg: u32, wparam: usize, lparam: isize) {
        post_message(self, msg, wparam, lparam);
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Tecla con la numeración de las teclas virtuales de Windows (es la que usan `config.toml` y los
/// atajos). En macOS se traduce desde el código de tecla físico.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Key(pub u16);

/// Teclas con nombre (los mismos valores que `VK_*` de Windows).
pub mod keys {
    use super::Key;
    pub const VK_BACK: Key = Key(0x08);
    pub const VK_TAB: Key = Key(0x09);
    pub const VK_RETURN: Key = Key(0x0D);
    pub const VK_SHIFT: Key = Key(0x10);
    pub const VK_CONTROL: Key = Key(0x11);
    pub const VK_ESCAPE: Key = Key(0x1B);
    pub const VK_SPACE: Key = Key(0x20);
    pub const VK_PRIOR: Key = Key(0x21);
    pub const VK_NEXT: Key = Key(0x22);
    pub const VK_END: Key = Key(0x23);
    pub const VK_HOME: Key = Key(0x24);
    pub const VK_LEFT: Key = Key(0x25);
    pub const VK_UP: Key = Key(0x26);
    pub const VK_RIGHT: Key = Key(0x27);
    pub const VK_DOWN: Key = Key(0x28);
    pub const VK_INSERT: Key = Key(0x2D);
    pub const VK_DELETE: Key = Key(0x2E);
    pub const VK_F1: Key = Key(0x70);
    pub const VK_BROWSER_BACK: Key = Key(0xA6);
    pub const VK_BROWSER_FORWARD: Key = Key(0xA7);
}

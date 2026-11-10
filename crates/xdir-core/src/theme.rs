//! Tema claro/oscuro y color de acento, leídos de la configuración del sistema.

use crate::geom::Color;

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub background: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub border: Color,
    pub text: Color,
    pub text_dim: Color,
    /// Datos secundarios de la lista (fecha, tipo, tamaño): entre `text` y `text_dim`.
    pub text_secondary: Color,
    pub accent: Color,
    /// Fondo tenue de la fila con foco (sin seleccionar).
    pub focus_fill: Color,
    /// Fila bajo el mouse.
    pub hover: Color,
    /// Filas seleccionadas.
    pub selection: Color,
    pub scrollbar: Color,
    /// Algo salió bien (tilde de «terminado»).
    pub ok: Color,
    /// Advertencia (ámbar): terminado con problemas.
    pub warning: Color,
}

/// Colores de acento predefinidos (rueda dentada de la barra de estado): (nombre, 0xRRGGBB).
pub const PRESETS: [(&str, u32); 12] = [
    ("Azul", 0x3B9EFF),
    ("Celeste", 0x4CC2FF),
    ("Turquesa", 0x2EC4B6),
    ("Verde", 0x3FB950),
    ("Lima", 0xA3D65C),
    ("Amarillo", 0xE3B341),
    ("Naranja", 0xF0883E),
    ("Rojo", 0xF85149),
    ("Rosa", 0xF778BA),
    ("Violeta", 0xA371F7),
    ("Índigo", 0x6E7BFF),
    ("Gris", 0x8B98A9),
];

/// Acento por defecto: el azul del diseño.
pub const DEFAULT_ACCENT: Accent = Accent::Rgb(PRESETS[0].1);

/// De dónde sale el color de acento.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    /// El acento de Windows (Configuración > Personalización > Colores).
    System,
    /// Un color elegido: 0xRRGGBB.
    Rgb(u32),
}

impl Theme {
    /// Arma la paleta con el acento elegido. `dark`: modo fijado por el usuario; sin él, el
    /// modo de apps de Windows.
    pub fn from_system(accent: Accent, dark: Option<bool>) -> Self {
        let dark = dark.unwrap_or_else(|| crate::sys::env().system_dark());

        let accent = match accent {
            Accent::Rgb(c) => rgb(c >> 16, (c >> 8) & 0xFF, c & 0xFF),
            Accent::System => crate::sys::env().system_accent()
                .map(|c| rgb(c >> 16, (c >> 8) & 0xFF, c & 0xFF))
                .unwrap_or(rgb(0x00, 0x78, 0xD4)),
        };

        // Oscuro con tinte azulado y superficies apenas más claras que el fondo; los paneles
        // se separan por su borde, no por contraste de relleno.
        if dark {
            Self {
                dark,
                background: rgb(0x0B, 0x0F, 0x14),
                surface: rgb(0x10, 0x15, 0x1C),
                surface_alt: rgb(0x15, 0x1B, 0x23),
                border: rgb(0x26, 0x30, 0x3C),
                text: rgb(0xE6, 0xED, 0xF3),
                text_dim: rgb(0x8B, 0x98, 0xA9),
                text_secondary: rgb(0xB4, 0xBF, 0xCC),
                accent,
                focus_fill: alpha(accent, 0.10),
                hover: rgba(0xFF, 0xFF, 0xFF, 0.05),
                selection: alpha(accent, 0.26),
                scrollbar: rgba(0xFF, 0xFF, 0xFF, 0.22),
                ok: rgb(0x33, 0xB8, 0x61),
                warning: rgb(0xFA, 0xC7, 0x4D),
            }
        } else {
            Self {
                dark,
                background: rgb(0xEE, 0xF1, 0xF5),
                surface: rgb(0xFF, 0xFF, 0xFF),
                surface_alt: rgb(0xF6, 0xF8, 0xFA),
                border: rgb(0xD5, 0xDC, 0xE4),
                text: rgb(0x1F, 0x23, 0x28),
                text_dim: rgb(0x59, 0x63, 0x6E),
                text_secondary: rgb(0x3D, 0x46, 0x50),
                accent,
                focus_fill: alpha(accent, 0.08),
                hover: rgba(0x00, 0x00, 0x00, 0.045),
                selection: alpha(accent, 0.20),
                scrollbar: rgba(0x00, 0x00, 0x00, 0.28),
                ok: rgb(0x33, 0xB8, 0x61),
                warning: rgb(0xB7, 0x79, 0x1F),
            }
        }
    }
}

fn rgb(r: u32, g: u32, b: u32) -> Color {
    rgba(r, g, b, 1.0)
}

fn rgba(r: u32, g: u32, b: u32, a: f32) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

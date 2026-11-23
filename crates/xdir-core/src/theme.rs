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
    /// Error o choque (rojo del diseño de la tanda D: renombrar varios, errores de la copia).
    pub danger: Color,
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
        Self::with_background(accent, dark, None)
    }

    /// Lo mismo con un fondo elegido (0xRRGGBB). Con fondo propio el modo claro u oscuro sale del
    /// fondo (el más legible para su luminosidad) y las superficies, bordes y textos se derivan de
    /// él, así el texto siempre se lee.
    pub fn with_background(accent: Accent, dark: Option<bool>, background: Option<u32>) -> Self {
        let dark = match background {
            Some(bg) => !bg_is_light(bg),
            None => dark.unwrap_or_else(|| crate::sys::env().system_dark()),
        };

        let accent = match accent {
            Accent::Rgb(c) => rgb(c >> 16, (c >> 8) & 0xFF, c & 0xFF),
            Accent::System => crate::sys::env().system_accent()
                .map(|c| rgb(c >> 16, (c >> 8) & 0xFF, c & 0xFF))
                .unwrap_or(rgb(0x00, 0x78, 0xD4)),
        };

        // Oscuro con tinte azulado y superficies apenas más claras que el fondo; los paneles
        // se separan por su borde, no por contraste de relleno.
        let mut theme = if dark {
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
                danger: rgb(0xEB, 0x52, 0x57),
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
                danger: rgb(0xEB, 0x52, 0x57),
            }
        };
        if let Some(bg) = background {
            theme.derive_from_background(bg);
        }
        theme
    }

    /// Reemplaza el fondo y deriva de él las superficies, el borde y los textos: las superficies
    /// se corren hacia el texto, el borde un poco más, y el texto secundario se acerca al fondo.
    fn derive_from_background(&mut self, bg: u32) {
        let base = rgb(bg >> 16, (bg >> 8) & 0xFF, bg & 0xFF);
        let toward = if self.dark { WHITE } else { BLACK };
        self.background = base;
        self.surface = mix(base, toward, if self.dark { 0.035 } else { 0.0 });
        self.surface_alt = mix(base, toward, if self.dark { 0.07 } else { 0.035 });
        if !self.dark {
            // En claro las superficies son más claras que el fondo, como en el tema de fábrica.
            self.surface = mix(base, WHITE, 0.6);
            self.surface_alt = mix(base, WHITE, 0.3);
        }
        self.border = mix(base, toward, if self.dark { 0.16 } else { 0.14 });
        // Texto casi blanco o casi negro (el más legible sobre este fondo) y de él los tenues.
        self.text = mix(if self.dark { WHITE } else { BLACK }, base, 0.04);
        self.text_dim = mix(self.text, base, 0.35);
        self.text_secondary = mix(self.text, base, 0.15);
        self.hover = Color { a: if self.dark { 0.05 } else { 0.045 }, ..toward };
        self.scrollbar = Color { a: if self.dark { 0.22 } else { 0.28 }, ..toward };
    }
}

const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

/// Luminancia relativa (WCAG) de 0xRRGGBB.
fn luminance(c: u32) -> f32 {
    let lin = |v: u32| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c >> 16 & 0xFF) + 0.7152 * lin(c >> 8 & 0xFF) + 0.0722 * lin(c & 0xFF)
}

/// ¿Con este fondo se lee mejor texto oscuro? Es el punto donde el contraste con blanco y con
/// negro se iguala (luminancia ≈ 0,179).
pub fn bg_is_light(bg: u32) -> bool {
    luminance(bg) > 0.179
}

/// `a` corrido `t` (0 a 1) hacia `b`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1.0 }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color) -> f32 {
        luminance(((c.r * 255.0).round() as u32) << 16 | ((c.g * 255.0).round() as u32) << 8 | (c.b * 255.0).round() as u32)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn without_background_nothing_changes() {
        let a = Theme::from_system(DEFAULT_ACCENT, Some(true));
        let b = Theme::with_background(DEFAULT_ACCENT, Some(true), None);
        assert_eq!(a.background, b.background);
        assert_eq!(a.surface, b.surface);
        assert_eq!(a.text, b.text);
    }

    #[test]
    fn background_decides_the_mode() {
        assert!(Theme::with_background(DEFAULT_ACCENT, Some(false), Some(0x101820)).dark);
        assert!(!Theme::with_background(DEFAULT_ACCENT, Some(true), Some(0xF4EFE6)).dark);
    }

    #[test]
    fn every_background_keeps_text_readable() {
        // Una grilla de fondos de todo el espectro: el texto y el secundario deben leerse.
        for r in (0..=255).step_by(51) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(51) {
                    let bg = r << 16 | g << 8 | b;
                    let t = Theme::with_background(DEFAULT_ACCENT, None, Some(bg));
                    assert_eq!(t.background, rgb(r, g, b));
                    assert!(contrast(t.text, t.background) >= 4.0, "texto en {bg:06X}: {}", contrast(t.text, t.background));
                    assert!(contrast(t.text_secondary, t.background) >= 3.0, "secundario en {bg:06X}");
                    assert!(contrast(t.text_dim, t.background) >= 2.0, "tenue en {bg:06X}");
                }
            }
        }
    }
}

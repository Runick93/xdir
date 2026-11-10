//! Dibujo neutro: el `Painter` que implementa cada plataforma (Direct2D en Windows, Core Graphics
//! en macOS) y las fuentes con que dibuja. Coordenadas en DIPs.

use crate::config::ColorRule;
use crate::geom::{Color, Rect};
use crate::theme::Theme;
use crate::thumbs;

/// Hay íconos nuevos listos para dibujar: lo mandan los hilos de íconos y miniaturas, y la `App`
/// repinta.
pub const WM_APP_ICONS: u32 = crate::WM_APP + 3;

/// Formatos de texto de la interfaz; cada plataforma tiene uno por valor (misma fuente, tamaño,
/// peso y alineación en las dos). El orden es el índice en la tabla de formatos.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum FontId {
    /// Texto normal, alineado a la izquierda, con "…" si no entra.
    Ui,
    UiRight,
    UiCenter,
    UiBold,
    /// Títulos (16, seminegrita): barra superior y tarjeta de xtree.
    Title,
    /// Texto chico (11,5): rótulos del mapa y de la barra de datos de xtree.
    Small,
    /// Texto chico en seminegrita (12,5): carpetas de primer nivel del mapa de xtree.
    SmallBold,
    /// Íconos de interfaz (Segoe Fluent Icons en Windows, SF Symbols en macOS).
    Glyphs,
    GlyphsSmall,
    /// Nombre debajo de una miniatura: centrado, arriba, partido en varias líneas y con "…" si no entra.
    Label,
    /// Ancho fijo (11,5; Consolas en Windows, Menlo en macOS), sin ajustar líneas: el texto de
    /// la vista previa.
    Mono,
}

impl FontId {
    /// Cantidad de formatos (largo de la tabla de cada plataforma).
    pub const COUNT: usize = 11;
}

// `Mono` es el último formato: si se agrega uno, hay que actualizar `COUNT`.
const _: () = assert!(FontId::Mono as usize + 1 == FontId::COUNT);

/// Píxeles BGRA premultiplicados.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Lo que se puede dibujar en un cuadro. Sin genéricos, para usarlo como `&dyn Painter`.
pub trait Painter {
    fn fill(&self, r: Rect, c: Color);
    fn round_fill(&self, r: Rect, radius: f32, c: Color);
    /// Polígono con esquinas redondeadas: cada punto es (x, y, radio de la esquina). Abierto, los
    /// extremos no se redondean.
    fn rounded_path(&self, pts: &[(f32, f32, f32)], closed: bool, fill: Option<Color>, stroke: Option<(Color, f32)>);
    fn circle(&self, cx: f32, cy: f32, radius: f32, fill: Color);
    fn circle_stroke(&self, cx: f32, cy: f32, radius: f32, c: Color, width: f32);
    /// Contorno redondeado de grosor arbitrario.
    fn round_stroke_w(&self, r: Rect, radius: f32, c: Color, width: f32);
    fn round_stroke(&self, r: Rect, radius: f32, c: Color);
    fn text(&self, font: FontId, s: &str, r: Rect, c: Color);
    /// Ancho de un texto en DIPs.
    fn measure(&self, font: FontId, s: &str) -> f32;
    /// Dibuja el ícono `index` del sistema en un cuadrado de `size` DIPs.
    fn shell_icon(&self, index: i32, overlay: u8, x: f32, y: f32, size: f32);
    /// Miniatura centrada en `r`, sin deformar y sin agrandarla más que su tamaño en píxeles.
    /// Devuelve false si todavía no está (se dibuja otra cosa).
    fn thumbnail(&self, key: &thumbs::Key, r: Rect) -> bool;
    /// Imagen grande (vista previa, Galería, vista rápida) centrada en `r` sin deformar. `key.1`
    /// es el escalón pedido (`thumbs::big_side`); si todavía no está, lo encarga y dibuja escalado
    /// el escalón más grande que haya de la misma fuente (`BigCache::best_for`). Devuelve false si
    /// no hay ninguno (se dibuja el ícono).
    fn big_image(&self, key: &thumbs::Key, r: Rect) -> bool;
    /// Olvida las imágenes grandes de `src` (el archivo cambió): la próxima `big_image` la vuelve
    /// a pedir. Donde no hay caché, no hace nada.
    fn forget_big(&self, _src: &thumbs::Source) {}
    /// Logo de Runick centrado en `r` sin deformarlo.
    fn logo(&self, r: Rect);
    /// Lado en píxeles para pedir una miniatura de `dips` DIPs.
    fn thumb_px(&self, dips: f32) -> u32;
    /// Dibuja lo que haga `f` recortado a `r`. Si hace falta un resultado, `f` lo deja en una
    /// variable capturada.
    fn clip(&self, r: Rect, f: &mut dyn FnMut());
    fn theme(&self) -> &Theme;
    /// Reglas de color por tipo de archivo de `config.toml` (vacías si están apagadas).
    fn file_colors(&self) -> &[ColorRule];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Color, rect};
    use crate::sys::testing::RecordingPainter;
    use crate::theme::{DEFAULT_ACCENT, Theme};
    use std::collections::HashSet;

    #[test]
    fn font_ids_are_distinct() {
        let all = [
            FontId::Ui,
            FontId::UiRight,
            FontId::UiCenter,
            FontId::UiBold,
            FontId::Title,
            FontId::Small,
            FontId::SmallBold,
            FontId::Glyphs,
            FontId::GlyphsSmall,
            FontId::Label,
            FontId::Mono,
        ];
        assert_eq!(all.len(), FontId::COUNT);
        assert_eq!(all.iter().collect::<HashSet<_>>().len(), 11);
    }

    #[test]
    fn recording_painter_records_text() {
        crate::sys::testing::install_fakes();
        let p = RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)));
        let r = rect(10.0, 20.0, 110.0, 40.0);
        let painter: &dyn Painter = &p;
        painter.text(FontId::UiRight, "12 KB", r, Color::default());
        assert_eq!(p.texts(), [(FontId::UiRight, "12 KB".to_string(), r)]);
        assert_eq!(painter.measure(FontId::Ui, "abc"), 21.0);
    }
}

//! Color y rectángulo.

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

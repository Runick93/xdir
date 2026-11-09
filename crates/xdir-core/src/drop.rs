//! Arrastrar y soltar archivos sobre la ventana: los tipos neutros. Cada sistema arma un
//! `DragEvent` por evento y se lo pasa a la `App` (`on_drag`), que decide la carpeta destino y el
//! efecto. En Windows viaja con `WM_APP_DRAG` (en el mismo hilo, llamada directa).

use std::path::PathBuf;

use crate::WM_APP;

/// Evento de arrastre para la ventana. `lparam` apunta a un `DragEvent` (en macOS la vista llama
/// directo a la ventana).
pub const WM_APP_DRAG: u32 = WM_APP + 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragKind {
    Enter,
    Over,
    Leave,
    Drop,
}

/// Efectos de soltar, combinables (los mismos valores que `DROPEFFECT` de Windows).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Effect(pub u32);

impl Effect {
    pub const NONE: Effect = Effect(0);
    pub const COPY: Effect = Effect(1);
    pub const MOVE: Effect = Effect(2);
    pub const LINK: Effect = Effect(4);
    pub const ALL: Effect = Effect(7);

    pub fn allows(self, e: Effect) -> bool {
        self.0 & e.0 != 0
    }
}

pub struct DragEvent<'a> {
    pub kind: DragKind,
    /// Posición en el área de la ventana, en DIPs.
    pub point: (f32, f32),
    /// Teclas apretadas: Ctrl (⌥ en macOS) copia, Shift (⌘ en macOS) mueve.
    pub ctrl: bool,
    pub shift: bool,
    /// Rutas arrastradas (solo archivos de disco; None si el origen no tiene rutas).
    pub paths: Option<&'a [PathBuf]>,
    /// Efectos que permite el origen.
    pub allowed: Effect,
    /// Respuesta de la ventana.
    pub effect: Effect,
}

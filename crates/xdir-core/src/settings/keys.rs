//! Atajos de teclado en la configuración: la grabación en curso y lo que muestra cada fila.

use crate::commands::{Combo, Command};
use crate::keys::Key;
use crate::sys::KeyStyle;

/// Grabando un atajo para `command`.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording {
    pub command: Command,
    /// Chip que se reemplaza; `None`: se agrega uno más.
    pub replace: Option<usize>,
    /// La combinación grabada ya es de otro comando: se pregunta si reasignarla.
    pub conflict: Option<(Combo, Command)>,
    /// Rechazo con su motivo («La usa el sistema»).
    pub message: Option<&'static str>,
    /// La combinación rechazada (para decir cuál).
    pub pressed: Option<Combo>,
}

impl Recording {
    pub fn new(command: Command, replace: Option<usize>) -> Self {
        Self { command, replace, conflict: None, message: None, pressed: None }
    }
}

/// Cómo está una fila de Atajos.
#[derive(Clone, Debug, PartialEq)]
pub enum RowState {
    Normal,
    /// «Apretá la combinación…».
    Recording,
    /// «⌘K ya es de "Conectar a un servidor…"», con Reasignar y Cancelar.
    Conflict(String),
    /// La combinación apretada no se puede usar: el motivo.
    Rejected(String),
}

/// Una fila de Atajos: el comando, sus atajos como chips y su estado.
#[derive(Clone, Debug, PartialEq)]
pub struct ShortcutRow {
    /// Id del comando en `config.toml` (también el id de las partes de la fila).
    pub id: &'static str,
    pub title: String,
    pub chips: Vec<String>,
    /// Distinto de los de fábrica.
    pub changed: bool,
    pub state: RowState,
}

/// Qué hacer con una tecla apretada mientras se graba.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Capture {
    /// Un modificador solo: se sigue esperando.
    Wait,
    /// Esc solo: se deja de grabar.
    Cancel,
    /// No se puede usar: el motivo.
    Reject(&'static str),
    Accept(Combo),
}

/// Las que usa la lista sin Ctrl, Alt ni Meta (con o sin Shift).
pub const LIST_KEY: &str = "Esa tecla ya la usa la lista";
pub const SYSTEM_KEY: &str = "La usa el sistema";
pub const UNNAMED_KEY: &str = "Esa tecla no se puede usar como atajo";
/// Para un comando de xtree: las teclas propias de la vista (F5, Retroceso, Esc, flechas, Enter).
pub const XTREE_KEY: &str = "Esa tecla ya la usa xtree";

/// Clasifica la tecla grabada (`Ctrl` es ⌘ en macOS y `meta` es ⌃; en Windows y Linux `meta` es
/// la tecla Windows/Super).
pub fn classify(vk: Key, ctrl: bool, shift: bool, alt: bool, meta: bool, style: KeyStyle) -> Capture {
    use crate::keys::keys::{VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_SHIFT};
    const VK_RWIN: Key = Key(0x5C);
    const VK_TAB: u16 = 0x09;
    const VK_DELETE: u16 = 0x2E;
    const VK_F4: u16 = 0x73;
    const VK_LSHIFT_RANGE: std::ops::RangeInclusive<u16> = 0xA0..=0xA5;
    if [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].contains(&vk) || VK_LSHIFT_RANGE.contains(&vk.0) {
        return Capture::Wait;
    }
    if vk == VK_ESCAPE && !ctrl && !shift && !alt && !meta {
        return Capture::Cancel;
    }
    let combo = Combo { vk: vk.0, ctrl, shift, alt, meta };
    let is = |text: &str| crate::commands::parse_combo(text).is_ok_and(|c| c == combo);
    match style {
        KeyStyle::Mac => {
            if ["Ctrl+Q", "Ctrl+H", "Ctrl+Alt+H", "Ctrl+Tab", "Ctrl+Espacio", "Meta+Ctrl+Q"].into_iter().any(is) {
                return Capture::Reject(SYSTEM_KEY);
            }
        }
        // Windows y Linux comparten estilo: Alt+F4, Alt+Tab, Ctrl+Alt+Supr y todo lo de Win/Super.
        KeyStyle::Windows => {
            let ctrl_alt_del = vk.0 == VK_DELETE && ctrl && alt;
            if meta || (alt && !ctrl && (vk.0 == VK_F4 || vk.0 == VK_TAB)) || ctrl_alt_del {
                return Capture::Reject(SYSTEM_KEY);
            }
        }
    }
    // Sin Ctrl, Alt ni Meta: las teclas de la lista (flechas, Enter, Retroceso, Inicio, Fin,
    // RePág, AvPág, Espacio y la escritura para saltar por nombre).
    if !ctrl && !alt && !meta && list_key(vk.0) {
        return Capture::Reject(LIST_KEY);
    }
    if !crate::commands::nameable(vk.0) {
        return Capture::Reject(UNNAMED_KEY);
    }
    Capture::Accept(combo)
}

/// Teclas fijas de la lista sin modificadores (o solo con Shift).
fn list_key(vk: u16) -> bool {
    matches!(vk,
        0x08 | 0x0D | 0x20 | 0x21..=0x28 // Retroceso, Enter, Espacio, RePág, AvPág, Fin, Inicio, flechas
        | 0x30..=0x39 | 0x41..=0x5A // números y letras
        | 0x60..=0x6F // teclado numérico
        | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 // signos
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::parse_combo;

    fn c(k: char) -> Key {
        Key(k as u16)
    }

    #[test]
    fn classify_rules() {
        use crate::keys::keys::{VK_ESCAPE, VK_F1, VK_SHIFT};
        let w = KeyStyle::Windows;
        assert_eq!(classify(VK_ESCAPE, false, false, false, false, w), Capture::Cancel);
        assert_eq!(classify(VK_SHIFT, false, true, false, false, w), Capture::Wait);
        assert_eq!(classify(c('A'), false, false, false, false, w), Capture::Reject(LIST_KEY));
        assert_eq!(classify(c('A'), false, true, false, false, w), Capture::Reject(LIST_KEY));
        assert_eq!(classify(Key(0x28), false, false, false, false, w), Capture::Reject(LIST_KEY));
        assert_eq!(classify(c('A'), true, false, false, false, w), Capture::Accept(parse_combo("Ctrl+A").unwrap()));
        assert_eq!(classify(c('Q'), true, false, false, false, KeyStyle::Mac), Capture::Reject(SYSTEM_KEY));
        assert_eq!(classify(c('Q'), true, false, false, true, KeyStyle::Mac), Capture::Reject(SYSTEM_KEY));
        assert_eq!(classify(c('Q'), true, false, false, false, w), Capture::Accept(parse_combo("Ctrl+Q").unwrap()));
        assert_eq!(classify(Key(0x73), false, false, true, false, w), Capture::Reject(SYSTEM_KEY), "Alt+F4");
        assert_eq!(classify(c('E'), false, false, false, true, w), Capture::Reject(SYSTEM_KEY), "Win+E");
        assert_eq!(classify(Key(0x2E), true, false, true, false, w), Capture::Reject(SYSTEM_KEY), "Ctrl+Alt+Supr");
        let f3 = Key(VK_F1.0 + 2);
        assert_eq!(classify(f3, false, false, false, false, w), Capture::Accept(parse_combo("F3").unwrap()));
        // ⌃⌘N sí se puede en la Mac.
        assert_eq!(classify(c('N'), true, false, false, true, KeyStyle::Mac), Capture::Accept(parse_combo("Meta+Ctrl+N").unwrap()));
        assert_eq!(classify(Key(0xBA), true, false, false, false, w), Capture::Reject(UNNAMED_KEY));
    }
}

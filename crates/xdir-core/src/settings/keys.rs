//! Atajos de teclado en la configuración: la grabación en curso y lo que muestra cada fila.

use crate::commands::{Combo, Command};

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

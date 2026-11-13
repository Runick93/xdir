//! Historial de tamaños de xtree (#58) del lado de la App: la carpeta y las opciones que necesita
//! la vista.

use super::*;
use super::tasks::TaskDone;
use crate::config::ConfigEdit;

impl App {
    /// Le pasa a la vista de xtree la opción `xtree_historial`, el período y la carpeta del
    /// historial (`<configuración>/xtree-historial`).
    pub(super) fn setup_xtree_history(&mut self) {
        let dir = crate::session::config_dir().map(|d| d.join(crate::xtree::history::DIR_NAME));
        let (on, days) = (self.config.xtree_history, self.xtree_days);
        if let Some(x) = &mut self.xtree {
            x.set_history(on, days, dir);
        }
    }

    /// Switch «Guardar historial de tamaños»: `xtree_historial = false` en `config.toml` (encendido
    /// es lo de fábrica: se quita la línea).
    pub(super) fn toggle_xtree_history(&mut self) {
        let on = !self.config.xtree_history;
        self.config.xtree_history = on;
        let key = "xtree_historial".to_string();
        self.edit_config(vec![if on {
            ConfigEdit::Remove { section: "opciones", key }
        } else {
            ConfigEdit::Set { section: "opciones", key, value: "false".into() }
        }]);
        if self.xtree.is_some() {
            self.setup_xtree_history();
        }
    }

    /// «Borrar historial»: pregunta y borra la carpeta `xtree-historial` en un hilo.
    pub(super) fn clear_xtree_history(&mut self) {
        let question = "¿Borrar el historial de tamaños de xtree? Se borran las fotos de todas las carpetas analizadas.";
        if !self.host.ask_yes_no("Borrar historial", question) {
            return;
        }
        let Some(dir) = crate::session::config_dir().map(|d| d.join(crate::xtree::history::DIR_NAME)) else { return };
        self.spawn_task(move || TaskDone::XtreeHistoryCleared(crate::xtree::history::clear(&dir)));
    }

    /// Terminó «Borrar historial»: la vista deja de mostrar el que tenía.
    pub(super) fn on_xtree_history_cleared(&mut self, result: Result<(), String>) {
        if result.is_ok()
            && let Some(x) = &mut self.xtree
        {
            x.clear_history();
        }
        self.xtree_history_msg = Some(result.map_or_else(|e| e, |()| "Historial borrado".to_string()));
        self.redraw();
    }
}

#[cfg(test)]
#[path = "xtree_history_tests.rs"]
mod tests;

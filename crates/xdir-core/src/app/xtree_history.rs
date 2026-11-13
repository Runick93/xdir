//! Historial de tamaños de xtree (#58) del lado de la App: la carpeta y las opciones que necesita
//! la vista.

use super::*;

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
}

#[cfg(test)]
#[path = "xtree_history_tests.rs"]
mod tests;

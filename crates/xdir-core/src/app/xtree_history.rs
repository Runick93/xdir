//! Historial de tamaños de xtree (#58) del lado de la App: la carpeta y las opciones que necesita
//! la vista.

use super::*;

impl App {
    /// Le pasa a la vista de xtree la opción `xtree_historial`, el período y la carpeta del
    /// historial (`<configuración>/xtree-historial`).
    pub(super) fn setup_xtree_history(&mut self) {
        let dir = crate::session::config_dir().map(|d| d.join(crate::xtree::history::DIR_NAME));
        let on = self.config.xtree_history;
        if let Some(x) = &mut self.xtree {
            let days = x.history_days();
            x.set_history(on, days, dir);
        }
    }
}

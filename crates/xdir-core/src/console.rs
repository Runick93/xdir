//! Abrir una consola (PowerShell, CMD o Windows Terminal en Windows; Terminal o iTerm en macOS;
//! la terminal del sistema en Linux) en la carpeta de un panel.
//! La lista y la forma de abrirlas las da la plataforma (`sys::env()`).

use std::path::Path;

use crate::sys::{ConsoleInfo, Handle, env};

/// Una consola de la lista del sistema: un índice opaco de la plataforma y su nombre.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Console {
    id: u8,
    label: &'static str,
}

impl From<ConsoleInfo> for Console {
    fn from(c: ConsoleInfo) -> Self {
        Self { id: c.id, label: c.label }
    }
}

impl Console {
    /// Las consolas del sistema, en el orden del menú (al menos una).
    pub fn all() -> Vec<Console> {
        env().consoles().into_iter().map(Console::from).collect()
    }

    /// La consola `i` del menú; si el sistema tiene menos, la primera. Los comandos "PowerShell"
    /// y "CMD" piden la 0 y la 1: en Windows, PowerShell y CMD (Windows Terminal, si está, es la
    /// 2); en macOS la 1 es iTerm si está instalada (si no, Terminal); en Linux hay una sola.
    pub fn nth(i: usize) -> Console {
        let all = Self::all();
        all.get(i).or(all.first()).copied().expect("el sistema no ofrece ninguna consola")
    }

    pub fn label(self) -> &'static str {
        self.label
    }

    /// Otras formas de abrirla (clic derecho en el menú de terminales): (verbo, texto del menú).
    pub fn alt_verbs() -> &'static [(&'static str, &'static str)] {
        env().console_alt_verbs()
    }

    /// Abre la consola con `dir` como carpeta de trabajo.
    pub fn open(self, owner: Handle, dir: &Path) -> bool {
        self.open_as(owner, dir, "open")
    }

    /// Igual, con uno de los verbos de `alt_verbs` (en Windows: `runas`, `runasuser`).
    pub fn open_as(self, owner: Handle, dir: &Path, verb: &str) -> bool {
        env().open_console(owner, self.id, dir, verb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::install_fakes;

    #[test]
    fn nth_falls_back_to_the_first() {
        install_fakes();
        let all = Console::all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].label(), "Terminal");
        assert_eq!(Console::nth(0), all[0]);
        assert_eq!(Console::nth(1), all[0]);
    }
}

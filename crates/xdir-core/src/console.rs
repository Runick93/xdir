//! Abrir una consola (PowerShell, CMD, Windows Terminal o Git Bash en Windows; Terminal, iTerm,
//! Ghostty, Warp… en macOS; las terminales del PATH en Linux) en la carpeta de un panel.
//! La lista y la forma de abrirlas las da la plataforma (`sys::env()`); se le suma la terminal
//! propia de `config.toml` (`terminal_propia`). Cuál se usa por defecto lo dice `terminal`.

use std::path::Path;
use std::sync::Mutex;

use crate::sys::{ConsoleInfo, Handle, env};

/// Clave de la terminal propia (la de `terminal_propia` en `config.toml`).
pub const CUSTOM_KEY: &str = "propia";

/// Una consola del menú: la plataforma la conoce por `id`; `key` es la que se guarda en
/// `config.toml`. La terminal propia lleva su programa en `program`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Console {
    id: u8,
    key: &'static str,
    label: &'static str,
    program: Option<&'static str>,
}

impl From<ConsoleInfo> for Console {
    fn from(c: ConsoleInfo) -> Self {
        Self { id: c.id, key: c.key, label: c.label, program: None }
    }
}

impl Console {
    /// Las consolas del sistema en el orden del menú (al menos una) y al final la propia, si hay.
    pub fn all(custom: Option<&str>) -> Vec<Console> {
        let mut list: Vec<Console> = env().consoles().into_iter().map(Console::from).collect();
        if let Some(c) = custom.and_then(Console::custom) {
            list.push(c);
        }
        list
    }

    /// La terminal propia: `program` es lo escrito en `terminal_propia` (sin espacios alrededor).
    pub fn custom(program: &str) -> Option<Console> {
        let program = program.trim();
        if program.is_empty() {
            return None;
        }
        Some(Console { id: u8::MAX, key: CUSTOM_KEY, label: intern(&custom_label(program)), program: Some(intern(program)) })
    }

    /// La de clave `key`, si está.
    pub fn by_key(key: &str, custom: Option<&str>) -> Option<Console> {
        Self::all(custom).into_iter().find(|c| c.key == key)
    }

    /// La terminal por defecto: la de clave `key` si está (instalada, o la propia si se escribió);
    /// si no, la primera del sistema.
    pub fn preferred(key: Option<&str>, custom: Option<&str>) -> Console {
        let all = Self::all(custom);
        key.and_then(|k| all.iter().find(|c| c.key == k)).or(all.first()).copied().expect("el sistema no ofrece ninguna consola")
    }

    pub fn key(self) -> &'static str {
        self.key
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
        match self.program {
            Some(program) => env().open_custom_console(owner, program, dir, verb),
            None => env().open_console(owner, self.id, dir, verb),
        }
    }
}

/// Nombre de la terminal propia en el menú: el del archivo, sin carpeta ni extensión de programa
/// (`/Applications/Hyper.app` → «Hyper», `C:\Git\git-bash.exe` → «git-bash»).
pub fn custom_label(program: &str) -> String {
    let name = program.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = name.to_lowercase();
    let stem = [".app", ".exe", ".lnk", ".bat", ".cmd"].iter().find(|e| lower.ends_with(*e)).map_or(name, |e| &name[..name.len() - e.len()]);
    if stem.is_empty() { program.to_string() } else { stem.to_string() }
}

/// El mismo texto con vida `'static`: las consolas son `Copy` y la propia sale de `config.toml`.
/// Cada texto distinto se guarda una sola vez (cambia solo si se edita `terminal_propia`).
fn intern(s: &str) -> &'static str {
    static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = seen.iter().find(|x| **x == s) {
        return s;
    }
    let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
    seen.push(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::install_fakes;

    #[test]
    fn preferred_by_key_or_the_first() {
        install_fakes();
        let all = Console::all(None);
        assert_eq!(all.iter().map(|c| c.key()).collect::<Vec<_>>(), ["terminal", "otra"]);
        assert_eq!(Console::preferred(None, None), all[0]);
        assert_eq!(Console::preferred(Some("otra"), None), all[1]);
        assert_eq!(Console::preferred(Some("no-instalada"), None), all[0], "si no está, la primera");
        assert_eq!(Console::preferred(Some(CUSTOM_KEY), None), all[0], "la propia sin terminal_propia");
    }

    #[test]
    fn custom_goes_last() {
        install_fakes();
        let all = Console::all(Some(" /Applications/Hyper.app "));
        assert_eq!(all.len(), 3);
        assert_eq!((all[2].key(), all[2].label()), (CUSTOM_KEY, "Hyper"));
        assert_eq!(Console::preferred(Some(CUSTOM_KEY), Some("/Applications/Hyper.app")), all[2]);
        assert_eq!(Console::all(Some("  ")).len(), 2, "vacía no cuenta");
    }

    #[test]
    fn custom_labels() {
        assert_eq!(custom_label("/Applications/Hyper.app"), "Hyper");
        assert_eq!(custom_label("/Applications/Hyper.app/"), "Hyper");
        assert_eq!(custom_label(r"C:\Program Files\Git\git-bash.exe"), "git-bash");
        assert_eq!(custom_label("Tabby"), "Tabby");
        assert_eq!(custom_label("/usr/bin/foot"), "foot");
        assert_eq!(custom_label(".app"), ".app");
    }
}

//! Abrir una consola (PowerShell o CMD) en la carpeta de un panel.

use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
#[cfg(windows)]
use windows::core::{HSTRING, PCWSTR};

#[cfg(windows)]
use crate::fs::wide_path;
use crate::platform::Handle;
#[cfg(windows)]
use crate::platform::HandleExt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Console {
    PowerShell,
    Cmd,
}

impl Console {
    #[cfg(windows)]
    pub const ALL: [Console; 2] = [Console::PowerShell, Console::Cmd];
    /// En macOS hay una sola: Terminal.
    #[cfg(not(windows))]
    pub const ALL: [Console; 1] = [Console::PowerShell];

    pub fn label(self) -> &'static str {
        #[cfg(not(windows))]
        return "Terminal";
        #[cfg(windows)]
        match self {
            Console::PowerShell => "PowerShell",
            Console::Cmd => "CMD",
        }
    }

    /// Ejecutable: PowerShell 7 si está instalado, si no el de Windows.
    #[cfg(windows)]
    fn program(self) -> PathBuf {
        match self {
            Console::PowerShell => {
                let pwsh = std::env::var_os("ProgramFiles")
                    .map(|p| PathBuf::from(p).join(r"PowerShell\7\pwsh.exe"))
                    .filter(|p| p.exists());
                pwsh.unwrap_or_else(|| PathBuf::from("powershell.exe"))
            }
            Console::Cmd => PathBuf::from("cmd.exe"),
        }
    }

    /// Otras formas de abrirla (clic derecho en el menú de terminales): (verbo, texto del menú).
    pub fn alt_verbs() -> &'static [(&'static str, &'static str)] {
        #[cfg(windows)]
        return &[("runas", "Ejecutar como administrador"), ("runasuser", "Ejecutar como otro usuario")];
        #[cfg(not(windows))]
        &[]
    }

    /// Abre la consola con `dir` como carpeta de trabajo. Si la terminal predeterminada de
    /// Windows es Windows Terminal, se abre ahí.
    pub fn open(self, owner: Handle, dir: &Path) -> bool {
        self.open_as(owner, dir, "open")
    }

    /// Igual, con un verbo del Shell: `runas` (administrador, pide confirmación de UAC) o
    /// `runasuser` (pide usuario y contraseña de otra cuenta).
    #[cfg(windows)]
    pub fn open_as(self, owner: Handle, dir: &Path, verb: &str) -> bool {
        let hwnd = owner.hwnd();
        let program = HSTRING::from(self.program().as_os_str());
        let verb = HSTRING::from(verb);
        let dir = wide_path(dir);
        let r = unsafe { ShellExecuteW(Some(hwnd), &verb, &program, PCWSTR::null(), PCWSTR(dir.as_ptr()), SW_SHOWNORMAL) };
        // ShellExecute devuelve un valor mayor que 32 si salió bien.
        r.0 as usize > 32
    }

    /// macOS: abre Terminal en la carpeta (el verbo no aplica).
    #[cfg(not(windows))]
    pub fn open_as(self, _owner: Handle, dir: &Path, _verb: &str) -> bool {
        std::process::Command::new("open").args(["-a", "Terminal"]).arg(dir).status().is_ok_and(|s| s.success())
    }
}

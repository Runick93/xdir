//! Abrir una consola (PowerShell o CMD) en la carpeta de un panel.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

use crate::fs::wide_path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Console {
    PowerShell,
    Cmd,
}

impl Console {
    pub const ALL: [Console; 2] = [Console::PowerShell, Console::Cmd];

    pub fn label(self) -> &'static str {
        match self {
            Console::PowerShell => "PowerShell",
            Console::Cmd => "CMD",
        }
    }

    /// Ejecutable: PowerShell 7 si está instalado, si no el de Windows.
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

    /// Abre la consola con `dir` como carpeta de trabajo. Si la terminal predeterminada de
    /// Windows es Windows Terminal, se abre ahí.
    pub fn open(self, hwnd: HWND, dir: &Path) -> bool {
        self.open_as(hwnd, dir, "open")
    }

    /// Igual, con un verbo del Shell: `runas` (administrador, pide confirmación de UAC) o
    /// `runasuser` (pide usuario y contraseña de otra cuenta).
    pub fn open_as(self, hwnd: HWND, dir: &Path, verb: &str) -> bool {
        let program = HSTRING::from(self.program().as_os_str());
        let verb = HSTRING::from(verb);
        let dir = wide_path(dir);
        let r = unsafe { ShellExecuteW(Some(hwnd), &verb, &program, PCWSTR::null(), PCWSTR(dir.as_ptr()), SW_SHOWNORMAL) };
        // ShellExecute devuelve un valor mayor que 32 si salió bien.
        r.0 as usize > 32
    }
}

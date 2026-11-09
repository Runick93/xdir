//! Parte de `platform` propia de Windows.

use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
use windows::core::{PCWSTR, w};

use super::{Handle, Key};

/// ¿La tecla está apretada ahora? (Ctrl, Shift, Alt…)
pub fn key_down(k: Key) -> bool {
    unsafe { GetKeyState(k.0 as i32) < 0 }
}

/// `Handle` vive en el núcleo, que no conoce `HWND`: la conversión va por este trait.
pub trait HandleExt {
    fn hwnd(self) -> HWND;
}

impl HandleExt for Handle {
    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// El `Handle` de una ventana de Windows.
pub fn handle_from_hwnd(h: HWND) -> Handle {
    Handle(h.0 as isize)
}

/// ¿Windows está en modo oscuro para las apps?
pub fn system_dark() -> bool {
    read_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(false)
}

/// Color de acento de Windows (Configuración > Personalización > Colores), 0xRRGGBB.
pub fn system_accent() -> Option<u32> {
    // AccentColor se guarda como 0xAABBGGRR.
    read_dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor"))
        .map(|abgr| (abgr & 0xFF) << 16 | (abgr & 0xFF00) | (abgr >> 16) & 0xFF)
}

/// ¿Windows permite animaciones? (por defecto sí si la consulta falla).
pub fn animations_allowed() -> bool {
    let mut on = windows::core::BOOL(1);
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as *mut _), Default::default()) };
    ok.is_err() || on.as_bool()
}

fn read_dword(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut _ as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

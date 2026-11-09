//! Parte de `platform` propia de Windows.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
use windows::core::{PCWSTR, w};

use super::{Color, Handle, Key, Rect};

pub(super) fn post_message(h: Handle, msg: u32, wparam: usize, lparam: isize) {
    unsafe {
        let _ = PostMessageW(Some(h.hwnd()), msg, WPARAM(wparam), LPARAM(lparam));
    }
}

/// ¿La tecla está apretada ahora? (Ctrl, Shift, Alt…)
pub fn key_down(k: Key) -> bool {
    unsafe { GetKeyState(k.0 as i32) < 0 }
}

impl Handle {
    pub fn hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

impl From<HWND> for Handle {
    fn from(h: HWND) -> Self {
        Handle(h.0 as isize)
    }
}

impl From<Color> for D2D1_COLOR_F {
    fn from(c: Color) -> Self {
        D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
    }
}

impl From<Rect> for D2D_RECT_F {
    fn from(r: Rect) -> Self {
        D2D_RECT_F { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
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

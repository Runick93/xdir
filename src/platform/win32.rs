//! Parte de `platform` propia de Windows.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

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

//! Parte de `platform` propia de Windows.

use windows::Win32::Foundation::HWND;

use super::Handle;

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

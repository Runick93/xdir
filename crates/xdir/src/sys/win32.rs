//! Servicios del sistema en Windows.

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use xdir_core::sys::{FileOps, Handle, Services, SystemEnv, Waker};

use crate::platform::HandleExt;

/// Despierta a la ventana con `PostMessageW` (la cola de mensajes de Windows).
struct Win32Waker;

impl Waker for Win32Waker {
    fn post(&self, window: Handle, msg: u32, wparam: usize, lparam: isize) {
        unsafe {
            let _ = PostMessageW(Some(window.hwnd()), msg, WPARAM(wparam), LPARAM(lparam));
        }
    }
}

struct Win32Env;
impl SystemEnv for Win32Env {}

struct Win32FileOps;
impl FileOps for Win32FileOps {}

/// Instala los servicios de Windows en el registro del núcleo.
pub fn install() {
    xdir_core::sys::install(Services { waker: Box::new(Win32Waker), env: Box::new(Win32Env), file_ops: Box::new(Win32FileOps) });
}

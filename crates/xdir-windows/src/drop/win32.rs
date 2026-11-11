//! Destino de arrastrar y soltar en Windows (`IDropTarget`). Cada evento se le pasa a la ventana con
//! `WM_APP_DRAG` (SendMessage: mismo hilo, llamada directa), que decide la carpeta destino y el
//! efecto. Además muestra la imagen de arrastre del Shell con `IDropTargetHelper`.

use std::cell::RefCell;
use std::path::PathBuf;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, POINTL, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
use windows::core::{Ref, Result, implement};

use crate::shell;

use super::*;

#[implement(IDropTarget)]
pub struct DropTarget {
    hwnd: usize,
    /// Imagen de arrastre del Shell. Se crea en el primer arrastre: crearla al iniciar
    /// demoraba el arranque.
    helper: RefCell<Option<IDropTargetHelper>>,
    data: RefCell<Option<IDataObject>>,
    paths: RefCell<Option<Vec<PathBuf>>>,
}

impl DropTarget {
    pub fn new(hwnd: HWND) -> Self {
        Self { hwnd: hwnd.0 as usize, helper: RefCell::new(None), data: RefCell::new(None), paths: RefCell::new(None) }
    }
}

impl DropTarget_Impl {
    fn helper(&self) -> Option<IDropTargetHelper> {
        let mut h = self.helper.borrow_mut();
        if h.is_none() {
            *h = unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() };
        }
        h.clone()
    }

    fn dispatch(&self, kind: DragKind, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> DROPEFFECT {
        let paths = self.paths.borrow();
        let allowed = if effect.is_null() { DROPEFFECT_NONE } else { unsafe { *effect } };
        let hwnd = HWND(self.hwnd as *mut _);
        // Pantalla → cliente, en DIPs.
        let mut client = POINT { x: pt.x, y: pt.y };
        let k = unsafe {
            let _ = ScreenToClient(hwnd, &mut client);
            96.0 / GetDpiForWindow(hwnd).max(1) as f32
        };
        let mut ev = DragEvent {
            kind,
            point: (client.x as f32 * k, client.y as f32 * k),
            ctrl: keys.0 & MK_CONTROL.0 != 0,
            shift: keys.0 & MK_SHIFT.0 != 0,
            paths: paths.as_deref(),
            allowed: Effect(allowed.0),
            effect: Effect::NONE,
        };
        unsafe {
            SendMessageW(hwnd, WM_APP_DRAG, Some(WPARAM(0)), Some(LPARAM(&mut ev as *mut _ as isize)));
            if !effect.is_null() {
                *effect = DROPEFFECT(ev.effect.0);
            }
        }
        DROPEFFECT(ev.effect.0)
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let data = data.cloned();
        *self.paths.borrow_mut() = data.as_ref().and_then(shell::win32::hdrop_paths);
        *self.data.borrow_mut() = data.clone();
        let result = self.dispatch(DragKind::Enter, keys, pt, effect);
        if let (Some(h), Some(d)) = (self.helper(), &data) {
            unsafe {
                let _ = h.DragEnter(HWND(self.hwnd as *mut _), d, &POINT { x: pt.x, y: pt.y }, result);
            }
        }
        Ok(())
    }

    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let result = self.dispatch(DragKind::Over, keys, pt, effect);
        if let Some(h) = self.helper.borrow().as_ref() {
            unsafe {
                let _ = h.DragOver(&POINT { x: pt.x, y: pt.y }, result);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.dispatch(DragKind::Leave, MODIFIERKEYS_FLAGS(0), &POINTL::default(), std::ptr::null_mut());
        if let Some(h) = self.helper.borrow().as_ref() {
            unsafe {
                let _ = h.DragLeave();
            }
        }
        self.data.borrow_mut().take();
        self.paths.borrow_mut().take();
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let result = self.dispatch(DragKind::Drop, keys, pt, effect);
        // Un movimiento de rutas de disco lo hace xdir (el motor de copia, en su hilo, después de
        // volver de acá): al origen se le dice «copiar» para que no borre nada por su cuenta. Con
        // «mover», un programa que no sea el Explorador puede borrar los originales mientras el
        // motor todavía los está copiando («movimiento no optimizado»).
        if !effect.is_null() {
            unsafe { *effect = performed_effect(result, self.paths.borrow().is_some()) };
        }
        if let (Some(h), Some(d)) = (self.helper.borrow().as_ref(), data.as_ref()) {
            unsafe {
                let _ = h.Drop(d, &POINT { x: pt.x, y: pt.y }, result);
            }
        }
        self.data.borrow_mut().take();
        self.paths.borrow_mut().take();
        Ok(())
    }
}

/// Lo que `Drop` le contesta al origen: un «mover» que hace xdir mismo (hay rutas de disco) se
/// informa como «copiar», así el origen nunca borra los originales.
fn performed_effect(chosen: DROPEFFECT, own_move: bool) -> DROPEFFECT {
    if own_move && chosen == DROPEFFECT_MOVE { DROPEFFECT_COPY } else { chosen }
}

pub fn register(hwnd: HWND) -> Result<IDropTarget> {
    let target: IDropTarget = DropTarget::new(hwnd).into();
    unsafe { windows::Win32::System::Ole::RegisterDragDrop(hwnd, &target)? };
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_moves_are_reported_as_copies() {
        assert_eq!(performed_effect(DROPEFFECT_MOVE, true), DROPEFFECT_COPY);
        assert_eq!(performed_effect(DROPEFFECT_COPY, true), DROPEFFECT_COPY);
        assert_eq!(performed_effect(DROPEFFECT_NONE, true), DROPEFFECT_NONE);
        // Sin rutas de disco (lo resuelve el Shell): lo que se eligió.
        assert_eq!(performed_effect(DROPEFFECT_MOVE, false), DROPEFFECT_MOVE);
    }
}

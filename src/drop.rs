//! Destino de arrastrar y soltar (`IDropTarget`). Cada evento se le pasa a la ventana con
//! `WM_APP_DRAG` (SendMessage: mismo hilo, llamada directa), que decide la carpeta destino y el
//! efecto. Además muestra la imagen de arrastre del Shell con `IDropTargetHelper`.

use std::cell::RefCell;
use std::path::PathBuf;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, POINTL, WPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, IDropTargetHelper};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_APP};
use windows::core::{Ref, Result, implement};

use crate::shell;

/// Evento de arrastre para la ventana. `lparam` apunta a un `DragEvent`.
pub const WM_APP_DRAG: u32 = WM_APP + 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DragKind {
    Enter,
    Over,
    Leave,
    Drop,
}

pub struct DragEvent<'a> {
    pub kind: DragKind,
    /// Posición en pantalla.
    pub point: POINT,
    pub keys: MODIFIERKEYS_FLAGS,
    /// Rutas arrastradas (solo archivos de disco; None si el origen no tiene rutas).
    pub paths: Option<&'a [PathBuf]>,
    /// Efectos que permite el origen.
    pub allowed: DROPEFFECT,
    /// Respuesta de la ventana.
    pub effect: DROPEFFECT,
}

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
        let mut ev = DragEvent {
            kind,
            point: POINT { x: pt.x, y: pt.y },
            keys,
            paths: paths.as_deref(),
            allowed,
            effect: DROPEFFECT_NONE,
        };
        unsafe {
            SendMessageW(HWND(self.hwnd as *mut _), WM_APP_DRAG, Some(WPARAM(0)), Some(LPARAM(&mut ev as *mut _ as isize)));
            if !effect.is_null() {
                *effect = ev.effect;
            }
        }
        ev.effect
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let data = data.cloned();
        *self.paths.borrow_mut() = data.as_ref().and_then(shell::hdrop_paths);
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

pub fn register(hwnd: HWND) -> Result<IDropTarget> {
    let target: IDropTarget = DropTarget::new(hwnd).into();
    unsafe { windows::Win32::System::Ole::RegisterDragDrop(hwnd, &target)? };
    Ok(target)
}

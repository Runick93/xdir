//! Accesibilidad (NF-8): UI Automation básica para lectores de pantalla (Narrator, NVDA, JAWS).
//!
//! El árbol (ventana → paneles → barra de dirección y lista → elementos, más la barra de estado),
//! los datos de cada elemento y las acciones están en `xdir_core::a11y`, sobre `App` como
//! `A11yView`. Acá queda el pegamento de COM: los proveedores, `VARIANT` / `SAFEARRAY`, los ids
//! de propiedades y de tipos de control, las coordenadas de pantalla y los RuntimeId.
//!
//! Los elementos de la lista soportan los patrones Selection / SelectionItem (seleccionar y leer
//! qué está seleccionado) e Invoke (abrir). Cuando cambia el elemento con foco se avisa con el
//! evento de cambio de foco, que es lo que hace que el lector lea cada elemento al moverse con
//! las flechas.
//!
//! Los proveedores se crean con `ProviderOptions_UseComThreading`: Windows los llama siempre
//! desde el hilo de la interfaz (el de la ventana, que bombea mensajes), así que pueden leer y
//! tocar `App` sin sincronizar. Los hilos de fondo nunca pasan por acá.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{GWLP_USERDATA, GetForegroundWindow, GetWindowLongPtrW};
use windows::core::{BOOL, Error, HRESULT, IUnknown, IUnknownImpl, Interface, Result, implement};

use xdir_core::a11y::{self, A11yAction, A11yView, El, Nav, Role};

use crate::platform::HandleExt;
use crate::render;
use super::App;

/// "El elemento ya no existe" (la carpeta cambió, el panel se ocultó…).
const UIA_E_ELEMENTNOTAVAILABLE: HRESULT = HRESULT(0x8004_0201_u32 as i32);

const PROVIDER_OPTIONS: ProviderOptions = ProviderOptions(ProviderOptions_ServerSideProvider.0 | ProviderOptions_UseComThreading.0);

/// Los proveedores pueden seguir vivos en un cliente después de cerrar la ventana.
static ALIVE: AtomicBool = AtomicBool::new(true);

/// Se cierra la ventana: de ahora en más los proveedores responden "no disponible".
pub fn shutdown() {
    ALIVE.store(false, Ordering::Relaxed);
}

/// Puntero a la aplicación. Solo se usa desde el hilo de la interfaz (ver arriba).
#[derive(Clone, Copy)]
struct AppPtr(*mut App);

impl AppPtr {
    fn get(&self) -> Result<&'static mut App> {
        if ALIVE.load(Ordering::Relaxed) { Ok(unsafe { &mut *self.0 }) } else { gone() }
    }
}

fn gone<T>() -> Result<T> {
    Err(Error::from_hresult(UIA_E_ELEMENTNOTAVAILABLE))
}

/// "No hay": UI Automation lo espera como S_OK con el puntero de salida en nulo.
fn none<T>() -> Result<T> {
    Err(Error::empty())
}

// ---------- datos de cada elemento ----------

fn nav(dir: NavigateDirection) -> Nav {
    if dir == NavigateDirection_Parent {
        Nav::Parent
    } else if dir == NavigateDirection_NextSibling {
        Nav::Next
    } else if dir == NavigateDirection_FirstChild {
        Nav::FirstChild
    } else if dir == NavigateDirection_LastChild {
        Nav::LastChild
    } else {
        Nav::Previous
    }
}

fn control_type(role: Role) -> UIA_CONTROLTYPE_ID {
    match role {
        Role::Pane => UIA_PaneControlTypeId,
        Role::Text => UIA_TextControlTypeId,
        Role::List => UIA_ListControlTypeId,
        Role::ListItem => UIA_ListItemControlTypeId,
        Role::StatusBar => UIA_StatusBarControlTypeId,
    }
}

fn property(app: &App, el: El, id: UIA_PROPERTY_ID) -> VARIANT {
    // La raíz es la ventana: nombre, tipo y demás los completa Windows.
    if el == El::Root {
        return VARIANT::default();
    }
    let i = a11y::info(app, el);
    if id == UIA_NamePropertyId {
        return i.name.as_deref().map(VARIANT::from).unwrap_or_default();
    }
    if id == UIA_ControlTypePropertyId {
        return i.role.map(|r| VARIANT::from(control_type(r).0)).unwrap_or_default();
    }
    if id == UIA_LocalizedControlTypePropertyId {
        return i.kind.map(VARIANT::from).unwrap_or_default();
    }
    if id == UIA_HelpTextPropertyId {
        return i.help.as_deref().map(VARIANT::from).unwrap_or_default();
    }
    if id == UIA_AutomationIdPropertyId {
        return VARIANT::from(i.id.as_str());
    }
    if id == UIA_IsControlElementPropertyId || id == UIA_IsContentElementPropertyId || id == UIA_IsEnabledPropertyId {
        return VARIANT::from(true);
    }
    if id == UIA_IsKeyboardFocusablePropertyId {
        return VARIANT::from(i.focusable);
    }
    if id == UIA_HasKeyboardFocusPropertyId {
        let focused = unsafe { GetForegroundWindow() } == app.hwnd.hwnd() && a11y::focus_el(app) == el;
        return VARIANT::from(focused);
    }
    if id == UIA_IsOffscreenPropertyId {
        return VARIANT::from(i.offscreen);
    }
    VARIANT::default()
}

// ---------- geometría ----------

/// DIPs de la ventana → píxeles de pantalla.
fn screen_rect(app: &App, r: render::Rect) -> UiaRect {
    let k = app.dpi as f64 / 96.0;
    let mut pt = POINT { x: (r.left as f64 * k).round() as i32, y: (r.top as f64 * k).round() as i32 };
    unsafe {
        let _ = ClientToScreen(app.hwnd.hwnd(), &mut pt);
    }
    UiaRect { left: pt.x as f64, top: pt.y as f64, width: (r.right - r.left) as f64 * k, height: (r.bottom - r.top) as f64 * k }
}

// ---------- SAFEARRAY ----------

/// RuntimeId: identifica al elemento entre los de la aplicación. La raíz no lleva (la da la ventana).
fn runtime_id(el: El) -> Option<Vec<i32>> {
    const APPEND: i32 = 3; // UiaAppendRuntimeId
    Some(match el {
        El::Root => return None,
        El::Pane(p) => vec![APPEND, 1, p as i32],
        El::Address(p) => vec![APPEND, 2, p as i32],
        El::List(p) => vec![APPEND, 3, p as i32],
        El::Item(p, pos) => vec![APPEND, 4, p as i32, pos as i32],
        El::Status => vec![APPEND, 5, 0],
    })
}

fn safearray_i32(values: &[i32]) -> Result<*mut SAFEARRAY> {
    unsafe {
        let sa = SafeArrayCreateVector(VT_I4, 0, values.len() as u32);
        if sa.is_null() {
            return Err(Error::from_hresult(HRESULT(0x8007_000E_u32 as i32))); // E_OUTOFMEMORY
        }
        for (i, v) in values.iter().enumerate() {
            SafeArrayPutElement(sa, &(i as i32), v as *const i32 as *const c_void)?;
        }
        Ok(sa)
    }
}

// ---------- proveedores ----------

/// La raíz: la ventana. Además de ser un fragmento, hace de raíz del árbol de fragmentos.
#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IRawElementProviderFragmentRoot)]
struct Root {
    app: AppPtr,
    el: El,
}

/// Cualquier otro elemento del árbol.
#[implement(IRawElementProviderSimple, IRawElementProviderFragment, ISelectionProvider, ISelectionItemProvider, IInvokeProvider)]
struct Elem {
    app: AppPtr,
    el: El,
}

fn fragment(app: AppPtr, el: El) -> IRawElementProviderFragment {
    if el == El::Root { Root { app, el }.into() } else { Elem { app, el }.into() }
}

/// Lo común de la parte de fragmento (navegación, identidad y geometría) de ambos proveedores.
macro_rules! fragment_impl {
    ($t:ident) => {
        impl IRawElementProviderFragment_Impl for $t {
            fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
                let app = self.app.get()?;
                if !a11y::exists(app, self.el) {
                    return gone();
                }
                match a11y::navigate(app, self.el, nav(direction)) {
                    Some(el) => Ok(fragment(self.app, el)),
                    None => none(),
                }
            }

            fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
                match runtime_id(self.el) {
                    Some(parts) => safearray_i32(&parts),
                    None => none(),
                }
            }

            fn BoundingRectangle(&self) -> Result<UiaRect> {
                let app = self.app.get()?;
                if !a11y::exists(app, self.el) {
                    return gone();
                }
                Ok(screen_rect(app, a11y::bounds(app, self.el)))
            }

            fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
                none()
            }

            fn SetFocus(&self) -> Result<()> {
                let app = self.app.get()?;
                if !a11y::exists(app, self.el) {
                    return gone();
                }
                app.a11y_apply(A11yAction::Focus(self.el));
                Ok(())
            }

            fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
                self.app.get()?;
                Ok(Root { app: self.app, el: El::Root }.into())
            }
        }
    };
}

fragment_impl!(Root_Impl);
fragment_impl!(Elem_Impl);

impl IRawElementProviderSimple_Impl for Root_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(PROVIDER_OPTIONS)
    }

    fn GetPatternProvider(&self, _id: UIA_PATTERN_ID) -> Result<IUnknown> {
        none()
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let app = self.app.get()?;
        Ok(property(app, self.el, id))
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        let app = self.app.get()?;
        unsafe { UiaHostProviderFromHwnd(app.hwnd.hwnd()) }
    }
}

impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let app = self.app.get()?;
        let mut pt = POINT { x: x.round() as i32, y: y.round() as i32 };
        unsafe {
            let _ = ScreenToClient(app.hwnd.hwnd(), &mut pt);
        }
        let k = 96.0 / app.dpi as f32;
        match a11y::element_at(app, pt.x as f32 * k, pt.y as f32 * k) {
            Some(el) => Ok(fragment(self.app, el)),
            // Nada propio ahí: que responda la ventana.
            None => none(),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let app = self.app.get()?;
        Ok(fragment(self.app, a11y::focus_el(app)))
    }
}

impl IRawElementProviderSimple_Impl for Elem_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(PROVIDER_OPTIONS)
    }

    fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> Result<IUnknown> {
        let app = self.app.get()?;
        if !a11y::exists(app, self.el) {
            return gone();
        }
        match self.el {
            El::List(_) if id == UIA_SelectionPatternId => self.to_interface::<ISelectionProvider>().cast(),
            El::Item(..) if id == UIA_SelectionItemPatternId => self.to_interface::<ISelectionItemProvider>().cast(),
            El::Item(..) if id == UIA_InvokePatternId => self.to_interface::<IInvokeProvider>().cast(),
            _ => none(),
        }
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let app = self.app.get()?;
        if !a11y::exists(app, self.el) {
            return gone();
        }
        Ok(property(app, self.el, id))
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl ISelectionProvider_Impl for Elem_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let app = self.app.get()?;
        let El::List(p) = self.el else { return gone() };
        if !a11y::exists(app, self.el) {
            return gone();
        }
        let selected = app.selected_positions(p);
        unsafe {
            let sa = SafeArrayCreateVector(VT_UNKNOWN, 0, selected.len() as u32);
            if sa.is_null() {
                return Err(Error::from_hresult(HRESULT(0x8007_000E_u32 as i32)));
            }
            for (i, pos) in selected.iter().enumerate() {
                let item: IRawElementProviderSimple = Elem { app: self.app, el: El::Item(p, *pos) }.into();
                // Para VT_UNKNOWN, `pv` es el propio puntero a la interfaz (el array le suma una referencia).
                SafeArrayPutElement(sa, &(i as i32), item.as_raw())?;
            }
            Ok(sa)
        }
    }

    fn CanSelectMultiple(&self) -> Result<BOOL> {
        Ok(true.into())
    }

    fn IsSelectionRequired(&self) -> Result<BOOL> {
        Ok(false.into())
    }
}

impl ISelectionItemProvider_Impl for Elem_Impl {
    fn Select(&self) -> Result<()> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        if !a11y::exists(app, self.el) {
            return gone();
        }
        app.a11y_apply(A11yAction::Select(p, pos));
        Ok(())
    }

    fn AddToSelection(&self) -> Result<()> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        if !a11y::exists(app, self.el) {
            return gone();
        }
        app.a11y_apply(A11yAction::AddToSelection(p, pos));
        Ok(())
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        if !a11y::exists(app, self.el) {
            return gone();
        }
        app.a11y_apply(A11yAction::RemoveFromSelection(p, pos));
        Ok(())
    }

    fn IsSelected(&self) -> Result<BOOL> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        match app.item_info(p, pos) {
            Some(it) => Ok(it.selected.into()),
            None => gone(),
        }
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        self.app.get()?;
        let El::Item(p, _) = self.el else { return gone() };
        Ok(Elem { app: self.app, el: El::List(p) }.into())
    }
}

impl IInvokeProvider_Impl for Elem_Impl {
    fn Invoke(&self) -> Result<()> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        // Como antes: alcanza con que el elemento esté en la lista (aunque el panel no se vea).
        if pos >= app.item_count(p) {
            return gone();
        }
        app.a11y_apply(A11yAction::Invoke(p, pos));
        Ok(())
    }
}

// ---------- puntos de entrada desde la app ----------

/// Respuesta a `WM_GETOBJECT` para la raíz de la ventana.
pub fn root_provider(app: &mut App) -> IRawElementProviderSimple {
    Root { app: AppPtr(app as *mut App), el: El::Root }.into()
}

/// ¿Hay algún lector de pantalla (o cliente de UI Automation) escuchando?
pub fn listening() -> bool {
    unsafe { UiaClientsAreListening() }.as_bool()
}

/// Avisa a los lectores que cambió el elemento con foco (lo llama `Win32Host`, que hace de
/// `Accessibility`). Solo si la ventana está en primer plano.
pub fn notify_focus(hwnd: HWND, el: El) {
    unsafe {
        if GetForegroundWindow() != hwnd {
            return;
        }
        // La `App` es la de la ventana (la deja `wndproc` al crearla).
        let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if app.is_null() {
            return;
        }
        let provider: IRawElementProviderSimple = Elem { app: AppPtr(app), el }.into();
        let _ = UiaRaiseAutomationEvent(&provider, UIA_AutomationFocusChangedEventId);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_unique_and_hierarchy_closes() {
        let els = [El::Pane(0), El::Pane(1), El::Address(0), El::List(0), El::List(1), El::Item(0, 0), El::Item(0, 1), El::Item(1, 0), El::Status];
        let ids: Vec<_> = els.iter().map(|&e| runtime_id(e).expect("todos menos la raíz tienen")).collect();
        for (i, a) in ids.iter().enumerate() {
            assert_eq!(a[0], 3, "empiezan con UiaAppendRuntimeId");
            for b in &ids[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert!(runtime_id(El::Root).is_none(), "la raíz la identifica la ventana");
        // Subiendo de cualquier elemento se llega a la raíz.
        for &e in &els {
            let mut cur = e;
            let mut steps = 0;
            while let Some(p) = a11y::parent(cur) {
                cur = p;
                steps += 1;
                assert!(steps < 5);
            }
            assert_eq!(cur, El::Root);
        }
        assert_eq!(a11y::pane_of(El::Item(2, 7)), Some(2));
        assert_eq!(a11y::pane_of(El::Status), None);
    }

    #[test]
    fn safearray_holds_the_runtime_id() {
        use windows::Win32::System::Ole::{SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetUBound};
        let sa = safearray_i32(&[3, 4, 1, 25]).unwrap();
        unsafe {
            assert_eq!(SafeArrayGetUBound(sa, 1).unwrap(), 3);
            let mut v = 0i32;
            SafeArrayGetElement(sa, &3, &mut v as *mut i32 as *mut c_void).unwrap();
            assert_eq!(v, 25);
            SafeArrayDestroy(sa).unwrap();
        }
    }
}

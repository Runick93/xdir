//! Accesibilidad (NF-8): UI Automation básica para lectores de pantalla (Narrator, NVDA, JAWS).
//!
//! La ventana expone este árbol:
//!
//! ```text
//! ventana (la raíz, con los datos de la ventana que da Windows)
//! ├─ Panel N                        (uno por cada panel a la vista)
//! │  ├─ barra de dirección          (texto con la ruta)
//! │  └─ lista de archivos
//! │     └─ elemento…                (nombre, tipo, tamaño y fecha; selección e invocación)
//! └─ barra de estado
//! ```
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

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::{BOOL, Error, HRESULT, IUnknown, IUnknownImpl, Interface, Result, implement};

use crate::list::Action;
use crate::render::{self, contains, rect};
use crate::{App, ui};

/// "El elemento ya no existe" (la carpeta cambió, el panel se ocultó…).
const UIA_E_ELEMENTNOTAVAILABLE: HRESULT = HRESULT(0x8004_0201_u32 as i32);

const PROVIDER_OPTIONS: ProviderOptions = ProviderOptions(ProviderOptions_ServerSideProvider.0 | ProviderOptions_UseComThreading.0);

/// Los proveedores pueden seguir vivos en un cliente después de cerrar la ventana.
static ALIVE: AtomicBool = AtomicBool::new(true);

/// Se cierra la ventana: de ahora en más los proveedores responden "no disponible".
pub fn shutdown() {
    ALIVE.store(false, Ordering::Relaxed);
}

/// Un elemento del árbol. Los paneles se identifican por su índice (0..3) y los elementos de
/// la lista por su posición entre los que se ven (después de filtrar y ordenar).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum El {
    Root,
    Pane(usize),
    Address(usize),
    List(usize),
    Item(usize, usize),
    Status,
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

// ---------- el árbol ----------

fn exists(app: &App, el: El) -> bool {
    let shown = app.shown_panes();
    match el {
        El::Root | El::Status => true,
        El::Pane(p) | El::Address(p) | El::List(p) => shown.contains(&p),
        El::Item(p, pos) => shown.contains(&p) && pos < app.panes[p].tab().list.count(),
    }
}

fn parent(el: El) -> Option<El> {
    match el {
        El::Root => None,
        El::Pane(_) | El::Status => Some(El::Root),
        El::Address(p) | El::List(p) => Some(El::Pane(p)),
        El::Item(p, _) => Some(El::List(p)),
    }
}

/// Hijos de un elemento que no sea la lista (esa tiene tantos como elementos).
fn kids(app: &App, el: El) -> Vec<El> {
    match el {
        El::Root => app.shown_panes().into_iter().map(El::Pane).chain([El::Status]).collect(),
        El::Pane(p) => vec![El::Address(p), El::List(p)],
        _ => Vec::new(),
    }
}

fn navigate(app: &App, el: El, dir: NavigateDirection) -> Option<El> {
    if dir == NavigateDirection_Parent {
        return parent(el);
    }
    if dir == NavigateDirection_FirstChild || dir == NavigateDirection_LastChild {
        let first = dir == NavigateDirection_FirstChild;
        return match el {
            El::List(p) => {
                let n = app.panes[p].tab().list.count();
                (n > 0).then(|| El::Item(p, if first { 0 } else { n - 1 }))
            }
            _ => {
                let k = kids(app, el);
                if first { k.first().copied() } else { k.last().copied() }
            }
        };
    }
    let next = dir == NavigateDirection_NextSibling;
    if let El::Item(p, pos) = el {
        let n = app.panes[p].tab().list.count();
        return if next { (pos + 1 < n).then_some(El::Item(p, pos + 1)) } else { pos.checked_sub(1).map(|q| El::Item(p, q)) };
    }
    let siblings = kids(app, parent(el)?);
    let i = siblings.iter().position(|&s| s == el)?;
    if next { siblings.get(i + 1).copied() } else { i.checked_sub(1).and_then(|j| siblings.get(j).copied()) }
}

/// Elemento con el foco del teclado: el de la lista del panel activo.
fn focus_el(app: &App) -> El {
    let p = app.active;
    match app.panes[p].tab().list.focus_pos() {
        Some(pos) if pos < app.panes[p].tab().list.count() => El::Item(p, pos),
        _ => El::List(p),
    }
}

fn pane_of(el: El) -> Option<usize> {
    match el {
        El::Pane(p) | El::Address(p) | El::List(p) | El::Item(p, _) => Some(p),
        _ => None,
    }
}

// ---------- datos de cada elemento ----------

fn pane_title(app: &App, p: usize) -> String {
    let pane = &app.panes[p];
    let title = pane.tab().location.title();
    if pane.tabs.len() > 1 {
        format!("Panel {}, pestaña {} de {}: {}", p + 1, pane.active + 1, pane.tabs.len(), title)
    } else {
        format!("Panel {}: {}", p + 1, title)
    }
}

/// Lo que se le cuenta a un lector de pantalla de un elemento.
struct Info {
    name: Option<String>,
    /// Frase extra (tipo, tamaño, fecha).
    help: Option<String>,
    control: Option<UIA_CONTROLTYPE_ID>,
    /// "carpeta", "archivo"…: cómo se llama el tipo de control cuando es más específico.
    kind: Option<&'static str>,
    id: String,
    focusable: bool,
    offscreen: bool,
}

fn info(app: &App, el: El) -> Info {
    let mut i = Info { name: None, help: None, control: None, kind: None, id: String::new(), focusable: false, offscreen: false };
    match el {
        El::Root => {}
        El::Pane(p) => {
            i.name = Some(pane_title(app, p));
            i.control = Some(UIA_PaneControlTypeId);
            i.id = format!("panel{}", p + 1);
        }
        El::Address(p) => {
            let path = match app.panes[p].tab().location.target() {
                Some(t) => t.display().to_string(),
                None => app.panes[p].tab().location.title(),
            };
            i.name = Some(format!("Ruta: {path}"));
            i.control = Some(UIA_TextControlTypeId);
            i.id = format!("panel{}.ruta", p + 1);
        }
        El::List(p) => {
            let tab = app.panes[p].tab();
            i.name = Some(format!("Archivos de {}, {} elementos", tab.location.title(), tab.list.count()));
            i.control = Some(UIA_ListControlTypeId);
            i.id = format!("panel{}.lista", p + 1);
            i.focusable = true;
        }
        El::Item(p, pos) => {
            if let Some(it) = app.panes[p].tab().list.item_info(pos) {
                i.name = Some(it.name);
                i.help = Some(it.help);
                i.kind = Some(if it.is_dir { "carpeta" } else { "archivo" });
                i.offscreen = it.offscreen;
            }
            i.control = Some(UIA_ListItemControlTypeId);
            i.id = format!("panel{}.elemento{}", p + 1, pos);
            i.focusable = true;
        }
        El::Status => {
            i.name = Some(app.panes[app.active].tab().status());
            i.control = Some(UIA_StatusBarControlTypeId);
            i.id = "estado".into();
        }
    }
    i
}

fn property(app: &App, el: El, id: UIA_PROPERTY_ID) -> VARIANT {
    // La raíz es la ventana: nombre, tipo y demás los completa Windows.
    if el == El::Root {
        return VARIANT::default();
    }
    let i = info(app, el);
    if id == UIA_NamePropertyId {
        return i.name.as_deref().map(VARIANT::from).unwrap_or_default();
    }
    if id == UIA_ControlTypePropertyId {
        return i.control.map(|c| VARIANT::from(c.0)).unwrap_or_default();
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
        let focused = unsafe { GetForegroundWindow() } == app.hwnd && focus_el(app) == el;
        return VARIANT::from(focused);
    }
    if id == UIA_IsOffscreenPropertyId {
        return VARIANT::from(i.offscreen);
    }
    VARIANT::default()
}

// ---------- geometría ----------

fn bounds(app: &App, el: El) -> render::Rect {
    let (w, h) = app.client_dips();
    match el {
        El::Root => rect(0.0, 0.0, w, h),
        El::Pane(p) => app.panes[p].rect,
        El::Address(p) => app.panes[p].address,
        El::List(p) => app.panes[p].list,
        El::Item(p, pos) => app.panes[p].tab().list.item_info(pos).map(|i| i.rect).unwrap_or_default(),
        El::Status => rect(0.0, h - ui::STATUSBAR_H, w, h),
    }
}

/// DIPs de la ventana → píxeles de pantalla.
fn screen_rect(app: &App, r: render::Rect) -> UiaRect {
    let k = app.dpi as f64 / 96.0;
    let mut pt = POINT { x: (r.left as f64 * k).round() as i32, y: (r.top as f64 * k).round() as i32 };
    unsafe {
        let _ = ClientToScreen(app.hwnd, &mut pt);
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
                if !exists(app, self.el) {
                    return gone();
                }
                match navigate(app, self.el, direction) {
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
                if !exists(app, self.el) {
                    return gone();
                }
                Ok(screen_rect(app, bounds(app, self.el)))
            }

            fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
                none()
            }

            fn SetFocus(&self) -> Result<()> {
                let app = self.app.get()?;
                if !exists(app, self.el) {
                    return gone();
                }
                if let Some(p) = pane_of(self.el) {
                    app.set_active(p);
                }
                if let El::Item(p, pos) = self.el {
                    app.panes[p].tab_mut().list.select_pos(pos);
                }
                app.redraw();
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
        unsafe { UiaHostProviderFromHwnd(app.hwnd) }
    }
}

impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let app = self.app.get()?;
        let mut pt = POINT { x: x.round() as i32, y: y.round() as i32 };
        unsafe {
            let _ = ScreenToClient(app.hwnd, &mut pt);
        }
        let k = 96.0 / app.dpi as f32;
        let (cx, cy) = (pt.x as f32 * k, pt.y as f32 * k);
        let (_, h) = app.client_dips();
        if cy >= h - ui::STATUSBAR_H {
            return Ok(fragment(self.app, El::Status));
        }
        for p in app.shown_panes() {
            let pane = &app.panes[p];
            if !contains(&pane.rect, cx, cy) {
                continue;
            }
            let el = if contains(&pane.address, cx, cy) {
                El::Address(p)
            } else if contains(&pane.list, cx, cy) {
                match pane.tab().list.pos_at_point(cx, cy) {
                    Some(pos) => El::Item(p, pos),
                    None => El::List(p),
                }
            } else {
                El::Pane(p)
            };
            return Ok(fragment(self.app, el));
        }
        // Nada propio ahí: que responda la ventana.
        none()
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let app = self.app.get()?;
        Ok(fragment(self.app, focus_el(app)))
    }
}

impl IRawElementProviderSimple_Impl for Elem_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(PROVIDER_OPTIONS)
    }

    fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> Result<IUnknown> {
        let app = self.app.get()?;
        if !exists(app, self.el) {
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
        if !exists(app, self.el) {
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
        if !exists(app, self.el) {
            return gone();
        }
        let selected = app.panes[p].tab().list.selected_positions();
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
        if !exists(app, self.el) {
            return gone();
        }
        app.set_active(p);
        app.panes[p].tab_mut().list.select_pos(pos);
        app.redraw();
        Ok(())
    }

    fn AddToSelection(&self) -> Result<()> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        if !exists(app, self.el) {
            return gone();
        }
        app.panes[p].tab_mut().list.set_selected_pos(pos, true);
        app.redraw();
        Ok(())
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        if !exists(app, self.el) {
            return gone();
        }
        app.panes[p].tab_mut().list.set_selected_pos(pos, false);
        app.redraw();
        Ok(())
    }

    fn IsSelected(&self) -> Result<BOOL> {
        let app = self.app.get()?;
        let El::Item(p, pos) = self.el else { return gone() };
        match app.panes[p].tab().list.item_info(pos) {
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
        let Some(entry) = app.panes[p].tab().list.entry_of_pos(pos) else { return gone() };
        app.set_active(p);
        app.apply(p, Action::Open(entry));
        Ok(())
    }
}

// ---------- puntos de entrada desde la app ----------

/// Respuesta a `WM_GETOBJECT` para la raíz de la ventana.
pub fn root_provider(app: &mut App) -> IRawElementProviderSimple {
    Root { app: AppPtr(app as *mut App), el: El::Root }.into()
}

/// Avisa a los lectores de pantalla cuando cambia el elemento con foco. Se llama después de
/// cada frame: es una comparación barata, y solo hace algo si hay un lector escuchando.
pub fn notify_focus(app: &mut App) {
    if !unsafe { UiaClientsAreListening() }.as_bool() {
        return;
    }
    let now = (app.active, app.panes[app.active].tab().list.focus_pos());
    if app.uia_focus == Some(now) {
        return;
    }
    app.uia_focus = Some(now);
    if unsafe { GetForegroundWindow() } != app.hwnd {
        return;
    }
    let el = focus_el(app);
    let provider: IRawElementProviderSimple = Elem { app: AppPtr(app as *mut App), el }.into();
    unsafe {
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
            while let Some(p) = parent(cur) {
                cur = p;
                steps += 1;
                assert!(steps < 5);
            }
            assert_eq!(cur, El::Root);
        }
        assert_eq!(pane_of(El::Item(2, 7)), Some(2));
        assert_eq!(pane_of(El::Status), None);
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

//! macOS: la vista rápida en el panel de QuickLook del sistema (`QLPreviewPanel`): video que se
//! reproduce, PDF con todas sus páginas, Office, etc.
//!
//! El panel recibe un solo elemento por vez (el actual): `numberOfPreviewItems` es 1 y al pasar a
//! otro se recarga, así el orden, los saltos y el final del recorrido los decide xdir
//! (`xdir_core::quicklook`). Lo que está solo en la nube (`SF_DATALESS`) no se le pasa: el panel lo
//! bajaría; xdir dibuja su tarjeta «Disponible solo en línea».
//!
//! El panel busca quién lo controla en la cadena de respondedores: la vista de xdir responde
//! `acceptsPreviewPanelControl:` mientras hay una vista rápida abierta (`app/mac.rs`) y en
//! `begin…` le pone el `Source` de acá como fuente y delegado. Las teclas que el panel no usa
//! (Espacio, Esc y flechas, si el panel tiene el foco) vuelven a `App::on_key`; si el usuario
//! cierra el panel, la `App` se entera por un mensaje (`on_quick_look_closed`), nunca anidada.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSWindowDelegate};
use objc2_foundation::{MainThreadMarker, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource, QLPreviewPanelDelegate};
use xdir_core::keys::Key;
use xdir_core::keys::keys::{VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RIGHT, VK_SPACE, VK_UP};
use xdir_core::quicklook::QuickLookItem;

thread_local! {
    /// El archivo que muestra el panel.
    static CURRENT: RefCell<Option<Retained<NSURL>>> = const { RefCell::new(None) };
    /// Hay una vista rápida de xdir en el panel (si el panel se cierra, hay que avisar).
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// Fuente y delegado del panel (vive mientras la aplicación).
    static SOURCE: RefCell<Option<Retained<Source>>> = const { RefCell::new(None) };
    /// Qué hacer con una tecla del panel (`App::on_key`) y con su cierre: los pone `app/mac.rs`.
    static HOOKS: Cell<Option<Hooks>> = const { Cell::new(None) };
}

/// Lo que este módulo necesita de la ventana: pasarle una tecla a la `App` (devuelve si se usó) y
/// avisarle que el usuario cerró el panel.
#[derive(Clone, Copy)]
pub struct Hooks {
    pub key: fn(Key) -> bool,
    pub closed: fn(),
}

/// Fija cómo llegan a la `App` las teclas y el cierre del panel.
pub fn set_hooks(hooks: Hooks) {
    HOOKS.with(|h| h.set(Some(hooks)));
}

define_class!(
    /// Fuente de datos (un elemento: el actual) y delegado del panel.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}

    unsafe impl QLPreviewPanelDataSource for Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_items(&self, _panel: Option<&QLPreviewPanel>) -> NSInteger {
            CURRENT.with(|c| NSInteger::from(c.borrow().is_some()))
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item_at(&self, _panel: Option<&QLPreviewPanel>, _index: NSInteger) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            CURRENT.with(|c| c.borrow().clone()).map(ProtocolObject::from_retained)
        }
    }

    unsafe impl NSWindowDelegate for Source {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            closed_by_user();
        }
    }

    unsafe impl QLPreviewPanelDelegate for Source {
        /// Las teclas que el panel no usa: Espacio y Esc cierran y las flechas recorren, como con
        /// la ventana de xdir activa.
        #[unsafe(method(previewPanel:handleEvent:))]
        fn handle_event(&self, _panel: Option<&QLPreviewPanel>, event: Option<&NSEvent>) -> bool {
            panel_event(event)
        }
    }
);

impl Source {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Un evento que el panel no usó: las teclas de la vista rápida (sin ⌘, ⌃ ni ⌥) van a la `App`.
fn panel_event(event: Option<&NSEvent>) -> bool {
    let Some(event) = event else { return false };
    if event.r#type() != NSEventType::KeyDown {
        return false;
    }
    let mods = event.modifierFlags();
    if mods.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option) {
        return false;
    }
    let Some(key) = panel_key(event.keyCode()) else { return false };
    HOOKS.with(|h| h.get()).is_some_and(|h| (h.key)(key))
}

/// Teclas que el panel le devuelve a xdir, por código físico.
fn panel_key(code: u16) -> Option<Key> {
    Some(match code {
        49 => VK_SPACE,
        53 => VK_ESCAPE,
        123 => VK_LEFT,
        124 => VK_RIGHT,
        125 => VK_DOWN,
        126 => VK_UP,
        _ => return None,
    })
}

/// El panel se cerró sin que xdir lo pidiera: se avisa a la `App` (una vez).
fn closed_by_user() {
    if OPEN.with(|o| o.replace(false)) {
        CURRENT.with(|c| c.borrow_mut().take());
        if let Some(h) = HOOKS.with(|h| h.get()) {
            (h.closed)();
        }
    }
}

fn shared_panel(mtm: MainThreadMarker) -> Option<Retained<QLPreviewPanel>> {
    unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }
}

/// Muestra el elemento en el panel (lo abre si estaba cerrado). `false` si no va al panel: solo
/// en la nube o sin ruta (lo dibuja xdir).
pub fn show(item: &QuickLookItem) -> bool {
    if item.cloud_only {
        return false;
    }
    let Some(path) = item.path.as_ref() else { return false };
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    CURRENT.with(|c| *c.borrow_mut() = Some(url));
    OPEN.with(|o| o.set(true));
    let Some(panel) = shared_panel(mtm) else {
        OPEN.with(|o| o.set(false));
        return false;
    };
    let ours = SOURCE.with(|s| s.borrow().as_ref().map(|s| Retained::as_ptr(s).cast::<objc2::runtime::AnyObject>()));
    let controlled = unsafe { panel.dataSource() }.is_some_and(|d| Some(Retained::as_ptr(&d).cast::<objc2::runtime::AnyObject>()) == ours);
    if panel.isVisible() && controlled {
        unsafe { panel.reloadData() };
    } else {
        // El panel busca su controlador en la cadena de respondedores de la ventana activa (la
        // vista de xdir); se muestra sin quitarle el foco a xdir, como en el Finder.
        unsafe { panel.updateController() };
        panel.orderFront(None);
        unsafe { panel.reloadData() };
    }
    true
}

/// Cierra el panel (lo pidió xdir: no se avisa a la `App`).
pub fn close() {
    OPEN.with(|o| o.set(false));
    CURRENT.with(|c| c.borrow_mut().take());
    let Some(mtm) = MainThreadMarker::new() else { return };
    if !unsafe { QLPreviewPanel::sharedPreviewPanelExists(mtm) } {
        return;
    }
    if let Some(panel) = shared_panel(mtm)
        && panel.isVisible()
    {
        panel.orderOut(None);
    }
}

/// `acceptsPreviewPanelControl:` de la vista: xdir controla el panel mientras tiene una vista
/// rápida abierta.
pub fn accepts() -> bool {
    OPEN.with(|o| o.get())
}

/// `beginPreviewPanelControl:`: el `Source` pasa a ser la fuente y el delegado del panel.
pub fn begin(panel: Option<&QLPreviewPanel>) {
    let (Some(panel), Some(mtm)) = (panel, MainThreadMarker::new()) else { return };
    let source = SOURCE.with(|s| s.borrow_mut().get_or_insert_with(|| Source::new(mtm)).clone());
    unsafe {
        panel.setDataSource(Some(ProtocolObject::from_ref(&*source)));
        panel.setDelegate(Some(&source));
        panel.reloadData();
    }
}

/// `endPreviewPanelControl:`: el panel deja de pedirle datos a xdir (se cerró, o la cadena de
/// respondedores cambió); si la vista rápida seguía abierta, la `App` se entera.
pub fn end(panel: Option<&QLPreviewPanel>) {
    if let Some(panel) = panel {
        unsafe {
            panel.setDataSource(None);
            panel.setDelegate(None);
        }
    }
    closed_by_user();
}

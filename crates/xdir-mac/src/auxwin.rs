//! Ventanas secundarias de macOS cuyo contenido dibuja el núcleo (`WindowHost::aux_*`): por ahora
//! la ventana de copia, una `NSWindow` con título, cerrar y minimizar (sin redimensionar: el alto
//! lo pone el contenido con `aux_resize`). Su vista `AuxView` dibuja con un `Renderer` propio
//! llamando a `App::paint_aux`, manda el mouse a `App::aux_mouse` y las teclas a `App::aux_key`.
//! Cerrarla la oculta (`orderOut`) y avisa `App::aux_closed`: los trabajos siguen. La ventana se
//! crea una vez y se reusa, así recuerda dónde la dejó el usuario mientras dure la sesión; la
//! primera vez va centrada sobre la ventana de xdir, sin salirse de su pantalla.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSWindowOrderingMode, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBackingStoreType, NSEvent, NSResponder,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use xdir_core::host::{AuxMouse, AuxWindow};

use crate::platform::Handle;
use crate::render::Renderer;
use crate::theme::Theme;

/// Separación de los bordes de la pantalla.
const MARGIN: f64 = 16.0;

/// Marco inicial centrado sobre `main` (la ventana principal), corrido para que entre en
/// `visible` (coordenadas de AppKit: el origen es la esquina de abajo a la izquierda). Si es más
/// alto que la pantalla, el borde de arriba queda a la vista.
pub fn initial_frame(main: NSRect, visible: NSRect, size: NSSize) -> NSRect {
    let cx = main.origin.x + main.size.width / 2.0;
    let cy = main.origin.y + main.size.height / 2.0;
    let (vr, vt) = (visible.origin.x + visible.size.width, visible.origin.y + visible.size.height);
    let x = (cx - size.width / 2.0).min(vr - size.width).max(visible.origin.x);
    let y = (cy - size.height / 2.0).max(visible.origin.y).min(vt - size.height);
    NSRect::new(NSPoint::new(x, y), size)
}

/// `frame` corrido lo justo para entrar en `visible` (una posición recordada en una pantalla que ya
/// no está, o que quedó medio afuera). Si es más alta, el borde de arriba queda a la vista.
pub fn clamp_frame(frame: NSRect, visible: NSRect) -> NSRect {
    let (vr, vt) = (visible.origin.x + visible.size.width, visible.origin.y + visible.size.height);
    let x = frame.origin.x.min(vr - frame.size.width).max(visible.origin.x);
    let y = frame.origin.y.max(visible.origin.y).min(vt - frame.size.height);
    NSRect::new(NSPoint::new(x, y), frame.size)
}

/// El marco nuevo al cambiar de alto: queda fijo el borde de arriba, salvo que el de abajo salga
/// de `visible` (entonces sube, sin pasar el de arriba de la pantalla).
pub fn resized_frame(old: NSRect, size: NSSize, visible: NSRect) -> NSRect {
    let top = old.origin.y + old.size.height;
    let floor = visible.origin.y + MARGIN;
    let mut y = top - size.height;
    if y < floor {
        y = floor.min(visible.origin.y + visible.size.height - size.height);
    }
    NSRect::new(NSPoint::new(old.origin.x, y), size)
}

struct Aux {
    window: Retained<NSWindow>,
    view: Retained<AuxView>,
    /// El delegado tiene que vivir mientras la ventana.
    _delegate: Retained<AuxDelegate>,
    main: Handle,
    theme: Theme,
}

thread_local! {
    static AUX: RefCell<Option<Aux>> = const { RefCell::new(None) };
    /// Aparte de `AUX`: dibujar no debe tener el estado prestado.
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Aux) -> R) -> Option<R> {
    AUX.with(|a| {
        let mut a = a.try_borrow_mut().ok()?;
        a.as_mut().map(f)
    })
}

define_class!(
    /// Contenido de la ventana de copia: invertida, como la vista principal.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct AuxView;

    unsafe impl NSObjectProtocol for AuxView {}

    impl AuxView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let size = self.bounds().size;
            let scale = self.window().map_or(2.0, |w| w.backingScaleFactor()) as f32;
            let Some((main, theme)) = with(|a| (a.main, a.theme)) else { return };
            RENDERER.with(|r| {
                let Ok(mut r) = r.try_borrow_mut() else { return };
                let Some(r) = r.as_mut() else { return };
                r.set_size(size.width as f32, size.height as f32, scale);
                let _ = r.draw(main, &theme, &[], |p, w, h| {
                    crate::app::with_app(|app| app.paint_aux(AuxWindow::Transfers, p, w, h));
                });
            });
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            for area in self.trackingAreas().iter() {
                self.removeTrackingArea(&area);
            }
            // También sin ser la ventana activa: se ve qué botón queda bajo el mouse.
            let options = NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), self.bounds(), options, Some(self), None)
            };
            self.addTrackingArea(&area);
            unsafe { msg_send![super(self), updateTrackingAreas] }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.mouse(event, AuxMouse::Move);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.mouse(event, AuxMouse::Move);
            self.mouse(event, AuxMouse::Down);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.mouse(event, AuxMouse::Up);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            crate::app::with_app(|app| app.aux_mouse(AuxWindow::Transfers, -1.0, -1.0, AuxMouse::Leave));
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let mut used = false;
            if let Some(key) = crate::app::translate_key(event) {
                crate::app::with_app(|app| used = app.aux_key(AuxWindow::Transfers, key));
            }
            if !used {
                unsafe { msg_send![super(self), keyDown: event] }
            }
        }
    }
);

impl AuxView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn mouse(&self, event: &NSEvent, ev: AuxMouse) {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        crate::app::with_app(|app| app.aux_mouse(AuxWindow::Transfers, p.x as f32, p.y as f32, ev));
    }
}

define_class!(
    /// Delegado de la ventana de copia: cerrarla solo la oculta.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct AuxDelegate;

    unsafe impl NSObjectProtocol for AuxDelegate {}

    unsafe impl NSWindowDelegate for AuxDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, sender: &NSWindow) -> bool {
            detach_and_hide(sender);
            crate::app::with_app(|app| app.aux_closed(AuxWindow::Transfers));
            false
        }
    }
);

impl AuxDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn apply_appearance(window: &NSWindow, dark: bool) {
    let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
    window.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
}

fn style() -> NSWindowStyleMask {
    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable
}

/// La ventana, creándola la primera vez (centrada sobre la de xdir).
fn ensure(main: Handle, theme: &Theme, size: (f32, f32)) -> Option<Retained<NSWindow>> {
    if let Some(w) = with(|a| {
        a.main = main;
        a.window.clone()
    }) {
        return Some(w);
    }
    let mtm = MainThreadMarker::new()?;
    let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.0 as f64, size.1 as f64));
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), content, style(), NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setHidesOnDeactivate(false);
    let view = AuxView::new(mtm, content);
    window.setContentView(Some(&view));
    window.makeFirstResponder(Some(&view));
    let delegate = AuxDelegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    apply_appearance(&window, theme.dark);
    let main_window = crate::app::main_window();
    let screen = main_window.as_ref().and_then(|w| w.screen()).or_else(|| window.screen());
    if let Some(screen) = screen {
        let visible = screen.visibleFrame();
        let over = main_window.map_or(visible, |w| w.frame());
        window.setFrame_display(initial_frame(over, visible, window.frame().size), false);
    }
    let renderer = Renderer::new().ok()?;
    RENDERER.with(|r| *r.borrow_mut() = Some(renderer));
    AUX.with(|a| *a.borrow_mut() = Some(Aux { window: window.clone(), view, _delegate: delegate, main, theme: *theme }));
    Some(window)
}

/// Lo de `WindowHost::aux_show`: crea la ventana o la trae al frente (sin quitarle el foco a la
/// principal), con `title` y un área de `size` puntos.
pub fn show(main: Handle, w: AuxWindow, title: &str, size: (f32, f32), theme: &Theme) -> bool {
    let AuxWindow::Transfers = w;
    let Some(window) = ensure(main, theme, size) else { return false };
    window.setTitle(&NSString::from_str(title));
    set_content_size(&window, size);
    if window.isMiniaturized() {
        window.deminiaturize(None);
    }
    let main_window = crate::app::main_window();
    if !window.isVisible() {
        // Donde quedó la vez anterior, pero entera en una pantalla que exista.
        let screen = window.screen().or_else(|| main_window.as_ref().and_then(|m| m.screen()));
        if let Some(screen) = screen {
            window.setFrame_display(clamp_frame(window.frame(), screen.visibleFrame()), false);
        }
    }
    // Hija de la principal: queda siempre encima de xdir (no se pierde detrás) y la acompaña.
    match &main_window {
        Some(m) if window.parentWindow().is_none() => unsafe { m.addChildWindow_ordered(&window, NSWindowOrderingMode::Above) },
        _ => window.orderFront(None),
    }
    invalidate(w);
    true
}

/// Se suelta de la principal y se oculta.
fn detach_and_hide(window: &NSWindow) {
    if let Some(parent) = window.parentWindow() {
        parent.removeChildWindow(window);
    }
    window.orderOut(None);
}

fn set_content_size(window: &NSWindow, size: (f32, f32)) {
    let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.0 as f64, size.1 as f64));
    let wanted = window.frameRectForContentRect(content).size;
    let old = window.frame();
    if (old.size.width - wanted.width).abs() < 0.5 && (old.size.height - wanted.height).abs() < 0.5 {
        return;
    }
    let visible = window.screen().map_or(old, |s| s.visibleFrame());
    window.setFrame_display(resized_frame(old, wanted, visible), true);
}

pub fn set_title(w: AuxWindow, title: &str) {
    let AuxWindow::Transfers = w;
    if let Some(window) = with(|a| a.window.clone()) {
        window.setTitle(&NSString::from_str(title));
    }
}

pub fn resize(w: AuxWindow, size: (f32, f32)) {
    let AuxWindow::Transfers = w;
    if let Some(window) = with(|a| a.window.clone()) {
        set_content_size(&window, size);
    }
}

pub fn invalidate(w: AuxWindow) {
    let AuxWindow::Transfers = w;
    if let Some(view) = with(|a| a.view.clone()) {
        view.setNeedsDisplay(true);
    }
}

pub fn hide(w: AuxWindow) {
    let AuxWindow::Transfers = w;
    if let Some(window) = with(|a| a.window.clone()) {
        detach_and_hide(&window);
    }
}

/// La principal se cierra: la de copia no queda sola en pantalla (y no frena la salida).
pub fn close_all() {
    hide(AuxWindow::Transfers);
}

/// Estado de la ventana de copia para las pruebas: si se ve, si es hija de la principal y su
/// lugar en el orden de las ventanas de xdir (0 = la de más arriba).
#[cfg(feature = "pruebas")]
pub fn debug_state() -> String {
    let Some(window) = with(|a| a.window.clone()) else { return "sin ventana".into() };
    let Some(mtm) = MainThreadMarker::new() else { return String::new() };
    let ordered = objc2_app_kit::NSApplication::sharedApplication(mtm).orderedWindows();
    let pos = |w: &NSWindow| ordered.iter().position(|o| std::ptr::eq(&*o, w));
    let main = crate::app::main_window();
    format!(
        "visible={} hija={} orden_copia={:?} orden_principal={:?}",
        window.isVisible(),
        window.parentWindow().is_some(),
        pos(&window),
        main.as_deref().and_then(pos)
    )
}

/// Número de la ventana de copia para el sistema (pruebas: capturar solo esa ventana).
#[cfg(feature = "pruebas")]
pub fn window_number() -> Option<isize> {
    with(|a| a.window.isVisible().then(|| a.window.windowNumber())).flatten()
}

/// El tema cambió (lo llama el host al dibujar la principal).
pub fn set_theme(theme: &Theme) {
    let changed = with(|a| {
        let t = &a.theme;
        let same = t.dark == theme.dark && t.accent == theme.accent && t.surface == theme.surface && t.text == theme.text;
        a.theme = *theme;
        (!same).then(|| (a.window.clone(), a.view.clone()))
    });
    if let Some(Some((window, view))) = changed {
        apply_appearance(&window, theme.dark);
        view.setNeedsDisplay(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_frame_centered_on_main() {
        let size = NSSize::new(560.0, 442.0);
        // AppKit: el origen es la esquina de abajo a la izquierda; el Dock deja 70 abajo.
        let visible = NSRect::new(NSPoint::new(0.0, 70.0), NSSize::new(1512.0, 845.0));
        let main = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(1280.0, 800.0));
        let f = initial_frame(main, visible, size);
        assert_eq!((f.origin.x, f.origin.y, f.size.width, f.size.height), (740.0 - 280.0, 500.0 - 221.0, 560.0, 442.0));
        // La principal corrida hasta casi salirse: la de copia entra entera en la pantalla.
        let off = NSRect::new(NSPoint::new(1300.0, -300.0), NSSize::new(800.0, 500.0));
        let f = initial_frame(off, visible, size);
        assert_eq!((f.origin.x + f.size.width, f.origin.y), (1512.0, 70.0));
        // Otra pantalla a la izquierda (x negativas).
        let left = NSRect::new(NSPoint::new(-1920.0, 0.0), NSSize::new(1920.0, 1055.0));
        let f = initial_frame(left, left, size);
        assert_eq!((f.origin.x, f.origin.y), (-960.0 - 280.0, 527.5 - 221.0));
        // Más alta que la pantalla: el borde de arriba queda a la vista.
        let small = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 400.0));
        let f = initial_frame(small, small, size);
        assert!(f.origin.y + f.size.height <= 400.0);
    }

    #[test]
    fn remembered_frame_is_brought_back_on_screen() {
        let visible = NSRect::new(NSPoint::new(0.0, 70.0), NSSize::new(1512.0, 845.0));
        let size = NSSize::new(560.0, 442.0);
        // Adentro: queda igual.
        let f = clamp_frame(NSRect::new(NSPoint::new(300.0, 200.0), size), visible);
        assert_eq!((f.origin.x, f.origin.y), (300.0, 200.0));
        // Quedó en una pantalla que ya no está (a la derecha): vuelve entera.
        let f = clamp_frame(NSRect::new(NSPoint::new(2400.0, -500.0), size), visible);
        assert_eq!((f.origin.x + f.size.width, f.origin.y), (1512.0, 70.0));
        // Arriba del todo: la barra de título queda a la vista.
        let f = clamp_frame(NSRect::new(NSPoint::new(10.0, 800.0), size), visible);
        assert_eq!(f.origin.y + f.size.height, 915.0);
    }

    #[test]
    fn resized_frame_keeps_top_and_stays_visible() {
        let visible = NSRect::new(NSPoint::new(0.0, 70.0), NSSize::new(1512.0, 845.0));
        // Crece hacia abajo con el borde de arriba fijo.
        let old = NSRect::new(NSPoint::new(900.0, 500.0), NSSize::new(560.0, 300.0));
        let f = resized_frame(old, NSSize::new(560.0, 340.0), visible);
        assert_eq!((f.origin.x, f.origin.y + f.size.height), (900.0, 800.0));
        // Abajo a la derecha: si crece, sube para no salirse.
        let old = NSRect::new(NSPoint::new(900.0, 86.0), NSSize::new(560.0, 300.0));
        let f = resized_frame(old, NSSize::new(560.0, 470.0), visible);
        assert_eq!(f.origin.y, 86.0);
    }
}

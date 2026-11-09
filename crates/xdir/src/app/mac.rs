//! macOS: arranque con `NSApplication`, la ventana y su vista (dibujo, mouse, teclado y
//! arrastrar y soltar) y el menú principal (las cajas de texto y los menús emergentes están en el
//! host, `host_mac.rs`). Es la
//! contraparte de `win.rs`: traduce los eventos de AppKit a los métodos neutros de `App`.
//!
//! Igual que en Windows, la aplicación vive detrás de un puntero que usan los eventos (que pueden
//! llegar anidados, por ejemplo mientras un menú está abierto).

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, ClassType, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply, NSBackingStoreType,
    NSCursor, NSDragOperation, NSDraggingContext, NSDraggingDestination, NSDraggingInfo, NSDraggingSession, NSDraggingSource,
    NSEvent, NSEventModifierFlags, NSMenu, NSMenuItem, NSPasteboardTypeFileURL, NSResponder, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};

use std::path::PathBuf;
use std::time::Instant;

use super::host_mac::MacHost;
use super::{App, CursorKind, INITIAL_SIZE, Init, ms_since, startup_args};
use crate::commands::Command;
use crate::drop::{DragEvent, DragKind, Effect};
use crate::platform::{Handle, Key, WM_APP, keys};
use crate::theme::Accent;
use xdir_core::host::WindowHost;
#[cfg(feature = "pruebas")]
use xdir_core::host::WM_APP_TEXT_CHANGED;

/// Mensajes propios de la ventana de macOS (por encima de los de la aplicación).
pub(super) const MSG_CLOSE: u32 = WM_APP + 90;
/// Se eligió un color en el panel de colores: `lparam` = 0xRRGGBB.
pub(super) const MSG_ACCENT: u32 = WM_APP + 91;
/// Hay un comando de prueba en la cola (feature `pruebas`).
#[cfg(feature = "pruebas")]
const MSG_TEST: u32 = WM_APP + 92;

// ---------- estado de la ventana ----------

thread_local! {
    static APP: Cell<*mut App> = const { Cell::new(std::ptr::null_mut()) };
    static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    static VIEW: RefCell<Option<Retained<XdirView>>> = const { RefCell::new(None) };
    /// Último evento de mouse (para empezar a arrastrar desde la lista).
    pub(super) static LAST_MOUSE: RefCell<Option<Retained<NSEvent>>> = const { RefCell::new(None) };
    /// Rutas que se están arrastrando sobre la ventana.
    static DRAG_PATHS: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
    static CLOSED: Cell<bool> = const { Cell::new(false) };
    /// Se está dibujando un cuadro (un pedido de repintar en ese momento AppKit lo descarta).
    pub(super) static DRAWING: Cell<bool> = const { Cell::new(false) };
    /// Objetos que tienen que vivir mientras la aplicación corre (delegados).
    pub(super) static KEEP: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
}

/// La aplicación, si ya existe (los eventos de AppKit pueden llegar antes o después).
fn with_app(f: impl FnOnce(&mut App)) {
    let ptr = APP.with(|a| a.get());
    if !ptr.is_null() {
        // SAFETY: la aplicación vive hasta que termina el proceso; los eventos llegan en el hilo
        // principal (como `wndproc` en Windows, pueden llegar anidados).
        f(unsafe { &mut *ptr });
    }
}

pub(super) fn window() -> Option<Retained<NSWindow>> {
    WINDOW.with(|w| w.borrow().clone())
}

pub(super) fn view() -> Option<Retained<XdirView>> {
    VIEW.with(|v| v.borrow().clone())
}

// ---------- arranque ----------

pub fn run() -> Result<(), String> {
    let started = Instant::now();
    // Servicios del sistema para el núcleo: antes que nada, porque la `App` ya los usa.
    crate::sys::install();
    let mtm = MainThreadMarker::new().ok_or("xdir tiene que arrancar en el hilo principal")?;
    let (session, probe, start_path) = startup_args();
    let marks = vec![("init", ms_since(started))];
    let mut init = Init::load(session, probe, start_path, started, marks);
    // El renderizador vive en el host: se crea acá (y se mide igual que en Windows).
    let host = MacHost::install()?;
    init.marks.push(("fabricas d2d/dwrite", ms_since(started)));
    let mut app = Box::new(App::new(init, host.clone()));

    let nsapp = NSApplication::sharedApplication(mtm);
    nsapp.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    nsapp.setMainMenu(Some(&main_menu(mtm)));

    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(INITIAL_SIZE.0 as f64, INITIAL_SIZE.1 as f64));
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("xdir"));
    window.setContentMinSize(NSSize::new(560.0, 360.0));
    let view = XdirView::new(mtm, frame);
    window.setContentView(Some(&view));
    window.makeFirstResponder(Some(&view));
    view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
    crate::platform::set_main_view(Retained::into_super(view.clone()));
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
    VIEW.with(|v| *v.borrow_mut() = Some(view.clone()));

    app.attach(Handle(1), 96);
    let scale = window.backingScaleFactor() as f32;
    host.set_size(INITIAL_SIZE.0 as f32, INITIAL_SIZE.1 as f32, scale);
    let ptr = Box::into_raw(app);
    APP.with(|a| a.set(ptr));
    crate::sys::set_message_handler(on_message);

    let delegate = Delegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    nsapp.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    for name in unsafe { [objc2_app_kit::NSWorkspaceDidMountNotification, objc2_app_kit::NSWorkspaceDidUnmountNotification] } {
        unsafe { center.addObserver_selector_name_object(&delegate, sel!(volumesChanged:), Some(name), None) };
    }
    KEEP.with(|k| k.borrow_mut().push(Retained::into_super(delegate)));

    with_app(|app| {
        app.apply_window_theme();
        // Las carpetas visibles empiezan a leerse antes de mostrar la ventana.
        app.sync_visibility();
        #[cfg(feature = "arbol")]
        app.start_quick_access();
        app.mark("ventana creada");
        let saved = app.take_window_state();
        host.show(saved);
    });
    #[cfg(feature = "pruebas")]
    pruebas::start();
    #[allow(deprecated)]
    nsapp.activateIgnoringOtherApps(true);
    nsapp.run();
    Ok(())
}

/// Mensajes de `Handle::post`, ya en el hilo principal.
fn on_message(msg: u32, wparam: usize, lparam: isize) {
    match msg {
        MSG_CLOSE => {
            if let Some(w) = window() {
                w.performClose(None);
            }
        }
        MSG_ACCENT => with_app(|app| app.set_accent(Accent::Rgb(lparam as u32))),
        #[cfg(feature = "pruebas")]
        MSG_TEST => pruebas::run_next(),
        _ => with_app(|app| {
            app.on_app_message(msg, wparam, lparam);
        }),
    }
}

/// Guarda la sesión una sola vez (cerrar la ventana y salir con ⌘Q llegan por caminos distintos).
fn closing() {
    if !CLOSED.with(|c| c.replace(true)) {
        with_app(|app| app.on_closing());
    }
}

/// Barra de menús: la de toda app de macOS (sin ella no andan ⌘Q ni copiar y pegar en las
/// cajas de texto). Lo demás se maneja con los atajos de xdir.
fn main_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let item = |title: &str, action: Option<Sel>, key: &str| unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), action, &NSString::from_str(key))
    };
    let submenu = |title: &str, items: Vec<Retained<NSMenuItem>>| {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        for i in items {
            menu.addItem(&i);
        }
        let top = item(title, None, "");
        top.setSubmenu(Some(&menu));
        top
    };
    let bar = NSMenu::new(mtm);
    let about = item("Acerca de xdir", Some(sel!(showAbout:)), "");
    let settings = item("Configuración…", Some(sel!(openSettings:)), ",");
    let delegate_items = [&about, &settings];
    for i in delegate_items {
        // Los atiende el delegado de la aplicación.
        unsafe { i.setTarget(None) };
    }
    bar.addItem(&submenu(
        "xdir",
        vec![
            about,
            NSMenuItem::separatorItem(mtm),
            settings,
            NSMenuItem::separatorItem(mtm),
            item("Ocultar xdir", Some(sel!(hide:)), "h"),
            item("Mostrar todo", Some(sel!(unhideAllApplications:)), ""),
            NSMenuItem::separatorItem(mtm),
            item("Salir de xdir", Some(sel!(terminate:)), "q"),
        ],
    ));
    bar.addItem(&submenu(
        "Edición",
        vec![
            item("Deshacer", Some(sel!(undo:)), "z"),
            item("Rehacer", Some(sel!(redo:)), "Z"),
            NSMenuItem::separatorItem(mtm),
            item("Cortar", Some(sel!(cut:)), "x"),
            item("Copiar", Some(sel!(copy:)), "c"),
            item("Pegar", Some(sel!(paste:)), "v"),
            item("Seleccionar todo", Some(sel!(selectAll:)), "a"),
        ],
    ));
    bar.addItem(&submenu(
        "Ventana",
        vec![item("Minimizar", Some(sel!(performMiniaturize:)), ""), item("Zoom", Some(sel!(performZoom:)), "")],
    ));
    bar
}

define_class!(
    /// Delegado de la aplicación y de la ventana.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window(&self, _app: &NSApplication) -> bool {
            true
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            closing();
            NSApplicationTerminateReply::TerminateNow
        }

        #[unsafe(method(applicationDidBecomeActive:))]
        fn did_become_active(&self, _n: &NSNotification) {
            with_app(|app| app.on_app_activated());
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            closing();
            true
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _n: &NSNotification) {
            with_app(|app| {
                app.on_resized();
                app.redraw();
            });
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing(&self, _n: &NSNotification) {
            with_app(|app| app.redraw());
        }
    }

    impl Delegate {
        /// Se conectó o se expulsó un disco: se actualiza "Este equipo", como en Windows.
        #[unsafe(method(volumesChanged:))]
        fn volumes_changed(&self, _n: &NSNotification) {
            with_app(|app| app.drives_changed());
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.run_command(Command::About));
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            with_app(|app| app.run_command(Command::OpenConfig));
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

// ---------- la vista ----------

define_class!(
    /// Vista que ocupa toda la ventana: xdir dibuja todo en ella. Invertida (y crece hacia
    /// abajo) para usar las mismas coordenadas que en Windows.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    pub(super) struct XdirView;

    unsafe impl NSObjectProtocol for XdirView {}

    impl XdirView {
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
            DRAWING.with(|d| d.set(true));
            if let Some(host) = MacHost::current() {
                host.set_size(size.width as f32, size.height as f32, scale);
            }
            with_app(|app| app.paint());
            DRAWING.with(|d| d.set(false));
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            with_app(|app| {
                if app.dark_mode().is_none() {
                    app.on_system_theme_changed();
                }
            });
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            for area in self.trackingAreas().iter() {
                self.removeTrackingArea(&area);
            }
            let options = NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveInKeyWindow
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(), self.bounds(), options, Some(self), None)
            };
            self.addTrackingArea(&area);
            unsafe { msg_send![super(self), updateTrackingAreas] }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            LAST_MOUSE.with(|m| *m.borrow_mut() = Some(event.retain()));
            let (x, y) = self.point(event);
            with_app(|app| {
                // El desplegable de la barra de dirección no le quita el foco a la caja.
                app.focus_for_click(x, y);
                app.on_left_down(x, y, event.clickCount() >= 2);
            });
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            LAST_MOUSE.with(|m| *m.borrow_mut() = Some(event.retain()));
            let (x, y) = self.point(event);
            with_app(|app| app.on_mouse_move(x, y));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let (x, y) = self.point(event);
            with_app(|app| app.on_left_up(x, y));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let (x, y) = self.point(event);
            with_app(|app| {
                app.on_mouse_move(x, y);
                #[allow(deprecated)]
                let cursor = match app.cursor_at(x, y) {
                    CursorKind::SizeWE => NSCursor::resizeLeftRightCursor(),
                    CursorKind::SizeNS => NSCursor::resizeUpDownCursor(),
                    CursorKind::Arrow => NSCursor::arrowCursor(),
                };
                cursor.set();
            });
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            with_app(|app| app.on_mouse_left());
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            let (x, y) = self.point(event);
            with_app(|app| {
                app.on_right_down(x, y);
                app.on_context_menu_request(Some((x, y)));
            });
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            let (x, y) = self.point(event);
            with_app(|app| match event.buttonNumber() {
                2 => app.on_middle_click(x, y),
                3 => app.on_back_forward(x, y, false),
                4 => app.on_back_forward(x, y, true),
                _ => {}
            });
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let (x, y) = self.point(event);
            // Touchpad: desplazamiento en puntos (una fila ≈ 24 puntos); rueda: "clics".
            let precise = event.hasPreciseScrollingDeltas();
            let (k, lines) = if precise { (1.0 / 24.0, 1.0) } else { (1.0, 3.0) };
            let (dx, dy) = (event.scrollingDeltaX() as f32 * k, event.scrollingDeltaY() as f32 * k);
            with_app(|app| {
                if dy != 0.0 {
                    app.on_wheel(x, y, dy, lines);
                }
                if dx != 0.0 {
                    app.on_wheel_h(x, y, -dx, lines);
                }
            });
        }

        #[unsafe(method(swipeWithEvent:))]
        fn swipe(&self, event: &NSEvent) {
            let (x, y) = self.point(event);
            let dx = event.deltaX();
            if dx != 0.0 {
                // Deslizar a la derecha es "atrás", como en el Finder y Safari.
                with_app(|app| app.on_back_forward(x, y, dx < 0.0));
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let flags = event.modifierFlags();
            let handled = match translate_key(event) {
                Some(key) => {
                    let mut handled = false;
                    with_app(|app| {
                        handled = if flags.contains(NSEventModifierFlags::Option) && !flags.contains(NSEventModifierFlags::Command) {
                            app.on_alt_key(key) || app.on_key(key)
                        } else {
                            app.on_key(key)
                        };
                    });
                    handled
                }
                None => false,
            };
            if handled || flags.contains(NSEventModifierFlags::Command) {
                return;
            }
            if let Some(chars) = event.characters() {
                for c in chars.to_string().chars() {
                    with_app(|app| app.on_char(c));
                }
            }
        }
    }

    unsafe impl NSDraggingSource for XdirView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy | NSDragOperation::Move | NSDragOperation::Link | NSDragOperation::Generic
        }
    }

    unsafe impl NSDraggingDestination for XdirView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            DRAG_PATHS.with(|d| *d.borrow_mut() = dragged_paths(sender));
            self.drag_event(sender, DragKind::Enter)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_event(sender, DragKind::Over)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            if let Some(sender) = sender {
                self.drag_event(sender, DragKind::Leave);
            }
            DRAG_PATHS.with(|d| d.borrow_mut().take());
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_for_drag(&self, _sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            true
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            let op = self.drag_event(sender, DragKind::Drop);
            DRAG_PATHS.with(|d| d.borrow_mut().take());
            op != NSDragOperation::None
        }
    }
);

impl XdirView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Punto del evento en coordenadas de la vista (DIPs).
    fn point(&self, event: &NSEvent) -> (f32, f32) {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        (p.x as f32, p.y as f32)
    }

    /// Pasa un evento de arrastre a la aplicación y devuelve la operación que corresponde.
    fn drag_event(&self, sender: &ProtocolObject<dyn NSDraggingInfo>, kind: DragKind) -> NSDragOperation {
        let p = self.convertPoint_fromView(sender.draggingLocation(), None);
        let mask = sender.draggingSourceOperationMask();
        let mut allowed = Effect::NONE;
        if mask.contains(NSDragOperation::Copy) {
            allowed.0 |= Effect::COPY.0;
        }
        if mask.contains(NSDragOperation::Move) || mask.contains(NSDragOperation::Generic) {
            allowed.0 |= Effect::MOVE.0;
        }
        if mask.contains(NSDragOperation::Link) {
            allowed.0 |= Effect::LINK.0;
        }
        let flags = NSEvent::modifierFlags_class();
        let mut effect = Effect::NONE;
        DRAG_PATHS.with(|d| {
            let paths = d.borrow();
            let mut ev = DragEvent {
                kind,
                point: (p.x as f32, p.y as f32),
                // ⌥ copia y ⌘ mueve, como en el Finder.
                ctrl: flags.contains(NSEventModifierFlags::Option),
                shift: flags.contains(NSEventModifierFlags::Command),
                paths: paths.as_deref(),
                allowed,
                effect: Effect::NONE,
            };
            with_app(|app| app.on_drag(&mut ev));
            effect = ev.effect;
        });
        match effect {
            Effect::COPY => NSDragOperation::Copy,
            Effect::MOVE => NSDragOperation::Move,
            Effect::LINK => NSDragOperation::Link,
            _ => NSDragOperation::None,
        }
    }
}

/// Rutas de archivo que trae un arrastre.
fn dragged_paths(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Option<Vec<PathBuf>> {
    let pb = sender.draggingPasteboard();
    let classes = NSArray::from_slice(&[NSURL::class() as &objc2::runtime::AnyClass]);
    let objects = unsafe { pb.readObjectsForClasses_options(&classes, None) }?;
    let paths: Vec<PathBuf> = objects
        .iter()
        .filter_map(|o| o.downcast::<NSURL>().ok())
        .filter(|u| u.isFileURL())
        .filter_map(|u| u.path().map(|p| PathBuf::from(p.to_string())))
        .collect();
    (!paths.is_empty()).then_some(paths)
}

/// Tecla del evento con la numeración de `platform::Key` (la de Windows): letras y números por
/// lo que escriben (respeta la distribución del teclado), el resto por su código físico.
fn translate_key(event: &NSEvent) -> Option<Key> {
    let code = event.keyCode();
    let special = match code {
        36 | 76 => Some(keys::VK_RETURN),
        48 => Some(keys::VK_TAB),
        49 => Some(keys::VK_SPACE),
        51 => Some(keys::VK_BACK),
        53 => Some(keys::VK_ESCAPE),
        117 => Some(keys::VK_DELETE),
        114 => Some(keys::VK_INSERT),
        115 => Some(keys::VK_HOME),
        119 => Some(keys::VK_END),
        116 => Some(keys::VK_PRIOR),
        121 => Some(keys::VK_NEXT),
        123 => Some(keys::VK_LEFT),
        124 => Some(keys::VK_RIGHT),
        125 => Some(keys::VK_DOWN),
        126 => Some(keys::VK_UP),
        122 => Some(Key(keys::VK_F1.0)),
        120 => Some(Key(keys::VK_F1.0 + 1)),
        99 => Some(Key(keys::VK_F1.0 + 2)),
        118 => Some(Key(keys::VK_F1.0 + 3)),
        96 => Some(Key(keys::VK_F1.0 + 4)),
        97 => Some(Key(keys::VK_F1.0 + 5)),
        98 => Some(Key(keys::VK_F1.0 + 6)),
        100 => Some(Key(keys::VK_F1.0 + 7)),
        101 => Some(Key(keys::VK_F1.0 + 8)),
        109 => Some(Key(keys::VK_F1.0 + 9)),
        103 => Some(Key(keys::VK_F1.0 + 10)),
        111 => Some(Key(keys::VK_F1.0 + 11)),
        // Signos por posición física (con ⇧ cambian de carácter): ".", ",", "[" y "]".
        47 => Some(Key(0xBE)),
        43 => Some(Key(0xBC)),
        33 => Some(Key(0xDB)),
        30 => Some(Key(0xDD)),
        _ => None,
    };
    if special.is_some() {
        return special;
    }
    let chars = event.charactersIgnoringModifiers()?.to_string();
    let mut it = chars.chars();
    let (Some(c), None) = (it.next(), it.next()) else { return None };
    c.is_ascii_alphanumeric().then(|| Key(c.to_ascii_uppercase() as u16))
}

/// Solo con `--features pruebas`: la app se maneja con comandos de texto escritos (uno por línea)
/// en el archivo de `XDIR_PRUEBAS`, igual que el gancho `WM_COPYDATA` de Windows. Además de los de
/// `test_hook`: `captura <archivo.png>` guarda la vista, `clic <x> <y>`, `doble <x> <y>`,
/// `derecho <x> <y>`, `tecla <vk>`, `escribir <texto>`, `enter`, `escape` y `salir`.
#[cfg(feature = "pruebas")]
mod pruebas {
    use super::*;
    use std::collections::VecDeque;
    use std::path::Path;

    use objc2_app_kit::NSStandardKeyBindingResponding;
    use std::sync::Mutex;

    static QUEUE: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

    pub(super) fn start() {
        let Some(file) = std::env::var_os("XDIR_PRUEBAS").map(PathBuf::from) else { return };
        std::thread::spawn(move || {
            let mut done = 0usize;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(150));
                let Ok(text) = std::fs::read_to_string(&file) else { continue };
                let lines: Vec<String> = text.lines().map(str::to_string).collect();
                for line in lines.iter().skip(done) {
                    QUEUE.lock().unwrap().push_back(line.clone());
                    Handle(1).post(MSG_TEST, 0, 0);
                }
                done = done.max(lines.len());
            }
        });
    }

    pub(super) fn run_next() {
        let Some(line) = QUEUE.lock().unwrap().pop_front() else { return };
        let mut it = line.splitn(2, ' ');
        let (verb, rest) = (it.next().unwrap_or(""), it.next().unwrap_or("").trim());
        let nums: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        match (verb, nums.as_slice()) {
            ("captura", _) => capture(Path::new(rest)),
            ("salir", _) => {
                if let Some(host) = MacHost::current() {
                    host.request_close();
                }
            }
            ("clic", [x, y]) => with_app(|app| {
                app.on_left_down(*x, *y, false);
                app.on_left_up(*x, *y);
            }),
            ("doble", [x, y]) => with_app(|app| {
                app.on_left_down(*x, *y, false);
                app.on_left_up(*x, *y);
                app.on_left_down(*x, *y, true);
                app.on_left_up(*x, *y);
            }),
            ("derecho", [x, y]) => with_app(|app| {
                app.on_right_down(*x, *y);
                app.on_context_menu_request(Some((*x, *y)));
            }),
            ("mover", [x, y]) => with_app(|app| app.on_mouse_move(*x, *y)),
            ("tecla", [vk]) => with_app(|app| {
                app.on_key(Key(*vk as u16));
            }),
            ("escribir", _) => {
                let host = MacHost::current();
                match host.as_ref().and_then(|h| h.focused_text_box().map(|b| (h, b))) {
                    Some((h, tb)) => {
                        h.text_set(tb, &(h.text_get(tb) + rest));
                        Handle(1).post(WM_APP_TEXT_CHANGED, 0, tb.0 as isize);
                    }
                    None => with_app(|app| rest.chars().for_each(|c| app.on_char(c))),
                }
            }
            // Enter o Escape en la caja de texto con foco (pasan por su delegado, como al teclear).
            ("enter" | "escape", _) => {
                let host = MacHost::current();
                if let Some(field) = host.as_ref().and_then(|h| h.focused_text_box().and_then(|b| h.text_field(b))) {
                    if let Some(editor) = field.currentEditor() {
                        let command = if verb == "enter" { sel!(insertNewline:) } else { sel!(cancelOperation:) };
                        unsafe { editor.doCommandBySelector(command) };
                    }
                }
            }
            _ => with_app(|app| app.test_hook(&line)),
        }
        with_app(|app| app.redraw());
    }

    /// Guarda lo que muestra la vista en un PNG.
    fn capture(path: &Path) {
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRepPropertyKey};
        let Some(v) = view() else { return };
        v.displayIfNeeded();
        let bounds = v.bounds();
        let Some(rep) = v.bitmapImageRepForCachingDisplayInRect(bounds) else { return };
        v.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        let props = objc2_foundation::NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
        if let Some(data) = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) } {
            let _ = std::fs::write(path, data.to_vec());
        }
    }
}

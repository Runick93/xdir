//! macOS: arranque con `NSApplication`, la ventana y su vista (dibujo, mouse, teclado y
//! arrastrar y soltar), cajas de texto (`NSTextField`), menús emergentes y diálogos. Es la
//! contraparte de `win.rs`: traduce los eventos de AppKit a los métodos neutros de `App`.
//!
//! Igual que en Windows, la aplicación vive detrás de un puntero que usan los eventos (que pueden
//! llegar anidados, por ejemplo mientras un menú está abierto).

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{AnyThread, ClassType, DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSAppearance, NSAppearanceNameAqua,
    NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationTerminateReply,
    NSBackingStoreType, NSColor, NSColorPanel, NSControl, NSControlTextEditingDelegate, NSCursor, NSDragOperation,
    NSDraggingContext, NSDraggingDestination, NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent,
    NSEventModifierFlags, NSFocusRingType, NSFont, NSMenu, NSMenuItem, NSPasteboardTypeFileURL, NSResponder, NSTextField,
    NSTextFieldDelegate, NSTextView, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL,
};

use super::*;
use crate::platform::{MenuAt, keys};

/// Mensajes propios de la ventana de macOS (por encima de los de la aplicación).
const MSG_CLOSE: u32 = WM_APP + 90;
/// Se eligió un color en el panel de colores: `lparam` = 0xRRGGBB.
const MSG_ACCENT: u32 = WM_APP + 91;
/// Hay un comando de prueba en la cola (feature `pruebas`).
#[cfg(feature = "pruebas")]
const MSG_TEST: u32 = WM_APP + 92;

// ---------- estado de la ventana ----------

thread_local! {
    static APP: Cell<*mut App> = const { Cell::new(std::ptr::null_mut()) };
    static WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    static VIEW: RefCell<Option<Retained<XdirView>>> = const { RefCell::new(None) };
    /// Último evento de mouse (para empezar a arrastrar desde la lista).
    static LAST_MOUSE: RefCell<Option<Retained<NSEvent>>> = const { RefCell::new(None) };
    /// Rutas que se están arrastrando sobre la ventana.
    static DRAG_PATHS: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
    static TEXT_BOXES: RefCell<HashMap<u32, TextBoxData>> = RefCell::new(HashMap::new());
    static NEXT_TEXT_BOX: Cell<u32> = const { Cell::new(1) };
    static CLOSED: Cell<bool> = const { Cell::new(false) };
    /// Se está dibujando un cuadro (un pedido de repintar en ese momento AppKit lo descarta).
    static DRAWING: Cell<bool> = const { Cell::new(false) };
    /// Ya hay un cuadro programado para el próximo refresco de pantalla.
    static FRAME_PENDING: Cell<bool> = const { Cell::new(false) };
    /// Objetos que tienen que vivir mientras la aplicación corre (delegados).
    static KEEP: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
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

fn window() -> Option<Retained<NSWindow>> {
    WINDOW.with(|w| w.borrow().clone())
}

fn view() -> Option<Retained<XdirView>> {
    VIEW.with(|v| v.borrow().clone())
}

/// Lo que la ventana de macOS necesita guardar además del estado de la aplicación.
pub(super) struct Sys {}

impl Sys {
    pub(super) fn new() -> Self {
        Sys {}
    }
}

// ---------- arranque ----------

pub fn run() -> Result<(), String> {
    let started = Instant::now();
    let mtm = MainThreadMarker::new().ok_or("xdir tiene que arrancar en el hilo principal")?;
    let (session, probe, start_path) = startup_args();
    let marks = vec![("init", ms_since(started))];
    let init = Init::load(session, probe, start_path, started, marks);
    let mut app = Box::new(App::new(init)?);

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

    app.hwnd = Handle(1);
    app.dpi = 96;
    let scale = window.backingScaleFactor() as f32;
    app.renderer.set_size(INITIAL_SIZE.0 as f32, INITIAL_SIZE.1 as f32, scale);
    let ptr = Box::into_raw(app);
    APP.with(|a| a.set(ptr));
    crate::platform::set_message_handler(on_message);

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
        app.marks.push(("ventana creada", ms_since(started)));
        app.show();
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
    struct XdirView;

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
            with_app(|app| {
                app.renderer.set_size(size.width as f32, size.height as f32, scale);
                app.paint();
            });
            DRAWING.with(|d| d.set(false));
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            with_app(|app| {
                if app.dark_mode.is_none() {
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
                if !matches!(app.hit(x, y), Hit::Suggest(_) | Hit::Pane(_, PaneHit::AddressClear)) {
                    focus_main(app.hwnd);
                }
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

// ---------- cajas de texto ----------

/// Para qué es una caja de edición: decide qué mensaje manda al terminar y qué teclas reenvía.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TextKind {
    /// Barra de dirección: las flechas recorren el desplegable de rutas; perder el foco cancela.
    Address,
    /// Renombrado en línea: perder el foco aplica (como en el Finder).
    Rename,
    /// Renombrado de un favorito.
    FavRename,
    /// Filtro rápido.
    Filter,
    /// Paleta: flechas, Re Pág / Av Pág, Enter y Escape van a la ventana.
    Palette,
}

impl TextKind {
    fn done_msg(self) -> u32 {
        match self {
            TextKind::Address => WM_APP_EDIT_DONE,
            TextKind::Rename => WM_APP_RENAME_DONE,
            TextKind::FavRename => WM_APP_FAVEDIT_DONE,
            TextKind::Filter => WM_APP_FILTER_DONE,
            TextKind::Palette => WM_APP_PALETTE_KEY,
        }
    }
}

struct TextBoxData {
    field: Retained<NSTextField>,
    delegate: Retained<TextDelegate>,
    /// Selección pedida antes de tener el foco (se aplica al dárselo).
    pending_select: Option<(usize, isize)>,
}

struct TextIvars {
    kind: TextKind,
    id: u32,
    /// Ya avisó que terminó (no volver a avisar al perder el foco).
    done: Cell<bool>,
}

define_class!(
    /// Delegado de una caja de texto: Enter, Escape, las flechas y perder el foco se avisan a la
    /// ventana con los mismos mensajes que en Windows.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextIvars]
    struct TextDelegate;

    unsafe impl NSObjectProtocol for TextDelegate {}

    unsafe impl NSControlTextEditingDelegate for TextDelegate {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _tv: &NSTextView, command: Sel) -> Bool {
            Bool::new(self.handle_command(command))
        }

        #[unsafe(method(controlTextDidChange:))]
        fn text_did_change(&self, _n: &NSNotification) {
            Handle(1).post(WM_APP_TEXT_CHANGED, 0, self.ivars().id as isize);
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn text_did_end_editing(&self, _n: &NSNotification) {
            let iv = self.ivars();
            if iv.done.replace(true) {
                return;
            }
            let h = Handle(1);
            match iv.kind {
                // Perder el foco: la dirección se cancela, el resto se aplica.
                TextKind::Address => h.post(WM_APP_EDIT_DONE, 0, 0),
                TextKind::Palette => h.post(WM_APP_PALETTE_KEY, 0, 0),
                kind => h.post(kind.done_msg(), 1, 0),
            }
        }
    }

    unsafe impl NSTextFieldDelegate for TextDelegate {}
);

impl TextDelegate {
    /// Enter, Escape y las flechas en la caja. Devuelve si se usó la tecla.
    fn handle_command(&self, command: Sel) -> bool {
        let iv = self.ivars();
        let h = Handle(1);
        let palette = iv.kind == TextKind::Palette;
        let cmd = NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Command) as isize;
        let nav = if command == sel!(moveUp:) {
            Some(keys::VK_UP)
        } else if command == sel!(moveDown:) {
            Some(keys::VK_DOWN)
        } else if command == sel!(scrollPageUp:) || command == sel!(pageUp:) {
            Some(keys::VK_PRIOR)
        } else if command == sel!(scrollPageDown:) || command == sel!(pageDown:) {
            Some(keys::VK_NEXT)
        } else {
            None
        };
        if let Some(vk) = nav {
            match iv.kind {
                TextKind::Address => h.post(WM_APP_SUGGEST_KEY, vk.0 as usize, 0),
                TextKind::Palette => h.post(WM_APP_PALETTE_KEY, vk.0 as usize, cmd),
                _ => return false,
            }
            return true;
        }
        let commit = if command == sel!(insertNewline:) {
            true
        } else if command == sel!(cancelOperation:) {
            false
        } else {
            return false;
        };
        if palette {
            let vk = if commit { keys::VK_RETURN } else { keys::VK_ESCAPE };
            h.post(WM_APP_PALETTE_KEY, vk.0 as usize, cmd);
        } else {
            iv.done.set(true);
            h.post(iv.kind.done_msg(), commit as usize, 0);
        }
        true
    }
}

impl TextDelegate {
    fn new(mtm: MainThreadMarker, kind: TextKind, id: u32) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TextIvars { kind, id, done: Cell::new(false) });
        unsafe { msg_send![super(this), init] }
    }
}

/// Caja de texto nativa de una línea sobre la vista (`NSTextField`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct TextBox(u32);

impl TextBox {
    /// Crea la caja sobre `r` (DIPs del área de la ventana) con `text`, sin foco ni selección.
    pub(super) fn create(app: &App, kind: TextKind, r: render::Rect, text: &str) -> Option<TextBox> {
        let mtm = MainThreadMarker::new()?;
        let view = view()?;
        let id = NEXT_TEXT_BOX.with(|n| n.replace(n.get() + 1));
        let frame = NSRect::new(NSPoint::new(r.left as f64, r.top as f64), NSSize::new((r.right - r.left) as f64, (r.bottom - r.top) as f64));
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), frame);
        field.setStringValue(&NSString::from_str(text));
        field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        let bordered = matches!(kind, TextKind::Rename | TextKind::FavRename);
        field.setBezeled(bordered);
        field.setBordered(bordered);
        field.setDrawsBackground(true);
        field.setFocusRingType(NSFocusRingType::None);
        style_field(&field, &app.theme);
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        field.setUsesSingleLineMode(true);
        let delegate = TextDelegate::new(mtm, kind, id);
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        view.addSubview(&field);
        TEXT_BOXES.with(|t| t.borrow_mut().insert(id, TextBoxData { field, delegate, pending_select: None }));
        Some(TextBox(id))
    }

    /// El control (se clona para no tener el registro prestado mientras AppKit trabaja: sus
    /// avisos pueden volver a entrar acá).
    fn field(self) -> Option<Retained<NSTextField>> {
        TEXT_BOXES.with(|t| t.borrow().get(&self.0).map(|d| d.field.clone()))
    }

    pub(super) fn id(self) -> isize {
        self.0 as isize
    }

    pub(super) fn text(self) -> String {
        self.field().map(|f| f.stringValue().to_string()).unwrap_or_default()
    }

    pub(super) fn set_text(self, text: &str) {
        if let Some(f) = self.field() {
            f.setStringValue(&NSString::from_str(text));
        }
    }

    /// Selecciona de `from` a `to` (en unidades UTF-16; `to` = -1: hasta el final).
    pub(super) fn select(self, from: usize, to: isize) {
        let Some(field) = self.field() else { return };
        match field.currentEditor() {
            Some(editor) => {
                let len = field.stringValue().length();
                let end = if to < 0 { len } else { (to as usize).min(len) };
                editor.setSelectedRange(NSRange::new(from.min(end), end - from.min(end)));
            }
            None => TEXT_BOXES.with(|t| {
                if let Some(d) = t.borrow_mut().get_mut(&self.0) {
                    d.pending_select = Some((from, to));
                }
            }),
        }
    }

    pub(super) fn focus(self) {
        let (Some(window), Some(field)) = (window(), self.field()) else { return };
        window.makeFirstResponder(Some(&field));
        let pending = TEXT_BOXES.with(|t| t.borrow_mut().get_mut(&self.0).and_then(|d| d.pending_select.take()));
        if let Some((from, to)) = pending {
            self.select(from, to);
        }
    }

    pub(super) fn has_focus(self) -> bool {
        self.field().is_some_and(|f| f.currentEditor().is_some())
    }

    pub(super) fn destroy(self) {
        if let Some(d) = TEXT_BOXES.with(|t| t.borrow_mut().remove(&self.0)) {
            // Que al perder el foco no avise de nuevo.
            d.delegate.ivars().done.set(true);
            unsafe { d.field.setDelegate(None) };
            d.field.removeFromSuperview();
        }
    }
}

fn ns_color(c: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r as f64, c.g as f64, c.b as f64, c.a as f64)
}

fn style_field(field: &NSTextField, theme: &Theme) {
    field.setBackgroundColor(Some(&ns_color(theme.surface_alt)));
    field.setTextColor(Some(&ns_color(theme.text)));
}

// ---------- menús ----------

/// Dónde abrir un menú emergente.
#[derive(Clone, Copy, Debug)]
pub(super) enum MenuPos {
    /// Esquina superior izquierda en este punto (DIPs del área de la ventana).
    At(f32, f32),
    /// Esquina inferior izquierda en este punto: el menú crece hacia arriba.
    Above(f32, f32),
    /// Junto al mouse (menú de clic derecho).
    Cursor,
    /// Junto al mouse, abierto desde otro menú todavía abierto.
    #[allow(dead_code)]
    CursorNested,
}

/// Muestra un menú y espera la elección. `dot`: el ítem con ese id lleva un punto de ese color.
pub(super) fn popup_menu(_app: &App, entries: &[shell::Entry], pos: MenuPos, dot: Option<(u32, Color)>) -> Option<u32> {
    let at = match pos {
        MenuPos::At(x, y) => MenuAt::Point(x, y),
        MenuPos::Above(x, y) => MenuAt::Above(x, y),
        MenuPos::Cursor | MenuPos::CursorNested => MenuAt::Cursor,
    };
    crate::platform::popup_menu(entries, at, dot)
}

// ---------- ventana ----------

/// Devuelve el foco del teclado a la vista principal.
pub(super) fn focus_main(_h: Handle) {
    if let (Some(w), Some(v)) = (window(), view()) {
        w.makeFirstResponder(Some(&v));
    }
}

/// Devuelve el foco a la vista solo si nadie lo tiene (se cerró la caja que lo tenía).
pub(super) fn focus_main_if_lost(h: Handle) {
    let Some(w) = window() else { return };
    let lost = match w.firstResponder() {
        None => true,
        Some(r) => r.isKindOfClass(NSWindow::class()),
    };
    if lost {
        focus_main(h);
    }
}

/// En macOS la vista recibe los arrastres aunque el mouse salga de ella.
pub(super) fn capture_mouse(_h: Handle) {}

pub(super) fn release_mouse() {}

/// Pide repintar toda la ventana. Durante el dibujo (una animación que pide el cuadro siguiente)
/// el pedido se programa para el próximo refresco de pantalla: AppKit descarta los que se hacen
/// mientras dibuja, y la animación avanzaría solo con cada evento del mouse.
pub(super) fn invalidate(_h: Handle) {
    let Some(v) = view() else { return };
    if !DRAWING.with(|d| d.get()) {
        v.setNeedsDisplay(true);
        return;
    }
    if FRAME_PENDING.with(|p| p.replace(true)) {
        return;
    }
    let fps = v.window().and_then(|w| w.screen()).map_or(60, |s| s.maximumFramesPerSecond()).clamp(30, 240);
    let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_secs_f64(1.0 / fps as f64)).unwrap_or(dispatch2::DispatchTime::NOW);
    let _ = dispatch2::DispatchQueue::main().after(when, || {
        FRAME_PENDING.with(|p| p.set(false));
        if let Some(v) = view() {
            v.setNeedsDisplay(true);
        }
    });
}

/// Tamaño del área de la ventana en DIPs (puntos).
pub(super) fn client_size(_h: Handle, _dpi: u32) -> (f32, f32) {
    view().map_or((INITIAL_SIZE.0 as f32, INITIAL_SIZE.1 as f32), |v| {
        let s = v.bounds().size;
        (s.width as f32, s.height as f32)
    })
}

/// Posición del mouse en DIPs del área de la ventana.
#[allow(dead_code)]
pub(super) fn cursor_dips(_h: Handle, _dpi: u32) -> (f32, f32) {
    let (Some(w), Some(v)) = (window(), view()) else { return (0.0, 0.0) };
    let p = v.convertPoint_fromView(w.mouseLocationOutsideOfEventStream(), None);
    (p.x as f32, p.y as f32)
}

pub(super) fn set_title(_h: Handle, title: &str) {
    if let Some(w) = window() {
        w.setTitle(&NSString::from_str(title));
    }
}

/// Pide cerrar la ventana (como el botón de cerrar).
pub(super) fn request_close(h: Handle) {
    h.post(MSG_CLOSE, 0, 0);
}

/// Sonido de "no se puede".
pub(super) fn beep() {
    objc2_app_kit::NSBeep();
}

/// Pregunta Sí / No / Cancelar. Some(true): sí; Some(false): no; None: cancelar.
pub(super) fn ask_yes_no_cancel(_h: Handle, title: &str, text: &str) -> Option<bool> {
    let mtm = MainThreadMarker::new()?;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(text));
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.addButtonWithTitle(&NSString::from_str("Sí"));
    alert.addButtonWithTitle(&NSString::from_str("No"));
    alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
    match alert.runModal() {
        r if r == NSAlertFirstButtonReturn => Some(true),
        r if r == NSAlertSecondButtonReturn => Some(false),
        _ => None,
    }
}

/// Abre un enlace en el navegador predeterminado.
pub(super) fn open_url(_h: Handle, url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

/// Abre un archivo de configuración en el editor de texto predeterminado.
pub(super) fn open_in_editor(_h: Handle, path: &std::path::Path) {
    let _ = std::process::Command::new("open").arg("-t").arg(path).spawn();
}

/// Filas por "clic" de una rueda de mouse.
#[allow(dead_code)]
pub(super) fn wheel_lines() -> f32 {
    3.0
}

/// Expande `~` y variables de entorno (`$HOME`, `${HOME}`).
pub(super) fn expand_env(s: &str) -> String {
    let mut s = s.to_string();
    if s == "~" || s.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            s = format!("{}{}", home.to_string_lossy(), &s[1..]);
        }
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (name, len) = if let Some(inner) = after.strip_prefix('{') {
            match inner.find('}') {
                Some(end) => (&inner[..end], end + 2),
                None => ("", 0),
            }
        } else {
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
            (&after[..end], end)
        };
        match std::env::var(name).ok().filter(|_| !name.is_empty()) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&rest[i..i + 1 + len]),
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}

// ---------- métodos de App propios de macOS ----------

impl App {
    /// Colores de las cajas de texto abiertas según el tema.
    pub(super) fn update_edit_resources(&mut self) {
        let theme = self.theme;
        let fields: Vec<Retained<NSTextField>> = TEXT_BOXES.with(|t| t.borrow().values().map(|d| d.field.clone()).collect());
        for f in fields {
            style_field(&f, &theme);
        }
    }

    /// Barra de título y menús al modo claro u oscuro del tema.
    pub(super) fn apply_window_theme(&mut self) {
        if let Some(w) = window() {
            let name = unsafe { if self.theme.dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
            let appearance = NSAppearance::appearanceNamed(name);
            // Siguiendo al sistema se deja la del sistema (así llegan sus cambios).
            w.setAppearance(if self.dark_mode.is_some() { appearance.as_deref() } else { None });
        }
        self.renderer.set_transparent(false);
        self.transparent = false;
        self.update_edit_resources();
    }

    /// AppKit avisa solo cuando el mouse sale (con el área de seguimiento de la vista).
    pub(super) fn track_mouse_leave(&mut self) {}

    /// Menú desplegable de una carpeta de favoritos o del desborde "»".
    pub(super) fn show_fav_menu(&mut self, pane: usize, source: FavMenuSource, anchor: render::Rect) {
        let Some(entries) = self.fav_menu_entries(&source) else { return };
        let mut ids: Vec<FavPath> = Vec::new();
        fn build(entries: &[(FavPath, Fav)], ids: &mut Vec<FavPath>) -> Vec<shell::Entry> {
            entries
                .iter()
                .map(|(path, fav)| match fav {
                    Fav::Link { .. } => {
                        ids.push(path.clone());
                        shell::Entry::Item { id: ids.len() as u32, label: fav.name().to_string(), enabled: true, checked: false }
                    }
                    Fav::Group { items, .. } => {
                        let children: Vec<(FavPath, Fav)> =
                            items.iter().enumerate().map(|(j, f)| ([path.as_slice(), &[j]].concat(), f.clone())).collect();
                        let mut sub = build(&children, ids);
                        if sub.is_empty() {
                            sub.push(shell::Entry::Item { id: 0, label: "(vacía)".into(), enabled: false, checked: false });
                        }
                        shell::Entry::Sub(fav.name().to_string(), sub)
                    }
                })
                .collect()
        }
        let menu = build(&entries, &mut ids);
        let Some(cmd) = popup_menu(self, &menu, MenuPos::At(anchor.left, anchor.bottom), None) else { return };
        if let Some(path) = ids.get(cmd as usize - 1).cloned() {
            self.activate_fav(pane, &path, key_down(VK_CONTROL));
        }
    }

    /// "Personalizado…": el panel de colores de macOS. El color se aplica al elegirlo.
    pub(super) fn choose_custom_color(&mut self) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let current = match self.accent {
            Accent::Rgb(c) => c,
            Accent::System => theme::PRESETS[0].1,
        };
        let panel = NSColorPanel::sharedColorPanel(mtm);
        let ch = |v: u32| v as f64 / 255.0;
        panel.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(ch(current >> 16), ch((current >> 8) & 0xFF), ch(current & 0xFF), 1.0));
        let target = ColorTarget::new(mtm);
        unsafe {
            panel.setTarget(Some(&target));
            panel.setAction(Some(sel!(colorChanged:)));
        }
        KEEP.with(|k| k.borrow_mut().push(Retained::into_super(target)));
        panel.orderFront(None);
    }

    /// Arrastra la selección hacia otras ventanas (o dentro de la misma).
    pub(super) fn start_drag(&mut self, pane: usize) {
        let items = self.selection_items(pane);
        self.drag = Drag::None;
        let (Some(view), Some(event), Some(mtm)) = (view(), LAST_MOUSE.with(|m| m.borrow().clone()), MainThreadMarker::new()) else { return };
        if items.is_empty() {
            return;
        }
        let at = view.convertPoint_fromView(event.locationInWindow(), None);
        let ws = NSWorkspace::sharedWorkspace();
        let dragging: Vec<Retained<NSDraggingItem>> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let path = NSString::from_str(&item.path.to_string_lossy());
                let url = NSURL::fileURLWithPath(&path);
                let drag = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
                let icon = ws.iconForFile(&path);
                let off = (i.min(4) * 4) as f64;
                let frame = NSRect::new(NSPoint::new(at.x - 16.0 + off, at.y - 16.0 + off), NSSize::new(32.0, 32.0));
                unsafe { drag.setDraggingFrame_contents(frame, Some(&icon)) };
                drag
            })
            .collect();
        let _ = mtm;
        view.beginDraggingSessionWithItems_event_source(&NSArray::from_retained_slice(&dragging), &event, ProtocolObject::from_ref(&*view));
    }

    /// Posición y tamaño de la ventana (puntos de pantalla) y si está con zoom.
    pub(super) fn window_placement(&self) -> Option<([i32; 4], bool)> {
        let w = window()?;
        let f = w.frame();
        Some(([f.origin.x as i32, f.origin.y as i32, f.size.width as i32, f.size.height as i32], w.isZoomed()))
    }

    /// Muestra la ventana con la posición guardada (o el tamaño inicial, centrada).
    pub(super) fn show(&mut self) {
        let Some(w) = window() else { return };
        match self.window_state.take() {
            Some(([x, y, width, height], zoomed)) if width > 0 && height > 0 => {
                let frame = NSRect::new(NSPoint::new(x as f64, y as f64), NSSize::new(width as f64, height as f64));
                w.setFrame_display(frame, false);
                if zoomed && !w.isZoomed() {
                    w.zoom(None);
                }
            }
            _ => w.center(),
        }
        w.makeKeyAndOrderFront(None);
    }
}

define_class!(
    /// Recibe los cambios del panel de colores.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct ColorTarget;

    unsafe impl NSObjectProtocol for ColorTarget {}

    impl ColorTarget {
        #[unsafe(method(colorChanged:))]
        fn color_changed(&self, panel: &NSColorPanel) {
            let Some(rgb) = panel.color().colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace()) else { return };
            let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
            let c = ch(rgb.redComponent()) << 16 | ch(rgb.greenComponent()) << 8 | ch(rgb.blueComponent());
            Handle(1).post(MSG_ACCENT, 0, c as isize);
        }
    }
);

impl ColorTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
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
            ("salir", _) => request_close(Handle(1)),
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
                let ids: Vec<u32> = TEXT_BOXES.with(|t| t.borrow().keys().copied().collect());
                let focused = ids.into_iter().find(|&id| TextBox(id).has_focus());
                match focused {
                    Some(id) => {
                        let tb = TextBox(id);
                        tb.set_text(&(tb.text() + rest));
                        Handle(1).post(WM_APP_TEXT_CHANGED, 0, id as isize);
                    }
                    None => with_app(|app| rest.chars().for_each(|c| app.on_char(c))),
                }
            }
            // Enter o Escape en la caja de texto con foco (pasan por su delegado, como al teclear).
            ("enter" | "escape", _) => {
                let ids: Vec<u32> = TEXT_BOXES.with(|t| t.borrow().keys().copied().collect());
                if let Some(field) = ids.into_iter().find(|&id| TextBox(id).has_focus()).and_then(|id| TextBox(id).field()) {
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

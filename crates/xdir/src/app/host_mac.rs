//! macOS: la ventana de la aplicación (`WindowHost`) sobre la `NSWindow` y la vista de `mac.rs`.
//! Es la contraparte de `host_win.rs`. La vista trabaja en puntos, que ya son DIPs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, ClassType, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSAppearance, NSAppearanceCustomization,
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor, NSColorPanel, NSDraggingItem, NSWindow, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use xdir_core::host::WindowHost;
use xdir_core::paint::Painter;

use crate::config::ColorRule;
use crate::render::Renderer;

use super::*;
use super::mac::{DRAWING, KEEP, LAST_MOUSE, MSG_ACCENT, MSG_CLOSE, view, window};

thread_local! {
    /// Ya hay un cuadro programado para el próximo refresco de pantalla.
    static FRAME_PENDING: Cell<bool> = const { Cell::new(false) };
    /// El host de la ventana, para el código de AppKit que necesita el tipo concreto (`drawRect:`
    /// le pasa el tamaño de la vista al renderizador): la `App` lo tiene como `dyn WindowHost`.
    static HOST: RefCell<Option<Rc<MacHost>>> = const { RefCell::new(None) };
}

/// La ventana principal de macOS. La ventana y la vista viven en los `thread_local!` de `mac.rs`.
pub(super) struct MacHost {
    window: Cell<Handle>,
    /// Dibujo con Core Graphics (fuentes, imágenes y tamaño del área).
    renderer: RefCell<Renderer>,
}

impl MacHost {
    /// Crea el host (todavía sin ventana) con su renderizador y lo deja en el `thread_local`.
    pub(super) fn install() -> Result<Rc<MacHost>, String> {
        let host = Rc::new(MacHost { window: Cell::new(Handle::default()), renderer: RefCell::new(Renderer::new()?) });
        HOST.with(|h| *h.borrow_mut() = Some(host.clone()));
        Ok(host)
    }

    /// El host instalado con `install`.
    pub(super) fn current() -> Option<Rc<MacHost>> {
        HOST.with(|h| h.borrow().clone())
    }

    /// Tamaño del área en puntos y píxeles por punto de la pantalla (lo fija la vista antes de
    /// dibujar).
    pub(super) fn set_size(&self, width: f32, height: f32, scale: f32) {
        self.renderer.borrow_mut().set_size(width, height, scale);
    }
}

impl WindowHost for MacHost {
    fn attach(&self, window: Handle) {
        self.window.set(window);
    }

    /// Dibuja en el contexto actual de AppKit (dentro de `drawRect:`).
    fn draw(&self, theme: &Theme, file_colors: &[ColorRule], paint: &mut dyn FnMut(&dyn Painter, f32, f32)) -> Result<(), String> {
        self.renderer.borrow_mut().draw(self.window.get(), theme, file_colors, |p, w, h| paint(p, w, h))
    }

    fn text_width(&self, s: &str, bold: bool) -> f32 {
        self.renderer.borrow().text_width(s, bold)
    }

    /// Durante el dibujo (una animación que pide el cuadro siguiente) el pedido se programa para
    /// el próximo refresco de pantalla: AppKit descarta los que se hacen mientras dibuja, y la
    /// animación avanzaría solo con cada evento del mouse.
    fn invalidate(&self) {
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

    fn client_size(&self) -> (f32, f32) {
        view().map_or((INITIAL_SIZE.0 as f32, INITIAL_SIZE.1 as f32), |v| {
            let s = v.bounds().size;
            (s.width as f32, s.height as f32)
        })
    }

    fn cursor_dips(&self) -> (f32, f32) {
        let (Some(w), Some(v)) = (window(), view()) else { return (0.0, 0.0) };
        let p = v.convertPoint_fromView(w.mouseLocationOutsideOfEventStream(), None);
        (p.x as f32, p.y as f32)
    }

    fn set_title(&self, title: &str) {
        if let Some(w) = window() {
            w.setTitle(&NSString::from_str(title));
        }
    }

    fn request_close(&self) {
        self.window.get().post(MSG_CLOSE, 0, 0);
    }

    fn focus_main(&self) {
        if let (Some(w), Some(v)) = (window(), view()) {
            w.makeFirstResponder(Some(&v));
        }
    }

    fn focus_main_if_lost(&self) {
        let Some(w) = window() else { return };
        let lost = match w.firstResponder() {
            None => true,
            Some(r) => r.isKindOfClass(NSWindow::class()),
        };
        if lost {
            self.focus_main();
        }
    }

    /// La vista recibe los arrastres aunque el mouse salga de ella.
    fn capture_mouse(&self) {}

    fn release_mouse(&self) {}

    /// AppKit avisa solo cuando el mouse sale (con el área de seguimiento de la vista).
    fn track_mouse_leave(&self) {}

    fn beep(&self) {
        objc2_app_kit::NSBeep();
    }

    fn ask_yes_no_cancel(&self, title: &str, text: &str) -> Option<bool> {
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

    fn open_url(&self, url: &str) {
        if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
            NSWorkspace::sharedWorkspace().openURL(&url);
        }
    }

    /// En el editor de texto predeterminado.
    fn open_in_editor(&self, path: &std::path::Path) {
        let _ = std::process::Command::new("open").arg("-t").arg(path).spawn();
    }

    /// Filas por "clic" de una rueda de mouse.
    fn wheel_lines(&self) -> f32 {
        3.0
    }

    /// Expande `~` y variables de entorno (`$HOME`, `${HOME}`).
    fn expand_env(&self, s: &str) -> String {
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

    /// Barra de título y menús al modo claro u oscuro del tema. Siguiendo al sistema se deja la
    /// apariencia del sistema (así llegan sus cambios). Sin fondo translúcido.
    fn apply_theme(&self, theme: &Theme, follow_system: bool) -> bool {
        if let Some(w) = window() {
            let name = unsafe { if theme.dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
            let appearance = NSAppearance::appearanceNamed(name);
            w.setAppearance(if follow_system { None } else { appearance.as_deref() });
        }
        false
    }

    /// El panel de colores de macOS: no es modal, cada color elegido llega después por
    /// `MSG_ACCENT`. Por eso devuelve siempre `None`.
    fn choose_color(&self, current: u32) -> Option<u32> {
        let mtm = MainThreadMarker::new()?;
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
        None
    }

    /// Sesión de arrastre de AppKit desde el último evento del mouse (no es modal).
    fn start_drag(&self, items: &[shell::Item]) -> Result<(), String> {
        let (Some(view), Some(event)) = (view(), LAST_MOUSE.with(|m| m.borrow().clone())) else { return Ok(()) };
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
        view.beginDraggingSessionWithItems_event_source(&NSArray::from_retained_slice(&dragging), &event, ProtocolObject::from_ref(&*view));
        Ok(())
    }

    /// Posición y tamaño de la ventana (puntos de pantalla) y si está con zoom.
    fn placement(&self) -> Option<([i32; 4], bool)> {
        let w = window()?;
        let f = w.frame();
        Some(([f.origin.x as i32, f.origin.y as i32, f.size.width as i32, f.size.height as i32], w.isZoomed()))
    }

    /// Con la posición guardada, o el tamaño inicial centrada.
    fn show(&self, saved: Option<([i32; 4], bool)>) {
        let Some(w) = window() else { return };
        match saved {
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

//! macOS: la ventana de la aplicación (`WindowHost`) sobre la `NSWindow` y la vista de `mac.rs`.
//! Es la contraparte de `host_win.rs`. La vista trabaja en puntos, que ya son DIPs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject, Sel};
use objc2::{AnyThread, ClassType, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertStyle, NSAppearance, NSAppearanceCustomization,
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor, NSColorPanel, NSControl, NSControlTextEditingDelegate,
    NSDraggingItem, NSEvent, NSEventModifierFlags, NSFocusRingType, NSFont, NSTextField, NSTextFieldDelegate, NSTextView,
    NSWindow, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL,
};
use xdir_core::host::WindowHost;
use xdir_core::paint::Painter;

use crate::config::ColorRule;
use crate::render::Renderer;

use std::collections::HashMap;

use super::INITIAL_SIZE;
use crate::platform::{Color, Handle};
use crate::theme::Theme;
use crate::{render, shell};
use xdir_core::host::{MenuPos, TextBox, TextKind, WM_APP_EDIT_DONE, WM_APP_PALETTE_KEY, WM_APP_SUGGEST_KEY, WM_APP_TEXT_CHANGED};
use super::mac::{DRAWING, KEEP, LAST_MOUSE, MSG_ACCENT, MSG_CLOSE, view, window};
use crate::platform::{MenuAt, keys};

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
    /// Cajas de texto abiertas, por número. No se prestan mientras AppKit trabaja con un control:
    /// sus avisos pueden volver a entrar acá.
    text_boxes: RefCell<HashMap<u32, TextBoxData>>,
    next_text_box: Cell<u32>,
}

impl MacHost {
    /// Crea el host (todavía sin ventana) con su renderizador y lo deja en el `thread_local`.
    pub(super) fn install() -> Result<Rc<MacHost>, String> {
        let host = Rc::new(MacHost {
            window: Cell::new(Handle::default()),
            renderer: RefCell::new(Renderer::new()?),
            text_boxes: RefCell::new(HashMap::new()),
            next_text_box: Cell::new(1),
        });
        HOST.with(|h| *h.borrow_mut() = Some(host.clone()));
        Ok(host)
    }

    /// El host instalado con `install`.
    pub(super) fn current() -> Option<Rc<MacHost>> {
        HOST.with(|h| h.borrow().clone())
    }

    /// Pasa a la caché las imágenes grandes que terminaron (al llegar `WM_APP_ICONS`).
    pub(super) fn absorb_big(&self) {
        if let Ok(r) = self.renderer.try_borrow() {
            r.absorb_big();
        }
    }

    /// Tamaño del área en puntos y píxeles por punto de la pantalla (lo fija la vista antes de
    /// dibujar).
    pub(super) fn set_size(&self, width: f32, height: f32, scale: f32) {
        self.renderer.borrow_mut().set_size(width, height, scale);
    }

    /// El control de una caja (se clona para no tener el registro prestado mientras AppKit
    /// trabaja: sus avisos pueden volver a entrar acá).
    pub(super) fn text_field(&self, b: TextBox) -> Option<Retained<NSTextField>> {
        self.text_boxes.borrow().get(&b.0).map(|d| d.field.clone())
    }

    /// La caja que tiene el foco (comandos de prueba).
    #[cfg(feature = "pruebas")]
    pub(super) fn focused_text_box(&self) -> Option<TextBox> {
        let ids: Vec<u32> = self.text_boxes.borrow().keys().copied().collect();
        ids.into_iter().map(TextBox).find(|&b| self.text_has_focus(b))
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

    /// Caja de texto nativa de una línea sobre la vista (`NSTextField`).
    fn text_create(&self, kind: TextKind, r: render::Rect, text: &str, theme: &Theme) -> Option<TextBox> {
        let mtm = MainThreadMarker::new()?;
        let view = view()?;
        let id = self.next_text_box.replace(self.next_text_box.get() + 1);
        let frame = NSRect::new(NSPoint::new(r.left as f64, r.top as f64), NSSize::new((r.right - r.left) as f64, (r.bottom - r.top) as f64));
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), frame);
        field.setStringValue(&NSString::from_str(text));
        field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        let bordered = matches!(kind, TextKind::Rename | TextKind::FavRename);
        field.setBezeled(bordered);
        field.setBordered(bordered);
        field.setDrawsBackground(true);
        field.setFocusRingType(NSFocusRingType::None);
        style_field(&field, theme);
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        field.setUsesSingleLineMode(true);
        let delegate = TextDelegate::new(mtm, kind, id);
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        view.addSubview(&field);
        self.text_boxes.borrow_mut().insert(id, TextBoxData { field, delegate, pending_select: None });
        Some(TextBox(id))
    }

    fn text_get(&self, b: TextBox) -> String {
        self.text_field(b).map(|f| f.stringValue().to_string()).unwrap_or_default()
    }

    fn text_set(&self, b: TextBox, text: &str) {
        if let Some(f) = self.text_field(b) {
            f.setStringValue(&NSString::from_str(text));
        }
    }

    /// Sin foco todavía no hay editor: la selección se guarda y se aplica al dárselo.
    fn text_select(&self, b: TextBox, from: usize, to: isize) {
        let Some(field) = self.text_field(b) else { return };
        match field.currentEditor() {
            Some(editor) => {
                let len = field.stringValue().length();
                let end = if to < 0 { len } else { (to as usize).min(len) };
                editor.setSelectedRange(NSRange::new(from.min(end), end - from.min(end)));
            }
            None => {
                if let Some(d) = self.text_boxes.borrow_mut().get_mut(&b.0) {
                    d.pending_select = Some((from, to));
                }
            }
        }
    }

    fn text_focus(&self, b: TextBox) {
        let (Some(window), Some(field)) = (window(), self.text_field(b)) else { return };
        window.makeFirstResponder(Some(&field));
        let pending = self.text_boxes.borrow_mut().get_mut(&b.0).and_then(|d| d.pending_select.take());
        if let Some((from, to)) = pending {
            self.text_select(b, from, to);
        }
    }

    fn text_has_focus(&self, b: TextBox) -> bool {
        self.text_field(b).is_some_and(|f| f.currentEditor().is_some())
    }

    fn text_destroy(&self, b: TextBox) {
        let removed = self.text_boxes.borrow_mut().remove(&b.0);
        if let Some(d) = removed {
            // Que al perder el foco no avise de nuevo.
            d.delegate.ivars().done.set(true);
            unsafe { d.field.setDelegate(None) };
            d.field.removeFromSuperview();
        }
    }

    /// Colores de las cajas de texto abiertas según el tema (la fuente no depende de la escala).
    fn text_restyle(&self, theme: &Theme) {
        let fields: Vec<Retained<NSTextField>> = self.text_boxes.borrow().values().map(|d| d.field.clone()).collect();
        for f in fields {
            style_field(&f, theme);
        }
    }

    /// Menú nativo de AppKit. El clic derecho dentro del menú no llega: `on_context` no se usa.
    fn popup_menu(&self, entries: &[shell::Entry], pos: MenuPos, dot: Option<(u32, Color)>, _on_context: Option<xdir_core::host::OnContext<'_>>) -> Option<u32> {
        let at = match pos {
            MenuPos::At(x, y) => MenuAt::Point(x, y),
            MenuPos::Above(x, y) => MenuAt::Above(x, y),
            MenuPos::Cursor | MenuPos::CursorNested => MenuAt::Cursor,
        };
        crate::platform::popup_menu(entries, at, dot)
    }

    /// `QLPreviewPanel` del sistema (lo solo en la nube no: lo dibuja la `App`).
    fn quick_look_show(&self, item: &xdir_core::quicklook::QuickLookItem) -> bool {
        // Pruebas: como un sistema sin visor (la tarjeta de xdir, la de Linux y Windows).
        #[cfg(feature = "pruebas")]
        if std::env::var_os("XDIR_PRUEBAS_SIN_VISOR").is_some() {
            return false;
        }
        crate::quicklook::show(item)
    }

    fn quick_look_close(&self) {
        crate::quicklook::close();
    }
}

// ---------- cajas de texto ----------

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
                // Un campo de panel sigue abierto: no avisa.
                TextKind::Field => {}
                kind => h.post(kind.done_msg(), 1, 0),
            }
        }
    }

    unsafe impl NSTextFieldDelegate for TextDelegate {}
);

impl TextDelegate {
    fn new(mtm: MainThreadMarker, kind: TextKind, id: u32) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TextIvars { kind, id, done: Cell::new(false) });
        unsafe { msg_send![super(this), init] }
    }

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
        // Tab y Shift+Tab en un campo de panel: el núcleo pasa el foco a la caja siguiente o a la
        // anterior (las cajas se crean y destruyen al cambiar el modo: no hay cadena fija de
        // `nextKeyView`).
        if iv.kind == TextKind::Field && (command == sel!(insertTab:) || command == sel!(insertBacktab:)) {
            h.post(xdir_core::host::WM_APP_FIELD_TAB, (command == sel!(insertBacktab:)) as usize, 0);
            return true;
        }
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

fn ns_color(c: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r as f64, c.g as f64, c.b as f64, c.a as f64)
}

fn style_field(field: &NSTextField, theme: &Theme) {
    field.setBackgroundColor(Some(&ns_color(theme.surface_alt)));
    field.setTextColor(Some(&ns_color(theme.text)));
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

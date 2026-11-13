//! Parte de `platform` propia de macOS.

use objc2_app_kit::NSColor;
use objc2_foundation::NSString;

// ---------- vista principal y menús emergentes ----------

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSImage, NSImageSymbolConfiguration, NSMenu, NSMenuItem, NSView};
use objc2_foundation::{MainThreadMarker, NSArray, NSObjectProtocol, NSPoint};

use super::Color;
use crate::shell::Entry;

thread_local! {
    static MAIN_VIEW: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
}

/// Registra la vista de la ventana principal (sobre ella se abren los menús).
pub fn set_main_view(view: Retained<NSView>) {
    MAIN_VIEW.with(|v| *v.borrow_mut() = Some(view));
}

pub fn main_view() -> Option<Retained<NSView>> {
    MAIN_VIEW.with(|v| v.borrow().clone())
}

define_class!(
    /// Recibe la elección de un menú emergente: guarda la etiqueta del ítem.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<isize>]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSMenuItem) {
            self.ivars().set(sender.tag());
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(0));
        unsafe { msg_send![super(this), init] }
    }
}

/// Dónde abrir un menú emergente (DIPs del área de la ventana).
#[derive(Clone, Copy, Debug)]
pub enum MenuAt {
    /// Esquina superior izquierda en este punto.
    Point(f32, f32),
    /// Esquina inferior izquierda en este punto (crece hacia arriba).
    Above(f32, f32),
    /// Junto al mouse.
    Cursor,
}

/// Punto de color para marcar un ítem.
fn dot_image(c: Color) -> Option<Retained<NSImage>> {
    let base = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("circle.fill"), None)?;
    let sized = NSImageSymbolConfiguration::configurationWithPointSize_weight(8.0, unsafe { objc2_app_kit::NSFontWeightRegular });
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
    let palette = NSImageSymbolConfiguration::configurationWithPaletteColors(&NSArray::from_retained_slice(&[color]));
    base.imageWithSymbolConfiguration(&sized.configurationByApplyingConfiguration(&palette))
}

/// Equivalente de teclado de un ítem de menú.
#[derive(Debug, PartialEq, Eq)]
struct KeyEquivalent {
    key: String,
    cmd: bool,
    shift: bool,
    alt: bool,
    ctrl: bool,
}

/// Atajo como lo muestra xdir en la Mac («⇧⌘N», «⌥⌘N») → equivalente de teclado. Solo una letra o
/// un número después de los modificadores; lo demás (F5, flechas) va como texto (`title_and_key`).
fn key_equivalent(text: &str) -> Option<KeyEquivalent> {
    let mut k = KeyEquivalent { key: String::new(), cmd: false, shift: false, alt: false, ctrl: false };
    let mut rest = text.trim();
    while let Some(c) = rest.chars().next() {
        match c {
            '⌘' => k.cmd = true,
            '⇧' => k.shift = true,
            '⌥' => k.alt = true,
            '⌃' => k.ctrl = true,
            _ => break,
        }
        rest = &rest[c.len_utf8()..];
    }
    let mut chars = rest.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => {
            k.key = c.to_ascii_lowercase().to_string();
            Some(k)
        }
        _ => None,
    }
}

/// Título y equivalente de teclado de una etiqueta «Título\tatajo». Si el atajo no se puede
/// poner como equivalente (F5, flechas), queda escrito en el título.
fn title_and_key(label: &str) -> (String, Option<KeyEquivalent>) {
    match label.split_once('\t') {
        Some((title, keys)) => match key_equivalent(keys) {
            Some(k) => (title.to_string(), Some(k)),
            None => (format!("{title}    {}", keys.trim()), None),
        },
        None => (label.to_string(), None),
    }
}

fn build_menu(entries: &[Entry], target: &MenuTarget, dot: Option<(u32, Color)>, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for e in entries {
        match e {
            Entry::Sep => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Entry::Item { id, label, enabled, checked } => {
                // «Carpeta nueva\t⇧⌘N»: el atajo va a la derecha como equivalente de teclado.
                let (title, key) = title_and_key(label);
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(&title), Some(sel!(pick:)), &NSString::new())
                };
                if let Some(k) = key {
                    item.setKeyEquivalent(&NSString::from_str(&k.key));
                    let mut mask = NSEventModifierFlags::empty();
                    for (on, flag) in [(k.cmd, NSEventModifierFlags::Command), (k.shift, NSEventModifierFlags::Shift), (k.alt, NSEventModifierFlags::Option), (k.ctrl, NSEventModifierFlags::Control)] {
                        if on {
                            mask |= flag;
                        }
                    }
                    item.setKeyEquivalentModifierMask(mask);
                }
                unsafe { item.setTarget(Some(target)) };
                item.setTag(*id as isize);
                item.setEnabled(*enabled);
                item.setState(if *checked { NSControlStateValueOn } else { NSControlStateValueOff });
                if let Some((_, color)) = dot.filter(|(d, _)| d == id) {
                    item.setImage(dot_image(color).as_deref());
                }
                menu.addItem(&item);
            }
            Entry::Sub(label, children) => {
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(label), None, &NSString::new())
                };
                let sub = build_menu(children, target, dot, mtm);
                sub.setTitle(&NSString::from_str(label));
                item.setSubmenu(Some(&sub));
                menu.addItem(&item);
            }
        }
    }
    menu
}

/// Muestra un menú y espera la elección. `dot`: el ítem con ese id lleva un punto de ese color.
/// Devuelve el id elegido (None si se cerró sin elegir).
pub fn popup_menu(entries: &[Entry], at: MenuAt, dot: Option<(u32, Color)>) -> Option<u32> {
    let mtm = MainThreadMarker::new()?;
    let view = main_view()?;
    let target = MenuTarget::new(mtm);
    let menu = build_menu(entries, &target, dot, mtm);
    let point = match at {
        MenuAt::Point(x, y) => NSPoint::new(x as f64, y as f64),
        MenuAt::Above(x, y) => NSPoint::new(x as f64, y as f64 - menu.size().height),
        MenuAt::Cursor => {
            let window = view.window()?;
            view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None)
        }
    };
    menu.popUpMenuPositioningItem_atLocation_inView(None, point, Some(&view));
    let chosen = target.ivars().get();
    (chosen > 0).then_some(chosen as u32)
}

#[cfg(test)]
mod tests {
    use super::{KeyEquivalent, key_equivalent, title_and_key};

    /// Con un atajo que no es letra ni número, el atajo queda a la vista en el título.
    #[test]
    fn menu_shortcut_text_kept_when_not_a_key_equivalent() {
        let (t, k) = title_and_key("Carpeta nueva\t⇧⌘N");
        assert_eq!((t.as_str(), k.map(|k| k.key)), ("Carpeta nueva", Some("n".to_string())));
        let (t, k) = title_and_key("Actualizar\t⌘F5");
        assert_eq!((t.as_str(), k), ("Actualizar    ⌘F5", None));
        assert_eq!(title_and_key("Pegar"), ("Pegar".to_string(), None));
    }

    /// «⇧⌘N» y «⌥⌘N» pasan a equivalentes de teclado; F5 o un texto suelto, no.
    #[test]
    fn menu_key_equivalent_from_label() {
        let k = |key: &str, cmd, shift, alt, ctrl| Some(KeyEquivalent { key: key.into(), cmd, shift, alt, ctrl });
        assert_eq!(key_equivalent("⇧⌘N"), k("n", true, true, false, false));
        assert_eq!(key_equivalent("⌥⌘N"), k("n", true, false, true, false));
        assert_eq!(key_equivalent("⌃⌘1"), k("1", true, false, false, true));
        assert_eq!(key_equivalent("⌘F5"), None);
        assert_eq!(key_equivalent(""), None);
    }
}

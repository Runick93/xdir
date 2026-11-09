//! Parte de `platform` propia de macOS.

use objc2_app_kit::{NSColor, NSColorSpace, NSEvent, NSEventModifierFlags, NSWorkspace};
use objc2_foundation::{NSString, NSUserDefaults};

use super::Key;

/// ¿La tecla está apretada ahora? En macOS, Ctrl de los atajos es ⌘ (como Ctrl+C → ⌘C) y Alt es ⌥.
pub fn key_down(k: Key) -> bool {
    let flags = NSEvent::modifierFlags_class();
    match k.0 {
        0x10 => flags.contains(NSEventModifierFlags::Shift),
        0x11 => flags.contains(NSEventModifierFlags::Command),
        0x12 => flags.contains(NSEventModifierFlags::Option),
        _ => false,
    }
}

/// ¿El sistema está en modo oscuro?
pub fn system_dark() -> bool {
    let defaults = NSUserDefaults::standardUserDefaults();
    defaults.stringForKey(&NSString::from_str("AppleInterfaceStyle")).is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark"))
}

/// Color de acento del sistema (Configuración del Sistema > Apariencia), 0xRRGGBB.
pub fn system_accent() -> Option<u32> {
    let color = NSColor::controlAccentColor();
    let rgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    Some(ch(rgb.redComponent()) << 16 | ch(rgb.greenComponent()) << 8 | ch(rgb.blueComponent()))
}

/// ¿El sistema permite animaciones? ("Reducir movimiento" en Accesibilidad las apaga.)
pub fn animations_allowed() -> bool {
    !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

// ---------- vista principal y menús emergentes ----------

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSImage, NSImageSymbolConfiguration, NSMenu, NSMenuItem, NSView};
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

fn build_menu(entries: &[Entry], target: &MenuTarget, dot: Option<(u32, Color)>, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for e in entries {
        match e {
            Entry::Sep => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Entry::Item { id, label, enabled, checked } => {
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(label), Some(sel!(pick:)), &NSString::new())
                };
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

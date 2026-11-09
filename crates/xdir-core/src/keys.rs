//! Teclas.

/// Tecla con la numeración de las teclas virtuales de Windows (es la que usan `config.toml` y los
/// atajos). En macOS se traduce desde el código de tecla físico.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Key(pub u16);

/// Teclas con nombre (los mismos valores que `VK_*` de Windows).
pub mod keys {
    use super::Key;
    pub const VK_BACK: Key = Key(0x08);
    pub const VK_TAB: Key = Key(0x09);
    pub const VK_RETURN: Key = Key(0x0D);
    pub const VK_SHIFT: Key = Key(0x10);
    pub const VK_CONTROL: Key = Key(0x11);
    /// Alt (⌥ Option en macOS).
    pub const VK_MENU: Key = Key(0x12);
    pub const VK_ESCAPE: Key = Key(0x1B);
    pub const VK_SPACE: Key = Key(0x20);
    pub const VK_PRIOR: Key = Key(0x21);
    pub const VK_NEXT: Key = Key(0x22);
    pub const VK_END: Key = Key(0x23);
    pub const VK_HOME: Key = Key(0x24);
    pub const VK_LEFT: Key = Key(0x25);
    pub const VK_UP: Key = Key(0x26);
    pub const VK_RIGHT: Key = Key(0x27);
    pub const VK_DOWN: Key = Key(0x28);
    pub const VK_INSERT: Key = Key(0x2D);
    pub const VK_DELETE: Key = Key(0x2E);
    pub const VK_F1: Key = Key(0x70);
    pub const VK_BROWSER_BACK: Key = Key(0xA6);
    pub const VK_BROWSER_FORWARD: Key = Key(0xA7);
}

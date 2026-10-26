//! Acciones de la ventana con nombre y atajo: la paleta de comandos las lista y busca, y los
//! atajos de teclado se resuelven acá (un solo lugar para agregar una acción o cambiar su tecla).

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_BACK, VK_BROWSER_BACK, VK_BROWSER_FORWARD, VK_DELETE, VK_F2, VK_F4, VK_F5, VK_LEFT, VK_RETURN,
    VK_RIGHT, VK_TAB, VK_UP,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    JumpTo,
    Palette,
    NewTab,
    CloseTab,
    ReopenTab,
    NextTab,
    PrevTab,
    NextPane,
    PrevPane,
    /// Panel N (0..3).
    FocusPane(u8),
    /// Layout N (índice de `Layout::ALL`).
    Layout(u8),
    Maximize,
    Back,
    Forward,
    Up,
    Refresh,
    EditAddress,
    Filter,
    GoComputer,
    ToggleHidden,
    SelectAll,
    InvertSelection,
    Copy,
    Cut,
    Paste,
    CopyPaths,
    Undo,
    NewFolder,
    Recycle,
    Delete,
    Rename,
    Properties,
    ToggleFavorite,
    ToggleFavBar,
    PowerShell,
    Cmd,
    ToggleDarkMode,
}

/// Nombre para la paleta y atajo para mostrar (vacío si no tiene).
pub struct Spec {
    pub command: Command,
    pub name: &'static str,
    pub keys: &'static str,
}

const fn spec(command: Command, name: &'static str, keys: &'static str) -> Spec {
    Spec { command, name, keys }
}

/// Todas las acciones, en el orden en que la paleta las muestra sin búsqueda.
pub const ALL: &[Spec] = &[
    spec(Command::JumpTo, "Ir a una carpeta…", "Ctrl+P"),
    spec(Command::NewTab, "Pestaña nueva", "Ctrl+T"),
    spec(Command::CloseTab, "Cerrar pestaña", "Ctrl+W"),
    spec(Command::ReopenTab, "Reabrir la última pestaña cerrada", "Ctrl+Shift+T"),
    spec(Command::NextTab, "Pestaña siguiente", "Ctrl+Tab"),
    spec(Command::PrevTab, "Pestaña anterior", "Ctrl+Shift+Tab"),
    spec(Command::NextPane, "Panel siguiente", "Tab"),
    spec(Command::PrevPane, "Panel anterior", "Shift+Tab"),
    spec(Command::FocusPane(0), "Ir al panel 1", "Alt+1"),
    spec(Command::FocusPane(1), "Ir al panel 2", "Alt+2"),
    spec(Command::FocusPane(2), "Ir al panel 3", "Alt+3"),
    spec(Command::FocusPane(3), "Ir al panel 4", "Alt+4"),
    spec(Command::Layout(0), "Layout: 1 panel", "Ctrl+1"),
    spec(Command::Layout(1), "Layout: 2 paneles lado a lado", "Ctrl+2"),
    spec(Command::Layout(2), "Layout: 2 paneles apilados", "Ctrl+3"),
    spec(Command::Layout(3), "Layout: 3 paneles", "Ctrl+4"),
    spec(Command::Layout(4), "Layout: 4 paneles", "Ctrl+5"),
    spec(Command::Maximize, "Maximizar o restaurar el panel", "Ctrl+M"),
    spec(Command::Back, "Atrás", "Alt+←"),
    spec(Command::Forward, "Adelante", "Alt+→"),
    spec(Command::Up, "Subir a la carpeta superior", "Alt+↑"),
    spec(Command::Refresh, "Recargar", "F5"),
    spec(Command::EditAddress, "Escribir una ruta", "Ctrl+L"),
    spec(Command::Filter, "Filtrar la lista", "Ctrl+F"),
    spec(Command::GoComputer, "Ir a Este equipo", ""),
    spec(Command::ToggleHidden, "Mostrar u ocultar archivos ocultos", "Ctrl+H"),
    spec(Command::SelectAll, "Seleccionar todo", "Ctrl+A"),
    spec(Command::InvertSelection, "Invertir selección", "Ctrl+I"),
    spec(Command::Copy, "Copiar", "Ctrl+C"),
    spec(Command::Cut, "Cortar", "Ctrl+X"),
    spec(Command::Paste, "Pegar", "Ctrl+V"),
    spec(Command::CopyPaths, "Copiar las rutas como texto", "Ctrl+Shift+C"),
    spec(Command::Undo, "Deshacer", "Ctrl+Z"),
    spec(Command::NewFolder, "Carpeta nueva", "Ctrl+Shift+N"),
    spec(Command::Recycle, "Enviar a la papelera", "Supr"),
    spec(Command::Delete, "Eliminar definitivamente", "Shift+Supr"),
    spec(Command::Rename, "Renombrar", "F2"),
    spec(Command::Properties, "Propiedades", "Alt+Enter"),
    spec(Command::ToggleFavorite, "Agregar o quitar de favoritos", "Ctrl+D"),
    spec(Command::ToggleFavBar, "Mostrar u ocultar la barra de favoritos", "Ctrl+Shift+B"),
    spec(Command::PowerShell, "Abrir PowerShell en la carpeta", ""),
    spec(Command::Cmd, "Abrir CMD en la carpeta", ""),
    spec(Command::ToggleDarkMode, "Cambiar entre modo claro y oscuro", ""),
];

/// Atajo de teclado → acción. `alt` viene de WM_SYSKEYDOWN.
pub fn from_key(vk: VIRTUAL_KEY, ctrl: bool, shift: bool, alt: bool) -> Option<Command> {
    use Command::*;
    let letter = |c: u8| vk == VIRTUAL_KEY(c as u16);
    Some(match () {
        _ if alt => match vk {
            VK_LEFT => Back,
            VK_RIGHT => Forward,
            VK_UP => Up,
            VK_RETURN => Properties,
            _ if letter(b'D') => EditAddress,
            VIRTUAL_KEY(k @ 0x31..=0x34) => FocusPane((k - 0x31) as u8),
            _ => return None,
        },
        _ if ctrl && shift => match vk {
            VK_TAB => PrevTab,
            _ if letter(b'P') => Palette,
            _ if letter(b'T') => ReopenTab,
            _ if letter(b'C') => CopyPaths,
            _ if letter(b'N') => NewFolder,
            _ if letter(b'B') => ToggleFavBar,
            _ => return None,
        },
        _ if ctrl => match vk {
            VK_TAB => NextTab,
            _ if letter(b'P') => JumpTo,
            _ if letter(b'T') => NewTab,
            _ if letter(b'W') => CloseTab,
            _ if letter(b'M') => Maximize,
            _ if letter(b'L') => EditAddress,
            _ if letter(b'F') => Filter,
            _ if letter(b'H') => ToggleHidden,
            _ if letter(b'A') => SelectAll,
            _ if letter(b'I') => InvertSelection,
            _ if letter(b'C') => Copy,
            _ if letter(b'X') => Cut,
            _ if letter(b'V') => Paste,
            _ if letter(b'Z') => Undo,
            _ if letter(b'D') => ToggleFavorite,
            VIRTUAL_KEY(k @ 0x31..=0x35) => Layout((k - 0x31) as u8),
            _ => return None,
        },
        _ => match vk {
            VK_TAB if shift => PrevPane,
            VK_TAB => NextPane,
            VK_DELETE if shift => Delete,
            VK_DELETE => Recycle,
            VK_BACK => Up,
            VK_BROWSER_BACK => Back,
            VK_BROWSER_FORWARD => Forward,
            VK_F5 => Refresh,
            VK_F4 => EditAddress,
            VK_F2 => Rename,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada atajo que muestra la paleta tiene que resolver a su propia acción.
    #[test]
    fn shown_keys_match_bindings() {
        let key = |name: &str| -> VIRTUAL_KEY {
            match name {
                "Tab" => VK_TAB,
                "Supr" => VK_DELETE,
                "F2" => VK_F2,
                "F5" => VK_F5,
                "Enter" => VK_RETURN,
                "←" => VK_LEFT,
                "→" => VK_RIGHT,
                "↑" => VK_UP,
                s if s.len() == 1 => VIRTUAL_KEY(s.as_bytes()[0] as u16),
                s => panic!("tecla desconocida en la lista: {s}"),
            }
        };
        for s in ALL.iter().filter(|s| !s.keys.is_empty()) {
            let parts: Vec<&str> = s.keys.split('+').collect();
            let (mods, k) = parts.split_at(parts.len() - 1);
            let has = |m: &str| mods.contains(&m);
            let got = from_key(key(k[0]), has("Ctrl"), has("Shift"), has("Alt"));
            assert_eq!(got, Some(s.command), "{} ({})", s.name, s.keys);
        }
    }
}

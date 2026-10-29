//! Acciones de la ventana con nombre, identificador y atajos: la paleta de comandos las lista y
//! busca, y los atajos de teclado se resuelven con un `Keymap` que parte de los de fábrica y
//! toma los cambios de la sección `[atajos]` de `config.toml`.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_BACK, VK_BROWSER_BACK, VK_BROWSER_FORWARD, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME,
    VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Command {
    JumpTo,
    Palette,
    NewTab,
    CloseTab,
    ReopenTab,
    NextTab,
    PrevTab,
    NextGroup,
    PrevGroup,
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
    SaveGroup,
    ToggleGroupBar,
    OpenConfig,
    About,
    /// Vista N (índice de `list::VIEWS`).
    View(u8),
    /// Ordenar por la columna N (índice de `list::COLUMNS`); repetirlo invierte el orden.
    SortBy(u8),
    /// Árbol de carpetas lateral: solo existe compilando con la feature `arbol`.
    ToggleTree,
    /// Árbol de carpetas: fijarlo o dejar que se oculte solo.
    ToggleTreePin,
}

/// ¿La acción existe en esta compilación? (las experimentales dependen de una feature).
pub fn available(c: Command) -> bool {
    c != Command::ToggleTree || cfg!(feature = "arbol")
}

/// Una acción: identificador para `config.toml`, nombre para la paleta y atajos de fábrica.
pub struct Spec {
    pub command: Command,
    pub id: &'static str,
    pub name: &'static str,
    pub default_keys: &'static [&'static str],
}

const fn spec(command: Command, id: &'static str, name: &'static str, default_keys: &'static [&'static str]) -> Spec {
    Spec { command, id, name, default_keys }
}

/// Todas las acciones, en el orden en que la paleta las muestra sin búsqueda.
pub const ALL: &[Spec] = &[
    spec(Command::JumpTo, "ir_a_carpeta", "Ir a una carpeta…", &["Ctrl+P"]),
    spec(Command::Palette, "paleta", "Paleta de comandos", &["Ctrl+Shift+P"]),
    spec(Command::NewTab, "pestana_nueva", "Pestaña nueva", &["Ctrl+T"]),
    spec(Command::CloseTab, "cerrar_pestana", "Cerrar pestaña", &["Ctrl+W"]),
    spec(Command::ReopenTab, "reabrir_pestana", "Reabrir la última pestaña cerrada", &["Ctrl+Shift+T"]),
    spec(Command::NextTab, "pestana_siguiente", "Pestaña siguiente", &["Ctrl+AvPag"]),
    spec(Command::PrevTab, "pestana_anterior", "Pestaña anterior", &["Ctrl+RePag"]),
    spec(Command::NextGroup, "grupo_siguiente", "Grupo siguiente", &["Ctrl+Tab"]),
    spec(Command::PrevGroup, "grupo_anterior", "Grupo anterior", &["Ctrl+Shift+Tab"]),
    spec(Command::NextPane, "panel_siguiente", "Panel siguiente", &["Tab"]),
    spec(Command::PrevPane, "panel_anterior", "Panel anterior", &["Shift+Tab"]),
    spec(Command::FocusPane(0), "panel_1", "Ir al panel 1", &["Alt+1"]),
    spec(Command::FocusPane(1), "panel_2", "Ir al panel 2", &["Alt+2"]),
    spec(Command::FocusPane(2), "panel_3", "Ir al panel 3", &["Alt+3"]),
    spec(Command::FocusPane(3), "panel_4", "Ir al panel 4", &["Alt+4"]),
    spec(Command::Layout(0), "layout_1", "Layout: 1 panel", &["Ctrl+1"]),
    spec(Command::Layout(1), "layout_2_lado_a_lado", "Layout: 2 paneles lado a lado", &["Ctrl+2"]),
    spec(Command::Layout(2), "layout_2_apilados", "Layout: 2 paneles apilados", &["Ctrl+3"]),
    spec(Command::Layout(3), "layout_3", "Layout: 3 paneles", &["Ctrl+4"]),
    spec(Command::Layout(4), "layout_4", "Layout: 4 paneles", &["Ctrl+5"]),
    spec(Command::Maximize, "maximizar_panel", "Maximizar o restaurar el panel", &["Ctrl+M"]),
    spec(Command::Back, "atras", "Atrás", &["Alt+Izquierda", "NavegadorAtras"]),
    spec(Command::Forward, "adelante", "Adelante", &["Alt+Derecha", "NavegadorAdelante"]),
    spec(Command::Up, "subir", "Subir a la carpeta superior", &["Alt+Arriba", "Retroceso"]),
    spec(Command::Refresh, "recargar", "Recargar", &["F5"]),
    spec(Command::EditAddress, "escribir_ruta", "Escribir una ruta", &["Ctrl+L", "F4", "Alt+D"]),
    spec(Command::Filter, "filtrar", "Filtrar la lista", &["Ctrl+F"]),
    spec(Command::GoComputer, "este_equipo", "Ir a Este equipo", &[]),
    spec(Command::ToggleHidden, "mostrar_ocultos", "Mostrar u ocultar archivos ocultos", &["Ctrl+H"]),
    spec(Command::SelectAll, "seleccionar_todo", "Seleccionar todo", &["Ctrl+A"]),
    spec(Command::InvertSelection, "invertir_seleccion", "Invertir selección", &["Ctrl+I"]),
    spec(Command::Copy, "copiar", "Copiar", &["Ctrl+C"]),
    spec(Command::Cut, "cortar", "Cortar", &["Ctrl+X"]),
    spec(Command::Paste, "pegar", "Pegar", &["Ctrl+V"]),
    spec(Command::CopyPaths, "copiar_rutas", "Copiar las rutas como texto", &["Ctrl+Shift+C"]),
    spec(Command::Undo, "deshacer", "Deshacer", &["Ctrl+Z"]),
    spec(Command::NewFolder, "carpeta_nueva", "Carpeta nueva", &["Ctrl+Shift+N"]),
    spec(Command::Recycle, "papelera", "Enviar a la papelera", &["Supr"]),
    spec(Command::Delete, "eliminar", "Eliminar definitivamente", &["Shift+Supr"]),
    spec(Command::Rename, "renombrar", "Renombrar", &["F2"]),
    spec(Command::Properties, "propiedades", "Propiedades", &["Alt+Enter"]),
    spec(Command::ToggleFavorite, "favorito", "Agregar o quitar de favoritos", &["Ctrl+D"]),
    spec(Command::ToggleFavBar, "barra_favoritos", "Mostrar u ocultar la barra de favoritos", &["Ctrl+Shift+B"]),
    spec(Command::PowerShell, "powershell", "Abrir PowerShell en la carpeta", &[]),
    spec(Command::Cmd, "cmd", "Abrir CMD en la carpeta", &[]),
    spec(Command::ToggleDarkMode, "modo_oscuro", "Cambiar entre modo claro y oscuro", &[]),
    spec(Command::SaveGroup, "guardar_grupo", "Guardar paneles y carpetas como grupo…", &[]),
    spec(Command::ToggleGroupBar, "barra_grupos", "Mostrar u ocultar la barra de grupos", &[]),
    spec(Command::OpenConfig, "abrir_configuracion", "Abrir la configuración (config.toml)", &[]),
    spec(Command::About, "acerca_de", "Acerca de xdir (versión y reportar un problema)", &[]),
    spec(Command::View(0), "vista_detalles", "Vista: detalles", &["Ctrl+Shift+1"]),
    spec(Command::View(1), "vista_iconos", "Vista: íconos medianos", &["Ctrl+Shift+2"]),
    spec(Command::View(2), "vista_iconos_grandes", "Vista: íconos grandes", &["Ctrl+Shift+3"]),
    spec(Command::View(3), "vista_miniaturas", "Vista: miniaturas", &["Ctrl+Shift+4"]),
    spec(Command::SortBy(0), "ordenar_nombre", "Ordenar por nombre", &[]),
    spec(Command::SortBy(1), "ordenar_modificado", "Ordenar por fecha de modificación", &[]),
    spec(Command::SortBy(2), "ordenar_creado", "Ordenar por fecha de creación", &[]),
    spec(Command::SortBy(3), "ordenar_tipo", "Ordenar por tipo", &[]),
    spec(Command::SortBy(4), "ordenar_extension", "Ordenar por extensión", &[]),
    spec(Command::SortBy(5), "ordenar_tamano", "Ordenar por tamaño", &[]),
    spec(Command::ToggleTree, "arbol", "Mostrar u ocultar el árbol de carpetas (experimental)", &["Ctrl+Shift+E"]),
];

/// Tecla con modificadores.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Combo {
    pub vk: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Nombres de teclas que no son una letra, un número ni F1..F24: (nombre en config, tecla, nombre
/// para mostrar). Se aceptan también en inglés.
const KEY_NAMES: &[(&[&str], VIRTUAL_KEY, &str)] = &[
    (&["tab"], VK_TAB, "Tab"),
    (&["enter", "intro", "return"], VK_RETURN, "Enter"),
    (&["supr", "delete", "del"], VK_DELETE, "Supr"),
    (&["insert", "ins"], VK_INSERT, "Insert"),
    (&["retroceso", "backspace"], VK_BACK, "Retroceso"),
    (&["esc", "escape"], VK_ESCAPE, "Esc"),
    (&["espacio", "space"], VK_SPACE, "Espacio"),
    (&["izquierda", "left", "←"], VK_LEFT, "←"),
    (&["derecha", "right", "→"], VK_RIGHT, "→"),
    (&["arriba", "up", "↑"], VK_UP, "↑"),
    (&["abajo", "down", "↓"], VK_DOWN, "↓"),
    (&["inicio", "home"], VK_HOME, "Inicio"),
    (&["fin", "end"], VK_END, "Fin"),
    (&["repag", "pageup"], VK_PRIOR, "RePág"),
    (&["avpag", "pagedown"], VK_NEXT, "AvPág"),
    (&["navegadoratras", "browserback"], VK_BROWSER_BACK, "Botón atrás del mouse"),
    (&["navegadoradelante", "browserforward"], VK_BROWSER_FORWARD, "Botón adelante del mouse"),
];

/// "Ctrl+Shift+T" → combinación. Sin distinguir mayúsculas; acepta "Control" y "Mayús".
pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let mut combo = Combo { vk: 0, ctrl: false, shift: false, alt: false };
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or("atajo vacío")?;
    for m in mods {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => combo.ctrl = true,
            "shift" | "mayús" | "mayus" => combo.shift = true,
            "alt" => combo.alt = true,
            other => return Err(format!("modificador desconocido «{other}» (usá Ctrl, Shift o Alt)")),
        }
    }
    let lower = key.to_lowercase();
    combo.vk = if let Some((_, vk, _)) = KEY_NAMES.iter().find(|(names, _, _)| names.contains(&lower.as_str())) {
        vk.0
    } else if lower.len() == 1 && lower.chars().all(|c| c.is_ascii_alphanumeric()) {
        lower.to_ascii_uppercase().as_bytes()[0] as u16
    } else if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
        VK_F1.0 + n - 1
    } else {
        return Err(format!("tecla desconocida «{key}»"));
    };
    Ok(combo)
}

/// Combinación → texto para mostrar ("Ctrl+Shift+T", "Alt+←").
pub fn combo_label(c: Combo) -> String {
    let mut s = String::new();
    for (on, name) in [(c.ctrl, "Ctrl+"), (c.shift, "Shift+"), (c.alt, "Alt+")] {
        if on {
            s.push_str(name);
        }
    }
    if let Some((_, _, label)) = KEY_NAMES.iter().find(|(_, vk, _)| vk.0 == c.vk) {
        s.push_str(label);
    } else if (VK_F1.0..VK_F1.0 + 24).contains(&c.vk) {
        s.push_str(&format!("F{}", c.vk - VK_F1.0 + 1));
    } else {
        s.push(c.vk as u8 as char);
    }
    s
}

/// Atajos en uso: los de fábrica con los cambios de `config.toml`.
#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: Vec<(Combo, Command)>,
}

impl Default for Keymap {
    fn default() -> Self {
        let bindings = ALL
            .iter()
            .filter(|s| available(s.command))
            .flat_map(|s| s.default_keys.iter().map(move |k| (parse_combo(k).expect("atajo de fábrica válido"), s.command)))
            .collect();
        Self { bindings }
    }
}

impl Keymap {
    /// Reemplaza los atajos de una acción (lista vacía: la deja sin atajo). Una combinación que
    /// ya usaba otra acción pasa a esta. Devuelve las acciones que perdieron esa combinación.
    pub fn set(&mut self, command: Command, combos: &[Combo]) -> Vec<Command> {
        self.bindings.retain(|(_, c)| *c != command);
        let mut displaced = Vec::new();
        for combo in combos {
            if let Some(pos) = self.bindings.iter().position(|(k, _)| k == combo) {
                displaced.push(self.bindings.remove(pos).1);
            }
            self.bindings.push((*combo, command));
        }
        displaced
    }

    pub fn lookup(&self, vk: VIRTUAL_KEY, ctrl: bool, shift: bool, alt: bool) -> Option<Command> {
        let combo = Combo { vk: vk.0, ctrl, shift, alt };
        self.bindings.iter().find(|(k, _)| *k == combo).map(|(_, c)| *c)
    }

    /// Atajos de una acción, para mostrar en la paleta.
    pub fn label(&self, command: Command) -> String {
        let labels: Vec<String> = self.bindings.iter().filter(|(_, c)| *c == command).map(|(k, _)| combo_label(*k)).collect();
        // En la paleta entra uno: el primero (el principal).
        labels.into_iter().next().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse_and_print() {
        let c = parse_combo("ctrl+shift+t").unwrap();
        assert_eq!(c, Combo { vk: b'T' as u16, ctrl: true, shift: true, alt: false });
        assert_eq!(combo_label(c), "Ctrl+Shift+T");
        assert_eq!(combo_label(parse_combo("Alt+Izquierda").unwrap()), "Alt+←");
        assert_eq!(combo_label(parse_combo("F12").unwrap()), "F12");
        assert_eq!(parse_combo("Mayús+Supr").unwrap().vk, VK_DELETE.0);
        assert!(parse_combo("Ctrl+Hiper").is_err());
        assert!(parse_combo("Meta+X").is_err());
    }

    /// Cada atajo de fábrica resuelve a su propia acción, y no hay dos acciones con la misma tecla.
    #[test]
    fn defaults_are_consistent() {
        let km = Keymap::default();
        let mut seen = std::collections::HashSet::new();
        for s in ALL.iter().filter(|s| available(s.command)) {
            for k in s.default_keys {
                let c = parse_combo(k).unwrap();
                assert!(seen.insert(c), "atajo repetido: {k}");
                assert_eq!(km.lookup(VIRTUAL_KEY(c.vk), c.ctrl, c.shift, c.alt), Some(s.command), "{k}");
            }
        }
        let ids: std::collections::HashSet<_> = ALL.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), ALL.len(), "identificadores repetidos");
    }

    /// El árbol solo existe con la feature `arbol`: sin ella, ni su atajo queda tomado.
    #[test]
    fn experimental_tree_depends_on_feature() {
        let km = Keymap::default();
        let e = parse_combo("Ctrl+Shift+E").unwrap();
        let bound = km.lookup(VIRTUAL_KEY(e.vk), e.ctrl, e.shift, e.alt);
        assert_eq!(available(Command::ToggleTree), cfg!(feature = "arbol"));
        assert_eq!(bound, cfg!(feature = "arbol").then_some(Command::ToggleTree));
    }

    #[test]
    fn overrides() {
        let mut km = Keymap::default();
        let ctrl_k = parse_combo("Ctrl+K").unwrap();
        let ctrl_w = parse_combo("Ctrl+W").unwrap();
        // pestana_nueva = ["Ctrl+K", "Ctrl+W"]: Ctrl+T deja de funcionar y Ctrl+W se le saca a cerrar.
        let displaced = km.set(Command::NewTab, &[ctrl_k, ctrl_w]);
        assert_eq!(displaced, [Command::CloseTab]);
        assert_eq!(km.lookup(VIRTUAL_KEY(b'K' as u16), true, false, false), Some(Command::NewTab));
        assert_eq!(km.lookup(VIRTUAL_KEY(b'W' as u16), true, false, false), Some(Command::NewTab));
        assert_eq!(km.lookup(VIRTUAL_KEY(b'T' as u16), true, false, false), None);
        assert_eq!(km.label(Command::CloseTab), "");
        km.set(Command::NewTab, &[]);
        assert_eq!(km.label(Command::NewTab), "");
    }
}

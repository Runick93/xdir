//! Acciones de la ventana con nombre, identificador y atajos: la paleta de comandos las lista y
//! busca, y los atajos de teclado se resuelven con un `Keymap` que parte de los de fábrica y
//! toma los cambios de la sección `[atajos]` de `config.toml`.

use crate::keys::Key;
use crate::sys::{KeyStyle, SystemEnv, env};
use crate::keys::keys::{
    VK_BACK, VK_BROWSER_BACK, VK_BROWSER_FORWARD, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT,
    VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
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
    /// Sumar a la selección los que coinciden con un patrón (`*.jpg;*.png`), escrito en la paleta.
    SelectPattern,
    /// Quitar de la selección los que coinciden con un patrón.
    DeselectPattern,
    Copy,
    Cut,
    Paste,
    CopyPaths,
    Undo,
    NewFolder,
    /// Archivo de texto vacío en la carpeta del panel, con el renombrado en línea abierto.
    NewTextFile,
    /// Carpeta nueva «Carpeta nueva con N elementos» con la selección adentro.
    NewFolderWithSelection,
    Recycle,
    Delete,
    /// Devolver de la Papelera a su lugar original.
    Restore,
    /// Acceso directo (alias, enlace) junto a cada seleccionado.
    CreateShortcut,
    /// Abrir la carpeta del original de un acceso directo, con él seleccionado.
    ShowOriginal,
    /// Un zip con la selección («nombre.zip» o «Archivo.zip»).
    Compress,
    /// Extraer el archivo seleccionado en su carpeta.
    ExtractHere,
    /// Extraer el archivo seleccionado en una carpeta nueva con su nombre.
    ExtractInto,
    /// Expulsar el disco enfocado en «Este equipo» o el de la carpeta del panel.
    Eject,
    /// Conectar a un servidor… (el panel con la dirección, favoritos y recientes).
    ConnectServer,
    /// Conectar unidad de red… (solo Windows).
    MapNetworkDrive,
    /// Desconectar unidad de red… (solo Windows).
    DisconnectNetworkDrive,
    Rename,
    /// Renombrar varios: el panel de la derecha con la vista previa en la lista.
    RenameMany,
    Properties,
    /// Copia de cada seleccionado a su lado («nombre copia»).
    Duplicate,
    ToggleFavorite,
    ToggleFavBar,
    PowerShell,
    Cmd,
    ToggleDarkMode,
    SaveGroup,
    ToggleGroupBar,
    OpenConfig,
    /// La pestaña Configuración (una sola: abrirla de nuevo la enfoca).
    Settings,
    /// La pestaña Configuración en la página Atajos de teclado.
    SettingsKeys,
    About,
    /// La pestaña Novedades (una sola: abrirla de nuevo la enfoca).
    Changelog,
    /// Vista N (índice de `list::VIEWS`).
    View(u8),
    /// Ordenar por la columna N (índice de `list::COLUMNS`); repetirlo invierte el orden.
    SortBy(u8),
    /// Agrupar por el criterio N (índice de `list::GROUPINGS`; el último es «No agrupar»).
    GroupBy(u8),
    /// Árbol de carpetas lateral: solo existe compilando con la feature `arbol`.
    ToggleTree,
    /// Árbol de carpetas: fijarlo o dejarlo en franja de íconos (se expande al asomar el mouse).
    ToggleTreePin,
    /// Anclar la carpeta del panel activo al Acceso rápido de Windows, o desanclarla.
    ToggleQuickPin,
    /// Abrir o cerrar xtree, el mapa de uso de disco.
    Xtree,
    /// Dentro de xtree: colorear el mapa por carpeta o por tipo de archivo (solo vale con la vista
    /// abierta: `xtree_only`).
    XtreeColors,
    /// Dentro de xtree: analizar de nuevo con permisos (administrador en Windows; en macOS, el
    /// ajuste de Acceso total al disco), si faltaron.
    XtreeElevate,
    /// La vista de la carpeta actual pasa a ser la por defecto y se borran las de las demás.
    ApplyViewToAll,
    /// La carpeta actual vuelve a la vista por defecto.
    ResetFolderView,
    /// Quita los filtros por columna de la lista activa.
    ClearFilters,
    /// Mostrar u ocultar la columna de vista previa (a la derecha de los paneles).
    TogglePreview,
    /// Vista rápida del elemento seleccionado (Espacio): la abre, o la cierra si estaba abierta.
    QuickLook,
    /// Muestra la ventana de copia (la cola de operaciones).
    ShowTransfers,
}

/// ¿La acción existe en esta compilación y en este sistema? (las experimentales dependen de una
/// feature; el sistema puede no tenerla, como el Acceso rápido de Windows).
pub fn available(c: Command) -> bool {
    available_in(env(), c)
}

/// `available` con el entorno dado.
pub fn available_in(env: &dyn SystemEnv, c: Command) -> bool {
    env.command_available(c) && (c != Command::ToggleTree || cfg!(feature = "arbol"))
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

impl Spec {
    /// Atajos de fábrica en este sistema.
    pub fn keys(&self) -> &'static [&'static str] {
        self.keys_in(env())
    }

    /// `keys` con el entorno dado.
    pub fn keys_in(&self, env: &dyn SystemEnv) -> &'static [&'static str] {
        env.command_keys(self.command).unwrap_or(self.default_keys)
    }

    /// Nombre para la paleta en este sistema.
    pub fn title(&self) -> &'static str {
        self.title_in(env())
    }

    /// `title` con el entorno dado.
    pub fn title_in(&self, env: &dyn SystemEnv) -> &'static str {
        env.command_title(self.command).unwrap_or(self.name)
    }
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
    spec(Command::Filter, "filtrar", "Buscar en la carpeta y sus subcarpetas", &["Ctrl+F"]),
    spec(Command::GoComputer, "este_equipo", "Ir a Este equipo", &[]),
    spec(Command::ToggleHidden, "mostrar_ocultos", "Mostrar u ocultar archivos ocultos", &["Ctrl+H"]),
    spec(Command::SelectAll, "seleccionar_todo", "Seleccionar todo", &["Ctrl+A"]),
    spec(Command::InvertSelection, "invertir_seleccion", "Invertir selección", &["Ctrl+I"]),
    spec(Command::SelectPattern, "seleccionar_patron", "Seleccionar por patrón…", &["Ctrl+Más", "Ctrl+MásNum"]),
    spec(Command::DeselectPattern, "deseleccionar_patron", "Deseleccionar por patrón…", &["Ctrl+Menos", "Ctrl+MenosNum"]),
    spec(Command::Copy, "copiar", "Copiar", &["Ctrl+C"]),
    spec(Command::Cut, "cortar", "Cortar", &["Ctrl+X"]),
    spec(Command::Paste, "pegar", "Pegar", &["Ctrl+V"]),
    spec(Command::CopyPaths, "copiar_rutas", "Copiar las rutas como texto", &["Ctrl+Shift+C"]),
    spec(Command::Undo, "deshacer", "Deshacer", &["Ctrl+Z"]),
    spec(Command::NewFolder, "carpeta_nueva", "Carpeta nueva", &["Ctrl+Shift+N"]),
    spec(Command::NewTextFile, "archivo_texto_nuevo", "Archivo de texto nuevo", &["Ctrl+Alt+N"]),
    spec(Command::NewFolderWithSelection, "carpeta_con_seleccion", "Carpeta nueva con la selección", &["Ctrl+Alt+Shift+N"]),
    spec(Command::Recycle, "papelera", "Enviar a la papelera", &["Supr"]),
    spec(Command::Delete, "eliminar", "Eliminar definitivamente", &["Shift+Supr"]),
    spec(Command::Restore, "devolver", "Devolver", &[]),
    spec(Command::CreateShortcut, "crear_acceso_directo", "Crear acceso directo", &[]),
    spec(Command::ShowOriginal, "mostrar_original", "Mostrar el original", &[]),
    spec(Command::Compress, "comprimir", "Comprimir", &[]),
    spec(Command::ExtractHere, "extraer_aqui", "Extraer aquí", &[]),
    spec(Command::ExtractInto, "extraer_en", "Extraer en una carpeta", &[]),
    spec(Command::Eject, "expulsar", "Expulsar", &[]),
    spec(Command::ConnectServer, "conectar_servidor", "Conectar a un servidor…", &["Ctrl+K"]),
    spec(Command::MapNetworkDrive, "conectar_unidad_red", "Conectar unidad de red…", &[]),
    spec(Command::DisconnectNetworkDrive, "desconectar_unidad_red", "Desconectar unidad de red…", &[]),
    spec(Command::Rename, "renombrar", "Renombrar", &["F2"]),
    spec(Command::RenameMany, "renombrar_varios", "Renombrar varios…", &[]),
    spec(Command::Properties, "propiedades", "Propiedades", &["Alt+Enter"]),
    spec(Command::Duplicate, "duplicar", "Duplicar", &["Ctrl+D"]),
    spec(Command::ToggleFavorite, "favorito", "Agregar o quitar de favoritos", &["Ctrl+Shift+D"]),
    spec(Command::ToggleFavBar, "barra_favoritos", "Mostrar u ocultar la barra de favoritos", &["Ctrl+Shift+B"]),
    spec(Command::PowerShell, "powershell", "Abrir PowerShell en la carpeta", &[]),
    spec(Command::Cmd, "cmd", "Abrir CMD en la carpeta", &[]),
    spec(Command::ToggleDarkMode, "modo_oscuro", "Cambiar entre modo claro y oscuro", &[]),
    spec(Command::SaveGroup, "guardar_grupo", "Guardar paneles y carpetas como grupo…", &[]),
    spec(Command::ToggleGroupBar, "barra_grupos", "Mostrar u ocultar la barra de grupos", &[]),
    spec(Command::OpenConfig, "abrir_configuracion", "Abrir config.toml", &[]),
    spec(Command::Settings, "configuracion", "Configuración", &["Ctrl+Coma"]),
    spec(Command::SettingsKeys, "configuracion_atajos", "Configuración: Atajos de teclado", &[]),
    spec(Command::TogglePreview, "vista_previa", "Mostrar u ocultar la vista previa", &["Alt+P"]),
    spec(Command::QuickLook, "vista_rapida", "Vista rápida", &["Espacio"]),
    spec(Command::ShowTransfers, "mostrar_operaciones", "Mostrar operaciones", &[]),
    spec(Command::Xtree, "xtree", "Mapa de uso de disco (xtree)", &[]),
    spec(Command::XtreeColors, "xtree_colores", "xtree: colorear por carpeta o por tipo de archivo", &["Ctrl+T"]),
    spec(Command::XtreeElevate, "xtree_permisos", "xtree: analizar de nuevo con permisos", &["Ctrl+Shift+A"]),
    spec(Command::About, "acerca_de", "Acerca de xdir (versión y reportar un problema)", &[]),
    spec(Command::Changelog, "novedades", "Novedades de xdir", &[]),
    spec(Command::View(0), "vista_detalles", "Vista: detalles", &["Ctrl+Shift+1"]),
    spec(Command::View(1), "vista_iconos", "Vista: íconos medianos", &["Ctrl+Shift+2"]),
    spec(Command::View(2), "vista_iconos_grandes", "Vista: íconos grandes", &["Ctrl+Shift+3"]),
    spec(Command::View(3), "vista_miniaturas", "Vista: miniaturas", &["Ctrl+Shift+4"]),
    spec(Command::View(4), "vista_lista", "Vista: lista", &["Ctrl+Shift+5"]),
    spec(Command::View(5), "vista_contenido", "Vista: contenido", &["Ctrl+Shift+6"]),
    spec(Command::View(6), "vista_galeria", "Vista: galería", &["Ctrl+Shift+7"]),
    spec(Command::SortBy(0), "ordenar_nombre", "Ordenar por nombre", &[]),
    spec(Command::SortBy(1), "ordenar_modificado", "Ordenar por fecha de modificación", &[]),
    spec(Command::SortBy(2), "ordenar_creado", "Ordenar por fecha de creación", &[]),
    spec(Command::SortBy(3), "ordenar_tipo", "Ordenar por tipo", &[]),
    spec(Command::SortBy(4), "ordenar_extension", "Ordenar por extensión", &[]),
    spec(Command::SortBy(5), "ordenar_tamano", "Ordenar por tamaño", &[]),
    spec(Command::GroupBy(0), "agrupar_nombre", "Agrupar por nombre", &[]),
    spec(Command::GroupBy(1), "agrupar_modificado", "Agrupar por fecha de modificación", &[]),
    spec(Command::GroupBy(2), "agrupar_creado", "Agrupar por fecha de creación", &[]),
    spec(Command::GroupBy(3), "agrupar_tipo", "Agrupar por tipo", &[]),
    spec(Command::GroupBy(4), "agrupar_tamano", "Agrupar por tamaño", &[]),
    spec(Command::GroupBy(5), "agrupar_dimensiones", "Agrupar por dimensiones", &[]),
    spec(Command::GroupBy(6), "agrupar_duracion", "Agrupar por duración", &[]),
    spec(Command::GroupBy(7), "no_agrupar", "No agrupar", &[]),
    spec(Command::ApplyViewToAll, "vista_en_todas", "Usar esta vista en todas las carpetas", &[]),
    spec(Command::ResetFolderView, "restablecer_vista", "Restablecer la vista de esta carpeta", &[]),
    spec(Command::ClearFilters, "quitar_filtros", "Quitar los filtros", &[]),
    spec(Command::ToggleTree, "arbol", "Mostrar u ocultar el árbol de carpetas (experimental)", &["Ctrl+Shift+E"]),
];

/// Grupos de la página de atajos (la wiki y la configuración), en este orden.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Group {
    Navigation,
    TabsPanes,
    Selection,
    Files,
    Views,
    Preview,
    Network,
    /// Lo de la vista de xtree (sus atajos solo valen con la vista abierta).
    Xtree,
    Other,
}

impl Group {
    pub const ALL: [Group; 9] =
        [Group::Navigation, Group::TabsPanes, Group::Selection, Group::Files, Group::Views, Group::Preview, Group::Network, Group::Xtree, Group::Other];

    pub fn title(self) -> &'static str {
        match self {
            Group::Navigation => "Navegación",
            Group::TabsPanes => "Pestañas y paneles",
            Group::Selection => "Selección",
            Group::Files => "Archivos",
            Group::Views => "Vistas",
            Group::Preview => "Vista previa y vista rápida",
            Group::Network => "Red",
            Group::Xtree => "xtree",
            Group::Other => "Otros",
        }
    }
}

/// El grupo de cada comando (sin `_`: un comando nuevo tiene que elegir el suyo).
pub fn group(c: Command) -> Group {
    match c {
        Command::JumpTo
        | Command::Back
        | Command::Forward
        | Command::Up
        | Command::Refresh
        | Command::EditAddress
        | Command::Filter
        | Command::GoComputer
        | Command::ToggleFavorite
        | Command::ToggleFavBar
        | Command::ToggleQuickPin => Group::Navigation,
        Command::NewTab
        | Command::CloseTab
        | Command::ReopenTab
        | Command::NextTab
        | Command::PrevTab
        | Command::NextGroup
        | Command::PrevGroup
        | Command::NextPane
        | Command::PrevPane
        | Command::FocusPane(_)
        | Command::Layout(_)
        | Command::Maximize
        | Command::SaveGroup
        | Command::ToggleGroupBar => Group::TabsPanes,
        Command::SelectAll | Command::InvertSelection | Command::SelectPattern | Command::DeselectPattern => Group::Selection,
        Command::Copy
        | Command::Cut
        | Command::Paste
        | Command::CopyPaths
        | Command::Undo
        | Command::NewFolder
        | Command::NewTextFile
        | Command::NewFolderWithSelection
        | Command::Recycle
        | Command::Delete
        | Command::Restore
        | Command::CreateShortcut
        | Command::ShowOriginal
        | Command::Eject
        | Command::Rename
        | Command::RenameMany
        | Command::Properties
        | Command::Duplicate
        | Command::Compress
        | Command::ExtractHere
        | Command::ExtractInto
        | Command::ShowTransfers => Group::Files,
        Command::View(_)
        | Command::SortBy(_)
        | Command::GroupBy(_)
        | Command::ApplyViewToAll
        | Command::ResetFolderView
        | Command::ClearFilters
        | Command::ToggleHidden
        | Command::ToggleTree
        | Command::ToggleTreePin
        | Command::ToggleDarkMode => Group::Views,
        Command::TogglePreview | Command::QuickLook => Group::Preview,
        Command::ConnectServer | Command::MapNetworkDrive | Command::DisconnectNetworkDrive => Group::Network,
        Command::Xtree | Command::XtreeColors | Command::XtreeElevate => Group::Xtree,
        Command::Palette
        | Command::PowerShell
        | Command::Cmd
        | Command::OpenConfig
        | Command::Settings
        | Command::SettingsKeys
        | Command::About
        | Command::Changelog => Group::Other,
    }
}

/// ¿El comando es de la vista de xtree? Sus atajos solo valen con la vista abierta y no chocan
/// con los de la ventana (Ctrl+T cambia los colores en xtree y abre una pestaña en los paneles).
pub fn xtree_only(c: Command) -> bool {
    matches!(c, Command::XtreeColors | Command::XtreeElevate)
}

/// Comandos de la ventana que siguen andando con xtree abierto (`App::on_xtree_key`): un comando
/// de xtree que tome su tecla choca con ellos.
pub fn live_in_xtree(c: Command) -> bool {
    matches!(c, Command::Filter | Command::Palette | Command::Xtree | Command::ToggleDarkMode | Command::About | Command::OpenConfig)
}

/// Tabla de atajos para la wiki: por grupo, en el orden de `ALL`, solo lo que existe en `env`.
pub fn markdown_table(env: &dyn SystemEnv) -> String {
    let mut out = String::new();
    for g in Group::ALL {
        let specs: Vec<&Spec> = ALL.iter().filter(|s| group(s.command) == g && available_in(env, s.command)).collect();
        if specs.is_empty() {
            continue;
        }
        out += &format!("### {}\n\n| Acción | Atajo | Identificador |\n|---|---|---|\n", g.title());
        for s in specs {
            let keys: Vec<String> = s.keys_in(env).iter().filter_map(|k| parse_combo(k).ok()).map(|c| combo_label_in(env.key_style(), c)).collect();
            out += &format!("| {} | {} | `{}` |\n", s.title_in(env), keys.join(", "), s.id);
        }
        out += "\n";
    }
    out
}

/// Tecla con modificadores.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Combo {
    pub vk: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// Tecla Windows / Super; en macOS, Control (⌃).
    pub meta: bool,
}

/// Nombres de teclas que no son una letra, un número ni F1..F24: (nombre en config, tecla, nombre
/// para mostrar). Se aceptan también en inglés.
const KEY_NAMES: &[(&[&str], Key, &str)] = &[
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
    (&["punto", "."], Key(0xBE), "."),
    (&["coma", ","], Key(0xBC), ","),
    (&["["], Key(0xDB), "["),
    (&["]"], Key(0xDD), "]"),
    // «+» y «-» del teclado principal y del numérico.
    (&["más", "mas", "plus"], Key(0xBB), "Más"),
    (&["menos", "minus"], Key(0xBD), "Menos"),
    (&["másnum", "masnum", "numplus", "add"], Key(0x6B), "Más (teclado numérico)"),
    (&["menosnum", "numminus", "subtract"], Key(0x6D), "Menos (teclado numérico)"),
];

/// Estilo Apple: símbolos, en el orden de Apple (⌃⌥⇧⌘).
fn mac_key_label(vk: u16) -> Option<&'static str> {
    Some(match vk {
        0x2E => "⌦",
        0x08 => "⌫",
        0x0D => "↩",
        0x1B => "⎋",
        0x09 => "⇥",
        0x21 => "⇞",
        0x22 => "⇟",
        0x24 => "↖",
        0x23 => "↘",
        0xBB | 0x6B => "+",
        0xBD | 0x6D => "-",
        _ => return None,
    })
}

/// "Ctrl+Shift+T" → combinación. Sin distinguir mayúsculas; acepta "Control" y "Mayús", y
/// "Meta", "Win" o "Super" para la tecla Windows (Control en macOS).
pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let mut combo = Combo { vk: 0, ctrl: false, shift: false, alt: false, meta: false };
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or("atajo vacío")?;
    for m in mods {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => combo.ctrl = true,
            "shift" | "mayús" | "mayus" => combo.shift = true,
            "alt" => combo.alt = true,
            "meta" | "win" | "super" => combo.meta = true,
            other => return Err(format!("modificador desconocido «{other}» (usá Ctrl, Shift, Alt o Meta)")),
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

/// Nombres de tecla como se escriben en `config.toml` (los de la plantilla), para las que no son
/// una letra, un número ni F1..F24.
const CONFIG_NAMES: &[(u16, &str)] = &[
    (VK_TAB.0, "Tab"),
    (VK_RETURN.0, "Enter"),
    (VK_DELETE.0, "Supr"),
    (VK_INSERT.0, "Insert"),
    (VK_BACK.0, "Retroceso"),
    (VK_ESCAPE.0, "Esc"),
    (VK_SPACE.0, "Espacio"),
    (VK_LEFT.0, "Izquierda"),
    (VK_RIGHT.0, "Derecha"),
    (VK_UP.0, "Arriba"),
    (VK_DOWN.0, "Abajo"),
    (VK_HOME.0, "Inicio"),
    (VK_END.0, "Fin"),
    (VK_PRIOR.0, "RePag"),
    (VK_NEXT.0, "AvPag"),
    (VK_BROWSER_BACK.0, "NavegadorAtras"),
    (VK_BROWSER_FORWARD.0, "NavegadorAdelante"),
    (0xBE, "Punto"),
    (0xBC, "Coma"),
    (0xDB, "["),
    (0xDD, "]"),
    (0xBB, "Más"),
    (0xBD, "Menos"),
    (0x6B, "MásNum"),
    (0x6D, "MenosNum"),
];

/// ¿La tecla tiene nombre en `config.toml`? (las demás no se pueden escribir como atajo).
pub fn nameable(vk: u16) -> bool {
    CONFIG_NAMES.iter().any(|(k, _)| *k == vk)
        || (VK_F1.0..VK_F1.0 + 24).contains(&vk)
        || (b'A' as u16..=b'Z' as u16).contains(&vk)
        || (b'0' as u16..=b'9' as u16).contains(&vk)
}

/// Combinación → texto de `config.toml` ("Meta+Ctrl+Shift+Alt+K", "Ctrl+Coma"): lo que
/// `parse_combo` vuelve a leer igual.
pub fn combo_text(c: Combo) -> String {
    let mut s = String::new();
    for (on, name) in [(c.meta, "Meta+"), (c.ctrl, "Ctrl+"), (c.shift, "Shift+"), (c.alt, "Alt+")] {
        if on {
            s.push_str(name);
        }
    }
    if let Some((_, name)) = CONFIG_NAMES.iter().find(|(k, _)| *k == c.vk) {
        s.push_str(name);
    } else if (VK_F1.0..VK_F1.0 + 24).contains(&c.vk) {
        s.push_str(&format!("F{}", c.vk - VK_F1.0 + 1));
    } else {
        s.push((c.vk as u8 as char).to_ascii_uppercase());
    }
    s
}

/// Combinación → texto para mostrar ("Ctrl+Shift+T", "Alt+←").
pub fn combo_label(c: Combo) -> String {
    combo_label_in(env().key_style(), c)
}

/// `combo_label` con el estilo dado.
pub fn combo_label_in(style: KeyStyle, c: Combo) -> String {
    let mut s = String::new();
    // Apple: ⌃⌥⇧⌘ (Meta es ⌃); Windows: Win, Ctrl, Shift, Alt.
    let mods = match style {
        KeyStyle::Mac => [(c.meta, "⌃"), (c.alt, "⌥"), (c.shift, "⇧"), (c.ctrl, "⌘")],
        KeyStyle::Windows => [(c.meta, "Win+"), (c.ctrl, "Ctrl+"), (c.shift, "Shift+"), (c.alt, "Alt+")],
    };
    for (on, name) in mods {
        if on {
            s.push_str(name);
        }
    }
    if style == KeyStyle::Mac
        && let Some(label) = mac_key_label(c.vk)
    {
        s.push_str(label);
        return s;
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

/// Texto de un atajo escrito como en `config.toml` ("Ctrl+Shift+B") para mostrar en la interfaz.
pub fn shortcut_label(text: &str) -> String {
    parse_combo(text).map(combo_label).unwrap_or_else(|_| text.to_string())
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
            .flat_map(|s| s.keys().iter().map(move |k| (parse_combo(k).expect("atajo de fábrica válido"), s.command)))
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
        let scope = xtree_only(command);
        for combo in combos {
            // Solo le quita la tecla a un comando del mismo ámbito (la ventana o xtree).
            if let Some(pos) = self.bindings.iter().position(|(k, c)| k == combo && xtree_only(*c) == scope) {
                displaced.push(self.bindings.remove(pos).1);
            }
            self.bindings.push((*combo, command));
        }
        displaced
    }

    pub fn lookup(&self, vk: Key, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Option<Command> {
        let combo = Combo { vk: vk.0, ctrl, shift, alt, meta };
        self.bindings.iter().find(|(k, c)| *k == combo && !xtree_only(*c)).map(|(_, c)| *c)
    }

    /// Como `lookup`, pero entre los comandos de la vista de xtree (`xtree_only`).
    pub fn lookup_xtree(&self, vk: Key, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Option<Command> {
        let combo = Combo { vk: vk.0, ctrl, shift, alt, meta };
        self.bindings.iter().find(|(k, c)| *k == combo && xtree_only(*c)).map(|(_, c)| *c)
    }

    /// Los atajos de una acción, en orden (el primero es el principal).
    pub fn combos(&self, command: Command) -> Vec<Combo> {
        self.bindings.iter().filter(|(_, c)| *c == command).map(|(k, _)| *k).collect()
    }

    /// La acción que tiene esa combinación.
    pub fn owner(&self, k: Combo) -> Option<Command> {
        self.bindings.iter().find(|(b, c)| *b == k && !xtree_only(*c)).map(|(_, c)| *c)
    }

    /// Quién tiene la tecla entre los comandos del mismo ámbito que `command` (con quién chocaría).
    /// Para uno de xtree, también los de la ventana que siguen andando dentro de xtree
    /// (`live_in_xtree`: Ctrl+F busca).
    pub fn owner_for(&self, k: Combo, command: Command) -> Option<Command> {
        let scope = xtree_only(command);
        let same = self.bindings.iter().find(|(b, c)| *b == k && xtree_only(*c) == scope).map(|(_, c)| *c);
        same.or_else(|| scope.then(|| self.bindings.iter().find(|(b, c)| *b == k && live_in_xtree(*c)).map(|(_, c)| *c)).flatten())
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
    use crate::sys::testing::install_fakes;

    /// Todos los atajos de fábrica se escriben como `config.toml` y se vuelven a leer iguales.
    #[test]
    fn combo_text_roundtrip() {
        install_fakes();
        for spec in ALL {
            for k in spec.default_keys {
                let c = parse_combo(k).unwrap();
                assert_eq!(parse_combo(&combo_text(c)), Ok(c), "{k} → {}", combo_text(c));
            }
        }
        assert_eq!(combo_text(parse_combo("Ctrl+Coma").unwrap()), "Ctrl+Coma");
        assert_eq!(combo_text(parse_combo("win+ctrl+n").unwrap()), "Meta+Ctrl+N");
        assert_eq!(combo_text(parse_combo("alt+shift+ctrl+f12").unwrap()), "Ctrl+Shift+Alt+F12");
        assert_eq!(combo_text(parse_combo("Ctrl+MásNum").unwrap()), "Ctrl+MásNum");
        assert_eq!(combo_text(parse_combo("Ctrl+.").unwrap()), "Ctrl+Punto");
        assert!(!nameable(0xBA) && nameable(b'K' as u16) && nameable(VK_F1.0 + 3));
    }

    /// Cada comando tiene grupo (el `match` no tiene `_`) y los de la wiki están donde van.
    #[test]
    fn every_command_has_group() {
        assert_eq!(group(Command::Duplicate), Group::Files);
        assert_eq!(group(Command::Eject), Group::Files);
        for c in [Command::Settings, Command::SettingsKeys, Command::OpenConfig, Command::About, Command::Changelog] {
            assert_eq!(group(c), Group::Other);
        }
        assert_eq!(Group::ALL.iter().map(|g| g.title()).collect::<Vec<_>>()[6], "Red");
    }

    /// F1: los comandos de red van en el grupo «Red».
    #[test]
    fn network_commands_grouped() {
        for c in [Command::ConnectServer, Command::MapNetworkDrive, Command::DisconnectNetworkDrive] {
            assert_eq!(group(c), Group::Network);
        }
    }

    #[test]
    fn markdown_table_order() {
        install_fakes();
        let t = markdown_table(env());
        let positions: Vec<usize> = Group::ALL.iter().filter_map(|g| t.find(&format!("### {}\n", g.title()))).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
        assert!(t.contains("| Duplicar | Ctrl+D | `duplicar` |"));
        assert!(t.contains("| Configuración | Ctrl+, | `configuracion` |"));
        assert!(t.contains("| Novedades de xdir |  | `novedades` |"), "{t}");
        assert!(t.contains("### Red\n"));
        assert!(t.contains("| Conectar a un servidor… | Ctrl+K | `conectar_servidor` |"), "{t}");
    }

    #[test]
    fn keymap_combos_and_owner() {
        install_fakes();
        let mut km = Keymap::default();
        let (k, f3) = (parse_combo("Ctrl+K").unwrap(), parse_combo("F3").unwrap());
        km.set(Command::Duplicate, &[k, f3]);
        assert_eq!(km.combos(Command::Duplicate), [k, f3]);
        assert_eq!(km.owner(f3), Some(Command::Duplicate));
        assert_eq!(km.owner(parse_combo("Ctrl+T").unwrap()), Some(Command::NewTab));
        assert_eq!(km.owner(parse_combo("Ctrl+Alt+Shift+F24").unwrap()), None);
    }

    #[test]
    fn combos_parse_and_print() {
        install_fakes();
        let c = parse_combo("ctrl+shift+t").unwrap();
        assert_eq!(c, Combo { vk: b'T' as u16, ctrl: true, shift: true, alt: false, meta: false });
        assert_eq!(combo_label(c), "Ctrl+Shift+T");
        assert_eq!(combo_label(parse_combo("Alt+Izquierda").unwrap()), "Alt+←");
        // Estilo Apple: símbolos en el orden de Apple; Ctrl es ⌘ y Alt es ⌥.
        assert_eq!(combo_label_in(KeyStyle::Mac, c), "⇧⌘T");
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Alt+Izquierda").unwrap()), "⌥←");
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Retroceso").unwrap()), "⌘⌫");
        assert_eq!(combo_label(parse_combo("F12").unwrap()), "F12");
        assert_eq!(parse_combo("Mayús+Supr").unwrap().vk, VK_DELETE.0);
        assert!(parse_combo("Ctrl+Hiper").is_err());
        assert!(parse_combo("Hiper+X").unwrap_err().contains("(usá Ctrl, Shift, Alt o Meta)"));
        assert!(parse_combo("Meta+X").unwrap().meta);
    }

    /// Meta: la tecla Control en macOS (⌃, primero) y Windows/Super en los demás («Win+»).
    #[test]
    fn meta_combos_parse_and_print() {
        install_fakes();
        let c = parse_combo("Meta+Ctrl+N").unwrap();
        assert_eq!(c, Combo { vk: b'N' as u16, ctrl: true, shift: false, alt: false, meta: true });
        assert_eq!(parse_combo("win+ctrl+n").unwrap(), c);
        assert_eq!(parse_combo("SUPER+Ctrl+N").unwrap(), c);
        assert_eq!(combo_label_in(KeyStyle::Mac, c), "⌃⌘N");
        assert_eq!(combo_label_in(KeyStyle::Windows, c), "Win+Ctrl+N");
        let all = parse_combo("Meta+Ctrl+Shift+Alt+K").unwrap();
        assert_eq!(combo_label_in(KeyStyle::Mac, all), "⌃⌥⇧⌘K");
        assert_eq!(combo_label_in(KeyStyle::Windows, all), "Win+Ctrl+Shift+Alt+K");
    }

    /// Ctrl+N y Meta+Ctrl+N son atajos distintos.
    #[test]
    fn meta_is_part_of_lookup() {
        install_fakes();
        let mut km = Keymap::default();
        km.set(Command::NewTab, &[parse_combo("Ctrl+N").unwrap()]);
        km.set(Command::NewFolderWithSelection, &[parse_combo("Meta+Ctrl+N").unwrap()]);
        let n = Key(b'N' as u16);
        assert_eq!(km.lookup(n, true, false, false, false), Some(Command::NewTab));
        assert_eq!(km.lookup(n, true, false, false, true), Some(Command::NewFolderWithSelection));
    }

    /// «Carpeta nueva con la selección»: Ctrl+Alt+Shift+N, después de «Archivo de texto nuevo».
    #[test]
    fn new_folder_with_selection_shortcut() {
        install_fakes();
        let s = ALL.iter().find(|s| s.id == "carpeta_con_seleccion").unwrap();
        assert_eq!(
            (s.command, s.name, s.default_keys),
            (Command::NewFolderWithSelection, "Carpeta nueva con la selección", &["Ctrl+Alt+Shift+N"][..])
        );
        let pos = |id: &str| ALL.iter().position(|s| s.id == id).unwrap();
        assert_eq!(pos("archivo_texto_nuevo") + 1, pos("carpeta_con_seleccion"));
        let km = Keymap::default();
        assert_eq!(km.lookup(Key(b'N' as u16), true, true, true, false), Some(Command::NewFolderWithSelection));
        assert_eq!(km.label(Command::NewFolderWithSelection), "Ctrl+Shift+Alt+N");
    }

    /// Duplicar toma Ctrl+D (⌘D) y favoritos pasa a Ctrl+Shift+D (⇧⌘D).
    #[test]
    fn duplicate_command_and_favorite_moved() {
        install_fakes();
        let spec = |id: &str| ALL.iter().find(|s| s.id == id).unwrap();
        let d = spec("duplicar");
        assert_eq!((d.command, d.name, d.default_keys), (Command::Duplicate, "Duplicar", &["Ctrl+D"][..]));
        assert_eq!(spec("favorito").default_keys, &["Ctrl+Shift+D"][..]);
        let pos = |id: &str| ALL.iter().position(|s| s.id == id).unwrap();
        assert_eq!(pos("duplicar") + 1, pos("favorito"));
        let km = Keymap::default();
        let (cd, csd) = (parse_combo("Ctrl+D").unwrap(), parse_combo("Ctrl+Shift+D").unwrap());
        assert_eq!(km.lookup(Key(cd.vk), cd.ctrl, cd.shift, cd.alt, cd.meta), Some(Command::Duplicate));
        assert_eq!(km.lookup(Key(csd.vk), csd.ctrl, csd.shift, csd.alt, csd.meta), Some(Command::ToggleFavorite));
        assert_eq!(combo_label_in(KeyStyle::Mac, cd), "⌘D");
        assert_eq!(combo_label_in(KeyStyle::Mac, csd), "⇧⌘D");
    }

    /// El árbol de carpetas solo existe compilando con la feature `arbol`; las demás acciones no
    /// dependen de ella.
    #[test]
    fn toggle_tree_depends_on_the_feature() {
        use crate::sys::testing::FakeEnv;
        assert_eq!(available_in(&FakeEnv, Command::ToggleTree), cfg!(feature = "arbol"));
        assert!(available_in(&FakeEnv, Command::ToggleTreePin));
        assert!(available_in(&FakeEnv, Command::ToggleQuickPin));
    }

    /// Cada atajo de fábrica resuelve a su propia acción, y no hay dos acciones con la misma tecla.
    #[test]
    fn defaults_are_consistent() {
        install_fakes();
        let km = Keymap::default();
        let mut seen = std::collections::HashSet::new();
        for s in ALL.iter().filter(|s| available(s.command)) {
            for k in s.keys() {
                let c = parse_combo(k).unwrap();
                // Cada ámbito (la ventana, xtree) tiene sus propias teclas.
                assert!(seen.insert((xtree_only(s.command), c)), "atajo repetido: {k}");
                let found = if xtree_only(s.command) { km.lookup_xtree(Key(c.vk), c.ctrl, c.shift, c.alt, c.meta) } else { km.lookup(Key(c.vk), c.ctrl, c.shift, c.alt, c.meta) };
                assert_eq!(found, Some(s.command), "{k}");
            }
        }
        let ids: std::collections::HashSet<_> = ALL.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), ALL.len(), "identificadores repetidos");
    }

    /// El árbol solo existe con la feature `arbol`: sin ella, ni su atajo queda tomado.
    #[test]
    fn experimental_tree_depends_on_feature() {
        install_fakes();
        let km = Keymap::default();
        let e = parse_combo("Ctrl+Shift+E").unwrap();
        let bound = km.lookup(Key(e.vk), e.ctrl, e.shift, e.alt, e.meta);
        assert_eq!(available(Command::ToggleTree), cfg!(feature = "arbol"));
        assert_eq!(bound, cfg!(feature = "arbol").then_some(Command::ToggleTree));
    }

    #[test]
    fn overrides() {
        install_fakes();
        let mut km = Keymap::default();
        let ctrl_k = parse_combo("Ctrl+K").unwrap();
        let ctrl_w = parse_combo("Ctrl+W").unwrap();
        // pestana_nueva = ["Ctrl+K", "Ctrl+W"]: Ctrl+T deja de funcionar, y Ctrl+K y Ctrl+W se les
        // sacan a conectar a un servidor y a cerrar.
        let displaced = km.set(Command::NewTab, &[ctrl_k, ctrl_w]);
        assert_eq!(displaced, [Command::ConnectServer, Command::CloseTab]);
        assert_eq!(km.lookup(Key(b'K' as u16), true, false, false, false), Some(Command::NewTab));
        assert_eq!(km.lookup(Key(b'W' as u16), true, false, false, false), Some(Command::NewTab));
        assert_eq!(km.lookup(Key(b'T' as u16), true, false, false, false), None);
        assert_eq!(km.label(Command::CloseTab), "");
        km.set(Command::NewTab, &[]);
        assert_eq!(km.label(Command::NewTab), "");
    }

    /// Selección por patrón: Ctrl+Más / Ctrl+Menos (también los del teclado numérico), que en
    /// macOS se ven ⌘+ / ⌘-.
    #[test]
    fn pattern_commands_have_keys() {
        install_fakes();
        let km = Keymap::default();
        for (text, command) in [
            ("Ctrl+Más", Command::SelectPattern),
            ("Ctrl+MásNum", Command::SelectPattern),
            ("Ctrl+Menos", Command::DeselectPattern),
            ("Ctrl+MenosNum", Command::DeselectPattern),
        ] {
            let c = parse_combo(text).unwrap();
            assert_eq!(km.lookup(Key(c.vk), c.ctrl, c.shift, c.alt, c.meta), Some(command), "{text}");
        }
        assert_eq!(km.label(Command::SelectPattern), "Ctrl+Más");
        assert_eq!(km.label(Command::DeselectPattern), "Ctrl+Menos");
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Más").unwrap()), "⌘+");
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Menos").unwrap()), "⌘-");
        let spec = |c| ALL.iter().find(|s| s.command == c).unwrap();
        assert_eq!((spec(Command::SelectPattern).id, spec(Command::SelectPattern).name), ("seleccionar_patron", "Seleccionar por patrón…"));
        assert_eq!((spec(Command::DeselectPattern).id, spec(Command::DeselectPattern).name), ("deseleccionar_patron", "Deseleccionar por patrón…"));
    }

    /// La columna de vista previa: `vista_previa`, Alt+P (en macOS ⌥⌘P, porque ⇧⌘P es la paleta;
    /// lo prueba la tabla de macOS), sin chocar con otro atajo.
    #[test]
    fn toggle_preview_command_and_shortcuts() {
        install_fakes();
        let s = ALL.iter().find(|s| s.id == "vista_previa").unwrap();
        assert_eq!((s.command, s.name, s.default_keys), (Command::TogglePreview, "Mostrar u ocultar la vista previa", &["Alt+P"][..]));
        let km = Keymap::default();
        let c = parse_combo("Alt+P").unwrap();
        assert_eq!(km.lookup(Key(c.vk), c.ctrl, c.shift, c.alt, c.meta), Some(Command::TogglePreview));
        assert_eq!(km.label(Command::TogglePreview), "Alt+P");
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Alt+P").unwrap()), "⌥⌘P");
        let p = parse_combo("Ctrl+Shift+P").unwrap();
        assert_eq!(km.lookup(Key(p.vk), p.ctrl, p.shift, p.alt, p.meta), Some(Command::Palette));
    }

    /// Vistas Lista y Contenido: `Command::View(4)` y `(5)` (índices de `list::VIEWS`), con sus
    /// ids, títulos y atajos Ctrl+Shift+5 / Ctrl+Shift+6 (⇧⌘5 / ⇧⌘6 en macOS).
    /// #57: «xtree_colores» (Ctrl+T) solo vale dentro de xtree: no choca con «Pestaña nueva», que
    /// sigue siendo Ctrl+T en los paneles; cambiar uno no le quita la tecla al otro.
    #[test]
    fn xtree_colors_command_is_scoped() {
        install_fakes();
        let s = ALL.iter().find(|s| s.id == "xtree_colores").unwrap();
        assert_eq!((s.command, s.default_keys), (Command::XtreeColors, &["Ctrl+T"][..]));
        assert!(xtree_only(Command::XtreeColors) && !xtree_only(Command::NewTab));
        assert_eq!(group(Command::XtreeColors), Group::Xtree);
        let mut km = Keymap::default();
        let t = Key(b'T' as u16);
        assert_eq!(km.lookup(t, true, false, false, false), Some(Command::NewTab));
        assert_eq!(km.lookup_xtree(t, true, false, false, false), Some(Command::XtreeColors));
        assert_eq!(km.lookup_xtree(Key(b'W' as u16), true, false, false, false), None, "solo los de xtree");
        let ct = parse_combo("Ctrl+T").unwrap();
        assert_eq!(km.owner(ct), Some(Command::NewTab));
        assert_eq!(km.owner_for(ct, Command::XtreeColors), Some(Command::XtreeColors));
        assert_eq!(km.owner_for(ct, Command::Duplicate), Some(Command::NewTab));
        // Ctrl+K para los colores no le quita Ctrl+K a «Conectar a un servidor».
        let k = parse_combo("Ctrl+K").unwrap();
        assert_eq!(km.set(Command::XtreeColors, &[k]), Vec::<Command>::new());
        assert_eq!(km.lookup(Key(b'K' as u16), true, false, false, false), Some(Command::ConnectServer));
        assert_eq!(km.lookup_xtree(Key(b'K' as u16), true, false, false, false), Some(Command::XtreeColors));
        assert_eq!(km.label(Command::XtreeColors), "Ctrl+K");
        // Y un comando de la ventana con Ctrl+K desplaza solo al de la ventana.
        assert_eq!(km.set(Command::Duplicate, &[k]), vec![Command::ConnectServer]);
        assert_eq!(km.lookup_xtree(Key(b'K' as u16), true, false, false, false), Some(Command::XtreeColors));
        assert!(markdown_table(env()).contains("### xtree\n"));
    }

    /// «xtree_permisos» (Ctrl+Shift+A): analizar con permisos, solo dentro de xtree (la A sola es
    /// del salto por letra).
    #[test]
    fn xtree_elevate_command_is_scoped() {
        install_fakes();
        let s = ALL.iter().find(|s| s.id == "xtree_permisos").unwrap();
        assert_eq!((s.command, s.default_keys), (Command::XtreeElevate, &["Ctrl+Shift+A"][..]));
        assert!(xtree_only(Command::XtreeElevate));
        assert_eq!(group(Command::XtreeElevate), Group::Xtree);
        let km = Keymap::default();
        let a = Key(b'A' as u16);
        assert_eq!(km.lookup_xtree(a, true, true, false, false), Some(Command::XtreeElevate));
        assert_eq!(km.lookup_xtree(a, false, false, false, false), None);
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Shift+A").unwrap()), "⇧⌘A");
    }

    #[test]
    fn view_shortcuts() {
        install_fakes();
        let km = Keymap::default();
        let spec = |id: &str| ALL.iter().find(|s| s.id == id).unwrap();
        for (id, i, title, keys, view) in [
            ("vista_lista", 4u8, "Vista: lista", "Ctrl+Shift+5", crate::list::View::List),
            ("vista_contenido", 5, "Vista: contenido", "Ctrl+Shift+6", crate::list::View::Content),
        ] {
            let s = spec(id);
            assert_eq!((s.command, s.name, s.default_keys), (Command::View(i), title, &[keys][..]));
            assert_eq!(crate::list::VIEWS[i as usize].0, view);
            let c = parse_combo(keys).unwrap();
            assert_eq!(km.lookup(Key(c.vk), c.ctrl, c.shift, c.alt, c.meta), Some(Command::View(i)), "{keys}");
        }
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Shift+5").unwrap()), "⇧⌘5");
    }

    /// Galería: `Command::View(6)` (índice de `list::VIEWS`), «Vista: galería», Ctrl+Shift+7
    /// (⌥⌘7 en macOS, en la tabla de la Mac).
    #[test]
    fn gallery_shortcuts() {
        install_fakes();
        let km = Keymap::default();
        let s = ALL.iter().find(|s| s.id == "vista_galeria").unwrap();
        assert_eq!((s.command, s.name, s.default_keys), (Command::View(6), "Vista: galería", &["Ctrl+Shift+7"][..]));
        assert_eq!(crate::list::VIEWS[6].0, crate::list::View::Gallery);
        let c = parse_combo("Ctrl+Shift+7").unwrap();
        assert_eq!(km.lookup(Key(c.vk), c.ctrl, c.shift, c.alt, c.meta), Some(Command::View(6)));
        assert_eq!(combo_label_in(KeyStyle::Mac, parse_combo("Ctrl+Alt+7").unwrap()), "⌥⌘7");
    }
}

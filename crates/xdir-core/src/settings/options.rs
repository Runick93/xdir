//! Tabla de opciones de la configuración: de ella salen las filas de cada sección (en este orden)
//! y el buscador.

use super::Section;

/// Una opción: su id (también el de su fila), sección, título, descripción corta y palabras
/// clave para el buscador.
#[derive(Debug)]
pub struct OptionSpec {
    pub id: &'static str,
    pub section: Section,
    pub title: &'static str,
    pub desc: &'static str,
    pub keywords: &'static [&'static str],
}

const fn opt(id: &'static str, section: Section, title: &'static str, desc: &'static str, keywords: &'static [&'static str]) -> OptionSpec {
    OptionSpec { id, section, title, desc, keywords }
}

pub const OPTIONS: &[OptionSpec] = &[
    opt("tema", Section::Appearance, "Tema", "Claro, oscuro o el del sistema", &["oscuro", "claro", "modo", "noche", "dia", "apariencia"]),
    opt("acento", Section::Appearance, "Color de acento", "Se usa en la selección, los bordes y los botones", &["color", "acento", "resaltado", "seleccion"]),
    opt("animaciones", Section::Appearance, "Animaciones", "Desplazamiento suave y transición al cambiar de layout", &["animacion", "movimiento", "suave", "efectos"]),
    opt("extensiones", Section::ListViews, "Mostrar extensiones", "Los nombres de archivo con su extensión (.pdf, .txt)", &["extension", "nombre", "archivo", "mostrar"]),
    opt("ocultos", Section::ListViews, "Mostrar archivos ocultos", "Lo mismo que Ctrl+H en la lista", &["ocultos", "escondidos", "punto", "mostrar", "archivos"]),
    opt("casillas", Section::ListViews, "Casillas de selección", "Una casilla en cada fila para elegir varios", &["casilla", "checkbox", "seleccion", "marcar"]),
    opt("barra_grupos", Section::ListViews, "Barra de grupos", "Las pestañas de grupos de paneles arriba de los paneles", &["grupos", "barra", "pestañas", "carpeta"]),
    opt("colores_tipo", Section::ListViews, "Colores por tipo", "El nombre en color según el tipo de archivo", &["color", "tipo", "archivo", "reglas", "colores"]),
    opt("vista_defecto", Section::ListViews, "Vista por defecto", "La de las carpetas que no tienen una propia", &["vista", "detalles", "iconos", "miniaturas", "lista", "carpeta"]),
    opt("vistas_carpetas", Section::ListViews, "Vistas de las carpetas", "Cada carpeta recuerda su vista; acá se igualan todas", &["vista", "carpeta", "restablecer", "todas"]),
    opt("arbol", Section::Tree, "Mostrar el árbol de carpetas", "La columna con las carpetas a la izquierda de los paneles", &["arbol", "carpetas", "lateral", "mostrar"]),
    opt("arbol_fijo", Section::Tree, "Fijo al lado de los paneles", "Si no, queda una franja de íconos que se abre al pasar el mouse", &["arbol", "fijo", "franja", "chincheta"]),
    opt("arbol_sigue", Section::Tree, "El árbol sigue al panel", "Despliega y muestra la carpeta del panel activo", &["arbol", "seguir", "panel", "carpeta"]),
    opt("arbol_config", Section::Tree, "De fábrica: lo que diga arbol_sigue_panel en config.toml", "Una vez tocado el switch, manda el switch", &["arbol", "config", "fabrica"]),
    opt("vista_previa", Section::Preview, "Mostrar la vista previa", "La columna a la derecha de los paneles (Alt+P)", &["vista", "previa", "columna", "mostrar"]),
    opt("vista_previa_ancho", Section::Preview, "Ancho", "De 240 a 640 píxeles", &["ancho", "tamaño", "columna"]),
    opt("xtree_por_tipo", Section::Xtree, "Colorear el mapa por tipo de archivo al abrir", "Lo mismo que la tecla T, desde que se abre", &["xtree", "color", "tipo", "mapa"]),
    opt("xtree_abrir", Section::Xtree, "Disco que se analiza al abrir", "El del sistema, el de la carpeta del panel o el último", &["xtree", "disco", "analizar", "carpeta"]),
    opt("xtree_teclas", Section::Xtree, "Teclas de xtree", "Solo para ver: se cambian en Atajos de teclado", &["xtree", "teclas", "atajos"]),
    opt("abrir_config", Section::Advanced, "config.toml", "Atajos, colores y opciones escritos a mano", &["config", "toml", "abrir", "editor", "archivo"]),
    opt("carpeta_config", Section::Advanced, "Carpeta de configuración", "Donde xdir guarda sus archivos", &["carpeta", "configuracion", "archivos", "mostrar"]),
    opt("archivos", Section::Advanced, "Dónde están los archivos", "Qué guarda cada archivo de la carpeta", &["archivos", "sesion", "favoritos", "grupos", "historial"]),
    opt("portable", Section::Advanced, "Modo portable", "Los archivos junto a xdir en vez de en la carpeta del usuario", &["portable", "usb", "carpeta"]),
];

/// Las opciones de una sección, en orden.
pub fn of(section: Section) -> impl Iterator<Item = &'static OptionSpec> {
    OPTIONS.iter().filter(move |o| o.section == section)
}

/// La opción con ese id.
pub fn get(id: &str) -> Option<&'static OptionSpec> {
    OPTIONS.iter().find(|o| o.id == id)
}

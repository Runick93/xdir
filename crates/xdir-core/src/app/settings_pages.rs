//! Páginas de la configuración que escriben otros módulos (Red): su lugar en la lista, su
//! búsqueda, su dibujo y sus clics.

use super::App;
use crate::geom::Rect;
use crate::host::TextBox;
use crate::paint::Painter;
use crate::settings::{SearchItem, Section};

/// Dibuja el contenido de una página extra: (app, pintor, área desplazada, mouse) → alto.
pub(super) type DrawPage = fn(&mut App, &dyn Painter, Rect, Option<(f32, f32)>) -> f32;

/// Una página extra de la configuración.
pub(super) struct ExtraPage {
    /// Id de la página (también el ancla de la sesión: `xdir:configuracion#red`).
    pub id: &'static str,
    /// Nombre en la lista de la izquierda y en las migas («Red»).
    pub title: &'static str,
    /// Va después de esta sección en la lista.
    pub after: Section,
    /// Lo que se puede buscar en la página.
    pub search: fn(&App) -> Vec<SearchItem>,
    /// Dibuja el contenido en el área dada (ya desplazada) con el mouse en esa posición, si está
    /// encima. Devuelve el alto del contenido.
    pub draw: DrawPage,
    /// Un clic en el contenido; `true` si lo atendió.
    pub click: fn(&mut App, f32, f32) -> bool,
    /// Terminó la edición de un campo propio de la página (`true`: con Enter); `true` si era suyo.
    pub field_done: fn(&mut App, TextBox, bool) -> bool,
    /// Tab en un campo propio (`true`: con Shift, hacia atrás); `true` si era suyo.
    pub field_tab: fn(&mut App, TextBox, bool) -> bool,
}

// Contrato de los campos de una página extra: la página crea la caja con
// `host.text_create(TextKind::Field, …)` y la anota en `App::settings_field`; `field_done` y
// `field_tab` reciben esa caja. Si la configuración deja de verse, la `App` la destruye y deja
// `settings_field` en `None` (la página no tiene que hacerlo). `draw` solo dibuja: no abre cajas
// ni lanza trabajos.

/// Las páginas extra, en el orden en que aparecen después de su sección.
pub(super) const PAGES: &[ExtraPage] = &[];

//! Dibujo de la columna de vista previa (diseño `preview::draw` y `preview::draw_details`): arriba
//! la vista previa (imagen grande, primeras líneas de texto, ícono de carpeta o del tipo), abajo
//! el nombre, el tipo y los detalles. Es neutro: lo que se muestra llega armado en `Shown`.

use std::sync::Arc;

use super::{PreviewBody, PreviewInfo};
use crate::fs::{Entry, Meta};
use crate::geom::{Color, Rect, rect};
use crate::list::meta::duration_text;
use crate::paint::{FontId, Painter};
use crate::sys::SystemEnv;
use crate::thumbs;

/// Margen de la columna.
pub const MARGIN: f32 = 16.0;
/// Alto del recuadro de texto.
pub const TEXT_H: f32 = 300.0;
/// Separación entre la vista previa y los detalles.
const DETAILS_GAP: f32 = 20.0;
/// Alto de cada fila de detalles, y dónde empieza el valor.
pub const ROW_H: f32 = 22.0;
pub const VALUE_X: f32 = 104.0;
/// Lado del ícono grande (carpetas, sin vista previa, cargando).
const ICON: f32 = 96.0;
/// Líneas de texto: paso y alto.
const LINE_STEP: f32 = 17.0;
const LINE_H: f32 = 14.0;
/// Líneas que puede ocupar el nombre.
const NAME_LINES: usize = 3;
const NAME_LINE_H: f32 = 20.0;

pub const CLOUD_TEXT: &str = "Disponible solo en línea";
pub const UNREADABLE_TEXT: &str = "No se puede leer este archivo";

/// Lo que muestra la columna.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    /// Un elemento (o la carpeta abierta, o el disco enfocado en «Este equipo»). `key`: de dónde
    /// sale la imagen grande (el lado lo pone `draw` según el ancho). `info: None`: cargando (se
    /// dibuja el ícono del tipo). `rows`: los detalles; el de «Tipo» va debajo del nombre.
    Item { name: String, key: thumbs::Key, icon: i32, info: Option<PreviewInfo>, rows: Vec<(&'static str, String)>, tags: Vec<String> },
    /// Varios seleccionados: cuántos, hasta 3 miniaturas (con su ícono por si no están), el
    /// tamaño total y el tipo si es el mismo para todos.
    Many { count: usize, keys: Vec<thumbs::Key>, icons: Vec<i32>, bytes: u64, kind: Option<Arc<str>> },
    Empty,
}

/// Filas de detalles de un elemento: Tipo, Tamaño (no en carpetas), Creado, Modificado, Último
/// acceso y los metadatos que tengan valor (Dimensiones, Duración, Autor, Artista, Álbum; las
/// etiquetas van aparte, en píldoras). Sin datos, la fila no aparece.
pub fn detail_rows(e: &Entry, meta: Option<&Meta>, accessed: u64, env: &dyn SystemEnv) -> Vec<(&'static str, String)> {
    let mut rows = vec![("Tipo", e.type_name.to_string())];
    if !e.is_dir {
        rows.push(("Tamaño", env.format_size_short(e.size)));
    }
    for (label, ft) in [("Creado", e.created), ("Modificado", e.modified), ("Último acceso", accessed)] {
        if ft != 0 {
            rows.push((label, env.format_date(ft)));
        }
    }
    if let Some(m) = meta {
        if let (Some(w), Some(h)) = (m.width, m.height) {
            rows.push(("Dimensiones", format!("{w} × {h}")));
        }
        if let Some(ms) = m.duration_ms {
            rows.push(("Duración", duration_text(ms)));
        }
        if !m.authors.is_empty() {
            rows.push(("Autor", m.authors.join("; ")));
        }
        if let Some(a) = m.artist.as_ref().filter(|a| !a.is_empty()) {
            rows.push(("Artista", a.clone()));
        }
        if let Some(a) = m.album.as_ref().filter(|a| !a.is_empty()) {
            rows.push(("Álbum", a.clone()));
        }
    }
    rows.retain(|(_, v)| !v.is_empty());
    rows
}

/// «1 elemento» / «N elementos».
pub fn items_text(n: u64) -> String {
    if n == 1 { "1 elemento".to_string() } else { format!("{} elementos", crate::format::thousands(n)) }
}

/// Alto de la vista previa de una columna de ancho interior `inner` (4:3).
fn image_h(inner: f32) -> f32 {
    inner * 0.75
}

/// Dibuja la columna en `r` (fondo, borde, vista previa y detalles).
pub fn draw(p: &dyn Painter, r: Rect, s: &Shown) {
    let th = p.theme();
    p.round_fill(r, 8.0, th.surface);
    p.round_stroke(r, 8.0, th.border);
    let (left, right) = (r.left + MARGIN, r.right - MARGIN);
    if right - left < 40.0 {
        return;
    }
    let inner = right - left;
    let top = r.top + MARGIN;
    let ih = image_h(inner);
    p.clip(r, &mut || match s {
        Shown::Empty => {}
        Shown::Item { name, key, icon, info, rows, tags } => {
            let body = info.as_ref().map(|i| &i.body);
            let ph = match body {
                Some(PreviewBody::Text { .. }) => TEXT_H,
                _ => ih,
            };
            let area = rect(left, top, right, top + ph);
            let caption = match body {
                Some(PreviewBody::Folder { items }) => Some(folder_caption(rows, *items)),
                Some(PreviewBody::CloudOnly) => Some(CLOUD_TEXT.to_string()),
                _ => None,
            };
            draw_body(p, area, key, *icon, body, caption.as_deref());
            let subtitle = rows.iter().find(|(l, _)| *l == "Tipo").map_or("", |(_, v)| v.as_str());
            let rest: Vec<(&'static str, String)> = rows.iter().filter(|(l, _)| *l != "Tipo").cloned().collect();
            draw_head(p, rect(left, area.bottom + DETAILS_GAP, right, r.bottom - MARGIN), name, subtitle, &rest, tags);
        }
        Shown::Many { count, keys, icons, bytes, kind } => {
            draw_fan(p, rect(left, top, right, top + ih), keys, icons, *count);
            let size = crate::format::size_short(*bytes);
            let kb = crate::format::size_kb(*bytes);
            let subtitle = match kind {
                Some(k) => format!("{size} · {k}"),
                None => size.clone(),
            };
            // El detalle en KB solo si dice algo más que el tamaño corto.
            let mut rows = vec![("Tamaño", if kb == size { size.clone() } else { format!("{size} ({kb})") })];
            if let Some(k) = kind {
                rows.push(("Tipo", k.to_string()));
            }
            let title = format!("{} elementos seleccionados", crate::format::thousands(*count as u64));
            draw_head(p, rect(left, top + ih + DETAILS_GAP, right, r.bottom - MARGIN), &title, &subtitle, &rows, &[]);
        }
    });
}

/// La vista previa de un elemento en `area`.
fn draw_body(p: &dyn Painter, area: Rect, key: &thumbs::Key, icon: i32, body: Option<&PreviewBody>, caption: Option<&str>) {
    let th = p.theme();
    let (cx, w) = ((area.left + area.right) / 2.0, area.right - area.left);
    // Ícono grande con una leyenda debajo (o solo el ícono), centrados en el área.
    let icon_with = |caption: Option<&str>, cloud: bool| {
        let block = if caption.is_some() { ICON + 18.0 + 16.0 } else { ICON };
        let shift = if caption.is_some() { -6.0 } else { -4.0 };
        let y = (area.top + (area.bottom - area.top - block) / 2.0 + shift).max(area.top);
        p.shell_icon(icon, 0, cx - ICON / 2.0, y, ICON);
        if cloud {
            draw_cloud(p, cx + 70.0 - ICON / 2.0, y + 72.0, th.text_dim);
        }
        if let Some(c) = caption {
            p.text(FontId::UiCenter, c, rect(area.left, y + ICON + 18.0, area.right, y + ICON + 34.0), th.text_dim);
        }
    };
    match body {
        Some(PreviewBody::Image) => {
            let side = thumbs::big_side(p.thumb_px(w.max(area.bottom - area.top)));
            if !p.big_image(&(key.0.clone(), side), area) {
                icon_with(None, false);
            }
        }
        Some(PreviewBody::Text { lines }) => {
            p.round_fill(area, 6.0, th.background);
            p.round_stroke(area, 6.0, th.border);
            let inner = rect(area.left + 1.0, area.top + 1.0, area.right - 1.0, area.bottom - 1.0);
            p.clip(inner, &mut || {
                let mut y = area.top + 8.0;
                for line in lines {
                    if y + LINE_H > area.bottom - 4.0 {
                        break;
                    }
                    if !line.is_empty() {
                        p.text(FontId::Mono, line, rect(area.left + 10.0, y, area.right - 6.0, y + LINE_H), th.text);
                    }
                    y += LINE_STEP;
                }
            });
        }
        // La carpeta lleva «N elementos · tamaño»; lo que está solo en la nube, la nube y su leyenda.
        Some(PreviewBody::Folder { .. }) => icon_with(caption, false),
        Some(PreviewBody::CloudOnly) => icon_with(caption, true),
        Some(PreviewBody::Drive { .. } | PreviewBody::Unreadable | PreviewBody::Nothing) | None => icon_with(None, false),
    }
}

/// Leyenda de una carpeta: «N elementos · tamaño» (el tamaño, el de xtree, sale de los detalles).
fn folder_caption(rows: &[(&'static str, String)], items: u64) -> String {
    match rows.iter().find(|(l, _)| *l == "Tamaño") {
        Some((_, size)) => format!("{} · {size}", items_text(items)),
        None => items_text(items),
    }
}

/// Nombre (en varias líneas si hace falta), tipo, línea y los detalles, desde `r.top`.
fn draw_head(p: &dyn Painter, r: Rect, name: &str, subtitle: &str, rows: &[(&'static str, String)], tags: &[String]) {
    let th = p.theme();
    let lines = wrap(p, FontId::Title, name, r.right - r.left, NAME_LINES);
    let mut y = r.top;
    for line in &lines {
        p.text(FontId::Title, line, rect(r.left, y, r.right, y + 19.0), th.text);
        y += NAME_LINE_H;
    }
    let y = r.top + (lines.len().max(1) as f32 - 1.0) * NAME_LINE_H;
    if !subtitle.is_empty() {
        p.text(FontId::Ui, subtitle, rect(r.left, y + 26.0, r.right, y + 42.0), th.text_dim);
    }
    p.fill(rect(r.left, y + 54.0, r.right, y + 55.0), th.border);
    draw_details(p, rect(r.left, y + 66.0, r.right, r.bottom), rows, tags);
}

/// Bloque de detalles: «Información» y una fila por dato (rótulo a la izquierda, valor en
/// `VALUE_X`), y las etiquetas en píldoras.
pub fn draw_details(p: &dyn Painter, r: Rect, rows: &[(&'static str, String)], tags: &[String]) {
    let th = p.theme();
    p.text(FontId::SmallBold, "Información", rect(r.left, r.top, r.right, r.top + 14.0), th.text_secondary);
    let mut y = r.top + 24.0;
    let vx = r.left + VALUE_X;
    for (label, value) in rows {
        p.text(FontId::Ui, label, rect(r.left, y, vx - 4.0, y + 16.0), th.text_dim);
        let mut x = vx;
        if *label == "Estado" && value == CLOUD_TEXT {
            draw_cloud(p, x, y - 1.0, th.text_dim);
            x += 32.0;
        }
        p.text(FontId::Ui, value, rect(x, y, r.right, y + 16.0), th.text);
        y += ROW_H;
    }
    if !tags.is_empty() {
        p.text(FontId::Ui, "Etiquetas", rect(r.left, y, vx - 4.0, y + 16.0), th.text_dim);
        let (mut x, mut ty) = (vx, y - 2.0);
        for tag in tags {
            let tw = p.measure(FontId::Small, tag) + 16.0;
            if x + tw > r.right && x > vx {
                x = vx;
                ty += 24.0;
            }
            let pill = rect(x, ty, (x + tw).min(r.right), ty + 20.0);
            p.round_fill(pill, 10.0, th.surface_alt);
            p.round_stroke(pill, 10.0, th.border);
            p.text(FontId::Small, tag, rect(pill.left + 8.0, ty + 3.0, pill.right - 8.0, ty + 17.0), th.text_secondary);
            x += tw + 6.0;
        }
    }
}

/// Tres tarjetas en abanico con las miniaturas de los seleccionados y la cantidad en acento.
fn draw_fan(p: &dyn Painter, area: Rect, keys: &[thumbs::Key], icons: &[i32], count: usize) {
    let th = p.theme();
    let cx = (area.left + area.right) / 2.0;
    let top = area.top - MARGIN;
    // Atrás a la izquierda, atrás a la derecha (más chicas, corridas para que se vea parte de su
    // miniatura) y adelante (diseño: 150×112).
    let spots = [(cx - 128.0, top + 92.0, 128.0, 96.0), (cx, top + 52.0, 128.0, 96.0), (cx - 75.0, top + 68.0, 150.0, 112.0)];
    let n = keys.len().max(icons.len()).clamp(1, 3);
    for (i, &(x, y, cw, ch)) in spots.iter().enumerate().skip(3 - n) {
        let k = i + n - 3;
        let card = rect(x, y, x + cw, y + ch);
        p.round_fill(card, 6.0, th.surface_alt);
        p.round_stroke(card, 6.0, th.border);
        let inner = rect(card.left + 4.0, card.top + 4.0, card.right - 4.0, card.bottom - 4.0);
        let shown = keys.get(k).is_some_and(|key| p.thumbnail(&(key.0.clone(), p.thumb_px(inner.right - inner.left)), inner));
        if !shown && let Some(&icon) = icons.get(k) {
            p.shell_icon(icon, 0, (inner.left + inner.right) / 2.0 - 24.0, (inner.top + inner.bottom) / 2.0 - 24.0, 48.0);
        }
    }
    let (bx, by) = (cx + 84.0, top + 66.0);
    p.circle(bx, by, 12.0, th.accent);
    let white = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    let label = if count > 99 { "99+".to_string() } else { count.to_string() };
    p.text(FontId::UiCenter, &label, rect(bx - 14.0, by - 8.0, bx + 14.0, by + 8.0), white);
}

/// Nube de 28×18 con la esquina de arriba a la izquierda en `(x, y)`.
fn draw_cloud(p: &dyn Painter, x: f32, y: f32, c: Color) {
    p.circle(x + 8.0, y + 12.0, 6.0, c);
    p.circle(x + 15.0, y + 7.0, 7.0, c);
    p.circle(x + 21.0, y + 11.0, 6.0, c);
    p.fill(rect(x + 8.0, y + 10.0, x + 22.0, y + 18.0), c);
}

/// Parte `s` en hasta `max` líneas que entran en `width` (por caracteres); la última lleva el
/// resto (la fuente le pone «…» si no entra).
fn wrap(p: &dyn Painter, font: FontId, s: &str, width: f32, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        if lines.len() + 1 == max || p.measure(font, rest) <= width {
            lines.push(rest.to_string());
            break;
        }
        // El corte más largo que entra (al menos un carácter), por búsqueda binaria entre los
        // límites de carácter; mejor después de un espacio, punto o guion.
        let bounds: Vec<usize> = rest.char_indices().skip(1).map(|(i, _)| i).collect();
        let fit = bounds.partition_point(|&i| p.measure(font, &rest[..i]) <= width);
        let mut cut = if fit == 0 { bounds.first().copied().unwrap_or(rest.len()) } else { bounds[fit - 1] };
        if let Some(b) = rest[..cut].rfind([' ', '-', '_', '.']).filter(|&b| b > cut / 2) {
            cut = b + 1;
        }
        lines.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::testing::{FakeEnv, RecordingPainter, install_fakes};
    use crate::theme::{DEFAULT_ACCENT, Theme};
    use std::path::PathBuf;

    fn painter() -> RecordingPainter {
        install_fakes();
        RecordingPainter::new(Theme::from_system(DEFAULT_ACCENT, Some(true)))
    }

    /// Columna de 320 (como en el diseño) a partir de (1000, 3).
    fn column() -> Rect {
        rect(1000.0, 3.0, 1320.0, 790.0)
    }

    fn key(name: &str) -> thumbs::Key {
        (thumbs::Source::Path(PathBuf::from(name)), 0)
    }

    fn item(body: Option<PreviewBody>, rows: Vec<(&'static str, String)>) -> Shown {
        Shown::Item { name: "foto.jpg".into(), key: key("/x/foto.jpg"), icon: 3, info: body.map(PreviewInfo::new), rows, tags: Vec::new() }
    }

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            full: None,
            group: 0,
            pidl: None,
            display: name.into(),
            sort_key: name.to_lowercase(),
            is_dir,
            size: 3_871_000,
            modified: 133_000_000_000_000_000,
            created: 132_000_000_000_000_000,
            attrs: 0,
            type_name: Arc::from("Imagen JPEG"),
            icon: 0,
            overlay: 0,
            overlay_checked: true,
        }
    }

    /// Texto: en la fuente de ancho fijo, las primeras líneas en orden, dentro del recuadro de
    /// 288×300 (margen 16, 10 adentro).
    #[test]
    fn preview_text_draws_mono_lines() {
        let p = painter();
        let lines: Vec<String> = (0..40).map(|i| format!("línea {i}")).collect();
        draw(&p, column(), &item(Some(PreviewBody::Text { lines }), vec![("Tipo", "Archivo RS".into())]));
        let mono: Vec<_> = p.texts().into_iter().filter(|(f, _, _)| *f == FontId::Mono).collect();
        assert_eq!(mono[0].1, "línea 0");
        assert_eq!(mono[1].1, "línea 1");
        let (l, t) = (column().left + MARGIN, column().top + MARGIN);
        assert_eq!((mono[0].2.left, mono[0].2.top), (l + 10.0, t + 8.0));
        // Solo las que entran en 300 de alto.
        assert!(mono.iter().all(|(_, _, r)| r.bottom <= t + TEXT_H), "{mono:?}");
        assert_eq!(mono.len(), 17);
        // Los detalles empiezan 20 más abajo: nombre y tipo.
        let texts = p.texts();
        let name = texts.iter().find(|(_, s, _)| s == "foto.jpg").unwrap();
        assert_eq!(name.2.top, t + TEXT_H + 20.0);
        assert!(texts.iter().any(|(_, s, _)| s == "Archivo RS"));
    }

    /// Imagen: la grande, con el lado del escalón para 288 y en 288×216 (4:3 del ancho).
    #[test]
    fn preview_image_uses_big_image() {
        let p = painter();
        draw(&p, column(), &item(Some(PreviewBody::Image), vec![("Tipo", "Imagen JPEG".into())]));
        let draws = p.big_draws();
        assert_eq!(draws.len(), 1);
        let (k, r) = &draws[0];
        assert_eq!(k.1, thumbs::big_side(p.thumb_px(288.0)));
        assert_eq!(k.0, key("/x/foto.jpg").0);
        assert_eq!((r.right - r.left, r.bottom - r.top), (288.0, 216.0));
        // Cargando: ícono del tipo, sin pedir la imagen.
        let p = painter();
        draw(&p, column(), &item(None, vec![("Tipo", "Imagen JPEG".into())]));
        assert!(p.big_draws().is_empty());
    }

    /// Varios: «3 elementos seleccionados», el tamaño total y el tipo común; sin tipo común, solo
    /// el tamaño.
    #[test]
    fn preview_many_selected() {
        let p = painter();
        let keys = vec![key("a"), key("b"), key("c")];
        let many = Shown::Many { count: 3, keys: keys.clone(), icons: vec![1, 2, 3], bytes: 13_002_752, kind: Some(Arc::from("Imagen JPEG")) };
        draw(&p, column(), &many);
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        let size = crate::format::size_short(13_002_752);
        assert!(texts.contains(&"3 elementos seleccionados".to_string()), "{texts:?}");
        assert!(texts.contains(&format!("{size} · Imagen JPEG")), "{texts:?}");
        assert!(texts.contains(&"3".to_string()), "la cantidad en acento");
        let kb = crate::format::size_kb(13_002_752);
        assert!(texts.contains(&format!("{size} ({kb})")), "{texts:?}");
        // 647 KB justos: el detalle en KB no agrega nada.
        let p2 = painter();
        draw(&p2, column(), &Shown::Many { count: 2, keys: vec![key("a"), key("b")], icons: vec![1, 2], bytes: 662_528, kind: None });
        let texts2: Vec<String> = p2.texts().into_iter().map(|t| t.1).collect();
        let short = crate::format::size_short(662_528);
        assert!(texts2.contains(&short), "{texts2:?}");
        assert!(!texts2.iter().any(|t| t.contains('(')), "sin paréntesis que repite: {texts2:?}");
        let p = painter();
        draw(&p, column(), &Shown::Many { count: 2, keys, icons: vec![1, 2], bytes: 10, kind: None });
        let texts: Vec<String> = p.texts().into_iter().map(|t| t.1).collect();
        assert!(texts.contains(&"2 elementos seleccionados".to_string()));
        assert!(!texts.iter().any(|t| t.contains("Imagen JPEG")));
        assert!(!texts.contains(&"Tipo".to_string()));
    }

    /// Filas de una imagen con metadatos: Tipo, Tamaño, Creado, Modificado, Último acceso y
    /// Dimensiones; sin metadatos no hay rótulos vacíos. Las carpetas no llevan Tamaño.
    #[test]
    fn preview_details_rows() {
        install_fakes();
        let env = FakeEnv;
        let e = entry("IMG_3101.jpg", false);
        let meta = Meta { width: Some(4032), height: Some(3024), authors: vec!["Javier Boero".into()], ..Meta::default() };
        let rows = detail_rows(&e, Some(&meta), 134_000_000_000_000_000, &env);
        let labels: Vec<&str> = rows.iter().map(|r| r.0).collect();
        assert_eq!(labels, ["Tipo", "Tamaño", "Creado", "Modificado", "Último acceso", "Dimensiones", "Autor"]);
        assert_eq!(rows[5].1, "4032 × 3024");
        assert_eq!(rows[1].1, env.format_size_short(3_871_000));
        let rows = detail_rows(&e, None, 0, &env);
        let labels: Vec<&str> = rows.iter().map(|r| r.0).collect();
        assert_eq!(labels, ["Tipo", "Tamaño", "Creado", "Modificado"]);
        assert!(rows.iter().all(|r| !r.1.is_empty()));
        let rows = detail_rows(&entry("Viaje", true), Some(&Meta::default()), 0, &env);
        assert!(!rows.iter().any(|r| r.0 == "Tamaño"));
    }

    /// Detalles: rótulo a la izquierda, valor en x = 104, filas de 22; etiquetas en píldoras.
    #[test]
    fn details_layout() {
        let p = painter();
        let r = rect(16.0, 300.0, 304.0, 700.0);
        draw_details(&p, r, &[("Tamaño", "3,8 MB".into()), ("Creado", "hoy".into())], &["viaje".into()]);
        let texts = p.texts();
        let find = |s: &str| texts.iter().find(|t| t.1 == s).unwrap().2;
        assert_eq!(find("Información").top, 300.0);
        assert_eq!((find("Tamaño").left, find("Tamaño").top), (16.0, 324.0));
        assert_eq!((find("3,8 MB").left, find("3,8 MB").top), (120.0, 324.0));
        assert_eq!(find("Creado").top, 346.0);
        assert_eq!(find("Etiquetas").top, 368.0);
        assert!(find("viaje").left >= 120.0);
    }

    /// Carpeta: ícono y «N elementos · tamaño»; solo en la nube: la leyenda «Disponible solo en
    /// línea»; un nombre largo se parte en varias líneas.
    #[test]
    fn folder_cloud_and_long_name() {
        let p = painter();
        let rows = vec![("Tipo", "Carpeta".into()), ("Contenido", "12 elementos".into()), ("Tamaño", "1,24 GB".into())];
        draw(&p, column(), &item(Some(PreviewBody::Folder { items: 12 }), rows));
        assert!(p.texts().iter().any(|t| t.1 == "12 elementos · 1,24 GB"));
        let p = painter();
        draw(&p, column(), &item(Some(PreviewBody::CloudOnly), vec![("Estado", CLOUD_TEXT.into())]));
        assert_eq!(p.texts().iter().filter(|t| t.1 == CLOUD_TEXT).count(), 2);
        assert!(p.big_draws().is_empty());
        let p = painter();
        let long = Shown::Item { name: "un-nombre-muy-largo-para-una-sola-linea-de-la-columna.jpg".into(), key: key("x"), icon: 0, info: None, rows: vec![], tags: vec![] };
        draw(&p, column(), &long);
        let names: Vec<_> = p.texts().into_iter().filter(|t| t.0 == FontId::Title).collect();
        assert!(names.len() >= 2, "{names:?}");
        assert!(names.iter().all(|t| t.2.right <= column().right - MARGIN));
    }
}

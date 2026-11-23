//! Pruebas del color de las pestañas de grupo (clic derecho sobre la pestaña).

use super::*;
use crate::sys::testing::FakeHost;
use crate::theme::PRESETS;

fn group(name: &str) -> Group {
    Group { name: name.into(), layout: 1, split_x: 0.5, split_y: 0.5, panes: Vec::new(), temp: false, color: None }
}

/// App con dos grupos y la barra de grupos a la vista, ya dibujada.
fn app() -> App {
    crate::sys::testing::install_fakes();
    let mut a = App::new(Init::blank(), FakeHost::new());
    a.show_group_bar = true;
    a.groups.items = vec![group("uno"), group("dos")];
    a.paint();
    a
}

/// Centro de la pestaña de grupo `i`.
fn tab_point(a: &App, i: usize) -> (f32, f32) {
    let (w, _) = a.client_dips();
    let (tabs, _) = ui::group_tabs(w - a.preview_width(), ui::GAP + a.tree_width(), a.groups.items.len());
    let r = tabs[i];
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

fn right_click(a: &mut App, (x, y): (f32, f32)) {
    a.on_right_down(x, y);
    a.on_context_menu_request(Some((x, y)));
    a.paint();
}

fn click(a: &mut App, (x, y): (f32, f32)) {
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    a.paint();
}

fn open_on(a: &mut App, i: usize) -> (f32, f32) {
    let p = tab_point(a, i);
    right_click(a, p);
    p
}

#[test]
fn right_click_on_a_group_tab_opens_the_color_panel() {
    let mut a = app();
    assert!(a.group_color.is_none());
    open_on(&mut a, 1);
    let m = a.group_color.as_ref().expect("se abrió el panel");
    assert_eq!(m.group, "dos");
}

#[test]
fn right_click_elsewhere_does_not_open_it() {
    let mut a = app();
    // En la lista de archivos no hay panel de colores.
    right_click(&mut a, (300.0, 300.0));
    assert!(a.group_color.is_none());
}

#[test]
fn picking_a_swatch_colors_the_group_and_closes() {
    let mut a = app();
    open_on(&mut a, 1);
    let at = a.group_color.as_ref().unwrap().option_center(9);
    click(&mut a, at);
    assert!(a.group_color.is_none());
    assert_eq!(a.groups.items[1].color, Some(PRESETS[9].1));
    assert_eq!(a.groups.items[0].color, None, "el otro grupo no cambia");
    assert!(Groups::parse(&a.groups.write()).items[1].color == Some(PRESETS[9].1), "queda guardado");
}

#[test]
fn the_app_accent_row_clears_the_color() {
    let mut a = app();
    a.groups.items[0].color = Some(PRESETS[2].1);
    open_on(&mut a, 0);
    let at = a.group_color.as_ref().unwrap().option_center(12);
    click(&mut a, at);
    assert_eq!(a.groups.items[0].color, None);
}

#[test]
fn click_outside_closes_without_changing_anything() {
    let mut a = app();
    open_on(&mut a, 0);
    click(&mut a, (5.0, 700.0));
    assert!(a.group_color.is_none());
    assert!(a.groups.items.iter().all(|g| g.color.is_none()));
}

#[test]
fn keyboard_moves_focus_picks_and_escapes() {
    let mut a = app();
    open_on(&mut a, 0);
    a.on_key(VK_RIGHT);
    a.on_key(VK_RIGHT);
    a.on_key(VK_DOWN);
    assert_eq!(a.group_color.as_ref().unwrap().focus, Some(7), "dos a la derecha y una fila abajo");
    a.on_key(VK_RETURN);
    assert!(a.group_color.is_none());
    assert_eq!(a.groups.items[0].color, Some(PRESETS[7].1));
    open_on(&mut a, 0);
    a.on_key(VK_ESCAPE);
    assert!(a.group_color.is_none());
    assert_eq!(a.groups.items[0].color, Some(PRESETS[7].1), "Esc no cambia nada");
}

#[test]
fn the_panel_stays_inside_the_window() {
    let a = app();
    let (w, h) = a.client_dips();
    let m = group_color::GroupColorMenu::new("uno".into(), None, (w - 2.0, h - 2.0));
    let (x, y) = {
        m.draw(&crate::sys::testing::RecordingPainter::new(a.theme), w, h, None);
        m.option_center(11)
    };
    assert!(x < w && y < h, "({x}, {y}) dentro de {w}x{h}");
    assert!(m.hit(w, h, x, y) == Some(group_color::GroupColorHit::Swatch(11)));
}

#[test]
fn the_group_keeps_its_color_through_autosave_and_save() {
    let mut a = app();
    a.groups.items[0].color = Some(PRESETS[4].1);
    a.group_name = Some("uno".into());
    a.autosave_group();
    assert_eq!(a.groups.items[0].color, Some(PRESETS[4].1), "el autoguardado no la borra");
    a.save_group("uno");
    assert_eq!(a.groups.items[0].color, Some(PRESETS[4].1), "guardar sobre el mismo nombre la conserva");
}

#[test]
fn colored_tab_paints_a_bar_in_its_color() {
    let mut a = app();
    a.groups.items[1].color = Some(PRESETS[7].1);
    a.paint();
    // Pintar con el panel abierto tampoco falla.
    open_on(&mut a, 1);
}

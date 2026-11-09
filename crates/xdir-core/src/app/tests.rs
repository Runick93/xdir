//! Pruebas del estado de `App` sin ventana: se arma con `Init::blank()` (nada del disco del usuario)
//! y se ejercitan las transiciones de paneles, pestañas, layout y vistas.

use super::*;
use std::rc::Rc;
use crate::sys::testing::FakeHost;

fn app() -> App {
    app_with(Init::blank())
}

/// Toda `App` de prueba sale de acá: primero los servicios del sistema falsos (el `Waker` solo
/// cuenta los mensajes), así ningún hilo de trabajo los pide sin instalar.
fn app_with(init: Init) -> App {
    app_with_host(init, FakeHost::new())
}

/// Igual que `app_with`, con una ventana falsa propia para mirar qué le pidió la `App`.
fn app_with_host(init: Init, host: Rc<FakeHost>) -> App {
    crate::sys::testing::install_fakes();
    App::new(init, host)
}

/// La `App` de prueba usa el entorno falso: configuración en la carpeta temporal del hilo, una
/// consola y formatos deterministas.
#[test]
fn test_app_uses_the_fake_env() {
    let _a = app();
    let name = format!("{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], "");
    assert_eq!(crate::session::config_dir(), Some(std::env::temp_dir().join("xdir-tests").join(name)));
    let labels: Vec<_> = Console::all().iter().map(|c| c.label()).collect();
    assert_eq!(labels, ["Terminal"]);
    assert_eq!(crate::format::date(116_444_736_000_000_000), "1970-01-01");
    assert_eq!(crate::format::size_kb(2048), "2 KB");
}

/// La plataforma le pasa la ventana y su DPI con `attach`: la `App` los guarda y le pasa la
/// ventana al host.
#[test]
fn attach_sets_window_and_dpi() {
    let host = FakeHost::new();
    let mut app = App::new(Init::blank(), host.clone());
    app.attach(Handle(7), 144);
    assert_eq!(app.window(), Handle(7));
    assert_eq!(app.dpi(), 144);
    assert_eq!(host.attached(), Some(Handle(7)));
}

fn count(a: &App, pane: usize) -> usize {
    a.panes[pane].tabs.len()
}

#[test]
fn test_app_posts_through_the_fake_waker() {
    use crate::sys::testing::FakeWaker;
    // Número propio de este test, para que los demás (en paralelo) no lo cuenten.
    const MSG: u32 = WM_APP + 0x7B01;
    let a = app();
    let before = FakeWaker::count(MSG);
    a.hwnd.post(MSG, 0, 0);
    assert_eq!(FakeWaker::count(MSG) - before, 1);
}

#[test]
fn starts_with_four_panes_one_tab_each() {
    let a = app();
    assert_eq!(a.panes.len(), PANES);
    assert_eq!(a.layout, Layout::Four);
    assert_eq!(a.active, 0);
    assert_eq!(a.shown_panes(), vec![0, 1, 2, 3]);
    assert!((0..PANES).all(|p| count(&a, p) == 1));
}

#[test]
fn start_path_opens_a_new_active_tab_in_the_active_pane() {
    let dir = std::env::temp_dir().join("xdir-test-start");
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = Init::blank();
    init.start_path = Some(dir.clone());
    let a = app_with(init);
    assert_eq!(count(&a, 0), 2);
    // Windows puede normalizar la ruta (nombres cortos 8.3): se compara el último componente.
    let Location::Path(p) = &a.panes[0].tab().location else { panic!("no es una carpeta") };
    assert_eq!(p.file_name(), dir.file_name());
}

#[test]
fn tab_ids_are_unique() {
    let mut a = app();
    a.new_tab(0, Location::Computer, true);
    a.new_tab(1, Location::Computer, false);
    let mut ids: Vec<usize> = a.panes.iter().flat_map(|p| p.tabs.iter().map(|t| t.id)).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

#[test]
fn new_tab_activation_and_view_inheritance() {
    let mut a = app();
    a.set_view(0, list::View::Icons);
    a.new_tab(0, Location::Computer, true);
    assert_eq!(count(&a, 0), 2);
    assert_eq!(a.panes[0].active, 1);
    assert_eq!(a.panes[0].tab().list.view, list::View::Icons);
    // En segundo plano no cambia la pestaña activa.
    a.new_tab(0, Location::Computer, false);
    assert_eq!(a.panes[0].active, 1);
    assert_eq!(count(&a, 0), 3);
}

#[test]
fn last_tab_cannot_be_closed() {
    let mut a = app();
    a.close_tab(0, 0);
    assert_eq!(count(&a, 0), 1);
    a.new_tab(0, Location::Computer, true);
    a.close_tab(0, 1);
    assert_eq!(count(&a, 0), 1);
    assert_eq!(a.panes[0].active, 0);
}

#[test]
fn select_tab_ignores_out_of_range() {
    let mut a = app();
    a.new_tab(2, Location::Computer, false);
    a.select_tab(2, 1);
    assert_eq!(a.panes[2].active, 1);
    a.select_tab(2, 9);
    assert_eq!(a.panes[2].active, 1);
}

#[test]
fn moving_a_tab_between_panes() {
    let mut a = app();
    a.new_tab(0, Location::Path(std::env::temp_dir()), true);
    let id = a.panes[0].tab().id;
    a.move_tab_to_pane(0, 1, 2, 0);
    assert_eq!(count(&a, 0), 1);
    assert_eq!(count(&a, 2), 2);
    assert_eq!(a.panes[2].tab().id, id, "la pestaña movida queda activa en el destino");
    assert_eq!(a.active, 2);
}

#[test]
fn moving_the_only_tab_leaves_this_pc_behind() {
    let mut a = app();
    let id = a.panes[1].tab().id;
    a.move_tab_to_pane(1, 0, 3, 5);
    assert_eq!(count(&a, 1), 1);
    assert_eq!(a.panes[1].tab().location, Location::Computer);
    assert_ne!(a.panes[1].tab().id, id);
    assert_eq!(a.panes[3].tabs.last().unwrap().id, id);
}

#[test]
fn moving_within_the_same_pane_or_out_of_range_does_nothing() {
    let mut a = app();
    a.move_tab_to_pane(0, 0, 0, 0);
    a.move_tab_to_pane(0, 7, 1, 0);
    assert_eq!((0..PANES).map(|p| count(&a, p)).collect::<Vec<_>>(), vec![1; PANES]);
}

#[test]
fn shrinking_the_layout_clamps_the_active_pane_and_unmaximizes() {
    let mut a = app();
    a.set_active(3);
    a.toggle_maximize(3);
    assert_eq!(a.shown_panes(), vec![3]);
    a.set_layout(Layout::TwoSide);
    assert_eq!(a.maximized, None);
    assert_eq!(a.active, 1);
    assert_eq!(a.shown_panes(), vec![0, 1]);
    a.set_layout(Layout::One);
    assert_eq!(a.active, 0);
    assert_eq!(a.shown_panes(), vec![0]);
}

#[test]
fn maximize_toggles_and_needs_two_panes() {
    let mut a = app();
    a.toggle_maximize(2);
    assert_eq!((a.maximized, a.active), (Some(2), 2));
    a.toggle_maximize(2);
    assert_eq!(a.maximized, None);
    a.toggle_maximize(9);
    assert_eq!(a.maximized, None);
    a.set_layout(Layout::One);
    a.toggle_maximize(0);
    assert_eq!(a.maximized, None);
}

#[test]
fn activating_another_pane_restores_a_maximized_one() {
    let mut a = app();
    a.toggle_maximize(0);
    a.set_active(1);
    assert_eq!((a.maximized, a.active), (None, 1));
    // Un panel fuera del layout se ignora.
    a.set_layout(Layout::TwoSide);
    a.set_active(3);
    assert_eq!(a.active, 1);
}

#[test]
fn only_visible_panes_have_visible_tabs() {
    let mut a = app();
    a.set_layout(Layout::TwoSide);
    let vis: Vec<bool> = (0..PANES).map(|p| a.panes[p].tab().is_visible()).collect();
    assert_eq!(vis, vec![true, true, false, false]);
}

#[test]
fn view_applies_to_all_tabs_of_a_pane_and_cycles_within_bounds() {
    let mut a = app();
    a.new_tab(0, Location::Computer, false);
    a.set_view(0, list::View::Icons);
    assert!(a.panes[0].tabs.iter().all(|t| t.list.view == list::View::Icons));
    assert_eq!(a.panes[1].tab().list.view, list::View::Details);
    for _ in 0..list::VIEWS.len() + 2 {
        a.cycle_view(1, true);
    }
    assert_eq!(a.panes[1].tab().list.view.index(), list::VIEWS.len() - 1);
    for _ in 0..list::VIEWS.len() + 2 {
        a.cycle_view(1, false);
    }
    assert_eq!(a.panes[1].tab().list.view.index(), 0);
}

#[test]
fn columns_are_shared_by_every_tab() {
    let mut a = app();
    a.new_tab(1, Location::Computer, false);
    let mut cols = list::default_columns();
    cols.pop();
    a.set_columns(cols.clone());
    assert_eq!(a.columns, cols);
    assert!(a.panes.iter().flat_map(|p| p.tabs.iter()).all(|t| t.list.columns == cols));
    // Las pestañas nuevas las heredan.
    a.new_tab(3, Location::Computer, false);
    assert_eq!(a.panes[3].tabs[1].list.columns, cols);
}

#[test]
fn ghost_converges_to_its_target() {
    let mut g = Ghost { title: "x".into(), w: 10.0, x: 0.0, y: 0.0, tx: 100.0, ty: 50.0, detach: 1.0, docked: true };
    let mut steps = 0;
    while g.step(1.0 / 60.0) {
        steps += 1;
        assert!(steps < 600, "no converge");
    }
    assert!((g.x - 100.0).abs() < 0.5 && (g.y - 50.0).abs() < 0.5);
    assert!(g.detach < 0.02, "acoplada: despegue casi 0");
}

#[test]
fn fav_act_ids_round_trip() {
    for id in 1..=9 {
        let act = FavAct::from_id(id).expect("id válido");
        assert_eq!(act as i32, id);
    }
    assert_eq!(FavAct::from_id(0), None);
    assert_eq!(FavAct::from_id(10), None);
}

// ---------- hit-testing ----------

fn center(r: &geom::Rect) -> (f32, f32) {
    ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
}

#[test]
fn status_bar_buttons_are_hit() {
    let a = app();
    let (w, h) = a.client_dips();
    let (x, y) = center(&ui::gear_rect(w, h));
    assert_eq!(a.hit(x, y), Hit::Gear);
    let (x, y) = center(&ui::groups_rect(w, h));
    assert_eq!(a.hit(x, y), Hit::Groups);
    let (x, y) = center(&ui::xtree_rect(w, h));
    assert_eq!(a.hit(x, y), Hit::Xtree);
    for (layout, r) in ui::layout_buttons(w, h) {
        let (x, y) = center(&r);
        assert_eq!(a.hit(x, y), Hit::Layout(layout));
    }
}

#[test]
fn splitters_are_hit_unless_a_pane_is_maximized() {
    let mut a = app();
    let (w, h) = a.client_dips();
    let area = ui::panes_area(w, h, a.tree_width(), a.groups_bar_h());
    let (s, r) = ui::splitter_rects(a.layout, area, a.splits)[0];
    let (x, y) = center(&r);
    assert_eq!(a.hit(x, y), Hit::Splitter(s));
    a.toggle_maximize(0);
    assert!(!matches!(a.hit(x, y), Hit::Splitter(_)));
}

#[test]
fn about_dialog_is_modal() {
    let mut a = app();
    let (w, h) = a.client_dips();
    let (x, y) = center(&ui::gear_rect(w, h));
    a.about_open = true;
    assert!(!matches!(a.hit(x, y), Hit::Gear), "el diálogo tapa la barra de estado");
    assert_eq!(a.hit(0.0, 0.0), Hit::None);
}

#[test]
fn pane_hit_follows_the_pane_rect() {
    let mut a = app();
    a.panes[1].rect = geom::rect(100.0, 100.0, 500.0, 400.0);
    assert_eq!(a.hit(300.0, 101.0), Hit::Pane(1, PaneHit::TabStrip));
    // Un panel fuera del layout no responde aunque tenga rectángulo.
    a.set_layout(Layout::One);
    assert_ne!(a.hit(300.0, 101.0), Hit::Pane(1, PaneHit::TabStrip));
}

#[test]
fn group_bar_only_takes_space_when_there_are_groups() {
    let mut a = app();
    assert_eq!(a.groups_bar_h(), 0.0);
    a.show_group_bar = true;
    assert_eq!(a.groups_bar_h(), 0.0, "encendida pero sin grupos");
    a.ensure_group();
    assert_eq!(a.groups_bar_h(), ui::GROUPBAR_H);
}

// ---------- grupos ----------

fn named(a: &App, i: usize) -> &str {
    &a.groups.items[i].name
}

#[test]
fn ensure_group_creates_a_temporary_one_only_with_the_bar_on() {
    let mut a = app();
    a.ensure_group();
    assert!(a.groups.items.is_empty());
    a.show_group_bar = true;
    a.ensure_group();
    assert_eq!(a.groups.items.len(), 1);
    assert!(a.groups.items[0].temp);
    assert_eq!(a.group_name.as_deref(), Some("Grupo 1"));
    a.ensure_group();
    assert_eq!(a.groups.items.len(), 1, "no duplica");
}

#[test]
fn new_group_picks_a_free_name_and_activates_it() {
    let mut a = app();
    a.new_group();
    a.new_group();
    assert_eq!((named(&a, 0), named(&a, 1)), ("Grupo 1", "Grupo 2"));
    assert_eq!(a.group_name.as_deref(), Some("Grupo 2"));
    assert!(a.groups.items.iter().all(|g| g.temp));
    // Cada panel del grupo nuevo está en "Este equipo".
    assert!(a.panes.iter().all(|p| p.tab().location == Location::Computer));
}

#[test]
fn save_group_snapshots_the_panes_and_apply_restores_them() {
    let mut a = app();
    a.set_layout(Layout::TwoSide);
    a.new_tab(0, Location::Path(std::env::temp_dir()), true);
    a.save_group("trabajo");
    assert_eq!(a.group_name.as_deref(), Some("trabajo"));
    let g = &a.groups.items[0];
    assert!(!g.temp);
    assert_eq!(g.layout, Layout::TwoSide.index());
    assert_eq!(g.panes[0].tabs.len(), 2);
    // Cambiar todo y volver al grupo.
    a.group_name = None;
    a.set_layout(Layout::Four);
    a.close_tab(0, 1);
    a.apply_group(0);
    assert_eq!(a.layout, Layout::TwoSide);
    assert_eq!(count(&a, 0), 2);
    assert_eq!(a.active, 0);
    assert_eq!(a.maximized, None);
}

#[test]
fn saving_with_an_existing_name_replaces_it() {
    let mut a = app();
    a.save_group("A");
    a.save_group("a");
    assert_eq!(a.groups.items.len(), 1);
}

#[test]
fn rename_group_rejects_duplicates_and_follows_the_active_one() {
    let mut a = app();
    a.save_group("uno");
    a.save_group("dos");
    a.rename_group("dos", "uno");
    assert_eq!(named(&a, 1), "dos");
    assert!(a.notice.as_deref().unwrap().contains("Ya hay un grupo"));
    a.rename_group("dos", "tres");
    assert_eq!(named(&a, 1), "tres");
    assert_eq!(a.group_name.as_deref(), Some("tres"));
    // Renombrar uno que no existe no hace nada.
    a.rename_group("nada", "x");
    assert_eq!(a.groups.items.len(), 2);
}

#[test]
fn save_group_in_rename_mode_renames_instead() {
    let mut a = app();
    a.save_group("viejo");
    a.group_rename = Some("viejo".into());
    a.save_group("nuevo");
    assert_eq!(a.groups.items.len(), 1);
    assert_eq!(named(&a, 0), "nuevo");
    assert_eq!(a.group_rename, None);
}

#[test]
fn autosave_keeps_the_active_group_in_sync() {
    let mut a = app();
    a.save_group("g");
    assert_eq!(a.groups.items[0].panes[2].tabs.len(), 1);
    a.new_tab(2, Location::Computer, false);
    a.autosave_group();
    assert_eq!(a.groups.items[0].panes[2].tabs.len(), 2);
    assert_eq!(named(&a, 0), "g");
    // Sin grupo activo no toca nada.
    a.group_name = None;
    a.new_tab(2, Location::Computer, false);
    a.autosave_group();
    assert_eq!(a.groups.items[0].panes[2].tabs.len(), 2);
}

#[test]
fn switch_group_only_applies_when_it_is_another() {
    let mut a = app();
    a.save_group("a");
    a.set_layout(Layout::One);
    a.save_group("b");
    a.switch_group(0);
    assert_eq!(a.group_name.as_deref(), Some("a"));
    assert_eq!(a.layout, Layout::Four);
    // Sobre el activo no reaplica: lo cambiado a mano se conserva.
    a.set_layout(Layout::TwoStack);
    a.switch_group(0);
    assert_eq!(a.layout, Layout::TwoStack);
}

#[test]
fn applying_a_missing_group_is_ignored() {
    let mut a = app();
    a.apply_group(3);
    assert_eq!(a.layout, Layout::Four);
}

// ---------- favoritos ----------

fn link(name: &str, dir: &str) -> Fav {
    Fav::Link { name: name.into(), location: Location::Path(PathBuf::from(dir)) }
}

#[test]
fn toggle_favorite_adds_then_removes_the_pane_folder() {
    let mut a = app();
    a.toggle_favorite(0);
    assert_eq!(a.favorites.items.len(), 1);
    assert!(a.notice.as_deref().unwrap().starts_with("Agregado"));
    a.toggle_favorite(0);
    assert!(a.favorites.items.is_empty());
    assert!(a.notice.as_deref().unwrap().starts_with("Quitado"));
}

#[test]
fn favorite_link_opens_in_the_pane_or_in_a_new_tab() {
    let mut a = app();
    a.favorites.items.push(Fav::Link { name: "tmp".into(), location: Location::Path(std::env::temp_dir()) });
    a.activate_fav(1, &[0], false);
    assert!(matches!(a.panes[1].tab().location, Location::Path(_)));
    assert_eq!(count(&a, 1), 1);
    a.activate_fav(1, &[0], true);
    assert_eq!(count(&a, 1), 2);
}

#[test]
fn favorite_group_with_new_tab_opens_every_link() {
    let mut a = app();
    a.favorites.items.push(Fav::Group {
        name: "g".into(),
        items: vec![link("a", r"C:\a"), Fav::Group { name: "sub".into(), items: vec![] }, link("b", r"C:\b")],
    });
    a.activate_fav(0, &[0], true);
    assert_eq!(count(&a, 0), 3, "los subgrupos no abren pestaña");
    // Un favorito que no existe se ignora.
    a.activate_fav(0, &[5], true);
    assert_eq!(count(&a, 0), 3);
}

// ---------- comandos y teclado ----------

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("xdir-test-fixtures").join(format!("{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Último componente de la carpeta que muestra la pestaña activa de un panel, en minúsculas
/// (Windows puede cambiar mayúsculas y nombres cortos).
fn leaf(a: &App, pane: usize) -> String {
    match &a.panes[pane].tab().location {
        Location::Path(p) => p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
        other => format!("{other:?}"),
    }
}

#[test]
fn new_tab_duplicates_the_folder_and_close_tab_remembers_it() {
    let mut a = app();
    let dir = fixture("newtab");
    a.go_to_text(0, &dir.to_string_lossy());
    a.run_command(Command::NewTab);
    assert_eq!(count(&a, 0), 2);
    assert_eq!(leaf(&a, 0), leaf_of(&dir));
    a.run_command(Command::CloseTab);
    assert_eq!(count(&a, 0), 1);
    // Reabrir la cerrada.
    a.run_command(Command::ReopenTab);
    assert_eq!(count(&a, 0), 2);
    assert_eq!(a.panes[0].active, 1);
    // Sin nada que reabrir no pasa nada.
    a.run_command(Command::ReopenTab);
    assert_eq!(count(&a, 0), 2);
}

#[test]
fn tab_and_pane_cycling_wraps_around() {
    let mut a = app();
    a.new_tab(0, Location::Computer, false);
    a.new_tab(0, Location::Computer, false);
    assert_eq!(a.panes[0].active, 0);
    a.run_command(Command::PrevTab);
    assert_eq!(a.panes[0].active, 2);
    a.run_command(Command::NextTab);
    assert_eq!(a.panes[0].active, 0);
    a.run_command(Command::PrevPane);
    assert_eq!(a.active, 3);
    a.run_command(Command::NextPane);
    assert_eq!(a.active, 0);
    a.run_command(Command::FocusPane(2));
    assert_eq!(a.active, 2);
    a.run_command(Command::FocusPane(9));
    assert_eq!(a.active, 2, "fuera del layout se ignora");
}

#[test]
fn layout_and_maximize_commands() {
    let mut a = app();
    a.run_command(Command::Layout(1));
    assert_eq!(a.layout, Layout::TwoSide);
    a.run_command(Command::Layout(99));
    assert_eq!(a.layout, Layout::TwoSide);
    a.run_command(Command::Maximize);
    assert_eq!(a.maximized, Some(0));
    a.run_command(Command::Maximize);
    assert_eq!(a.maximized, None);
}

#[test]
fn group_cycling_falls_back_to_tabs_when_there_are_no_groups() {
    let mut a = app();
    a.new_tab(0, Location::Computer, false);
    a.run_command(Command::NextGroup);
    assert_eq!(a.panes[0].active, 1);
    a.save_group("a");
    a.save_group("b");
    a.save_group("c");
    a.switch_group(0);
    a.run_command(Command::NextGroup);
    assert_eq!(a.group_name.as_deref(), Some("b"));
    a.run_command(Command::PrevGroup);
    a.run_command(Command::PrevGroup);
    assert_eq!(a.group_name.as_deref(), Some("c"), "da la vuelta");
}

#[test]
fn toggles_flip_their_flags() {
    let mut a = app();
    let favbar = a.show_favorites;
    a.run_command(Command::ToggleFavBar);
    assert_eq!(a.show_favorites, !favbar);

    a.new_tab(1, Location::Computer, false);
    let hidden = a.show_hidden;
    a.run_command(Command::ToggleHidden);
    assert_eq!(a.show_hidden, !hidden);
    assert!(a.panes.iter().flat_map(|p| p.tabs.iter()).all(|t| t.list.show_hidden == !hidden), "todas las pestañas");

    assert!(!a.show_group_bar);
    a.run_command(Command::ToggleGroupBar);
    assert!(a.show_group_bar);
    assert_eq!(a.groups.items.len(), 1, "enciende la barra y crea el grupo temporal");
}

#[test]
fn about_closes_other_overlays() {
    let mut a = app();
    a.picker_open = true;
    a.term_menu = Some(1);
    a.run_command(Command::About);
    assert!(a.about_open && !a.picker_open && a.term_menu.is_none());
}

#[test]
fn view_and_sort_commands() {
    let mut a = app();
    a.run_command(Command::View(2));
    assert_eq!(a.panes[0].tab().list.view, list::VIEWS[2].0);
    a.run_command(Command::View(99));
    assert_eq!(a.panes[0].tab().list.view, list::VIEWS[2].0);
    let col = list::COLUMNS[1].0;
    a.run_command(Command::SortBy(1));
    assert_eq!(a.panes[0].tab().list.sort, col);
    let dir = a.panes[0].tab().list.descending;
    a.run_command(Command::SortBy(1));
    assert_eq!(a.panes[0].tab().list.descending, !dir, "repetir invierte el orden");
}

#[test]
fn toggle_favorite_command_uses_the_active_pane() {
    let mut a = app();
    a.run_command(Command::FocusPane(1));
    a.run_command(Command::ToggleFavorite);
    assert_eq!(a.favorites.items.len(), 1);
}

#[test]
fn escape_closes_overlays_one_at_a_time() {
    let mut a = app();
    a.about_open = true;
    a.picker_open = true;
    a.term_menu = Some(0);
    assert!(a.on_key(VK_ESCAPE));
    assert!(!a.about_open && a.picker_open);
    assert!(a.on_key(VK_ESCAPE));
    assert!(!a.picker_open && a.term_menu.is_some());
    assert!(a.on_key(VK_ESCAPE));
    assert!(a.term_menu.is_none());
}

#[test]
fn unmapped_keys_are_not_consumed() {
    let mut a = app();
    assert!(!a.on_key(Key(0xFF)));
}

// ---------- ir a una ruta escrita ----------

#[test]
fn go_to_text_empty_or_this_pc_goes_to_computer() {
    let mut a = app();
    a.go_to_text(0, &fixture("goto").to_string_lossy());
    assert_ne!(a.panes[0].tab().location, Location::Computer);
    a.go_to_text(0, "  Este Equipo ");
    assert_eq!(a.panes[0].tab().location, Location::Computer);
    a.go_to_text(0, &fixture("goto").to_string_lossy());
    a.go_to_text(0, "");
    assert_eq!(a.panes[0].tab().location, Location::Computer);
}

#[test]
fn go_to_text_accepts_quotes_relative_paths_and_variables() {
    let mut a = app();
    let base = fixture("rel");
    std::fs::create_dir_all(base.join("hijo")).unwrap();
    a.go_to_text(0, &format!("\"{}\"", base.display()));
    assert_eq!(leaf(&a, 0), leaf_of(&base));
    a.go_to_text(0, "hijo");
    assert_eq!(leaf(&a, 0), "hijo");
    // ".." sube una carpeta, y la ruta queda limpia.
    a.go_to_text(0, "..");
    assert_eq!(leaf(&a, 0), leaf_of(&base));
    let Location::Path(p) = &a.panes[0].tab().location else { panic!("no es una carpeta") };
    assert!(!p.to_string_lossy().contains(".."), "{}", p.display());
    #[cfg(windows)]
    {
        a.go_to_text(0, r"hijo\..\hijo");
        assert_eq!(leaf(&a, 0), "hijo");
        a.go_to_text(1, "%TEMP%");
        assert_eq!(leaf(&a, 1), leaf_of(&std::env::temp_dir()));
    }
    #[cfg(not(windows))]
    {
        a.go_to_text(0, "hijo/../hijo");
        assert_eq!(leaf(&a, 0), "hijo");
        // $HOME siempre está definida (TMPDIR suele faltar en Linux).
        a.go_to_text(1, "$HOME");
        assert_eq!(leaf(&a, 1), leaf_of(std::path::Path::new(&std::env::var("HOME").unwrap())));
    }
}

fn leaf_of(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

#[test]
#[cfg(windows)]
fn go_to_text_drive_letter_means_the_root() {
    let mut a = app();
    a.go_to_text(0, "C:");
    let Location::Path(p) = &a.panes[0].tab().location else { panic!("no es una carpeta") };
    assert_eq!(p.to_string_lossy().to_uppercase(), "C:\\");
}

#[test]
fn go_to_text_with_a_file_opens_its_folder() {
    let mut a = app();
    let base = fixture("file");
    let file = base.join("dato.txt");
    std::fs::write(&file, "x").unwrap();
    a.go_to_text(2, &file.to_string_lossy());
    assert_eq!(leaf(&a, 2), leaf_of(&base));
}

// ---------- operaciones terminadas ----------

fn result(undo: Option<Undo>, focus: Option<PathBuf>, rename_after: bool, error: Option<&str>) -> OpResult {
    OpResult { undo, focus, rename_after, error: error.map(String::from) }
}

#[test]
fn op_done_reports_errors_and_clears_the_notice() {
    let mut a = app();
    a.on_op_done(result(None, None, false, Some("sin permiso")));
    assert_eq!(a.notice.as_deref(), Some("Error: sin permiso"));
    a.on_op_done(result(None, None, false, None));
    assert_eq!(a.notice, None);
}

#[test]
fn op_done_stacks_undo_with_a_cap() {
    let mut a = app();
    for i in 0..UNDO_MAX + 10 {
        a.on_op_done(result(Some(Undo::Created(vec![PathBuf::from(format!("C:\\x{i}"))])), None, false, None));
    }
    assert_eq!(a.undo.len(), UNDO_MAX);
    // Se descartan las más viejas: la última sigue siendo la más reciente.
    let Undo::Created(last) = a.undo.last().unwrap() else { panic!() };
    assert_eq!(last[0], PathBuf::from(format!("C:\\x{}", UNDO_MAX + 9)));
}

#[test]
fn op_done_with_a_new_folder_asks_for_the_rename() {
    let mut a = app();
    let path = PathBuf::from(r"C:\nada\Nueva carpeta");
    a.on_op_done(result(None, Some(path.clone()), true, None));
    assert_eq!(a.pending_rename, Some(path.clone()));
    let mut b = app();
    b.on_op_done(result(None, Some(path), false, None));
    assert_eq!(b.pending_rename, None, "sin rename_after solo se enfoca");
    let mut c = app();
    c.on_op_done(result(None, None, true, None));
    assert_eq!(c.pending_rename, None);
}

/// Entrega a la `App` lo que el `FileOps` falso respondió por el canal (lo que haría el
/// `WM_APP_OP_DONE` en la ventana).
fn drain_op_results(a: &mut App) {
    while let Ok(r) = a.ops_rx.try_recv() {
        a.on_op_done(r);
    }
}

#[test]
fn rename_sends_rename_op() {
    use crate::sys::testing::FakeFileOps;
    let mut a = app();
    FakeFileOps::take_ops();
    let path = std::env::temp_dir().join("viejo.txt");
    a.commit_rename(path.clone(), "nuevo.txt".into());
    assert_eq!(FakeFileOps::take_ops(), [Op::Rename { path, new_name: "nuevo.txt".into() }]);
}

#[test]
fn undo_after_new_folder_sends_undo() {
    use crate::sys::testing::FakeFileOps;
    let mut a = app();
    // La ruta como la guarda la pestaña: en Windows la carpeta temporal puede venir con nombre
    // corto (`RUNNER~1`) y la navegación la pasa al largo.
    let tmp = sys::file_system().true_case(&std::env::temp_dir());
    a.new_tab(0, Location::Path(tmp.clone()), true);
    FakeFileOps::take_ops();
    let created = vec![tmp.join("Nueva carpeta")];
    FakeFileOps::respond_with(OpResult { undo: Some(Undo::Created(created.clone())), ..OpResult::default() });
    a.run_command(Command::NewFolder);
    drain_op_results(&mut a);
    assert_eq!(FakeFileOps::take_ops(), [Op::NewFolder { parent: tmp }]);
    FakeFileOps::respond_with(OpResult::default());
    a.run_command(Command::Undo);
    assert_eq!(FakeFileOps::take_ops(), [Op::Undo(Undo::Created(created))]);
    assert!(a.undo.is_empty());
}

// ---------- cajas de texto y menús del host ----------

/// Perder el foco de la barra de dirección (`WM_APP_EDIT_DONE` con 0) cancela: no navega aunque
/// se haya escrito otra ruta, y la caja se cierra.
#[test]
fn address_edit_cancel_on_focus_loss() {
    use crate::host::TextKind;
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    let from = fixture("barra-desde");
    let other = fixture("barra-otra");
    a.new_tab(0, Location::Path(from.clone()), true);
    a.begin_edit(0);
    assert_eq!(host.text_of(TextKind::Address), Some(from.to_string_lossy().into_owned()));
    let (edit, _) = a.edit.expect("la barra quedó abierta");
    assert!(host.text_has_focus(edit));
    host.text_set(edit, &other.to_string_lossy());
    assert!(a.on_app_message(WM_APP_EDIT_DONE, 0, 0));
    assert_eq!(a.panes[0].tab().location, Location::Path(from));
    assert!(a.edit.is_none());
    assert_eq!(host.text_of(TextKind::Address), None, "la caja se destruyó");
}

/// El renombrado en línea: lo escrito en la caja se aplica al terminar (`WM_APP_RENAME_DONE` con 1).
#[test]
fn rename_applies_on_done() {
    use crate::host::TextKind;
    use crate::sys::testing::FakeFileOps;
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    let dir = std::env::temp_dir().join("xdir-tests").join("renombrar-caja");
    a.new_tab(0, Location::Path(dir.clone()), true);
    let list = &mut a.panes[0].tab_mut().list;
    list.append(vec![file_entry("viejo.txt")]);
    assert!(list.focus_name(&"viejo.txt".into()));
    a.begin_rename(0);
    assert_eq!(host.text_of(TextKind::Rename).as_deref(), Some("viejo.txt"));
    let edit = a.rename.as_ref().expect("se abrió el renombrado").edit;
    host.text_set(edit, "nuevo.txt");
    FakeFileOps::take_ops();
    assert!(a.on_app_message(WM_APP_RENAME_DONE, 1, 0));
    assert_eq!(FakeFileOps::take_ops(), [Op::Rename { path: dir.join("viejo.txt"), new_name: "nuevo.txt".into() }]);
    assert_eq!(host.text_of(TextKind::Rename), None);
}

/// El menú de una carpeta de favoritos con otra carpeta adentro lleva un submenú con su nombre.
#[test]
fn fav_folder_menu_is_nested() {
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    a.favorites.items.push(Fav::Group {
        name: "Trabajo".into(),
        items: vec![link("a", r"C:\a"), Fav::Group { name: "Clientes".into(), items: vec![link("b", r"C:\b")] }],
    });
    host.set_menu_choice(None);
    a.activate_fav(0, &[0], false);
    let menus = host.menus();
    let menu = menus.last().expect("se mostró el menú");
    assert!(
        menu.iter().any(|e| matches!(e, shell::Entry::Sub(name, items) if name == "Clientes" && items.len() == 1)),
        "{menu:?}"
    );
}

// ---------- xtree ----------

/// Carpeta temporal propia de una prueba (así la vista no analiza el disco del usuario).
fn xtree_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xdir-xtree-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // La ruta como la guarda la pestaña: en Windows la carpeta temporal puede venir con nombre
    // corto (`RUNNER~1`) y la navegación la pasa al largo.
    sys::file_system().true_case(&dir)
}

/// App con el panel activo en una carpeta temporal.
fn app_in(name: &str) -> (App, PathBuf) {
    let mut a = app();
    let dir = xtree_dir(name);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir.clone()));
    (a, dir)
}

fn click_xtree_button(a: &mut App) {
    let (w, h) = a.client_dips();
    let (x, y) = center(&ui::xtree_rect(w, h));
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
}

#[test]
fn xtree_button_toggles_view() {
    let (mut a, _) = app_in("toggle");
    click_xtree_button(&mut a);
    assert!(a.xtree.is_some() && a.xtree_open);
    click_xtree_button(&mut a);
    assert!(!a.xtree_open);
    let x = a.xtree.as_ref().expect("la vista cerrada no se destruye");
    assert!(!x.cancelled(), "cerrar la vista no cancela el escaneo");
    a.on_closing();
}

/// Cerrar la vista con el botón o con el comando también cierra el menú de discos: al volver a
/// abrirla no debe aparecer desplegado.
#[test]
fn closing_xtree_closes_drive_menu() {
    let (mut a, _) = app_in("menu");
    click_xtree_button(&mut a);
    a.xtree.as_mut().unwrap().open_menu();
    click_xtree_button(&mut a);
    assert!(!a.xtree_open);
    assert!(!a.xtree.as_ref().unwrap().menu_is_open(), "botón");
    a.run_command(Command::Xtree);
    a.xtree.as_mut().unwrap().open_menu();
    a.run_command(Command::Xtree);
    assert!(!a.xtree_open);
    assert!(!a.xtree.as_ref().unwrap().menu_is_open(), "comando");
    a.on_closing();
}

/// Lo que cambia los paneles con la vista abierta (botones de layout, grupos, comandos de la
/// paleta) cierra la vista para que se vea el efecto; lo que no los toca la deja abierta.
#[test]
fn pane_actions_close_xtree() {
    let (mut a, _) = app_in("paneles");
    let (w, h) = a.client_dips();
    click_xtree_button(&mut a);
    let (layout, r) = ui::layout_buttons(w, h).into_iter().find(|(l, _)| *l != a.layout).unwrap();
    let (x, y) = center(&r);
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert_eq!(a.layout, layout);
    assert!(!a.xtree_open, "botón de layout");

    a.run_command(Command::Xtree);
    a.run_command(Command::ToggleDarkMode);
    assert!(a.xtree_open, "el modo oscuro no toca los paneles");
    a.run_command(Command::Layout(0));
    assert!(!a.xtree_open, "comando de layout");

    a.save_group("g");
    a.run_command(Command::Xtree);
    a.apply_group(0);
    assert!(!a.xtree_open, "grupo");

    a.run_command(Command::Xtree);
    a.run_command(Command::NewTab);
    assert!(!a.xtree_open, "pestaña nueva");
    a.on_closing();
}

#[test]
fn closing_xdir_cancels_scan() {
    let (mut a, _) = app_in("closing");
    click_xtree_button(&mut a);
    a.on_closing();
    assert!(a.xtree.as_ref().unwrap().cancelled());
}

#[test]
fn xtree_view_swallows_pane_clicks() {
    let (mut a, _) = app_in("swallow");
    a.set_active(1);
    a.panes[0].rect = geom::rect(100.0, 100.0, 500.0, 400.0);
    // La tira de pestañas: sin la vista, un clic ahí activa el panel 0.
    let (x, y) = (300.0, 101.0);
    assert!(matches!(a.hit(x, y), Hit::Pane(0, _)), "sin la vista el clic es del panel 0: {:?}", a.hit(x, y));
    click_xtree_button(&mut a);
    let before = a.panes[0].tab().list.focused_index();
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
    assert_eq!(a.active, 1);
    assert_eq!(a.panes[0].tab().list.focused_index(), before);
    assert!(a.xtree_open);
    a.on_closing();
}

#[test]
fn open_in_panel_navigates_active_pane() {
    // Se abre desde una carpeta temporal: desde "Este equipo" analizaría el disco del usuario.
    let (mut a, from) = app_in("open-desde");
    let tmp = xtree_dir("open");
    assert_eq!(a.panes[a.active].tab().location, Location::Path(from));
    a.run_command(Command::Xtree);
    assert!(a.xtree_open);
    a.xtree_action(crate::xtree::view::Action::OpenInPanel { folder: tmp.clone(), select: None });
    assert!(!a.xtree_open);
    let Location::Path(p) = &a.panes[a.active].tab().location else { panic!("no es una carpeta") };
    assert_eq!(p, &tmp);
    a.on_closing();
}

/// Desde «Este equipo» (sin carpeta) xtree analiza la raíz por defecto del `DiskScan`: con los
/// falsos, una carpeta temporal servida por el listador en memoria, nunca el disco real.
#[test]
fn view_default_root_comes_from_disk_scan() {
    use crate::sys::testing::FakeDiskScan;
    let mut a = app();
    assert!(!matches!(a.panes[a.active].tab().location, Location::Path(_)), "la App en blanco arranca en «Este equipo»");
    a.run_command(Command::Xtree);
    assert!(a.xtree_open);
    let root = FakeDiskScan::root();
    assert_eq!(a.xtree.as_ref().unwrap().root(), root.as_path());
    let t0 = std::time::Instant::now();
    while !FakeDiskScan::was_listed(&root) {
        assert!(t0.elapsed() < std::time::Duration::from_secs(5), "el escaneo no usó el listador del DiskScan");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    a.on_closing();
}

#[test]
fn command_xtree_is_listed() {
    let spec = crate::commands::ALL.iter().find(|s| s.command == Command::Xtree).expect("falta el comando");
    assert_eq!(spec.id, "xtree");
    assert_eq!(spec.name, "Mapa de uso de disco (xtree)");
    assert!(spec.default_keys.is_empty());
}

// ---------- shell ----------

/// Una fila de archivo para cargar a mano en la lista (sin leer el disco).
fn file_entry(name: &str) -> fs::Entry {
    fs::Entry {
        name: name.into(),
        full: None,
        group: 0,
        pidl: None,
        display: name.into(),
        sort_key: name.to_lowercase(),
        is_dir: false,
        size: 0,
        modified: 0,
        created: 0,
        attrs: 0,
        type_name: std::sync::Arc::from("Archivo"),
        icon: 0,
        overlay: 0,
        overlay_checked: true,
    }
}

/// «Copiar las rutas como texto» manda al portapapeles del `Shell` las rutas entre comillas, una
/// por línea.
#[test]
fn copy_paths_uses_shell() {
    use crate::sys::testing::{FakeShell, ShellCall};
    let mut a = app();
    let dir = std::env::temp_dir().join("xdir-tests").join("copiar-rutas");
    a.new_tab(0, Location::Path(dir.clone()), true);
    let list = &mut a.panes[0].tab_mut().list;
    list.append(vec![file_entry("a.txt"), file_entry("b.txt")]);
    list.select_all();
    FakeShell::take_calls();
    a.run_command(Command::CopyPaths);
    let expected = format!("\"{}\"\r\n\"{}\"", dir.join("a.txt").display(), dir.join("b.txt").display());
    assert_eq!(FakeShell::take_calls(), [ShellCall::CopyText(expected)]);
}

/// El menú contextual del fondo de un panel le pasa al `Shell` las entradas propias (Ver, Ordenar
/// por, Actualizar, Pegar, Deshacer) y la carpeta del panel.
#[test]
fn context_menu_own_entries_reach_shell() {
    use crate::sys::testing::{FakeShell, ShellCall};
    let mut a = app();
    let dir = std::env::temp_dir().join("xdir-tests").join("menu-fondo");
    a.new_tab(0, Location::Path(dir.clone()), true);
    FakeShell::take_calls();
    a.on_context_menu(Some((10.0, 20.0)));
    let calls = FakeShell::take_calls();
    let [ShellCall::ContextMenu { folder, items, at, own, .. }] = calls.as_slice() else { panic!("se esperaba un menú: {calls:?}") };
    assert_eq!(folder, &Location::Path(dir));
    assert!(items.is_empty());
    assert_eq!(*at, (10.0, 20.0));
    let labels: Vec<&str> = own
        .iter()
        .map(|e| match e {
            shell::Entry::Item { label, .. } | shell::Entry::Sub(label, _) => label.as_str(),
            shell::Entry::Sep => "-",
        })
        .collect();
    assert_eq!(&labels[..6], ["Ver", "Ordenar por", "Actualizar", "-", "Pegar", "Deshacer"]);
}

// ---------- ventana (WindowHost) ----------

/// El título de la ventana sigue al grupo cargado.
#[test]
fn title_follows_group_name() {
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    a.save_group("Trabajo");
    a.group_name = None;
    a.apply_group(0);
    a.paint();
    assert_eq!(host.titles().last().map(String::as_str), Some("xdir - Trabajo"));
}

/// Punto del panel de colores donde está «Personalizado…».
fn custom_color_point(a: &App) -> (f32, f32) {
    let (w, h) = a.client_dips();
    let r = ui::picker_rect(w, h);
    let mut y = r.top;
    while y < r.bottom {
        let x = (r.left + r.right) / 2.0;
        if ui::picker_at(w, h, x, y) == Some(ui::PickerHit::Custom) {
            return (x, y);
        }
        y += 1.0;
    }
    panic!("no se encontró «Personalizado…»");
}

fn click(a: &mut App, (x, y): (f32, f32)) {
    a.on_left_down(x, y, false);
    a.on_left_up(x, y);
}

/// Si el selector no devuelve color (se canceló, o en macOS llega después), el acento no cambia.
#[test]
fn custom_color_none_keeps_accent() {
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    host.set_color_choice(None);
    let before = a.accent;
    a.picker_open = true;
    let at = custom_color_point(&a);
    click(&mut a, at);
    assert_eq!(a.accent, before);
}

/// El color elegido en el selector modal queda como acento.
#[test]
fn custom_color_some_sets_accent() {
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    host.set_color_choice(Some(0xF85149));
    a.picker_open = true;
    let at = custom_color_point(&a);
    click(&mut a, at);
    assert_eq!(a.accent, Accent::Rgb(0xF85149));
}

/// Si el sistema no puede arrastrar, se avisa en la barra de estado.
#[test]
fn failed_drag_shows_notice() {
    let host = FakeHost::new();
    host.set_drag_result(Err("x".into()));
    let mut a = app_with_host(Init::blank(), host.clone());
    let dir = std::env::temp_dir().join("xdir-tests").join("arrastre");
    a.new_tab(0, Location::Path(dir), true);
    let list = &mut a.panes[0].tab_mut().list;
    list.append(vec![file_entry("a.txt")]);
    list.select_all();
    a.apply(0, Action::BeginDrag);
    assert_eq!(a.notice.as_deref(), Some("No se pudo arrastrar: x"));
    assert_eq!(a.drag, Drag::None);
}

// ---------- dibujo con el host ----------

/// `paint` dibuja a través del host: el pintor falso registra el texto de la barra de estado del
/// panel activo.
#[test]
fn paint_draws_with_host() {
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    a.paint();
    let status = a.panes[a.active].tab().status();
    let texts = host.drawn_texts();
    assert!(texts.iter().any(|(_, s, _)| *s == status), "falta «{status}» entre {:?}", texts.iter().map(|t| &t.1).collect::<Vec<_>>());
}

/// App con el panel 0 en una carpeta temporal, con archivos cuyo nombre más largo tiene 10
/// caracteres, ya dibujada (la lista conoce su área).
fn app_with_files(host: Rc<FakeHost>, name: &str) -> App {
    let mut a = app_with_host(Init::blank(), host);
    let dir = xtree_dir(name);
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir));
    a.set_active(0);
    a.panes[0].tab_mut().list.replace(vec![file_entry("a.txt"), file_entry("abcdefghij"), file_entry("b")]);
    a.paint();
    a
}

/// Punto sobre el borde derecho de la columna Nombre, en el encabezado de la lista del panel 0.
fn name_edge(a: &App) -> (f32, f32) {
    let r = a.panes[0].rect;
    let list = &a.panes[0].tab().list;
    let x_mid = (r.left + r.right) / 2.0;
    let y = (r.top as i32..r.bottom as i32).map(|y| y as f32 + 0.5).find(|&y| list.in_header(x_mid, y)).expect("encabezado");
    let xs: Vec<f32> = (0..((r.right - r.left) * 4.0) as i32).map(|i| r.left + i as f32 / 4.0).collect();
    let start = xs.iter().position(|&x| list.wants_resize_cursor(x, y)).expect("borde del nombre");
    let len = xs[start..].iter().take_while(|&&x| list.wants_resize_cursor(x, y)).count();
    (xs[start + len / 2], y)
}

/// Doble clic en el borde de la columna Nombre: se ajusta con el ancho de texto del host (7 DIPs
/// por carácter en el falso), igual que `fit_column_at` con esa medida.
#[test]
fn double_click_header_fits_column_with_host_width() {
    let (host_a, host_b) = (FakeHost::new(), FakeHost::new());
    let mut a = app_with_files(host_a.clone(), "ajuste");
    let mut b = app_with_files(host_b.clone(), "ajuste");
    let (x, y) = name_edge(&a);
    assert_eq!(name_edge(&b), (x, y));
    a.on_left_down(x, y, true);
    a.on_left_up(x, y);
    let fit = b.panes[0].tab_mut().list.fit_column_at(x, y, &|s, _| 7.0 * s.chars().count() as f32);
    assert!(fit.is_some(), "el punto está sobre el borde");
    assert_ne!(name_edge(&a).0, x, "el borde se movió");
    assert_eq!(name_edge(&a), name_edge(&b));
    a.paint();
    b.paint();
    assert_eq!(host_a.drawn_texts(), host_b.drawn_texts());
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// `--probe <archivo>` pide sesión limpia y el archivo donde dejar las medidas; la carpeta se abre.
#[test]
fn startup_args_reads_probe_and_folder() {
    let tmp = fixture("args-probe");
    let t = tmp.to_string_lossy().to_string();
    assert_eq!(parse_args(&args(&["--probe", "p.txt", &t])), (true, Some("p.txt".to_string()), Some(tmp.clone())));
    // Sin carpeta: solo la medición.
    assert_eq!(parse_args(&args(&["--probe", "p.txt"])), (true, Some("p.txt".to_string()), None));
}

/// Solo una carpeta: se abre (si existe) y la sesión es la guardada.
#[test]
fn startup_args_folder_only() {
    let tmp = fixture("args-folder");
    assert_eq!(parse_args(&args(&[&tmp.to_string_lossy()])), (false, None, Some(tmp.clone())));
    let missing = tmp.join("no-existe");
    assert_eq!(parse_args(&args(&[&missing.to_string_lossy()])), (false, None, None));
    assert_eq!(parse_args(&[]), (false, None, None));
}

// ---------- tanda A: selección, copiar como ruta, extensiones ----------

/// Espera a que la pestaña activa del panel 0 termine de leer su carpeta (lo que haría el
/// `WM_APP_LOAD` en la ventana).
fn wait_loaded(a: &mut App) {
    while a.panes[0].tab().loading {
        let ev = a.rx.recv_timeout(std::time::Duration::from_secs(10)).expect("la carga termina");
        if let Some(tab) = a.tab_by_id(ev.panel) {
            tab.on_load(ev);
        }
    }
}

/// Con algo seleccionado la barra de estado dice cuántos y cuánto pesan los archivos; las
/// carpetas no suman y, si solo hay carpetas, no hay tamaño.
#[test]
fn status_shows_selection_size() {
    let (mut a, dir) = app_in("seleccion-tamano");
    std::fs::write(dir.join("uno.bin"), vec![0u8; 1000]).unwrap();
    std::fs::write(dir.join("tres.bin"), vec![0u8; 3000]).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.panes[0].tab_mut().reload(hwnd, &tx, true);
    wait_loaded(&mut a);
    a.set_active(0);
    let list = &mut a.panes[0].tab_mut().list;
    assert_eq!(list.count(), 3);
    list.select_all();
    let status = a.panes[0].tab().status();
    assert_eq!(status, format!("3 elementos   ·   {}", crate::tab::selection_summary(3, Some(4000))));
    assert!(status.contains(&format!("3 elementos seleccionados · {}", crate::format::size_short(4000))), "{status}");
    let list = &mut a.panes[0].tab_mut().list;
    let sub = list.entries.iter().position(|e| e.is_dir).unwrap();
    list.select_index(sub);
    let status = a.panes[0].tab().status();
    assert_eq!(status, "3 elementos   ·   1 elemento seleccionado");
    let _ = std::fs::remove_dir_all(&dir);
}

/// «Copiar como ruta de acceso» del menú contextual de los elementos copia la selección entre
/// comillas, una ruta por línea.
#[test]
fn copy_as_path_quotes() {
    use crate::sys::testing::{FakeShell, ShellCall};
    let mut a = app();
    let dir = std::env::temp_dir().join("xdir-tests").join("copiar-como-ruta");
    a.new_tab(0, Location::Path(dir.clone()), true);
    let list = &mut a.panes[0].tab_mut().list;
    list.append(vec![file_entry("a.txt"), file_entry("b.txt")]);
    list.select_all();
    // Primer menú: se mira qué entradas propias trae y no se elige nada.
    FakeShell::choose(None);
    FakeShell::take_calls();
    a.on_context_menu(Some((10.0, 20.0)));
    let calls = FakeShell::take_calls();
    let [ShellCall::ContextMenu { items, own, .. }] = calls.as_slice() else { panic!("se esperaba un menú: {calls:?}") };
    assert_eq!(items.len(), 2);
    let id = own
        .iter()
        .find_map(|e| match e {
            shell::Entry::Item { id, label, .. } if label == "Copiar como ruta de acceso" => Some(*id),
            _ => None,
        })
        .expect("falta «Copiar como ruta de acceso»");
    // Segundo menú: se elige la entrada.
    FakeShell::choose(Some(&format!("own:{id}")));
    a.on_context_menu(Some((10.0, 20.0)));
    FakeShell::choose(None);
    let copied: Vec<String> = FakeShell::take_calls()
        .into_iter()
        .filter_map(|c| match c {
            ShellCall::CopyText(t) => Some(t),
            _ => None,
        })
        .collect();
    let [text] = copied.as_slice() else { panic!("se esperaba una copia: {copied:?}") };
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines, [format!("\"{}\"", dir.join("a.txt").display()), format!("\"{}\"", dir.join("b.txt").display())]);
    assert_eq!(*text, quoted_paths(&[dir.join("a.txt"), dir.join("b.txt")]));
}

/// Punto del panel de la rueda dentada sobre un elemento.
fn picker_point(a: &App, want: ui::PickerHit) -> (f32, f32) {
    let (w, h) = a.client_dips();
    let r = ui::picker_rect(w, h);
    let x = (r.left + r.right) / 2.0;
    let mut y = r.top;
    while y < r.bottom {
        if ui::picker_at(w, h, x, y) == Some(want) {
            return (x, y);
        }
        y += 1.0;
    }
    panic!("no se encontró {want:?}");
}

/// El switch «Mostrar extensiones» se apaga desde la rueda dentada, se guarda en la sesión y, apagado,
/// la lista dibuja los nombres de archivo sin la extensión.
#[test]
fn extensions_switch_persists() {
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    assert!(a.show_extensions, "prendido por defecto");
    a.picker_open = true;
    a.paint();
    assert!(host.drawn_texts().iter().any(|(_, s, _)| s == "Mostrar extensiones"));
    let at = picker_point(&a, ui::PickerHit::Extensions);
    click(&mut a, at);
    assert!(!a.show_extensions);
    assert!(a.panes.iter().flat_map(|p| p.tabs.iter()).all(|t| !t.list.show_extensions));
    let s = a.session();
    assert!(!s.show_extensions);
    session::save(&s).unwrap();
    let loaded = session::load().expect("sesión guardada");
    assert!(!loaded.show_extensions);
    let mut init = Init::blank();
    init.session = loaded;
    let host = FakeHost::new();
    let mut b = app_with_host(init, host.clone());
    assert!(!b.show_extensions);
    // Las pestañas nuevas también nacen sin extensiones.
    b.new_tab(0, Location::Computer, true);
    assert!(!b.panes[0].tab().list.show_extensions);
    let dir = xtree_dir("extensiones");
    let (hwnd, tx) = (b.hwnd, b.tx.clone());
    b.panes[0].tab_mut().navigate(hwnd, &tx, Location::Path(dir));
    b.set_active(0);
    b.panes[0].tab_mut().list.replace(vec![file_entry("foto.jpg")]);
    b.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|t| t.1).collect();
    assert!(texts.iter().any(|s| s == "foto"), "{texts:?}");
    assert!(!texts.iter().any(|s| s == "foto.jpg"), "{texts:?}");
}

/// «Seleccionar por patrón…» abre la paleta en modo patrón: lo escrito es la única fila, Enter
/// suma a la selección los que coinciden y cierra; Esc cierra sin cambios.
#[test]
fn palette_pattern_mode_selects_on_enter() {
    use crate::host::{TextKind, WM_APP_PALETTE_KEY};
    let host = FakeHost::new();
    let mut a = app_with_host(Init::blank(), host.clone());
    let dir = std::env::temp_dir().join("xdir-tests").join("seleccion-patron");
    a.new_tab(0, Location::Path(dir), true);
    a.set_active(0);
    a.panes[0].tab_mut().list.append(vec![file_entry("a.jpg"), file_entry("B.JPG"), file_entry("c.png")]);
    let selected = |a: &App| a.panes[0].tab().list.selected_entries().map(|e| e.name.to_string_lossy().into_owned()).collect::<Vec<_>>();

    // Esc: se cierra sin tocar la selección.
    a.run_command(Command::SelectPattern);
    let (pal, edit) = a.palette.as_ref().expect("se abrió la paleta");
    assert_eq!(pal.mode, PaletteMode::Pattern { select: true });
    let edit = *edit;
    host.text_set(edit, "*.jpg");
    a.on_text_changed(edit.0 as isize);
    assert!(a.on_app_message(WM_APP_PALETTE_KEY, VK_ESCAPE.0 as usize, 0));
    assert!(a.palette.is_none());
    assert!(selected(&a).is_empty());

    // Enter: aplica y cierra.
    a.run_command(Command::SelectPattern);
    let edit = a.palette.as_ref().unwrap().1;
    host.text_set(edit, "*.jpg");
    a.on_text_changed(edit.0 as isize);
    let titles: Vec<String> = a.palette.as_ref().unwrap().0.items.iter().map(|i| i.title.clone()).collect();
    assert_eq!(titles, ["Seleccionar los que coinciden con «*.jpg»"]);
    assert!(a.on_app_message(WM_APP_PALETTE_KEY, VK_RETURN.0 as usize, 0));
    assert!(a.palette.is_none());
    assert_eq!(host.text_of(TextKind::Palette), None);
    assert_eq!(selected(&a), ["a.jpg", "B.JPG"]);

    // Deseleccionar.
    a.run_command(Command::DeselectPattern);
    let edit = a.palette.as_ref().unwrap().1;
    host.text_set(edit, "b.*");
    a.on_text_changed(edit.0 as isize);
    let titles: Vec<String> = a.palette.as_ref().unwrap().0.items.iter().map(|i| i.title.clone()).collect();
    assert_eq!(titles, ["Deseleccionar los que coinciden con «b.*»"]);
    assert!(a.on_app_message(WM_APP_PALETTE_KEY, VK_RETURN.0 as usize, 0));
    assert_eq!(selected(&a), ["a.jpg"]);
}

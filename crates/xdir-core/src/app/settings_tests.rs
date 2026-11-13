//! Pruebas de la pestaña Configuración: una sola, entre paneles, sesión, comandos y dibujo.

use super::*;
use crate::keys::keys::{VK_CONTROL, VK_SHIFT};
use crate::commands;
use crate::settings::{AccentChoice, Control, Item, PageId, Row, Section, SettingsHit};
use crate::sys::testing::{FakeEnv, FakeFileSystem, FakeHost, FakeShell};

fn app() -> App {
    crate::sys::testing::install_fakes();
    App::new(Init::blank(), FakeHost::new())
}

fn app_host() -> (App, Rc<FakeHost>) {
    crate::sys::testing::install_fakes();
    let host = FakeHost::new();
    (App::new(Init::blank(), host.clone()), host)
}

fn settings_count(a: &App) -> usize {
    a.panes.iter().flat_map(|p| p.tabs.iter()).filter(|t| t.is_settings()).count()
}

fn page(a: &App) -> PageId {
    a.settings_tab().and_then(|(p, t)| a.panes[p].tabs[t].settings.as_ref()).map(|v| v.page).expect("hay configuración")
}

/// Una tecla con los modificadores apretados.
fn press(a: &mut App, mods: &[crate::keys::Key], vk: crate::keys::Key) -> bool {
    FakeEnv::hold_keys(mods);
    let used = a.on_key(vk);
    FakeEnv::hold_keys(&[]);
    used
}

#[test]
fn settings_opens_once() {
    let mut a = app();
    let before = a.panes[0].tabs[0].location.clone();
    a.run_command(Command::Settings);
    a.run_command(Command::Settings);
    assert_eq!(settings_count(&a), 1);
    assert_eq!(a.panes[0].tabs.len(), 2, "abre una pestaña nueva, no reemplaza la carpeta");
    assert_eq!(a.panes[0].tabs[0].location, before);
    assert!(a.panes[0].tab().is_settings());
}

#[test]
fn settings_focuses_existing_in_other_pane() {
    let mut a = app();
    a.set_active(2);
    a.run_command(Command::Settings);
    a.set_active(0);
    a.run_command(Command::Settings);
    assert_eq!(settings_count(&a), 1);
    assert_eq!(a.active, 2);
    assert!(a.panes[2].tab().is_settings());
}

#[test]
fn settings_moves_not_duplicates() {
    let mut a = app();
    a.run_command(Command::Settings);
    let (pane, index) = a.settings_tab().unwrap();
    a.move_tab_to_pane(pane, index, 1, 0);
    a.set_active(0);
    a.run_command(Command::Settings);
    assert_eq!(settings_count(&a), 1);
    assert_eq!(a.settings_tab(), Some((1, 0)));
    assert_eq!(a.active, 1);
}

#[test]
fn settings_close_and_reopen() {
    let mut a = app();
    a.run_command(Command::SettingsKeys);
    assert!(press(&mut a, &[VK_CONTROL], Key(b'W' as u16)));
    assert_eq!(settings_count(&a), 0);
    a.run_command(Command::ReopenTab);
    assert_eq!(settings_count(&a), 1);
    // Reabrir con otra ya abierta la enfoca (no hay dos).
    a.run_command(Command::NewTab);
    let i = a.panes[0].active;
    let at = a.panes[0].tabs.iter().position(|t| t.is_settings()).unwrap();
    a.close_tab(0, at);
    a.run_command(Command::Settings);
    let n = a.panes[0].tabs.len();
    a.panes[0].select_tab(i.min(n - 1));
    a.run_command(Command::ReopenTab);
    assert_eq!(settings_count(&a), 1);
}

#[test]
fn settings_not_loaded_nor_watched() {
    let mut a = app();
    a.run_command(Command::Settings);
    a.sync_visibility();
    a.run_command(Command::Refresh);
    let tab = a.panes[0].tab();
    assert!(tab.is_loaded() && !tab.loading);
    assert_eq!(FakeFileSystem::loads(std::path::Path::new("xdir:configuracion")), 0);
    assert_eq!(tab.status(), "Configuración   ·   los cambios se guardan solos");
}

#[test]
fn settings_session_roundtrip() {
    let mut a = app();
    a.run_command(Command::SettingsKeys);
    let s = a.session();
    assert_eq!(s.settings_page.as_deref(), Some("atajos"));
    assert!(s.panes[0].tabs.contains(&Location::Settings));
    let mut init = Init::blank();
    init.session = s;
    let b = App::new(init, FakeHost::new());
    assert_eq!(page(&b), PageId::Builtin(Section::Keys));
    // Una página que no existe vuelve a Apariencia.
    let mut init = Init::blank();
    init.session.panes[1].tabs = vec![Location::Settings, Location::Settings];
    init.session.settings_page = Some("nada".into());
    let c = App::new(init, FakeHost::new());
    assert_eq!(settings_count(&c), 1, "una sola aunque la sesión traiga dos");
    assert_eq!(page(&c), PageId::Builtin(Section::Appearance));
}

#[test]
fn settings_title_and_crumbs() {
    let (mut a, host) = app_host();
    a.run_command(Command::SettingsKeys);
    let tab = a.panes[0].tab();
    assert_eq!(tab.location.title(), "Configuración");
    assert!(!tab.can_back() && !tab.can_forward() && !tab.can_up());
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(texts.iter().any(|t| t == "Configuración"));
    assert!(texts.iter().filter(|t| *t == "Atajos de teclado").count() >= 2, "en las migas, la lista y el título: {texts:?}");
    // Ir a otro lado desde la configuración la reemplaza y no queda en Atrás.
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.tab_mut().navigate(hwnd, &tx, Location::Computer);
    assert!(a.panes[0].tab().settings.is_none() && !a.panes[0].tab().can_back());
}

#[test]
fn settings_keys_command_opens_keys_page() {
    let mut a = app();
    a.run_command(Command::Settings);
    assert_eq!(page(&a), PageId::Builtin(Section::Appearance));
    a.run_command(Command::SettingsKeys);
    assert_eq!(page(&a), PageId::Builtin(Section::Keys));
    assert_eq!(settings_count(&a), 1);
}

#[test]
fn picker_more_options_opens_settings() {
    let mut a = app();
    a.picker_open = true;
    let (w, h) = a.client_dips();
    let at = (0..w as i32)
        .step_by(4)
        .flat_map(|x| (0..h as i32).step_by(4).map(move |y| (x as f32, y as f32)))
        .find(|&(x, y)| ui::picker_at(w, h, x, y) == Some(ui::PickerHit::MoreOptions))
        .expect("la rueda tiene «Más opciones…»");
    a.on_left_down(at.0, at.1, false);
    assert!(!a.picker_open);
    assert_eq!(settings_count(&a), 1);
    assert!(a.panes[a.active].tab().is_settings());
}

#[test]
fn folder_commands_ignored_in_settings() {
    let mut a = app();
    a.run_command(Command::Settings);
    FakeShell::take_calls();
    let views_before = a.folder_views.default_view().clone();
    press(&mut a, &[VK_CONTROL], Key(b'C' as u16));
    press(&mut a, &[VK_CONTROL, VK_SHIFT], Key(b'1' as u16));
    a.run_command(Command::View(1));
    a.run_command(Command::NewFolder);
    assert!(FakeShell::take_calls().is_empty());
    assert_eq!(a.folder_views.default_view(), &views_before);
    assert!(a.panes[0].tab().is_settings());
    // Pestaña nueva desde la configuración: «Este equipo», no otra configuración.
    a.run_command(Command::NewTab);
    assert_eq!(settings_count(&a), 1);
    assert_eq!(a.panes[0].tab().location, Location::Computer);
}

#[test]
fn settings_command_shortcuts() {
    let spec = commands::ALL.iter().find(|s| s.id == "configuracion").unwrap();
    assert_eq!((spec.command, spec.name), (Command::Settings, "Configuración"));
    let c = commands::parse_combo(spec.default_keys[0]).unwrap();
    assert_eq!(commands::combo_label_in(sys::KeyStyle::Mac, c), "⌘,");
    assert_eq!(commands::combo_label_in(sys::KeyStyle::Windows, c), "Ctrl+,");
    let keys = commands::ALL.iter().find(|s| s.id == "configuracion_atajos").unwrap();
    assert_eq!((keys.command, keys.name, keys.default_keys.len()), (Command::SettingsKeys, "Configuración: Atajos de teclado", 0));
    assert_eq!(commands::ALL.iter().find(|s| s.id == "abrir_configuracion").unwrap().name, "Abrir config.toml");
    // Ctrl+Coma abre la pestaña.
    let mut a = app();
    assert!(press(&mut a, &[VK_CONTROL], Key(0xBC)));
    assert_eq!(settings_count(&a), 1);
}

#[test]
fn draw_settings_frame() {
    let (mut a, host) = app_host();
    // Un solo panel: con «Red» la lista de secciones es más larga.
    a.set_layout(Layout::One);
    a.run_command(Command::Settings);
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(texts.iter().any(|t| t == "Buscar en la configuración"), "{texts:?}");
    for s in Section::ALL {
        assert!(texts.iter().any(|t| t == s.title()), "falta {}", s.title());
    }
    // El título de la página (y la miga).
    assert!(texts.iter().filter(|t| *t == "Apariencia").count() >= 3, "{texts:?}");
    // La lista de la izquierda se puede cliquear.
    let (pane, _) = a.settings_tab().unwrap();
    let nav = a.settings_view(pane).unwrap().hits.iter().find(|(_, h)| *h == SettingsHit::Nav(PageId::Builtin(Section::Tree))).map(|(r, _)| *r).unwrap();
    a.on_left_down(nav.left + 4.0, nav.top + 4.0, false);
    assert_eq!(page(&a), PageId::Builtin(Section::Tree));
}

/// Ctrl+L (escribir una ruta) con la configuración a la vista abre su buscador.
#[test]
fn edit_address_focuses_settings_search() {
    let (mut a, host) = app_host();
    a.run_command(Command::Settings);
    a.paint();
    a.run_command(Command::EditAddress);
    assert!(a.edit.is_none());
    assert!(a.filter_edit.is_some());
    assert!(host.text_of(TextKind::Filter).is_some());
}

// ---------- Apariencia, Lista y vistas, Árbol, Vista previa y config.toml ----------

/// La configuración abierta en un solo panel grande, ya dibujada (con sus lugares).
fn settings_app(page: Section) -> (App, Rc<FakeHost>) {
    let (mut a, host) = app_host();
    host.client.set((1800.0, 1400.0));
    a.set_layout(Layout::One);
    a.run_command(Command::Settings);
    if let Some(v) = a.settings_view_mut(0) {
        v.set_page(PageId::Builtin(page));
    }
    a.paint();
    (a, host)
}

/// Un clic en el centro de una parte de una fila (según el último cuadro).
fn click(a: &mut App, id: &'static str, part: usize) {
    let r = a.part_rect(0, id, part).unwrap_or_else(|| panic!("no se dibujó {id}/{part}"));
    a.on_left_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, false);
    a.on_left_up((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    a.paint();
}

/// Espera a que terminen las escrituras de `config.toml` en camino.
fn wait_config(a: &mut App) {
    while a.config_saving {
        let t = a.task_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("la escritura termina");
        a.on_task_done(t);
    }
}

fn config_text() -> String {
    std::fs::read_to_string(config::file_path().unwrap()).unwrap_or_default()
}

/// Empieza sin `config.toml` (la carpeta de configuración es propia del hilo de la prueba).
fn fresh_config() {
    crate::sys::testing::install_fakes();
    let _ = std::fs::remove_file(config::file_path().unwrap());
}

#[test]
fn theme_three_states() {
    let (mut a, _) = settings_app(Section::Appearance);
    click(&mut a, "tema", 2);
    assert_eq!(a.dark_mode, None);
    click(&mut a, "tema", 1);
    assert_eq!(a.dark_mode, Some(true));
    click(&mut a, "tema", 0);
    assert_eq!(a.dark_mode, Some(false));
    // El switch de la rueda sigue fijando un modo.
    a.set_theme_mode(None);
    a.set_dark_mode(!a.theme.dark);
    assert!(a.dark_mode.is_some());
}

#[test]
fn accent_preset_and_system() {
    let (mut a, host) = settings_app(Section::Appearance);
    click(&mut a, "acento", 3);
    assert_eq!(a.accent, Accent::Rgb(theme::PRESETS[3].1));
    click(&mut a, "acento", theme::PRESETS.len());
    assert_eq!(a.accent, Accent::System);
    host.set_color_choice(Some(0x123456));
    click(&mut a, "acento", theme::PRESETS.len() + 1);
    assert_eq!(a.accent, Accent::Rgb(0x123456));
    assert!(a.settings_items(Section::Appearance).iter().any(|i| matches!(i, Item::Row(r) if r.control == Control::Accent(AccentChoice::Other(0x123456)))));
}

#[test]
fn animations_writes_and_removes_line() {
    fresh_config();
    let (mut a, _) = settings_app(Section::Appearance);
    click(&mut a, "animaciones", 1);
    assert_eq!(a.config.animations, Some(true));
    wait_config(&mut a);
    let text = config_text();
    assert!(text.contains("# animaciones = true\nanimaciones = true\n"), "{text}");
    click(&mut a, "animaciones", 0);
    wait_config(&mut a);
    assert_eq!(a.config.animations, None);
    let text = config_text();
    assert!(text.contains("# animaciones = true\n") && !text.contains("\nanimaciones = true"), "{text}");
    assert_eq!(text, config::template(), "volver a Sistema deja la plantilla como estaba");
}

#[test]
fn config_writes_are_serialized() {
    fresh_config();
    let (mut a, _) = settings_app(Section::Appearance);
    // Sin apagar las animaciones: es un estado del proceso y lo miran otras pruebas.
    a.set_animations(Some(true));
    a.set_animations(None);
    a.set_animations(Some(true));
    assert!(a.config_saving, "una escritura en camino");
    assert_eq!(a.config_pending.len(), 2, "las otras esperan");
    wait_config(&mut a);
    assert!(a.config_pending.is_empty());
    assert_eq!(a.config.animations, Some(true));
    let text = config_text();
    assert_eq!(text.matches("\nanimaciones = ").count(), 1, "{text}");
    assert!(text.contains("\nanimaciones = true\n"));
}

#[test]
fn edit_after_external_change_keeps_both() {
    fresh_config();
    let (mut a, _) = settings_app(Section::Appearance);
    a.set_animations(Some(true));
    wait_config(&mut a);
    // Cambio a mano entre dos cambios de la pestaña, con \r\n y sin salto final.
    let hand = config_text().replace('\n', "\r\n") + "\r\n[colores]\r\n\"*.log\" = \"#112233\" # mío";
    std::fs::write(config::file_path().unwrap(), &hand).unwrap();
    a.set_animations(None);
    wait_config(&mut a);
    let text = config_text();
    assert!(text.contains("[colores]\r\n\"*.log\" = \"#112233\" # mío"), "{text}");
    assert!(text.contains("\r\n# animaciones = true\r\n") && !text.contains("\nanimaciones = true"));
    assert!(!text.ends_with('\n'));
    assert!(a.config.colors.iter().any(|r| config::rule_label(r) == "*.log"));
    assert_eq!(a.config.animations, None);
}

#[test]
fn config_write_error_shown() {
    fresh_config();
    let (mut a, host) = settings_app(Section::Appearance);
    // Un archivo que no es texto: no se pisa y se avisa.
    let path = config::file_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0xFFu8, 0xFE, 0x00]).unwrap();
    a.set_animations(Some(true));
    wait_config(&mut a);
    assert_eq!(std::fs::read(&path).unwrap(), [0xFFu8, 0xFE, 0x00]);
    assert_eq!(a.config.animations, Some(true), "lo de memoria queda");
    a.paint();
    assert!(host.drawn_texts().iter().any(|(_, t, _)| t.starts_with("No se pudo guardar config.toml:")));
    fresh_config();
}

#[test]
fn own_write_not_reported_as_external() {
    fresh_config();
    let (mut a, _) = settings_app(Section::Appearance);
    a.set_animations(Some(true));
    wait_config(&mut a);
    a.notice = None;
    a.reload_config();
    assert_eq!(a.notice, None);
    assert!(!a.config.changed_on_disk());
}

#[test]
fn list_switches_mirror_picker() {
    let (mut a, _) = settings_app(Section::ListViews);
    click(&mut a, "casillas", 0);
    assert!(a.picker_switches().checkboxes);
    assert!(a.panes[0].tabs.iter().all(|t| t.list.checkboxes));
    click(&mut a, "extensiones", 0);
    assert!(!a.picker_switches().extensions);
    click(&mut a, "ocultos", 0);
    assert!(a.picker_switches().hidden && a.show_hidden);
    click(&mut a, "colores_tipo", 0);
    assert!(a.file_colors);
    // Y al revés: lo que cambia la rueda se ve en la pestaña.
    a.set_checkboxes(false);
    assert!(a.settings_items(Section::ListViews).iter().any(|i| matches!(i, Item::Row(r) if r.id == "casillas" && r.control == Control::Switch(false))));
}

#[test]
fn default_view_keeps_custom_folders() {
    let (mut a, host) = settings_app(Section::ListViews);
    let own = std::env::temp_dir().join(format!("xdir-vista-propia-{}", std::process::id()));
    std::fs::create_dir_all(&own).unwrap();
    let key = crate::folder_views::folder_key(&Location::Path(own.clone()), None);
    a.folder_views.set(&key, crate::folder_views::ViewSettings { view: list::View::Icons, ..Default::default() });
    host.set_menu_choice(Some(5)); // Lista (quinta de list::VIEWS)
    click(&mut a, "vista_defecto", 0);
    assert_eq!(a.folder_views.default_view().view, list::View::List);
    assert_eq!(a.folder_views.get(&key).view, list::View::Icons);
    a.new_tab(0, Location::Path(own.clone()), true);
    assert_eq!(a.panes[0].tab().list.view, list::View::Icons);
    a.new_tab(0, Location::Path(std::env::temp_dir()), true);
    assert_eq!(a.panes[0].tab().list.view, list::View::List);
    // «Restablecer las vistas…» pregunta y vuelve todo a fábrica.
    a.run_command(Command::Settings);
    a.paint();
    host.set_answer(Some(true));
    click(&mut a, "vistas_carpetas", 1);
    assert_eq!(a.folder_views.default_view().view, list::View::Details);
    assert!(!a.folder_views.contains(&key));
    let _ = std::fs::remove_dir_all(&own);
}

#[test]
fn tree_follow_back_to_config() {
    let (mut a, _) = settings_app(Section::Tree);
    let before = a.tree_follows();
    click(&mut a, "arbol_sigue", 0);
    assert_eq!(a.tree_follow, Some(!before));
    click(&mut a, "arbol_config", 0);
    assert_eq!(a.tree_follow, None);
    assert_eq!(a.tree_follows(), a.config.tree_follows_pane);
    // Sin cambio propio el botón está apagado.
    assert!(a.part_rect(0, "arbol_config", 0).is_none());
}

#[test]
fn preview_width_slider_clamps() {
    let (mut a, _) = settings_app(Section::Preview);
    let r = a.part_rect(0, "vista_previa_ancho", 0).unwrap();
    a.on_left_down(r.left + 20.0, (r.top + r.bottom) / 2.0, false);
    a.on_mouse_move(r.left - 500.0, r.top);
    assert_eq!(a.preview.width, 240.0);
    a.on_mouse_move(r.right + 500.0, r.top);
    assert_eq!(a.preview.width, 640.0);
    a.on_left_up(r.right + 500.0, r.top);
    assert_eq!(a.drag, Drag::None);
}

#[test]
fn color_rules_listed_own_first() {
    let mut init = Init::blank();
    init.config = Config::parse("[colores]\n\"*.log\" = \"#112233\"\n");
    let mut a = App::new(init, FakeHost::new());
    a.run_command(Command::Settings);
    let items = a.settings_items(Section::ListViews);
    let rules = items
        .iter()
        .find_map(|i| match i {
            Item::Row(r) if r.id == "reglas_color" => match &r.control {
                Control::ColorRules(rules) => Some(rules.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(rules[0].0, "*.log");
    assert!(rules[0].2, "propia");
    assert_eq!(rules[1].0, "*.pdf");
    assert!(!rules[1].2, "de fábrica");
}

// ---------- xtree y Avanzado ----------

#[test]
fn xtree_open_uses_option() {
    use crate::config::XtreeOpen;
    use crate::sys::testing::FakeDiskScan;
    let mut a = app();
    let dir = std::env::temp_dir();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.tab_mut().navigate(hwnd, &tx, Location::Path(dir.clone()));
    let Location::Path(here) = a.panes[0].tab().location.clone() else { panic!() };
    assert_eq!(a.config.xtree_open, XtreeOpen::Pane);
    assert_eq!(a.xtree_root(), here);
    a.config.xtree_open = XtreeOpen::System;
    assert_eq!(a.xtree_root(), FakeDiskScan::root());
    a.config.xtree_open = XtreeOpen::Last;
    let last = std::env::temp_dir().join(format!("xdir-xtree-ultimo-{}", std::process::id()));
    std::fs::create_dir_all(&last).unwrap();
    a.xtree_last = Some(last.clone());
    assert_eq!(a.xtree_root(), last);
    std::fs::remove_dir_all(&last).unwrap();
    assert_eq!(a.xtree_root(), here, "si ya no existe, la del panel");
    // Cada análisis nuevo anota su raíz (y va a la sesión).
    a.config.xtree_open = XtreeOpen::Pane;
    a.config.xtree_scan_on_open = true;
    a.run_command(Command::Xtree);
    assert_eq!(a.session().xtree_last, Some(here));
}

#[test]
fn xtree_starts_by_type_from_config() {
    let mut a = app();
    a.config.xtree_by_type = true;
    a.run_command(Command::Xtree);
    assert!(a.xtree.as_ref().unwrap().by_type());
    let mut b = app();
    b.run_command(Command::Xtree);
    assert!(!b.xtree.as_ref().unwrap().by_type());
}

/// De fábrica xtree no analiza al abrirse: muestra la vista vacía con la raíz elegida, sin escaneo,
/// y la sesión no anota una raíz que no se analizó.
#[test]
fn xtree_opens_without_scanning_by_default() {
    let mut a = app();
    assert!(!a.config.xtree_scan_on_open);
    a.run_command(Command::Xtree);
    let x = a.xtree.as_ref().unwrap();
    assert!(a.xtree_open && !x.scanned());
    assert_eq!(x.root(), a.xtree_root());
    assert_eq!(a.xtree_badge(), crate::xtree::view::XtreeBadge::Idle);
    assert_eq!(a.session().xtree_last, None);
    // Al cerrar y volver a abrir se ve lo mismo (no se crea otra vista).
    a.run_command(Command::Xtree);
    a.run_command(Command::Xtree);
    assert!(!a.xtree.as_ref().unwrap().scanned());
}

/// Una vista que no analizó nada no se guarda: al volver a abrir xtree en otra carpeta, el
/// desplegable muestra la que corresponde, y si mientras tanto se prendió la opción, analiza.
#[test]
fn xtree_unscanned_reopen_follows_pane_and_option() {
    let mut a = app();
    let base = std::env::temp_dir().join(format!("xdir-xtree-reabrir-{}", std::process::id()));
    let (d1, d2) = (base.join("uno"), base.join("dos"));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.tab_mut().navigate(hwnd, &tx, Location::Path(d1));
    a.run_command(Command::Xtree);
    a.run_command(Command::Xtree);
    a.tab_mut().navigate(hwnd, &tx, Location::Path(d2));
    a.run_command(Command::Xtree);
    assert_eq!(a.xtree.as_ref().unwrap().root(), a.xtree_root());
    a.run_command(Command::Xtree);
    a.config.xtree_scan_on_open = true;
    a.run_command(Command::Xtree);
    assert!(a.xtree.as_ref().unwrap().scanned());
    a.on_closing();
    let _ = std::fs::remove_dir_all(&base);
}

/// Lo elegido a mano en el desplegable de una vista sin analizar se conserva al volver a abrirla
/// (aunque el panel esté en otra carpeta).
#[test]
fn xtree_unscanned_reopen_keeps_user_pick() {
    let mut a = app();
    let base = std::env::temp_dir().join(format!("xdir-xtree-eleccion-{}", std::process::id()));
    let (d1, d2, elegido) = (base.join("uno"), base.join("dos"), base.join("elegido"));
    for d in [&d1, &d2, &elegido] {
        std::fs::create_dir_all(d).unwrap();
    }
    let (hwnd, tx) = (a.hwnd, a.tx.clone());
    a.tab_mut().navigate(hwnd, &tx, Location::Path(d1));
    a.run_command(Command::Xtree);
    a.xtree.as_mut().unwrap().set_pick(elegido.clone());
    a.run_command(Command::Xtree);
    a.tab_mut().navigate(hwnd, &tx, Location::Path(d2));
    a.run_command(Command::Xtree);
    let x = a.xtree.as_ref().unwrap();
    assert!(!x.scanned());
    assert_eq!(x.pick(), elegido.as_path());
    a.on_closing();
    let _ = std::fs::remove_dir_all(&base);
}

/// Con «Analizar al abrir xtree» encendido, como antes: arranca el análisis.
#[test]
fn xtree_scans_on_open_with_option() {
    let mut a = app();
    a.config.xtree_scan_on_open = true;
    a.run_command(Command::Xtree);
    assert!(a.xtree.as_ref().unwrap().scanned());
    a.on_closing();
}

/// El switch de Configuración ▸ xtree escribe `xtree_analizar_al_abrir`.
#[test]
fn xtree_scan_on_open_switch_writes_option() {
    fresh_config();
    let (mut a, _host) = settings_app(Section::Xtree);
    click(&mut a, "xtree_analizar_al_abrir", 0);
    assert!(a.config.xtree_scan_on_open);
    wait_config(&mut a);
    assert!(config_text().contains("# xtree_analizar_al_abrir = false\nxtree_analizar_al_abrir = true\n"), "{}", config_text());
    click(&mut a, "xtree_analizar_al_abrir", 0);
    assert!(!a.config.xtree_scan_on_open);
    wait_config(&mut a);
    assert!(!config_text().contains("\nxtree_analizar_al_abrir = true"));
}

#[test]
fn xtree_switch_writes_option() {
    fresh_config();
    let (mut a, host) = settings_app(Section::Xtree);
    click(&mut a, "xtree_por_tipo", 0);
    assert!(a.config.xtree_by_type);
    wait_config(&mut a);
    assert!(config_text().contains("# xtree_por_tipo = false\nxtree_por_tipo = true\n"));
    host.set_menu_choice(Some(3));
    click(&mut a, "xtree_abrir", 0);
    wait_config(&mut a);
    assert_eq!(a.config.xtree_open, config::XtreeOpen::Last);
    assert!(config_text().contains("\nxtree_abrir = \"ultimo\"\n"));
    // «Ver los atajos» lleva a la página Atajos.
    click(&mut a, "ver_atajos", 0);
    assert_eq!(page(&a), PageId::Builtin(Section::Keys));
}

fn row_of(items: &[Item], id: &str) -> Row {
    items.iter().find_map(|i| match i {
        Item::Row(r) if r.id == id => Some(r.clone()),
        _ => None,
    }).unwrap_or_else(|| panic!("falta la fila {id}"))
}

#[test]
fn advanced_lists_files_and_dir() {
    let a = app();
    let items = a.settings_items(Section::Advanced);
    let Control::Table(files) = row_of(&items, "archivos").control else { panic!() };
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["config.toml", "sesion.toml", "carpetas.toml", "favoritos.txt", "grupos.toml", "historial.txt", "red.toml"]);
    let dir = row_of(&items, "carpeta_config");
    assert_eq!(dir.desc, session::config_dir().unwrap().display().to_string());
    assert_eq!(dir.control, Control::Buttons(vec![("Mostrar en el Explorador".to_string(), true)]));
    FakeEnv::set_key_style(sys::KeyStyle::Mac);
    let items = a.settings_items(Section::Advanced);
    FakeEnv::set_key_style(sys::KeyStyle::Windows);
    let Control::Table(files) = row_of(&items, "archivos").control else { panic!() };
    assert_eq!(files.last().unwrap().0, "rutas.txt");
}

/// #57: las teclas de xtree dicen el atajo en uso de los colores (ya no la T) y las letras; en
/// Atajos de teclado, «xtree_colores» va en el grupo xtree y Ctrl+T no choca con «Pestaña nueva».
#[test]
fn xtree_keys_show_colors_shortcut() {
    let mut a = app();
    let Control::Table(rows) = row_of(&a.settings_items(Section::Xtree), "xtree_teclas").control else { panic!() };
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(&keys[..4], ["Letras", "Ctrl+T", "Ctrl+F", "Ctrl+Shift+A"]);
    assert!(!keys.contains(&"A"), "la A sola ya no hace nada");
    assert!(!keys.contains(&"T"));
    a.config.keymap.set(Command::XtreeColors, &[commands::parse_combo("Ctrl+Shift+C").unwrap()]);
    let Control::Table(rows) = row_of(&a.settings_items(Section::Xtree), "xtree_teclas").control else { panic!() };
    assert_eq!(rows[1].0, "Ctrl+Shift+C");
}

/// Grabar Ctrl+T para los colores de xtree no es un choque con «Pestaña nueva» (otro ámbito).
#[test]
fn xtree_colors_shortcut_records_without_conflict() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "xtree_colores", PART_RESET, 0.0);
    a.config.keymap.set(Command::XtreeColors, &[]);
    a.settings_part(0, "xtree_colores", PART_ADD, 0.0);
    assert!(a.capturing_keys());
    press(&mut a, &[VK_CONTROL], Key(b'T' as u16));
    assert!(!a.capturing_keys(), "sin choque, se asigna");
    assert_eq!(a.config.keymap.combos(Command::XtreeColors), [commands::parse_combo("Ctrl+T").unwrap()]);
    assert_eq!(a.config.keymap.combos(Command::NewTab), [commands::parse_combo("Ctrl+T").unwrap()]);
}

/// M3: dentro de xtree siguen andando algunas teclas de la ventana (Ctrl+F busca) y las de la
/// vista (F5): grabarlas para un comando de xtree se avisa (choque o rechazo), no se acepta callado.
#[test]
fn xtree_shortcut_flags_keys_live_in_xtree() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "xtree_colores", PART_ADD, 0.0);
    press(&mut a, &[VK_CONTROL], Key(b'F' as u16));
    assert!(a.capturing_keys(), "Ctrl+F busca dentro de xtree: se pregunta");
    assert_eq!(a.recording().unwrap().conflict.map(|(_, c)| c), Some(Command::Filter));
    assert!(!a.config.keymap.combos(Command::XtreeColors).contains(&commands::parse_combo("Ctrl+F").unwrap()));
    a.set_recording(None);
    a.settings_part(0, "xtree_colores", PART_ADD, 0.0);
    press(&mut a, &[], Key(0x74));
    assert!(a.capturing_keys(), "F5 la usa xtree");
    assert_eq!(a.recording().unwrap().message, Some(crate::settings::keys::XTREE_KEY));
    // Con otro comando de la ventana (no de xtree), F5 es Recargar: ese choque es el de siempre.
    assert_eq!(a.config.keymap.owner_for(commands::parse_combo("F5").unwrap(), Command::Duplicate), Some(Command::Refresh));
}

#[test]
fn portable_info_only_when_unsupported() {
    let a = app();
    let r = row_of(&a.settings_items(Section::Advanced), "portable");
    assert_eq!(r.desc, "Los archivos se guardan en la carpeta del usuario");
    assert_eq!(r.control, Control::Buttons(vec![("Activar el modo portable".to_string(), true)]));
    FakeEnv::set_portable_supported(false);
    let r = row_of(&a.settings_items(Section::Advanced), "portable");
    FakeEnv::set_portable_supported(true);
    assert_eq!(r.control, Control::None, "solo se informa");
}

// ---------- Atajos de teclado ----------

use crate::keys::keys::{VK_ESCAPE, VK_TAB};
use crate::settings::draw::{PART_ADD, PART_CHIP_REMOVE, PART_EDIT, PART_REASSIGN, PART_RESET};

fn combo(text: &str) -> commands::Combo {
    commands::parse_combo(text).unwrap()
}

fn keys_app() -> (App, Rc<FakeHost>) {
    fresh_config();
    settings_app(Section::Keys)
}

fn recording(a: &App) -> Option<crate::settings::keys::Recording> {
    let (p, t) = a.settings_tab()?;
    a.panes[p].tabs[t].settings.as_ref()?.recording.clone()
}

#[test]
fn record_replaces_chip_and_writes_line() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    assert!(a.capturing_keys());
    assert!(press(&mut a, &[VK_CONTROL, VK_SHIFT], Key(b'K' as u16)));
    assert!(!a.capturing_keys());
    assert_eq!(a.config.keymap.combos(Command::Duplicate), [combo("Ctrl+Shift+K")]);
    wait_config(&mut a);
    let text = config_text();
    assert!(text.contains("\nduplicar = \"Ctrl+Shift+K\"\n"), "{text}");
    assert_eq!(a.config.keymap.lookup(Key(b'K' as u16), true, true, false, false), Some(Command::Duplicate));
}

#[test]
fn record_conflict_offers_reassign() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    press(&mut a, &[VK_CONTROL], Key(b'T' as u16));
    assert_eq!(recording(&a).unwrap().conflict, Some((combo("Ctrl+T"), Command::NewTab)));
    assert!(!a.config_saving, "nada escrito todavía");
    let row = a.settings_items(Section::Keys).into_iter().find_map(|i| match i {
        Item::Shortcut(s) if s.id == "duplicar" => Some(s),
        _ => None,
    });
    assert_eq!(row.unwrap().state, crate::settings::keys::RowState::Conflict("Ctrl+T ya es de \"Pestaña nueva\"".into()));
    a.settings_part(0, "duplicar", PART_REASSIGN, 0.0);
    assert!(!a.capturing_keys());
    wait_config(&mut a);
    let text = config_text();
    assert!(text.contains("\nduplicar = \"Ctrl+T\"\n") && text.contains("\npestana_nueva = \"\"\n"), "{text}");
    assert_eq!(a.config.keymap.owner(combo("Ctrl+T")), Some(Command::Duplicate));
    assert!(a.config.keymap.combos(Command::NewTab).is_empty());
}

#[test]
fn record_own_combo_is_noop() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_ADD, 0.0);
    press(&mut a, &[VK_CONTROL], Key(b'D' as u16));
    assert!(recording(&a).is_none());
    assert!(!a.config_saving && a.config_pending.is_empty(), "no se escribe nada");
    assert_eq!(a.config.keymap.combos(Command::Duplicate), [combo("Ctrl+D")]);
}

#[test]
fn add_second_shortcut() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_ADD, 0.0);
    press(&mut a, &[], Key(0x72)); // F3
    assert_eq!(a.config.keymap.combos(Command::Duplicate), [combo("Ctrl+D"), combo("F3")]);
    wait_config(&mut a);
    assert!(config_text().contains("\nduplicar = [\"Ctrl+D\", \"F3\"]\n"));
}

#[test]
fn remove_chip_leaves_empty_string() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_CHIP_REMOVE, 0.0);
    assert!(a.config.keymap.combos(Command::Duplicate).is_empty());
    wait_config(&mut a);
    assert!(config_text().contains("\nduplicar = \"\"\n"));
}

#[test]
fn reset_one_removes_line() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_CHIP_REMOVE, 0.0);
    wait_config(&mut a);
    a.settings_part(0, "duplicar", PART_RESET, 0.0);
    assert_eq!(a.config.keymap.combos(Command::Duplicate), [combo("Ctrl+D")]);
    wait_config(&mut a);
    assert_eq!(config_text(), config::template());
}

#[test]
fn reset_all_asks_and_clears_active_lines() {
    let (mut a, host) = keys_app();
    a.settings_part(0, "duplicar", PART_CHIP_REMOVE, 0.0);
    wait_config(&mut a);
    a.settings_part(0, "copiar", PART_CHIP_REMOVE, 0.0);
    wait_config(&mut a);
    host.set_answer(Some(false));
    a.settings_part(0, "restablecer_atajos", 0, 0.0);
    assert_eq!(host.questions().last().unwrap(), &("Atajos".to_string(), "¿Volver todos los atajos a los de fábrica? Se borran 2 atajos propios.".to_string()));
    assert!(a.config.keymap.combos(Command::Copy).is_empty(), "con No no cambia nada");
    host.set_answer(Some(true));
    a.settings_part(0, "restablecer_atajos", 0, 0.0);
    assert_eq!(a.config.keymap.combos(Command::Copy), [combo("Ctrl+C")]);
    wait_config(&mut a);
    assert_eq!(config_text(), config::template(), "los comentarios quedan");
}

#[test]
fn recording_swallows_keys() {
    let (mut a, _) = keys_app();
    a.set_layout(Layout::TwoSide);
    let tabs = a.panes[0].tabs.len();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    press(&mut a, &[VK_CONTROL], Key(b'W' as u16));
    // Ctrl+W ya es de «Cerrar pestaña»: queda el choque, la pestaña no se cierra.
    assert_eq!(a.panes[0].tabs.len(), tabs);
    a.settings_part(0, "duplicar", crate::settings::draw::PART_CANCEL, 0.0);
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    press(&mut a, &[], VK_TAB);
    assert_eq!(a.active, 0, "Tab no cambia de panel");
    assert_eq!(recording(&a).unwrap().conflict, Some((combo("Tab"), Command::NextPane)));
    press(&mut a, &[], Key(b'A' as u16));
    assert_eq!(recording(&a).unwrap().message, Some(crate::settings::keys::LIST_KEY));
    a.on_char('x');
    assert!(a.capturing_keys());
    // Un clic afuera cancela.
    a.on_left_down(2.0, 2.0, false);
    assert!(!a.capturing_keys());
}

#[test]
fn escape_cancels_recording() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    press(&mut a, &[], VK_ESCAPE);
    assert!(!a.capturing_keys());
    assert_eq!(a.config.keymap.combos(Command::Duplicate), [combo("Ctrl+D")]);
    assert!(a.panes[0].tab().is_settings(), "Esc no hizo otra cosa");
}

#[test]
fn key_errors_shown_on_top() {
    let mut init = Init::blank();
    init.config = Config::parse("[atajos]\nduplicar = \"Ctrl+Hiper\"\n");
    let a = App::new(init, FakeHost::new());
    let items = a.settings_items(Section::Keys);
    assert!(matches!(&items[0], Item::Action("restablecer_atajos", _)));
    match &items[1] {
        Item::Error(text, Some(("error_config", label))) => {
            assert!(text.starts_with("config.toml, línea 2: ") && text.contains("Hiper"), "{text}");
            assert_eq!(label, "Abrir config.toml");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn keys_page_mac_labels() {
    FakeEnv::set_key_style(sys::KeyStyle::Mac);
    let a = app();
    let items = a.settings_items(Section::Keys);
    FakeEnv::set_key_style(sys::KeyStyle::Windows);
    let chips = |id: &str| {
        items.iter().find_map(|i| match i {
            Item::Shortcut(s) if s.id == id => Some(s.chips.clone()),
            _ => None,
        })
    };
    assert_eq!(chips("ir_a_carpeta").unwrap(), ["⌘P"]);
    assert_eq!(chips("pestana_nueva").unwrap(), ["⌘T"]);
    // Los grupos en el orden de la wiki, solo con comandos.
    let headings: Vec<&str> = items.iter().filter_map(|i| if let Item::Heading(h) = i { Some(h.as_str()) } else { None }).collect();
    assert_eq!(headings[0], "Navegación");
    assert_eq!(headings.last().copied(), Some("Otros"));
}

// ---------- Buscador ----------

fn search(a: &mut App, text: &str) {
    a.search_typed(0, text);
}

fn count_of(counts: &[(PageId, usize)], s: Section) -> usize {
    counts.iter().find(|(p, _)| *p == PageId::Builtin(s)).unwrap().1
}

#[test]
fn search_across_sections_counts() {
    let a = app();
    let (counts, items) = a.settings_results("mostrar", &[]);
    assert_eq!(count_of(&counts, Section::ListViews), 2);
    assert_eq!(count_of(&counts, Section::Tree), usize::from(cfg!(feature = "arbol")));
    assert_eq!(count_of(&counts, Section::Preview), 1);
    assert_eq!(count_of(&counts, Section::Advanced), 0);
    assert!(matches!(&items[0], Item::Heading(h) if h == "Lista y vistas"));
    // Sin tildes ni mayúsculas.
    let (counts, _) = a.settings_results("ÁRBOL fijo", &[]);
    assert_eq!(count_of(&counts, Section::Tree), 1);
}

#[test]
fn search_keys_by_label() {
    let mut a = app();
    a.config.keymap.set(Command::Duplicate, &[combo("Ctrl+K")]);
    let found = |a: &App, q: &str| {
        a.settings_results(q, &[]).1.iter().any(|i| matches!(i, Item::Shortcut(s) if s.id == "duplicar"))
    };
    assert!(found(&a, "ctrl+k"));
    FakeEnv::set_key_style(sys::KeyStyle::Mac);
    let mac = found(&a, "⌘k");
    FakeEnv::set_key_style(sys::KeyStyle::Windows);
    assert!(mac);
    assert!(found(&a, "duplicar"));
    let (counts, items) = a.settings_results("atajo", &[]);
    // La sección y el comando «Configuración: Atajos de teclado».
    assert_eq!(count_of(&counts, Section::Keys), 2);
    assert!(matches!(&items[1], Item::Link(PageId::Builtin(Section::Keys), ..)));
    assert!(matches!(&items[2], Item::Shortcut(s) if s.id == "configuracion_atajos"));
}

#[test]
fn search_empty_returns_to_page() {
    let (mut a, host) = settings_app(Section::Tree);
    search(&mut a, "oscuro");
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(texts.iter().any(|t| t == "Resultados para \"oscuro\""), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Tema"));
    search(&mut a, "");
    a.paint();
    let texts: Vec<String> = host.drawn_texts().into_iter().map(|(_, t, _)| t).collect();
    assert!(!texts.iter().any(|t| t.starts_with("Resultados")));
    assert_eq!(page(&a), PageId::Builtin(Section::Tree));
    // Elegir una sección con una búsqueda la termina y va ahí.
    search(&mut a, "oscuro");
    a.paint();
    let nav = a.settings_view(0).unwrap().hits.iter().find(|(_, h)| *h == SettingsHit::Nav(PageId::Builtin(Section::Preview))).map(|(r, _)| *r).unwrap();
    a.on_left_down(nav.left + 4.0, nav.top + 4.0, false);
    assert_eq!(page(&a), PageId::Builtin(Section::Preview));
    assert_eq!(a.panes[0].tab().search_query(), "");
}

#[test]
fn search_draws_groups() {
    let (mut a, host) = settings_app(Section::Appearance);
    search(&mut a, "mostrar");
    a.paint();
    let texts = host.drawn_texts();
    let heading = |t: &str| texts.iter().position(|(f, s, _)| *f == crate::paint::FontId::SmallBold && s == t);
    let (lists, preview) = (heading("Lista y vistas").unwrap(), heading("Vista previa").unwrap());
    let row = texts.iter().position(|(_, s, _)| s == "Mostrar extensiones").unwrap();
    assert!(lists < row && row < preview);
    // La cantidad al lado de cada sección.
    assert!(texts.iter().any(|(_, s, _)| s == "2"));
}

#[test]
fn extra_page_items_searchable() {
    use super::settings_pages::ExtraPage;
    fn items(_: &App) -> Vec<crate::settings::SearchItem> {
        vec![crate::settings::SearchItem { title: "Servidores favoritos".into(), desc: "NAS y oficina".into(), keywords: vec!["smb".into()] }]
    }
    fn draw(_: &mut App, _: &dyn crate::paint::Painter, _: geom::Rect, _: Option<(f32, f32)>) -> f32 {
        0.0
    }
    fn click(_: &mut App, _: f32, _: f32) -> bool {
        false
    }
    fn field(_: &mut App, _: TextBox, _: bool) -> bool {
        false
    }
    let pages = [ExtraPage { id: "red", title: "Red", after: Section::Keys, search: items, draw, click, field_done: field, field_tab: field }];
    let a = app();
    let (counts, results) = a.settings_results("smb", &pages);
    let order: Vec<PageId> = counts.iter().map(|(p, _)| *p).collect();
    assert_eq!(order[5], PageId::Extra("red"), "Red va después de Atajos");
    assert_eq!(counts[5].1, 1);
    assert!(matches!(&results[..], [Item::Heading(h), Item::Link(PageId::Extra("red"), t, _)] if h == "Red" && t == "Servidores favoritos"));
}

/// Activar el modo portable solo deja la marca: lo que se sigue guardando en esta ejecución va a
/// la carpeta de siempre (se aplica al volver a abrir xdir). Si en la carpeta de xdir ya hay
/// archivos de otra vez, la pregunta lo dice.
#[test]
fn portable_applies_on_next_start() {
    fresh_config();
    let (mut a, host) = settings_app(Section::Advanced);
    let exe = std::env::temp_dir().join(format!("xdir-portable-app-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
    let _ = std::fs::remove_dir_all(&exe);
    std::fs::create_dir_all(&exe).unwrap();
    std::fs::write(exe.join("config.toml"), "# de otra vez\n").unwrap();
    a.exe_dir = Some(exe.clone());
    host.set_answer(Some(true));
    a.settings_part(0, "portable", 0, 0.0);
    let (_, question) = host.questions().last().cloned().unwrap();
    assert!(question.contains("Ya hay archivos de configuración"), "{question}");
    let t = a.task_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    a.on_task_done(t);
    assert!(exe.join(session::PORTABLE_MARKER).exists());
    a.set_animations(Some(true));
    wait_config(&mut a);
    assert_eq!(std::fs::read_to_string(exe.join("config.toml")).unwrap(), "# de otra vez\n", "no se escribió en la carpeta de xdir");
    assert!(config_text().contains("\nanimaciones = true\n"));
    let r = row_of(&a.settings_items(Section::Advanced), "portable");
    assert!(r.desc.contains("al volver a abrir xdir"), "{}", r.desc);
    assert_eq!(r.control, Control::Buttons(vec![("Desactivar".to_string(), true)]));
    let _ = std::fs::remove_dir_all(&exe);
}

/// La grabación termina cuando la pestaña deja de verse o se usa otra cosa: ⌘W / Ctrl+W y las
/// demás teclas vuelven a funcionar.
#[test]
fn recording_ends_when_settings_not_in_front() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    // Un comando (del menú, de la paleta) la termina.
    a.run_command(Command::NewTab);
    assert!(!a.capturing_keys());
    let tabs = a.panes[0].tabs.len();
    press(&mut a, &[VK_CONTROL], Key(b'W' as u16));
    assert_eq!(a.panes[0].tabs.len(), tabs - 1, "Ctrl+W cierra la pestaña");
    // Cambiar a otra pestaña.
    a.run_command(Command::Settings);
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    a.select_tab(0, 0);
    assert!(!a.capturing_keys());
    // La configuración en otro panel que no es el activo: las teclas no son para ella.
    a.set_layout(Layout::TwoSide);
    a.run_command(Command::Settings);
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    if let Some(v) = a.settings_view_mut(a.active) {
        v.recording = Some(crate::settings::keys::Recording::new(Command::Duplicate, None));
    }
    let other = 1 - a.active;
    a.set_active(other);
    assert!(!a.capturing_keys());
}

/// Escribir en el buscador deja de grabar (M-12).
#[test]
fn typing_in_search_cancels_recording() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    a.search_typed(0, "dup");
    assert!(!a.capturing_keys());
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    a.begin_filter(0);
    assert!(!a.capturing_keys());
}

/// M-4: el mouse que se mueve dentro de una página extra pide otro cuadro (la página dibuja su
/// hover con la posición).
#[test]
fn hover_inside_extra_page_redraws() {
    let (mut a, host) = settings_app(Section::Appearance);
    let content = a.settings_view(0).unwrap().content;
    if let Some(v) = a.settings_view_mut(0) {
        v.hits.insert(0, (content, SettingsHit::Extra));
    }
    let (x, y) = (content.left + 20.0, content.top + 20.0);
    a.on_mouse_move(x, y);
    let before = host.invalidations();
    a.on_mouse_move(x + 5.0, y + 5.0);
    assert!(host.invalidations() > before);
}

/// M-5: el campo de una página extra se suelta (y se destruye) cuando la configuración deja de
/// verse; sin él, `WM_APP_FIELD_DONE` sigue a lo de siempre.
#[test]
fn settings_field_released_when_hidden() {
    let (mut a, host) = settings_app(Section::Appearance);
    let b = host.text_create(TextKind::Field, geom::rect(0.0, 0.0, 10.0, 10.0), "x", &a.theme).unwrap();
    a.settings_field = Some(b);
    a.run_command(Command::NewTab);
    assert_eq!(a.settings_field, None);
    assert!(host.text_boxes_of(TextKind::Field).is_empty());
    assert!(!a.settings_field_done(true));
    assert!(!a.settings_field_tab(false));
}

/// M-6: las migas llevan a la sección (con una búsqueda, la terminan).
#[test]
fn crumb_click_goes_to_section() {
    let (mut a, _) = settings_app(Section::Tree);
    search(&mut a, "oscuro");
    a.paint();
    let r = a.panes[0].crumb_rect(1).expect("miga de la sección");
    a.on_left_down(r.left + 2.0, r.top + 2.0, false);
    assert_eq!(a.panes[0].tab().search_query(), "");
    assert_eq!(page(&a), PageId::Builtin(Section::Tree));
}

/// M-7: F10 (que en Windows llega como tecla de sistema, sin Alt) se graba como F10.
#[test]
fn f10_alone_is_not_alt() {
    let (mut a, _) = keys_app();
    a.settings_part(0, "duplicar", PART_EDIT, 0.0);
    FakeEnv::hold_keys(&[]);
    a.on_alt_key(Key(0x79));
    assert_eq!(a.config.keymap.combos(Command::Duplicate), [combo("F10")]);
}

/// M-8: la cantidad de atajos propios cuenta comandos, no escrituras.
#[test]
fn own_shortcuts_counted_once() {
    let (mut a, host) = keys_app();
    for k in [0x72u16, 0x73, 0x74] {
        a.settings_part(0, "duplicar", PART_EDIT, 0.0);
        press(&mut a, &[], Key(k));
    }
    assert_eq!(a.config.own_shortcuts(), 1);
    a.settings_part(0, "duplicar", PART_RESET, 0.0);
    assert_eq!(a.config.own_shortcuts(), 0);
    a.settings_part(0, "duplicar", PART_CHIP_REMOVE, 0.0);
    wait_config(&mut a);
    host.set_answer(Some(false));
    a.settings_part(0, "restablecer_atajos", 0, 0.0);
    assert!(host.questions().last().unwrap().1.ends_with("Se borran 1 atajos propios."));
}

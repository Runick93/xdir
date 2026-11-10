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

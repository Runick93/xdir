//! Pruebas de la pestaña Configuración: una sola, entre paneles, sesión, comandos y dibujo.

use super::*;
use crate::keys::keys::{VK_CONTROL, VK_SHIFT};
use crate::commands;
use crate::settings::{PageId, Section, SettingsHit};
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

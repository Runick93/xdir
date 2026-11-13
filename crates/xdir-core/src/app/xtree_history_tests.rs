//! Pruebas del historial de xtree del lado de la App (#58).

use super::*;
use crate::sys::testing::install_fakes;

fn app() -> App {
    install_fakes();
    App::new(Init::blank(), crate::sys::testing::FakeHost::new())
}

/// El período de la tarjeta sale de la sesión y vuelve a ella (lo que eligió la vista).
#[test]
fn period_comes_from_and_goes_to_session() {
    let mut a = app();
    a.xtree_days = 30;
    a.run_command(Command::Xtree);
    let x = a.xtree.as_mut().unwrap();
    assert_eq!(x.history_days(), 30);
    x.set_history(true, 90, None);
    assert_eq!(a.session().xtree_days, 90);
    a.on_closing();
}

/// La vista recibe la carpeta del historial dentro de la de configuración.
#[test]
fn view_gets_history_folder() {
    let mut a = app();
    a.run_command(Command::Xtree);
    let dir = crate::session::config_dir().unwrap().join("xtree-historial");
    assert_eq!(a.xtree.as_ref().unwrap().history_dir(), Some(dir.as_path()));
    a.on_closing();
}

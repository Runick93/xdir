//! xdir para Linux: arranque y servicios del sistema. El estado de la aplicación y todo lo
//! neutral viven en `xdir-core`. Todavía no hay ventana: el núcleo arranca con un host sin
//! interfaz. En otros sistemas la crate compila vacía (un `main` que avisa y sale), así
//! `cargo build` y `cargo test` en la raíz del workspace andan en todos.

#[cfg(target_os = "linux")]
mod env;
#[cfg(target_os = "linux")]
mod fs;
#[cfg(target_os = "linux")]
mod host;
#[cfg(target_os = "linux")]
mod info;
#[cfg(target_os = "linux")]
mod ops;
#[cfg(target_os = "linux")]
mod scan;
#[cfg(target_os = "linux")]
mod shell;
#[cfg(target_os = "linux")]
mod sys;
#[cfg(target_os = "linux")]
mod trash;

#[cfg(target_os = "linux")]
fn main() {
    use std::rc::Rc;
    use std::time::Instant;

    use xdir_core::app::{App, Init, startup_args};

    // Servicios del sistema para el núcleo: antes que nada, porque la `App` ya los usa.
    sys::install();
    // Sin interfaz no hay qué medir (`--probe`) ni panel donde abrir una carpeta: solo la sesión.
    let (session, _probe, _start_path) = startup_args();
    let init = Init::load(session, None, None, Instant::now(), Vec::new());
    let _app = App::new(init, Rc::new(host::HeadlessHost));
    println!("xdir-linux: el núcleo arrancó con los servicios de Linux; la interfaz de Linux todavía no está hecha.");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("xdir-linux solo funciona en Linux");
}

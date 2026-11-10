//! Vista rápida en Windows: una ventana propia de xdir (`xdir.quicklook`, dueña de la principal)
//! con la barra de título del núcleo y, debajo, el visor del panel de vista previa del Explorador
//! (`IPreviewHandler`, fuera de proceso) o, si el tipo no tiene visor, si el visor falla o tarda
//! más de `HANDLER_TIMEOUT`, la imagen grande con «Abrir con…» y «Abrir» (`draw_fallback`).
//!
//! - `show` / `close` los llama el host (`WindowHost::quick_look_show` / `quick_look_close`)
//!   desde la `App`; todo corre en el hilo de interfaz y nunca espera al visor (ver `handler`).
//! - Lo que el usuario hace en la ventana vuelve a la `App` por mensajes a la ventana principal:
//!   Espacio, Esc, las flechas y los botones ‹ › × como teclas (`WM_APP_QL_KEY`), y Alt+F4 como
//!   `WM_APP_QL_CLOSED`. «Abrir» y «Abrir con…» los atiende la ventana (no cambian la `App`).
//! - Lo solo en la nube no llega al visor: la ventana dibuja el ícono y «Disponible solo en
//!   línea» (`WindowHost::quick_look_shows_cloud`).
//!
//! Lo que no toca el sistema (teclas, espera con tiempo límite, estados y medidas) está en
//! `pure.rs`, probado también fuera de Windows.

mod handler;
mod pure;
mod window;

pub use pure::{WM_APP_QL_CLOSED, WM_APP_QL_KEY};
use pure::{WM_QL_DIED, WM_QL_SETTLED, encode_outcome};

pub use window::{close, set_theme, show, shutdown};

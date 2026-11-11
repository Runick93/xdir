//! Lo de la vista rápida de Windows que no toca el sistema: qué handlers cuentan como visor, qué
//! teclas devuelve el visor a xdir, la espera con tiempo límite del hilo del visor, los estados de
//! la ventana y sus medidas. Fuera de Windows `main.rs` incluye este archivo suelto para que sus
//! pruebas corran en todos los sistemas.
#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::mpsc;
use std::time::Duration;

use xdir_core::WM_APP;
use xdir_core::keys::Key;
use xdir_core::keys::keys::{VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RIGHT, VK_SPACE, VK_UP};
use xdir_core::quicklook::{MIN_H, MIN_W, TITLE_H, TitleHit, WINDOW_H, WINDOW_W, title_hit};

// Avisos a la ventana principal (los atiende `app/messages.rs`).

/// Una tecla de la vista rápida (`wparam` = tecla virtual): Espacio, Esc o una flecha que llegó
/// a su ventana o al visor. Va a `App::on_key`.
pub const WM_APP_QL_KEY: u32 = WM_APP + 100;
/// El usuario cerró la ventana de la vista rápida por su cuenta (Alt+F4): `App::on_quick_look_closed`.
pub const WM_APP_QL_CLOSED: u32 = WM_APP + 101;

// Avisos a la ventana de la vista rápida.

/// Terminó la espera del visor: `wparam` = generación, `lparam` = `encode_outcome`.
pub const WM_QL_SETTLED: u32 = WM_APP + 102;
/// El visor que se veía murió (`prevhost.exe` cerrado): `wparam` = generación.
pub const WM_QL_DIED: u32 = WM_APP + 103;

/// Lo que se espera al visor (crearlo en `prevhost.exe`, inicializarlo y `DoPreview`) antes de
/// mostrar la imagen grande en su lugar.
pub const HANDLER_TIMEOUT: Duration = Duration::from_secs(3);

/// IID de `IPreviewHandler`: la subclave `shellex\{…}` de un tipo dice el CLSID de su visor.
pub const PREVIEW_HANDLER_IID: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";

/// El valor `DisableProcessIsolation` del CLSID del visor: si pide correr dentro del proceso que
/// lo aloja, se trata como «sin visor» (xdir nunca carga un visor dentro de su proceso).
pub fn isolation_disabled(value: Option<u32>) -> bool {
    matches!(value, Some(v) if v != 0)
}

/// Las teclas de la tabla de aceleradores que se le da al visor (`GetWindowContext`): muchos
/// visores (Office, el ejemplo del SDK) solo llaman a `TranslateAccelerator` del marco si la tecla
/// está en esa tabla. Son las de `forwarded`.
pub const ACCEL_KEYS: [u16; 6] = [VK_SPACE.0, VK_ESCAPE.0, VK_LEFT.0, VK_UP.0, VK_RIGHT.0, VK_DOWN.0];

/// Las teclas que el visor le devuelve a xdir (`IPreviewHandlerFrame::TranslateAccelerator`):
/// Espacio y Esc cierran, las flechas recorren. Las demás quedan para el visor.
pub fn forwarded(vk: u16) -> bool {
    [VK_SPACE, VK_ESCAPE, VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN].contains(&Key(vk))
}

/// `WM_KEYDOWN`.
pub const KEYDOWN: u32 = 0x0100;

/// Si un mensaje que el visor ofrece como acelerador es de xdir: `WM_KEYDOWN` de una de las
/// teclas de `forwarded`, sin Ctrl, Alt ni Mayúsculas (con modificadores, sigue en el visor).
pub fn forward_key(message: u32, vk: u16, modifiers: bool) -> bool {
    message == KEYDOWN && !modifiers && forwarded(vk)
}

// ---------- espera con tiempo límite ----------

/// Cómo terminó la espera de `run_with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    /// El hilo contestó a tiempo.
    Done(T),
    /// No contestó a tiempo: se lo abandona (sigue solo y lo que conteste se descarta).
    Timeout,
    /// Terminó sin contestar (falló al arrancar o entró en pánico).
    Gone,
}

/// La respuesta de un hilo de `run_with`: se manda una vez y el hilo puede seguir (el del visor
/// sigue con su bucle de mensajes).
pub struct Reply<T>(mpsc::Sender<T>);

impl<T> Reply<T> {
    pub fn send(self, v: T) {
        let _ = self.0.send(v);
    }
}

/// Corre `f` en un hilo nuevo y espera su respuesta hasta `timeout`. Si no llega, devuelve
/// `Timeout` sin esperar más: el hilo queda abandonado (nunca se lo espera ni se lo mata).
/// Bloquea al que llama: en xdir lo llama un hilo vigía, nunca el de interfaz.
pub fn run_with<T: Send + 'static>(timeout: Duration, f: impl FnOnce(Reply<T>) + Send + 'static) -> Outcome<T> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("xdir-visor".into()).spawn(move || f(Reply(tx)));
    if spawned.is_err() {
        return Outcome::Gone;
    }
    match rx.recv_timeout(timeout) {
        Ok(v) => Outcome::Done(v),
        Err(mpsc::RecvTimeoutError::Timeout) => Outcome::Timeout,
        Err(mpsc::RecvTimeoutError::Disconnected) => Outcome::Gone,
    }
}

/// `Outcome<Status>` como `lparam` de `WM_QL_SETTLED`.
pub fn encode_outcome(o: Outcome<Status>) -> isize {
    match o {
        Outcome::Done(Status::Ready) => 0,
        Outcome::Done(Status::Failed) => 1,
        Outcome::Done(Status::Cloud) => 2,
        Outcome::Timeout => 3,
        Outcome::Gone => 4,
    }
}

/// La inversa de `encode_outcome` (lo desconocido cuenta como `Gone`).
pub fn decode_outcome(v: isize) -> Outcome<Status> {
    match v {
        0 => Outcome::Done(Status::Ready),
        1 => Outcome::Done(Status::Failed),
        2 => Outcome::Done(Status::Cloud),
        3 => Outcome::Timeout,
        _ => Outcome::Gone,
    }
}

/// Al cerrar xdir: si algún hilo de visor retirado (parado o abandonado) sigue vivo, puede estar
/// colgado en `prevhost.exe` (en `Unload`, o en un visor colgado que todavía no se detectó), así
/// que se termina el proceso sin destruir ventanas. Destruir la ventana de la vista rápida le
/// manda `WM_DESTROY` de forma síncrona a la ventana colgada, que es hija suya, y xdir no
/// terminaría nunca.
pub fn hard_exit(retired_alive: &[bool]) -> bool {
    retired_alive.iter().any(|a| *a)
}

/// Cuánto se espera al cerrar a que terminen los hilos retirados: `EXIT_WAIT_STEPS` × `EXIT_WAIT_STEP`.
pub const EXIT_WAIT_STEPS: u32 = 30;
pub const EXIT_WAIT_STEP: Duration = Duration::from_millis(10);

/// Espera hasta `steps` × `step` a que ningún hilo retirado siga vivo (`alive` los mira cada vez).
/// `true` si alguno sigue vivo al final: hay que salir sin destruir ventanas.
pub fn exit_wait(alive: impl Fn() -> Vec<bool>, steps: u32, step: Duration) -> bool {
    for _ in 0..steps {
        if !hard_exit(&alive()) {
            return false;
        }
        std::thread::sleep(step);
    }
    hard_exit(&alive())
}

/// Si el visor que se ve sigue vivo, mirado cada `ALIVE_MS` desde su hilo: murió si su ventana
/// desapareció (se cerró `prevhost.exe`) o si no contesta `HUNG_MISSES` veces seguidas (colgado).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Liveness {
    had_child: bool,
    misses: u8,
}

/// Veces seguidas que el visor no contesta antes de darlo por colgado.
/// Ocho vistazos de 500 ms (unos 4 s), en línea con `HANDLER_TIMEOUT` y con los 5 s de
/// `IsHungAppWindow`: un visor ocupado un rato (un PDF grande, una planilla) no se descarta.
pub const HUNG_MISSES: u8 = 8;

impl Liveness {
    pub const fn new() -> Liveness {
        Liveness { had_child: false, misses: 0 }
    }

    /// Un vistazo: `child`, la ventana del visor existe; `responsive`, contestó a tiempo.
    /// Devuelve `true` si el visor murió o se colgó.
    pub fn tick(&mut self, child: bool, responsive: bool) -> bool {
        if !child {
            // Antes de que aparezca su ventana no se juzga (algunos visores la crean tarde).
            return self.had_child;
        }
        self.had_child = true;
        if responsive {
            self.misses = 0;
            return false;
        }
        self.misses = self.misses.saturating_add(1);
        self.misses >= HUNG_MISSES
    }
}

// ---------- estados de la ventana ----------

/// Lo que dice el hilo del visor después de `DoPreview`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// El visor está mostrando el archivo.
    Ready,
    /// No se pudo crear, inicializar o mostrar.
    Failed,
    /// El archivo resultó estar solo en la nube: no se le pasó al visor.
    Cloud,
}

/// Qué muestra el cuerpo de la ventana (debajo de la barra de título).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Esperando al visor: el ícono del elemento.
    Loading,
    /// El visor del sistema.
    Viewer,
    /// La imagen grande (`quicklook::draw_fallback`); `viewer_missing`: con el aviso y los
    /// botones «Abrir con…» y «Abrir».
    Fallback { viewer_missing: bool },
}

/// El elemento que muestra la ventana, numerado: lo que llega de un visor viejo se descarta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub generation: u64,
    pub mode: Mode,
}

impl Default for Slot {
    fn default() -> Self {
        Slot { generation: 0, mode: Mode::Fallback { viewer_missing: true } }
    }
}

impl Slot {
    /// Un elemento nuevo: con visor, a esperarlo; sin visor (o solo en la nube), la imagen
    /// grande enseguida. Devuelve su número.
    pub fn start(&mut self, viewer: bool, cloud: bool) -> u64 {
        self.generation += 1;
        self.mode = if viewer && !cloud { Mode::Loading } else { Mode::Fallback { viewer_missing: !cloud } };
        self.generation
    }

    /// Terminó la espera del visor `generation`. `None` si es de un elemento anterior o ya se
    /// resolvió; si no, el modo nuevo (solo `Ready` a tiempo muestra el visor).
    pub fn settle(&mut self, generation: u64, outcome: Outcome<Status>) -> Option<Mode> {
        if generation != self.generation || self.mode != Mode::Loading {
            return None;
        }
        self.mode = match outcome {
            Outcome::Done(Status::Ready) => Mode::Viewer,
            Outcome::Done(Status::Cloud) => Mode::Fallback { viewer_missing: false },
            Outcome::Done(Status::Failed) | Outcome::Timeout | Outcome::Gone => Mode::Fallback { viewer_missing: true },
        };
        Some(self.mode)
    }

    /// El visor que se estaba viendo murió (se cerró `prevhost.exe`): la imagen grande.
    pub fn died(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.mode != Mode::Viewer {
            return false;
        }
        self.mode = Mode::Fallback { viewer_missing: true };
        true
    }
}

// ---------- medidas ----------

/// Borde para redimensionar la ventana (DIPs; la ventana no tiene marco del sistema).
pub const BORDER: f32 = 6.0;

/// Códigos de `WM_NCHITTEST`.
pub const HTCLIENT: u32 = 1;
pub const HTCAPTION: u32 = 2;
pub const HTLEFT: u32 = 10;
pub const HTRIGHT: u32 = 11;
pub const HTTOP: u32 = 12;
pub const HTTOPLEFT: u32 = 13;
pub const HTTOPRIGHT: u32 = 14;
pub const HTBOTTOM: u32 = 15;
pub const HTBOTTOMLEFT: u32 = 16;
pub const HTBOTTOMRIGHT: u32 = 17;

/// `WM_NCHITTEST` de la ventana de `w`×`h` DIPs en `(x, y)`: los bordes redimensionan, el resto
/// de la barra de título (fuera de sus botones) la mueve y lo demás es del área de la ventana.
pub fn nc_hit(w: f32, h: f32, x: f32, y: f32) -> u32 {
    let (left, right, top, bottom) = (x < BORDER, x >= w - BORDER, y < BORDER, y >= h - BORDER);
    match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, ..) => HTLEFT,
        (_, true, ..) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (.., true) => HTBOTTOM,
        _ if title_hit(bar(w), x, y) == Some(TitleHit::Drag) => HTCAPTION,
        _ => HTCLIENT,
    }
}

/// La barra de título de una ventana de `w` DIPs de ancho.
pub fn bar(w: f32) -> xdir_core::geom::Rect {
    xdir_core::geom::rect(0.0, 0.0, w, TITLE_H)
}

/// El cuerpo (debajo de la barra), donde va el visor o la imagen grande.
pub fn body(w: f32, h: f32) -> xdir_core::geom::Rect {
    xdir_core::geom::rect(0.0, TITLE_H, w, h.max(TITLE_H))
}

/// Dónde va la ventana del visor: el cuerpo menos el borde de redimensionar a los costados y
/// abajo (el visor está en otro proceso: sobre él, el mouse ya no llega a la ventana).
pub fn viewer_rect(w: f32, h: f32) -> xdir_core::geom::Rect {
    xdir_core::geom::rect(BORDER, TITLE_H, (w - BORDER).max(BORDER), (h - BORDER).max(TITLE_H))
}

/// Alto de la superficie que dibuja xdir: con el visor, solo la barra (el visor es otra ventana y
/// nunca queda debajo del dibujo de Direct2D); si no, toda la ventana.
pub fn canvas_height(h: f32, mode: Mode) -> f32 {
    if mode == Mode::Viewer { TITLE_H.min(h) } else { h }
}

/// Tamaño mínimo de la ventana en píxeles.
pub fn min_size(dpi: u32) -> (i32, i32) {
    (px(MIN_W, dpi), px(MIN_H, dpi))
}

fn px(dips: f32, dpi: u32) -> i32 {
    (dips * dpi as f32 / 96.0).round() as i32
}

/// Dónde abre la ventana (píxeles de pantalla, `[left, top, right, bottom]`): 960×680 centrada
/// sobre la ventana de xdir `owner`, achicada para entrar en el área de trabajo `work` de su
/// monitor (nunca menos de 480×360, salvo que el monitor sea más chico) y corrida para no salirse.
pub fn initial_rect(owner: [i32; 4], work: [i32; 4], dpi: u32) -> [i32; 4] {
    let (ww, wh) = (work[2] - work[0], work[3] - work[1]);
    let (min_w, min_h) = min_size(dpi);
    let w = px(WINDOW_W, dpi).min(ww).max(min_w.min(ww));
    let h = px(WINDOW_H, dpi).min(wh).max(min_h.min(wh));
    let (cx, cy) = ((owner[0] + owner[2]) / 2, (owner[1] + owner[3]) / 2);
    let left = (cx - w / 2).clamp(work[0], work[2] - w);
    let top = (cy - h / 2).clamp(work[1], work[3] - h);
    [left, top, left + w, top + h]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn in_process_handler_is_no_viewer() {
        assert!(isolation_disabled(Some(1)));
        assert!(!isolation_disabled(None));
        assert!(!isolation_disabled(Some(0)));
    }

    #[test]
    fn frame_forwards_keys() {
        for vk in [0x20, 0x1B, 0x25, 0x26, 0x27, 0x28] {
            assert!(forwarded(vk), "{vk:#x}");
            assert!(forward_key(KEYDOWN, vk, false), "{vk:#x}");
            assert!(!forward_key(KEYDOWN, vk, true), "con modificadores sigue en el visor: {vk:#x}");
            assert!(!forward_key(0x0101, vk, false), "WM_KEYUP no: {vk:#x}");
        }
        // Tab y letras quedan para el visor (recorrer un formulario, buscar en un PDF).
        for vk in [0x09, b'A' as u16, b'Z' as u16, 0x0D, 0x21, 0x22] {
            assert!(!forwarded(vk), "{vk:#x}");
            assert!(!forward_key(KEYDOWN, vk, false), "{vk:#x}");
        }
    }

    #[test]
    fn accelerator_table_is_forwarded_keys() {
        assert!(ACCEL_KEYS.iter().all(|k| forwarded(*k)));
        let all: Vec<u16> = (0..=0xFF).filter(|k| forwarded(*k)).collect();
        assert_eq!(all.len(), ACCEL_KEYS.len(), "la tabla tiene todas las teclas de xdir");
    }

    #[test]
    fn hard_exit_only_with_live_abandoned_viewer() {
        assert!(!hard_exit(&[]));
        assert!(!hard_exit(&[false, false]));
        assert!(hard_exit(&[false, true]));
    }

    /// Un visor parado (no abandonado) cuyo hilo termina mientras se espera: salida normal. Uno
    /// que sigue trabado (en `Unload` de un `prevhost.exe` colgado): salida sin destruir ventanas.
    #[test]
    fn exit_waits_for_stopped_viewer() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let alive = Arc::new(AtomicBool::new(true));
        let thread = alive.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            thread.store(false, Ordering::Release);
        });
        let t0 = Instant::now();
        assert!(!exit_wait(|| vec![alive.load(Ordering::Acquire)], EXIT_WAIT_STEPS, EXIT_WAIT_STEP));
        assert!(t0.elapsed() < Duration::from_millis(300));
        // Trabado: se espera lo justo y se sale igual.
        let t0 = Instant::now();
        assert!(exit_wait(|| vec![false, true], 5, Duration::from_millis(10)));
        assert!(t0.elapsed() < Duration::from_millis(500));
        assert!(!exit_wait(Vec::new, 5, Duration::from_millis(10)));
    }

    #[test]
    fn viewer_liveness() {
        let mut l = Liveness::default();
        // Sin ventana todavía: no se juzga.
        assert!(!l.tick(false, false));
        assert!(!l.tick(true, true));
        // Lento un rato no alcanza; contestar vuelve a empezar la cuenta.
        for _ in 0..HUNG_MISSES - 1 {
            assert!(!l.tick(true, false));
        }
        assert!(!l.tick(true, true));
        for _ in 0..HUNG_MISSES - 1 {
            assert!(!l.tick(true, false));
        }
        // `HUNG_MISSES` seguidas (unos 4 s): colgado.
        assert!(l.tick(true, false));
        assert_eq!(HUNG_MISSES, 8);
        // La ventana desapareció (prevhost.exe cerrado).
        let mut l = Liveness::default();
        assert!(!l.tick(true, true));
        assert!(l.tick(false, false));
    }

    #[test]
    fn handler_timeout_falls_back() {
        let t0 = Instant::now();
        let o = run_with(Duration::from_millis(100), |reply: Reply<Status>| {
            std::thread::sleep(Duration::from_secs(5));
            reply.send(Status::Ready);
        });
        assert_eq!(o, Outcome::Timeout);
        assert!(t0.elapsed() < Duration::from_secs(2), "no espera al visor colgado: {:?}", t0.elapsed());
        // La ventana pasa a la imagen grande con «No hay un visor…».
        let mut slot = Slot::default();
        let g = slot.start(true, false);
        assert_eq!(slot.mode, Mode::Loading);
        assert_eq!(slot.settle(g, o), Some(Mode::Fallback { viewer_missing: true }));
        // Lo que contesta tarde ya no cambia nada.
        assert_eq!(slot.settle(g, Outcome::Done(Status::Ready)), None);
        assert_eq!(slot.mode, Mode::Fallback { viewer_missing: true });
    }

    #[test]
    fn handler_reply_in_time_shows_viewer() {
        let o = run_with(Duration::from_secs(5), |reply| {
            reply.send(Status::Ready);
            // El hilo del visor sigue con su bucle después de contestar.
            std::thread::sleep(Duration::from_millis(50));
        });
        assert_eq!(o, Outcome::Done(Status::Ready));
        assert_eq!(run_with(Duration::from_secs(5), |reply: Reply<u8>| drop(reply)), Outcome::Gone);
        let mut slot = Slot::default();
        let g = slot.start(true, false);
        assert_eq!(slot.settle(g, o), Some(Mode::Viewer));
        // Se cerró prevhost.exe: la imagen grande.
        assert!(slot.died(g));
        assert_eq!(slot.mode, Mode::Fallback { viewer_missing: true });
        assert!(!slot.died(g));
    }

    #[test]
    fn settled_message_round_trip() {
        for o in [Outcome::Done(Status::Ready), Outcome::Done(Status::Failed), Outcome::Done(Status::Cloud), Outcome::Timeout, Outcome::Gone] {
            assert_eq!(decode_outcome(encode_outcome(o)), o);
        }
        assert_eq!(decode_outcome(99), Outcome::Gone);
    }

    #[test]
    fn stale_viewer_ignored() {
        let mut slot = Slot::default();
        let old = slot.start(true, false);
        let new = slot.start(true, false);
        assert_eq!(slot.settle(old, Outcome::Done(Status::Ready)), None);
        assert!(!slot.died(old));
        assert_eq!(slot.mode, Mode::Loading);
        assert_eq!(slot.settle(new, Outcome::Done(Status::Failed)), Some(Mode::Fallback { viewer_missing: true }));
        // Sin visor o solo en la nube no se espera nada.
        slot.start(false, false);
        assert_eq!(slot.mode, Mode::Fallback { viewer_missing: true });
        slot.start(true, true);
        assert_eq!(slot.mode, Mode::Fallback { viewer_missing: false });
        let g = slot.start(true, false);
        assert_eq!(slot.settle(g, Outcome::Done(Status::Cloud)), Some(Mode::Fallback { viewer_missing: false }));
    }

    #[test]
    fn quick_look_window_layout() {
        let (w, h) = (960.0, 680.0);
        assert_eq!(nc_hit(w, h, 2.0, 2.0), HTTOPLEFT);
        assert_eq!(nc_hit(w, h, w - 2.0, h - 2.0), HTBOTTOMRIGHT);
        assert_eq!(nc_hit(w, h, 2.0, 300.0), HTLEFT);
        assert_eq!(nc_hit(w, h, 500.0, h - 1.0), HTBOTTOM);
        assert_eq!(nc_hit(w, h, 500.0, 1.0), HTTOP);
        // La barra se arrastra salvo en sus botones (‹ en 744, «Abrir» en 844, la cruz en 933).
        assert_eq!(nc_hit(w, h, 300.0, 20.0), HTCAPTION);
        assert_eq!(nc_hit(w, h, 746.0, 20.0), HTCLIENT);
        assert_eq!(nc_hit(w, h, 880.0, 20.0), HTCLIENT);
        assert_eq!(nc_hit(w, h, 938.0, 20.0), HTCLIENT);
        assert_eq!(nc_hit(w, h, 300.0, 300.0), HTCLIENT);
        // Con el visor, Direct2D solo dibuja la barra.
        assert_eq!(canvas_height(h, Mode::Viewer), 40.0);
        assert_eq!(canvas_height(h, Mode::Loading), h);
        assert_eq!(canvas_height(h, Mode::Fallback { viewer_missing: true }), h);
        let b = body(w, h);
        assert_eq!((b.left, b.top, b.right, b.bottom), (0.0, 40.0, w, h));
        let v = viewer_rect(w, h);
        assert_eq!((v.left, v.top, v.right, v.bottom), (6.0, 40.0, w - 6.0, h - 6.0));
        // El borde de abajo y los costados siguen siendo de la ventana con el visor puesto.
        assert_eq!(nc_hit(w, h, v.left - 1.0, 300.0), HTLEFT);
        assert_eq!(nc_hit(w, h, 300.0, v.bottom + 1.0), HTBOTTOM);
        // 960×680 centrada sobre xdir (a 96 y a 144 DPI).
        assert_eq!(initial_rect([100, 50, 1500, 870], [0, 0, 1920, 1040], 96), [320, 120, 1280, 800]);
        assert_eq!(initial_rect([0, 0, 2400, 1400], [0, 0, 2880, 1560], 144), [480, 190, 1920, 1210]);
        // xdir pegada al borde: la ventana no se sale del área de trabajo.
        assert_eq!(initial_rect([1500, 0, 1920, 400], [0, 0, 1920, 1040], 96), [960, 0, 1920, 680]);
        // Un monitor chico: achicada, con el mínimo de 480×360.
        assert_eq!(initial_rect([0, 0, 800, 600], [0, 0, 800, 560], 96), [0, 0, 800, 560]);
        assert_eq!(min_size(96), (480, 360));
        assert_eq!(min_size(144), (720, 540));
    }
}

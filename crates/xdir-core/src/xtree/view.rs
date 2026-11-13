//! Vista de xtree dentro de xdir: ocupa el área de los paneles con la barra superior (discos y
//! «Analizar»), la tabla de lo más pesado de la carpeta actual, el mapa, la tarjeta de detalle,
//! la leyenda y la barra de datos del disco.
//!
//! Recibe las coordenadas en DIPs de la ventana y dibuja con el `Painter` y el `Theme` de xdir; el
//! mapa conserva su rueda de colores y sus categorías. Clic selecciona; doble clic entra en la
//! carpeta; clic derecho o Retroceso suben; T cambia el color; F5 vuelve a analizar.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::colors::{CATS, file_color, fmt_size, lighten, position_color, text_on};
use super::scan::{self, DriveEntry, Progress, Scan, WM_APP_XTREE};
use super::tree::{NO_PARENT, Node, node_path, rows_of};
use super::treemap::{Cell, header_h, layout};
use crate::keys::keys::{VK_BACK, VK_ESCAPE};
use crate::geom::{Color, Rect, contains, rect};
use crate::keys::Key;
use crate::sys::Handle;
use crate::paint::{FontId, Painter};
use crate::theme::alpha;
use crate::sys::disk_scan;

// Medidas (DIPs).
const MARGIN: f32 = 8.0;
const TOP_H: f32 = 40.0;
const STATUS_H: f32 = 58.0;
const LIST_W: f32 = 372.0;
const TABLE_H: f32 = 34.0;
const ROW_H: f32 = 32.0;
const CARD_H: f32 = 76.0;
const LEGEND_H: f32 = 40.0;
const GAP: f32 = 6.0;
const MENU_ROW: f32 = 34.0;
const MENU_W: f32 = 400.0;
/// Ancho del estado del análisis, a la derecha de la barra superior («Análisis completado en
/// 1:02:15»).
const STATE_W: f32 = 220.0;
/// Botón «← xdir», a la izquierda de todo en la barra superior.
const BACK_W: f32 = 76.0;

const VK_A: Key = Key(0x41);
const VK_T: Key = Key(0x54);
const VK_F5: Key = Key(0x74);

/// Colores fijos (no dependen del tema): carpeta dorada y tilde de «Análisis completado».
const FOLDER_ICON: (f32, f32, f32) = (0.98, 0.78, 0.30);
const GREEN: (f32, f32, f32) = (0.20, 0.72, 0.38);
const RED: (f32, f32, f32) = (0.92, 0.32, 0.34);

/// Generación de escaneo: los avisos de un escaneo que ya se reemplazó se ignoran.
static NEXT_SCAN: AtomicUsize = AtomicUsize::new(1);

fn color(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

fn rgb((r, g, b): (f32, f32, f32)) -> Color {
    color(r, g, b)
}

fn hex(v: u32) -> Color {
    color((v >> 16) as f32 / 255.0, ((v >> 8) & 0xFF) as f32 / 255.0, (v & 0xFF) as f32 / 255.0)
}

/// Lo que la vista le pide a xdir después de un evento.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Redraw,
    /// Cerrar la vista (el escaneo sigue en segundo plano).
    Close,
    /// Abrir la carpeta en el panel activo y, si es un archivo, enfocarlo.
    OpenInPanel { folder: PathBuf, select: Option<OsString> },
}

/// Estado del escaneo para el indicador junto al botón «xtree» de la barra de estado.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XtreeBadge {
    /// Sin escaneo, cancelado, o el resultado ya se vio en la vista: el botón de siempre.
    #[default]
    Idle,
    /// Escaneando un volumen entero: porcentaje de avance.
    Progress(u8),
    /// Escaneando una carpeta (sin porcentaje) o reescaneando con permisos: fase del arco, que
    /// avanza con cada aviso del escaneo.
    Spinning(u8),
    /// Terminado y todavía sin ver.
    Done,
    /// Terminado con carpetas sin permiso, todavía sin ver.
    DoneWithIssues,
}

/// Rectángulos de cada zona de la vista.
#[derive(Clone, Copy, Debug)]
pub struct Ui {
    pub list: Rect,
    pub map: Rect,
    pub card: Rect,
    pub legend: Rect,
    pub status: Rect,
    /// «Abrir en el panel», «Copiar ruta», «Explorar aquí».
    pub buttons: [Rect; 3],
    /// «← xdir» (vuelve a los paneles), desplegable de discos y botón «Analizar», en la barra
    /// superior.
    pub back: Rect,
    pub combo: Rect,
    pub scan_btn: Rect,
}

/// Distribución de la vista en `area`. `legend_h` es 0 cuando no hay leyenda (color por carpeta):
/// el mapa gana ese espacio.
pub fn layout_ui(area: Rect, legend_h: f32) -> Ui {
    let (l, t, r, b) = (area.left, area.top, area.right, area.bottom);
    let body_bottom = b - STATUS_H - 4.0;
    let x0 = l + MARGIN + LIST_W + MARGIN;
    let list = rect(l + MARGIN, t + TOP_H + 4.0, l + MARGIN + LIST_W, body_bottom);
    let legend = rect(x0, body_bottom - legend_h, r - MARGIN, body_bottom);
    let card = rect(x0, legend.top - GAP - CARD_H, r - MARGIN, legend.top - GAP);
    let map = rect(x0, t + TOP_H + 4.0, r - MARGIN, card.top - GAP);
    let widths = [168.0, 134.0, 150.0];
    let (top, bottom) = ((card.top + card.bottom) / 2.0 - 20.0, (card.top + card.bottom) / 2.0 + 20.0);
    let mut x = card.right - 14.0;
    let mut buttons = [rect(0.0, 0.0, 0.0, 0.0); 3];
    for i in (0..3).rev() {
        buttons[i] = rect(x - widths[i], top, x, bottom);
        x -= widths[i] + 8.0;
    }
    let back = back_rect(area);
    let combo = rect(back.right + 8.0, t + 6.0, back.right + 8.0 + 250.0, t + TOP_H - 6.0);
    let scan_btn = rect(combo.right + 8.0, t + 6.0, combo.right + 108.0, t + TOP_H - 6.0);
    Ui { list, map, card, legend, status: rect(l, b - STATUS_H, r, b), buttons, back, combo, scan_btn }
}

/// Botón «← xdir» de la vista en `area`: a la izquierda de todo en la barra superior.
pub fn back_rect(area: Rect) -> Rect {
    rect(area.left + MARGIN, area.top + 6.0, area.left + MARGIN + BACK_W, area.top + TOP_H - 6.0)
}

/// Lista desplegable de discos (más la fila «Seleccionar carpeta…»), justo debajo del desplegable.
fn menu_rect(ui: &Ui, rows: usize) -> Rect {
    rect(ui.combo.left, ui.combo.bottom + 2.0, ui.combo.left + MENU_W, ui.combo.bottom + 2.0 + rows as f32 * MENU_ROW + 8.0)
}

/// Número con separador de miles: 1284311 → «1.284.311».
fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Duración del escaneo: «0:42», «1:23»; desde una hora, «1:02:15».
pub fn fmt_elapsed(d: Duration) -> String {
    let t = d.as_secs();
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Avance del escaneo de un volumen entero: bytes leídos sobre el espacio usado, entero y con tope
/// en 99 % (APFS con clones, hardlinks o archivos que cambian pueden pasarse). Sin espacio usado,
/// no hay porcentaje.
fn scan_percent(read: u64, used: u64) -> Option<u8> {
    if used == 0 {
        return None;
    }
    Some((read as u128 * 100 / used as u128).min(99) as u8)
}

fn pct(v: u64, of: u64) -> String {
    format!("{:.1} %", v as f64 / of.max(1) as f64 * 100.0).replace('.', ",")
}

/// Escaneo vacío (ningún hilo): el de una vista que todavía no analizó nada.
fn idle_scan() -> Scan {
    Scan {
        id: 0,
        live: Arc::new(std::sync::RwLock::new(Vec::new())),
        progress: Arc::new(Progress::default()),
        cancel: Arc::new(AtomicBool::new(false)),
        result: Arc::new(std::sync::Mutex::new(None)),
    }
}

pub struct Xtree {
    hwnd: Handle,
    /// Carpeta analizada.
    root: PathBuf,
    scan: Scan,
    /// El escaneo sigue en curso: se dibuja el árbol en vivo (`scan.live`).
    scanning: bool,
    /// Árbol terminado (vacío mientras se escanea).
    arena: Vec<Node>,
    current: u32,
    selected: Option<u32>,
    cells: Vec<Cell>,
    /// Para qué carpeta y qué área se calcularon `cells`.
    cells_for: Option<(u32, [i32; 4])>,
    hover: Option<usize>,
    hover_row: Option<usize>,
    hover_btn: Option<usize>,
    /// El mouse está sobre «← xdir».
    hover_back: bool,
    scroll: f32,
    /// Colorear por tipo de archivo en vez de por posición (tecla T).
    by_type: bool,
    /// Espacio del volumen que contiene la carpeta: (total, libre).
    vol: Option<(u64, u64)>,
    /// El espacio de un volumen de red, pedido en otro hilo (llega acá).
    vol_remote: std::sync::Arc<std::sync::Mutex<Option<(u64, u64)>>>,
    /// La raíz es un volumen entero: se muestra el porcentaje de avance.
    whole: bool,
    /// Inicio del escaneo en curso (o del último).
    scan_started: Option<Instant>,
    /// Duración fija del escaneo: al terminar o al cortarse.
    scan_elapsed: Option<Duration>,
    /// El escaneo se cortó antes de terminar (`cancel`).
    scan_stopped: bool,
    /// Reescaneo con permisos (tecla A) después de un análisis terminado: cronómetro del análisis
    /// anterior (inicio, duración, ya visto), para volver a él si el escaneo elevado no trae árbol.
    before_elevate: Option<(Option<Instant>, Option<Duration>, bool)>,
    /// Mayor porcentaje mostrado en este escaneo: no retrocede.
    pct_max: std::cell::Cell<u8>,
    /// Disco o carpeta elegida en el desplegable; se analiza al pulsar «Analizar».
    pick: PathBuf,
    drives: Vec<DriveEntry>,
    menu_open: bool,
    menu_hover: Option<usize>,
    /// Hay un escaneo con permisos elevados en curso (tecla A).
    elevating: Arc<AtomicBool>,
    /// El resultado del último escaneo ya se vio en la vista: el indicador vuelve a `Idle`.
    seen: bool,
    /// Avisos del escaneo recibidos: fase del arco del indicador que gira.
    ticks: u8,
}

impl Xtree {
    /// Abre la vista sobre `root` y empieza a analizarla; `by_type`: el mapa arranca coloreado por
    /// tipo de archivo (`[opciones] xtree_por_tipo`; la tecla T lo sigue cambiando).
    pub fn new(hwnd: Handle, root: PathBuf, by_type: bool) -> Xtree {
        let id = NEXT_SCAN.fetch_add(1, Ordering::Relaxed);
        let scan = scan::start(hwnd, id, root.clone());
        let mut x = Xtree::with_scan(hwnd, root, scan);
        x.by_type = by_type;
        x.scanning = true;
        x.start_clock();
        x
    }

    /// Abre la vista sobre `root` sin analizar (`[opciones] xtree_analizar_al_abrir` apagado): el
    /// desplegable muestra la raíz y el análisis empieza con «Analizar». Solo se piden los datos
    /// del disco (los de red, en otro hilo).
    pub fn unscanned(hwnd: Handle, root: PathBuf, by_type: bool) -> Xtree {
        let mut x = Xtree::with_scan(hwnd, root, idle_scan());
        x.by_type = by_type;
        x.read_volume();
        x
    }

    /// Lo elegido en el desplegable.
    pub fn pick(&self) -> &Path {
        &self.pick
    }

    /// Lo elegido a mano en el desplegable, si no es la raíz con la que se abrió la vista.
    pub fn user_pick(&self) -> Option<PathBuf> {
        (self.pick != self.root && !self.pick.as_os_str().is_empty()).then(|| self.pick.clone())
    }

    /// Elige en el desplegable sin pasar por el menú (pruebas de `App`).
    #[cfg(test)]
    pub fn set_pick(&mut self, p: PathBuf) {
        self.pick = p;
    }

    /// ¿Empezó algún análisis en esta vista? (una abierta sin analizar, todavía no).
    pub fn scanned(&self) -> bool {
        self.scan_started.is_some()
    }

    fn with_scan(hwnd: Handle, root: PathBuf, scan: Scan) -> Xtree {
        Xtree {
            hwnd,
            pick: root.clone(),
            root,
            scan,
            scanning: false,
            arena: Vec::new(),
            current: 0,
            selected: None,
            cells: Vec::new(),
            cells_for: None,
            hover: None,
            hover_row: None,
            hover_btn: None,
            hover_back: false,
            scroll: 0.0,
            by_type: false,
            vol: None,
            vol_remote: Default::default(),
            whole: false,
            scan_started: None,
            scan_elapsed: None,
            scan_stopped: false,
            before_elevate: None,
            pct_max: std::cell::Cell::new(0),
            drives: Vec::new(),
            menu_open: false,
            menu_hover: None,
            elevating: Arc::new(AtomicBool::new(false)),
            seen: false,
            ticks: 0,
        }
    }

    /// Vista sin escaneo (pruebas).
    #[cfg(test)]
    pub fn idle(root: PathBuf) -> Xtree {
        // La vista avisa a la ventana con `Handle::post`: en las pruebas, al `Waker` falso.
        crate::sys::testing::install_fakes();
        Xtree::with_scan(Handle::default(), root, idle_scan())
    }

    /// Generación del escaneo en curso: la de sus avisos (`WM_APP_XTREE`). Pruebas de `App`.
    #[cfg(test)]
    pub fn scan_id(&self) -> usize {
        self.scan.id
    }

    /// Reemplaza el árbol por uno terminado (pruebas).
    #[cfg(test)]
    pub fn set_arena(&mut self, arena: Vec<Node>) {
        self.arena = arena;
        self.scanning = false;
        self.go(0);
    }

    /// Tamaño de una carpeta (o archivo) del último escaneo terminado: se busca en el árbol tramo
    /// por tramo desde la raíz. `None` si no hay escaneo terminado o la ruta no está en él.
    pub fn size_of(&self, path: &Path) -> Option<u64> {
        let root = self.arena.first()?;
        let rel = path.strip_prefix(&self.root).ok()?;
        let mut node = root;
        for part in rel.components() {
            let name = part.as_os_str().to_string_lossy();
            let kids = node.children.iter().filter_map(|&c| self.arena.get(c as usize)).filter(|n| !n.small);
            let mut kids: Vec<&Node> = kids.collect();
            node = match kids.iter().position(|n| *n.name == *name) {
                Some(i) => kids.swap_remove(i),
                None => kids.into_iter().find(|n| n.name.eq_ignore_ascii_case(&name))?,
            };
        }
        Some(node.size)
    }

    /// Cierra el menú de discos (al cerrar la vista: no debe reaparecer abierto).
    pub fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_hover = None;
        self.hover_back = false;
    }

    /// Abre el menú de discos sin pasar por el clic. Lo usan las pruebas de `App` (app/tests.rs).
    pub fn open_menu(&mut self) {
        self.menu_open = true;
    }

    /// ¿Está abierto el menú de discos? Lo usan las pruebas de `App` (app/tests.rs).
    pub fn menu_is_open(&self) -> bool {
        self.menu_open
    }

    /// Cancela el escaneo en curso (al cerrar xdir o al cambiar de raíz). Si no había terminado,
    /// el cronómetro queda fijo y el estado dice «Detenido a los …».
    pub fn cancel(&mut self) {
        self.scan.cancel.store(true, Ordering::Relaxed);
        if (self.scanning || self.elevated_pending()) && self.scan_elapsed.is_none() {
            self.scan_elapsed = Some(self.elapsed());
            self.scan_stopped = true;
            self.before_elevate = None;
        }
    }

    /// Carpeta analizada.
    /// ¿El mapa está coloreado por tipo de archivo?
    pub fn by_type(&self) -> bool {
        self.by_type
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// ¿Se canceló el escaneo?
    pub fn cancelled(&self) -> bool {
        self.scan.cancel.load(Ordering::Relaxed)
    }

    /// Texto para la barra de estado de xdir mientras la vista está abierta.
    pub fn status_text(&self) -> String {
        if self.elevating.load(Ordering::Relaxed) {
            if self.elevated_pending() {
                format!("xtree \u{B7} {} \u{B7} {} \u{B7} {}", self.root.display(), disk_scan().elevating_text(), self.state_text())
            } else {
                format!("xtree \u{B7} {} \u{B7} {}", self.root.display(), disk_scan().elevating_text())
            }
        } else if self.scanning {
            format!("xtree \u{B7} {} \u{B7} {}", self.root.display(), self.state_text())
        } else if self.arena.is_empty() {
            format!("xtree \u{B7} {}", self.root.display())
        } else {
            format!("xtree \u{B7} {} \u{B7} {}", self.path_of(&self.arena, self.current).display(), self.state_text())
        }
    }

    /// Estado del escaneo para el indicador de la barra de estado (y de la columna izquierda).
    pub fn badge(&self) -> XtreeBadge {
        if self.scan_stopped {
            XtreeBadge::Idle
        } else if self.elevating.load(Ordering::Relaxed) || self.elevated_pending() {
            // El reescaneo con permisos no informa bytes leídos: gira.
            XtreeBadge::Spinning(self.ticks)
        } else if self.scanning {
            match self.percent() {
                Some(p) => XtreeBadge::Progress(p),
                None => XtreeBadge::Spinning(self.ticks),
            }
        } else if self.scan_elapsed.is_none() || self.seen {
            XtreeBadge::Idle
        } else if self.progress().denied.load(Ordering::Relaxed) > 0 {
            XtreeBadge::DoneWithIssues
        } else {
            XtreeBadge::Done
        }
    }

    /// Cartel del botón «xtree» con el detalle del indicador: «Escaneando / · 37 % · 0:42»,
    /// «Escaneando ~/Proyectos · 0:12», «Análisis completado en 1:23 · 458 carpetas sin permiso».
    /// Sin cartel cuando el indicador no se ve.
    pub fn badge_tip(&self) -> Option<String> {
        let t = fmt_elapsed(self.elapsed());
        let root = self.root.display();
        match self.badge() {
            XtreeBadge::Idle => None,
            XtreeBadge::Progress(p) => Some(format!("Escaneando {root} \u{B7} {p} % \u{B7} {t}")),
            XtreeBadge::Spinning(_) => Some(format!("Escaneando {root} \u{B7} {t}")),
            XtreeBadge::Done => Some(format!("Análisis completado en {t}")),
            XtreeBadge::DoneWithIssues => {
                let n = self.progress().denied.load(Ordering::Relaxed);
                let what = if n == 1 { "carpeta" } else { "carpetas" };
                Some(format!("Análisis completado en {t} \u{B7} {} {what} sin permiso", fmt_count(n)))
            }
        }
    }

    /// La vista se abrió (o está abierta): el resultado terminado ya se vio. Mientras escanea no
    /// cuenta: al terminar, el indicador vuelve a mostrarse hasta que se vea.
    pub fn mark_seen(&mut self) {
        if !self.scanning && !self.elevated_pending() {
            self.seen = true;
        }
    }

    /// Estado del análisis con el cronómetro: «Escaneando… 0:42», «Escaneando… 37 % · 0:42»,
    /// «Análisis completado en 1:23» o «Detenido a los 0:37».
    fn state_text(&self) -> String {
        let t = fmt_elapsed(self.elapsed());
        if !self.scanned() && !self.elevating.load(Ordering::Relaxed) {
            "Sin analizar".to_string()
        } else if self.scan_stopped {
            format!("Detenido a los {t}")
        } else if self.elevated_pending() || (self.scanning && self.elevating.load(Ordering::Relaxed)) {
            // El ayudante elevado no informa bytes leídos: solo el cronómetro (como el indicador,
            // que gira).
            format!("Escaneando\u{2026} {t}")
        } else if self.scanning {
            match self.percent() {
                Some(p) => format!("Escaneando\u{2026} {p} % \u{B7} {t}"),
                None => format!("Escaneando\u{2026} {t}"),
            }
        } else if self.scan_elapsed.is_some() {
            format!("Análisis completado en {t}")
        } else if self.arena.is_empty() {
            "Escaneando\u{2026}".to_string()
        } else {
            "Análisis completado".to_string()
        }
    }

    /// Hay un reescaneo con permisos en curso que empezó con el análisis ya terminado: cuenta como
    /// un escaneo nuevo (el árbol anterior se sigue viendo hasta que llegue el nuevo).
    fn elevated_pending(&self) -> bool {
        !self.scanning && self.scan_elapsed.is_none() && self.before_elevate.is_some()
    }

    /// Tiempo del escaneo: fijo si terminó o se cortó; si no, el que lleva.
    fn elapsed(&self) -> Duration {
        self.scan_elapsed.or_else(|| self.scan_started.map(|t| t.elapsed())).unwrap_or_default()
    }

    /// Porcentaje de avance mientras se escanea un volumen entero (bytes leídos sobre el espacio
    /// usado). Nunca baja dentro del mismo escaneo.
    fn percent(&self) -> Option<u8> {
        if !self.whole {
            return None;
        }
        let (total, free) = self.volume()?;
        // En Windows, si el escaneo rápido (MFT) pide administrador, el escáner vuelve a empezar
        // con el recorrido normal y los bytes leídos vuelven a 0: el porcentaje queda en `pct_max`
        // hasta que el recorrido lo alcanza (no retrocede).
        let p = scan_percent(self.progress().bytes.load(Ordering::Relaxed), total - free.min(total))?;
        let p = p.max(self.pct_max.get());
        self.pct_max.set(p);
        Some(p)
    }

    /// El espacio del volumen: directo si es local; de uno de red, en otro hilo (con tiempo
    /// límite), así el hilo de la ventana no espera al servidor.
    fn read_volume(&mut self) {
        if !crate::sys::network().is_remote(&self.root) {
            self.vol = disk_scan().volume_space(&self.root);
            return;
        }
        let (root, slot) = (self.root.clone(), self.vol_remote.clone());
        std::thread::spawn(move || {
            let host = crate::tab::remote_host(&root);
            if let Ok(Some(v)) = crate::net::with_timeout(crate::net::ITEM_LIMIT, &host, move || Ok(disk_scan().volume_space(&root)))
                && let Ok(mut s) = slot.lock()
            {
                *s = Some(v);
            }
        });
    }

    /// El espacio del volumen que se sabe (el de red, cuando llegó).
    fn volume(&self) -> Option<(u64, u64)> {
        self.vol.or_else(|| self.vol_remote.lock().ok().and_then(|s| *s))
    }

    /// Arranca el cronómetro de un escaneo nuevo (de 0) y toma los datos del volumen.
    fn start_clock(&mut self) {
        self.scan_started = Some(Instant::now());
        self.scan_elapsed = None;
        self.scan_stopped = false;
        self.seen = false;
        self.pct_max.set(0);
        self.read_volume();
        self.whole = !crate::sys::network().is_remote(&self.root) && disk_scan().whole_volume(&self.root);
    }

    /// Aviso de la barra de datos: escaneo elevado en curso o permisos que faltan (tecla A).
    fn data_hint(&self) -> String {
        let progress = self.progress();
        if self.elevating.load(Ordering::Relaxed) {
            disk_scan().elevating_text().to_string()
        } else if disk_scan().permission_issue(progress) {
            disk_scan().permission_hint().replace("{n}", &fmt_count(progress.denied.load(Ordering::Relaxed)))
        } else {
            String::new()
        }
    }

    fn legend_h(&self) -> f32 {
        if self.by_type { LEGEND_H } else { 0.0 }
    }

    fn progress(&self) -> &Progress {
        &self.scan.progress
    }

    /// Ruta real de un nodo.
    fn path_of(&self, arena: &[Node], id: u32) -> PathBuf {
        node_path(arena, &self.root, id)
    }

    /// Nombre para mostrar: la raíz se muestra con la ruta analizada (`C:\`, `/`, `/Users/x`).
    fn name_of(&self, arena: &[Node], id: u32) -> String {
        match arena.get(id as usize) {
            Some(n) if n.parent == NO_PARENT => self.root.display().to_string(),
            Some(n) => n.name.to_string(),
            None => String::new(),
        }
    }

    // ---------- escaneo ----------

    /// Vuelve a analizar `root` (cancela el escaneo anterior si seguía).
    fn rescan(&mut self) {
        self.cancel();
        let id = NEXT_SCAN.fetch_add(1, Ordering::Relaxed);
        self.scan = scan::start(self.hwnd, id, self.root.clone());
        self.arena.clear();
        self.cells.clear();
        self.cells_for = None;
        self.current = 0;
        self.hover = None;
        self.hover_row = None;
        self.hover_btn = None;
        self.selected = None;
        self.scroll = 0.0;
        self.scanning = true;
        self.start_clock();
    }

    /// Toma el árbol terminado, si ya llegó. Devuelve si cambió algo.
    fn take_result(&mut self) -> bool {
        let Some(arena) = self.scan.result.lock().unwrap().take() else { return false };
        self.arena = arena;
        // El espacio libre cambió mientras se escaneaba.
        self.read_volume();
        // Primer árbol de este escaneo: queda fijo el tiempo total. Un árbol elevado que llega
        // durante el recorrido normal lo reemplaza sin cambiar el tiempo; el de un reescaneo con
        // permisos (análisis ya terminado) siempre fija el tiempo de ese reescaneo.
        if self.before_elevate.take().is_some() || self.scan_elapsed.is_none() {
            self.scan_elapsed = Some(self.elapsed());
        }
        self.scanning = false;
        // Resultado nuevo: el indicador lo muestra hasta que se abra la vista.
        self.seen = false;
        self.go(0);
        true
    }

    /// Aviso del escáner (`WM_APP_XTREE`). Devuelve si hay que repintar.
    pub fn on_message(&mut self, wparam: usize, lparam: isize) -> bool {
        if wparam != self.scan.id {
            return false; // aviso de un escaneo que ya se reemplazó
        }
        // Fase del arco del indicador que gira: avanza con cada aviso (cada 250 ms).
        self.ticks = self.ticks.wrapping_add(1);
        if lparam == 1 {
            return self.take_result();
        }
        // El reescaneo con permisos terminó sin árbol (permiso rechazado o error): vuelve el
        // cronómetro del análisis anterior. Si dejó un árbol, su aviso de entrega puede llegar
        // después de un aviso de repintado: no se restaura nada y `take_result` fija el tiempo.
        if !self.elevating.load(Ordering::Relaxed)
            && self.elevated_pending()
            && self.scan.result.lock().unwrap().is_none()
            && let Some((started, elapsed, seen)) = self.before_elevate.take()
        {
            self.scan_started = started;
            self.scan_elapsed = elapsed;
            // Un resultado que ya se había visto no vuelve a aparecer como «terminado».
            self.seen = seen;
        }
        // Avance: el mapa en vivo se recalcula con lo que lleva descubierto el escáner. También
        // llega cuando termina el escaneo elevado sin árbol: hay que repintar su aviso.
        if self.scanning {
            self.cells_for = None;
        }
        true
    }

    /// Tecla A: vuelve a analizar con permisos (administrador en Windows; en macOS abre el ajuste
    /// de Acceso total al disco). Solo si faltaron permisos.
    fn elevate(&mut self) -> Action {
        let hwnd = self.hwnd;
        self.elevate_with(move |root, progress, cancel| disk_scan().elevate(hwnd, root, progress, cancel))
    }

    /// `elevate` con el escaneo elevado como parámetro (las pruebas pasan uno falso).
    fn elevate_with<F>(&mut self, run: F) -> Action
    where
        F: FnOnce(&Path, &Progress, &AtomicBool) -> Option<Vec<Node>> + Send + 'static,
    {
        if !disk_scan().permission_issue(self.progress()) || self.elevating.swap(true, Ordering::Relaxed) {
            return Action::None;
        }
        let (root, hwnd, id) = (self.root.clone(), self.hwnd, self.scan.id);
        // Con el análisis ya terminado es un escaneo nuevo: el cronómetro arranca de 0 (incluye el
        // pedido de permisos) y, como el escáner ya no avisa, un hilo pide repintar cada 250 ms
        // mientras dure. Si el recorrido normal sigue, su tiempo ya incluye al ayudante.
        if !self.scanning && !self.scan_stopped && self.scan_elapsed.is_some() {
            self.before_elevate = Some((self.scan_started, self.scan_elapsed, self.seen));
            self.start_clock();
            let (elevating, cancel) = (self.elevating.clone(), self.scan.cancel.clone());
            let _ = std::thread::Builder::new().spawn(move || {
                // También se corta al cancelar (cerrar xdir o cambiar de raíz).
                while elevating.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                    hwnd.post(WM_APP_XTREE, id, 0);
                    std::thread::sleep(Duration::from_millis(250));
                }
            });
        }
        let (progress, cancel, result, elevating) =
            (self.scan.progress.clone(), self.scan.cancel.clone(), self.scan.result.clone(), self.elevating.clone());
        let _ = std::thread::Builder::new().spawn(move || {
            let tree = run(&root, &progress, &cancel);
            let delivered = tree.is_some_and(|arena| scan::deliver(&result, &cancel, arena, true));
            if delivered {
                // Ya no faltan permisos: se apagan el aviso y la tecla A.
                progress.need_admin.store(false, Ordering::Relaxed);
                progress.denied.store(0, Ordering::Relaxed);
            }
            elevating.store(false, Ordering::Relaxed);
            // Siempre se avisa: el escaneo normal pudo haber terminado y ya no hay quien repinte.
            hwnd.post(WM_APP_XTREE, id, delivered as isize);
        });
        Action::Redraw
    }

    // ---------- navegación ----------

    fn go(&mut self, node: u32) {
        self.current = node;
        // Los bloques son de la carpeta (o del árbol) anterior: no deben responder más.
        self.cells.clear();
        self.cells_for = None;
        self.hover = None;
        self.hover_row = None;
        self.selected = None;
        self.scroll = 0.0;
    }

    fn go_up(&mut self) -> bool {
        match self.arena.get(self.current as usize) {
            Some(n) if n.parent != NO_PARENT => {
                self.go(n.parent);
                true
            }
            _ => false,
        }
    }

    /// Filas de la tabla: `None` es «subir»; el resto son los hijos de la carpeta actual.
    fn rows(&self) -> Vec<Option<u32>> {
        rows_of(&self.arena, self.current)
    }

    /// Elemento sobre el que actúan los botones y que muestra la tarjeta: el seleccionado, o la
    /// carpeta actual si no hay selección.
    fn target(&self) -> u32 {
        self.selected.unwrap_or(self.current)
    }

    // ---------- hit-tests ----------

    fn row_at(&self, area: Rect, x: f32, y: f32) -> Option<usize> {
        let ui = layout_ui(area, self.legend_h());
        let top = ui.list.top + TABLE_H;
        if !contains(&ui.list, x, y) || y < top {
            return None;
        }
        let i = ((y - top + self.scroll) / ROW_H) as usize;
        (i < self.rows().len()).then_some(i)
    }

    fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
        self.cells.iter().rposition(|c| contains(&c.r, x, y))
    }

    fn button_at(&self, area: Rect, x: f32, y: f32) -> Option<usize> {
        layout_ui(area, self.legend_h()).buttons.iter().position(|b| contains(b, x, y))
    }

    /// Fila del menú de discos bajo el cursor.
    fn menu_row_at(&self, area: Rect, x: f32, y: f32) -> Option<usize> {
        let n = self.drives.len() + 1;
        let m = menu_rect(&layout_ui(area, self.legend_h()), n);
        if !contains(&m, x, y) || y < m.top + 4.0 {
            return None;
        }
        let i = ((y - m.top - 4.0) / MENU_ROW) as usize;
        (i < n).then_some(i)
    }

    /// Elige una fila del menú: un disco, o el diálogo para elegir una carpeta.
    fn choose(&mut self, i: usize) {
        if let Some(d) = self.drives.get(i) {
            self.pick = d.path.clone();
        } else if let Some(p) = crate::sys::shell().pick_folder(self.hwnd, "Elige la carpeta a analizar") {
            self.pick = p;
        }
    }

    /// Analiza lo que está elegido en el desplegable.
    fn scan_pick(&mut self) {
        if self.pick.as_os_str().is_empty() {
            return;
        }
        self.root = self.pick.clone();
        self.rescan();
    }

    /// Texto del desplegable: el disco elegido o, si es una carpeta, su ruta.
    fn pick_label(&self) -> String {
        let trim = |s: String| s.trim_end_matches(['\\', '/']).to_lowercase();
        let p = trim(self.pick.to_string_lossy().into_owned());
        match self.drives.iter().find(|d| trim(d.path.to_string_lossy().into_owned()) == p) {
            Some(d) => d.label.clone(),
            None => self.pick.display().to_string(),
        }
    }

    // ---------- eventos ----------

    /// Movimiento del mouse. Devuelve si hay que repintar.
    pub fn on_mouse_move(&mut self, area: Rect, x: f32, y: f32) -> bool {
        if self.menu_open {
            let row = self.menu_row_at(area, x, y);
            let changed = row != self.menu_hover;
            self.menu_hover = row;
            return changed;
        }
        let back = contains(&back_rect(area), x, y);
        let back_changed = back != self.hover_back;
        self.hover_back = back;
        if self.scanning {
            return back_changed; // mientras se escanea el mapa es solo de lectura
        }
        let (row, cell, btn) = (self.row_at(area, x, y), self.cell_at(x, y), self.button_at(area, x, y));
        let changed = back_changed || row != self.hover_row || cell != self.hover || btn != self.hover_btn;
        self.hover_row = row;
        self.hover = cell;
        self.hover_btn = btn;
        changed
    }

    /// Rueda sobre la tabla: `lines` filas (positivo: hacia arriba).
    pub fn on_wheel(&mut self, area: Rect, lines: f32) -> bool {
        let ui = layout_ui(area, self.legend_h());
        let visible = ui.list.bottom - ui.list.top - TABLE_H;
        let max = (self.rows().len() as f32 * ROW_H - visible).max(0.0);
        let scroll = (self.scroll - lines * ROW_H).clamp(0.0, max);
        let changed = scroll != self.scroll;
        self.scroll = scroll;
        changed
    }

    /// Clic (o doble clic) con el botón izquierdo.
    pub fn on_click(&mut self, area: Rect, x: f32, y: f32, double: bool) -> Action {
        if double {
            return self.on_double_click(area, x, y);
        }
        // Barra superior: desplegable de discos y botón «Analizar» (también mientras se escanea).
        if self.menu_open {
            let row = self.menu_row_at(area, x, y);
            self.menu_open = false;
            self.menu_hover = None;
            if let Some(i) = row {
                self.choose(i);
            }
            return Action::Redraw;
        }
        let ui = layout_ui(area, self.legend_h());
        if contains(&ui.back, x, y) {
            return Action::Close;
        }
        if contains(&ui.combo, x, y) {
            self.drives = disk_scan().drives();
            self.menu_open = true;
            return Action::Redraw;
        }
        if contains(&ui.scan_btn, x, y) {
            self.scan_pick();
            return Action::Redraw;
        }
        if self.arena.is_empty() || self.scanning {
            return Action::None;
        }
        if let Some(b) = self.button_at(area, x, y) {
            let t = self.target();
            let Some(n) = self.arena.get(t as usize) else { return Action::None };
            return match b {
                0 => {
                    // La carpeta del elemento; si es un archivo, su carpeta con el archivo enfocado.
                    if n.is_dir || n.parent == NO_PARENT {
                        Action::OpenInPanel { folder: self.path_of(&self.arena, t), select: None }
                    } else {
                        let select = (!n.small).then(|| OsString::from(&*n.name));
                        Action::OpenInPanel { folder: self.path_of(&self.arena, n.parent), select }
                    }
                }
                1 => {
                    let _ = crate::sys::shell().copy_text(self.hwnd, &self.path_of(&self.arena, t).to_string_lossy());
                    Action::None
                }
                _ if n.is_dir && t != self.current => {
                    self.go(t);
                    Action::Redraw
                }
                _ => Action::None,
            };
        }
        if let Some(i) = self.row_at(area, x, y) {
            match self.rows()[i] {
                None => {
                    self.go_up();
                }
                Some(n) => self.selected = Some(n),
            }
        } else if let Some(i) = self.cell_at(x, y) {
            self.selected = Some(self.cells[i].node);
        } else {
            return Action::None;
        }
        Action::Redraw
    }

    fn on_double_click(&mut self, area: Rect, x: f32, y: f32) -> Action {
        if self.scanning || self.menu_open {
            return Action::None;
        }
        let node = if let Some(i) = self.row_at(area, x, y) {
            self.rows()[i]
        } else {
            self.cell_at(x, y).map(|i| self.cells[i].top)
        };
        match node {
            Some(n) if self.arena.get(n as usize).is_some_and(|n| n.is_dir) => {
                self.go(n);
                Action::Redraw
            }
            _ => Action::None,
        }
    }

    /// Clic derecho: sube a la carpeta superior.
    pub fn on_right_click(&mut self) -> Action {
        if self.go_up() { Action::Redraw } else { Action::None }
    }

    /// Teclas de la vista: Retroceso, Esc, T, F5 y A.
    pub fn on_key(&mut self, k: Key) -> Action {
        match k {
            VK_BACK => self.on_right_click(),
            VK_ESCAPE if self.menu_open => {
                self.menu_open = false;
                self.menu_hover = None;
                Action::Redraw
            }
            VK_ESCAPE => Action::Close,
            VK_T => {
                self.by_type = !self.by_type;
                self.cells_for = None;
                Action::Redraw
            }
            // Sin análisis todavía, F5 es «Analizar»: lo elegido en el desplegable.
            VK_F5 if !self.scanned() => {
                self.scan_pick();
                Action::Redraw
            }
            VK_F5 => {
                self.rescan();
                Action::Redraw
            }
            VK_A => self.elevate(),
            _ => Action::None,
        }
    }

    // ---------- dibujo ----------

    /// Recalcula lo que depende del árbol y del área: bloques del mapa y límite del scroll.
    fn prepare(&mut self, area: Rect) {
        // El escáner terminó y el aviso todavía no llegó: se toma el resultado ya (si no, el árbol
        // en vivo queda vacío un instante).
        if self.scanning {
            self.take_result();
        }
        let ui = layout_ui(area, self.legend_h());
        let live = self.scan.live.clone();
        let guard = if self.scanning { Some(live.read().unwrap()) } else { None };
        let arena: &[Node] = match &guard {
            Some(g) => g.as_slice(),
            None => self.arena.as_slice(),
        };
        let key = (self.current, [ui.map.left as i32, ui.map.top as i32, ui.map.right as i32, ui.map.bottom as i32]);
        if self.cells_for != Some(key) {
            self.cells.clear();
            if !arena.is_empty() && ui.map.right - ui.map.left > 8.0 && ui.map.bottom - ui.map.top > 8.0 {
                layout(arena, self.current, ui.map, &mut self.cells);
            }
            self.cells_for = Some(key);
            self.hover = None;
        }
        let visible = (ui.list.bottom - ui.list.top - TABLE_H).max(0.0);
        self.scroll = self.scroll.clamp(0.0, (rows_of(arena, self.current).len() as f32 * ROW_H - visible).max(0.0));
    }

    pub fn paint(&mut self, p: &dyn Painter, area: Rect) {
        self.prepare(area);
        let th = p.theme();
        let ui = layout_ui(area, self.legend_h());
        let (l0, t0, r0, b0) = (area.left, area.top, area.right, area.bottom);
        let white = color(1.0, 1.0, 1.0);
        let warning = if th.dark { rgb(FOLDER_ICON) } else { hex(0xB7791F) };
        let file_icon = if th.dark { hex(0xD9E0ED) } else { hex(0x9AA5B3) };
        // Neutros del mapa por tipo: marcos de carpetas anidadas y teselas de archivos chicos.
        let map_dir_n = if th.dark { hex(0x1A2432) } else { hex(0xE6EBF1) };
        let map_small = if th.dark { hex(0x131A27) } else { hex(0xE1E7ED) };
        let stroke = |r: Rect, c: Color, w: f32| p.round_stroke_w(r, 0.0, c, w);
        // Íconos simples: carpeta dorada o página.
        let icon = |x: f32, y: f32, dir: bool, c: Color| {
            if dir {
                p.round_fill(rect(x, y + 3.0, x + 7.0, y + 7.0), 1.5, c);
                p.round_fill(rect(x, y + 5.0, x + 16.0, y + 15.0), 2.0, c);
            } else {
                p.round_fill(rect(x + 2.0, y, x + 14.0, y + 16.0), 2.0, c);
            }
        };

        p.fill(area, th.background);

        // Mientras se escanea se dibuja el árbol compartido (con el bloqueo de lectura tomado).
        let live = self.scan.live.clone();
        let guard = if self.scanning { Some(live.read().unwrap()) } else { None };
        let arena: &[Node] = match &guard {
            Some(g) => g.as_slice(),
            None => self.arena.as_slice(),
        };
        let progress = self.scan.progress.clone();
        let done = !self.scanning && !arena.is_empty();

        // ----- barra superior: disco y total a la izquierda, estado del análisis a la derecha -----
        let unscanned = !self.scanned() && arena.is_empty();
        let top_text = if unscanned {
            self.root.display().to_string()
        } else if done {
            format!("{}  \u{B7}  {} analizados", self.name_of(arena, 0), fmt_size(arena[0].size))
        } else {
            format!(
                "{}  \u{B7}  {} archivos - {}",
                self.root.display(),
                fmt_count(progress.files.load(Ordering::Relaxed)),
                fmt_size(progress.bytes.load(Ordering::Relaxed))
            )
        };
        p.text(FontId::Title, &top_text, rect(ui.scan_btn.right + 18.0, t0, r0 - STATE_W - 60.0, t0 + TOP_H), th.text);
        // Barra de avance (volumen entero) sobre el borde inferior de la barra superior.
        if self.scanning && !self.scan_stopped
            && let Some(pc) = self.percent()
        {
            let (y0, y1) = (t0 + TOP_H - 3.0, t0 + TOP_H);
            p.fill(rect(l0, y0, r0, y1), th.border);
            p.fill(rect(l0, y0, l0 + (r0 - l0) * pc as f32 / 100.0, y1), th.accent);
        }
        let bb = ui.back;
        p.round_fill(bb, 6.0, if self.hover_back { th.hover } else { th.surface_alt });
        p.round_stroke_w(bb, 6.0, th.border, 1.0);
        p.text(FontId::UiCenter, "\u{2190} xdir", bb, th.text);
        let cb = ui.combo;
        p.round_fill(cb, 6.0, th.surface_alt);
        p.round_stroke_w(cb, 6.0, if self.menu_open { th.accent } else { th.border }, 1.0);
        p.text(FontId::Ui, &self.pick_label(), rect(cb.left + 10.0, cb.top, cb.right - 28.0, cb.bottom), th.text);
        p.text(FontId::GlyphsSmall, "\u{E70D}", rect(cb.right - 26.0, cb.top, cb.right - 6.0, cb.bottom), th.text_dim);
        p.round_fill(ui.scan_btn, 6.0, th.accent);
        p.text(FontId::UiCenter, "Analizar", ui.scan_btn, white);
        let state = self.state_text();
        // Con el reescaneo con permisos en curso el árbol anterior se sigue viendo, pero el estado
        // es de escaneo.
        let done_state = done && !self.elevated_pending();
        if unscanned {
            // Sin análisis: sin punto de estado, el texto apagado.
            p.text(FontId::Ui, &state, rect(r0 - STATE_W, t0, r0 - MARGIN, t0 + TOP_H), th.text_dim);
        } else {
            p.circle(r0 - STATE_W - 18.0, t0 + TOP_H / 2.0, 9.0, if done_state { rgb(GREEN) } else { warning });
            if done_state {
                p.text(FontId::UiCenter, "\u{2713}", rect(r0 - STATE_W - 27.0, t0, r0 - STATE_W - 9.0, t0 + TOP_H), white);
            }
            p.text(FontId::Ui, &state, rect(r0 - STATE_W, t0, r0 - MARGIN, t0 + TOP_H), th.text);
        }

        if arena.is_empty() {
            p.round_fill(ui.map, 8.0, th.surface);
            p.round_stroke_w(ui.map, 8.0, th.border, 1.0);
            p.round_fill(ui.list, 8.0, th.surface);
            p.round_stroke_w(ui.list, 8.0, th.border, 1.0);
            let msg = match progress.phase.load(Ordering::Relaxed) {
                1 => {
                    let total = progress.mft_total.load(Ordering::Relaxed).max(1);
                    let pct = (progress.mft_done.load(Ordering::Relaxed) * 100 / total).min(100);
                    format!("Leyendo la tabla de archivos (MFT)\u{2026} {pct} %")
                }
                2 => "Armando el árbol de carpetas\u{2026}".to_string(),
                _ if unscanned => "Elegí un disco o una carpeta y tocá Analizar".to_string(),
                _ if self.scan_stopped => self.state_text(),
                _ if self.scanning => "Escaneando el disco\u{2026}".to_string(),
                _ => "No hay nada que mostrar".to_string(),
            };
            p.text(FontId::UiCenter, &msg, ui.map, th.text_dim);
        } else {
            let cur = &arena[self.current as usize];
            let rows = rows_of(arena, self.current);
            let hover_node = self
                .hover
                .and_then(|i| self.cells.get(i))
                .map(|c| c.node)
                .or_else(|| self.hover_row.and_then(|i| rows.get(i).copied().flatten()));
            // `hot_top`: hijo directo de la carpeta actual bajo el mouse; se resalta en mapa y tabla.
            let hot_top = self
                .hover
                .and_then(|i| self.cells.get(i))
                .map(|c| c.top)
                .or_else(|| self.hover_row.and_then(|i| rows.get(i).copied().flatten()));

            // ----- tabla -----
            let l = ui.list;
            p.round_fill(l, 8.0, th.surface);
            p.round_stroke_w(l, 8.0, th.border, 1.0);
            let (hy0, hy1) = (l.top, l.top + TABLE_H);
            p.text(FontId::Ui, "Nombre", rect(l.left + 36.0, hy0, l.right - 214.0, hy1), th.text_dim);
            p.text(FontId::UiRight, "Tamaño", rect(l.right - 214.0, hy0, l.right - 124.0, hy1), th.text_dim);
            p.text(FontId::UiRight, "%", rect(l.right - 124.0, hy0, l.right - 72.0, hy1), th.text_dim);
            p.fill(rect(l.left + 1.0, l.top + TABLE_H - 1.0, l.right - 1.0, l.top + TABLE_H), th.border);
            p.clip(rect(l.left, l.top + TABLE_H, l.right, l.bottom - 2.0), &mut || {
                for (i, row) in rows.iter().enumerate() {
                    let top = l.top + TABLE_H + i as f32 * ROW_H - self.scroll;
                    if top + ROW_H < l.top + TABLE_H || top > l.bottom {
                        continue;
                    }
                    let band = rect(l.left + 3.0, top + 1.0, l.right - 3.0, top + ROW_H - 1.0);
                    let is_sel = row.is_some() && *row == self.selected;
                    let is_hot = self.hover_row == Some(i) || (row.is_some() && *row == hot_top);
                    if is_sel {
                        p.round_fill(band, 5.0, th.selection);
                        p.round_stroke_w(band, 5.0, th.accent, 1.0);
                    } else if is_hot {
                        p.round_fill(band, 5.0, th.hover);
                    }
                    match row {
                        None => p.text(FontId::Ui, "\u{2191}  Subir", rect(l.left + 12.0, top, l.right - 8.0, top + ROW_H), th.text_dim),
                        Some(id) => {
                            let n = &arena[*id as usize];
                            let frac = if cur.size > 0 { n.size as f32 / cur.size as f32 } else { 0.0 };
                            let ic = if n.is_dir { rgb(FOLDER_ICON) } else { file_icon };
                            icon(l.left + 12.0, top + (ROW_H - 16.0) / 2.0, n.is_dir, ic);
                            p.text(FontId::Ui, &n.name, rect(l.left + 36.0, top, l.right - 214.0, top + ROW_H), th.text);
                            p.text(FontId::UiRight, &fmt_size(n.size), rect(l.right - 214.0, top, l.right - 124.0, top + ROW_H), th.text);
                            let pc = format!("{:.1} %", frac * 100.0).replace('.', ",");
                            p.text(FontId::UiRight, &pc, rect(l.right - 124.0, top, l.right - 72.0, top + ROW_H), th.text_dim);
                            let track = rect(l.right - 62.0, top + ROW_H / 2.0 - 4.0, l.right - 12.0, top + ROW_H / 2.0 + 4.0);
                            p.round_fill(track, 4.0, th.border);
                            let bar_w = ((track.right - track.left) * frac).max(3.0);
                            p.round_fill(rect(track.left, track.top, track.left + bar_w, track.bottom), 4.0, th.accent);
                        }
                    }
                }
            });

            // ----- mapa -----
            p.round_fill(ui.map, 8.0, th.surface);
            p.clip(ui.map, &mut || {
                for c in &self.cells {
                    let Some(n) = arena.get(c.node as usize) else { continue };
                    let same = hot_top == Some(c.top);
                    let exact = hover_node == Some(c.node);
                    let sel = self.selected == Some(c.node);
                    let (cw, ch) = (c.r.right - c.r.left, c.r.bottom - c.r.top);
                    if c.dir {
                        let lift = if exact { 0.07 } else if same { 0.035 } else { 0.0 };
                        let tinted = !self.by_type;
                        let base = if tinted {
                            position_color(c.hue, c.depth, true)
                        } else if c.depth == 0 {
                            th.surface_alt
                        } else {
                            map_dir_n
                        };
                        p.round_fill(c.r, 3.0, lighten(base, lift));
                        p.round_stroke_w(c.r, 3.0, if tinted { th.background } else { th.border }, 1.0);
                        let hh = header_h(c.depth, cw, ch);
                        if hh > 0.0 {
                            let label = format!("{}   {}", n.name, fmt_size(n.size));
                            let font = if c.depth == 0 { FontId::SmallBold } else { FontId::Small };
                            let tc = if tinted { text_on(base) } else if c.depth == 0 { th.text } else { th.text_dim };
                            p.text(font, &label, rect(c.r.left + 8.0, c.r.top + 2.0, c.r.right - 4.0, c.r.top + 2.0 + hh), tc);
                        }
                    } else {
                        let col = if c.small {
                            map_small // grupo de archivos pequeños: casi del color del fondo
                        } else if self.by_type {
                            file_color(&n.name)
                        } else {
                            position_color(c.hue, c.depth, false)
                        };
                        let col = if exact { lighten(col, 0.22) } else if same { lighten(col, 0.08) } else { col };
                        p.fill(rect(c.r.left + 0.5, c.r.top + 0.5, c.r.right - 0.5, c.r.bottom - 0.5), col);
                        let tc = if c.small { th.text_dim } else if self.by_type { white } else { text_on(col) };
                        if cw >= 60.0 && ch >= 38.0 {
                            p.text(FontId::Small, &n.name, rect(c.r.left + 6.0, c.r.top + 3.0, c.r.right - 3.0, c.r.top + 19.0), tc);
                            p.text(FontId::Small, &fmt_size(n.size), rect(c.r.left + 6.0, c.r.top + 19.0, c.r.right - 3.0, c.r.top + 35.0), tc);
                        } else if cw >= 60.0 && ch >= 18.0 {
                            p.text(FontId::Small, &n.name, rect(c.r.left + 6.0, c.r.top, c.r.right - 3.0, c.r.bottom), tc);
                        }
                    }
                    if sel {
                        stroke(rect(c.r.left + 1.0, c.r.top + 1.0, c.r.right - 1.0, c.r.bottom - 1.0), th.accent, 2.0);
                    }
                }
            });
            p.round_stroke_w(ui.map, 8.0, th.border, 1.0);

            // ----- tarjeta de detalle -----
            // Un nodo que no está en este árbol (selección vieja) cae en la carpeta actual.
            let shown = hover_node.or(self.selected).filter(|&n| (n as usize) < arena.len()).unwrap_or(self.current);
            let n = &arena[shown as usize];
            let cd = ui.card;
            p.round_fill(cd, 8.0, th.surface);
            p.round_stroke_w(cd, 8.0, th.border, 1.0);
            let cy = (cd.top + cd.bottom) / 2.0;
            if n.is_dir {
                p.round_fill(rect(cd.left + 18.0, cy - 12.0, cd.left + 30.0, cy - 6.0), 2.0, rgb(FOLDER_ICON));
                p.round_fill(rect(cd.left + 18.0, cy - 8.0, cd.left + 46.0, cy + 12.0), 3.0, rgb(FOLDER_ICON));
            } else {
                p.round_fill(rect(cd.left + 20.0, cy - 15.0, cd.left + 44.0, cy + 15.0), 3.0, file_icon);
            }
            let text_right = ui.buttons[0].left - 14.0;
            let title = format!("{}  \u{B7}  {}", self.name_of(arena, shown), fmt_size(n.size));
            p.text(FontId::Title, &title, rect(cd.left + 62.0, cd.top + 8.0, text_right, cd.top + 38.0), th.text);
            let path = self.path_of(arena, shown).display().to_string();
            p.text(FontId::Ui, &path, rect(cd.left + 62.0, cd.top + 38.0, text_right, cd.top + 66.0), th.text_dim);
            let labels = [("\u{E838}", "Abrir en el panel"), ("\u{E8C8}", "Copiar ruta"), ("\u{E8A7}", "Explorar aquí")];
            let t = self.target();
            let target_is_dir = arena.get(t as usize).is_some_and(|n| n.is_dir) && t != self.current;
            for (i, (glyph, label)) in labels.iter().enumerate() {
                let b = ui.buttons[i];
                let enabled = i != 2 || target_is_dir;
                let hot = self.hover_btn == Some(i) && enabled;
                let k = if enabled { 1.0 } else { 0.5 };
                p.round_fill(b, 7.0, th.surface_alt);
                if hot {
                    p.round_fill(b, 7.0, th.hover);
                }
                p.round_stroke_w(b, 7.0, alpha(if hot { th.accent } else { th.border }, k), 1.0);
                let tc = alpha(th.text, k);
                p.text(FontId::Glyphs, glyph, rect(b.left + 10.0, b.top, b.left + 30.0, b.bottom), tc);
                p.text(FontId::Ui, label, rect(b.left + 40.0, b.top, b.right - 6.0, b.bottom), tc);
            }

            // ----- leyenda (solo al colorear por tipo; por carpeta el color no necesita explicación) -----
            if self.by_type {
                let lg = ui.legend;
                p.round_fill(lg, 8.0, th.surface);
                p.round_stroke_w(lg, 8.0, th.border, 1.0);
                let ly = (lg.top + lg.bottom) / 2.0;
                let mut entries: Vec<(&str, Color)> = CATS.iter().map(|(n, c, _)| (*n, rgb(*c))).collect();
                entries.push(("Carpeta", th.accent));
                let slot = (lg.right - lg.left) / entries.len() as f32;
                for (i, (name, c)) in entries.iter().enumerate() {
                    let x = lg.left + 16.0 + slot * i as f32;
                    p.round_fill(rect(x, ly - 9.0, x + 18.0, ly + 9.0), 4.0, *c);
                    p.text(FontId::Ui, name, rect(x + 26.0, ly - 12.0, x + slot - 4.0, ly + 12.0), th.text);
                }
            }
        }

        // ----- barra de datos del disco: carpeta actual, espacio total, utilizado y libre -----
        let bar = rect(l0 + MARGIN, ui.status.top + 4.0, r0 - MARGIN, b0 - 6.0);
        p.round_fill(bar, 8.0, th.surface);
        p.round_stroke_w(bar, 8.0, th.border, 1.0);
        let mut cols: Vec<(&str, String)> = Vec::new();
        if done {
            let cur = &arena[self.current as usize];
            let count = cur.children.iter().filter(|&&c| arena.get(c as usize).is_some_and(|n| n.size > 0)).count();
            cols.push(("Carpeta actual", format!("{} \u{B7} {} elementos", fmt_size(cur.size), count)));
        } else if !unscanned {
            cols.push(("Analizado", fmt_size(progress.bytes.load(Ordering::Relaxed))));
        }
        if let Some((total, free)) = self.volume() {
            let free = free.min(total);
            let used = total - free;
            cols.push(("Espacio total", fmt_size(total)));
            cols.push(("Utilizado", format!("{} ({})", fmt_size(used), pct(used, total))));
            cols.push(("Libre", format!("{} ({})", fmt_size(free), pct(free, total))));
            if done {
                let seen = arena[0].size;
                if disk_scan().whole_volume(&self.root) {
                    cols.push(("Sin analizar", fmt_size(used.saturating_sub(seen))));
                } else {
                    cols.push(("Analizado", format!("{} ({} del uso)", fmt_size(seen), pct(seen, used))));
                }
            }
            // Barra de ocupación del volumen: roja cuando queda poco espacio libre.
            let track = rect(bar.left + 16.0, bar.bottom - 10.0, bar.right - 16.0, bar.bottom - 6.0);
            p.round_fill(track, 2.0, th.border);
            let frac = used as f32 / total.max(1) as f32;
            let fillc = if frac > 0.9 { rgb(RED) } else { th.accent };
            p.round_fill(rect(track.left, track.top, track.left + (track.right - track.left) * frac, track.bottom), 2.0, fillc);
        }
        // Aviso de permisos: administrador en Windows, Acceso total al disco en macOS (tecla A).
        let hint = self.data_hint();
        // Con el aviso, las columnas se achican para que entre el texto.
        let widths: [f32; 5] = if hint.is_empty() { [250.0, 120.0, 190.0, 180.0, 190.0] } else { [220.0, 110.0, 170.0, 150.0, 170.0] };
        let mut x = bar.left + 16.0;
        for (i, (label, value)) in cols.iter().enumerate() {
            let cw = widths[i.min(widths.len() - 1)];
            p.text(FontId::Small, label, rect(x, bar.top + 4.0, x + cw, bar.top + 20.0), th.text_dim);
            p.text(FontId::UiBold, value, rect(x, bar.top + 19.0, x + cw, bar.top + 37.0), th.text);
            x += cw;
        }
        if !hint.is_empty() {
            p.text(FontId::UiRight, &hint, rect((x + 10.0).max(bar.right - 480.0), bar.top + 10.0, bar.right - 16.0, bar.top + 30.0), warning);
        } else if done {
            let help = if self.by_type { "T: color por carpeta" } else { "T: color por tipo de archivo" };
            p.text(FontId::UiRight, help, rect((x + 10.0).max(bar.right - 300.0), bar.top + 10.0, bar.right - 16.0, bar.top + 30.0), th.text_dim);
        }

        // ----- lista de discos: va al final para quedar por encima de todo -----
        if self.menu_open {
            let n = self.drives.len() + 1;
            let m = menu_rect(&ui, n);
            p.round_fill(m, 8.0, th.surface);
            p.round_stroke_w(m, 8.0, th.accent, 1.0);
            for i in 0..n {
                let top = m.top + 4.0 + i as f32 * MENU_ROW;
                let row = rect(m.left + 4.0, top, m.right - 4.0, top + MENU_ROW);
                if self.menu_hover == Some(i) {
                    p.round_fill(row, 5.0, th.selection);
                }
                if let Some(d) = self.drives.get(i) {
                    p.text(FontId::Ui, &d.label, rect(row.left + 10.0, top, row.right - 170.0, top + MENU_ROW), th.text);
                    p.text(FontId::UiRight, &d.detail, rect(row.right - 170.0, top, row.right - 10.0, top + MENU_ROW), th.text_dim);
                } else {
                    p.text(FontId::Ui, "Seleccionar carpeta\u{2026}", rect(row.left + 10.0, top, row.right - 10.0, top + MENU_ROW), th.text);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::keys::{VK_BACK, VK_ESCAPE};
    use crate::geom::rect;
    use crate::xtree::tree::{NO_PARENT, Node};
    use std::time::{Duration, Instant};

    fn node(name: &str, parent: u32, size: u64, is_dir: bool, children: Vec<u32>) -> Node {
        Node { name: name.into(), parent, size, is_dir, small: false, children }
    }

    /// Recorre la vista entera con hit-tests y clics: nada debe entrar en pánico.
    fn poke(x: &mut Xtree, area: Rect) {
        x.prepare(area);
        let ui = layout_ui(area, x.legend_h());
        for r in [ui.list, ui.map, ui.card, ui.legend, ui.status, ui.back, ui.combo, ui.scan_btn, ui.buttons[0], ui.buttons[1], ui.buttons[2]] {
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            assert_eq!(x.row_at(area, cx, cy), None);
            let _ = x.cell_at(cx, cy);
            let _ = x.button_at(area, cx, cy);
            x.on_mouse_move(area, cx, cy);
            if r != ui.combo && r != ui.scan_btn {
                let _ = x.on_click(area, cx, cy, false);
                let _ = x.on_click(area, cx, cy, true);
            }
        }
        x.on_wheel(area, -3.0);
        let _ = x.on_right_click();
        let _ = x.on_key(VK_BACK);
        x.prepare(area);
    }

    /// «← xdir» va a la izquierda de todo en la barra superior; el desplegable y «Analizar» se
    /// corren a su derecha.
    #[test]
    fn back_button_left_of_combo() {
        let area = rect(0.0, 0.0, 1400.0, 790.0);
        let ui = layout_ui(area, 0.0);
        assert_eq!(ui.back, back_rect(area));
        assert_eq!((ui.back.left, ui.back.right - ui.back.left), (MARGIN, 76.0));
        assert_eq!(ui.combo.left, ui.back.right + 8.0);
        assert!(ui.scan_btn.left > ui.combo.right);
        assert_eq!((ui.back.top, ui.back.bottom), (ui.combo.top, ui.combo.bottom));
    }

    /// El clic en «← xdir» cierra la vista (también con el menú de discos cerrado y mientras se
    /// escanea); el mouse encima lo resalta.
    #[test]
    fn back_button_closes_view() {
        let area = rect(0.0, 0.0, 1400.0, 790.0);
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        let b = layout_ui(area, 0.0).back;
        let (cx, cy) = ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
        assert!(x.on_mouse_move(area, cx, cy), "resalta");
        assert!(!x.on_mouse_move(area, cx + 1.0, cy), "sigue resaltado");
        assert_eq!(x.on_click(area, cx, cy, false), Action::Close);
        x.scanning = true;
        assert_eq!(x.on_click(area, cx, cy, false), Action::Close);
        let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        x.scanning = false;
        x.paint(&p, area);
        assert!(p.texts().iter().any(|(_, t, _)| t == "\u{2190} xdir"));
    }

    /// Vista abierta sin analizar: no escanea, el indicador queda quieto, el desplegable muestra la
    /// raíz y el mapa dice qué hacer.
    #[test]
    fn unscanned_view_shows_hint() {
        crate::sys::testing::install_fakes();
        let area = rect(0.0, 0.0, 1400.0, 790.0);
        let root = std::env::temp_dir().join("xdir-xtree-sin-analizar");
        let mut x = Xtree::unscanned(Handle::default(), root.clone(), true);
        assert!(!x.scanned() && !x.scanning && x.by_type());
        assert_eq!(x.badge(), XtreeBadge::Idle);
        assert_eq!(x.status_text(), format!("xtree \u{B7} {}", root.display()));
        assert_eq!(x.pick_label(), root.display().to_string());
        let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        x.paint(&p, area);
        let texts: Vec<String> = p.texts().into_iter().map(|(_, t, _)| t).collect();
        assert!(texts.iter().any(|t| t == "Elegí un disco o una carpeta y tocá Analizar"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Sin analizar"), "{texts:?}");
        assert!(!texts.iter().any(|t| t.starts_with("Escaneando")), "{texts:?}");
        poke(&mut x, area);
    }

    /// F5 en una vista sin analizar analiza lo elegido en el desplegable (como «Analizar»).
    #[test]
    fn f5_unscanned_scans_the_pick() {
        crate::sys::testing::install_fakes();
        let root = std::env::temp_dir().join("xdir-xtree-f5-raiz");
        let other = std::env::temp_dir().join("xdir-xtree-f5-elegida");
        let mut x = Xtree::unscanned(Handle::default(), root, false);
        x.pick = other.clone();
        assert_eq!(x.on_key(VK_F5), Action::Redraw);
        assert!(x.scanned());
        assert_eq!(x.root(), other.as_path());
        x.cancel();
    }

    #[test]
    fn paint_with_empty_tree_does_not_panic() {
        let area = rect(0.0, 0.0, 1400.0, 790.0);
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        poke(&mut x, area);
        // Un solo nodo de tamaño 0 (carpeta vacía) y una carpeta con un archivo vacío.
        x.set_arena(vec![node("raiz", NO_PARENT, 0, true, vec![])]);
        poke(&mut x, area);
        x.set_arena(vec![node("raiz", NO_PARENT, 0, true, vec![1]), node("vacio.txt", 0, 0, false, vec![])]);
        poke(&mut x, area);
        // Área demasiado chica para el mapa.
        poke(&mut x, rect(0.0, 0.0, 10.0, 10.0));
    }

    /// Un árbol nuevo (resultado elevado, escaneo rápido) reemplaza al anterior: los bloques viejos
    /// no pueden seleccionar nodos que ya no existen.
    #[test]
    fn click_after_tree_swap_does_not_panic() {
        let area = rect(0.0, 0.0, 1400.0, 790.0);
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        let mut big = vec![node("raiz", NO_PARENT, 300, true, (1..4).collect())];
        big.push(node("a", 0, 100, true, vec![4, 5]));
        big.push(node("b.bin", 0, 100, false, vec![]));
        big.push(node("c.bin", 0, 50, false, vec![]));
        big.push(node("d.bin", 1, 60, false, vec![]));
        big.push(node("e.bin", 1, 40, false, vec![]));
        x.set_arena(big);
        x.prepare(area);
        // Un bloque de un nodo que no existe en el árbol chico.
        let c = x.cells.iter().find(|c| c.node >= 2).expect("bloque");
        let (cx, cy) = ((c.r.left + c.r.right) / 2.0, (c.r.top + c.r.bottom) / 2.0);
        // Mismo camino que el aviso del escáner (`WM_APP_XTREE`, lparam 1).
        x.scanning = true;
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz", NO_PARENT, 10, true, vec![1]), node("f.bin", 0, 10, false, vec![])]);
        assert!(x.take_result());
        let _ = x.on_click(area, cx, cy, false);
        assert!(x.selected.is_none_or(|n| (n as usize) < x.arena.len()), "selección fuera del árbol");
        let ui = layout_ui(area, x.legend_h());
        let b = ui.buttons[2];
        let _ = x.on_click(area, (b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0, false);
        x.prepare(area);
    }

    /// Espera a que termine el hilo de la tecla A.
    fn wait_elevated(x: &Xtree) {
        let t0 = std::time::Instant::now();
        while x.elevating.load(Ordering::Relaxed) {
            assert!(t0.elapsed() < std::time::Duration::from_secs(5), "el hilo elevado no terminó");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn lacking_permissions(x: &Xtree) {
        x.scan.progress.need_admin.store(true, Ordering::Relaxed);
        x.scan.progress.denied.store(1, Ordering::Relaxed);
    }

    /// Tecla A: el árbol elevado le gana al del escaneo normal aunque este termine después.
    #[test]
    fn elevated_tree_wins_over_later_normal_result() {
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        x.scanning = true;
        lacking_permissions(&x);
        let elevated = || vec![node("raiz", NO_PARENT, 99, true, vec![1]), node("admin.bin", 0, 99, false, vec![])];
        assert_eq!(x.elevate_with(move |_, _, _| Some(elevated())), Action::Redraw);
        wait_elevated(&x);
        // El escaneo normal termina después: su árbol no debe pisar al elevado.
        let normal = vec![node("raiz", NO_PARENT, 5, true, vec![])];
        assert!(!scan::deliver(&x.scan.result, &x.scan.cancel, normal, false));
        assert!(x.take_result());
        assert_eq!(x.arena[0].size, 99);
        // Ya no faltan permisos: sin aviso y la tecla A no vuelve a lanzar nada.
        assert!(!disk_scan().permission_issue(&x.scan.progress));
        assert_eq!(x.data_hint(), "");
        assert_eq!(x.elevate_with(|_, _, _| panic!("no debe relanzar")), Action::None);
    }

    /// Mientras corre el escaneo elevado, la barra y la barra de estado lo dicen; si falla (UAC
    /// rechazado), vuelve el aviso de permisos y la vista se repinta.
    #[test]
    fn elevating_is_shown_and_failure_restores_hint() {
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        x.set_arena(vec![node("raiz", NO_PARENT, 5, true, vec![])]);
        lacking_permissions(&x);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert_eq!(x.elevate_with(move |_, _, _| { let _ = rx.recv(); None }), Action::Redraw);
        assert_eq!(x.data_hint(), disk_scan().elevating_text());
        assert!(x.status_text().contains(disk_scan().elevating_text()));
        tx.send(()).unwrap();
        wait_elevated(&x);
        assert!(disk_scan().permission_issue(&x.scan.progress));
        assert!(x.data_hint().contains("(tecla A)"));
        assert!(!x.status_text().contains(disk_scan().elevating_text()));
        // El aviso del hilo elevado (sin árbol) repinta aunque el escaneo normal ya haya terminado.
        assert!(x.on_message(x.scan.id, 0));
    }

    /// Si el escaneo normal terminó antes, el elevado igual lo reemplaza.
    #[test]
    fn elevated_tree_replaces_earlier_normal_result() {
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        x.scanning = true;
        lacking_permissions(&x);
        assert!(scan::deliver(&x.scan.result, &x.scan.cancel, vec![node("raiz", NO_PARENT, 5, true, vec![])], false));
        assert_eq!(x.elevate_with(|_, _, _| Some(vec![node("raiz", NO_PARENT, 99, true, vec![])])), Action::Redraw);
        wait_elevated(&x);
        assert!(x.take_result());
        assert_eq!(x.arena[0].size, 99);
    }

    #[test]
    fn escape_closes_menu_then_view() {
        let mut x = Xtree::idle(PathBuf::from("raiz"));
        x.menu_open = true;
        assert_eq!(x.on_key(VK_ESCAPE), Action::Redraw);
        assert!(!x.menu_open);
        assert_eq!(x.on_key(VK_ESCAPE), Action::Close);
    }

    // ---------- cronómetro y porcentaje ----------

    #[test]
    fn fmt_elapsed_formats() {
        let s = Duration::from_secs;
        assert_eq!(fmt_elapsed(s(0)), "0:00");
        assert_eq!(fmt_elapsed(s(42)), "0:42");
        assert_eq!(fmt_elapsed(s(83)), "1:23");
        assert_eq!(fmt_elapsed(s(3735)), "1:02:15");
        assert_eq!(fmt_elapsed(Duration::from_millis(42_900)), "0:42");
    }

    #[test]
    fn scan_percent_caps_and_handles_zero() {
        assert_eq!(scan_percent(50, 100), Some(50));
        assert_eq!(scan_percent(150, 100), Some(99));
        assert_eq!(scan_percent(100, 100), Some(99));
        assert_eq!(scan_percent(1, 0), None);
        assert_eq!(scan_percent(u64::MAX, u64::MAX / 2), Some(99));
    }

    /// Vista escaneando `root` desde hace `secs` segundos (sin escáner real).
    fn scanning_since(root: &str, secs: u64) -> Xtree {
        let mut x = Xtree::idle(PathBuf::from(root));
        x.scanning = true;
        x.start_clock();
        x.scan_started = Instant::now().checked_sub(Duration::from_secs(secs));
        x
    }

    /// Texto de estado que se dibujó en la barra superior.
    fn painted(x: &mut Xtree) -> Vec<String> {
        let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        x.paint(&p, rect(0.0, 0.0, 1400.0, 790.0));
        p.texts().into_iter().map(|(_, s, _)| s).collect()
    }

    #[test]
    fn status_shows_elapsed_while_scanning() {
        let mut x = scanning_since("raiz-crono", 42);
        assert!(x.status_text().contains("Escaneando\u{2026} 0:42"), "{}", x.status_text());
        assert!(painted(&mut x).iter().any(|t| t == "Escaneando\u{2026} 0:42"));
    }

    #[test]
    fn status_shows_total_when_done() {
        let mut x = scanning_since("raiz-total", 83);
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz-total", NO_PARENT, 5, true, vec![])]);
        assert!(x.take_result());
        assert!(x.status_text().contains("Análisis completado en 1:23"), "{}", x.status_text());
        assert!(painted(&mut x).iter().any(|t| t == "Análisis completado en 1:23"));
        // El tiempo total queda fijo.
        x.scan_started = Instant::now().checked_sub(Duration::from_secs(500));
        assert!(x.status_text().contains("Análisis completado en 1:23"));
    }

    #[test]
    fn percent_only_for_whole_volume() {
        // Volumen entero: 400 de 1000 usados; leídos 148 → 37 %.
        crate::sys::testing::install_fakes();
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 600)), true);
        let mut x = scanning_since("disco", 42);
        x.scan.progress.bytes.store(148, Ordering::Relaxed);
        assert!(x.status_text().contains("Escaneando\u{2026} 37 % \u{B7} 0:42"), "{}", x.status_text());
        assert!(painted(&mut x).iter().any(|t| t == "Escaneando\u{2026} 37 % \u{B7} 0:42"));
        // Más leído que usado (APFS, hardlinks): tope 99 %; y no retrocede.
        x.scan.progress.bytes.store(5000, Ordering::Relaxed);
        assert!(x.status_text().contains(" 99 % \u{B7} "), "{}", x.status_text());
        x.scan.progress.bytes.store(10, Ordering::Relaxed);
        assert!(x.status_text().contains(" 99 % \u{B7} "), "{}", x.status_text());
        // Volumen con espacio usado 0: sin porcentaje.
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 1000)), true);
        let x = scanning_since("disco-vacio", 42);
        assert!(!x.status_text().contains(" % \u{B7} "), "{}", x.status_text());
        // Una carpeta (no es un volumen entero): solo el cronómetro.
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 600)), false);
        let x = scanning_since("carpeta", 42);
        x.scan.progress.bytes.store(148, Ordering::Relaxed);
        assert!(!x.status_text().contains(" % \u{B7} "), "{}", x.status_text());
        assert!(x.status_text().contains("Escaneando\u{2026} 0:42"), "{}", x.status_text());
        crate::sys::testing::FakeDiskScan::set_volume(None, false);
    }

    #[test]
    fn restart_resets_timer() {
        let mut x = scanning_since("raiz-a", 42);
        x.cancel();
        x.pick = PathBuf::from("raiz-b");
        x.scan_pick();
        let status = x.status_text();
        assert!(status.contains("Escaneando\u{2026} 0:00"), "{status}");
        assert!(!status.contains("Detenido"), "{status}");
        x.cancel();
    }

    /// Tecla A con el análisis terminado: escaneo nuevo con el cronómetro desde 0; al llegar el
    /// árbol elevado, «Análisis completado en» con la duración nueva.
    #[test]
    fn elevate_after_done_restarts_clock() {
        let mut x = scanning_since("raiz-elev", 83);
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz-elev", NO_PARENT, 5, true, vec![])]);
        assert!(x.take_result());
        assert!(x.state_text().contains("Análisis completado en 1:23"));
        lacking_permissions(&x);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let elevated = vec![node("raiz-elev", NO_PARENT, 99, true, vec![])];
        assert_eq!(x.elevate_with(move |_, _, _| { let _ = rx.recv(); Some(elevated) }), Action::Redraw);
        assert_eq!(x.state_text(), "Escaneando\u{2026} 0:00");
        assert!(x.status_text().contains("Escaneando\u{2026} 0:00"), "{}", x.status_text());
        assert!(painted(&mut x).iter().any(|t| t == "Escaneando\u{2026} 0:00"));
        // Simula 7 s de escaneo elevado.
        x.scan_started = Instant::now().checked_sub(Duration::from_secs(7));
        tx.send(()).unwrap();
        wait_elevated(&x);
        assert!(x.on_message(x.scan.id, 1));
        assert_eq!(x.arena[0].size, 99);
        assert_eq!(x.state_text(), "Análisis completado en 0:07");
    }

    /// Un aviso del hilo de repintado que llega después de que el escaneo elevado dejó su árbol
    /// (pero antes del aviso de entrega) no devuelve el tiempo anterior.
    #[test]
    fn late_tick_does_not_restore_old_time() {
        let mut x = scanning_since("raiz-elev-tick", 83);
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz-elev-tick", NO_PARENT, 5, true, vec![])]);
        assert!(x.take_result());
        lacking_permissions(&x);
        let elevated = vec![node("raiz-elev-tick", NO_PARENT, 99, true, vec![])];
        assert_eq!(x.elevate_with(move |_, _, _| Some(elevated)), Action::Redraw);
        x.scan_started = Instant::now().checked_sub(Duration::from_secs(7));
        wait_elevated(&x);
        assert!(x.scan.result.lock().unwrap().is_some());
        assert!(x.on_message(x.scan.id, 0));
        assert_eq!(x.state_text(), "Escaneando\u{2026} 0:07");
        assert!(x.on_message(x.scan.id, 1));
        assert_eq!(x.state_text(), "Análisis completado en 0:07");
    }

    /// Si el reescaneo con permisos falla, vuelve el tiempo del análisis anterior.
    #[test]
    fn elevate_failure_restores_previous_time() {
        let mut x = scanning_since("raiz-elev-falla", 83);
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz-elev-falla", NO_PARENT, 5, true, vec![])]);
        assert!(x.take_result());
        lacking_permissions(&x);
        assert_eq!(x.elevate_with(|_, _, _| None), Action::Redraw);
        wait_elevated(&x);
        assert!(x.on_message(x.scan.id, 0));
        assert_eq!(x.state_text(), "Análisis completado en 1:23");
    }

    /// El estado más largo entra en su lugar de la barra superior.
    #[test]
    fn longest_state_fits() {
        let p = crate::sys::testing::RecordingPainter::new(crate::theme::Theme::from_system(crate::theme::DEFAULT_ACCENT, Some(true)));
        assert!(p.measure(FontId::Ui, "Análisis completado en 1:02:15") <= STATE_W - MARGIN);
        assert!(p.measure(FontId::Ui, "Escaneando\u{2026} 99 % \u{B7} 1:02:15") <= STATE_W - MARGIN);
    }

    #[test]
    fn cancel_says_stopped() {
        let mut x = scanning_since("raiz-corte", 37);
        x.cancel();
        assert!(x.status_text().contains("Detenido a los 0:37"), "{}", x.status_text());
        // El tiempo queda fijo al cortarse.
        x.scan_started = Instant::now().checked_sub(Duration::from_secs(500));
        assert!(x.status_text().contains("Detenido a los 0:37"));
        assert!(painted(&mut x).iter().any(|t| t == "Detenido a los 0:37"));
        // Cancelar (al cerrar xdir) un análisis ya terminado no lo marca como detenido.
        let mut x = scanning_since("raiz-hecha", 83);
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz-hecha", NO_PARENT, 5, true, vec![])]);
        assert!(x.take_result());
        x.cancel();
        assert!(x.status_text().contains("Análisis completado en 1:23"), "{}", x.status_text());
    }

    // ---------- indicador de la barra de estado ----------

    /// Termina el escaneo de `x` con un árbol de un nodo (mismo camino que el aviso de entrega).
    fn finish(x: &mut Xtree) {
        let name = x.root.display().to_string();
        *x.scan.result.lock().unwrap() = Some(vec![node(&name, NO_PARENT, 5, true, vec![])]);
        assert!(x.on_message(x.scan.id, 1));
    }

    #[test]
    fn badge_progress_while_scanning_volume() {
        crate::sys::testing::install_fakes();
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 0)), true);
        let x = scanning_since("disco-indicador", 42);
        x.scan.progress.bytes.store(370, Ordering::Relaxed);
        assert_eq!(x.badge(), XtreeBadge::Progress(37));
        crate::sys::testing::FakeDiskScan::set_volume(None, false);
    }

    #[test]
    fn badge_spins_for_folder_scan() {
        crate::sys::testing::install_fakes();
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 0)), false);
        let mut x = scanning_since("carpeta-indicador", 12);
        x.scan.progress.bytes.store(370, Ordering::Relaxed);
        let XtreeBadge::Spinning(a) = x.badge() else { panic!("{:?}", x.badge()) };
        // Cada aviso del escaneo avanza el arco.
        assert!(x.on_message(x.scan.id, 0));
        let XtreeBadge::Spinning(b) = x.badge() else { panic!("{:?}", x.badge()) };
        assert_ne!(a, b);
        // Un aviso de otro escaneo no lo mueve.
        assert!(!x.on_message(x.scan.id + 1000, 0));
        assert_eq!(x.badge(), XtreeBadge::Spinning(b));
        crate::sys::testing::FakeDiskScan::set_volume(None, false);
    }

    /// Reescaneo con permisos (tecla A) después de terminar: giro, no porcentaje.
    #[test]
    fn badge_spins_while_elevating() {
        crate::sys::testing::install_fakes();
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 0)), true);
        let mut x = scanning_since("disco-elevado", 83);
        finish(&mut x);
        lacking_permissions(&x);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert_eq!(x.elevate_with(move |_, _, _| { let _ = rx.recv(); None }), Action::Redraw);
        assert!(matches!(x.badge(), XtreeBadge::Spinning(_)), "{:?}", x.badge());
        tx.send(()).unwrap();
        wait_elevated(&x);
        crate::sys::testing::FakeDiskScan::set_volume(None, false);
    }

    #[test]
    fn badge_done_until_view_opened() {
        let mut x = scanning_since("raiz-indicador", 83);
        // Ver la vista mientras escanea no cuenta: el resultado todavía no se vio.
        x.mark_seen();
        finish(&mut x);
        assert_eq!(x.badge(), XtreeBadge::Done);
        assert_eq!(x.badge_tip().as_deref(), Some("Análisis completado en 1:23"));
        x.mark_seen();
        assert_eq!(x.badge(), XtreeBadge::Idle);
        assert_eq!(x.badge_tip(), None);
        // Un escaneo nuevo vuelve a mostrar su avance.
        x.rescan();
        assert!(matches!(x.badge(), XtreeBadge::Spinning(_)), "{:?}", x.badge());
        x.cancel();
    }

    #[test]
    fn badge_done_with_issues_when_denied() {
        let mut x = scanning_since("raiz-sin-permiso", 83);
        x.scan.progress.denied.store(458, Ordering::Relaxed);
        finish(&mut x);
        assert_eq!(x.badge(), XtreeBadge::DoneWithIssues);
        assert_eq!(x.badge_tip().as_deref(), Some("Análisis completado en 1:23 \u{B7} 458 carpetas sin permiso"));
        x.scan.progress.denied.store(1, Ordering::Relaxed);
        assert_eq!(x.badge_tip().as_deref(), Some("Análisis completado en 1:23 \u{B7} 1 carpeta sin permiso"));
        x.mark_seen();
        assert_eq!(x.badge(), XtreeBadge::Idle);
    }

    #[test]
    fn badge_idle_after_cancel() {
        let mut x = scanning_since("raiz-cancelada", 37);
        x.cancel();
        assert_eq!(x.badge(), XtreeBadge::Idle);
        assert_eq!(x.badge_tip(), None);
        // El árbol de un escaneo cancelado que llega tarde no marca «terminado».
        *x.scan.result.lock().unwrap() = Some(vec![node("raiz-cancelada", NO_PARENT, 5, true, vec![])]);
        let _ = x.on_message(x.scan.id, 1);
        assert_eq!(x.badge(), XtreeBadge::Idle);
        // Escaneo nuevo encima de uno en curso: el viejo no deja nada pegado.
        let mut x = scanning_since("raiz-a-indicador", 20);
        x.pick = PathBuf::from("raiz-b-indicador");
        x.scan_pick();
        assert!(matches!(x.badge(), XtreeBadge::Spinning(_)), "{:?}", x.badge());
        x.cancel();
        assert_eq!(x.badge(), XtreeBadge::Idle);
    }

    #[test]
    fn badge_tip_texts() {
        crate::sys::testing::install_fakes();
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 0)), true);
        let x = scanning_since("/", 42);
        x.scan.progress.bytes.store(370, Ordering::Relaxed);
        assert_eq!(x.badge_tip().as_deref(), Some("Escaneando / \u{B7} 37 % \u{B7} 0:42"));
        crate::sys::testing::FakeDiskScan::set_volume(None, false);
        let x = scanning_since("~/Proyectos", 12);
        assert_eq!(x.badge_tip().as_deref(), Some("Escaneando ~/Proyectos \u{B7} 0:12"));
        let mut x = scanning_since("raiz-tip", 83);
        finish(&mut x);
        assert_eq!(x.badge_tip().as_deref(), Some("Análisis completado en 1:23"));
        x.scan.progress.denied.store(458, Ordering::Relaxed);
        assert_eq!(x.badge_tip().as_deref(), Some("Análisis completado en 1:23 \u{B7} 458 carpetas sin permiso"));
    }

    /// Si el reescaneo con permisos falla, un resultado que ya se había visto no vuelve a
    /// aparecer como «terminado»; mientras dura, el indicador gira.
    #[test]
    fn badge_stays_seen_after_refused_elevation() {
        let mut x = scanning_since("raiz-elev-visto", 83);
        finish(&mut x);
        x.mark_seen();
        lacking_permissions(&x);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert_eq!(x.elevate_with(move |_, _, _| { let _ = rx.recv(); None }), Action::Redraw);
        assert!(matches!(x.badge(), XtreeBadge::Spinning(_)), "{:?}", x.badge());
        tx.send(()).unwrap();
        wait_elevated(&x);
        assert!(x.on_message(x.scan.id, 0));
        assert_eq!(x.state_text(), "Análisis completado en 1:23");
        assert_eq!(x.badge(), XtreeBadge::Idle);
    }

    /// Reescaneo con permisos durante el escaneo de un volumen entero: el estado de la vista,
    /// como el indicador, sin porcentaje.
    #[test]
    fn elevating_during_volume_scan_hides_percent() {
        crate::sys::testing::install_fakes();
        crate::sys::testing::FakeDiskScan::set_volume(Some((1000, 0)), true);
        let mut x = scanning_since("disco-elevando", 42);
        x.scan.progress.bytes.store(370, Ordering::Relaxed);
        assert_eq!(x.state_text(), "Escaneando\u{2026} 37 % \u{B7} 0:42");
        lacking_permissions(&x);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        assert_eq!(x.elevate_with(move |_, _, _| { let _ = rx.recv(); None }), Action::Redraw);
        assert_eq!(x.state_text(), "Escaneando\u{2026} 0:42");
        assert!(matches!(x.badge(), XtreeBadge::Spinning(_)), "{:?}", x.badge());
        tx.send(()).unwrap();
        wait_elevated(&x);
        x.cancel();
        crate::sys::testing::FakeDiskScan::set_volume(None, false);
    }
}

//! Lo que corre en segundo plano (conexiones, expulsar, tamaños, sumas, búsquedas, lecturas que
//! tardan, xtree, extracciones): el indicador de la barra de estado y su cartel. Nada de esto mira
//! el disco: lo alimentan los lugares que lanzan los trabajos (`begin`/`end`) y, antes de cada
//! cuadro, lo que ya dice el estado de la `App` (`App::sync_activity`).
//!
//! Un trabajo nuevo se suma con una llamada al empezar y otra al terminar:
//! `app.activity.begin(ActivityKey::Hash(id), "Calculando la suma de \"x\"", app.now())` y
//! `app.activity.end(&ActivityKey::Hash(id))`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;

/// Toca redibujar el indicador (gira, o algo pasó el umbral).
pub(super) const WM_APP_ACTIVITY: u32 = WM_APP + 36;
/// Lo que dura menos que esto no se muestra (sin parpadeo).
pub(super) const SHOW_AFTER: Duration = Duration::from_millis(300);
/// Cada cuánto se redibuja mientras haya algo en curso (el indicador avanza 10 pasos por segundo).
pub(super) const FRAME: Duration = Duration::from_millis(100);

/// De qué es cada cosa en curso (una por clave).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ActivityKey {
    /// Una conexión (su generación, `NetState::generation`).
    Connect(u64),
    /// Expulsar un disco o desconectar una conexión (su raíz).
    Eject(PathBuf),
    /// El tamaño de lo elegido.
    SelSize,
    /// Sumas de verificación y comparaciones (las suma la rama de sumas: un id propio de cada una).
    #[allow(dead_code)]
    Hash(u64),
    #[allow(dead_code)]
    Compare(u64),
    /// Una búsqueda en subcarpetas (id de la pestaña).
    Search(usize),
    /// La lectura de una carpeta (id de la pestaña); se ve desde `tab::LOADING_DELAY`.
    Load(usize),
    /// El análisis de xtree con la vista cerrada.
    XtreeScan,
    /// Extraer a la temporal un archivo de adentro de un comprimido para abrirlo.
    Extract,
}

/// Una cosa en curso.
#[derive(Clone, Debug)]
pub(super) struct Activity {
    pub key: ActivityKey,
    /// Lo que dice el cartel («Expulsando "Pendrive"»).
    pub text: String,
    pub since: Instant,
    /// Desde cuándo se muestra (`SHOW_AFTER`, o más).
    pub delay: Duration,
    /// Sale del estado de la `App` (`set_derived`), no de `begin`.
    derived: bool,
}

impl Activity {
    fn shown(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= self.delay
    }
}

/// Lo que sale del estado en un cuadro: clave, texto, desde cuándo (`None`: desde que se vio por
/// primera vez) y el umbral.
pub(super) type Derived = (ActivityKey, String, Option<Instant>, Duration);

#[derive(Default)]
pub(super) struct Activities {
    items: Vec<Activity>,
    /// Hay un redibujo pedido para este momento (`WM_APP_ACTIVITY`).
    pub tick_due: Option<Instant>,
}

impl Activities {
    /// Empezó algo (o cambió su texto: con la misma clave se queda el inicio).
    pub fn begin(&mut self, key: ActivityKey, text: impl Into<String>, now: Instant) {
        self.put(key, text.into(), Some(now), SHOW_AFTER, false, now);
    }

    /// Terminó (bien, mal o cancelado).
    pub fn end(&mut self, key: &ActivityKey) {
        self.items.retain(|a| &a.key != key);
    }

    fn put(&mut self, key: ActivityKey, text: String, since: Option<Instant>, delay: Duration, derived: bool, now: Instant) {
        match self.items.iter_mut().find(|a| a.key == key) {
            Some(a) => {
                a.text = text;
                a.delay = delay;
                a.derived = derived;
                if let Some(s) = since.filter(|_| derived) {
                    a.since = s;
                }
            }
            None => self.items.push(Activity { key, text, since: since.unwrap_or(now), delay, derived }),
        }
    }

    /// Lo que sale del estado en este cuadro: reemplaza lo derivado del cuadro anterior (lo que
    /// ya no está, terminó); lo de `begin` queda.
    pub fn set_derived(&mut self, list: Vec<Derived>, now: Instant) {
        self.items.retain(|a| !a.derived || list.iter().any(|(k, ..)| *k == a.key));
        for (key, text, since, delay) in list {
            self.put(key, text, since, delay, true, now);
        }
    }

    #[cfg(test)]
    pub fn contains(&self, key: &ActivityKey) -> bool {
        self.items.iter().any(|a| &a.key == key)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Lo que se muestra ahora (pasó su umbral), del más viejo al más nuevo.
    pub fn shown(&self, now: Instant) -> Vec<&Activity> {
        let mut v: Vec<&Activity> = self.items.iter().filter(|a| a.shown(now)).collect();
        v.sort_by_key(|a| a.since);
        v
    }

    /// El cartel del indicador: «Expulsando "Pendrive" · Calculando el tamaño de 3 carpetas».
    pub fn tip(&self, now: Instant) -> Option<String> {
        // Lo repetido (dos paneles leyendo «Este equipo») va una vez.
        let mut texts: Vec<&str> = Vec::new();
        for a in self.shown(now) {
            if !texts.contains(&a.text.as_str()) {
                texts.push(&a.text);
            }
        }
        (!texts.is_empty()).then(|| texts.join(" · "))
    }

    /// Cuándo hace falta el próximo redibujo: mientras haya algo en curso, `FRAME` (el indicador
    /// gira y lo que espera su umbral aparece a tiempo); sin nada, ninguno.
    pub fn next_tick(&self) -> Option<Duration> {
        (!self.items.is_empty()).then_some(FRAME)
    }
}

/// «Calculando el tamaño de 3 carpetas».
pub(super) fn sel_size_text(folders: usize) -> String {
    if folders == 1 { "Calculando el tamaño de 1 carpeta".into() } else { format!("Calculando el tamaño de {folders} carpetas") }
}

impl App {
    /// El reloj de lo que se muestra (las pruebas lo fijan).
    pub(super) fn now(&self) -> Instant {
        #[cfg(test)]
        if let Some(t) = self.clock {
            return t;
        }
        Instant::now()
    }

    /// Antes de dibujar: lo que dice el estado de la `App` pasa al registro.
    pub(super) fn sync_activity(&mut self, now: Instant) {
        let mut list: Vec<Derived> = Vec::new();
        for (root, text, since) in self.ejecting_now() {
            list.push((ActivityKey::Eject(root), text.trim_end_matches('…').to_string(), Some(since), SHOW_AFTER));
        }
        if self.sel_size.total() == crate::tab::SelTotal::Computing {
            list.push((ActivityKey::SelSize, sel_size_text(self.sel_size.folders()), None, SHOW_AFTER));
        }
        for tab in self.panes.iter().flat_map(|p| p.tabs.iter()) {
            if let Some(s) = tab.search.as_ref().filter(|s| !s.done && !s.in_place) {
                list.push((ActivityKey::Search(tab.id), format!("Buscando \"{}\"", s.query), Some(s.started), SHOW_AFTER));
            } else if let Some(t0) = tab.loading_since().filter(|_| tab.is_visible()) {
                list.push((ActivityKey::Load(tab.id), format!("Cargando \"{}\"", tab.location.title()), Some(t0), crate::tab::LOADING_DELAY));
            }
        }
        if !self.xtree_open
            && let Some(x) = &self.xtree
            && matches!(x.badge(), ui::XtreeBadge::Progress(_) | ui::XtreeBadge::Spinning(_))
        {
            let root = x.root();
            let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
            list.push((ActivityKey::XtreeScan, format!("Analizando \"{name}\" en xtree"), None, SHOW_AFTER));
        }
        self.activity.set_derived(list, now);
    }

    /// Después de dibujar: el próximo redibujo, solo mientras haya algo en curso.
    pub(super) fn schedule_activity_tick(&mut self) {
        let Some(wait) = self.activity.next_tick() else { return };
        let now = Instant::now();
        if self.activity.tick_due.is_some_and(|d| d > now) {
            return;
        }
        self.activity.tick_due = Some(now + wait);
        let hwnd = self.hwnd;
        std::thread::spawn(move || {
            std::thread::sleep(wait);
            hwnd.post(WM_APP_ACTIVITY, 0, 0);
        });
    }

    /// `WM_APP_ACTIVITY`: otro cuadro.
    pub(super) fn on_activity_tick(&mut self) {
        self.activity.tick_due = None;
        if !self.activity.is_empty() {
            self.redraw();
        }
    }

    /// Clic en el indicador: con copias, su ventana; sin, nada.
    pub(super) fn activity_click(&mut self) {
        if self.transfers_progress().is_some() {
            self.show_transfers();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn hidden_until_threshold() {
        let t0 = Instant::now();
        let mut a = Activities::default();
        a.begin(ActivityKey::Connect(1), "Conectando con \"nas\"", t0);
        assert!(a.shown(t0 + ms(299)).is_empty());
        assert_eq!(a.tip(t0 + ms(299)), None);
        assert_eq!(a.shown(t0 + ms(300)).len(), 1);
        assert_eq!(a.tip(t0 + ms(300)).as_deref(), Some("Conectando con \"nas\""));
        // Las lecturas de carpetas, desde su propio umbral.
        a.set_derived(vec![(ActivityKey::Load(3), "Cargando \"x\"".into(), Some(t0), crate::tab::LOADING_DELAY)], t0);
        assert_eq!(a.shown(t0 + ms(400)).len(), 1);
        assert_eq!(a.shown(t0 + ms(500)).len(), 2);
    }

    #[test]
    fn begin_again_keeps_start_and_updates_text() {
        let t0 = Instant::now();
        let mut a = Activities::default();
        a.begin(ActivityKey::Extract, "Extrayendo \"a.txt\"", t0);
        a.begin(ActivityKey::Extract, "Extrayendo \"b.txt\"", t0 + ms(200));
        assert_eq!(a.tip(t0 + ms(300)).as_deref(), Some("Extrayendo \"b.txt\""));
        a.end(&ActivityKey::Extract);
        assert!(a.is_empty());
        // Lo repetido, una vez.
        let load = |id| (ActivityKey::Load(id), "Cargando \"Este equipo\"".to_string(), Some(t0), SHOW_AFTER);
        a.set_derived(vec![load(1), load(2)], t0);
        assert_eq!(a.tip(t0 + ms(300)).as_deref(), Some("Cargando \"Este equipo\""));
    }

    #[test]
    fn derived_replaced_each_frame_explicit_stays() {
        let t0 = Instant::now();
        let mut a = Activities::default();
        a.begin(ActivityKey::Connect(7), "Conectando con \"nas\"", t0);
        a.set_derived(vec![(ActivityKey::SelSize, sel_size_text(3), None, SHOW_AFTER)], t0 + ms(100));
        // Sigue en el cuadro siguiente: conserva desde cuándo.
        a.set_derived(vec![(ActivityKey::SelSize, sel_size_text(3), None, SHOW_AFTER)], t0 + ms(350));
        assert_eq!(a.tip(t0 + ms(400)).as_deref(), Some("Conectando con \"nas\" · Calculando el tamaño de 3 carpetas"));
        a.set_derived(Vec::new(), t0 + ms(450));
        assert!(a.contains(&ActivityKey::Connect(7)));
        assert!(!a.contains(&ActivityKey::SelSize));
        assert_eq!(sel_size_text(1), "Calculando el tamaño de 1 carpeta");
    }

    #[test]
    fn ticks_only_while_active() {
        let t0 = Instant::now();
        let mut a = Activities::default();
        assert_eq!(a.next_tick(), None);
        a.begin(ActivityKey::SelSize, "x", t0);
        assert_eq!(a.next_tick(), Some(FRAME));
        a.end(&ActivityKey::SelSize);
        assert_eq!(a.next_tick(), None);
    }
}

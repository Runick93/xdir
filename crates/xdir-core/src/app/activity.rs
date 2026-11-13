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
/// Lo que llegó a verse queda al menos esto (aunque termine antes): sin destellos de un cuadro.
pub(super) const MIN_SHOWN: Duration = Duration::from_millis(400);
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
    /// Sumas de verificación y comparaciones (el número de su trabajo, `compare::Job::id`).
    Hash(u64),
    Compare(u64),
    /// Una búsqueda en subcarpetas (id de la pestaña).
    Search(usize),
    /// La lectura de una carpeta (id de la pestaña); se ve desde `tab::LOADING_DELAY`.
    Load(usize),
    /// Una carpeta de red que «no responde» (id de la pestaña): quieta, no pide redibujos.
    Stalled(usize),
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
    /// Cuándo se vio por primera vez en el indicador (`observe`).
    seen_at: Option<Instant>,
    /// Ya terminó; sigue a la vista hasta cumplir `MIN_SHOWN` desde `seen_at`.
    ended: bool,
}

impl Activity {
    fn shown(&self, now: Instant) -> bool {
        self.seen_at.is_some() || now.saturating_duration_since(self.since) >= self.delay
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
        Self::finish(&mut self.items, |a| &a.key == key);
    }

    /// Termina los que cumplen `done`: lo que no llegó a verse sale ya; lo que se vio queda
    /// marcado hasta cumplir `MIN_SHOWN` (lo saca `observe`).
    fn finish(items: &mut Vec<Activity>, done: impl Fn(&Activity) -> bool) {
        items.retain_mut(|a| {
            if !done(a) {
                return true;
            }
            a.ended = true;
            a.seen_at.is_some()
        });
    }

    /// Después de cada cuadro: anota lo que se empezó a ver y saca lo terminado que ya se vio el
    /// mínimo.
    pub fn observe(&mut self, now: Instant) {
        for a in &mut self.items {
            if a.seen_at.is_none() && a.shown(now) {
                a.seen_at = Some(now);
            }
        }
        self.items.retain(|a| !a.ended || a.seen_at.is_some_and(|s| now.saturating_duration_since(s) < MIN_SHOWN));
    }

    fn put(&mut self, key: ActivityKey, text: String, since: Option<Instant>, delay: Duration, derived: bool, now: Instant) {
        match self.items.iter_mut().find(|a| a.key == key) {
            Some(a) => {
                a.ended = false;
                a.text = text;
                a.delay = delay;
                a.derived = derived;
                if let Some(s) = since.filter(|_| derived) {
                    a.since = s;
                }
            }
            None => self.items.push(Activity { key, text, since: since.unwrap_or(now), delay, derived, seen_at: None, ended: false }),
        }
    }

    /// Lo que sale del estado en este cuadro: reemplaza lo derivado del cuadro anterior (lo que
    /// ya no está, terminó); lo de `begin` queda.
    pub fn set_derived(&mut self, list: Vec<Derived>, now: Instant) {
        Self::finish(&mut self.items, |a| a.derived && !list.iter().any(|(k, ..)| *k == a.key));
        for (key, text, since, delay) in list {
            self.put(key, text, since, delay, true, now);
        }
    }

    #[cfg(test)]
    pub fn contains(&self, key: &ActivityKey) -> bool {
        self.items.iter().any(|a| &a.key == key && !a.ended)
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

    /// ¿Todo lo que se ve está quieto (solo carpetas que «no responden»)? El indicador muestra el
    /// aviso en lugar de un giro congelado.
    pub fn all_still(&self, now: Instant) -> bool {
        let shown = self.shown(now);
        !shown.is_empty() && shown.iter().all(|a| matches!(a.key, ActivityKey::Stalled(_)))
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

    /// Cuándo hace falta el próximo redibujo: con algo a la vista (o un ⏏ girando), `FRAME`; si
    /// no, cuando lo próximo pase su umbral. Las lecturas de carpetas antes de los 0,5 s no piden
    /// nada (el «Cargando…» lo anuncia `update_placeholders`): una lectura rápida no prende el
    /// temporizador. Sin nada en curso, ninguno.
    pub fn next_tick(&self, now: Instant) -> Option<Duration> {
        let still = |a: &Activity| matches!(a.key, ActivityKey::Stalled(_));
        let spinning = |a: &Activity| !still(a) && (a.shown(now) || matches!(a.key, ActivityKey::Eject(_)));
        if self.items.iter().any(spinning) {
            return Some(FRAME);
        }
        self.items
            .iter()
            .filter(|a| !matches!(a.key, ActivityKey::Load(_) | ActivityKey::Stalled(_)))
            .map(|a| (a.since + a.delay).saturating_duration_since(now))
            .min()
    }
}

/// «Calculando el tamaño de 3 carpetas».
pub(super) fn sel_size_text(folders: usize) -> String {
    if folders == 1 { "Calculando el tamaño de 1 carpeta".into() } else { format!("Calculando el tamaño de {folders} carpetas") }
}

impl App {
    /// El reloj de lo que se muestra (`crate::clock`: las pruebas lo fijan).
    pub(super) fn now(&self) -> Instant {
        crate::clock::now()
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
        for (pi, p) in self.panes.iter().enumerate() {
            for (ti, tab) in p.tabs.iter().enumerate() {
                if let Some(s) = tab.search.as_ref().filter(|s| !s.done && !s.in_place) {
                    list.push((ActivityKey::Search(tab.id), format!("Buscando \"{}\"", s.query), Some(s.started), SHOW_AFTER));
                    continue;
                }
                // Lecturas: solo las de las pestañas a la vista (la activa de cada panel visible).
                let Some(t0) = tab.loading_since().filter(|_| ti == p.active && tab.is_visible() && pi < self.layout.panes()) else { continue };
                match (&tab.location, tab.slow_state(now)) {
                    // Ya dice «no responde»: quieto (sin girar ni redibujar), con su texto.
                    (Location::Path(path), crate::tab::Slow::NotResponding { .. }) => {
                        list.push((ActivityKey::Stalled(tab.id), format!("\"{}\" no responde", self.remote_host_of(path)), Some(t0), SHOW_AFTER));
                    }
                    _ => list.push((ActivityKey::Load(tab.id), format!("Cargando \"{}\"", tab.location.title()), Some(t0), crate::tab::LOADING_DELAY)),
                }
            }
        }
        // Comparar contenido y las sumas (del panel de comparar y del de información).
        if let Some(st) = &self.compare {
            if let Some(j) = &st.job {
                list.push((ActivityKey::Compare(j.id), format!("Comparando \"{}\" y \"{}\"", st.sides[0].name, st.sides[1].name), None, SHOW_AFTER));
            }
            for (h, side) in st.hashes.iter().zip(&st.sides) {
                if let Some(j) = &h.job {
                    let text = format!("Calculando la suma {} de \"{}\"", crate::sys::HashAlgo::Sha256.label(), side.name);
                    list.push((ActivityKey::Hash(j.id), text, None, SHOW_AFTER));
                }
            }
        }
        if let Some((algo, j)) = &self.info.sum.job {
            let name = self.info.sum.target.as_ref().map(|t| t.name()).unwrap_or_default();
            list.push((ActivityKey::Hash(j.id), format!("Calculando la suma {} de \"{name}\"", algo.label()), None, SHOW_AFTER));
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
        let now = self.now();
        let Some(wait) = self.activity.next_tick(now) else { return };
        if self.activity.tick_due.is_some_and(|d| d > now && d <= now + wait) {
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

    /// Ronda 1: lo que se llegó a ver queda al menos `MIN_SHOWN`; lo que no se vio sale enseguida.
    #[test]
    fn shown_stays_a_minimum() {
        let t0 = Instant::now();
        let mut a = Activities::default();
        a.begin(ActivityKey::Connect(1), "Conectando", t0);
        a.observe(t0 + ms(310));
        a.end(&ActivityKey::Connect(1));
        assert!(!a.contains(&ActivityKey::Connect(1)));
        a.observe(t0 + ms(320));
        assert_eq!(a.shown(t0 + ms(320)).len(), 1, "sigue a la vista");
        assert_eq!(a.next_tick(t0 + ms(320)), Some(FRAME));
        a.observe(t0 + ms(710));
        assert!(a.is_empty());
        // Lo derivado, igual.
        a.set_derived(vec![(ActivityKey::SelSize, "x".into(), Some(t0), SHOW_AFTER)], t0);
        a.observe(t0 + ms(300));
        a.set_derived(Vec::new(), t0 + ms(301));
        a.observe(t0 + ms(301));
        assert_eq!(a.shown(t0 + ms(301)).len(), 1);
        // Termina antes de verse: sale ya.
        a.begin(ActivityKey::Extract, "x", t0 + ms(400));
        a.end(&ActivityKey::Extract);
        assert!(!a.items.iter().any(|x| x.key == ActivityKey::Extract));
    }

    #[test]
    fn ticks_only_while_active() {
        let t0 = Instant::now();
        let mut a = Activities::default();
        assert_eq!(a.next_tick(t0), None);
        // Antes del umbral, un solo redibujo cuando lo pasa; después, uno por cuadro.
        a.begin(ActivityKey::SelSize, "x", t0);
        assert_eq!(a.next_tick(t0 + ms(100)), Some(ms(200)));
        assert_eq!(a.next_tick(t0 + ms(300)), Some(FRAME));
        a.end(&ActivityKey::SelSize);
        assert_eq!(a.next_tick(t0 + ms(300)), None);
        // Expulsar: su ⏏ gira desde el principio.
        a.set_derived(vec![(ActivityKey::Eject("e".into()), "Expulsando".into(), Some(t0), SHOW_AFTER)], t0);
        assert_eq!(a.next_tick(t0), Some(FRAME));
        // Una lectura rápida no pide ningún redibujo; pasados los 0,5 s, uno por cuadro.
        a.set_derived(vec![(ActivityKey::Load(1), "Cargando".into(), Some(t0), crate::tab::LOADING_DELAY)], t0);
        assert_eq!(a.next_tick(t0 + ms(100)), None);
        assert_eq!(a.next_tick(t0 + ms(500)), Some(FRAME));
    }
}

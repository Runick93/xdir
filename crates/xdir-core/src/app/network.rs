//! Red: favoritos y recientes (`red.toml`), lo conectado, las máquinas de la red local y
//! conectar a un servidor. Todo lo que puede esperar a un servidor corre en hilos de trabajo
//! (`spawn_task` + `net::with_timeout`) y vuelve por `TaskDone`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::App;
use super::tasks::TaskDone;
use crate::fs::Location;
use crate::geom::{self, Rect};
use crate::host::TextKind;
use crate::net::dialog::{self, ConnectDialog, DialogHit};
use crate::net::{self, Address, ConnectOptions, NetConnection, NetError, NetPlaces};
use crate::sys::{self, KeyStyle};

/// Tiempos límite (spec §B.2).
pub(super) const CONNECT_LIMIT: Duration = Duration::from_secs(30);
pub(super) const LIST_LIMIT: Duration = Duration::from_secs(10);
pub(super) const DISCONNECT_LIMIT: Duration = Duration::from_secs(15);

#[derive(Default)]
#[allow(dead_code)] // `hosts` y `browse_cancel` los usa «Red» en el árbol.
pub(super) struct NetState {
    pub places: NetPlaces,
    /// Lo conectado (lo último que dijo `Network::connections`).
    pub connections: Vec<NetConnection>,
    /// Máquinas que anuncia la red local (macOS y Linux, por `browse`).
    pub hosts: Vec<String>,
    /// Cancela el `browse` en curso.
    pub browse_cancel: Option<Arc<AtomicBool>>,
    /// «Conectar a un servidor» abierto.
    pub dialog: Option<ConnectDialog>,
    /// Generación de los intentos de conectar (un resultado de otra se descarta).
    pub generation: u64,
    /// Conexiones pedidas desde la barra de dirección: (pestaña, generación).
    pub bar_pending: Vec<(usize, u64)>,
    /// Conexiones en las que ya se reintentó desconectar (la próxima vez se ofrece forzar).
    pub retried: std::collections::HashSet<PathBuf>,
    /// Hay un redibujo pedido para este momento (`WM_APP_NET_TICK`).
    pub tick_due: Option<std::time::Instant>,
}

/// Segundos desde 1970 (UTC).
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// ¿El texto escrito es una dirección de servidor (`smb://`, `afp://`, `sftp://`)?
pub(super) fn is_server_url(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    ["smb://", "afp://", "sftp://"].iter().any(|p| t.starts_with(p))
}

/// La caja de texto dentro de un campo dibujado.
fn inner(r: Rect) -> Rect {
    geom::rect(r.left + 6.0, r.top + 4.0, r.right - 6.0, r.bottom - 4.0)
}

impl App {
    /// Lee `red.toml` en un hilo (al arrancar).
    pub(super) fn load_net_places(&self) {
        let dir = crate::session::config_dir();
        self.spawn_task(move || TaskDone::NetPlaces(dir.map(|d| NetPlaces::load_from(&d)).unwrap_or_default()));
    }

    /// Pide lo conectado en un hilo (al arrancar y al cambiar los discos) y refresca la lista de
    /// rutas remotas del servicio.
    pub(super) fn request_connections(&self) {
        self.spawn_task(|| {
            let net = sys::network();
            net.refresh_remote();
            TaskDone::NetConnections(net::with_timeout(LIST_LIMIT, "", move || sys::network().connections()))
        });
    }

    /// Guarda `red.toml` en un hilo.
    pub(super) fn save_net_places(&self) {
        // La carpeta se resuelve acá (sin E/S de red); la escritura va en un hilo.
        let (places, dir) = (self.net.places.clone(), crate::session::config_dir());
        std::thread::spawn(move || {
            if let Some(dir) = dir {
                let _ = places.save_in(&dir);
            }
        });
    }

    /// Conecta `addr` en un hilo; vuelve con `TaskDone::Connected { pane, tab, generation, .. }`.
    /// Devuelve la generación del intento.
    pub(super) fn spawn_connect(&mut self, pane: usize, tab: Option<usize>, addr: Address, opts: ConnectOptions) -> u64 {
        self.net.generation += 1;
        let generation = self.net.generation;
        let owner = self.hwnd;
        self.spawn_task(move || {
            let host = addr.host.clone();
            let a = addr.clone();
            let result = net::with_timeout(CONNECT_LIMIT, &host, move || sys::network().connect(owner, &a, opts, &AtomicBool::new(false)));
            TaskDone::Connected { pane, tab, generation, addr, result }
        });
        generation
    }

    /// Una carpeta UNC falló por credenciales: se conecta (el sistema pide usuario y contraseña)
    /// en un hilo y la pestaña recarga al volver.
    pub(super) fn connect_for_tab(&mut self, tab_id: usize, path: &Path) {
        let Ok(addr) = Address::parse(&path.to_string_lossy()) else { return };
        let pane = self.panes.iter().position(|p| p.tabs.iter().any(|t| t.id == tab_id)).unwrap_or(self.active);
        self.spawn_connect(pane, Some(tab_id), addr, ConnectOptions::default());
    }

    pub(super) fn on_net_places(&mut self, places: NetPlaces) {
        self.net.places = places;
        self.net_changed();
    }

    pub(super) fn on_net_connections(&mut self, result: Result<Vec<NetConnection>, NetError>) {
        if let Ok(list) = result {
            self.net.connections = list;
            self.net_changed();
        }
    }

    pub(super) fn on_connected(&mut self, pane: usize, tab: Option<usize>, generation: u64, addr: Address, result: Result<PathBuf, NetError>) {
        if result.is_ok() {
            self.request_connections();
        }
        match tab {
            Some(id) => self.connected_in_tab(id, generation, &addr, result),
            None => self.connected_from_dialog(pane, generation, &addr, result),
        }
    }

    /// Volvió una conexión pedida por una pestaña: la carpeta UNC que falló por credenciales
    /// (recarga) o una dirección escrita en la barra (navega a la ruta o deja el error).
    fn connected_in_tab(&mut self, id: usize, generation: u64, addr: &Address, result: Result<PathBuf, NetError>) {
        let from_bar = self.net.bar_pending.iter().any(|&(t, g)| t == id && g == generation);
        let stale = self.net.bar_pending.iter().any(|&(t, g)| t == id && g != generation) && !from_bar;
        self.net.bar_pending.retain(|&(t, g)| !(t == id && g <= generation));
        if stale {
            return;
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        let Some(t) = self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()).find(|t| t.id == id) else { return };
        match result {
            Ok(path) if from_bar => {
                t.navigate(hwnd, &tx, Location::Path(path));
                self.net.places.add_recent(addr, unix_now());
                self.save_net_places();
            }
            Ok(_) => t.reload_after_connect(hwnd, &tx),
            Err(e) if from_bar => {
                t.loading = false;
                t.error = Some(e.message());
            }
            Err(_) => {}
        }
        self.redraw();
    }

    /// Volvió la conexión del panel: abre la ruta en una pestaña nueva del panel y la suma a los
    /// recientes; si falló, el error queda en el panel. Un intento cancelado se descarta.
    fn connected_from_dialog(&mut self, pane: usize, generation: u64, addr: &Address, result: Result<PathBuf, NetError>) {
        let Some(d) = &mut self.net.dialog else { return };
        if d.busy != Some(generation) {
            return;
        }
        d.busy = None;
        match result {
            Ok(path) => {
                self.close_connect_dialog();
                let pane = pane.min(self.panes.len().saturating_sub(1));
                self.new_tab(pane, Location::Path(path), true);
                self.net.places.add_recent(addr, unix_now());
                self.save_net_places();
            }
            Err(e) => d.error = Some(e.message()),
        }
        self.redraw();
    }

    /// Cómo se muestran las direcciones: UNC solo donde el único esquema es `smb` (Windows).
    pub(super) fn net_style(&self) -> KeyStyle {
        let n = sys::network();
        if sys::env().key_style() == KeyStyle::Mac || n.supports("sftp") || n.supports("afp") { KeyStyle::Mac } else { KeyStyle::Windows }
    }

    /// Lo que suma el ejemplo del panel en este sistema.
    pub(super) fn net_example(&self) -> String {
        let n = sys::network();
        let extra = if n.supports("sftp") {
            Some("sftp")
        } else if n.supports("afp") {
            Some("afp")
        } else {
            None
        };
        dialog::example(extra)
    }

    /// La dirección de la carpeta del panel activo si es de red (para empezar el panel con ella).
    fn remote_address_of_active(&self) -> Option<String> {
        let Location::Path(p) = &self.panes[self.active].tab().location else { return None };
        if !sys::network().is_remote(p) {
            return None;
        }
        if crate::fs::is_unc(p) {
            return Address::parse(&p.to_string_lossy()).ok().map(|a| a.display(self.net_style()));
        }
        let c = self.net.connections.iter().filter(|c| p.starts_with(&c.local)).max_by_key(|c| c.local.as_os_str().len())?;
        let mut a = Address::parse(&c.address).ok()?;
        let rest: Vec<String> = p.strip_prefix(&c.local).ok()?.components().map(|x| x.as_os_str().to_string_lossy().into_owned()).collect();
        for r in rest {
            if a.share.is_none() {
                a.share = Some(r);
            } else {
                a.rest.push(r);
            }
        }
        Some(a.display(self.net_style()))
    }

    /// Ctrl+K / ⌘K: abre el panel con el foco en el campo.
    pub(super) fn open_connect_dialog(&mut self) {
        if self.net.dialog.is_some() {
            if let Some(b) = self.net.dialog.as_ref().and_then(|d| d.field) {
                self.host.text_focus(b);
            }
            return;
        }
        let text = self.remote_address_of_active().unwrap_or_default();
        self.net.dialog = Some(ConnectDialog::default());
        self.sync_connect_boxes();
        if let Some(b) = self.net.dialog.as_ref().and_then(|d| d.field) {
            self.host.text_set(b, &text);
            self.host.text_select(b, 0, -1);
            self.host.text_focus(b);
        }
        self.redraw();
    }

    /// Cierra el panel (un intento en curso se descarta: su generación ya no vale).
    pub(super) fn close_connect_dialog(&mut self) {
        let Some(d) = self.net.dialog.take() else { return };
        for b in [d.field, d.naming].into_iter().flatten() {
            self.host.text_destroy(b);
        }
        self.host.focus_main();
        self.redraw();
    }

    /// Las cajas del panel en su lugar (se recrean si el panel cambió de tamaño o de lugar).
    pub(super) fn sync_connect_boxes(&mut self) {
        let (w, h) = self.client_dips();
        let Some(d) = &self.net.dialog else { return };
        let l = dialog::layout(w, h, d, &self.net.places);
        let want = (l.field, l.naming);
        if d.boxes_at == Some(want) {
            return;
        }
        let (old_field, old_naming) = (d.field, d.naming);
        let keep = |b: Option<crate::host::TextBox>| b.map(|b| (self.host.text_get(b), self.host.text_has_focus(b)));
        let (field_text, naming_text) = (keep(old_field), keep(old_naming));
        for b in [old_field, old_naming].into_iter().flatten() {
            self.host.text_destroy(b);
        }
        let field = self.host.text_create(TextKind::Field, inner(l.field), field_text.as_ref().map_or("", |t| &t.0), &self.theme);
        let naming = match (l.naming, old_naming) {
            (Some(r), Some(_)) => self.host.text_create(TextKind::Field, inner(r), naming_text.as_ref().map_or("", |t| &t.0), &self.theme),
            _ => None,
        };
        if field_text.as_ref().is_some_and(|t| t.1) {
            field.inspect(|b| self.host.text_focus(*b));
        }
        if naming_text.as_ref().is_some_and(|t| t.1) {
            naming.inspect(|b| self.host.text_focus(*b));
        }
        if let Some(d) = &mut self.net.dialog {
            d.field = field;
            d.naming = naming;
            d.boxes_at = Some(want);
        }
    }

    /// Lo escrito en el campo.
    pub(super) fn connect_field_text(&self) -> String {
        self.net.dialog.as_ref().and_then(|d| d.field).map(|b| self.host.text_get(b)).unwrap_or_default()
    }

    fn set_connect_field(&mut self, text: &str) {
        if let Some(d) = &mut self.net.dialog {
            d.error = None;
        }
        if let Some(b) = self.net.dialog.as_ref().and_then(|d| d.field) {
            self.host.text_set(b, text);
            self.host.text_focus(b);
        }
    }

    /// Conectar (o Enter): valida, y conecta en un hilo; el resultado vuelve por `TaskDone`.
    pub(super) fn connect_from_dialog(&mut self) {
        let text = self.connect_field_text();
        let style = self.net_style();
        let Some(d) = &mut self.net.dialog else { return };
        if d.busy.is_some() {
            return;
        }
        let addr = match Address::parse(&text) {
            Ok(a) => a,
            Err(e) => {
                d.error = Some(e);
                self.redraw();
                return;
            }
        };
        if !sys::network().supports(&addr.scheme) {
            d.error = Some(NetError::Unsupported { scheme: addr.scheme }.message());
            self.redraw();
            return;
        }
        // La caja se queda con la dirección sin clave: la contraseña no sigue a la vista.
        let clean = addr.display(style);
        if let Some(b) = d.field
            && text.contains('@')
        {
            self.host.text_set(b, &clean);
        }
        let host = addr.host.clone();
        let pane = self.active;
        let generation = self.spawn_connect(pane, None, addr, ConnectOptions::default());
        if let Some(d) = &mut self.net.dialog {
            d.error = None;
            d.busy = Some(generation);
            d.busy_host = host;
        }
        self.redraw();
    }

    /// Esc, Cancelar o ✕: con un intento en curso, Cancelar lo descarta y el panel queda; si no,
    /// se cierra.
    fn cancel_connect(&mut self, close: bool) {
        match &mut self.net.dialog {
            Some(d) if d.busy.is_some() && !close => {
                d.busy = None;
                self.redraw();
            }
            Some(_) => self.close_connect_dialog(),
            None => {}
        }
    }

    /// Enter o Esc en una caja del panel. `true` si el panel lo atendió.
    pub(super) fn connect_field_done(&mut self, commit: bool) -> bool {
        let Some(d) = &self.net.dialog else { return false };
        if let Some(nb) = d.naming {
            let name = self.host.text_get(nb);
            self.host.text_destroy(nb);
            if let Some(d) = &mut self.net.dialog {
                d.naming = None;
                d.boxes_at = None;
            }
            if commit && let Ok(addr) = Address::parse(&self.connect_field_text()) {
                self.net.places.toggle_favorite(&addr, Some(name));
                self.save_net_places();
                self.net_changed();
            }
            self.sync_connect_boxes();
            if let Some(b) = self.net.dialog.as_ref().and_then(|d| d.field) {
                self.host.text_focus(b);
            }
            self.redraw();
            return true;
        }
        if commit { self.connect_from_dialog() } else { self.cancel_connect(true) }
        true
    }

    /// La estrella: quita la dirección de favoritos o pide el nombre para agregarla.
    fn connect_star(&mut self) {
        let text = self.connect_field_text();
        let addr = match Address::parse(&text) {
            Ok(a) => a,
            Err(e) => {
                if let Some(d) = &mut self.net.dialog {
                    d.error = Some(e);
                }
                return;
            }
        };
        if self.net.places.is_favorite(&addr) {
            self.net.places.toggle_favorite(&addr, None);
            self.save_net_places();
            self.net_changed();
            return;
        }
        let Some(d) = &mut self.net.dialog else { return };
        if d.naming.is_some() {
            return;
        }
        // Una caja provisoria para que el diseño reserve la fila; `sync_connect_boxes` la crea en
        // su lugar.
        d.naming = self.host.text_create(TextKind::Field, Rect::default(), "", &self.theme);
        d.boxes_at = None;
        self.sync_connect_boxes();
        if let Some(b) = self.net.dialog.as_ref().and_then(|d| d.naming) {
            self.host.text_focus(b);
        }
    }

    /// Clic en el panel (`None`: afuera, lo cierra).
    pub(super) fn connect_dialog_click(&mut self, h: Option<DialogHit>, double: bool) {
        let nf = self.net.places.favorites.len();
        match h {
            None | Some(DialogHit::Close) => self.cancel_connect(true),
            Some(DialogHit::Cancel) => self.cancel_connect(false),
            Some(DialogHit::Connect) => self.connect_from_dialog(),
            Some(DialogHit::Star) => self.connect_star(),
            Some(DialogHit::Field) => {
                if let Some(b) = self.net.dialog.as_ref().and_then(|d| d.field) {
                    self.host.text_focus(b);
                }
            }
            Some(DialogHit::Favorite(i)) => {
                let Some(a) = self.net.places.favorites.get(i).map(|f| dialog::shown(&f.address, self.net_style())) else { return };
                self.pick_connect_row(i, &a, double);
            }
            Some(DialogHit::Recent(i)) => {
                let Some(a) = self.net.places.recents.get(i).map(|r| dialog::shown(&r.address, self.net_style())) else { return };
                self.pick_connect_row(nf + i, &a, double);
            }
            Some(DialogHit::RemoveRecent(i)) => {
                self.net.places.remove_recent(i);
                self.save_net_places();
                if let Some(d) = &mut self.net.dialog {
                    d.selected = None;
                }
                self.sync_connect_boxes();
            }
            Some(DialogHit::Panel) => {}
        }
        self.redraw();
    }

    fn pick_connect_row(&mut self, row: usize, address: &str, double: bool) {
        if self.net.dialog.as_ref().is_some_and(|d| d.busy.is_some()) {
            return;
        }
        self.set_connect_field(address);
        if let Some(d) = &mut self.net.dialog {
            d.selected = Some(row);
        }
        if double {
            self.connect_from_dialog();
        }
    }

    /// Una dirección `smb://`, `afp://` o `sftp://` escrita en la barra: se conecta sin el panel,
    /// en la misma pestaña (la lista dice «Conectando…» y el error queda en la pestaña).
    pub(super) fn connect_from_bar(&mut self, pane: usize, text: &str) {
        let tab_id = self.panes[pane].tab().id;
        let addr = match Address::parse(text) {
            Ok(a) if sys::network().supports(&a.scheme) => a,
            Ok(a) => {
                self.panes[pane].tab_mut().error = Some(NetError::Unsupported { scheme: a.scheme }.message());
                self.redraw();
                return;
            }
            Err(e) => {
                self.panes[pane].tab_mut().error = Some(e);
                self.redraw();
                return;
            }
        };
        let generation = self.spawn_connect(pane, Some(tab_id), addr, ConnectOptions::default());
        self.net.bar_pending.push((tab_id, generation));
        let t = self.panes[pane].tab_mut();
        t.error = None;
        self.redraw();
    }

    /// Los nombres de las unidades de red del árbol, en otro hilo (con tiempo límite).
    pub(super) fn request_remote_drive_labels(&mut self) {
        #[cfg(feature = "arbol")]
        {
            let pending = self.tree.take_pending_labels();
            if pending.is_empty() {
                return;
            }
            self.spawn_task(move || {
                let labels = pending
                    .into_iter()
                    .filter_map(|p| {
                        let q = p.clone();
                        let host = crate::tab::remote_host(&p);
                        net::with_timeout(LIST_LIMIT, &host, move || {
                            let fs = sys::file_system();
                            Ok(fs.drive_label(&fs.drive_name(&q)))
                        })
                        .ok()
                        .map(|l| (p, l))
                    })
                    .collect();
                TaskDone::DriveLabels(labels)
            });
        }
    }

    pub(super) fn on_drive_labels(&mut self, labels: Vec<(PathBuf, String)>) {
        #[cfg(feature = "arbol")]
        for (p, l) in labels {
            self.tree.set_drive_name(&p, l);
        }
        #[cfg(not(feature = "arbol"))]
        let _ = labels;
    }

    /// ⏏ de una conexión de red: con trabajos de copia encima pregunta (`eject_after_jobs`),
    /// suelta lo de xdir (pestañas, xtree, vista previa) y desconecta en otro hilo.
    pub(super) fn disconnect(&mut self, root: PathBuf) {
        let label = self.connection_label(&root);
        let at = root.clone();
        self.eject_after_jobs(&at, &label, super::transfers::Verb::Disconnect, Box::new(move |app: &mut App| app.disconnect_now(root, false)));
    }

    /// El nombre de una conexión para los avisos: el último tramo de su dirección.
    pub(super) fn connection_label(&self, root: &Path) -> String {
        self.network_root(root)
            .and_then(|c| Address::parse(&c.address).ok().map(|a| a.label().to_string()))
            .unwrap_or_else(|| crate::tab::remote_host(root))
    }

    fn disconnect_now(&mut self, root: PathBuf, force: bool) {
        self.release_volume(&root);
        let label = self.connection_label(&root);
        self.spawn_task(move || {
            let r = root.clone();
            let host = label.clone();
            let result = net::with_timeout(DISCONNECT_LIMIT, &host, move || sys::network().disconnect(&r, force));
            TaskDone::Disconnected { root, label, force, result }
        });
    }

    /// Terminó de desconectar: el aviso, o la pregunta de reintentar (y, si el sistema puede y
    /// ya se reintentó, la de desconectar igual).
    pub(super) fn on_disconnected(&mut self, root: PathBuf, label: String, force: bool, result: Result<(), NetError>) {
        self.eject_done(&root);
        match result {
            Ok(()) => {
                self.net.retried.remove(&root);
                self.notice = Some(format!("Se desconectó \"{label}\""));
                self.net.connections.retain(|c| c.local != root);
                self.request_connections();
            }
            Err(NetError::OpenFiles { by }) => {
                let who = by.unwrap_or_else(|| "otro programa".into());
                if !force && self.net.retried.contains(&root) && sys::network().can_force() {
                    let q = format!("¿Desconectar igual? Lo que esté abierto en \"{label}\" puede perder cambios.");
                    self.net.retried.remove(&root);
                    if self.host.ask_yes_no_cancel("Desconectar", &q) == Some(true) {
                        self.disconnect_again(root, true);
                    }
                    return;
                }
                let q = format!("No se pudo desconectar \"{label}\": lo está usando {who}. ¿Reintentar?");
                if self.host.ask_yes_no_cancel("Desconectar", &q) == Some(true) {
                    self.net.retried.insert(root.clone());
                    self.disconnect_again(root, false);
                } else {
                    self.net.retried.remove(&root);
                }
            }
            Err(e) => {
                self.net.retried.remove(&root);
                let q = format!("No se pudo desconectar \"{label}\": {}. ¿Reintentar?", e.message());
                if self.host.ask_yes_no_cancel("Desconectar", &q) == Some(true) {
                    self.disconnect_again(root, false);
                }
            }
        }
    }

    /// Otro intento (sin volver a preguntar por los trabajos: ya se cancelaron).
    fn disconnect_again(&mut self, root: PathBuf, force: bool) {
        let label = self.connection_label(&root);
        let at = root.clone();
        self.eject_after_jobs(&at, &label, super::transfers::Verb::Disconnect, Box::new(move |app: &mut App| app.disconnect_now(root, force)));
    }

    /// La conexión que contiene `p` (la de ruta local más larga).
    pub(super) fn network_root(&self, p: &Path) -> Option<&NetConnection> {
        self.net.connections.iter().filter(|c| c.connected && !c.local.as_os_str().is_empty() && p.starts_with(&c.local)).max_by_key(|c| c.local.as_os_str().len())
    }

    /// El nombre del servidor de una ruta de red: el de su conexión si se sabe, si no el de la
    /// ruta.
    pub(super) fn remote_host_of(&self, p: &Path) -> String {
        self.network_root(p).and_then(|c| Address::parse(&c.address).ok()).map(|a| a.host).unwrap_or_else(|| crate::tab::remote_host(p))
    }

    /// Antes de dibujar: el aviso de cada lista a la vista que carga una carpeta de red lenta, y
    /// el próximo redibujo (`WM_APP_NET_TICK`) para cuando cambie.
    pub(super) fn update_placeholders(&mut self) {
        use crate::list::placeholder::Placeholder;
        use crate::tab::{SLOW_CONNECTING, SLOW_NOT_RESPONDING, Slow};
        let now = std::time::Instant::now();
        let mut next: Option<std::time::Duration> = None;
        for i in 0..self.panes.len() {
            let tab = self.panes[i].tab();
            let slow = tab.slow_state(now);
            let host = match &tab.location {
                Location::Path(p) if slow != Slow::No => self.remote_host_of(p),
                _ => String::new(),
            };
            let ph = match slow {
                Slow::No => None,
                Slow::Connecting { .. } => Some(Placeholder::Connecting(host)),
                Slow::NotResponding { .. } => Some(Placeholder::NotResponding(host)),
            };
            // Cuánto falta para el próximo umbral de una carga remota sin lotes.
            if let Some(wait) = tab.slow_pending(now, &[SLOW_CONNECTING, SLOW_NOT_RESPONDING]) {
                next = Some(next.map_or(wait, |n| n.min(wait)));
            }
            let hover = match self.hover {
                super::Hit::Pane(j, crate::pane::PaneHit::Placeholder(b)) if j == i && ph.is_some() => Some(b),
                _ => None,
            };
            let list = &mut self.panes[i].tab_mut().list;
            list.placeholder_hover = hover;
            list.placeholder = ph;
        }
        if let Some(wait) = next {
            let at = now + wait;
            if self.net.tick_due.is_none_or(|d| d > at || d < now) {
                self.net.tick_due = Some(at);
                let hwnd = self.hwnd;
                std::thread::spawn(move || {
                    std::thread::sleep(wait + std::time::Duration::from_millis(10));
                    hwnd.post(net::WM_APP_NET_TICK, 0, 0);
                });
            }
        }
    }

    /// `WM_APP_NET_TICK`: toca redibujar el aviso de una carpeta de red lenta.
    pub(super) fn on_net_tick(&mut self) {
        self.net.tick_due = None;
        self.redraw();
    }

    /// Después de «Reintentar»: el aviso vuelve a «Conectando…» al rato.
    pub(super) fn watch_slow_load(&mut self, _pane: usize) {
        self.net.tick_due = None;
        self.redraw();
    }

    /// Cambió algo de la red (favoritos, conexiones, máquinas): el árbol y lo que lo muestre.
    pub(super) fn net_changed(&mut self) {
        self.redraw();
    }
}

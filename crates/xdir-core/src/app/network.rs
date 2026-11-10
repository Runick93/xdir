//! Red: favoritos y recientes (`red.toml`), lo conectado, las máquinas de la red local y
//! conectar a un servidor. Todo lo que puede esperar a un servidor corre en hilos de trabajo
//! (`spawn_task` + `net::with_timeout`) y vuelve por `TaskDone`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::App;
use super::tasks::TaskDone;
use crate::net::{self, Address, ConnectOptions, NetConnection, NetError, NetPlaces};
use crate::sys;

/// Tiempos límite (spec §B.2).
pub(super) const CONNECT_LIMIT: Duration = Duration::from_secs(30);
pub(super) const LIST_LIMIT: Duration = Duration::from_secs(10);

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
}

impl App {
    /// Lee `red.toml` en un hilo (al arrancar).
    pub(super) fn load_net_places(&self) {
        self.spawn_task(|| TaskDone::NetPlaces(NetPlaces::load()));
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

    /// Conecta `addr` en un hilo; vuelve con `TaskDone::Connected { pane, tab, .. }`.
    pub(super) fn spawn_connect(&self, pane: usize, tab: Option<usize>, addr: Address, opts: ConnectOptions) {
        let owner = self.hwnd;
        self.spawn_task(move || {
            let host = addr.host.clone();
            let a = addr.clone();
            let result = net::with_timeout(CONNECT_LIMIT, &host, move || sys::network().connect(owner, &a, opts, &AtomicBool::new(false)));
            TaskDone::Connected { pane, tab, addr, result }
        });
    }

    /// Una carpeta UNC falló por credenciales: se conecta (el sistema pide usuario y contraseña)
    /// en un hilo y la pestaña recarga al volver.
    pub(super) fn connect_for_tab(&self, tab_id: usize, path: &Path) {
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

    pub(super) fn on_connected(&mut self, _pane: usize, tab: Option<usize>, addr: Address, result: Result<PathBuf, NetError>) {
        if let Some(id) = tab {
            if result.is_ok() {
                let (hwnd, tx) = (self.hwnd, self.tx.clone());
                if let Some(t) = self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()).find(|t| t.id == id) {
                    t.reload_after_connect(hwnd, &tx);
                }
                self.request_connections();
            }
            let _ = addr;
        }
    }

    /// Cambió algo de la red (favoritos, conexiones, máquinas): el árbol y lo que lo muestre.
    pub(super) fn net_changed(&mut self) {
        self.redraw();
    }
}

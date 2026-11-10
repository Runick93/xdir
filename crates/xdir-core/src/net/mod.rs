//! Red: direcciones de servidores (`Address`), errores en castellano (`NetError`), lo que el
//! servicio del sistema (`sys::Network`) recibe y devuelve, y un tiempo límite genérico para las
//! llamadas bloqueantes que pueden quedar colgadas esperando a un servidor.
//!
//! Lógica pura: nada de esto toca la red. Las contraseñas nunca se guardan: una dirección con
//! `usuario:clave@` pierde la clave al leerse, y la clave de `Credentials` vive en un `Secret`
//! que se pisa con ceros al soltarse.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::sys::KeyStyle;

pub mod dialog;
pub mod drives_dialog;
pub mod places;

pub use places::{NetFavorite, NetPlaces, NetRecent, PlacesWriter};

/// Eventos de `Network::browse` (máquinas que aparecen o desaparecen en la red local).
pub const WM_APP_NET: u32 = crate::WM_APP + 33;
/// Redibujo de «Conectando…» / «no responde» de una carpeta remota que tarda.
pub const WM_APP_NET_TICK: u32 = crate::WM_APP + 34;

/// Error de una dirección mal escrita.
pub const BAD_ADDRESS: &str = "Escribí una dirección como smb://servidor/recurso";

/// Una dirección de servidor, normalizada: esquema en minúsculas, sin barras de más y nunca con
/// contraseña. `share` puede faltar («conectarse al servidor» y elegir el recurso después).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub scheme: String,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub share: Option<String>,
    pub rest: Vec<String>,
}

fn bad() -> String {
    BAD_ADDRESS.to_string()
}

/// Caracteres que pueden ir sin codificar en un tramo de la URL.
fn plain_url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b'=' | b'@')
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if plain_url_byte(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `%XX` → byte; un `%` suelto o un resultado que no es UTF-8 es un error.
fn decode(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or_else(bad)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| bad())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| bad())
}

/// Un tramo de ruta ya decodificado: sin separadores, sin caracteres de control, sin `.`/`..`.
fn valid_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

/// Nombre de máquina o IPv4: letras, números, `.`, `-`, `_`.
fn valid_host(h: &str) -> bool {
    !h.is_empty() && h.len() <= 253 && h.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) && !h.starts_with(['.', '-'])
}

const BONJOUR_SUFFIXES: [&str; 2] = ["._smb._tcp.local", "._afpovertcp._tcp.local"];

/// ¿Es el nombre de un servicio de Bonjour? Con nombre antes del sufijo y sin separadores,
/// `@`, `:`, `%` ni caracteres de control.
fn is_bonjour_host(h: &str) -> bool {
    let lower = h.to_lowercase();
    BONJOUR_SUFFIXES.iter().any(|s| lower.ends_with(s) && lower.len() > s.len())
        && !h.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '@' | ':' | '%'))
}

/// La dirección para conectarse a una máquina que anuncia la red local: `smb://nombre` si el
/// nombre es un nombre de host; si no (los nombres de Bonjour llevan espacios), su servicio SMB
/// (`smb://Nombre%20con%20espacios._smb._tcp.local`), que el sistema resuelve.
pub fn host_address(name: &str) -> String {
    match Address::parse(&format!("smb://{name}")) {
        Ok(a) if a.share.is_none() => a.to_url(),
        _ => format!("smb://{}._smb._tcp.local", encode(name)),
    }
}

fn valid_ipv6(h: &str) -> bool {
    h.contains(':') && h.bytes().all(|b| b.is_ascii_hexdigit() || matches!(b, b':' | b'.'))
}

fn valid_user(u: &str) -> bool {
    !u.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '@' | ':'))
}

impl Address {
    /// Lee lo que escribió el usuario: `smb://srv/rec/c`, `\\srv\rec\c`, `//srv/rec`,
    /// `afp://srv/vol`, `sftp://[u@]srv[:p]/ruta` o un nombre suelto (`nas`, `192.168.1.10`), que
    /// se toma como `smb://nas`. La contraseña de `u:clave@` se descarta.
    pub fn parse(text: &str) -> Result<Address, String> {
        let t = text.trim();
        if t.is_empty() || t.chars().any(char::is_control) {
            return Err(bad());
        }
        if let Some(r) = t.strip_prefix(r"\\").or_else(|| t.strip_prefix("//")) {
            return Self::from_parts("smb".into(), r, true);
        }
        if let Some((scheme, rest)) = t.split_once("://") {
            let ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
            if !ok {
                return Err(bad());
            }
            return Self::from_parts(scheme.to_ascii_lowercase(), rest, false);
        }
        Self::from_parts("smb".into(), t, false)
    }

    fn from_parts(scheme: String, rest: &str, unc: bool) -> Result<Address, String> {
        let is_sep = |c: char| c == '/' || (unc && c == '\\');
        if !unc && rest.contains('\\') {
            return Err(bad());
        }
        let (authority, path) = match rest.find(is_sep) {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };
        let (userinfo, hostport) = match authority.rsplit_once('@') {
            Some((u, h)) if !unc => (Some(u), h),
            Some(_) => return Err(bad()),
            None => (None, authority),
        };
        // La clave (lo que sigue a `:`) se descarta acá y no sale de esta función.
        let user = match userinfo.map(|u| u.split(':').next().unwrap_or("")) {
            Some(u) if !u.is_empty() => {
                let u = decode(u)?;
                if !valid_user(&u) {
                    return Err(bad());
                }
                Some(u)
            }
            _ => None,
        };
        let (host, port) = if let Some(r) = hostport.strip_prefix('[') {
            let (h, after) = r.split_once(']').ok_or_else(bad)?;
            if !valid_ipv6(h) {
                return Err(bad());
            }
            let port = match after {
                "" => None,
                p => Some(p.strip_prefix(':').ok_or_else(bad)?),
            };
            (h.to_string(), port)
        } else {
            let (h, port) = match hostport.split_once(':') {
                Some((h, p)) if !unc => (h, Some(p)),
                Some(_) => return Err(bad()),
                None => (hostport, None),
            };
            let host = if valid_host(h) {
                h.to_string()
            } else {
                // Un servicio de Bonjour («MacBook de Ana._smb._tcp.local»): el nombre puede llevar
                // espacios y signos, codificados en la URL.
                let d = decode(h)?;
                if !is_bonjour_host(&d) {
                    return Err(bad());
                }
                d
            };
            (host, port)
        };
        let port = match port {
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n > 0 && p.bytes().all(|b| b.is_ascii_digit()) => Some(n),
                _ => return Err(bad()),
            },
            None => None,
        };
        let mut segments = Vec::new();
        for s in path.split(is_sep).filter(|s| !s.is_empty()) {
            let s = if unc { s.to_string() } else { decode(s)? };
            // En UNC un tramo con `:` sería un flujo alternativo de NTFS (`c:x`).
            if !valid_segment(&s) || (unc && s.contains(':')) {
                return Err(bad());
            }
            segments.push(s);
        }
        let mut segments = segments.into_iter();
        let share = segments.next();
        Ok(Address { scheme, user, host, port, share, rest: segments.collect() })
    }

    fn host_for_url(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else if valid_host(&self.host) {
            self.host.clone()
        } else {
            encode(&self.host)
        }
    }

    /// `esquema://[usuario@]servidor[:puerto]/recurso/resto`, con lo que haga falta codificado.
    pub fn to_url(&self) -> String {
        let mut out = format!("{}://", self.scheme);
        if let Some(u) = &self.user {
            out += &encode(u);
            out.push('@');
        }
        out += &self.host_for_url();
        if let Some(p) = self.port {
            out += &format!(":{p}");
        }
        for s in self.share.iter().chain(self.rest.iter()) {
            out.push('/');
            out += &encode(s);
        }
        out
    }

    /// `\\servidor\recurso\resto` (solo `smb`).
    pub fn to_unc(&self) -> Option<String> {
        if self.scheme != "smb" {
            return None;
        }
        // Windows escribe una IPv6 en UNC como `fe80--1.ipv6-literal.net`.
        let host = if self.host.contains(':') { format!("{}.ipv6-literal.net", self.host.replace(':', "-")) } else { self.host.clone() };
        let mut out = format!(r"\\{host}");
        for s in self.share.iter().chain(self.rest.iter()) {
            out.push('\\');
            out += s;
        }
        Some(out)
    }

    /// Cómo se muestra en este sistema: UNC en Windows si se puede, si no la URL.
    pub fn display(&self, style: KeyStyle) -> String {
        match style {
            KeyStyle::Windows => self.to_unc().unwrap_or_else(|| self.to_url()),
            KeyStyle::Mac => self.to_url(),
        }
    }

    /// El último tramo con nombre (recurso o carpeta), o el servidor.
    pub fn label(&self) -> &str {
        self.rest.last().or(self.share.as_ref()).unwrap_or(&self.host)
    }

    /// ¿Es el mismo servidor y recurso? (sin distinguir mayúsculas; el resto no cuenta).
    pub fn same_share(&self, o: &Address) -> bool {
        let eq = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => true,
            _ => false,
        };
        self.scheme == o.scheme && self.host.eq_ignore_ascii_case(&o.host) && eq(&self.share, &o.share)
    }

    /// ¿Es el mismo lugar? Servidor, recurso y carpetas, sin distinguir mayúsculas (ruling: el
    /// plan pedía servidor y recurso; con las carpetas, dos recientes de subcarpetas distintas
    /// no se pisan).
    pub fn same(&self, o: &Address) -> bool {
        self.same_share(o) && self.rest.len() == o.rest.len() && self.rest.iter().zip(&o.rest).all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
}

/// Lo que puede salir mal al conectar, listar o desconectar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    NotFound { host: String },
    BadCredentials,
    NoShare { host: String, share: String },
    Timeout { host: String },
    CredentialConflict { host: String },
    /// Hay archivos abiertos en la conexión (`by`: el programa, si se sabe).
    OpenFiles { by: Option<String> },
    Unsupported { scheme: String },
    /// Linux: `gio` pidió usuario (y dominio) o contraseña y no se le dieron.
    NeedsCredentials { user: bool, domain: bool },
    NoGvfs,
    Cancelled,
    Other(String),
}

impl NetError {
    /// El texto para el usuario (nunca lleva una contraseña: ningún error la recibe).
    pub fn message(&self) -> String {
        match self {
            NetError::NotFound { host } => format!("No se encuentra el servidor \"{host}\""),
            NetError::BadCredentials => "Usuario o contraseña incorrectos".into(),
            NetError::NoShare { host, share } => format!("El recurso \"{share}\" no existe en \"{host}\""),
            NetError::Timeout { host } => format!("Se agotó el tiempo esperando a \"{host}\""),
            NetError::CredentialConflict { host } => format!("Ya hay una conexión con \"{host}\" con otro usuario"),
            NetError::OpenFiles { by } => format!("lo está usando {}", by.as_deref().unwrap_or("otro programa")),
            NetError::Unsupported { scheme } => format!("En este sistema no se puede conectar a direcciones {scheme}://"),
            NetError::NeedsCredentials { user: true, .. } => "El servidor pide usuario y contraseña".into(),
            NetError::NeedsCredentials { .. } => "El servidor pide una contraseña".into(),
            NetError::NoGvfs => "Hace falta gvfs para conectarse a servidores".into(),
            NetError::Cancelled => "Se canceló la conexión".into(),
            NetError::Other(s) => s.clone(),
        }
    }
}

/// Una conexión de red del sistema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetConnection {
    /// La dirección (URL o UNC), sin contraseña.
    pub address: String,
    /// Dónde se ve en el disco: `/Volumes/fotos`, `Z:\`, `\\srv\rec`.
    pub local: PathBuf,
    pub letter: Option<char>,
    /// Windows: se vuelve a conectar al iniciar sesión.
    pub reconnect: bool,
    /// false: unidad recordada que hoy no está conectada.
    pub connected: bool,
}

/// Una contraseña en memoria: sin `Debug` ni `Clone`, y sus bytes se pisan con ceros al soltarla.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Secret {
        Secret(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Pisa los bytes con ceros (también lo hace `Drop`).
    pub fn wipe(&mut self) {
        self.0.fill(0);
        // `black_box`: el compilador no puede quitar la escritura aunque nadie lea después.
        std::hint::black_box(&mut self.0);
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
    }
}

/// Usuario, dominio y contraseña para el servidor (solo Linux los pide xdir).
pub struct Credentials {
    pub user: String,
    pub domain: Option<String>,
    pub password: Secret,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("user", &self.user).field("domain", &self.domain).finish_non_exhaustive()
    }
}

/// Cómo conectar: letra y «Conectar de nuevo» solo en Windows.
#[derive(Debug, Default)]
pub struct ConnectOptions {
    pub drive_letter: Option<char>,
    pub reconnect: bool,
    pub other_credentials: bool,
    pub credentials: Option<Credentials>,
}

impl ConnectOptions {
    /// Las mismas opciones sin las credenciales (para anotar o mostrar).
    pub fn without_secret(&self) -> ConnectOptions {
        ConnectOptions { drive_letter: self.drive_letter, reconnect: self.reconnect, other_credentials: self.other_credentials, credentials: None }
    }
}

/// Corre `f` en un hilo propio y espera hasta `limit`. Si vence, devuelve `Timeout { host }` y el
/// hilo queda abandonado (su resultado se descarta al terminar, si termina).
pub fn with_timeout<T: Send + 'static>(
    limit: Duration,
    host: &str,
    f: impl FnOnce() -> Result<T, NetError> + Send + 'static,
) -> Result<T, NetError> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("xdir-red".into()).spawn(move || {
        let _ = tx.send(f());
    });
    if spawned.is_err() {
        return Err(NetError::Other("no se pudo crear un hilo".into()));
    }
    match rx.recv_timeout(limit) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(NetError::Timeout { host: host.to_string() }),
        // El hilo terminó sin respuesta (entró en pánico): no es que el servidor tarde.
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(NetError::Other("La operación de red falló".into())),
    }
}

/// Tiempo límite de un elemento de una carpeta de red en las colas de miniaturas y metadatos.
pub const ITEM_LIMIT: Duration = Duration::from_secs(5);

/// `f(path)` con tiempo límite si la ruta es de red (`ITEM_LIMIT`): sin respuesta, `T::default()`
/// y la cola sigue (el hilo se abandona). Una ruta local se lee directo. Corre en hilos de
/// trabajo (`f` es una función, así corre en un hilo nuevo sin estado del que llama).
pub fn remote_bounded<T: Default + Send + 'static>(path: &std::path::Path, f: fn(&std::path::Path) -> T) -> T {
    if !crate::sys::network().is_remote(path) {
        return f(path);
    }
    let p = path.to_path_buf();
    let host = crate::tab::remote_host(path);
    with_timeout(ITEM_LIMIT, &host, move || Ok(f(&p))).unwrap_or_default()
}

/// Qué servidor es una ruta de red: `\\srv` en UNC; si no, la carpeta más alta que todavía es
/// de red (el punto de montaje). Sin E/S.
fn server_key(path: &std::path::Path) -> String {
    if crate::fs::is_unc(path) {
        return crate::tab::remote_host(path);
    }
    let mut ancestors: Vec<&std::path::Path> = path.ancestors().collect();
    ancestors.reverse();
    let net = crate::sys::network();
    ancestors.into_iter().find(|a| net.is_remote(a)).unwrap_or(path).to_string_lossy().into_owned()
}

/// `remote_bounded` para una tanda (un pedido de metadatos): si un servidor ya venció, los
/// demás elementos de ese servidor no esperan (quedan sin dato) y la cola sigue.
pub struct RemoteBudget {
    limit: Duration,
    dead: Vec<String>,
}

impl Default for RemoteBudget {
    fn default() -> Self {
        Self::with_limit(ITEM_LIMIT)
    }
}

impl RemoteBudget {
    pub fn with_limit(limit: Duration) -> Self {
        RemoteBudget { limit, dead: Vec::new() }
    }

    pub fn read<T: Default + Send + 'static>(&mut self, path: &std::path::Path, f: fn(&std::path::Path) -> T) -> T {
        if !crate::sys::network().is_remote(path) {
            return f(path);
        }
        let host = server_key(path);
        if self.dead.contains(&host) {
            return T::default();
        }
        let p = path.to_path_buf();
        match with_timeout(self.limit, &host, move || Ok(f(&p))) {
            Ok(v) => v,
            Err(_) => {
                self.dead.push(host);
                T::default()
            }
        }
    }
}

#[cfg(test)]
mod tests;

//! Red en Linux (`LinuxNetwork`) con `gio` (de glib/gvfs) como proceso hijo, sin enlazar nada:
//! `gio mount <url>` para conectar (si pregunta usuario, dominio o contraseña y no se le dieron,
//! `NeedsCredentials`; con credenciales, se escriben en su `stdin` y la contraseña se pisa al
//! soltarla), `gio mount -l` para lo conectado, `gio mount -u` para desconectar y
//! `gio list network:///` para la red local. Sin `gio`, todo dice «Hace falta gvfs…».
//!
//! Los argumentos van como vector (nunca una línea de shell) y la dirección llega ya validada por
//! `Address::parse`. Todo es bloqueante: corre en hilos de trabajo.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use xdir_core::net::{Address, ConnectOptions, Credentials, NetConnection, NetError, WM_APP_NET};
use xdir_core::sys::{Handle, HostEvent, Network};

/// Lo que pregunta `gio mount`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    User,
    Domain,
    Password,
}

/// ¿Esta línea (sin terminar) es una pregunta de `gio`? («User [ana]: », «Domain [WORKGROUP]: »,
/// «Password: »).
pub fn gio_prompt(line: &str) -> Option<Prompt> {
    let l = line.trim();
    if !l.ends_with(':') {
        return None;
    }
    let head = l.split([' ', '[', ':']).next().unwrap_or("");
    match head {
        "User" | "Usuario" => Some(Prompt::User),
        "Domain" | "Dominio" => Some(Prompt::Domain),
        "Password" | "Contraseña" => Some(Prompt::Password),
        _ => None,
    }
}

/// El error de `gio` (su `stderr`) en castellano.
pub fn gio_error(stderr: &str, host: &str, share: &str) -> NetError {
    let low = stderr.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| low.contains(w));
    if has(&["connection refused", "no route to host", "name or service not known", "could not resolve", "failed to resolve", "host is down", "network is unreachable"]) {
        NetError::NotFound { host: host.into() }
    } else if has(&["no such file or directory", "no such share", "share does not exist"]) {
        NetError::NoShare { host: host.into(), share: share.into() }
    } else if has(&["permission denied", "login failed", "authentication failed", "access denied"]) {
        NetError::BadCredentials
    } else if has(&["timed out", "timeout"]) {
        NetError::Timeout { host: host.into() }
    } else if has(&["cancelled", "canceled", "aborted"]) {
        NetError::Cancelled
    } else if has(&["not supported"]) && low.contains("location") {
        NetError::Other("gvfs no sabe conectarse a esa dirección".into())
    } else {
        let line = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("error de gio");
        // «gio: smb://nas/x: …» → lo de después de la dirección.
        let msg = line.strip_prefix("gio:").map(str::trim).unwrap_or(line);
        let msg = msg.rsplit_once(": ").map_or(msg, |(_, m)| m);
        NetError::Other(msg.to_string())
    }
}

/// Los montajes de red de `gio mount -l` (líneas «Mount(N): nombre -> url»): solo smb, sftp, afp
/// y dav; sin contraseña en la dirección. La ruta local se completa aparte (`gio info`).
pub fn parse_mount_list(out: &str) -> Vec<NetConnection> {
    let mut list: Vec<NetConnection> = Vec::new();
    for line in out.lines().map(str::trim) {
        if !line.starts_with("Mount(") {
            continue;
        }
        let Some((_, uri)) = line.rsplit_once(" -> ") else { continue };
        let uri = uri.trim();
        let scheme = uri.split("://").next().unwrap_or("").to_ascii_lowercase();
        if !matches!(scheme.as_str(), "smb" | "sftp" | "afp" | "dav" | "davs") {
            continue;
        }
        let address = Address::parse(uri).map(|a| a.to_url()).unwrap_or_else(|_| uri.trim_end_matches('/').to_string());
        if !list.iter().any(|c| c.address == address) {
            list.push(NetConnection { address, local: PathBuf::new(), letter: None, reconnect: false, connected: true });
        }
    }
    list
}

/// Las máquinas de `gio list -d network:///` (una por línea; sin los grupos como «Windows
/// Network»).
pub fn parse_network_list(out: &str) -> Vec<String> {
    let groups = ["windows network", "red de windows", "network", "red"];
    let mut hosts: Vec<String> = Vec::new();
    for l in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if groups.contains(&l.to_lowercase().as_str()) || l.contains("://") {
            continue;
        }
        if !hosts.iter().any(|h| h.eq_ignore_ascii_case(l)) {
            hosts.push(l.to_string());
        }
    }
    hosts
}

/// Los puntos de montaje de red de `/proc/self/mounts` y todo lo de gvfs.
pub fn remote_mounts(mounts: &str, gvfs: &Path) -> Vec<PathBuf> {
    let mut out = vec![gvfs.to_path_buf()];
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        let (Some(_), Some(dir), Some(kind)) = (f.next(), f.next(), f.next()) else { continue };
        if matches!(kind, "cifs" | "smb3" | "nfs" | "nfs4" | "fuse.sshfs") {
            // `/proc/self/mounts` escapa los espacios como \040.
            out.push(PathBuf::from(dir.replace("\\040", " ")));
        }
    }
    out
}

/// La ruta local de `gio info` («local path: /run/user/1000/gvfs/…»).
pub fn parse_local_path(info: &str) -> Option<PathBuf> {
    info.lines().map(str::trim).find_map(|l| l.strip_prefix("local path:")).map(|p| PathBuf::from(p.trim())).filter(|p| !p.as_os_str().is_empty())
}

/// `gio` en este `PATH`.
fn find_in_path(path_var: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    path_var.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("gio")).find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

fn gvfs_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))).join("gvfs")
}

pub struct LinuxNetwork {
    gio: Option<PathBuf>,
    remote: RwLock<Vec<PathBuf>>,
    /// `afp`: 0 sin saber, 1 no, 2 sí (se lee una vez, en otro hilo).
    afp: Arc<AtomicU8>,
}

impl LinuxNetwork {
    pub fn new() -> LinuxNetwork {
        Self::with_path(&std::env::var("PATH").unwrap_or_default())
    }

    /// Con `gio` buscado en este `PATH` (las pruebas usan uno propio).
    pub fn with_path(path_var: &str) -> LinuxNetwork {
        let n = LinuxNetwork { gio: find_in_path(path_var), remote: RwLock::new(Vec::new()), afp: Arc::new(AtomicU8::new(0)) };
        n.refresh_remote();
        n
    }

    fn gio(&self) -> Result<&Path, NetError> {
        self.gio.as_deref().ok_or(NetError::NoGvfs)
    }

    fn run(&self, args: &[&str]) -> Result<(bool, String, String), NetError> {
        let out = Command::new(self.gio()?).args(args).stdin(Stdio::null()).output().map_err(|e| NetError::Other(e.to_string()))?;
        Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
    }

    fn local_path(&self, url: &str) -> Option<PathBuf> {
        let (ok, out, _) = self.run(&["info", url]).ok()?;
        ok.then(|| parse_local_path(&out)).flatten()
    }

    /// `gio mount <url>` contestando lo que pregunta (o cortando si no hay credenciales).
    fn mount(&self, url: &str, host: &str, share: &str, creds: Option<Credentials>, cancel: &AtomicBool) -> Result<(), NetError> {
        let mut child = Command::new(self.gio()?)
            .args(["mount", url])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| NetError::Other(e.to_string()))?;
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let mut stdout = child.stdout.take().expect("stdout por tubo");
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut stderr = child.stderr.take().expect("stderr por tubo");
        let err_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let mut creds = creds;
        let mut pending = String::new();
        let (mut asked_password, mut seen) = (false, Vec::new());
        let kill = |c: &mut Child| {
            let _ = c.kill();
            let _ = c.wait();
        };
        loop {
            if cancel.load(Ordering::Relaxed) {
                kill(&mut child);
                return Err(NetError::Cancelled);
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(bytes) => pending.push_str(&String::from_utf8_lossy(&bytes)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Ok(Some(_)) = child.try_wait() {
                        break;
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let line = pending.rsplit('\n').next().unwrap_or("").to_string();
            let Some(p) = gio_prompt(&line) else { continue };
            pending.clear();
            seen.push(p);
            let Some(c) = &mut creds else {
                kill(&mut child);
                let user = seen.contains(&Prompt::User);
                return Err(NetError::NeedsCredentials { user, domain: user && url.starts_with("smb") });
            };
            if p == Prompt::Password && asked_password {
                // Vuelve a preguntar: la contraseña no era.
                kill(&mut child);
                c.password.wipe();
                return Err(NetError::BadCredentials);
            }
            let Some(stdin) = child.stdin.as_mut() else { break };
            let wrote = match p {
                Prompt::User => stdin.write_all(c.user.as_bytes()),
                Prompt::Domain => stdin.write_all(c.domain.as_deref().unwrap_or("").as_bytes()),
                Prompt::Password => {
                    asked_password = true;
                    // Directo de la memoria de la clave al tubo; después se pisa.
                    let r = stdin.write_all(c.password.expose());
                    c.password.wipe();
                    r
                }
            };
            if wrote.and_then(|_| stdin.write_all(b"\n")).and_then(|_| stdin.flush()).is_err() {
                break;
            }
        }
        drop(creds);
        let status = child.wait().map_err(|e| NetError::Other(e.to_string()))?;
        let err = err_reader.join().unwrap_or_default();
        if status.success() || err.to_lowercase().contains("already mounted") {
            Ok(())
        } else {
            Err(gio_error(&err, host, share))
        }
    }

    /// La dirección de lo montado en `local`.
    fn url_of(&self, local: &Path) -> Option<String> {
        self.connections().ok()?.into_iter().find(|c| local.starts_with(&c.local) && !c.local.as_os_str().is_empty()).map(|c| c.address)
    }
}

impl Network for LinuxNetwork {
    fn supports(&self, scheme: &str) -> bool {
        if self.gio.is_none() {
            return false;
        }
        match scheme {
            "smb" | "sftp" => true,
            "afp" => {
                if self.afp.load(Ordering::Relaxed) == 0 {
                    // Una vez, en otro hilo; hasta saberlo, no.
                    self.afp.store(1, Ordering::Relaxed);
                    let (gio, flag) = (self.gio.clone(), self.afp.clone());
                    std::thread::spawn(move || {
                        let types = gio.and_then(|g| Command::new(g).args(["mount", "--list-types"]).stdin(Stdio::null()).output().ok());
                        let afp = types.is_some_and(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().any(|t| t == "afp"));
                        flag.store(if afp { 2 } else { 1 }, Ordering::Relaxed);
                    });
                }
                self.afp.load(Ordering::Relaxed) == 2
            }
            _ => false,
        }
    }

    fn connect(&self, _owner: Handle, addr: &Address, opts: ConnectOptions, cancel: &AtomicBool) -> Result<PathBuf, NetError> {
        self.gio()?;
        let url = addr.to_url();
        let share = addr.share.clone().unwrap_or_default();
        self.mount(&url, &addr.host, &share, opts.credentials, cancel)?;
        self.refresh_remote();
        // La carpeta pedida si gvfs la conoce; si no, la del recurso, o la carpeta de gvfs.
        self.local_path(&url)
            .or_else(|| {
                let mut base = addr.clone();
                base.rest.clear();
                self.local_path(&base.to_url()).map(|p| addr.rest.iter().fold(p, |p, r| p.join(r)))
            })
            .ok_or_else(|| NetError::Other("gvfs no dijo dónde quedó montado".into()))
    }

    fn connections(&self) -> Result<Vec<NetConnection>, NetError> {
        let (_, out, _) = self.run(&["mount", "-l"])?;
        let mut list = parse_mount_list(&out);
        for c in &mut list {
            c.local = self.local_path(&c.address).unwrap_or_default();
        }
        Ok(list)
    }

    fn disconnect(&self, local: &Path, _force: bool) -> Result<(), NetError> {
        self.gio()?;
        let url = self.url_of(local).ok_or_else(|| NetError::Other("No es una conexión de gvfs".into()))?;
        let (ok, _, err) = self.run(&["mount", "-u", &url])?;
        self.refresh_remote();
        if ok {
            return Ok(());
        }
        if err.to_lowercase().contains("busy") {
            let users = crate::shell::holders(Path::new("/proc"), local);
            let own = std::fs::read_to_string("/proc/self/comm").map(|s| s.trim().to_string()).ok();
            let by = users.into_iter().find(|u| Some(u) != own.as_ref());
            return Err(NetError::OpenFiles { by });
        }
        let host = Address::parse(&url).map(|a| a.host).unwrap_or_default();
        Err(gio_error(&err, &host, ""))
    }

    fn browse(&self, hwnd: Handle, tx: Sender<HostEvent>, cancel: Arc<AtomicBool>) {
        if self.gio.is_none() {
            return;
        }
        let mut known: Vec<String> = Vec::new();
        while !cancel.load(Ordering::Relaxed) {
            if let Ok((true, out, _)) = self.run(&["list", "-d", "network:///"]) {
                let now = parse_network_list(&out);
                let events: Vec<HostEvent> = now
                    .iter()
                    .filter(|h| !known.contains(h))
                    .map(|h| HostEvent::Up(h.clone()))
                    .chain(known.iter().filter(|h| !now.contains(h)).map(|h| HostEvent::Down(h.clone())))
                    .collect();
                for e in events {
                    if tx.send(e).is_err() {
                        return;
                    }
                    hwnd.post(WM_APP_NET, 0, 0);
                }
                known = now;
            }
            // Cada 30 s, mirando `cancel` seguido.
            for _ in 0..150 {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }

    fn is_remote(&self, path: &Path) -> bool {
        self.remote.read().map(|r| r.iter().any(|root| path.starts_with(root))).unwrap_or(false)
    }

    fn refresh_remote(&self) {
        let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let roots = remote_mounts(&mounts, &gvfs_dir());
        if let Ok(mut r) = self.remote.write() {
            *r = roots;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xdir_core::net::Secret;

    #[test]
    fn gio_prompts() {
        assert_eq!(gio_prompt("User [ana]: "), Some(Prompt::User));
        assert_eq!(gio_prompt("Domain [WORKGROUP]: "), Some(Prompt::Domain));
        assert_eq!(gio_prompt("Password: "), Some(Prompt::Password));
        assert_eq!(gio_prompt("Password"), None);
        assert_eq!(gio_prompt("Mounted smb://nas/x at …"), None);
    }

    #[test]
    fn gio_errors() {
        let e = |s| gio_error(s, "nas", "fotos");
        assert_eq!(e("gio: smb://nas/fotos: Failed to mount Windows share: Connection refused\n"), NetError::NotFound { host: "nas".into() });
        assert_eq!(e("gio: smb://nas/nada: Failed to mount Windows share: No such file or directory\n"), NetError::NoShare { host: "nas".into(), share: "fotos".into() });
        assert_eq!(e("gio: smb://nas/fotos: Failed to mount Windows share: Permission denied\n"), NetError::BadCredentials);
        assert_eq!(e("gio: sftp://vps/: Timed out\n"), NetError::Timeout { host: "nas".into() });
        assert_eq!(e("gio: smb://nas/fotos: Algo raro\n"), NetError::Other("Algo raro".into()));
    }

    #[test]
    fn parse_mount_list_network_only() {
        let out = "Drive(0): ST1000\n  Type: GProxyDrive\n  Volume(0): Datos\nMount(0): fotos en nas -> smb://nas/fotos/\n  Type: GDaemonMount\nMount(1): Datos -> file:///media/ana/Datos\nMount(2): www en vps -> sftp://ana@vps/var/www\nMount(3): x -> smb://bob:clave@nas/x/\n";
        let l = parse_mount_list(out);
        let a: Vec<&str> = l.iter().map(|c| c.address.as_str()).collect();
        assert_eq!(a, ["smb://nas/fotos", "sftp://ana@vps/var/www", "smb://bob@nas/x"]);
    }

    #[test]
    fn parse_network_list_hosts() {
        assert_eq!(parse_network_list("Windows Network\nNAS-CASA\nMACBOOK-DE-ANA\n\nNAS-CASA\n"), ["NAS-CASA", "MACBOOK-DE-ANA"]);
    }

    #[test]
    fn remote_mounts_types() {
        let m = "/dev/sda1 / ext4 rw 0 0\n//nas/fotos /mnt/fotos cifs rw 0 0\nnas:/x /mnt/nfs nfs4 rw 0 0\nana@vps:/ /mnt/my\\040vps fuse.sshfs rw 0 0\nproc /proc proc rw 0 0\n";
        let r = remote_mounts(m, Path::new("/run/user/1000/gvfs"));
        assert_eq!(r, ["/run/user/1000/gvfs", "/mnt/fotos", "/mnt/nfs", "/mnt/my vps"].map(PathBuf::from));
        assert_eq!(parse_local_path("display name: fotos\nlocal path: /run/user/1000/gvfs/smb-share:server=nas,share=fotos\nuri: smb://nas/fotos/\n"), Some(PathBuf::from("/run/user/1000/gvfs/smb-share:server=nas,share=fotos")));
    }

    #[test]
    fn no_gio_reports_needs_gvfs() {
        let n = LinuxNetwork::with_path("");
        assert!(!n.supports("smb"));
        let a = Address::parse("smb://nas/fotos").unwrap();
        let r = n.connect(Handle(0), &a, ConnectOptions::default(), &AtomicBool::new(false));
        assert_eq!(r, Err(NetError::NoGvfs));
        assert_eq!(NetError::NoGvfs.message(), "Hace falta gvfs para conectarse a servidores");
        assert_eq!(n.connections(), Err(NetError::NoGvfs));
    }

    /// Un `gio` falso en una carpeta propia: pide usuario, dominio y contraseña, escribe lo
    /// recibido en un archivo y dice dónde quedó montado.
    fn fake_gio(dir: &Path, received: &Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = mount ] && [ \"$2\" != -l ]; then\n  printf 'User [nobody]: '; read u\n  printf 'Domain [WORKGROUP]: '; read d\n  printf 'Password: '; read p\n  printf '%s|%s|%s' \"$u\" \"$d\" \"$p\" > '{}'\n  exit 0\nfi\nif [ \"$1\" = info ]; then echo 'local path: /run/user/1/gvfs/smb-share:server=nas,share=fotos'; exit 0; fi\nexit 1\n",
            received.display()
        );
        let gio = dir.join("gio");
        std::fs::write(&gio, script).unwrap();
        std::fs::set_permissions(&gio, std::fs::Permissions::from_mode(0o755)).unwrap();
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn credentials_written_once() {
        let dir = std::env::temp_dir().join(format!("xdir-gio-{}", std::process::id()));
        let received = dir.join("recibido.txt");
        let n = LinuxNetwork::with_path(&fake_gio(&dir.join("bin"), &received));
        let a = Address::parse("smb://nas/fotos").unwrap();
        // Sin credenciales: las pide.
        let r = n.connect(Handle(0), &a, ConnectOptions::default(), &AtomicBool::new(false));
        assert_eq!(r, Err(NetError::NeedsCredentials { user: true, domain: true }));
        assert!(!received.exists());
        // Con credenciales: las escribe en el stdin de gio y conecta.
        let creds = Credentials { user: "ana".into(), domain: None, password: Secret::new(b"secreto".to_vec()) };
        let opts = ConnectOptions { credentials: Some(creds), ..Default::default() };
        let r = n.connect(Handle(0), &a, opts, &AtomicBool::new(false));
        assert_eq!(r, Ok(PathBuf::from("/run/user/1/gvfs/smb-share:server=nas,share=fotos")));
        assert_eq!(std::fs::read_to_string(&received).unwrap(), "ana||secreto");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

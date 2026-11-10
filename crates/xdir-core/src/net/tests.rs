use std::time::{Duration, Instant};

use super::*;

fn p(s: &str) -> Address {
    Address::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

#[test]
fn parse_forms() {
    let a = p("smb://srv/rec/c");
    assert_eq!((a.scheme.as_str(), a.host.as_str(), a.share.as_deref(), a.rest.clone()), ("smb", "srv", Some("rec"), vec!["c".to_string()]));
    let a = p(r"\\srv\rec\c");
    assert_eq!((a.scheme.as_str(), a.host.as_str(), a.share.as_deref(), a.rest.clone()), ("smb", "srv", Some("rec"), vec!["c".to_string()]));
    let a = p("//srv/rec");
    assert_eq!((a.scheme.as_str(), a.host.as_str(), a.share.as_deref(), a.rest.len()), ("smb", "srv", Some("rec"), 0));
    let a = p("afp://srv/vol");
    assert_eq!((a.scheme.as_str(), a.host.as_str(), a.share.as_deref()), ("afp", "srv", Some("vol")));
    let a = p("sftp://srv/ruta");
    assert_eq!((a.scheme.as_str(), a.user.as_deref(), a.host.as_str(), a.share.as_deref()), ("sftp", None, "srv", Some("ruta")));
    assert_eq!(p("nas").to_url(), "smb://nas");
    assert_eq!(p("nas").share, None);
    assert_eq!(p("192.168.1.10").to_url(), "smb://192.168.1.10");
    assert_eq!(p("smb://nas").share, None);
}

#[test]
fn normalize() {
    assert_eq!(p("SMB://Nas/Fotos/").to_url(), "smb://Nas/Fotos");
    assert_eq!(p("  smb://nas//fotos//2024/  ").to_url(), "smb://nas/fotos/2024");
    assert_eq!(p("smb://nas/mis%20fotos").share.as_deref(), Some("mis fotos"));
    assert_eq!(p("smb://nas/mis fotos").to_url(), "smb://nas/mis%20fotos");
}

#[test]
fn bad_addresses_rejected() {
    for bad in ["", "   ", "smb://", "smb:///fotos", "smb://na s/x", "smb://nas:abc/x", "smb://nas:0/x", "smb://nas/a/../b", "1smb://nas", "smb://nas/a\u{7}b", "smb://n\"as/x", "smb://nas;rm -rf/x", "smb://$(id)/x", "smb://nas/a%2Fb", "smb://nas/%00"] {
        assert_eq!(Address::parse(bad), Err(BAD_ADDRESS.to_string()), "{bad:?}");
    }
    assert_eq!(BAD_ADDRESS, "Escribí una dirección como smb://servidor/recurso");
}

#[test]
fn unc_and_url() {
    assert_eq!(p(r"\\srv01\compartido").to_url(), "smb://srv01/compartido");
    assert_eq!(p("smb://srv01/compartido").to_unc().as_deref(), Some(r"\\srv01\compartido"));
    assert_eq!(p("smb://srv01/compartido/a/b").to_unc().as_deref(), Some(r"\\srv01\compartido\a\b"));
    assert_eq!(p("afp://srv/vol").to_unc(), None);
    assert_eq!(p("smb://srv01/compartido").display(KeyStyle::Windows), r"\\srv01\compartido");
    assert_eq!(p("smb://srv01/compartido").display(KeyStyle::Mac), "smb://srv01/compartido");
    assert_eq!(p("afp://srv/vol").display(KeyStyle::Windows), "afp://srv/vol");
}

#[test]
fn sftp_user_port() {
    let a = p("sftp://ana@vps:2222/var/www");
    assert_eq!(a.user.as_deref(), Some("ana"));
    assert_eq!(a.host, "vps");
    assert_eq!(a.port, Some(2222));
    assert_eq!(a.share.as_deref(), Some("var"));
    assert_eq!(a.rest, vec!["www".to_string()]);
    assert_eq!(a.to_url(), "sftp://ana@vps:2222/var/www");
    assert_eq!(p("smb://[fe80::1]/x").to_url(), "smb://[fe80::1]/x");
}

#[test]
fn same_ignores_case() {
    assert!(p("smb://NAS/Fotos").same(&p(r"\\nas\fotos")));
    assert!(!p("smb://nas/fotos").same(&p("smb://nas/musica")));
    assert!(!p("smb://nas/fotos").same(&p("afp://nas/fotos")));
    assert!(p("smb://nas/fotos/a").same_share(&p("smb://NAS/fotos/b")));
}

#[test]
fn password_never_persisted() {
    let a = p("smb://ana:secreto@nas/x");
    assert_eq!(a.user.as_deref(), Some("ana"));
    assert_eq!(a.to_url(), "smb://ana@nas/x");
    assert!(!format!("{a:?}").contains("secreto"));
    let mut places = NetPlaces::default();
    places.add_recent(&a, 100);
    places.toggle_favorite(&a, None);
    let text = places.write();
    assert!(!text.contains("secreto"), "{text}");
    let hand = "[[favorito]]\nnombre = \"x\"\ndireccion = \"smb://ana:secreto@nas/x\"\n\n[[reciente]]\ndireccion = \"smb://bob:otra@nas/y\"\nfecha = 5\n";
    let read = NetPlaces::parse(hand);
    assert_eq!(read.favorites[0].address, "smb://ana@nas/x");
    assert_eq!(read.recents[0].address, "smb://bob@nas/y");
    assert!(!read.write().contains("secreto") && !read.write().contains("otra"));
}

#[test]
fn error_messages() {
    let h = || "nas".to_string();
    let cases = [
        (NetError::NotFound { host: h() }, "No se encuentra el servidor \"nas\""),
        (NetError::BadCredentials, "Usuario o contraseña incorrectos"),
        (NetError::NoShare { host: h(), share: "fotos".into() }, "El recurso \"fotos\" no existe en \"nas\""),
        (NetError::Timeout { host: h() }, "Se agotó el tiempo esperando a \"nas\""),
        (NetError::CredentialConflict { host: h() }, "Ya hay una conexión con \"nas\" con otro usuario"),
        (NetError::OpenFiles { by: Some("Word".into()) }, "lo está usando Word"),
        (NetError::OpenFiles { by: None }, "lo está usando otro programa"),
        (NetError::Unsupported { scheme: "sftp".into() }, "En este sistema no se puede conectar a direcciones sftp://"),
        (NetError::NoGvfs, "Hace falta gvfs para conectarse a servidores"),
        (NetError::Other("La letra ya está en uso".into()), "La letra ya está en uso"),
    ];
    for (e, text) in cases {
        assert_eq!(e.message(), text);
    }
    assert!(!NetError::NeedsCredentials { user: true, domain: false }.message().is_empty());
    assert!(!NetError::Cancelled.message().is_empty());
}

#[test]
fn timeout_abandons_hung_call() {
    let t0 = Instant::now();
    let r = with_timeout(Duration::from_millis(50), "nas", || {
        std::thread::sleep(Duration::from_secs(5));
        Ok(())
    });
    assert!(t0.elapsed() < Duration::from_secs(1));
    assert_eq!(r, Err(NetError::Timeout { host: "nas".into() }));
    assert_eq!(with_timeout(Duration::from_secs(5), "nas", || Ok(7)), Ok(7));
    assert_eq!(with_timeout(Duration::from_secs(5), "nas", || Err::<(), _>(NetError::BadCredentials)), Err(NetError::BadCredentials));
}

#[test]
fn remote_item_has_time_limit() {
    crate::sys::testing::install_fakes();
    let remote = std::path::PathBuf::from("/net/n5-item");
    crate::sys::testing::FakeNetwork::set_remote(&remote);
    fn slow(_: &std::path::Path) -> u32 {
        std::thread::sleep(Duration::from_secs(30));
        7
    }
    fn fast(_: &std::path::Path) -> u32 {
        7
    }
    assert_eq!(remote_bounded(&remote.join("a.jpg"), fast), 7);
    assert_eq!(remote_bounded(std::path::Path::new("/local-n5/a.jpg"), fast), 7);
    let t0 = Instant::now();
    assert_eq!(remote_bounded(&remote.join("b.jpg"), slow), 0);
    assert!(t0.elapsed() < Duration::from_secs(7));
}

#[test]
fn secret_wipe_zeroes() {
    let mut s = Secret::new(b"clave".to_vec());
    assert_eq!(s.expose(), b"clave");
    s.wipe();
    assert_eq!(s.expose(), &[0, 0, 0, 0, 0]);
    // `Secret` no implementa `Debug` (si lo hiciera, ganaría el método inherente con la cota).
    struct Probe<T>(T);
    trait NotDebug {
        fn is_debug(&self) -> bool {
            false
        }
    }
    impl<T> NotDebug for Probe<T> {}
    #[allow(dead_code)]
    impl<T: std::fmt::Debug> Probe<T> {
        fn is_debug(&self) -> bool {
            true
        }
    }
    assert!(!Probe(Secret::new(Vec::new())).is_debug());
    assert!(Probe(1u8).is_debug());
    let c = Credentials { user: "ana".into(), domain: None, password: Secret::new(b"secreto".to_vec()) };
    assert!(!format!("{c:?}").contains("secreto"));
    let o = ConnectOptions { credentials: Some(c), ..Default::default() };
    assert!(o.without_secret().credentials.is_none());
}

#[test]
fn bonjour_service_hosts() {
    let a = p("smb://MacBook%20Pro%20de%20Javier%20(2)._smb._tcp.local/fotos");
    assert_eq!(a.host, "MacBook Pro de Javier (2)._smb._tcp.local");
    assert_eq!(a.to_url(), "smb://MacBook%20Pro%20de%20Javier%20(2)._smb._tcp.local/fotos");
    assert_eq!(Address::parse(&a.to_url()), Ok(a));
    assert_eq!(host_address("MacBook Pro de Javier (2)"), "smb://MacBook%20Pro%20de%20Javier%20(2)._smb._tcp.local");
    assert_eq!(host_address("NAS-CASA"), "smb://NAS-CASA");
    // Sin el sufijo de Bonjour, un host con espacios sigue sin valer; ni con `/` o `@` adentro.
    for bad in ["smb://Mac%20Book/x", "smb://a%2Fb._smb._tcp.local/x", "smb://a%40b._smb._tcp.local", "smb://._smb._tcp.local"] {
        assert!(Address::parse(bad).is_err(), "{bad}");
    }
}

/// M1: después del primer elemento que vence, los demás de ese servidor no esperan.
#[test]
fn remote_budget_stops_after_first_timeout() {
    crate::sys::testing::install_fakes();
    let remote = std::path::PathBuf::from("/net-r1budget");
    crate::sys::testing::FakeNetwork::set_remote(&remote);
    fn slow(_: &std::path::Path) -> u32 {
        std::thread::sleep(Duration::from_secs(30));
        7
    }
    let mut b = RemoteBudget::with_limit(Duration::from_millis(50));
    let t0 = Instant::now();
    for i in 0..20 {
        assert_eq!(b.read(&remote.join(format!("{i}.jpg")), slow), 0);
    }
    assert!(t0.elapsed() < Duration::from_millis(500), "{:?}", t0.elapsed());
    fn fast(_: &std::path::Path) -> u32 {
        7
    }
    assert_eq!(b.read(std::path::Path::new("/local-r1budget/a"), fast), 7);
}

/// M11: si la llamada entra en pánico no se informa como «Se agotó el tiempo».
#[test]
fn panic_is_not_a_timeout() {
    let r = with_timeout(Duration::from_secs(5), "nas", || -> Result<(), NetError> { panic!("falla adentro") });
    assert!(matches!(r, Err(NetError::Other(_))), "{r:?}");
}

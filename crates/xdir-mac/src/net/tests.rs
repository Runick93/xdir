use std::path::PathBuf;

use xdir_core::net::NetError;
use xdir_core::sys::HostEvent;

use super::*;

#[test]
fn mac_error_codes() {
    let h = "nas";
    assert_eq!(mac_error(80, h, "fotos"), NetError::BadCredentials);
    assert_eq!(mac_error(2, h, "fotos"), NetError::NoShare { host: "nas".into(), share: "fotos".into() });
    for c in [65, 51, 61] {
        assert_eq!(mac_error(c, h, "fotos"), NetError::NotFound { host: "nas".into() });
    }
    assert_eq!(mac_error(60, h, "fotos"), NetError::Timeout { host: "nas".into() });
    assert_eq!(mac_error(89, h, ""), NetError::Cancelled);
    assert_eq!(mac_error(-128, h, ""), NetError::Cancelled);
    assert_eq!(mac_error(5, h, ""), NetError::Other("error 5".into()));
}

#[test]
fn remote_roots_filters_types() {
    let m = vec![
        ("apfs".to_string(), PathBuf::from("/")),
        ("smbfs".to_string(), PathBuf::from("/Volumes/fotos")),
        ("afpfs".to_string(), PathBuf::from("/Volumes/vol")),
        ("nfs".to_string(), PathBuf::from("/Volumes/n")),
        ("webdav".to_string(), PathBuf::from("/Volumes/w")),
        ("hfs".to_string(), PathBuf::from("/Volumes/usb")),
    ];
    assert_eq!(remote_roots(&m), ["/Volumes/fotos", "/Volumes/vol", "/Volumes/n", "/Volumes/w"].map(PathBuf::from));
}

#[test]
fn bonjour_dedupes_services() {
    let mut h = BonjourHosts::default();
    assert_eq!(h.apply(0, Change::Added("nas".into())), Some(HostEvent::Up("nas".into())));
    assert_eq!(h.apply(1, Change::Added("nas".into())), None);
    assert_eq!(h.apply(1, Change::Added("mac".into())), Some(HostEvent::Up("mac".into())));
    assert_eq!(h.apply(0, Change::Removed("nas".into())), None);
    assert_eq!(h.apply(1, Change::Removed("nas".into())), Some(HostEvent::Down("nas".into())));
    assert_eq!(h.apply(1, Change::Removed("nas".into())), None);
}

#[test]
fn plist_has_local_network_keys() {
    let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/package-mac.sh")).unwrap();
    assert!(script.contains("<key>NSLocalNetworkUsageDescription</key><string>xdir muestra las computadoras de tu red local en \"Red\".</string>"));
    assert!(script.contains("<key>NSBonjourServices</key>"));
    assert!(script.contains("<string>_smb._tcp</string>") && script.contains("<string>_afpovertcp._tcp</string>"));
}

#[test]
fn local_volumes_not_network_ejectable() {
    use crate::shell::macos::ejectable_volume;
    // Disco de arranque: interno, local.
    assert!(!ejectable_volume(Some(false), Some(false), Some(true), Some(true)));
    // Imagen de disco (hdiutil): expulsable, local.
    assert!(ejectable_volume(Some(true), Some(false), Some(false), Some(true)));
    // Volumen de red: no es local; su ⏏ sale de `Network::connections`.
    assert!(!ejectable_volume(Some(false), Some(false), None, Some(false)));
    assert!(!ejectable_volume(Some(true), None, Some(false), Some(false)));
    // `/` sigue sin aparecer.
    assert!(crate::shell::macos::ejectable(&[PathBuf::from("/")]).is_empty());
}

#[test]
fn is_remote_without_io() {
    let n = MacNetwork { remote: std::sync::RwLock::new(vec![PathBuf::from("/Volumes/fotos-red")]), denied: Default::default() };
    assert!(n.is_remote(std::path::Path::new("/Volumes/fotos-red/a")));
    assert!(!n.is_remote(std::path::Path::new("/Volumes/fotos-redx")));
    assert!(n.supports("smb") && n.supports("afp") && !n.supports("sftp"));
}

#[test]
fn clean_address_drops_password() {
    assert_eq!(clean_address("smb://ana:secreto@nas.local/fotos"), "smb://ana@nas.local/fotos");
    assert_eq!(clean_address("weird://a:b@c d"), "weird://c d");
}

/// Prueba manual: `XDIR_NAS_URL=smb://nas.local/fotos cargo test -p xdir-mac mac_connect_nas_manual
/// -- --ignored` (la contraseña, del Llavero). Conecta, lista y desconecta.
#[test]
#[ignore]
fn mac_connect_nas_manual() {
    let url = std::env::var("XDIR_NAS_URL").expect("XDIR_NAS_URL");
    let addr = xdir_core::net::Address::parse(&url).unwrap();
    let n = MacNetwork::new();
    let local = n.connect(Handle(0), &addr, Default::default(), &AtomicBool::new(false)).unwrap();
    assert!(n.is_remote(&local));
    assert!(n.connections().unwrap().iter().any(|c| local.starts_with(&c.local)));
    n.disconnect(&local, false).unwrap();
}

/// I2: la lista de discos no hace `stat` de un volumen de red.
#[test]
fn drives_skip_stat_of_network_volumes() {
    use crate::fs::macos::keep_volume;
    assert!(keep_volume(true, || panic!("no se mira un volumen de red")));
    assert!(keep_volume(false, || true));
    assert!(!keep_volume(false, || false));
}

/// M7: el permiso de red local negado vuelve a darse si el navegador queda listo.
#[test]
fn local_network_denied_follows_state() {
    use super::bonjour::denied_after;
    assert!(denied_after(4, Some(-65570), false));
    assert!(denied_after(2, Some(-65570), false));
    assert!(!denied_after(1, None, true), "listo: ya hay permiso");
    assert!(denied_after(4, Some(-1), true), "otro error no cambia");
    assert!(!denied_after(4, Some(-1), false));
}

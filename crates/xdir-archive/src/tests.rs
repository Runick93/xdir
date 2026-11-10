use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use xdir_core::archive::safe_entry_path;

use super::*;

static N: AtomicU32 = AtomicU32::new(0);

/// Carpeta temporal propia de cada test (se borra al terminar).
struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Tmp {
        let p = std::env::temp_dir().join(format!("xdir-archive-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn p(&self, s: &str) -> PathBuf {
        self.0.join(s)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn svc() -> Service {
    Service { tar_gz: true }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

struct Raw<'a> {
    name: &'a [u8],
    flags: u16,
    data: &'a [u8],
    eattr: u32,
    /// Tamaño declarado en zip64 (solo central, sin datos).
    big: Option<u64>,
}

fn raw<'a>(name: &'a [u8], data: &'a [u8]) -> Raw<'a> {
    Raw { name, flags: 0, data, eattr: 0, big: None }
}

/// Zip mínimo armado a mano (método store).
fn raw_zip(entries: &[Raw]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let off = out.len() as u32;
        let crc = crc32(e.data);
        let n = e.data.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0]);
        out.extend(e.flags.to_le_bytes());
        out.extend([0, 0, 0, 0, 0x21, 0]);
        out.extend(crc.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend((e.name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(e.name);
        out.extend(e.data);

        let extra: Vec<u8> = match e.big {
            Some(b) => [&[1u8, 0, 16, 0][..], &b.to_le_bytes(), &b.to_le_bytes()].concat(),
            None => vec![],
        };
        let (cs, us) = if e.big.is_some() { (u32::MAX, u32::MAX) } else { (n, n) };
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([0x1E, 3, 20, 0]);
        central.extend(e.flags.to_le_bytes());
        central.extend([0, 0, 0, 0, 0x21, 0]);
        central.extend(crc.to_le_bytes());
        central.extend(cs.to_le_bytes());
        central.extend(us.to_le_bytes());
        central.extend((e.name.len() as u16).to_le_bytes());
        central.extend((extra.len() as u16).to_le_bytes());
        central.extend([0, 0, 0, 0, 0, 0]);
        central.extend(e.eattr.to_le_bytes());
        central.extend(off.to_le_bytes());
        central.extend(e.name);
        central.extend(&extra);
    }
    let cd_off = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd_off.to_le_bytes());
    out.extend([0, 0]);
    out
}

/// Extrae todo con `extract_all` (enlaces al final, una pasada).
fn extract_all(zip: &Path, root: &Path) -> Result<(), Error> {
    svc().extract_all(zip, root, &mut |_| Flow::Continue).map(|_| ())
}

/// Extrae entrada por entrada, en el orden del archivo (como un llamador torpe).
#[cfg(unix)]
fn extract_each(zip: &Path, root: &Path) -> Result<(), Error> {
    for e in svc().list(zip)? {
        svc().extract(zip, &e, root, None, &mut |_| Flow::Continue)?;
    }
    Ok(())
}

fn tree(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<(String, Option<Vec<u8>>)>) {
        let mut v: Vec<_> = fs::read_dir(d).unwrap().map(|e| e.unwrap().path()).collect();
        v.sort();
        for p in v {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                out.push((format!("{rel}/"), None));
                walk(root, &p, out);
            } else {
                out.push((rel, Some(fs::read(&p).unwrap())));
            }
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out
}

fn sample(t: &Tmp) -> (PathBuf, Vec<(PathBuf, String)>) {
    let src = t.p("src");
    fs::create_dir_all(src.join("sub/vacia")).unwrap();
    fs::write(src.join("a.txt"), b"hola").unwrap();
    fs::write(src.join("b.bin"), vec![7u8; 200_000]).unwrap();
    fs::write(src.join("sub/c.txt"), b"").unwrap();
    let items = vec![
        (src.clone(), "src/".to_string()),
        (src.join("a.txt"), "src/a.txt".to_string()),
        (src.join("b.bin"), "src/b.bin".to_string()),
        (src.join("sub"), "src/sub/".to_string()),
        (src.join("sub/c.txt"), "src/sub/c.txt".to_string()),
        (src.join("sub/vacia"), "src/sub/vacia/".to_string()),
    ];
    (src, items)
}

#[test]
fn zip_roundtrip() {
    let t = Tmp::new();
    let (src, items) = sample(&t);
    let z = t.p("out.zip");
    svc().create(&z, &items, &mut |_| Flow::Continue).unwrap();
    let list = svc().list(&z).unwrap();
    let names: Vec<_> = list.iter().map(|e| (e.name.as_str(), e.kind)).collect();
    assert!(names.contains(&("src/sub/vacia/", ItemKind::Dir)));
    assert!(names.contains(&("src/b.bin", ItemKind::File)));
    assert_eq!(list.iter().find(|e| e.name == "src/b.bin").unwrap().size, 200_000);
    assert!(list.iter().find(|e| e.name == "src/a.txt").unwrap().modified > 1_000_000_000);
    let dst = t.p("dst");
    fs::create_dir(&dst).unwrap();
    extract_all(&z, &dst).unwrap();
    assert_eq!(tree(&src), tree(&dst.join("src")));
    // No pisa.
    let a_txt = list.iter().find(|e| e.name == "src/a.txt").unwrap();
    let again = svc().extract(&z, a_txt, &dst, None, &mut |_| Flow::Continue);
    assert_eq!(again.unwrap_err().kind, ErrKind::Exists);
    // Conservar ambos: otro nombre final, siempre adentro.
    svc().extract(&z, a_txt, &dst, Some("a 2.txt".as_ref()), &mut |_| Flow::Continue).unwrap();
    assert_eq!(fs::read(dst.join("src/a 2.txt")).unwrap(), b"hola");
    for bad in ["../x.txt", "a/b.txt", "..", "C:x", "a\\b"] {
        let r = svc().extract(&z, a_txt, &dst, Some(std::ffi::OsStr::new(bad)), &mut |_| Flow::Continue);
        assert!(matches!(r.unwrap_err().kind, ErrKind::Other(_)), "{bad}");
    }
}

#[test]
fn create_refuses_to_overwrite_and_bad_names() {
    let t = Tmp::new();
    let (_, items) = sample(&t);
    let z = t.p("out.zip");
    fs::write(&z, b"x").unwrap();
    assert_eq!(svc().create(&z, &items, &mut |_| Flow::Continue).unwrap_err().kind, ErrKind::Exists);
    assert_eq!(fs::read(&z).unwrap(), b"x");
    let z2 = t.p("o2.zip");
    let bad = vec![(t.p("src/a.txt"), "../a.txt".to_string())];
    assert!(svc().create(&z2, &bad, &mut |_| Flow::Continue).is_err());
    assert!(!z2.exists());
}

#[test]
fn zip_progress_and_stop() {
    let t = Tmp::new();
    let (_, items) = sample(&t);
    let z = t.p("out.zip");
    let mut seen = vec![];
    svc().create(&z, &items, &mut |b| {
        seen.push(b);
        Flow::Continue
    })
    .unwrap();
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(*seen.last().unwrap(), 200_004);

    let z2 = t.p("stop.zip");
    let err = svc().create(&z2, &items, &mut |b| if b > 100 { Flow::Stop } else { Flow::Continue }).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
    assert!(!z2.exists());

    // Extraer: avance acumulado y Stop borra lo escrito.
    let list = svc().list(&z).unwrap();
    let big = list.iter().find(|e| e.name == "src/b.bin").unwrap();
    let root = t.p("o1");
    fs::create_dir(&root).unwrap();
    let mut last = 0;
    svc().extract(&z, big, &root, None, &mut |b| {
        last = b;
        Flow::Continue
    })
    .unwrap();
    assert_eq!(last, 200_000);
    assert_eq!(fs::metadata(root.join("src/b.bin")).unwrap().len(), 200_000);
    let err = svc().extract(&z, big, &root, Some("big2.bin".as_ref()), &mut |_| Flow::Stop).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
    assert!(!root.join("src/big2.bin").exists());

    // extract_all: avance total y cancelar.
    let all = t.p("o2");
    fs::create_dir(&all).unwrap();
    let mut seen = vec![];
    svc().extract_all(&z, &all, &mut |b| {
        seen.push(b);
        Flow::Continue
    })
    .unwrap();
    assert_eq!(*seen.last().unwrap(), 200_004);
    assert_eq!(tree(&t.p("src")), tree(&all.join("src")));
    let all2 = t.p("o3");
    fs::create_dir(&all2).unwrap();
    let err = svc().extract_all(&z, &all2, &mut |_| Flow::Stop).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
}

#[test]
fn zip_cp437_names() {
    let t = Tmp::new();
    let z = t.p("viejo.zip");
    // 0xA5 = «Ñ» en CP437, sin la marca UTF-8 (bit 11).
    fs::write(&z, raw_zip(&[raw(&[0xA5, b'.', b't', b'x', b't'], b"x")])).unwrap();
    let list = svc().list(&z).unwrap();
    assert_eq!(list[0].name, "Ñ.txt");
    // Con la marca UTF-8, tal cual.
    let z2 = t.p("nuevo.zip");
    fs::write(&z2, raw_zip(&[Raw { flags: 1 << 11, ..raw("ñandú.txt".as_bytes(), b"x") }])).unwrap();
    assert_eq!(svc().list(&z2).unwrap()[0].name, "ñandú.txt");
}

#[test]
fn zip_password_error() {
    let t = Tmp::new();
    let z = t.p("clave.zip");
    fs::write(&z, raw_zip(&[Raw { flags: 1, ..raw(b"secreto.txt", b"datos cifrados") }])).unwrap();
    let list = svc().list(&z).unwrap();
    assert_eq!(list[0].name, "secreto.txt");
    let err = svc().extract(&z, &list[0], &t.0, None, &mut |_| Flow::Continue).unwrap_err();
    assert_eq!(err.kind, ErrKind::Other("Este zip tiene contraseña".into()));
    assert!(!t.p("secreto.txt").exists());
}

#[test]
fn zip_unsupported_method() {
    let t = Tmp::new();
    let z = t.p("raro.zip");
    let mut b = raw_zip(&[raw(b"a.txt", b"x")]);
    // Método 14 (LZMA) en la cabecera local y en la central.
    b[8] = 14;
    let c = b.windows(4).position(|w| w == 0x0201_4b50u32.to_le_bytes()).unwrap();
    b[c + 10] = 14;
    fs::write(&z, b).unwrap();
    let list = svc().list(&z).unwrap();
    let err = svc().extract(&z, &list[0], &t.0, None, &mut |_| Flow::Continue).unwrap_err();
    assert_eq!(err.kind, ErrKind::Other("Formato no soportado".into()));
    assert!(!t.p("a.txt").exists());
}

#[test]
fn zip64_large_entry_listed() {
    let t = Tmp::new();
    let z = t.p("grande.zip");
    let five_gb = 5u64 << 30;
    fs::write(&z, raw_zip(&[Raw { big: Some(five_gb), ..raw(b"enorme.bin", b"") }])).unwrap();
    let list = svc().list(&z).unwrap();
    assert_eq!(list[0].name, "enorme.bin");
    assert_eq!(list[0].size, five_gb);
}

#[test]
fn zip_malicious_names_write_nothing_outside() {
    let t = Tmp::new();
    for (i, name) in ["../evil.txt", "a/../../evil.txt", "/abs/evil.txt", "C:\\evil.txt", "\\\\srv\\x\\evil.txt", "..\\evil.txt"]
        .iter()
        .enumerate()
    {
        let z = t.p(&format!("mal{i}.zip"));
        fs::write(&z, raw_zip(&[raw(name.as_bytes(), b"pwn")])).unwrap();
        let root = t.p(&format!("root{i}/in"));
        fs::create_dir_all(&root).unwrap();
        let list = svc().list(&z).unwrap();
        assert_eq!(safe_entry_path(&list[0].name), None, "{name}");
        let err = svc().extract(&z, &list[0], &root, None, &mut |_| Flow::Continue).unwrap_err();
        assert!(matches!(err.kind, ErrKind::Other(_)), "{name}");
        assert!(extract_all(&z, &root).is_err());
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "{name}");
        assert!(!t.p(&format!("root{i}/evil.txt")).exists());
        assert!(!t.p("evil.txt").exists());
    }
}

#[cfg(unix)]
fn link_entry<'a>(name: &'a [u8], target: &'a [u8]) -> Raw<'a> {
    Raw { eattr: 0o120777 << 16, ..raw(name, target) }
}

#[cfg(unix)]
#[test]
fn zip_symlinks_outside_are_refused() {
    let t = Tmp::new();
    let cases: [(&[u8], &[u8]); 3] = [(b"l", b"/etc/passwd"), (b"l", b"../fuera"), (b"d/l", b"../../fuera")];
    for (i, (n, target)) in cases.iter().enumerate() {
        let z = t.p(&format!("l{i}.zip"));
        fs::write(&z, raw_zip(&[link_entry(n, target)])).unwrap();
        let root = t.p(&format!("r{i}"));
        fs::create_dir_all(&root).unwrap();
        let err = extract_all(&z, &root).unwrap_err();
        assert!(matches!(err.kind, ErrKind::Other(_)));
        assert!(fs::symlink_metadata(root.join(String::from_utf8_lossy(n).as_ref())).is_err());
    }
    // Enlace interno: sí.
    let z = t.p("ok.zip");
    fs::write(&z, raw_zip(&[raw(b"a.txt", b"x"), link_entry(b"d/l", b"../a.txt")])).unwrap();
    let root = t.p("rok");
    fs::create_dir_all(&root).unwrap();
    extract_all(&z, &root).unwrap();
    assert_eq!(fs::read_link(root.join("d/l")).unwrap(), Path::new("../a.txt"));
}

#[cfg(unix)]
#[test]
fn zip_symlink_chain_cannot_escape() {
    // d/l -> .. (raíz) y luego x -> d/l/.. : por el texto queda adentro, en el disco sale.
    let t = Tmp::new();
    let z = t.p("cadena.zip");
    fs::write(&z, raw_zip(&[link_entry(b"d/l", b".."), link_entry(b"x", b"d/l/..")])).unwrap();
    let root = t.p("raiz/in");
    fs::create_dir_all(&root).unwrap();
    let err = extract_all(&z, &root).unwrap_err();
    assert!(matches!(err.kind, ErrKind::Other(_)));
    assert!(fs::symlink_metadata(root.join("x")).is_err());
}

#[test]
fn zip_corrupt_is_clear_error() {
    let t = Tmp::new();
    let z = t.p("roto.zip");
    fs::write(&z, b"esto no es un zip").unwrap();
    assert_eq!(svc().list(&z).unwrap_err().kind, ErrKind::Other("El zip está dañado".into()));
    assert_eq!(svc().list(&t.p("no.rar")).unwrap_err().kind, ErrKind::Other("Formato no soportado".into()));
    let win = Service { tar_gz: false };
    assert_eq!(win.list(&t.p("a.tar.gz")).unwrap_err().kind, ErrKind::Other("Formato no soportado".into()));
    assert_eq!(win.formats().len(), 1);
}

#[cfg(unix)]
fn tar_gz(path: &Path, build: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<fs::File>>)) {
    let f = fs::File::create(path).unwrap();
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::default()));
    build(&mut b);
    b.into_inner().unwrap().finish().unwrap();
}

#[cfg(unix)]
fn tar_entry(b: &mut tar::Builder<flate2::write::GzEncoder<fs::File>>, name: &[u8], ty: tar::EntryType, link: Option<&[u8]>, data: &[u8]) {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(ty);
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_mtime(1_700_000_000);
    // Se escribe el nombre crudo para poder armar rutas que la crate `tar` no deja.
    h.as_old_mut().name[..name.len()].copy_from_slice(name);
    if let Some(l) = link {
        h.as_old_mut().linkname[..l.len()].copy_from_slice(l);
    }
    h.set_cksum();
    b.append(&h, data).unwrap();
}

#[cfg(unix)]
#[test]
fn targz_roundtrip() {
    let t = Tmp::new();
    let z = t.p("a.tar.gz");
    tar_gz(&z, |b| {
        tar_entry(b, b"d/", tar::EntryType::Directory, None, b"");
        tar_entry(b, b"d/a.txt", tar::EntryType::Regular, None, b"hola");
        tar_entry(b, b"d/enlace", tar::EntryType::Symlink, Some(b"a.txt"), b"");
        tar_entry(b, b"d/vacia/", tar::EntryType::Directory, None, b"");
    });
    let list = svc().list(&z).unwrap();
    assert_eq!(list.len(), 4);
    assert_eq!(list[2].kind, ItemKind::Symlink);
    assert_eq!(list[2].link.as_deref(), Some("a.txt"));
    assert_eq!(list[1].size, 4);
    assert_eq!(list[1].modified, 1_700_000_000);
    let dst = t.p("dst");
    fs::create_dir(&dst).unwrap();
    extract_all(&z, &dst).unwrap();
    assert_eq!(fs::read(dst.join("d/a.txt")).unwrap(), b"hola");
    assert_eq!(fs::read_link(dst.join("d/enlace")).unwrap(), Path::new("a.txt"));
    assert!(dst.join("d/vacia").is_dir());
    // El tar.gz no se crea.
    assert_eq!(svc().create(&t.p("n.tar.gz"), &[], &mut |_| Flow::Continue).unwrap_err().kind, ErrKind::Other("Formato no soportado".into()));
}

#[cfg(unix)]
#[test]
fn targz_malicious_entries_are_refused() {
    let t = Tmp::new();
    let z = t.p("mal.tar.gz");
    tar_gz(&z, |b| {
        tar_entry(b, b"../evil.txt", tar::EntryType::Regular, None, b"pwn");
        tar_entry(b, b"/abs/evil.txt", tar::EntryType::Regular, None, b"pwn");
        tar_entry(b, b"l", tar::EntryType::Symlink, Some(b"/etc/passwd"), b"");
        tar_entry(b, b"m", tar::EntryType::Symlink, Some(b"../../x"), b"");
    });
    let root = t.p("r/in");
    fs::create_dir_all(&root).unwrap();
    for e in svc().list(&z).unwrap() {
        let r = svc().extract(&z, &e, &root, None, &mut |_| Flow::Continue);
        assert!(r.is_err(), "{}", e.name);
    }
    assert!(extract_all(&z, &root).is_err());
    assert!(!t.p("r/evil.txt").exists());
    assert!(fs::symlink_metadata(root.join("l")).is_err());
    assert!(fs::symlink_metadata(root.join("m")).is_err());
}

#[test]
fn error_messages_and_kinds() {
    assert_eq!(kind_of(&io::Error::from(io::ErrorKind::PermissionDenied)), ErrKind::Denied);
    assert_eq!(kind_of(&io::Error::from(io::ErrorKind::NotFound)), ErrKind::NotFound);
    assert_eq!(kind_of(&io::Error::from(io::ErrorKind::StorageFull)), ErrKind::DiskFull);
    assert_eq!(Error::other("x", Path::new("a")).message(), "x");
    assert_eq!(Error { kind: ErrKind::DiskFull, path: PathBuf::new() }.message(), "El disco está lleno");
}

// ---- Ronda de arreglos 1: regresiones de seguridad ----

#[cfg(unix)]
fn outside_marker(t: &Tmp) -> PathBuf {
    t.p("raiz/pwned.txt")
}

#[cfg(unix)]
#[test]
fn reg_zip_redirected_symlink_cannot_write_outside() {
    // l -> b/.. (b todavía no existe), b -> ., y luego un archivo l/pwned.txt.
    let t = Tmp::new();
    let z = t.p("r.zip");
    fs::write(&z, raw_zip(&[link_entry(b"l", b"b/.."), link_entry(b"b", b"."), raw(b"l/pwned.txt", b"pwn")])).unwrap();
    let root = t.p("raiz/in");
    fs::create_dir_all(&root).unwrap();
    assert!(extract_all(&z, &root).is_err());
    assert!(!outside_marker(&t).exists());
}

#[cfg(unix)]
#[test]
fn reg_zip_redirected_symlink_folder_entry() {
    let t = Tmp::new();
    let z = t.p("r.zip");
    fs::write(&z, raw_zip(&[link_entry(b"l", b"b/.."), link_entry(b"b", b"."), raw(b"l/sub/", b"")])).unwrap();
    let root = t.p("raiz/in");
    fs::create_dir_all(&root).unwrap();
    assert!(extract_all(&z, &root).is_err());
    assert!(!t.p("raiz/sub").exists());
}

#[cfg(unix)]
#[test]
fn reg_targz_redirected_symlink_cannot_write_outside() {
    let t = Tmp::new();
    let z = t.p("r.tar.gz");
    tar_gz(&z, |b| {
        tar_entry(b, b"l", tar::EntryType::Symlink, Some(b"b/.."), b"");
        tar_entry(b, b"b", tar::EntryType::Symlink, Some(b"."), b"");
        tar_entry(b, b"l/pwned.txt", tar::EntryType::Regular, None, b"pwn");
    });
    let root = t.p("raiz/in");
    fs::create_dir_all(&root).unwrap();
    assert!(extract_all(&z, &root).is_err());
    assert!(!outside_marker(&t).exists());
}

#[cfg(unix)]
#[test]
fn reg_existing_symlink_in_destination_is_not_traversed() {
    let t = Tmp::new();
    let z = t.p("r.zip");
    fs::write(&z, raw_zip(&[raw(b"enlace/pwned.txt", b"pwn"), raw(b"enlace/sub/", b"")])).unwrap();
    let root = t.p("raiz/in");
    let fuera = t.p("fuera");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&fuera).unwrap();
    std::os::unix::fs::symlink(&fuera, root.join("enlace")).unwrap();
    assert!(extract_all(&z, &root).is_err());
    assert!(!fuera.join("pwned.txt").exists());
    assert!(!fuera.join("sub").exists());
}

#[cfg(unix)]
#[test]
fn reg_each_entry_order_also_refused() {
    let t = Tmp::new();
    let z = t.p("r.zip");
    fs::write(&z, raw_zip(&[link_entry(b"l", b"b/.."), link_entry(b"b", b"."), raw(b"l/pwned.txt", b"pwn")])).unwrap();
    let root = t.p("raiz/in");
    fs::create_dir_all(&root).unwrap();
    assert!(extract_each(&z, &root).is_err());
    assert!(!outside_marker(&t).exists());
    // b primero y después l -> b/.. : en el disco sale de la raíz.
    let z2 = t.p("r2.zip");
    fs::write(&z2, raw_zip(&[link_entry(b"b", b"."), link_entry(b"l", b"b/..")])).unwrap();
    let root2 = t.p("raiz2/in");
    fs::create_dir_all(&root2).unwrap();
    assert!(extract_each(&z2, &root2).is_err());
    assert!(fs::symlink_metadata(root2.join("l")).is_err());
    assert!(extract_all(&z2, &t.p("raiz2/in")).is_err());
}

#[cfg(unix)]
#[test]
fn reg_targz_folder_entry_and_existing_symlink() {
    let t = Tmp::new();
    let z = t.p("r.tar.gz");
    tar_gz(&z, |b| {
        tar_entry(b, b"l", tar::EntryType::Symlink, Some(b"b/.."), b"");
        tar_entry(b, b"b", tar::EntryType::Symlink, Some(b"."), b"");
        tar_entry(b, b"l/sub/", tar::EntryType::Directory, None, b"");
    });
    let root = t.p("raiz/in");
    fs::create_dir_all(&root).unwrap();
    assert!(extract_all(&z, &root).is_err());
    assert!(!t.p("raiz/sub").exists());
    let z2 = t.p("r2.tar.gz");
    tar_gz(&z2, |b| tar_entry(b, b"enlace/pwned.txt", tar::EntryType::Regular, None, b"pwn"));
    let root2 = t.p("r2/in");
    let fuera = t.p("fuera");
    fs::create_dir_all(&root2).unwrap();
    fs::create_dir_all(&fuera).unwrap();
    std::os::unix::fs::symlink(&fuera, root2.join("enlace")).unwrap();
    assert!(extract_all(&z2, &root2).is_err());
    assert!(!fuera.join("pwned.txt").exists());
}

#[test]
fn zip_long_symlink_target_is_rejected() {
    let t = Tmp::new();
    let z = t.p("largo.zip");
    let target = vec![b'a'; 5000];
    fs::write(&z, raw_zip(&[Raw { eattr: 0o120777 << 16, ..raw(b"l", &target) }])).unwrap();
    let list = svc().list(&z).unwrap();
    assert_eq!(list[0].link, None);
    let root = t.p("r");
    fs::create_dir(&root).unwrap();
    assert!(svc().extract(&z, &list[0], &root, None, &mut |_| Flow::Continue).is_err());
}

#[test]
fn missing_entry_is_not_found_and_declared_total() {
    let t = Tmp::new();
    let (_, items) = sample(&t);
    let z = t.p("out.zip");
    svc().create(&z, &items, &mut |_| Flow::Continue).unwrap();
    assert_eq!(svc().declared_total(&z).unwrap(), 200_004);
    let ghost = ArchiveEntry { index: 99, name: "x".into(), size: 0, kind: ItemKind::File, link: None, modified: 0 };
    let err = svc().extract(&z, &ghost, &t.0, None, &mut |_| Flow::Continue).unwrap_err();
    assert_eq!(err.kind, ErrKind::NotFound);
}

#[test]
fn create_cancels_with_many_empty_items_and_normalizes_names() {
    let t = Tmp::new();
    let d = t.p("d");
    fs::create_dir(&d).unwrap();
    let items: Vec<_> = (0..50).map(|i| (d.clone(), format!("d{i}/"))).collect();
    let z = t.p("o.zip");
    let mut calls = 0;
    let err = svc().create(&z, &items, &mut |_| {
        calls += 1;
        if calls >= 3 { Flow::Stop } else { Flow::Continue }
    })
    .unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
    assert!(!z.exists());
    for bad in ["a\\b", "./a", "a/./b", "a/../b", "/a"] {
        let z = t.p("n.zip");
        let r = svc().create(&z, &[(d.clone(), format!("{bad}/"))], &mut |_| Flow::Continue);
        assert!(r.is_err(), "{bad}");
        assert!(!z.exists());
    }
    let z = t.p("n2.zip");
    svc().create(&z, &[(d.clone(), "a//b/".to_string())], &mut |_| Flow::Continue).unwrap();
    assert_eq!(svc().list(&z).unwrap()[0].name, "a/b/");
}

#[cfg(unix)]
#[test]
fn create_refuses_fifos() {
    let t = Tmp::new();
    let f = t.p("fifo");
    assert!(std::process::Command::new("mkfifo").arg(&f).status().unwrap().success());
    let z = t.p("o.zip");
    let err = svc().create(&z, &[(f, "fifo".to_string())], &mut |_| Flow::Continue).unwrap_err();
    assert_eq!(err.kind, ErrKind::Other("No se puede comprimir este tipo de elemento".into()));
    assert!(!z.exists());
}

#[cfg(unix)]
#[test]
fn targz_list_is_cancellable_and_extract_all_one_pass() {
    let t = Tmp::new();
    let z = t.p("a.tar.gz");
    tar_gz(&z, |b| {
        tar_entry(b, b"a.txt", tar::EntryType::Regular, None, b"hola");
        tar_entry(b, b"b.txt", tar::EntryType::Regular, None, b"mundo");
    });
    let err = svc().list_cancellable(&z, &mut || Flow::Stop).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
    let root = t.p("r");
    fs::create_dir(&root).unwrap();
    let err = svc().extract_all(&z, &root, &mut |_| Flow::Stop).unwrap_err();
    assert_eq!(err.kind, ErrKind::Cancelled);
    let root2 = t.p("r2");
    fs::create_dir(&root2).unwrap();
    assert_eq!(svc().extract_all(&z, &root2, &mut |_| Flow::Continue).unwrap(), 0);
    assert_eq!(fs::read(root2.join("b.txt")).unwrap(), b"mundo");
    assert_eq!(svc().declared_total(&z).unwrap(), 9);
}

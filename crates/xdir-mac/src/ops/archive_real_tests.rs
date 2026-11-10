//! Extraer de punta a punta con las primitivas de la Mac y `xdir_archive::Service` (en carpetas
//! temporales; nunca la Papelera: no hay conflictos con «Reemplazar»).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use xdir_core::sys::{Archives, NOT_ALLOWED};
use xdir_core::transfer::{Answer, Choice, Control, JobOutcome, JobSpec, Update, run_with};

use super::MacCopyPrims;

static N: AtomicU32 = AtomicU32::new(0);

fn tmp() -> PathBuf {
    let p = std::env::temp_dir().join(format!("xdir-zip-real-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p.canonicalize().unwrap()
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

/// (nombre, contenido, atributos externos) → zip mínimo (store).
fn raw_zip(entries: &[(&[u8], &[u8], u32)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data, eattr) in entries {
        let (off, crc, n) = (out.len() as u32, crc32(data), data.len() as u32);
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        out.extend(crc.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(*name);
        out.extend(*data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([0x1E, 3, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        central.extend(crc.to_le_bytes());
        central.extend(n.to_le_bytes());
        central.extend(n.to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend(eattr.to_le_bytes());
        central.extend(off.to_le_bytes());
        central.extend(*name);
    }
    let cd = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend([0, 0]);
    out
}

const LINK: u32 = 0o120777 << 16;

fn run(spec: JobSpec, choice: Choice) -> (JobOutcome, Vec<String>) {
    run_cb(spec, choice, || {})
}

/// Como `run`; `on_conflict` corre mientras el trabajo espera la respuesta.
fn run_cb(spec: JobSpec, choice: Choice, mut on_conflict: impl FnMut()) -> (JobOutcome, Vec<String>) {
    let ctl: Arc<Control> = Control::new();
    let (mut fin, mut asked) = (None, Vec::new());
    let svc = xdir_archive::Service { tar_gz: true };
    let c2 = ctl.clone();
    run_with(spec, &MacCopyPrims, &svc as &dyn Archives, &ctl, &mut |u| match u {
        Update::Conflict(_) => {
            on_conflict();
            c2.answer(Answer { choice, all: true })
        }
        Update::Confirm(q) => {
            asked.push(q);
            c2.confirm(true)
        }
        Update::Finished(o) => fin = Some(o),
        _ => {}
    });
    (fin.unwrap(), asked)
}

#[test]
fn real_link_through_user_link_after_keepboth_is_removed() {
    let t = tmp();
    let inn = t.join("in");
    fs::create_dir_all(&inn).unwrap();
    fs::create_dir_all(t.join("outside")).unwrap();
    fs::write(t.join("outside/secret"), b"s").unwrap();
    std::os::unix::fs::symlink("../outside", inn.join("b")).unwrap();
    let z = inn.join("m.zip");
    fs::write(&z, raw_zip(&[(b"b/", b"", 0), (b"b/secret", b"decoy", 0), (b"y", b"b/secret", LINK), (b"bien", b"b 2", LINK)])).unwrap();
    let (o, _) = run(JobSpec::Extract { archive: z, into_folder: false }, Choice::KeepBoth);
    assert!(fs::symlink_metadata(inn.join("y")).is_err(), "el enlace que sale quedó");
    assert_eq!(fs::read(inn.join("b 2/secret")).unwrap(), b"decoy");
    assert_eq!(fs::read_link(inn.join("bien")).unwrap(), Path::new("b 2"));
    assert!(o.errors.iter().any(|e| e.entry.as_deref() == Some("y") && e.message == NOT_ALLOWED), "{:?}", o.errors);
    assert_eq!(fs::read(t.join("outside/secret")).unwrap(), b"s");
    let _ = fs::remove_dir_all(&t);
}

#[test]
fn real_extract_into_single_top_folder() {
    let t = tmp();
    let z = t.join("carpeta.zip");
    fs::write(&z, raw_zip(&[(b"carpeta/", b"", 0), (b"carpeta/a.txt", b"a", 0), (b"__MACOSX/carpeta/._a.txt", b"x", 0)])).unwrap();
    let (o, _) = run(JobSpec::Extract { archive: z, into_folder: true }, Choice::Skip);
    assert_eq!(fs::read(t.join("carpeta/a.txt")).unwrap(), b"a");
    assert!(o.errors.is_empty(), "{:?}", o.errors);
    assert_eq!(o.result.select, vec![t.join("carpeta")]);
    let mut names: Vec<_> = fs::read_dir(&t).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["carpeta", "carpeta.zip"]);
    // La carpeta queda con los permisos de siempre (la oculta del motor es 0700).
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(t.join("carpeta")).unwrap().permissions().mode() & 0o777;
    assert_ne!(mode, 0o700, "{mode:o}");
    let _ = fs::remove_dir_all(&t);
}

#[test]
fn real_many_empty_entries_do_not_ask() {
    let t = tmp();
    let z = t.join("many.tar.gz");
    let f = fs::File::create(&z).unwrap();
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::default()));
    for i in 0..20_000 {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_size(0);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, format!("d/f{i:06}"), &b""[..]).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap();
    let (o, asked) = run(JobSpec::Extract { archive: z, into_folder: true }, Choice::Skip);
    assert!(asked.is_empty(), "{asked:?}");
    assert!(o.errors.is_empty());
    assert_eq!(fs::read_dir(t.join("many")).unwrap().count(), 20_000);
    let _ = fs::remove_dir_all(&t);
}

#[test]
fn real_escaping_link_is_gone_before_the_next_question_and_user_links_stay() {
    let t = tmp();
    let inn = t.join("in");
    fs::create_dir_all(inn.join("a")).unwrap();
    fs::create_dir_all(t.join("outside")).unwrap();
    fs::write(t.join("outside/secret"), b"s").unwrap();
    std::os::unix::fs::symlink("../outside", inn.join("b")).unwrap();
    std::os::unix::fs::symlink("../../outside", inn.join("a/u")).unwrap();
    fs::write(inn.join("z.txt"), b"mine").unwrap();
    let z = inn.join("m.zip");
    fs::write(&z, raw_zip(&[(b"a/", b"", 0), (b"a/w", b"../b/secret", LINK), (b"a/ok", b"u", LINK), (b"a_link", b"b/secret", LINK), (b"z.txt", b"theirs", 0)])).unwrap();
    let mut visible = Vec::new();
    let p = inn.join("a_link");
    let (o, _) = run_cb(JobSpec::Extract { archive: z, into_folder: false }, Choice::Merge, || {
        visible.push(fs::symlink_metadata(&p).is_ok());
        // En la pregunta de «z.txt» (la segunda) el usuario pone uno suyo con ese nombre: no se
        // toca.
        if visible.len() == 2 {
            std::os::unix::fs::symlink("../outside/secret", &p).unwrap();
        }
    });
    // La primera pregunta es la carpeta «a» (Combinar); la de «z.txt», después de «a_link».
    assert_eq!(visible, [false, false]);
    assert_eq!(fs::read_link(&p).unwrap(), Path::new("../outside/secret"));
    assert_eq!(fs::read_link(inn.join("a/u")).unwrap(), Path::new("../../outside"));
    assert!(fs::symlink_metadata(inn.join("a/w")).is_err());
    // «a/ok» → «u» (el enlace del usuario que sale): también sale, se borra.
    assert!(fs::symlink_metadata(inn.join("a/ok")).is_err());
    let mut bad: Vec<_> = o.errors.iter().filter(|e| e.message == NOT_ALLOWED).map(|e| e.entry.clone().unwrap()).collect();
    bad.sort();
    assert_eq!(bad, ["a/ok", "a/w", "a_link"]);
    // «Combinar» con un archivo es «Conservar ambos».
    assert_eq!(fs::read(inn.join("z.txt")).unwrap(), b"mine");
    assert_eq!(fs::read(inn.join("z 2.txt")).unwrap(), b"theirs");
    let _ = fs::remove_dir_all(&t);
}

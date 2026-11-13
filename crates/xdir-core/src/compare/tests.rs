use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::sys::testing::{FakeArchives, FakeFileSystem, FakeNetwork, install_fakes};

fn cmp(a: &[u8], b: &[u8]) -> Verdict {
    compare_readers(&mut &a[..], &mut &b[..], &mut |_| Flow::Continue).unwrap().unwrap()
}

/// Una carpeta temporal real, propia de cada prueba.
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("xdir-test-comparar").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn equal_contents() {
    let data: Vec<u8> = (0..(2 * BLOCK + 123)).map(|i| (i % 253) as u8).collect();
    assert_eq!(cmp(&data, &data), Verdict::Same);
    assert_eq!(cmp(b"", b""), Verdict::Same);
}

/// El primer byte distinto, en el primer byte, el último y en el segundo bloque.
#[test]
fn first_differing_offset() {
    let data = vec![5u8; 2 * BLOCK + 10];
    let mut b = data.clone();
    b[0] = 6;
    assert_eq!(cmp(&data, &b), Verdict::DiffersAt(0));
    let mut b = data.clone();
    *b.last_mut().unwrap() = 6;
    assert_eq!(cmp(&data, &b), Verdict::DiffersAt(2 * BLOCK as u64 + 9));
    let mut b = data.clone();
    b[BLOCK + 5] = 0;
    b[BLOCK + 9] = 0;
    assert_eq!(cmp(&data, &b), Verdict::DiffersAt(BLOCK as u64 + 5));
}

/// Si uno se termina antes, los largos leídos son distintos.
#[test]
fn shorter_reader_differs_where_it_ends() {
    assert_eq!(cmp(b"abcdef", b"abc"), Verdict::SizeDiffers { a: 6, b: 3 });
    assert_eq!(cmp(b"", b"x"), Verdict::SizeDiffers { a: 0, b: 1 });
}

/// Un archivo que cambia de tamaño mientras se compara (el que se abrió decía otro tamaño): se
/// avisa que cambió, no se informa una diferencia de contenido.
#[test]
fn file_changed_during_compare() {
    use crate::sys::testing::FakeHashes;
    install_fakes();
    let d = tmp("cambio");
    let (a, b) = (d.join("a.bin"), d.join("b.bin"));
    std::fs::write(&a, b"12345678").unwrap();
    std::fs::write(&b, b"1234567890").unwrap();
    // Al abrirlos los dos medían 10; «a» se achicó a 8 mientras se leía.
    FakeHashes::report_size(&a, 10);
    let got = run(&Source::Disk(a.clone()), &Source::Disk(b.clone()));
    assert_eq!(got, Err(Failure::Failed("«a.bin» cambió mientras se comparaba".into())));
    // Los dos crecieron igual (o se achicaron igual): tampoco son «iguales» a lo que se abrió.
    std::fs::write(&a, b"1234567890").unwrap();
    FakeHashes::report_size(&a, 12);
    FakeHashes::report_size(&b, 12);
    let got = run(&Source::Disk(a), &Source::Disk(b));
    assert_eq!(got, Err(Failure::Failed("«a.bin» cambió mientras se comparaba".into())));
}

/// Un lector que entrega de a pedazos chicos: los bloques se completan igual.
struct Chunky<'a>(&'a [u8], usize);

impl Read for Chunky<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.1.min(buf.len()).min(self.0.len());
        buf[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        self.1 = self.1 % 7 + 1;
        Ok(n)
    }
}

#[test]
fn short_reads_still_find_the_offset() {
    let a: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
    let mut b = a.clone();
    b[4321] ^= 1;
    let got = compare_readers(&mut Chunky(&a, 1), &mut Chunky(&b, 3), &mut |_| Flow::Continue).unwrap().unwrap();
    assert_eq!(got, Verdict::DiffersAt(4321));
}

/// El avance llega con lo comparado en total, por bloque; `Stop` corta.
#[test]
fn progress_and_cancel() {
    let data = vec![1u8; 2 * BLOCK + 1];
    let mut seen = Vec::new();
    compare_readers(&mut &data[..], &mut &data[..], &mut |b| {
        seen.push(b);
        Flow::Continue
    })
    .unwrap();
    assert_eq!(seen, [BLOCK as u64, 2 * BLOCK as u64, 2 * BLOCK as u64 + 1]);
    let (mut a, mut b) = (&data[..], &data[..]);
    assert_eq!(compare_readers(&mut a, &mut b, &mut |_| Flow::Stop).unwrap(), None);
    assert_eq!(a.len(), BLOCK + 1, "leyó un solo bloque");
}

/// Un error de lectura dice de cuál de los dos fue.
#[test]
fn read_error_says_which_side() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disco"))
        }
    }
    let data = [1u8; 10];
    assert!(matches!(compare_readers(&mut &data[..], &mut Broken, &mut |_| Flow::Continue), Err((1, _))));
    assert!(matches!(compare_readers(&mut Broken, &mut &data[..], &mut |_| Flow::Continue), Err((0, _))));
}

fn run(a: &Source, b: &Source) -> Result<Verdict, Failure> {
    compare_sources(a, b, &AtomicBool::new(false), &Progress::default())
}

/// Dos archivos del disco: iguales, distintos (con el primer byte) y de tamaños distintos (sin
/// comparar contenido).
#[test]
fn compare_disk_files() {
    install_fakes();
    let d = tmp("disco");
    let (a, b, c, e) = (d.join("a.bin"), d.join("b.bin"), d.join("c.bin"), d.join("e.bin"));
    std::fs::write(&a, b"hola mundo").unwrap();
    std::fs::write(&b, b"hola mundo").unwrap();
    std::fs::write(&c, b"hola Mundo").unwrap();
    std::fs::write(&e, b"hola").unwrap();
    let (sa, sb, sc, se) = (Source::Disk(a.clone()), Source::Disk(b), Source::Disk(c), Source::Disk(e));
    assert_eq!(run(&sa, &sb), Ok(Verdict::Same));
    assert_eq!(run(&sa, &sc), Ok(Verdict::DiffersAt(5)));
    assert_eq!(run(&sa, &se), Ok(Verdict::SizeDiffers { a: 10, b: 4 }));
    assert_eq!(run(&sa, &sa), Ok(Verdict::SameFile));
    // Uno que ya no está: el error dice cuál.
    let gone = Source::Disk(d.join("no-esta.bin"));
    assert!(matches!(run(&sa, &gone), Err(Failure::Failed(m)) if m.contains("no-esta.bin")), "{:?}", run(&sa, &gone));
}

/// Lo que está solo en la nube no se lee (no se baja nada).
#[test]
fn cloud_only_is_not_read() {
    install_fakes();
    let d = tmp("nube");
    let (a, b) = (d.join("a.bin"), d.join("b.bin"));
    std::fs::write(&a, b"x").unwrap();
    std::fs::write(&b, b"x").unwrap();
    FakeFileSystem::set_cloud_only_everywhere(&b);
    let got = run(&Source::Disk(a), &Source::Disk(b));
    assert!(matches!(&got, Err(Failure::Failed(m)) if m.contains("nube")), "{got:?}");
}

/// Un elemento de un comprimido se extrae a la temporal (con su límite y el origen del
/// comprimido) y se compara desde ahí.
#[test]
fn archive_entry_is_extracted_and_compared() {
    install_fakes();
    let d = tmp("zip");
    let zip = d.join("fotos.zip");
    FakeArchives::shared().add_archive(&zip, &[("docs/nota.txt", b"hola mundo"), ("otra.txt", b"hola Mundo"), ("grande.txt", b"hola mundo")], 40);
    // (la lista del comprimido se guarda al leerla: lo declarado va antes)
    FakeArchives::shared().set_declared(&zip, "grande.txt", crate::archive_nav::temps::OPEN_MAX + 1);
    let disk = d.join("nota.txt");
    std::fs::write(&disk, b"hola mundo").unwrap();
    let inner = |rel: &str| Source::Archive { file: zip.clone(), nested: Vec::new(), rel: rel.into() };
    assert_eq!(run(&inner("docs/nota.txt"), &Source::Disk(disk.clone())), Ok(Verdict::Same));
    assert_eq!(run(&inner("docs/nota.txt"), &inner("otra.txt")), Ok(Verdict::DiffersAt(5)));
    assert!(!FakeArchives::shared().propagated(&zip).is_empty(), "lo extraído hereda el origen del comprimido");
    // Más grande que lo que se extrae para abrir desde adentro: no se extrae.
    let got = run(&inner("grande.txt"), &Source::Disk(disk));
    assert!(matches!(&got, Err(Failure::Failed(m)) if m.contains(crate::archive_nav::temps::TOO_BIG)), "{got:?}");
}

/// La suma de un archivo del disco y de uno de adentro de un comprimido.
#[test]
fn hash_disk_and_archive() {
    install_fakes();
    let d = tmp("suma");
    let f = d.join("a.txt");
    std::fs::write(&f, b"hola").unwrap();
    let zip = d.join("x.zip");
    FakeArchives::shared().add_archive(&zip, &[("a.txt", b"hola")], 10);
    let stop = AtomicBool::new(false);
    let p = Progress::default();
    let disk = hash_source(&Source::Disk(f), HashAlgo::Sha256, &stop, &p).unwrap();
    assert_eq!(disk.len(), 64);
    let inner = Source::Archive { file: zip, nested: Vec::new(), rel: "a.txt".into() };
    assert_eq!(hash_source(&inner, HashAlgo::Sha256, &stop, &p).unwrap(), disk);
    assert_eq!(hash_source(&inner, HashAlgo::Md5, &stop, &p).unwrap().len(), 32);
}

/// La vigilancia: un servidor de red que deja de responder corta con «no responde» al pasar el
/// límite sin avance; uno lento que avanza no se corta; Cancelar corta enseguida.
#[test]
fn watched_stalled_network_slow_and_cancel() {
    install_fakes();
    let d = tmp("red");
    let (a, b) = (d.join("a.bin"), d.join("b.bin"));
    std::fs::write(&a, vec![3u8; 100]).unwrap();
    std::fs::write(&b, vec![3u8; 100]).unwrap();
    FakeNetwork::set_remote(&d);
    FakeNetwork::hang(&d.to_string_lossy(), Duration::from_secs(3));
    let (sa, sb) = (Source::Disk(a.clone()), Source::Disk(b.clone()));
    assert!(sa.remote_host().is_some());
    let t0 = Instant::now();
    let progress = Arc::new(Progress::default());
    let got = run_watched(sa.remote_host(), Duration::from_millis(200), &AtomicBool::new(false), &progress, Duration::from_millis(50), &|| {}, &std::sync::Mutex::new(None), {
        let (sa, sb, p) = (sa.clone(), sb.clone(), progress.clone());
        move |stop| compare_sources(&sa, &sb, stop, &p)
    });
    assert!(matches!(&got, Err(Failure::Failed(m)) if m.contains("no responde")), "{got:?}");
    // Nombra la conexión (la carpeta de red montada), no el archivo.
    let share = d.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(got, Err(Failure::Failed(format!("\"{share}\" no responde"))));
    assert!(t0.elapsed() < Duration::from_secs(2), "no esperó al servidor");
    // Cancelar: corta enseguida aunque el hilo siga colgado.
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        c2.store(true, Ordering::Relaxed);
    });
    let t0 = Instant::now();
    let got = run_watched(sa.remote_host(), Duration::from_secs(30), &cancel, &progress, Duration::from_millis(50), &|| {}, &std::sync::Mutex::new(None), {
        let (sa, sb, p) = (sa.clone(), sb.clone(), progress.clone());
        move |stop| compare_sources(&sa, &sb, stop, &p)
    });
    assert_eq!(got, Err(Failure::Cancelled));
    assert!(t0.elapsed() < Duration::from_secs(2));
    // Local y lento, pero avanzando: termina.
    let ticks = std::sync::atomic::AtomicUsize::new(0);
    let tick = || {
        ticks.fetch_add(1, Ordering::Relaxed);
    };
    let ok = run_watched(None, Duration::from_millis(50), &AtomicBool::new(false), &Arc::new(Progress::default()), Duration::from_millis(30), &tick, &std::sync::Mutex::new(None), |_| {
        std::thread::sleep(Duration::from_millis(200));
        Ok(7)
    });
    assert_eq!(ok, Ok(7));
    assert!(ticks.load(Ordering::Relaxed) >= 2, "avisa mientras espera");
}

/// «Comparar con…»: la suma publicada sin espacios ni mayúsculas, con el nombre al lado o en la
/// forma de BSD; el largo elige el algoritmo.
#[test]
fn parse_published_sums() {
    let sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let md5 = "d41d8cd98f00b204e9800998ecf8427e";
    assert_eq!(parse_expected(sha), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&sha.to_uppercase()), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("  {sha}\n")), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("{sha}  xdir-0.1.23.zip")), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("{sha} *xdir.zip")), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("SHA256 (xdir.zip) = {sha}")), Some((HashAlgo::Sha256, sha.into())));
    // En grupos y con el nombre al lado, o con el nombre antes («nombre: suma»).
    let grouped: String = sha.as_bytes().chunks(8).map(|c| std::str::from_utf8(c).unwrap().to_uppercase()).collect::<Vec<_>>().join(" ");
    assert_eq!(parse_expected(&format!("{grouped}  a-igual.bin")), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("xdir.zip: {grouped}")), Some((HashAlgo::Sha256, sha.into())));
    // Un nombre con «=», la fila de la tabla de Get-FileHash y «sha256:suma».
    assert_eq!(parse_expected(&format!("{sha}  a=b.zip")), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("SHA256          {}       C:\\x\\a.zip", sha.to_uppercase())), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("sha256:{sha}")), Some((HashAlgo::Sha256, sha.into())));
    assert_eq!(parse_expected(&format!("SHA256 (a=b.zip) = {sha}")), Some((HashAlgo::Sha256, sha.into())));
    // Con espacios en el medio (como certutil) y en otra línea.
    let spaced: String = md5.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ");
    assert_eq!(parse_expected(&spaced.to_uppercase()), Some((HashAlgo::Md5, md5.into())));
    assert_eq!(parse_expected(&format!("Hash MD5 de x.zip:\n{spaced}\nCertUtil: listo")), Some((HashAlgo::Md5, md5.into())));
    assert_eq!(parse_expected(""), None);
    assert_eq!(parse_expected("hola"), None);
    assert_eq!(parse_expected(&sha[..40]), None);
    assert_eq!(parse_expected(&format!("{}g", &md5[..31])), None);
}

/// La suma en líneas de 64, 32 o 16 caracteres según el ancho.
#[test]
fn hex_lines_fit_the_width() {
    let sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert_eq!(hex_lines(sha, 64.0 * MONO_W), vec![sha.to_string()]);
    assert_eq!(hex_lines(sha, 40.0 * MONO_W), vec![sha[..32].to_string(), sha[32..].to_string()]);
    assert_eq!(hex_lines(sha, 20.0 * MONO_W).len(), 4);
    assert_eq!(hex_lines(&sha[..32], 33.0 * MONO_W).len(), 1);
}

#[test]
fn offset_text() {
    assert_eq!(super::offset_text(0), "0 (0x0)");
    assert_eq!(super::offset_text(1_234_567), "1.234.567 (0x12D687)");
}


/// Una red lenta pero viva (de a 128 KB, una pausa antes de cada lectura, más que el límite por
/// bloque de 1 MB pero menos que el límite por lectura): no se corta, ni al comparar ni al sumar.
#[test]
fn slow_but_alive_network_is_not_cut() {
    use crate::sys::testing::FakeHashes;
    install_fakes();
    let d = tmp("red-lenta");
    let data: Vec<u8> = (0..(3 * BLOCK / 2)).map(|i| (i % 249) as u8).collect();
    let (a, b) = (d.join("a.bin"), d.join("b.bin"));
    std::fs::write(&a, &data).unwrap();
    std::fs::write(&b, &data).unwrap();
    FakeNetwork::set_remote(&d);
    FakeHashes::throttle(&d, 128 << 10, Duration::from_millis(60));
    let (sa, sb) = (Source::Disk(a), Source::Disk(b));
    let progress = Arc::new(Progress::default());
    let got = run_watched(sa.remote_host(), Duration::from_millis(250), &AtomicBool::new(false), &progress, Duration::from_millis(50), &|| {}, &std::sync::Mutex::new(None), {
        let (sa, sb, p) = (sa.clone(), sb.clone(), progress.clone());
        move |stop| compare_sources(&sa, &sb, stop, &p)
    });
    assert_eq!(got, Ok(Verdict::Same));
    let got = run_watched(sa.remote_host(), Duration::from_millis(250), &AtomicBool::new(false), &progress, Duration::from_millis(50), &|| {}, &std::sync::Mutex::new(None), {
        let (sa, p) = (sa.clone(), progress.clone());
        move |stop| hash_source(&sa, HashAlgo::Sha256, stop, &p)
    });
    assert!(got.is_ok(), "{got:?}");
}

/// Un archivo de sumas entero (SHA256SUMS): se usa la línea del archivo y se dice cuál; si ninguna
/// lo nombra, la primera, y también se dice.
#[test]
fn pick_line_of_a_sums_file() {
    let (s1, s2) = ("1".repeat(64), "2".repeat(64));
    let sums = format!("{s1}  xdir-0.2.0-mac.zip\n{s2} *xdir-0.2.0-windows.zip\n");
    let p = pick_expected(&sums, "xdir-0.2.0-windows.zip").unwrap();
    assert_eq!((p.algo, p.hex.as_str(), p.line, p.of, p.by_name), (HashAlgo::Sha256, s2.as_str(), 2, 2, true));
    let p = pick_expected(&sums, "otro.zip").unwrap();
    assert_eq!((p.hex.as_str(), p.line, p.of, p.by_name), (s1.as_str(), 1, 2, false));
    let p = pick_expected(&format!("{s1}  a.zip"), "b.zip").unwrap();
    assert_eq!((p.line, p.of), (1, 1));
    assert!(pick_expected("nada", "a.zip").is_none());
}

/// El mismo archivo por dos caminos (un alias de la carpeta, una unidad de red y su UNC): por la
/// identidad del archivo (la del motor de copia) es «el mismo archivo», sin leerlo.
#[test]
fn same_file_by_identity() {
    use crate::sys::testing::FakeCopyPrims;
    install_fakes();
    let disk = FakeCopyPrims::shared();
    let base = std::env::temp_dir().join("xdir-test-comparar-identidad").join(std::process::id().to_string());
    let (real, alias) = (base.join("real"), base.join("alias"));
    disk.add_file(&real.join("a.bin"), b"hola".to_vec());
    disk.add_alias(&alias, &real);
    let got = run(&Source::Disk(real.join("a.bin")), &Source::Disk(alias.join("a.bin")));
    assert_eq!(got, Ok(Verdict::SameFile));
}

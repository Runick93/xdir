//! Pruebas de la vista de la ventana de copia (`transfer::view`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::speed::Speed;
use super::view::{self, ViewHit};
use super::{Choice, ConflictInfo, JobError, JobRoute, JobState, JobView};
use crate::geom::{Rect, rect};
use crate::paint::FontId;
use crate::sys::testing::{FakeEnv, RecordingPainter};
use crate::sys::{Holder, ItemKind, ItemStat};
use crate::theme::{DEFAULT_ACCENT, Theme};

const MB: f64 = 1024.0 * 1024.0;

fn job(id: u64, state: JobState) -> JobView {
    let now = Instant::now();
    JobView {
        id,
        title: format!("Copiando 1.284 elementos de «Descargas» a «Respaldo» ({id})"),
        state,
        items: (0, 10),
        bytes: (0, 0),
        current: String::new(),
        speed: Speed::new(now, 0),
        conflict: None,
        errors: Vec::new(),
        stopped: None,
        paused: false,
        started: now,
        apply_all: false,
        conflict_since: None,
        done_items: None,
        route: JobRoute { done_one: "Se copió", done_many: "Se copiaron", from: Some("Descargas".into()), to: Some("Respaldo".into()), sources: vec![PathBuf::from("/u/Descargas/Facturas"), PathBuf::from("/u/Descargas/Instalador-Rust.exe")] },
    }
}

/// Un trabajo al 58 % que va a 48,25 MB/s desde hace 3 s y le quedan 2 minutos.
fn running(id: u64) -> JobView {
    let rate = 48.25 * MB;
    let done = (rate * 3.0) as u64;
    let left = (rate * 120.0) as u64;
    let start = Instant::now() - Duration::from_secs(3);
    let mut j = job(id, JobState::Running);
    // La barra va por los bytes del trabajo (58 %); el tiempo restante, por la medición.
    j.bytes = (58 * 1024 * 1024 * 1024 / 10, 100 * 1024 * 1024 * 1024 / 10);
    j.items = (745, 1284);
    j.current = "IMG_2041.jpg".into();
    j.speed = Speed::new(start, done + left);
    j.speed.push(start + Duration::from_secs(3), done);
    j
}

fn theme() -> Theme {
    Theme::from_system(DEFAULT_ACCENT, Some(true))
}

/// Expandido solo `e` (o ninguno).
fn only(e: Option<u64>) -> impl Fn(u64) -> bool {
    move |id| e == Some(id)
}

fn area(jobs: &[JobView], expanded: Option<u64>) -> Rect {
    rect(0.0, 0.0, view::WIDTH, view::content_height(jobs, &only(expanded)))
}

fn look(expanded: &dyn Fn(u64) -> bool, scroll: f32) -> view::Look<'_> {
    view::Look { expanded, scroll, hover: None, dragging: false }
}

fn texts(jobs: &[JobView], expanded: Option<u64>) -> Vec<String> {
    texts_in(area(jobs, expanded), jobs, &only(expanded), 0.0)
}

fn texts_in(r: Rect, jobs: &[JobView], expanded: &dyn Fn(u64) -> bool, scroll: f32) -> Vec<String> {
    let p = RecordingPainter::new(theme());
    view::draw(&p, r, jobs, &look(expanded, scroll), &FakeEnv);
    p.texts().into_iter().map(|(_, s, _)| s).collect()
}

fn has(texts: &[String], needle: &str) -> bool {
    texts.iter().any(|t| t.contains(needle))
}

fn stat(size: u64, modified: u64, kind: ItemKind) -> ItemStat {
    ItemStat { kind, size, modified, cloud_only: false, file_id: None }
}

fn conflict(id: u64, dir: bool) -> JobView {
    let mut j = running(id);
    j.state = JobState::Conflict;
    let kind = if dir { ItemKind::Dir } else { ItemKind::File };
    j.conflict = Some(ConflictInfo {
        name: "informe.pdf".into(),
        folder: "Respaldo".into(),
        is_dir: dir,
        can_merge: dir,
        incoming: stat(2_200_000, 20, kind),
        existing: stat(1_990_000, 10, kind),
        remaining: Some(11),
    });
    j
}

/// Centro del control `h` (lo busca recorriendo el área).
fn center_of(jobs: &[JobView], expanded: Option<u64>, h: ViewHit) -> Option<(f32, f32)> {
    center_in(area(jobs, expanded), jobs, &only(expanded), 0.0, h)
}

/// Lo mismo en un área visible `r` con el contenido desplazado `scroll`.
fn center_in(r: Rect, jobs: &[JobView], expanded: &dyn Fn(u64) -> bool, scroll: f32, h: ViewHit) -> Option<(f32, f32)> {
    let mut y = r.top;
    while y < r.bottom {
        let mut x = r.left;
        while x < r.right {
            if view::hit(r, jobs, expanded, scroll, x, y) == Some(h) {
                return Some((x + 1.0, y + 1.0));
            }
            x += 2.0;
        }
        y += 2.0;
    }
    None
}

#[test]
fn rows_draw_progress_speed_eta() {
    let jobs = [running(1)];
    let t = texts(&jobs, None);
    assert!(has(&t, "58 %"), "{t:?}");
    assert!(has(&t, "48,2 MB/s"), "{t:?}");
    assert!(has(&t, "quedan 2 min"), "{t:?}");
    // Antes de 2 s de datos: «Calculando…».
    let mut j = running(2);
    let now = Instant::now();
    j.speed = Speed::new(now - Duration::from_millis(500), j.bytes.1);
    j.speed.push(now, j.bytes.0);
    let t = texts(&[j], None);
    assert!(has(&t, "Calculando…"), "{t:?}");
    assert!(!has(&t, "quedan"), "{t:?}");
}

#[test]
fn expanded_shows_details() {
    let jobs = [running(1)];
    let t = texts(&jobs, Some(1));
    for s in ["58 % completado", "Pausa", "Cancelar", "Nombre:", "IMG_2041.jpg", "Tiempo restante:", "Unos 2 minutos", "Elementos restantes:", "539 (", "Menos detalles"] {
        assert!(has(&t, s), "falta {s:?} en {t:?}");
    }
    assert_eq!(view::content_height(&jobs, &only(Some(1))), 290.0);
    assert_eq!(view::content_height(&jobs, &only(None)), 64.0);
}

#[test]
fn queued_row_says_en_cola() {
    let jobs = [job(1, JobState::Waiting)];
    let t = texts(&jobs, None);
    assert!(t.iter().any(|s| s == "En cola"), "{t:?}");
    assert_eq!(view::content_height(&jobs, &only(None)), 56.0);
    let mut prep = job(2, JobState::Preparing);
    prep.items = (0, 1240);
    let t = texts(&[prep], None);
    assert!(has(&t, "Preparando… 1.240 elementos"), "{t:?}");
    let mut paused = running(3);
    paused.paused = true;
    assert!(has(&texts(&[paused], None), "En pausa"));
}

#[test]
fn graph_drawn_for_expanded() {
    let mut j = running(1);
    // 58 muestras de velocidad, una por cada 1 %.
    let start = Instant::now() - Duration::from_secs(60);
    j.speed = Speed::new(start, 100 * MB as u64);
    for k in 1..=58u64 {
        j.speed.push(start + Duration::from_millis(k * 1000), k * MB as u64);
    }
    assert_eq!(j.speed.graph().len(), 58);
    let jobs = [j];
    let p = RecordingPainter::new(theme());
    view::draw(&p, area(&jobs, Some(1)), &jobs, &look(&only(Some(1)), 0.0), &FakeEnv);
    let t: Vec<String> = p.texts().into_iter().map(|(_, s, _)| s).collect();
    assert!(has(&t, "Velocidad: "), "{t:?}");
    assert!(has(&t, "Promedio: "), "{t:?}");
    // El área del gráfico (cerrada, con relleno) llega hasta el 58 % del ancho.
    let area_path = p.paths().into_iter().find(|(_, closed, fill)| *closed && fill.is_some()).expect("área");
    let right = area_path.0.iter().map(|(x, _)| *x).fold(0.0, f32::max);
    let (l, r) = (17.0, view::WIDTH - 17.0);
    assert!((right - (l + (r - l) * 0.58)).abs() < 1.0, "{right}");
    // Sin expandir, no hay gráfico.
    assert!(!has(&texts(&jobs, None), "Velocidad: "));
}

#[test]
fn conflict_view_buttons() {
    let file = [conflict(1, false)];
    let t = texts(&file, None);
    for s in ["Ya existe «informe.pdf» en «Respaldo»", "El que se copia", "El que ya está", "Más nuevo", "Reemplazar", "Conservar ambos", "Omitir", "Hacer esto con todos los archivos (12 conflictos)"] {
        assert!(has(&t, s), "falta {s:?} en {t:?}");
    }
    assert!(!has(&t, "Combinar"));
    assert!(has(&t, "2,09 MB") && has(&t, "1,89 MB"), "tamaños: {t:?}");
    assert!(center_of(&file, None, ViewHit::Choose(1, Choice::Merge)).is_none());
    assert!(center_of(&file, None, ViewHit::Choose(1, Choice::KeepBoth)).is_some());
    assert_eq!(view::content_height(&file, &only(None)), 316.0);
    let folder = [conflict(1, true)];
    let t = texts(&folder, None);
    assert!(has(&t, "Combinar"), "{t:?}");
    assert!(t.iter().any(|s| s == "Hacer esto con todas las carpetas (12 conflictos)"), "{t:?}");
    // Sin cuenta exacta (adentro de una carpeta que se combina): sin número.
    let mut nested = conflict(1, false);
    nested.conflict.as_mut().unwrap().remaining = None;
    assert!(texts(&[nested], None).iter().any(|s| s == "Hacer esto con todos los archivos"));
    assert!(center_of(&folder, None, ViewHit::Choose(1, Choice::Merge)).is_some());
    assert!(center_of(&folder, None, ViewHit::ApplyAll(1)).is_some());
}

fn error(name: &str, why: &str, retry: bool) -> JobError {
    JobError { src: PathBuf::from(format!("/d/Descargas/{name}")), dst: PathBuf::from(format!("/r/{name}")), message: why.into(), retry, entry: None, holders: Vec::new() }
}

#[test]
fn errors_view_and_retry() {
    let mut j = job(1, JobState::Done);
    j.errors = vec![error("Instalador-Rust.exe", "Lo está usando otro programa", true), error("factura-0039.pdf", "No tenés permiso", true), error("setup-viejo.msi", "Ya no existe", true)];
    let jobs = [j];
    let t = texts(&jobs, None);
    for s in ["Terminó con 3 errores", "Instalador-Rust.exe", "Lo está usando otro programa", "No tenés permiso", "Ya no existe", "Reintentar", "Cerrar"] {
        assert!(has(&t, s), "falta {s:?} en {t:?}");
    }
    assert_eq!(view::content_height(&jobs, &only(None)), 76.0 + 44.0 * 3.0 + 16.0 + 45.0);
    assert!(center_of(&jobs, None, ViewHit::Retry(1)).is_some());
    assert!(center_of(&jobs, None, ViewHit::Dismiss(1)).is_some());
    // Solo avisos (nada para reintentar): sin «Reintentar».
    let mut w = job(2, JobState::Done);
    w.errors = vec![error("a.txt", "Quedó como «a 2.txt»", false)];
    let w = [w];
    assert!(!has(&texts(&w, None), "Reintentar"));
    assert!(center_of(&w, None, ViewHit::Retry(2)).is_none());
    // Uno solo: en singular.
    let mut one = job(3, JobState::Done);
    one.errors = vec![error("a.txt", "Ya no existe", true)];
    assert!(has(&texts(&[one], None), "Terminó con 1 error"));
}

fn pwsh(pid: u32, user: Option<&str>) -> Holder {
    Holder { name: "PowerShell 7".into(), exe: Some("pwsh.exe".into()), pid, user: user.map(Into::into) }
}

/// Un trabajo terminado con un solo error, del archivo que tienen `holders`.
fn held(holders: Vec<Holder>) -> [JobView; 1] {
    let mut j = job(1, JobState::Done);
    let mut e = error("a.txt", "Lo está usando otro programa", true);
    e.holders = holders;
    j.errors = vec![e];
    [j]
}

#[test]
fn error_row_names_who_holds_the_file() {
    let jobs = held(vec![pwsh(24012, Some(r"PC\Leandro"))]);
    let p = RecordingPainter::new(theme());
    view::draw(&p, area(&jobs, None), &jobs, &look(&only(None), 0.0), &FakeEnv);
    let draws = p.text_draws();
    let th = theme();
    // Cuatro líneas, una debajo de la otra: rótulo en texto secundario y valor en texto normal.
    let mut last = f32::MIN;
    for (label, value) in [("Archivo", "a.txt"), ("Ubicación", ""), ("Proceso", "PowerShell 7 · PID 24012"), ("Usuario", r"PC\Leandro")] {
        let l = draws.iter().find(|d| d.1 == label).unwrap_or_else(|| panic!("{label}: {draws:?}"));
        assert_eq!(l.3, th.text_dim);
        assert!(l.2.top > last, "{label} fuera de orden");
        last = l.2.top;
        if !value.is_empty() {
            let v = draws.iter().find(|d| d.1 == value).unwrap_or_else(|| panic!("{value}: {draws:?}"));
            assert_eq!(v.3, th.text);
            assert!(v.2.left > l.2.right - 0.5 && (v.2.top - l.2.top).abs() < 0.5, "{l:?} {v:?}");
        }
    }
    assert!(!draws.iter().any(|d| d.1.contains("Lo está usando otro programa") || d.1.contains("En uso por")), "{draws:?}");
    // El alto y los botones, como siempre.
    assert_eq!(view::content_height(&jobs, &only(None)), 76.0 + 26.0 * 4.0 + 4.0 + 16.0 + 45.0);
    assert!(center_of(&jobs, None, ViewHit::Retry(1)).is_some());
    assert!(center_of(&jobs, None, ViewHit::Dismiss(1)).is_some());
}

#[test]
fn error_row_counts_other_programs_by_pid() {
    let two = texts(&held(vec![pwsh(24012, None), Holder { name: "Explorador".into(), exe: None, pid: 9, user: None }]), None);
    assert!(two.iter().any(|s| s == "PowerShell 7 · PID 24012 y 1 más"), "{two:?}");
    // El mismo proceso dos veces es uno solo.
    let same = texts(&held(vec![pwsh(24012, None), pwsh(24012, None)]), None);
    assert!(same.iter().any(|s| s == "PowerShell 7 · PID 24012"), "{same:?}");
    assert!(!has(&same, "más"), "{same:?}");
}

#[test]
fn error_row_without_name_or_user() {
    let t = texts(&held(vec![Holder { name: String::new(), exe: None, pid: 24012, user: None }]), None);
    assert!(t.iter().any(|s| s == "Proceso 24012 · PID 24012"), "{t:?}");
    assert!(t.iter().any(|s| s == "Desconocido"), "{t:?}");
    // Sin nombre pero con ejecutable: el ejecutable.
    let t = texts(&held(vec![Holder { name: " ".into(), exe: Some("pwsh.exe".into()), pid: 5, user: None }]), None);
    assert!(t.iter().any(|s| s == "pwsh.exe · PID 5"), "{t:?}");
}

/// Sin holders, la fila es la de siempre: el motivo en una línea, centrado en la fila.
#[test]
fn error_row_without_holders_is_unchanged() {
    let jobs = held(Vec::new());
    let p = RecordingPainter::new(theme());
    view::draw(&p, area(&jobs, None), &jobs, &look(&only(None), 0.0), &FakeEnv);
    let draws = p.text_draws();
    let why = "Lo está usando otro programa";
    let reason = draws.iter().find(|d| d.1 == why).unwrap_or_else(|| panic!("{draws:?}"));
    let (right, y) = (view::WIDTH - 16.0, 76.0);
    let mw = (why.chars().count() as f32 * 7.0).min(220.0);
    assert_eq!((reason.0, reason.2, reason.3), (FontId::UiRight, rect(right - 16.0 - mw, y + 13.0, right - 12.0, y + 31.0), theme().danger));
    let name = draws.iter().find(|d| d.1 == "a.txt").expect("nombre");
    assert_eq!(name.2, rect(16.0 + 38.0, y + 6.0, right - 16.0 - mw - 12.0, y + 24.0));
    assert!(!has(&draws.iter().map(|d| d.1.clone()).collect::<Vec<_>>(), "En uso por"));
}

/// Con textos largos, todo queda adentro de la fila (con «…») y el nombre no pisa la columna de la derecha.
#[test]
fn error_row_long_texts_stay_inside() {
    let long = Holder {
        name: "Un programa con un nombre larguísimo que no entra en la fila de ningún modo posible".into(),
        exe: None,
        pid: 24012,
        user: Some(r"UN-DOMINIO-MUY-LARGO\un.usuario.con.nombre.larguisimo.que.tampoco.entra.en.la.fila".into()),
    };
    let mut jobs = held(vec![long]);
    jobs[0].errors[0].src = PathBuf::from("/d/Descargas/un-archivo-con-un-nombre-tambien-muy-largo-para-la-fila-que-no-entra-nunca-jamas.txt");
    let p = RecordingPainter::new(theme());
    view::draw(&p, area(&jobs, None), &jobs, &look(&only(None), 0.0), &FakeEnv);
    let row = rect(16.0, 76.0, view::WIDTH - 16.0, 76.0 + 26.0 * 4.0 + 4.0);
    let inside: Vec<_> = p.texts().into_iter().filter(|(_, _, r)| r.top >= row.top && r.bottom <= row.bottom).collect();
    for start in ["Un programa", "UN-DOMINIO", "un-archivo"] {
        let t = inside.iter().find(|t| t.1.starts_with(start)).unwrap_or_else(|| panic!("{start}: {inside:?}"));
        assert!(t.1.ends_with('…'), "{t:?}");
        assert!(t.2.left >= row.left && t.2.right <= row.right, "{t:?}");
        // El texto entra en su rectángulo (7 DIPs por carácter en el pintor de prueba).
        assert!(t.1.chars().count() as f32 * 7.0 <= t.2.right - t.2.left + 0.5, "{t:?}");
    }
}

#[test]
fn not_enough_space_text() {
    let mut j = job(1, JobState::Done);
    j.stopped = Some("Faltan 3,20 GB en \"Backup (E:)\"".into());
    let jobs = [j];
    let t = texts(&jobs, None);
    assert!(t.iter().any(|s| s == "Faltan 3,20 GB en \"Backup (E:)\""), "{t:?}");
    assert!(center_of(&jobs, None, ViewHit::Dismiss(1)).is_some());
}

#[test]
fn pause_resume_cancel_buttons() {
    let jobs = [running(1), job(2, JobState::Waiting)];
    // Expandido: «Pausa» y «Cancelar» con texto; en la fila: los íconos.
    assert!(center_of(&jobs, Some(1), ViewHit::Pause(1)).is_some());
    assert!(center_of(&jobs, Some(1), ViewHit::Cancel(1)).is_some());
    assert!(center_of(&jobs, None, ViewHit::Pause(1)).is_some());
    assert!(center_of(&jobs, None, ViewHit::Expand(1)).is_some());
    // En cola: solo cancelar.
    assert!(center_of(&jobs, None, ViewHit::Cancel(2)).is_some());
    assert!(center_of(&jobs, None, ViewHit::Pause(2)).is_none());
    let mut paused = running(1);
    paused.paused = true;
    let paused = [paused];
    assert!(center_of(&paused, Some(1), ViewHit::Resume(1)).is_some());
    assert!(center_of(&paused, Some(1), ViewHit::Pause(1)).is_none());
    assert!(has(&texts(&paused, Some(1)), "Reanudar"));
    // «Menos detalles» pliega el expandido.
    assert!(center_of(&jobs, Some(1), ViewHit::Expand(1)).is_some());
}

#[test]
fn eta_texts() {
    assert_eq!(view::eta_short(Duration::from_secs(20)), "quedan 20 s");
    assert_eq!(view::eta_short(Duration::from_secs(119)), "quedan 2 min");
    assert_eq!(view::eta_short(Duration::from_secs(3900)), "quedan 1 h 5 min");
    assert_eq!(view::eta_long(Duration::from_secs(120)), "Unos 2 minutos");
    assert_eq!(view::eta_long(Duration::from_secs(20)), "Unos 20 segundos");
}

#[test]
fn finished_header_counts_and_relative_folders() {
    let mut j = job(1, JobState::Done);
    j.items = (1284, 1284);
    let err = |src: &str, why: &str| JobError { src: PathBuf::from(src), dst: PathBuf::from("/r/x"), message: why.into(), retry: true, entry: None, holders: Vec::new() };
    j.errors = vec![
        err("/u/Descargas/Instalador-Rust.exe", "Lo está usando otro programa"),
        err("/u/Descargas/Facturas/factura-0039.pdf", "No tenés permiso"),
        err("/otro/lado/suelto.txt", "Ya no existe"),
    ];
    let t = texts(&[j], None);
    assert!(t.iter().any(|s| s == "Se copiaron 1.281 de 1.284 elementos de «Descargas» a «Respaldo»"), "{t:?}");
    // La carpeta de cada error, desde la carpeta de lo que se copió.
    let facturas = std::path::Path::new("Descargas").join("Facturas").display().to_string();
    assert!(t.contains(&facturas), "{t:?}");
    assert!(t.iter().any(|s| s == "Descargas"), "{t:?}");
    // Fuera de lo elegido (no debería pasar): la ruta entera.
    let full = std::path::Path::new("/otro/lado").display().to_string();
    assert!(t.contains(&full), "{t:?}");
    // Con la cuenta del motor (lo de adentro de una carpeta que no se pudo crear no llegó).
    let mut counted = job(3, JobState::Done);
    counted.items = (1284, 1284);
    counted.done_items = Some(1000);
    counted.errors = vec![err("/u/Descargas/Facturas", "No tenés permiso")];
    assert!(texts(&[counted], None).iter().any(|s| s == "Se copiaron 1.000 de 1.284 elementos de «Descargas» a «Respaldo»"));
    // En singular.
    let mut one = job(2, JobState::Done);
    one.items = (1, 1);
    one.route.to = Some("d".into());
    one.route.from = None;
    one.errors = vec![err("/u/Descargas/a.txt", "Ya no existe")];
    assert!(texts(&[one], None).iter().any(|s| s == "Se copiaron 0 de 1 elemento a «d»"));
}

#[test]
fn window_title_counts_waiting_conflicts() {
    assert_eq!(view::window_title(&[running(1)]), "Copiando…");
    assert_eq!(view::window_title(&[conflict(1, false), running(2)]), "Copiando: un conflicto");
    assert_eq!(view::window_title(&[conflict(1, false), conflict(2, true)]), "Copiando: 2 conflictos");
}

/// Un trabajo sin bytes (eliminar, devolver): 40 de 100 elementos en 3 s.
fn items_only(id: u64) -> JobView {
    let start = Instant::now() - Duration::from_secs(3);
    let mut j = job(id, JobState::Running);
    j.title = "Eliminando 100 elementos".into();
    j.items = (40, 100);
    j.current = "a.txt".into();
    // Sin bytes, la velocidad se mide en elementos.
    j.speed = Speed::new(start, 100);
    j.speed.push(start + Duration::from_secs(3), 40);
    j
}

#[test]
fn items_only_job_has_no_byte_speed() {
    let jobs = [items_only(1)];
    for expanded in [None, Some(1)] {
        let t = texts(&jobs, expanded);
        assert!(!has(&t, "bytes/s") && !has(&t, "/s"), "{t:?}");
        assert!(!has(&t, "(0 bytes)") && !has(&t, "Calculando…"), "{t:?}");
        assert!(!has(&t, "Velocidad: ") && !has(&t, "Promedio: "), "{t:?}");
    }
    // El avance y el tiempo restante van por elementos.
    let t = texts(&jobs, None);
    assert!(has(&t, "40 %") && has(&t, "quedan 5 s"), "{t:?}");
    let t = texts(&jobs, Some(1));
    for s in ["40 % completado", "Tiempo restante:", "Unos 5 segundos", "Elementos restantes:", "Cancelar", "Menos detalles"] {
        assert!(t.iter().any(|x| x == s || x.contains(s)), "falta {s:?} en {t:?}");
    }
    assert!(t.iter().any(|x| x == "60"), "{t:?}");
    // Sin gráfico, el bloque es más bajo y «Menos detalles» sigue respondiendo.
    assert!(view::content_height(&jobs, &only(Some(1))) < 290.0);
    assert!(center_of(&jobs, Some(1), ViewHit::Expand(1)).is_some());
}

#[test]
fn queued_row_has_start_now() {
    let jobs = [running(1), job(2, JobState::Waiting)];
    assert!(center_of(&jobs, None, ViewHit::StartNow(2)).is_some());
    assert!(center_of(&jobs, None, ViewHit::StartNow(1)).is_none());
    assert!(has(&texts(&jobs, None), "Empezar ahora"));
    // Alto de un botón de texto (BTN_H), adentro de la fila en cola.
    let r = rect(0.0, 0.0, view::WIDTH, view::content_height(&jobs, &only(None)));
    let ys: Vec<f32> = (0..(r.bottom as i32 * 2))
        .map(|i| i as f32 / 2.0)
        .filter(|y| view::hit(r, &jobs, &only(None), 0.0, r.right - 60.0, *y) == Some(ViewHit::StartNow(2)))
        .collect();
    assert!(!ys.is_empty());
    let h = ys.last().unwrap() - ys.first().unwrap();
    assert!((h - 28.0).abs() <= 1.0, "alto {h}");
}

/// Un trabajo corriendo con 58 muestras de velocidad (el gráfico tiene algo que dibujar).
fn with_graph(id: u64) -> JobView {
    let mut j = running(id);
    let start = Instant::now() - Duration::from_secs(60);
    j.speed = Speed::new(start, 100 * MB as u64);
    for k in 1..=58u64 {
        j.speed.push(start + Duration::from_millis(k * 1000), k * MB as u64);
    }
    j
}

/// Como en Windows: cada copia en curso tiene sus detalles; expandir una no pliega la otra.
#[test]
fn two_running_jobs_both_expanded() {
    let jobs = [with_graph(1), with_graph(2)];
    let all = |_: u64| true;
    let r = rect(0.0, 0.0, view::WIDTH, view::content_height(&jobs, &all));
    assert_eq!(r.bottom, 2.0 * 290.0);
    let p = RecordingPainter::new(theme());
    view::draw(&p, r, &jobs, &look(&all, 0.0), &FakeEnv);
    let t: Vec<String> = p.texts().into_iter().map(|(_, s, _)| s).collect();
    let count = |needle: &str| t.iter().filter(|s| s.contains(needle)).count();
    assert_eq!(count("Menos detalles"), 2, "{t:?}");
    assert_eq!(count("Velocidad: "), 2, "{t:?}");
    assert_eq!(count("% completado"), 2, "{t:?}");
    // Un área de gráfico por trabajo.
    assert_eq!(p.paths().into_iter().filter(|(_, closed, fill)| *closed && fill.is_some()).count(), 2);
    for id in [1, 2] {
        assert!(center_in(r, &jobs, &all, 0.0, ViewHit::Expand(id)).is_some(), "falta «Menos detalles» de {id}");
        assert!(center_in(r, &jobs, &all, 0.0, ViewHit::Cancel(id)).is_some());
    }
    // Plegado solo el 1: el 2 sigue con sus detalles.
    let not_one = |id: u64| id != 1;
    assert_eq!(view::content_height(&jobs, &not_one), 64.0 + 290.0);
    let t = texts_in(rect(0.0, 0.0, view::WIDTH, 354.0), &jobs, &not_one, 0.0);
    assert_eq!(t.iter().filter(|s| s.contains("Menos detalles")).count(), 1, "{t:?}");
}

#[test]
fn scroll_thumb_size_and_position() {
    // Todo entra: sin barra.
    assert_eq!(view::thumb(500.0, 640.0, 0.0), None);
    assert_eq!(view::thumb(640.0, 640.0, 0.0), None);
    // El doble de contenido: el pulgar ocupa la mitad del riel y baja con el desplazamiento.
    assert_eq!(view::thumb(1280.0, 640.0, 0.0), Some((0.0, 320.0)));
    assert_eq!(view::thumb(1280.0, 640.0, 320.0), Some((160.0, 320.0)));
    assert_eq!(view::thumb(1280.0, 640.0, 640.0), Some((320.0, 320.0)));
    // Pasado el límite, el pulgar queda al fondo.
    assert_eq!(view::thumb(1280.0, 640.0, 9000.0), Some((320.0, 320.0)));
    // Mucho contenido: el pulgar no baja de 24.
    assert_eq!(view::thumb(64_000.0, 640.0, 0.0).map(|t| t.1), Some(24.0));
    // Al revés: dónde queda el contenido si el pulgar se arrastra a una altura.
    assert_eq!(view::scroll_for_thumb(1280.0, 640.0, 160.0), 320.0);
    assert_eq!(view::scroll_for_thumb(1280.0, 640.0, -50.0), 0.0);
    assert_eq!(view::scroll_for_thumb(1280.0, 640.0, 1000.0), 640.0);
    assert_eq!(view::max_scroll(1280.0, 640.0), 640.0);
    assert_eq!(view::max_scroll(300.0, 640.0), 0.0);
}

/// Con el contenido desplazado, los clics y el dibujo van al trabajo que se ve en ese lugar.
#[test]
fn hit_and_draw_follow_the_scroll() {
    let jobs = [running(1), running(2), running(3)];
    let all = |_: u64| true;
    let r = rect(0.0, 0.0, view::WIDTH, 300.0);
    // Sin desplazar, el 3 no se ve; desplazado al bloque del 3, sí, y el 1 ya no.
    assert!(center_in(r, &jobs, &all, 0.0, ViewHit::Cancel(3)).is_none());
    assert!(center_in(r, &jobs, &all, 0.0, ViewHit::Cancel(1)).is_some());
    assert!(center_in(r, &jobs, &all, 580.0, ViewHit::Cancel(3)).is_some());
    assert!(center_in(r, &jobs, &all, 580.0, ViewHit::Cancel(1)).is_none());
    // El mismo punto de la ventana es el «Cancelar» del 2 con el contenido corrido un bloque.
    let (x, y) = center_in(r, &jobs, &all, 0.0, ViewHit::Cancel(1)).unwrap();
    assert_eq!(view::hit(r, &jobs, &all, 290.0, x, y), Some(ViewHit::Cancel(2)));
    // Fuera del área visible no hay nada.
    assert_eq!(view::hit(r, &jobs, &all, 0.0, x, y + 300.0), None);
    // El bloque de cada trabajo, en coordenadas del contenido.
    assert_eq!(view::block_span(&jobs, &all, 2), Some((290.0, 580.0)));
    assert_eq!(view::block_span(&jobs, &all, 9), None);
    // Solo se dibuja lo que se ve.
    let t = texts_in(r, &jobs, &all, 580.0);
    assert_eq!(t.iter().filter(|s| s.contains("% completado")).count(), 1, "{t:?}");
    // La barra a la derecha: el riel ocupa el alto visible y el pulgar está al fondo.
    let (track, thumb) = view::scrollbar(r, view::content_height(&jobs, &all), 570.0).expect("barra");
    assert_eq!((track.top, track.bottom, track.right), (0.0, 300.0, view::WIDTH));
    assert_eq!(thumb.bottom, 300.0);
    // Sobre el riel no hay controles de los trabajos.
    assert_eq!(view::hit(r, &jobs, &all, 0.0, view::WIDTH - 2.0, y), None);
    assert!(view::scrollbar(rect(0.0, 0.0, view::WIDTH, 900.0), 870.0, 0.0).is_none());
}

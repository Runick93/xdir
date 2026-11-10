//! Pruebas de la vista de la ventana de copia (`transfer::view`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::speed::Speed;
use super::view::{self, ViewHit};
use super::{Choice, ConflictInfo, JobError, JobState, JobView};
use crate::geom::{Rect, rect};
use crate::sys::testing::{FakeEnv, RecordingPainter};
use crate::sys::{ItemKind, ItemStat};
use crate::theme::{DEFAULT_ACCENT, Theme};

const MB: f64 = 1024.0 * 1024.0;

fn job(id: u64, state: JobState) -> JobView {
    let now = Instant::now();
    JobView {
        id,
        title: format!("Copiando 1.284 elementos de \"Descargas\" a \"Respaldo\" ({id})"),
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

fn area(jobs: &[JobView], expanded: Option<u64>) -> Rect {
    rect(0.0, 0.0, view::WIDTH, view::content_height(jobs, expanded))
}

fn texts(jobs: &[JobView], expanded: Option<u64>) -> Vec<String> {
    let p = RecordingPainter::new(theme());
    view::draw(&p, area(jobs, expanded), jobs, expanded, None, &FakeEnv);
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
        remaining: 12,
    });
    j
}

/// Centro del control `h` (lo busca recorriendo el área).
fn center_of(jobs: &[JobView], expanded: Option<u64>, h: ViewHit) -> Option<(f32, f32)> {
    let r = area(jobs, expanded);
    let mut y = 0.0;
    while y < r.bottom {
        let mut x = 0.0;
        while x < r.right {
            if view::hit(r, jobs, expanded, x, y) == Some(h) {
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
    assert_eq!(view::content_height(&jobs, Some(1)), 290.0);
    assert_eq!(view::content_height(&jobs, None), 64.0);
}

#[test]
fn queued_row_says_en_cola() {
    let jobs = [job(1, JobState::Waiting)];
    let t = texts(&jobs, None);
    assert!(t.iter().any(|s| s == "En cola"), "{t:?}");
    assert_eq!(view::content_height(&jobs, None), 56.0);
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
    view::draw(&p, area(&jobs, Some(1)), &jobs, Some(1), None, &FakeEnv);
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
    for s in ["Ya existe «informe.pdf» en «Respaldo»", "El que se copia", "El que ya está", "Más nuevo", "Reemplazar", "Conservar ambos", "Omitir", "Hacer esto con todos (quedan 12 conflictos)"] {
        assert!(has(&t, s), "falta {s:?} en {t:?}");
    }
    assert!(!has(&t, "Combinar"));
    assert!(has(&t, "2,09 MB") && has(&t, "1,89 MB"), "tamaños: {t:?}");
    assert!(center_of(&file, None, ViewHit::Choose(1, Choice::Merge)).is_none());
    assert!(center_of(&file, None, ViewHit::Choose(1, Choice::KeepBoth)).is_some());
    assert_eq!(view::content_height(&file, None), 316.0);
    let folder = [conflict(1, true)];
    let t = texts(&folder, None);
    assert!(has(&t, "Combinar"), "{t:?}");
    assert!(center_of(&folder, None, ViewHit::Choose(1, Choice::Merge)).is_some());
    assert!(center_of(&folder, None, ViewHit::ApplyAll(1)).is_some());
}

fn error(name: &str, why: &str, retry: bool) -> JobError {
    JobError { src: PathBuf::from(format!("/d/Descargas/{name}")), dst: PathBuf::from(format!("/r/{name}")), message: why.into(), retry }
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
    assert_eq!(view::content_height(&jobs, None), 76.0 + 44.0 * 3.0 + 16.0 + 45.0);
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

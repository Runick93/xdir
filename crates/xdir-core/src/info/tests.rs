//! Pruebas de lo neutro del panel de información: mezcla, permisos, tamaño de carpetas, lectura
//! mínima y armado del panel.

use super::draw::{self, Body, Header, InfoHit, Panel, Prim, Row, SectionView};
use super::size::folder_size;
use super::*;
use crate::geom::rect;
use std::sync::atomic::AtomicBool;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("xdir-test-info").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn item(name: &str, hidden: bool, tags: &[&str]) -> ItemInfo {
    ItemInfo {
        path: PathBuf::from(name),
        kind_name: "Imagen JPEG".into(),
        size: 100,
        on_disk: 4096,
        flags: vec![(Flag::Hidden, hidden, true), (Flag::Locked, false, true)],
        tags: Some(tags.iter().map(|t| Tag { name: t.to_string(), color: None }).collect()),
        ..ItemInfo::default()
    }
}

/// Dos archivos, uno oculto: Oculto mezclado, Bloqueado apagado; etiquetas comunes; tipo común.
#[test]
fn info_multi_merge_tri() {
    let s = merge(&[item("a.jpg", true, &["Rojo", "Viaje"]), item("b.jpg", false, &["Viaje"])]);
    assert_eq!(s.count, 2);
    assert_eq!((s.size, s.on_disk), (200, 8192));
    assert_eq!(s.kind.as_deref(), Some("Imagen JPEG"));
    assert_eq!(s.flags, [(Flag::Hidden, Tri::Mixed, true), (Flag::Locked, Tri::Off, true)]);
    assert_eq!(s.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Viaje"]);
    let mut other = item("c.txt", true, &[]);
    other.kind_name = "Documento de texto".into();
    let s = merge(&[item("a.jpg", true, &[]), other]);
    assert_eq!(s.kind, None);
    assert_eq!(s.flags[0].1, Tri::On);
}

#[test]
fn access_bits() {
    assert_eq!(access_of(0o640, 0), Access::ReadWrite);
    assert_eq!(access_of(0o640, 1), Access::ReadOnly);
    assert_eq!(access_of(0o640, 2), Access::None);
    assert_eq!(with_access(0o755, 2, Access::None), 0o751);
    assert_eq!(with_access(0o644, 1, Access::ReadWrite), 0o664);
    assert_eq!(with_access(0o4755, 0, Access::ReadOnly), 0o4555);
    assert_eq!(rw_text(true, false), "Solo lectura");
}

/// La suma no entra en las carpetas solo en la nube (las cuenta, sin abrirlas) y avisa cada 500.
#[test]
fn folder_size_skips_cloud_dirs() {
    let d = dir("suma");
    std::fs::create_dir_all(d.join("local/sub")).unwrap();
    std::fs::create_dir_all(d.join("nube")).unwrap();
    std::fs::write(d.join("local/a.bin"), [0u8; 10]).unwrap();
    std::fs::write(d.join("local/sub/b.bin"), [0u8; 20]).unwrap();
    std::fs::write(d.join("nube/no.bin"), [0u8; 1000]).unwrap();
    std::fs::write(d.join("c.bin"), [0u8; 5]).unwrap();
    let cloud = d.join("nube");
    let looked = std::cell::RefCell::new(Vec::new());
    let mut last = (0, 0, 0);
    let skipped = folder_size(&d, &|p| {
        looked.borrow_mut().push(p.to_path_buf());
        p == cloud
    }, &AtomicBool::new(false), &mut |b, f, fo| last = (b, f, fo));
    assert_eq!(last, (35, 3, 3));
    assert_eq!(skipped, 0);
    assert!(looked.borrow().contains(&cloud));
    // 1.200 archivos: avisa a los 500 y 1.000, y al final.
    let d = dir("pasos");
    for i in 0..1200 {
        std::fs::write(d.join(format!("{i}")), b"x").unwrap();
    }
    let mut steps = Vec::new();
    folder_size(&d, &|_| false, &AtomicBool::new(false), &mut |b, f, _| steps.push((b, f)));
    assert_eq!(steps, [(500, 500), (1000, 1000), (1200, 1200)]);
    // Cancelada: termina enseguida con lo que llevaba.
    let mut calls = 0;
    folder_size(&d, &|_| false, &AtomicBool::new(true), &mut |_, f, _| {
        calls += 1;
        assert_eq!(f, 0);
    });
    assert_eq!(calls, 1);
}

/// Cancelada, no abre ninguna carpeta más (ni la primera): se mira antes de cada `read_dir`, no
/// cada 500 elementos.
#[test]
fn folder_size_cancel_checked_per_directory() {
    let d = dir("corta-por-carpeta");
    // No existe: si se intentara leer, contaría como sin permiso.
    let missing = d.join("no-existe");
    let skipped = folder_size(&missing, &|_| false, &AtomicBool::new(true), &mut |_, _, _| {});
    assert_eq!(skipped, 0, "no se abrió nada");
    // Se corta en medio: después de la primera carpeta no se entra a otra.
    for sub in ["a", "b", "c"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
        std::fs::write(d.join(sub).join("x.bin"), [0u8; 4]).unwrap();
    }
    let ctl = AtomicBool::new(false);
    let mut last = (0, 0, 0);
    folder_size(&d, &|_| {
        ctl.store(true, std::sync::atomic::Ordering::Relaxed);
        false
    }, &ctl, &mut |b, f, fo| last = (b, f, fo));
    assert_eq!(last, (0, 0, 1), "solo vio la primera carpeta, sin entrar");
}

/// Las carpetas que no se pueden leer se cuentan aparte (para avisar que faltan).
#[cfg(unix)]
#[test]
fn folder_size_counts_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("sin-permiso");
    std::fs::create_dir_all(d.join("cerrada")).unwrap();
    std::fs::write(d.join("cerrada/a"), b"x").unwrap();
    std::fs::set_permissions(d.join("cerrada"), std::fs::Permissions::from_mode(0o000)).unwrap();
    // Con permisos de administrador (Docker) se lee igual: no hay qué probar.
    let readable = std::fs::read_dir(d.join("cerrada")).is_ok();
    let skipped = folder_size(&d, &|_| false, &AtomicBool::new(false), &mut |_, _, _| {});
    std::fs::set_permissions(d.join("cerrada"), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(skipped, if readable { 0 } else { 1 });
}

/// Lo que está solo en la nube se informa así y se lee sin seguir el enlace.
#[test]
fn read_minimal_cloud_and_dates() {
    let d = dir("minimo");
    std::fs::write(d.join("a.txt"), b"hola").unwrap();
    let i = read_minimal(&d.join("a.txt"), &|_| false);
    assert_eq!((i.size, i.is_dir, i.cloud_only), (4, false, false));
    assert!(i.modified > 0 && i.created > 0);
    let i = read_minimal(&d.join("a.txt"), &|_| true);
    assert!(i.cloud_only);
    let i = read_minimal(&d, &|_| false);
    assert!(i.is_dir);
    assert_eq!(filetime_from_unix(0), 0);
    assert_eq!(filetime_from_unix(1), 116_444_736_010_000_000);
}

fn measure(s: &str) -> f32 {
    7.0 * s.chars().count() as f32
}

fn panel(sections: Vec<SectionView>) -> Panel {
    Panel {
        title: "Información de «IMG_3101.jpg»".into(),
        header: Header { key: None, icon: 1, image: false, name: "IMG_3101.jpg".into(), lines: vec![("Imagen JPEG".into(), false), ("3,8 MB".into(), false)] },
        sections,
        more_button: true,
        eject_button: false,
    }
}

fn sv(section: Section, open: bool, body: Body) -> SectionView {
    SectionView { section, open, error: None, body }
}

fn texts(l: &draw::Layout) -> Vec<String> {
    l.fixed.iter().chain(&l.content).filter_map(|p| if let Prim::Text { s, .. } = p { Some(s.clone()) } else { None }).collect()
}

/// Medidas del diseño: barra de 40, encabezado de 100, cabeceras de 32, valor a 112 del borde,
/// «Más en el sistema…» de 150×28 a la derecha.
#[test]
fn layout_measures() {
    let r = rect(1074.0, 3.0, 1394.0, 790.0);
    let p = panel(vec![
        sv(Section::Tags, true, Body::Tags(Some(vec![Tag { name: "Rojo".into(), color: Some(0xFF0000) }]))),
        sv(Section::General, true, Body::General { capacity: None, rows: vec![Row::new("Tipo", "Imagen JPEG")], flags: vec![(Flag::Hidden, Tri::Off, true)] }),
        sv(Section::Comments, false, Body::Comment(Some(String::new()))),
    ]);
    let l = draw::layout(r, &p, 0.0, &measure);
    let find = |s: &str| {
        l.content.iter().find_map(|p| match p {
            Prim::Text { s: t, r, .. } if t == s => Some(*r),
            _ => None,
        })
    };
    assert_eq!(find("IMG_3101.jpg").unwrap().left, 1074.0 + 16.0 + 72.0 + 12.0);
    let tags = find("Etiquetas").unwrap();
    assert_eq!(tags.top, 3.0 + 40.0 + 100.0 + 8.0);
    assert_eq!(find("Imagen JPEG").map(|r| r.left), Some(1074.0 + 16.0 + 72.0 + 12.0));
    let value = l.content.iter().filter_map(|p| if let Prim::Text { s, r, .. } = p { (s == "Imagen JPEG").then_some(*r) } else { None }).next_back().unwrap();
    assert_eq!(value.left, 1074.0 + 112.0);
    let (_, more) = l.hits.iter().find(|(h, _)| *h == InfoHit::MoreInSystem).unwrap();
    assert_eq!((more.right - more.left, more.bottom - more.top, more.right), (150.0, 28.0, 1394.0 - 16.0));
    // Plegada: sin el cuerpo.
    assert!(find("Agregar un comentario").is_none());
    assert_eq!(draw::hit(&l, 1380.0, 20.0), Some(InfoHit::Close));
    assert_eq!(draw::hit(&l, 1200.0, tags.top + 4.0), Some(InfoHit::Toggle(Section::Tags)));
    assert_eq!(draw::hit(&l, 10.0, 20.0), None);
}

/// Con más contenido que alto, se desplaza: lo que queda debajo de la barra superior no se toca.
#[test]
fn layout_scrolls_under_top_bar() {
    let r = rect(0.0, 0.0, 320.0, 300.0);
    let rows: Vec<Row> = (0..30).map(|i| Row::new(format!("R{i}"), "v")).collect();
    let p = panel(vec![sv(Section::More, true, Body::Rows { rows, note: None })]);
    let l = draw::layout(r, &p, 10_000.0, &measure);
    assert!(l.content_h > 300.0);
    assert_eq!(l.scroll, l.max_scroll());
    let first = draw::layout(r, &p, 0.0, &measure);
    let toggle = first.hits.iter().find(|(h, _)| *h == InfoHit::Toggle(Section::More)).unwrap().1;
    let l = draw::layout(r, &p, 160.0, &measure);
    // La cabecera quedó debajo de la barra: ya no responde.
    assert!(!l.hits.iter().any(|(h, _)| *h == InfoHit::Toggle(Section::More)));
    assert!(toggle.top - 160.0 < 40.0);
    assert!(texts(&l).contains(&"R10".to_string()));
}

/// «⏏ Expulsar» a la izquierda, en la misma fila que «Más en el sistema…»; solo si se pide.
#[test]
fn layout_eject_button() {
    let r = rect(0.0, 0.0, 320.0, 790.0);
    let mut p = panel(Vec::new());
    let l = draw::layout(r, &p, 0.0, &measure);
    assert!(!l.hits.iter().any(|(h, _)| *h == InfoHit::Eject));
    assert!(!texts(&l).contains(&draw::EJECT_TEXT.to_string()));
    p.eject_button = true;
    let l = draw::layout(r, &p, 0.0, &measure);
    let (_, eject) = *l.hits.iter().find(|(h, _)| *h == InfoHit::Eject).expect("⏏");
    let (_, more) = *l.hits.iter().find(|(h, _)| *h == InfoHit::MoreInSystem).unwrap();
    assert_eq!((eject.left, eject.top, eject.bottom), (16.0, more.top, more.bottom));
    assert!(eject.right < more.left);
    assert!(texts(&l).contains(&draw::EJECT_TEXT.to_string()));
    assert_eq!(draw::hit(&l, eject.left + 4.0, eject.top + 4.0), Some(InfoHit::Eject));
    // Sin «Más en el sistema…» (Linux) igual aparece.
    p.more_button = false;
    let l = draw::layout(r, &p, 0.0, &measure);
    assert!(l.hits.iter().any(|(h, _)| *h == InfoHit::Eject));
}

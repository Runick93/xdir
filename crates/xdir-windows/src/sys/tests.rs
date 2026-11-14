use xdir_core::commands::{ALL, Command, available_in, combo_label_in, parse_combo};
use xdir_core::sys::SystemEnv;

// Las pruebas con el sistema real solo corren en Windows; la de la tabla contra el entorno falso
// corre en todos (fuera de Windows, `main.rs` incluye este archivo suelto).
#[cfg(windows)]
use super::{find_wt, platform_env, platform_file_system, platform_preview, windows_consoles, wt_params};

/// El menú de terminales empieza como antes (PowerShell, CMD); después Windows Terminal si está
/// instalada en esta máquina.
#[test]
#[cfg(windows)]
fn consoles_menu_same_entries() {
    let env = platform_env();
    let labels: Vec<&str> = env.consoles().iter().map(|c| c.label).collect();
    let verbs = env.console_alt_verbs();
    assert_eq!(labels[..2], ["PowerShell", "CMD"]);
    assert!(labels[2..].iter().all(|l| *l == "Windows Terminal"), "{labels:?}");
    assert_eq!(verbs, [("runas", "Ejecutar como administrador"), ("runasuser", "Ejecutar como otro usuario")]);
}

/// Windows Terminal aparece si `wt.exe` está en el PATH o en `%LOCALAPPDATA%\Microsoft\WindowsApps`.
#[test]
#[cfg(windows)]
fn windows_terminal_listed_when_present() {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    let path = OsStr::new(r"C:\Windows\System32;C:\Herramientas");
    let local = Path::new(r"C:\Users\ana\AppData\Local");
    let labels = |exists: &dyn Fn(&Path) -> bool| {
        windows_consoles(Some(path), Some(local), exists).iter().map(|c| c.label).collect::<Vec<_>>()
    };
    assert_eq!(labels(&|p| p == Path::new(r"C:\Herramientas\wt.exe")), ["PowerShell", "CMD", "Windows Terminal"]);
    let alias = Path::new(r"C:\Users\ana\AppData\Local\Microsoft\WindowsApps\wt.exe");
    assert_eq!(labels(&|p| p == alias), ["PowerShell", "CMD", "Windows Terminal"]);
    // El del PATH gana.
    assert_eq!(find_wt(Some(path), Some(local), &|_| true), Some(PathBuf::from(r"C:\Windows\System32\wt.exe")));
    // La carpeta se pasa con -d entre comillas; la barra final se duplica para no escapar la comilla.
    assert_eq!(wt_params(Path::new(r"C:\Mis cosas")), r#"-d "C:\Mis cosas""#);
    assert_eq!(wt_params(Path::new(r"C:\")), r#"-d "C:\\""#);
    // El ; separa comandos de wt: se escapa con \;.
    assert_eq!(wt_params(Path::new(r"C:\a;b")), r#"-d "C:\a\;b""#);
}

#[test]
#[cfg(windows)]
fn windows_terminal_not_listed_when_absent() {
    use std::ffi::OsStr;
    use std::path::Path;
    let labels: Vec<_> = windows_consoles(Some(OsStr::new(r"C:\Windows")), Some(Path::new(r"C:\L")), &|_| false).iter().map(|c| c.label).collect();
    assert_eq!(labels, ["PowerShell", "CMD"]);
    assert_eq!(windows_consoles(None, None, &|_| true).len(), 2);
}

/// Foto de la tabla completa de comandos con el entorno real: id, atajos, título, cómo se ven los
/// atajos y si existe. Protege la tabla de atajos y nombres de Windows (la de macOS se prueba en
/// `crates/xdir-mac`).
fn table(env: &dyn SystemEnv) -> String {
    ALL.iter()
        .map(|s| {
            let labels: Vec<String> = s.keys_in(env).iter().map(|k| combo_label_in(env.key_style(), parse_combo(k).unwrap())).collect();
            let exists = available_in(env, s.command) || s.command == Command::ToggleTree;
            format!("{}|{}|{}|{}|{}\n", s.id, s.keys_in(env).join(","), s.title_in(env), labels.join(","), exists)
        })
        .collect()
}

#[test]
#[cfg(windows)]
fn windows_table_is_unchanged() {
    assert_eq!(table(&*platform_env()), WINDOWS_TABLE);
}

/// La tabla de Windows se confirma desde cualquier sistema: el entorno falso no tiene atajos ni
/// nombres propios y usa el estilo de Windows, igual que el entorno real de Windows.
#[test]
fn windows_table_matches_the_fake_env() {
    assert_eq!(table(&xdir_core::sys::testing::FakeEnv), WINDOWS_TABLE);
}

const WINDOWS_TABLE: &str = "ir_a_carpeta|Ctrl+P|Ir a una carpeta…|Ctrl+P|true\npaleta|Ctrl+Shift+P|Paleta de comandos|Ctrl+Shift+P|true\npestana_nueva|Ctrl+T|Pestaña nueva|Ctrl+T|true\ncerrar_pestana|Ctrl+W|Cerrar pestaña|Ctrl+W|true\nreabrir_pestana|Ctrl+Shift+T|Reabrir la última pestaña cerrada|Ctrl+Shift+T|true\npestana_siguiente|Ctrl+AvPag|Pestaña siguiente|Ctrl+AvPág|true\npestana_anterior|Ctrl+RePag|Pestaña anterior|Ctrl+RePág|true\ngrupo_siguiente|Ctrl+Tab|Grupo siguiente|Ctrl+Tab|true\ngrupo_anterior|Ctrl+Shift+Tab|Grupo anterior|Ctrl+Shift+Tab|true\npanel_siguiente|Tab|Panel siguiente|Tab|true\npanel_anterior|Shift+Tab|Panel anterior|Shift+Tab|true\npanel_1|Alt+1|Ir al panel 1|Alt+1|true\npanel_2|Alt+2|Ir al panel 2|Alt+2|true\npanel_3|Alt+3|Ir al panel 3|Alt+3|true\npanel_4|Alt+4|Ir al panel 4|Alt+4|true\nlayout_1|Ctrl+1|Layout: 1 panel|Ctrl+1|true\nlayout_2_lado_a_lado|Ctrl+2|Layout: 2 paneles lado a lado|Ctrl+2|true\nlayout_2_apilados|Ctrl+3|Layout: 2 paneles apilados|Ctrl+3|true\nlayout_3|Ctrl+4|Layout: 3 paneles|Ctrl+4|true\nlayout_4|Ctrl+5|Layout: 4 paneles|Ctrl+5|true\nmaximizar_panel|Ctrl+M|Maximizar o restaurar el panel|Ctrl+M|true\natras|Alt+Izquierda,NavegadorAtras|Atrás|Alt+←,Botón atrás del mouse|true\nadelante|Alt+Derecha,NavegadorAdelante|Adelante|Alt+→,Botón adelante del mouse|true\nsubir|Alt+Arriba,Retroceso|Subir a la carpeta superior|Alt+↑,Retroceso|true\nrecargar|F5|Recargar|F5|true\nescribir_ruta|Ctrl+L,F4,Alt+D|Escribir una ruta|Ctrl+L,F4,Alt+D|true\nfiltrar|Ctrl+F|Buscar en la carpeta y sus subcarpetas|Ctrl+F|true\neste_equipo||Ir a Este equipo||true\nmostrar_ocultos|Ctrl+H|Mostrar u ocultar archivos ocultos|Ctrl+H|true\nseleccionar_todo|Ctrl+A|Seleccionar todo|Ctrl+A|true\ninvertir_seleccion|Ctrl+I|Invertir selección|Ctrl+I|true\nseleccionar_patron|Ctrl+Más,Ctrl+MásNum|Seleccionar por patrón…|Ctrl+Más,Ctrl+Más (teclado numérico)|true\ndeseleccionar_patron|Ctrl+Menos,Ctrl+MenosNum|Deseleccionar por patrón…|Ctrl+Menos,Ctrl+Menos (teclado numérico)|true\ncopiar|Ctrl+C|Copiar|Ctrl+C|true\ncortar|Ctrl+X|Cortar|Ctrl+X|true\npegar|Ctrl+V|Pegar|Ctrl+V|true\ncopiar_rutas|Ctrl+Shift+C|Copiar las rutas como texto|Ctrl+Shift+C|true\ndeshacer|Ctrl+Z|Deshacer|Ctrl+Z|true\ncarpeta_nueva|Ctrl+Shift+N|Carpeta nueva|Ctrl+Shift+N|true\narchivo_texto_nuevo|Ctrl+Alt+N|Archivo de texto nuevo|Ctrl+Alt+N|true\ncarpeta_con_seleccion|Ctrl+Alt+Shift+N|Carpeta nueva con la selección|Ctrl+Shift+Alt+N|true\npapelera|Supr|Enviar a la papelera|Supr|true\neliminar|Shift+Supr|Eliminar definitivamente|Shift+Supr|true\ndevolver||Devolver||true\ncrear_acceso_directo||Crear acceso directo||true\nmostrar_original||Mostrar el original||true\ncomprimir||Comprimir||true\nextraer_aqui||Extraer aquí||true\nextraer_en||Extraer en una carpeta||true\nexpulsar||Expulsar||true\nconectar_servidor|Ctrl+K|Conectar a un servidor…|Ctrl+K|true\nconectar_unidad_red||Conectar unidad de red…||true\ndesconectar_unidad_red||Desconectar unidad de red…||true\nrenombrar|F2|Renombrar|F2|true\nrenombrar_varios||Renombrar varios…||true\ncomparar_contenido||Comparar contenido||true\npropiedades|Alt+Enter|Propiedades|Alt+Enter|true\nduplicar|Ctrl+D|Duplicar|Ctrl+D|true\nfavorito|Ctrl+Shift+D|Agregar o quitar de favoritos|Ctrl+Shift+D|true\nbarra_favoritos|Ctrl+Shift+B|Mostrar u ocultar la barra de favoritos|Ctrl+Shift+B|true\npowershell||Abrir PowerShell en la carpeta||true\ncmd||Abrir CMD en la carpeta||true\nmodo_oscuro||Cambiar entre modo claro y oscuro||true\nguardar_grupo||Guardar paneles y carpetas como grupo…||true\nbarra_grupos||Mostrar u ocultar la barra de grupos||true\nabrir_configuracion||Abrir config.toml||true\nconfiguracion|Ctrl+Coma|Configuración|Ctrl+,|true\nconfiguracion_atajos||Configuración: Atajos de teclado||true\nvista_previa|Alt+P|Mostrar u ocultar la vista previa|Alt+P|true\nvista_rapida|Espacio|Vista rápida|Espacio|true\nmostrar_operaciones||Mostrar operaciones||true\nxtree||Mapa de uso de disco (xtree)||true\nxtree_colores|Ctrl+T|xtree: colorear por carpeta o por tipo de archivo|Ctrl+T|true\nxtree_permisos|Ctrl+Shift+A|xtree: analizar de nuevo con permisos|Ctrl+Shift+A|true\nacerca_de||Acerca de xdir (versión y reportar un problema)||true\nnovedades||Novedades de xdir||true\nvista_detalles|Ctrl+Shift+1|Vista: detalles|Ctrl+Shift+1|true\nvista_iconos|Ctrl+Shift+2|Vista: íconos medianos|Ctrl+Shift+2|true\nvista_iconos_grandes|Ctrl+Shift+3|Vista: íconos grandes|Ctrl+Shift+3|true\nvista_miniaturas|Ctrl+Shift+4|Vista: miniaturas|Ctrl+Shift+4|true\nvista_lista|Ctrl+Shift+5|Vista: lista|Ctrl+Shift+5|true\nvista_contenido|Ctrl+Shift+6|Vista: contenido|Ctrl+Shift+6|true\nvista_galeria|Ctrl+Shift+7|Vista: galería|Ctrl+Shift+7|true\nordenar_nombre||Ordenar por nombre||true\nordenar_modificado||Ordenar por fecha de modificación||true\nordenar_creado||Ordenar por fecha de creación||true\nordenar_tipo||Ordenar por tipo||true\nordenar_extension||Ordenar por extensión||true\nordenar_tamano||Ordenar por tamaño||true\nagrupar_nombre||Agrupar por nombre||true\nagrupar_modificado||Agrupar por fecha de modificación||true\nagrupar_creado||Agrupar por fecha de creación||true\nagrupar_tipo||Agrupar por tipo||true\nagrupar_tamano||Agrupar por tamaño||true\nagrupar_dimensiones||Agrupar por dimensiones||true\nagrupar_duracion||Agrupar por duración||true\nno_agrupar||No agrupar||true\nvista_en_todas||Usar esta vista en todas las carpetas||true\nrestablecer_vista||Restablecer la vista de esta carpeta||true\nquitar_filtros||Quitar los filtros||true\narbol|Ctrl+Shift+E|Mostrar u ocultar el árbol de carpetas (experimental)|Ctrl+Shift+E|true\n";


/// Título y migas de la raíz y de una carpeta con el sistema de archivos real: la unidad se nombra
/// como hoy («Disco local (C:)»).
#[test]
#[cfg(windows)]
fn location_titles_per_os() {
    use crate::fs::Location;
    use std::path::PathBuf;
    let env = platform_file_system();
    let (root, users, drive) = (r"C:\", r"C:\Users", env.drive_label("C:"));
    let root = Location::Path(PathBuf::from(root));
    let users = Location::Path(PathBuf::from(users));
    assert_eq!(root.title_in(&*env), drive);
    assert_eq!(users.title_in(&*env), "Users");
    assert_eq!(
        users.crumbs_in(&*env),
        [("Este equipo".to_string(), Location::Computer), (drive.clone(), root.clone()), ("Users".to_string(), users.clone())]
    );
    assert_eq!(root.crumbs_in(&*env), [("Este equipo".to_string(), Location::Computer), (drive.clone(), root.clone())]);
    assert!(drive.ends_with("(C:)"), "{drive}");
}

/// Corre una búsqueda con el sistema de archivos real de Windows y junta lo encontrado (rutas
/// relativas a `root`, ordenadas) y el último avance.
#[cfg(windows)]
fn search_real(root: &std::path::Path, query: &str, show_hidden: bool) -> (Vec<String>, f32) {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use xdir_core::fs::{LoadMsg, search_with};
    // La búsqueda avisa a la ventana con `Handle::post`: el `Waker` falso solo cuenta.
    xdir_core::sys::testing::install_fakes();
    let fs: &'static dyn xdir_core::sys::FileSystem = Box::leak(platform_file_system());
    let (tx, rx) = std::sync::mpsc::channel();
    let current = Arc::new(AtomicU64::new(7));
    search_with(fs, crate::platform::Handle(0), tx, 1, 7, current, vec![root.to_path_buf()], show_hidden, xdir_core::list::name_matcher(query));
    let (mut names, mut progress) = (Vec::new(), 0.0);
    while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
        match ev.msg {
            LoadMsg::Entries(batch) => {
                for e in batch {
                    let full = std::path::PathBuf::from(e.full.expect("con ruta completa"));
                    names.push(full.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
            LoadMsg::Progress { done, .. } => progress = done,
            LoadMsg::Listed(_) => break,
            _ => {}
        }
    }
    names.sort();
    (names, progress)
}

/// La búsqueda en subcarpetas con los ganchos reales de Windows: ocultos por atributo, enlaces y
/// paquetes, y las entradas con ruta completa.
#[test]
#[cfg(windows)]
fn search_finds_matches_with_the_real_file_system() {
    let root = std::env::temp_dir().join(format!("xdir-busqueda-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["a/b/c", "informes/2024", ".oculta"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    for file in ["informe.txt", "a/b/c/Informe final.pdf", "a/nada.txt", "informes/2024/foto.jpg", ".oculta/informe.txt"] {
        std::fs::write(root.join(file), "x").unwrap();
    }
    // En Windows lo oculto es por atributo, no por el punto.
    assert!(std::process::Command::new("attrib").arg("+h").arg(root.join(".oculta")).status().unwrap().success());

    let (names, progress) = search_real(&root, "informe", false);
    // La carpeta "informes" también coincide; lo de la carpeta oculta no se ve.
    assert_eq!(names, ["a/b/c/Informe final.pdf", "informe.txt", "informes"]);
    assert_eq!(progress, 1.0);

    // Con patrones y mostrando ocultos.
    let (names, _) = search_real(&root, "*.txt", true);
    assert_eq!(names, [".oculta/informe.txt", "a/nada.txt", "informe.txt"]);

    let _ = std::fs::remove_dir_all(&root);
}

/// La vista previa responde desde su hilo con la lectura neutra; un archivo local no está en la
/// nube.
#[test]
#[cfg(windows)]
fn windows_preview_answers_from_thread() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use xdir_core::preview::{PreviewBody, PreviewRequest, PreviewTarget};
    xdir_core::sys::testing::install_fakes();
    let dir = std::env::temp_dir().join(format!("xdir-win-preview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notas.md");
    std::fs::write(&file, "# Hola\r\nmundo\r\n").unwrap();
    assert!(!platform_file_system().cloud_only(&file));
    let (tx, rx) = std::sync::mpsc::channel();
    let target = PreviewTarget::File(file);
    platform_preview().request(PreviewRequest { hwnd: Default::default(), tx, generation: 2, current: Arc::new(AtomicU64::new(2)), target });
    let reply = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("respuesta del hilo");
    assert_eq!(reply.generation, 2);
    assert_eq!(reply.info.body, PreviewBody::Text { lines: vec!["# Hola".into(), "mundo".into()] });
    let _ = std::fs::remove_dir_all(&dir);
}

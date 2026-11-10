use xdir_core::commands::{ALL, Command, available_in, combo_label_in, parse_combo};
use xdir_core::sys::SystemEnv;

use super::{mac_consoles, platform_env, platform_file_system, platform_preview};

/// El menú de terminales empieza con Terminal, como antes; después iTerm si está instalada en
/// esta máquina.
#[test]
fn consoles_menu_same_entries() {
    let env = platform_env();
    let labels: Vec<&str> = env.consoles().iter().map(|c| c.label).collect();
    let verbs = env.console_alt_verbs();
    assert_eq!(labels[0], "Terminal");
    assert!(labels[1..].iter().all(|l| *l == "iTerm"), "{labels:?}");
    assert!(verbs.is_empty());
}

/// iTerm aparece después de Terminal si está en /Applications o en ~/Applications.
#[test]
fn iterm_listed_when_present() {
    use std::path::Path;
    let home = Path::new("/Users/ana");
    let labels = |exists: &dyn Fn(&Path) -> bool| mac_consoles(Some(home), exists).iter().map(|c| c.label).collect::<Vec<_>>();
    assert_eq!(labels(&|p| p == Path::new("/Applications/iTerm.app")), ["Terminal", "iTerm"]);
    assert_eq!(labels(&|p| p == Path::new("/Users/ana/Applications/iTerm.app")), ["Terminal", "iTerm"]);
    assert_eq!(labels(&|_| false), ["Terminal"]);
    // Sin HOME solo se mira /Applications.
    assert_eq!(mac_consoles(None, &|p| p == Path::new("/Users/ana/Applications/iTerm.app")).len(), 1);
}

/// Foto de la tabla completa de comandos con el entorno real: id, atajos, título, cómo se ven los
/// atajos y si existe. Protege la tabla de atajos y nombres de macOS (la de Windows se prueba en
/// `crates/xdir-windows`).
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
fn mac_table_is_unchanged() {
    assert_eq!(table(&*platform_env()), MAC_TABLE);
}

/// Selección por patrón en macOS: ⌘+ y ⌘- en la tabla de atajos de macOS.
#[test]
fn pattern_commands_have_mac_keys() {
    let env = platform_env();
    let labels = |c: Command| -> Vec<String> {
        env.command_keys(c).expect("está en la tabla de macOS").iter().map(|k| combo_label_in(env.key_style(), parse_combo(k).unwrap())).collect()
    };
    assert_eq!(labels(Command::SelectPattern).first().map(String::as_str), Some("⌘+"));
    assert_eq!(labels(Command::DeselectPattern).first().map(String::as_str), Some("⌘-"));
}

/// Vistas en macOS: ⇧⌘3, ⇧⌘4 y ⇧⌘5 son las capturas de pantalla del sistema y no llegan a la
/// app, así que las 6 vistas van con ⌥⌘1…⌥⌘6, en el orden de `Command::View(n)` (n + 1).
#[test]
fn mac_view_shortcuts_avoid_screenshot_keys() {
    let env = platform_env();
    let screenshots = ["⇧⌘3", "⇧⌘4", "⇧⌘5"];
    for n in 0..6u8 {
        let keys = env.command_keys(Command::View(n)).expect("las vistas están en la tabla de macOS");
        let labels: Vec<String> = keys.iter().map(|k| combo_label_in(env.key_style(), parse_combo(k).unwrap())).collect();
        assert_eq!(labels, [format!("⌥⌘{}", n + 1)], "vista {n}");
        assert!(!labels.iter().any(|l| screenshots.contains(&l.as_str())));
    }
    // Ningún otro comando usa esas teclas.
    let views: Vec<_> = (1..=6).map(|n| parse_combo(&format!("Ctrl+Alt+{n}")).unwrap()).collect();
    for s in ALL.iter().filter(|s| !matches!(s.command, Command::View(_))) {
        for k in s.keys_in(&*env) {
            assert!(!views.contains(&parse_combo(k).unwrap()), "{} usa {k}", s.id);
        }
    }
}

/// En macOS no existen la consola CMD ni el Acceso rápido de Windows; el resto sí.
#[test]
fn mac_hides_cmd_and_quick_pin() {
    let env = platform_env();
    assert!(!available_in(&*env, Command::Cmd));
    assert!(!available_in(&*env, Command::ToggleQuickPin));
    assert!(available_in(&*env, Command::PowerShell));
    assert!(available_in(&*env, Command::Xtree));
}

const MAC_TABLE: &str = "ir_a_carpeta|Ctrl+P|Ir a una carpeta…|⌘P|true\npaleta|Ctrl+Shift+P|Paleta de comandos|⇧⌘P|true\npestana_nueva|Ctrl+T|Pestaña nueva|⌘T|true\ncerrar_pestana|Ctrl+W|Cerrar pestaña|⌘W|true\nreabrir_pestana|Ctrl+Shift+T|Reabrir la última pestaña cerrada|⇧⌘T|true\npestana_siguiente|Ctrl+AvPag|Pestaña siguiente|⌘⇟|true\npestana_anterior|Ctrl+RePag|Pestaña anterior|⌘⇞|true\ngrupo_siguiente|Ctrl+Tab|Grupo siguiente|⌘⇥|true\ngrupo_anterior|Ctrl+Shift+Tab|Grupo anterior|⇧⌘⇥|true\npanel_siguiente|Tab|Panel siguiente|⇥|true\npanel_anterior|Shift+Tab|Panel anterior|⇧⇥|true\npanel_1|Alt+1|Ir al panel 1|⌥1|true\npanel_2|Alt+2|Ir al panel 2|⌥2|true\npanel_3|Alt+3|Ir al panel 3|⌥3|true\npanel_4|Alt+4|Ir al panel 4|⌥4|true\nlayout_1|Ctrl+1|Layout: 1 panel|⌘1|true\nlayout_2_lado_a_lado|Ctrl+2|Layout: 2 paneles lado a lado|⌘2|true\nlayout_2_apilados|Ctrl+3|Layout: 2 paneles apilados|⌘3|true\nlayout_3|Ctrl+4|Layout: 3 paneles|⌘4|true\nlayout_4|Ctrl+5|Layout: 4 paneles|⌘5|true\nmaximizar_panel|Ctrl+M|Maximizar o restaurar el panel|⌘M|true\natras|Ctrl+[,Ctrl+Izquierda,NavegadorAtras|Atrás|⌘[,⌘←,Botón atrás del mouse|true\nadelante|Ctrl+],Ctrl+Derecha,NavegadorAdelante|Adelante|⌘],⌘→,Botón adelante del mouse|true\nsubir|Ctrl+Arriba,Retroceso|Subir a la carpeta superior|⌘↑,⌫|true\nrecargar|F5,Ctrl+R|Recargar|F5,⌘R|true\nescribir_ruta|Ctrl+L,Ctrl+Shift+G,F4|Escribir una ruta|⌘L,⇧⌘G,F4|true\nfiltrar|Ctrl+F|Buscar en la carpeta y sus subcarpetas|⌘F|true\neste_equipo||Ir a Este equipo||true\nmostrar_ocultos|Ctrl+Shift+.|Mostrar u ocultar archivos ocultos|⇧⌘.|true\nseleccionar_todo|Ctrl+A|Seleccionar todo|⌘A|true\ninvertir_seleccion|Ctrl+Alt+I|Invertir selección|⌥⌘I|true\nseleccionar_patron|Ctrl+Más,Ctrl+MásNum|Seleccionar por patrón…|⌘+,⌘+|true\ndeseleccionar_patron|Ctrl+Menos,Ctrl+MenosNum|Deseleccionar por patrón…|⌘-,⌘-|true\ncopiar|Ctrl+C|Copiar|⌘C|true\ncortar|Ctrl+X|Cortar|⌘X|true\npegar|Ctrl+V|Pegar|⌘V|true\ncopiar_rutas|Ctrl+Alt+C|Copiar las rutas como texto|⌥⌘C|true\ndeshacer|Ctrl+Z|Deshacer|⌘Z|true\ncarpeta_nueva|Ctrl+Shift+N|Carpeta nueva|⇧⌘N|true\narchivo_texto_nuevo|Ctrl+Alt+N|Archivo de texto nuevo|⌥⌘N|true\npapelera|Ctrl+Retroceso,Supr|Trasladar a la Papelera|⌘⌫,⌦|true\neliminar|Ctrl+Alt+Retroceso,Shift+Supr|Eliminar definitivamente|⌥⌘⌫,⇧⌦|true\nrenombrar|F2|Renombrar|F2|true\npropiedades|Ctrl+I|Obtener información|⌘I|true\nfavorito|Ctrl+D|Agregar o quitar de favoritos|⌘D|true\nbarra_favoritos|Ctrl+Shift+B|Mostrar u ocultar la barra de favoritos|⇧⌘B|true\npowershell||Abrir Terminal en la carpeta||true\ncmd||Abrir CMD en la carpeta||false\nmodo_oscuro||Cambiar entre modo claro y oscuro||true\nguardar_grupo||Guardar paneles y carpetas como grupo…||true\nbarra_grupos||Mostrar u ocultar la barra de grupos||true\nabrir_configuracion||Abrir la configuración (config.toml)||true\nxtree||Mapa de uso de disco (xtree)||true\nacerca_de||Acerca de xdir (versión y reportar un problema)||true\nvista_detalles|Ctrl+Alt+1|Vista: detalles|⌥⌘1|true\nvista_iconos|Ctrl+Alt+2|Vista: íconos medianos|⌥⌘2|true\nvista_iconos_grandes|Ctrl+Alt+3|Vista: íconos grandes|⌥⌘3|true\nvista_miniaturas|Ctrl+Alt+4|Vista: miniaturas|⌥⌘4|true\nvista_lista|Ctrl+Alt+5|Vista: lista|⌥⌘5|true\nvista_contenido|Ctrl+Alt+6|Vista: contenido|⌥⌘6|true\nordenar_nombre||Ordenar por nombre||true\nordenar_modificado||Ordenar por fecha de modificación||true\nordenar_creado||Ordenar por fecha de creación||true\nordenar_tipo||Ordenar por tipo||true\nordenar_extension||Ordenar por extensión||true\nordenar_tamano||Ordenar por tamaño||true\nagrupar_nombre||Agrupar por nombre||true\nagrupar_modificado||Agrupar por fecha de modificación||true\nagrupar_creado||Agrupar por fecha de creación||true\nagrupar_tipo||Agrupar por tipo||true\nagrupar_tamano||Agrupar por tamaño||true\nagrupar_dimensiones||Agrupar por dimensiones||true\nagrupar_duracion||Agrupar por duración||true\nno_agrupar||No agrupar||true\nvista_en_todas||Usar esta vista en todas las carpetas||true\nrestablecer_vista||Restablecer la vista de esta carpeta||true\nquitar_filtros||Quitar los filtros||true\narbol|Ctrl+Shift+E|Mostrar u ocultar el árbol de carpetas (experimental)|⇧⌘E|true\n";


/// Título y migas de la raíz y de una carpeta con el sistema de archivos real: la unidad se nombra
/// como hoy (el disco de arranque).
#[test]
fn location_titles_per_os() {
    use crate::fs::Location;
    use std::path::PathBuf;
    let env = platform_file_system();
    let (root, users, drive) = ("/", "/Users", env.drive_label("/"));
    let root = Location::Path(PathBuf::from(root));
    let users = Location::Path(PathBuf::from(users));
    assert_eq!(root.title_in(&*env), drive);
    assert_eq!(users.title_in(&*env), "Users");
    assert_eq!(
        users.crumbs_in(&*env),
        [("Este equipo".to_string(), Location::Computer), (drive.clone(), root.clone()), ("Users".to_string(), users.clone())]
    );
    assert_eq!(root.crumbs_in(&*env), [("Este equipo".to_string(), Location::Computer), (drive.clone(), root.clone())]);
    assert!(!drive.is_empty() && drive != "/", "{drive}");
}

/// Corre una búsqueda con el sistema de archivos real de macOS y junta lo encontrado
/// (rutas relativas a `root`, ordenadas) y el último avance.
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

/// La búsqueda en subcarpetas con los ganchos reales de macOS: ocultos por punto, enlaces y
/// paquetes, y las entradas con ruta completa.
#[test]
fn search_finds_matches_with_the_real_file_system() {
    let root = std::env::temp_dir().join(format!("xdir-busqueda-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["a/b/c", "informes/2024", ".oculta"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    for file in ["informe.txt", "a/b/c/Informe final.pdf", "a/nada.txt", "informes/2024/foto.jpg", ".oculta/informe.txt"] {
        std::fs::write(root.join(file), "x").unwrap();
    }
    // Lo que empieza con punto ya está oculto.

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
fn mac_preview_answers_from_thread() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use xdir_core::preview::{PreviewBody, PreviewRequest, PreviewTarget};
    xdir_core::sys::testing::install_fakes();
    let dir = std::env::temp_dir().join(format!("xdir-mac-preview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notas.md");
    std::fs::write(&file, "# Hola\nmundo\n").unwrap();
    assert!(!platform_file_system().cloud_only(&file));
    let (tx, rx) = std::sync::mpsc::channel();
    let target = PreviewTarget::File(file);
    platform_preview().request(PreviewRequest { hwnd: Default::default(), tx, generation: 2, current: Arc::new(AtomicU64::new(2)), target });
    let reply = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("respuesta del hilo");
    assert_eq!(reply.generation, 2);
    assert_eq!(reply.info.body, PreviewBody::Text { lines: vec!["# Hola".into(), "mundo".into()] });
    assert!(reply.info.accessed > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

use xdir_core::commands::{ALL, Command, available_in, combo_label_in, parse_combo};
use xdir_core::sys::SystemEnv;

use super::platform_env;

/// El menú de terminales muestra lo mismo que antes en cada sistema.
#[test]
fn consoles_menu_same_entries() {
    let env = platform_env();
    let labels: Vec<&str> = env.consoles().iter().map(|c| c.label).collect();
    let verbs = env.console_alt_verbs();
    if cfg!(windows) {
        assert_eq!(labels, ["PowerShell", "CMD"]);
        assert_eq!(verbs, [("runas", "Ejecutar como administrador"), ("runasuser", "Ejecutar como otro usuario")]);
    } else {
        assert_eq!(labels, ["Terminal"]);
        assert!(verbs.is_empty());
    }
}

/// Foto de la tabla completa de comandos con el entorno real: id, atajos, título, cómo se ven los
/// atajos y si existe. Protege las tablas de atajos y nombres por sistema.
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
    let expected = if cfg!(target_os = "macos") { MAC_TABLE } else { WINDOWS_TABLE };
    assert_eq!(table(&*platform_env()), expected);
}

/// La tabla de Windows se confirma desde cualquier sistema: el entorno falso no tiene atajos ni
/// nombres propios y usa el estilo de Windows, igual que el entorno real de Windows.
#[test]
fn windows_table_matches_the_fake_env() {
    assert_eq!(table(&xdir_core::sys::testing::FakeEnv), WINDOWS_TABLE);
}

/// En macOS no existen la consola CMD ni el Acceso rápido de Windows; el resto sí.
#[test]
#[cfg(target_os = "macos")]
fn mac_hides_cmd_and_quick_pin() {
    let env = platform_env();
    assert!(!available_in(&*env, Command::Cmd));
    assert!(!available_in(&*env, Command::ToggleQuickPin));
    assert!(available_in(&*env, Command::PowerShell));
    assert!(available_in(&*env, Command::Xtree));
}

const MAC_TABLE: &str = "ir_a_carpeta|Ctrl+P|Ir a una carpeta…|⌘P|true\npaleta|Ctrl+Shift+P|Paleta de comandos|⇧⌘P|true\npestana_nueva|Ctrl+T|Pestaña nueva|⌘T|true\ncerrar_pestana|Ctrl+W|Cerrar pestaña|⌘W|true\nreabrir_pestana|Ctrl+Shift+T|Reabrir la última pestaña cerrada|⇧⌘T|true\npestana_siguiente|Ctrl+AvPag|Pestaña siguiente|⌘⇟|true\npestana_anterior|Ctrl+RePag|Pestaña anterior|⌘⇞|true\ngrupo_siguiente|Ctrl+Tab|Grupo siguiente|⌘⇥|true\ngrupo_anterior|Ctrl+Shift+Tab|Grupo anterior|⇧⌘⇥|true\npanel_siguiente|Tab|Panel siguiente|⇥|true\npanel_anterior|Shift+Tab|Panel anterior|⇧⇥|true\npanel_1|Alt+1|Ir al panel 1|⌥1|true\npanel_2|Alt+2|Ir al panel 2|⌥2|true\npanel_3|Alt+3|Ir al panel 3|⌥3|true\npanel_4|Alt+4|Ir al panel 4|⌥4|true\nlayout_1|Ctrl+1|Layout: 1 panel|⌘1|true\nlayout_2_lado_a_lado|Ctrl+2|Layout: 2 paneles lado a lado|⌘2|true\nlayout_2_apilados|Ctrl+3|Layout: 2 paneles apilados|⌘3|true\nlayout_3|Ctrl+4|Layout: 3 paneles|⌘4|true\nlayout_4|Ctrl+5|Layout: 4 paneles|⌘5|true\nmaximizar_panel|Ctrl+M|Maximizar o restaurar el panel|⌘M|true\natras|Ctrl+[,Ctrl+Izquierda,NavegadorAtras|Atrás|⌘[,⌘←,Botón atrás del mouse|true\nadelante|Ctrl+],Ctrl+Derecha,NavegadorAdelante|Adelante|⌘],⌘→,Botón adelante del mouse|true\nsubir|Ctrl+Arriba,Retroceso|Subir a la carpeta superior|⌘↑,⌫|true\nrecargar|F5,Ctrl+R|Recargar|F5,⌘R|true\nescribir_ruta|Ctrl+L,Ctrl+Shift+G,F4|Escribir una ruta|⌘L,⇧⌘G,F4|true\nfiltrar|Ctrl+F|Buscar en la carpeta y sus subcarpetas|⌘F|true\neste_equipo||Ir a Este equipo||true\nmostrar_ocultos|Ctrl+Shift+.|Mostrar u ocultar archivos ocultos|⇧⌘.|true\nseleccionar_todo|Ctrl+A|Seleccionar todo|⌘A|true\ninvertir_seleccion|Ctrl+Alt+I|Invertir selección|⌥⌘I|true\ncopiar|Ctrl+C|Copiar|⌘C|true\ncortar|Ctrl+X|Cortar|⌘X|true\npegar|Ctrl+V|Pegar|⌘V|true\ncopiar_rutas|Ctrl+Alt+C|Copiar las rutas como texto|⌥⌘C|true\ndeshacer|Ctrl+Z|Deshacer|⌘Z|true\ncarpeta_nueva|Ctrl+Shift+N|Carpeta nueva|⇧⌘N|true\npapelera|Ctrl+Retroceso,Supr|Trasladar a la Papelera|⌘⌫,⌦|true\neliminar|Ctrl+Alt+Retroceso,Shift+Supr|Eliminar definitivamente|⌥⌘⌫,⇧⌦|true\nrenombrar|F2|Renombrar|F2|true\npropiedades|Ctrl+I|Obtener información|⌘I|true\nfavorito|Ctrl+D|Agregar o quitar de favoritos|⌘D|true\nbarra_favoritos|Ctrl+Shift+B|Mostrar u ocultar la barra de favoritos|⇧⌘B|true\npowershell||Abrir Terminal en la carpeta||true\ncmd||Abrir CMD en la carpeta||false\nmodo_oscuro||Cambiar entre modo claro y oscuro||true\nguardar_grupo||Guardar paneles y carpetas como grupo…||true\nbarra_grupos||Mostrar u ocultar la barra de grupos||true\nabrir_configuracion||Abrir la configuración (config.toml)||true\nxtree||Mapa de uso de disco (xtree)||true\nacerca_de||Acerca de xdir (versión y reportar un problema)||true\nvista_detalles|Ctrl+Shift+1|Vista: detalles|⇧⌘1|true\nvista_iconos|Ctrl+Shift+2|Vista: íconos medianos|⇧⌘2|true\nvista_iconos_grandes|Ctrl+Shift+3|Vista: íconos grandes|⇧⌘3|true\nvista_miniaturas|Ctrl+Shift+4|Vista: miniaturas|⇧⌘4|true\nordenar_nombre||Ordenar por nombre||true\nordenar_modificado||Ordenar por fecha de modificación||true\nordenar_creado||Ordenar por fecha de creación||true\nordenar_tipo||Ordenar por tipo||true\nordenar_extension||Ordenar por extensión||true\nordenar_tamano||Ordenar por tamaño||true\narbol|Ctrl+Shift+E|Mostrar u ocultar el árbol de carpetas (experimental)|⇧⌘E|true\n";
const WINDOWS_TABLE: &str = "ir_a_carpeta|Ctrl+P|Ir a una carpeta…|Ctrl+P|true\npaleta|Ctrl+Shift+P|Paleta de comandos|Ctrl+Shift+P|true\npestana_nueva|Ctrl+T|Pestaña nueva|Ctrl+T|true\ncerrar_pestana|Ctrl+W|Cerrar pestaña|Ctrl+W|true\nreabrir_pestana|Ctrl+Shift+T|Reabrir la última pestaña cerrada|Ctrl+Shift+T|true\npestana_siguiente|Ctrl+AvPag|Pestaña siguiente|Ctrl+AvPág|true\npestana_anterior|Ctrl+RePag|Pestaña anterior|Ctrl+RePág|true\ngrupo_siguiente|Ctrl+Tab|Grupo siguiente|Ctrl+Tab|true\ngrupo_anterior|Ctrl+Shift+Tab|Grupo anterior|Ctrl+Shift+Tab|true\npanel_siguiente|Tab|Panel siguiente|Tab|true\npanel_anterior|Shift+Tab|Panel anterior|Shift+Tab|true\npanel_1|Alt+1|Ir al panel 1|Alt+1|true\npanel_2|Alt+2|Ir al panel 2|Alt+2|true\npanel_3|Alt+3|Ir al panel 3|Alt+3|true\npanel_4|Alt+4|Ir al panel 4|Alt+4|true\nlayout_1|Ctrl+1|Layout: 1 panel|Ctrl+1|true\nlayout_2_lado_a_lado|Ctrl+2|Layout: 2 paneles lado a lado|Ctrl+2|true\nlayout_2_apilados|Ctrl+3|Layout: 2 paneles apilados|Ctrl+3|true\nlayout_3|Ctrl+4|Layout: 3 paneles|Ctrl+4|true\nlayout_4|Ctrl+5|Layout: 4 paneles|Ctrl+5|true\nmaximizar_panel|Ctrl+M|Maximizar o restaurar el panel|Ctrl+M|true\natras|Alt+Izquierda,NavegadorAtras|Atrás|Alt+←,Botón atrás del mouse|true\nadelante|Alt+Derecha,NavegadorAdelante|Adelante|Alt+→,Botón adelante del mouse|true\nsubir|Alt+Arriba,Retroceso|Subir a la carpeta superior|Alt+↑,Retroceso|true\nrecargar|F5|Recargar|F5|true\nescribir_ruta|Ctrl+L,F4,Alt+D|Escribir una ruta|Ctrl+L,F4,Alt+D|true\nfiltrar|Ctrl+F|Buscar en la carpeta y sus subcarpetas|Ctrl+F|true\neste_equipo||Ir a Este equipo||true\nmostrar_ocultos|Ctrl+H|Mostrar u ocultar archivos ocultos|Ctrl+H|true\nseleccionar_todo|Ctrl+A|Seleccionar todo|Ctrl+A|true\ninvertir_seleccion|Ctrl+I|Invertir selección|Ctrl+I|true\ncopiar|Ctrl+C|Copiar|Ctrl+C|true\ncortar|Ctrl+X|Cortar|Ctrl+X|true\npegar|Ctrl+V|Pegar|Ctrl+V|true\ncopiar_rutas|Ctrl+Shift+C|Copiar las rutas como texto|Ctrl+Shift+C|true\ndeshacer|Ctrl+Z|Deshacer|Ctrl+Z|true\ncarpeta_nueva|Ctrl+Shift+N|Carpeta nueva|Ctrl+Shift+N|true\npapelera|Supr|Enviar a la papelera|Supr|true\neliminar|Shift+Supr|Eliminar definitivamente|Shift+Supr|true\nrenombrar|F2|Renombrar|F2|true\npropiedades|Alt+Enter|Propiedades|Alt+Enter|true\nfavorito|Ctrl+D|Agregar o quitar de favoritos|Ctrl+D|true\nbarra_favoritos|Ctrl+Shift+B|Mostrar u ocultar la barra de favoritos|Ctrl+Shift+B|true\npowershell||Abrir PowerShell en la carpeta||true\ncmd||Abrir CMD en la carpeta||true\nmodo_oscuro||Cambiar entre modo claro y oscuro||true\nguardar_grupo||Guardar paneles y carpetas como grupo…||true\nbarra_grupos||Mostrar u ocultar la barra de grupos||true\nabrir_configuracion||Abrir la configuración (config.toml)||true\nxtree||Mapa de uso de disco (xtree)||true\nacerca_de||Acerca de xdir (versión y reportar un problema)||true\nvista_detalles|Ctrl+Shift+1|Vista: detalles|Ctrl+Shift+1|true\nvista_iconos|Ctrl+Shift+2|Vista: íconos medianos|Ctrl+Shift+2|true\nvista_iconos_grandes|Ctrl+Shift+3|Vista: íconos grandes|Ctrl+Shift+3|true\nvista_miniaturas|Ctrl+Shift+4|Vista: miniaturas|Ctrl+Shift+4|true\nordenar_nombre||Ordenar por nombre||true\nordenar_modificado||Ordenar por fecha de modificación||true\nordenar_creado||Ordenar por fecha de creación||true\nordenar_tipo||Ordenar por tipo||true\nordenar_extension||Ordenar por extensión||true\nordenar_tamano||Ordenar por tamaño||true\narbol|Ctrl+Shift+E|Mostrar u ocultar el árbol de carpetas (experimental)|Ctrl+Shift+E|true\n";


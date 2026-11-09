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

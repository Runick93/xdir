//! Nombre del archivo temporal con el que el ayudante elevado de xtree (Windows, tecla A) le
//! devuelve el árbol a xdir. Es lógica pura: se compila en Windows y en las pruebas de cualquier
//! sistema.

/// Archivo del pedido `n` del proceso `pid`: `xdir-xtree-<pid>-<n>.bin`.
pub fn helper_file_name(pid: u32, n: u32) -> String {
    format!("xdir-xtree-{pid}-{n}.bin")
}

/// `true` si el último componente de `path` (con `\` o `/`) es `xdir-xtree-<dígitos>.bin` o
/// `xdir-xtree-<dígitos>-<dígitos>.bin`.
pub fn is_helper_file(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or("");
    let Some(middle) = name.strip_prefix("xdir-xtree-").and_then(|r| r.strip_suffix(".bin")) else { return false };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match middle.split_once('-') {
        Some((pid, n)) => digits(pid) && digits(n),
        None => digits(middle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_has_pid_and_counter() {
        assert_eq!(helper_file_name(4321, 0), "xdir-xtree-4321-0.bin");
        assert_eq!(helper_file_name(4321, 7), "xdir-xtree-4321-7.bin");
        assert!(is_helper_file(&helper_file_name(u32::MAX, u32::MAX)));
    }

    #[test]
    fn accepts_only_helper_names() {
        assert!(is_helper_file(r"C:\Users\ana\AppData\Local\Temp\xdir-xtree-123.bin"));
        assert!(is_helper_file(r"C:\Temp\xdir-xtree-123-4.bin"));
        assert!(is_helper_file("/tmp/xdir-xtree-1-2.bin"));
        assert!(is_helper_file("xdir-xtree-9.bin"));
        for bad in [
            r"C:\Windows\System32\config\SAM",
            r"C:\Temp\xdir-xtree-.bin",
            r"C:\Temp\xdir-xtree-12-.bin",
            r"C:\Temp\xdir-xtree--3.bin",
            r"C:\Temp\xdir-xtree-1-2-3.bin",
            r"C:\Temp\xdir-xtree-12a.bin",
            r"C:\Temp\xdir-xtree-12.bin.exe",
            r"C:\Temp\xdir-xtree-12.BIN",
            r"C:\Temp\otro-xdir-xtree-12.bin",
            r"C:\Temp\xdir-xtree-12.bin\",
            r"C:\Temp\xdir-xtree-１２.bin",
            "",
        ] {
            assert!(!is_helper_file(bad), "{bad}");
        }
    }
}

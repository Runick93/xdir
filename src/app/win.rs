//! Utilidades de Win32 y subclases de controles de edición.

use super::*;


/// Subclase de los controles de edición (dirección y renombrado): Enter confirma, Escape
/// cancela. `done_msg` es el mensaje que se le manda a la ventana al terminar.
pub(super) unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    done_msg: usize,
) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd).unwrap_or_default();
        match msg {
            WM_KEYDOWN
                if done_msg as u32 == WM_APP_EDIT_DONE
                    && matches!(Key(wparam.0 as u16), VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT) =>
            {
                let _ = PostMessageW(Some(parent), WM_APP_SUGGEST_KEY, wparam, LPARAM(0));
                return LRESULT(0);
            }
            WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize || wparam.0 == VK_ESCAPE.0 as usize => {
                let commit = wparam.0 == VK_RETURN.0 as usize;
                let _ = PostMessageW(Some(parent), done_msg as u32, WPARAM(commit as usize), LPARAM(0));
                return LRESULT(0);
            }
            // Evita el "ding" de Enter/Escape en un edit de una línea.
            WM_CHAR if wparam.0 == '\r' as usize || wparam.0 == 0x1B => return LRESULT(0),
            // Al perder el foco: el renombrado se aplica (como en el Explorador), la dirección no.
            WM_KILLFOCUS => {
                let commit = done_msg as u32 != WM_APP_EDIT_DONE;
                let _ = PostMessageW(Some(parent), done_msg as u32, WPARAM(commit as usize), LPARAM(0));
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Subclase del control de edición de la paleta: las flechas, Re Pág / Av Pág, Enter, Escape y
/// Ctrl+P van a la ventana (que mueve la selección o abre el resultado); al perder el foco, avisa.
pub(super) unsafe extern "system" fn palette_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd).unwrap_or_default();
        let ctrl = key_down(VK_CONTROL);
        let post = |vk: usize| {
            let _ = PostMessageW(Some(parent), WM_APP_PALETTE_KEY, WPARAM(vk), LPARAM(ctrl as isize));
        };
        match msg {
            WM_KEYDOWN => {
                let vk = Key(wparam.0 as u16);
                if matches!(vk, VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_RETURN | VK_ESCAPE) || (ctrl && vk == Key(0x50)) {
                    post(wparam.0);
                    return LRESULT(0);
                }
            }
            // Evita el "ding" de Enter/Escape (y el carácter de Ctrl+P) en un edit de una línea.
            WM_CHAR if wparam.0 == '\r' as usize || wparam.0 == 0x1B || wparam.0 == 0x10 => return LRESULT(0),
            WM_KILLFOCUS => post(0),
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Efecto al soltar como en el Explorador: Ctrl copia, Shift mueve; sin teclas, mueve dentro
/// de la misma unidad y copia entre unidades. Soltar en la misma carpeta (o dentro de sí
/// misma) no hace nada.
pub(super) fn drop_effect(paths: &[PathBuf], dest: &std::path::Path, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
    if paths.iter().any(|p| p.parent() == Some(dest) || dest.starts_with(p)) {
        return DROPEFFECT_NONE;
    }
    let same_volume = paths.first().and_then(|p| shell::volume_of(p)) == shell::volume_of(dest);
    let want = if keys.0 & MK_CONTROL.0 != 0 {
        DROPEFFECT_COPY
    } else if keys.0 & MK_SHIFT.0 != 0 || same_volume {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };
    if allowed.0 & want.0 != 0 {
        want
    } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

/// Abre un enlace en el navegador predeterminado.
pub(super) fn open_url(hwnd: HWND, url: &str) {
    let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    unsafe {
        ShellExecuteW(Some(hwnd), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Abre un archivo de configuración con el programa asociado; si `.toml` no tiene ninguno, con el
/// Bloc de notas.
pub(super) fn open_in_editor(hwnd: HWND, path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let r = ShellExecuteW(Some(hwnd), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        // Menor o igual a 32 es un error (p. ej. SE_ERR_NOASSOC: la extensión no tiene programa).
        if r.0 as isize <= 32 {
            let arg: Vec<u16> = format!("\"{}\"", path.display()).encode_utf16().chain(Some(0)).collect();
            let _ = ShellExecuteW(Some(hwnd), w!("open"), w!("notepad.exe"), PCWSTR(arg.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

/// Texto de un control de edición.
pub(super) fn window_text(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 32 * 1024];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Ícono de la app (recurso 1) al tamaño de la métrica dada; nulo si no está (la clase usa el genérico).
pub(super) fn app_icon(instance: HINSTANCE, metric: SYSTEM_METRICS_INDEX) -> HICON {
    unsafe {
        let size = GetSystemMetrics(metric);
        LoadImageW(Some(instance), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_default()
    }
}

/// Líneas por "clic" de la rueda según la configuración de Windows.
pub(super) fn wheel_lines() -> f32 {
    let mut lines = 3u32;
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some(&mut lines as *mut _ as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    // u32::MAX significa "una pantalla por clic"; se aproxima con 10 filas.
    if lines == u32::MAX { 10.0 } else { lines as f32 }
}

pub(super) fn expand_env(s: &str) -> String {
    let src: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 32 * 1024];
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) } as usize;
    if n == 0 || n > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..n - 1])
}

pub(super) fn colorref(c: Color) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | ch(c.g) << 8 | ch(c.b) << 16)
}

pub(super) fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

pub(super) fn scale(dips: i32, dpi: u32) -> i32 {
    dips * dpi as i32 / 96
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: DROPEFFECT = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0);

    fn effect(sources: &[&str], dest: &str, keys: MODIFIERKEYS_FLAGS, allowed: DROPEFFECT) -> DROPEFFECT {
        let paths: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        drop_effect(&paths, std::path::Path::new(dest), keys, allowed)
    }

    #[test]
    fn drop_rules() {
        let none = MODIFIERKEYS_FLAGS(0);
        // Misma unidad: mueve; otra unidad: copia.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", none, ALL), DROPEFFECT_MOVE);
        assert_eq!(effect(&[r"D:\a\x.txt"], r"C:\b", none, ALL), DROPEFFECT_COPY);
        // Ctrl copia, Shift mueve.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", MK_CONTROL, ALL), DROPEFFECT_COPY);
        assert_eq!(effect(&[r"D:\a\x.txt"], r"C:\b", MK_SHIFT, ALL), DROPEFFECT_MOVE);
        // A la misma carpeta, o una carpeta dentro de sí misma: nada.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\a", none, ALL), DROPEFFECT_NONE);
        assert_eq!(effect(&[r"C:\a"], r"C:\a\sub", none, ALL), DROPEFFECT_NONE);
        // Si el origen solo permite copiar, se copia.
        assert_eq!(effect(&[r"C:\a\x.txt"], r"C:\b", none, DROPEFFECT_COPY), DROPEFFECT_COPY);
    }
}

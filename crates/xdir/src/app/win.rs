//! Windows: arranque y ventana Win32 (los controles de edición nativos y los menús emergentes
//! están en el host, `host_win.rs`). macOS tiene lo mismo en `mac.rs`.

use super::*;

pub(super) use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
pub(super) use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, ClientToScreen, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DeleteObject, EndPaint, FW_NORMAL, FillRect, GetStockObject, HBRUSH, HDC, HFONT, InvalidateRect,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, ScreenToClient, SetBkColor, SetTextColor,
};
pub(super) use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
pub(super) use windows::Win32::System::LibraryLoader::GetModuleHandleW;
pub(super) use windows::Win32::System::Ole::{
    DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, IDropTarget, OleInitialize, RevokeDragDrop,
};
pub(super) use windows::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
pub(super) use windows::Win32::UI::Controls::{EM_SETSEL, WM_MOUSELEAVE};
pub(super) use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
pub(super) use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
pub(super) use windows::Win32::UI::Shell::{DefSubclassProc, SHDoDragDrop, SetWindowSubclass, ShellExecuteW};
pub(super) use windows::Win32::UI::WindowsAndMessaging::*;
pub(super) use windows::core::{PCWSTR, w};

const CLASS_NAME: PCWSTR = w!("xdir.main");
/// Notificación de un control de edición: el texto cambió.
pub(super) const EN_CHANGE_CODE: usize = 0x0300;

// ---------- arranque ----------

pub fn run() -> windows::core::Result<()> {
    // `xdir --xtree-mft <unidad> <archivo> <pid>`: proceso elevado de xtree (tecla A). Lee la MFT, deja
    // el árbol en el archivo y sale, sin ventana, sesión ni configuración.
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if args.first().is_some_and(|a| a == crate::fs::XTREE_HELPER_ARG) {
        std::process::exit(crate::fs::xtree_helper_main(&args[1..]));
    }
    let started = Instant::now();
    // Servicios del sistema para el núcleo: antes que nada (salvo el ayudante de arriba, que no
    // los usa), porque la `App` ya los usa.
    crate::sys::install();
    let (session, probe, start_path) = startup_args();

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        // OLE (STA): lo exigen el Shell, el portapapeles y arrastrar y soltar.
        OleInitialize(None)?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // Ícono del exe (recurso 1, ver assets/xdir.rc): grande para Alt+Tab, chico para la barra de tareas.
            hIcon: app_icon(instance.into(), SM_CXICON),
            hIconSm: app_icon(instance.into(), SM_CXSMICON),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }

        let marks = vec![("init", ms_since(started))];
        let mut init = Init::load(session, probe, start_path, started, marks);
        shell::set_menu_mode(init.session.dark_mode);
        // El renderizador vive en el host: crear sus fábricas es lo que mide esta marca.
        let host = Win32Host::install()?;
        init.marks.push(("fabricas d2d/dwrite", ms_since(started)));
        let app = App::new(init, host.clone());
        let app = Box::into_raw(Box::new(app));

        // La ventana se crea oculta para aplicar tema y tamaño antes de mostrarla (sin flash blanco).
        // El HWND queda guardado en `App` desde WM_NCCREATE.
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS_NAME,
            w!("xdir"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            Some(instance.into()),
            Some(app as *const _),
        )?;

        // Sin ícono en la barra de título (la barra de tareas y Alt+Tab siguen usando el de la clase).
        let no_icon = windows::Win32::UI::Controls::WTA_OPTIONS {
            dwFlags: windows::Win32::UI::Controls::WTNCA_NODRAWICON,
            dwMask: windows::Win32::UI::Controls::WTNCA_NODRAWICON,
        };
        let _ = windows::Win32::UI::Controls::SetWindowThemeAttribute(
            (*app).hwnd.hwnd(),
            windows::Win32::UI::Controls::WTA_NONCLIENT,
            &no_icon as *const _ as *const std::ffi::c_void,
            size_of::<windows::Win32::UI::Controls::WTA_OPTIONS>() as u32,
        );

        // Las carpetas visibles empiezan a leerse antes de mostrar la ventana.
        (*app).sync_visibility();
        host.register_drop_target();
        #[cfg(feature = "arbol")]
        (*app).start_quick_access();
        (*app).marks.push(("ventana creada", ms_since(started)));
        let saved = (*app).window_state.take();
        host.show(saved);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            let app = cs.lpCreateParams as *mut App;
            (*app).hwnd = crate::platform::handle_from_hwnd(hwnd);
            (*app).dpi = GetDpiForWindow(hwnd);
            (*app).host.attach((*app).hwnd);
            (*app).apply_window_theme();
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if app.is_null() {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        if msg == WM_NCDESTROY {
            drop(Box::from_raw(app));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }

        (*app).handle(msg, wparam, lparam)
    }
}

/// Ícono de la app (recurso 1) al tamaño de la métrica dada; nulo si no está (la clase usa el genérico).
fn app_icon(instance: HINSTANCE, metric: SYSTEM_METRICS_INDEX) -> HICON {
    unsafe {
        let size = GetSystemMetrics(metric);
        LoadImageW(Some(instance), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_default()
    }
}

pub(super) fn colorref(c: Color) -> COLORREF {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(ch(c.r) | ch(c.g) << 8 | ch(c.b) << 16)
}

pub(super) fn scale(dips: i32, dpi: u32) -> i32 {
    dips * dpi as i32 / 96
}

//! El visor del panel de vista previa del Explorador (`IPreviewHandler`) para un archivo.
//!
//! - `viewer_for` busca el visor del tipo; los que piden correr dentro del proceso
//!   (`DisableProcessIsolation`) cuentan como «sin visor».
//! - Cada elemento tiene su hilo STA (`start`): crea el visor fuera de proceso
//!   (`CLSCTX_LOCAL_SERVER`, Windows lo corre en `prevhost.exe`), lo inicializa con el archivo, lo
//!   pone en una ventana propia del hilo (hija de la de la vista rápida) y sigue con su bucle de
//!   mensajes mientras se vea. Un hilo vigía espera su respuesta hasta `HANDLER_TIMEOUT`
//!   (`run_with`) y le avisa a la ventana con `WM_QL_SETTLED`.
//! - El hilo de interfaz nunca llama al visor: solo le manda mensajes asíncronos a la ventana del
//!   hilo del visor (tamaño, mostrar, cerrar). Si el visor se cuelga, se abandona su hilo
//!   (`Viewer::abandon`: se oculta su ventana, se cancela la llamada en curso con `CoCancelCall`)
//!   y la ventana muestra la imagen grande. Si `prevhost.exe` muere con el visor a la vista, el
//!   hilo lo nota porque desaparece la ventana del visor (`WM_QL_DIED`).
//! - Las teclas que el visor no usa vuelven por `IPreviewHandlerFrame::TranslateAccelerator`:
//!   Espacio, Esc y las flechas van a la ventana principal (`WM_APP_QL_KEY`). El marco le da al
//!   visor una tabla de aceleradores con esas teclas (`GetWindowContext`): muchos visores solo
//!   llaman al marco por las teclas de esa tabla.
//! - Si el visor se cuelga ya a la vista (no contesta dos veces seguidas), también se pasa a la
//!   imagen grande. Al cerrar xdir con un hilo abandonado todavía vivo, el proceso termina sin
//!   destruir ventanas (`must_exit_hard`).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, S_FALSE, WPARAM};
use windows::Win32::Graphics::Gdi::{DEFAULT_CHARSET, FW_NORMAL, LOGFONTW};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CLSIDFromString, COINIT_APARTMENTTHREADED, CoCancelCall, CoCreateInstance, CoEnableCallCancellation,
    CoInitializeEx, CoUninitialize, STGM_READ, STGM_SHARE_DENY_NONE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::IObjectWithSite;
use windows::Win32::System::Registry::{HKEY_CLASSES_ROOT, REG_ROUTINE_FLAGS, RRF_RT_REG_DWORD, RRF_SUBKEY_WOW6432KEY, RegGetValueW};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, AssocQueryStringW, IInitializeWithItem, IPreviewHandler,
    IPreviewHandlerFrame, IPreviewHandlerFrame_Impl, IPreviewHandlerVisuals, IShellItem, PREVIEWHANDLERFRAMEINFO, SHCreateItemFromParsingName,
    SHCreateStreamOnFileEx,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, GUID, HSTRING, Interface, PCWSTR, PWSTR, Result, implement, w};

use super::pure::{
    ACCEL_KEYS, HANDLER_TIMEOUT, Liveness, Outcome, PREVIEW_HANDLER_IID, Reply, Status, forward_key, hard_exit, isolation_disabled,
    run_with,
};
use super::{WM_APP_QL_KEY, WM_QL_DIED, WM_QL_SETTLED, encode_outcome};

/// CLSID del visor del panel de vista previa para la extensión `ext` («.pdf» o «pdf»). `None` si
/// el tipo no tiene, o si el visor pide correr dentro del proceso (`DisableProcessIsolation`).
/// Solo lee el registro.
pub fn viewer_for(ext: &str) -> Option<GUID> {
    if ext.is_empty() {
        return None;
    }
    let ext = HSTRING::from(if ext.starts_with('.') { ext.to_string() } else { format!(".{ext}") });
    let mut buf = [0u16; 64];
    let mut len = buf.len() as u32;
    unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_DEFAULTTOSTAR,
            ASSOCSTR_SHELLEXTENSION,
            &ext,
            &HSTRING::from(PREVIEW_HANDLER_IID),
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
        .ok()
        .ok()?;
        let clsid = CLSIDFromString(PCWSTR(buf.as_ptr())).ok()?;
        // El visor puede estar registrado en la vista de 64 bits o en la de 32 (WOW6432Node).
        let in_process = [RRF_RT_REG_DWORD, RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6432KEY]
            .into_iter()
            .any(|view| isolation_disabled(disable_process_isolation(&clsid, view)));
        (!in_process).then_some(clsid)
    }
}

/// `HKCR\CLSID\{clsid}\DisableProcessIsolation` (DWORD) en la vista del registro de `flags`, si está.
fn disable_process_isolation(clsid: &GUID, flags: REG_ROUTINE_FLAGS) -> Option<u32> {
    let key = HSTRING::from(format!("CLSID\\{{{clsid:?}}}"));
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            &key,
            w!("DisableProcessIsolation"),
            flags,
            None,
            Some(&mut value as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    r.is_ok().then_some(value)
}

// ---------- el marco (`IPreviewHandlerFrame`) ----------

/// El marco que el visor ve como su anfitrión: le devuelve a xdir las teclas suyas. Vive en el
/// hilo del visor (STA); `prevhost.exe` lo llama por COM.
#[implement(IPreviewHandlerFrame)]
struct Frame {
    /// La ventana principal de xdir (recibe `WM_APP_QL_KEY`).
    main: isize,
    /// Tabla de aceleradores con `ACCEL_KEYS` (nula si no se pudo crear).
    accel: HACCEL,
}

impl Frame {
    fn new(main: isize) -> Frame {
        let table: Vec<ACCEL> = ACCEL_KEYS.iter().map(|&key| ACCEL { fVirt: FVIRTKEY, key, cmd: key }).collect();
        let accel = unsafe { CreateAcceleratorTableW(&table) }.unwrap_or_default();
        Frame { main, accel }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if !self.accel.is_invalid() {
            unsafe {
                let _ = DestroyAcceleratorTable(self.accel);
            }
        }
    }
}

/// Ctrl, Alt o Mayúsculas apretadas ahora (el visor está en otro proceso: se mira el teclado).
fn modifiers() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT].iter().any(|k| unsafe { GetAsyncKeyState(k.0 as i32) } < 0)
}

#[allow(non_snake_case)]
impl IPreviewHandlerFrame_Impl for Frame_Impl {
    /// La tabla con Espacio, Esc y las flechas: los visores que filtran con `IsAccelerator`
    /// llaman a `TranslateAccelerator` solo por esas teclas.
    fn GetWindowContext(&self) -> Result<PREVIEWHANDLERFRAMEINFO> {
        let entries = if self.accel.is_invalid() { 0 } else { ACCEL_KEYS.len() as u32 };
        Ok(PREVIEWHANDLERFRAMEINFO { haccel: self.accel, cAccelEntries: entries })
    }

    /// Espacio, Esc y las flechas (sin modificadores) son de xdir: `S_OK` y van a la ventana
    /// principal. Las demás, `S_FALSE` (el visor las usa).
    fn TranslateAccelerator(&self, pmsg: *const MSG) -> Result<()> {
        let Some(msg) = (unsafe { pmsg.as_ref() }) else { return Err(Error::from_hresult(S_FALSE)) };
        let vk = msg.wParam.0 as u16;
        if !forward_key(msg.message, vk, modifiers()) {
            return Err(Error::from_hresult(S_FALSE));
        }
        unsafe {
            let _ = PostMessageW(Some(HWND(self.main as *mut _)), WM_APP_QL_KEY, WPARAM(vk as usize), LPARAM(0));
        }
        Ok(())
    }
}

// ---------- el hilo del visor ----------

/// Lo que necesita el hilo del visor.
pub struct Job {
    pub clsid: GUID,
    pub path: PathBuf,
    /// La ventana de la vista rápida (padre de la del visor; recibe los avisos).
    pub parent: isize,
    /// La ventana principal (recibe las teclas).
    pub main: isize,
    /// Dónde va el visor dentro de `parent` (píxeles).
    pub rect: RECT,
    pub generation: u64,
    /// Colores del tema (`COLORREF`) y DPI, para los visores que los aceptan
    /// (`IPreviewHandlerVisuals`).
    pub back: u32,
    pub text: u32,
    pub dpi: u32,
}

/// Lo que el hilo del visor comparte con el de interfaz.
struct Shared {
    /// El hilo no terminó (empieza en `true`: un hilo que todavía no arrancó cuenta como vivo).
    alive: AtomicBool,
    /// Id del hilo (0 hasta que arranca): para `CoCancelCall`.
    thread: AtomicU32,
    /// La ventana del hilo donde va el visor (0 hasta que se crea).
    body: AtomicIsize,
    /// El hilo de interfaz ya no lo quiere: termina en cuanto pueda.
    cancelled: AtomicBool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            alive: AtomicBool::new(true),
            thread: AtomicU32::new(0),
            body: AtomicIsize::new(0),
            cancelled: AtomicBool::new(false),
        }
    }
}

thread_local! {
    /// Los visores abandonados (hilo de interfaz), hasta que su hilo termine.
    static ABANDONED: RefCell<Vec<Arc<Shared>>> = const { RefCell::new(Vec::new()) };
}

/// Al cerrar xdir: algún hilo de visor abandonado sigue vivo (colgado en `prevhost.exe`), así que
/// hay que terminar el proceso sin destruir ventanas (ver `pure::hard_exit`).
pub fn must_exit_hard() -> bool {
    ABANDONED.with(|a| {
        let mut a = a.borrow_mut();
        a.retain(|s| s.alive.load(Ordering::Acquire));
        let alive: Vec<bool> = a.iter().map(|s| s.alive.load(Ordering::Acquire)).collect();
        hard_exit(&alive)
    })
}

/// Un visor encargado, visto desde el hilo de interfaz. Ningún método espera al hilo del visor.
pub struct Viewer {
    shared: Arc<Shared>,
}

/// Mensaje propio de la ventana del visor: terminar (descargar el visor y salir del hilo).
const WM_BODY_QUIT: u32 = WM_USER + 1;
/// Cada cuánto el hilo mira si el visor sigue vivo.
const ALIVE_MS: u32 = 500;
const ALIVE_TIMER: usize = 1;
/// Lo que se espera a que el visor conteste en cada vistazo.
const HUNG_MS: u32 = 500;

/// Arranca el visor de `job` en un hilo STA propio y vuelve enseguida. El resultado llega a
/// `job.parent` como `WM_QL_SETTLED` (`wparam` = generación, `lparam` = `encode_outcome`).
pub fn start(job: Job) -> Viewer {
    let shared = Arc::new(Shared::default());
    let (parent, generation) = (job.parent, job.generation);
    let for_thread = shared.clone();
    let post = move |o: Outcome<Status>| unsafe {
        let _ = PostMessageW(Some(HWND(parent as *mut _)), WM_QL_SETTLED, WPARAM(generation as usize), LPARAM(encode_outcome(o)));
    };
    let watched = shared.clone();
    let watcher = std::thread::Builder::new().name("xdir-visor-vigia".into()).spawn(move || {
        let outcome = run_with(HANDLER_TIMEOUT, move |reply| viewer_thread(job, for_thread, reply));
        // El hilo del visor no llegó a arrancar: no queda nada vivo.
        if outcome == Outcome::Gone && watched.thread.load(Ordering::Acquire) == 0 {
            watched.alive.store(false, Ordering::Release);
        }
        post(outcome);
    });
    if watcher.is_err() {
        shared.alive.store(false, Ordering::Release);
        post(Outcome::Gone);
    }
    Viewer { shared }
}

impl Viewer {
    fn body(&self) -> Option<HWND> {
        match self.shared.body.load(Ordering::Acquire) {
            0 => None,
            h => Some(HWND(h as *mut _)),
        }
    }

    /// Muestra la ventana del visor (sin activarla).
    pub fn show(&self) {
        if let Some(body) = self.body() {
            unsafe {
                let _ = ShowWindowAsync(body, SW_SHOWNA);
            }
        }
    }

    /// Lleva la ventana del visor a `r` (píxeles de la ventana de la vista rápida), sin esperar.
    pub fn place(&self, r: RECT) {
        if let Some(body) = self.body() {
            unsafe {
                let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
                let _ = SetWindowPos(body, None, r.left, r.top, r.right - r.left, r.bottom - r.top, flags);
            }
        }
    }

    /// Ya no se lo quiere (otro elemento, o se cerró la vista rápida): el hilo descarga el visor
    /// (`Unload`) y termina.
    pub fn stop(&self) {
        self.shared.cancelled.store(true, Ordering::Release);
        unsafe {
            match self.body() {
                Some(body) => {
                    let _ = ShowWindowAsync(body, SW_HIDE);
                    let _ = PostMessageW(Some(body), WM_BODY_QUIT, WPARAM(0), LPARAM(0));
                }
                None => {
                    let thread = self.shared.thread.load(Ordering::Acquire);
                    if thread != 0 {
                        let _ = PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0));
                    }
                }
            }
        }
    }

    /// El visor no contestó a tiempo o murió: además de `stop`, se cancela la llamada que el hilo
    /// tenga en curso a `prevhost.exe`. Si igual sigue colgado, el hilo queda solo (oculto).
    pub fn abandon(&self) {
        ABANDONED.with(|a| a.borrow_mut().push(self.shared.clone()));
        self.stop();
        let thread = self.shared.thread.load(Ordering::Acquire);
        if thread != 0 {
            unsafe {
                let _ = CoCancelCall(thread, 0);
            }
        }
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.stop();
    }
}

thread_local! {
    /// El visor del hilo (para la ventana del visor: `SetRect` al cambiar de tamaño).
    static HANDLER: RefCell<Option<IPreviewHandler>> = const { RefCell::new(None) };
    /// Ya se le dio su ventana al visor (`SetWindow`): recién ahí se le pasa el tamaño.
    static WINDOW_SET: Cell<bool> = const { Cell::new(false) };
    /// Si el visor sigue vivo (`ALIVE_TIMER`).
    static LIVENESS: Cell<Liveness> = const { Cell::new(Liveness::new()) };
    /// A quién avisar si el visor muere: (ventana de la vista rápida, generación).
    static NOTIFY: Cell<(isize, u64)> = const { Cell::new((0, 0)) };
}

/// El hilo STA del visor: lo abre, contesta y sigue con su bucle de mensajes hasta que se lo
/// cierre. Al terminar descarga el visor y destruye su ventana.
fn viewer_thread(job: Job, shared: Arc<Shared>, reply: Reply<Status>) {
    /// Marca el hilo como terminado al salir (también si entra en pánico).
    struct Alive(Arc<Shared>);
    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.alive.store(false, Ordering::Release);
        }
    }
    let _alive = Alive(shared.clone());
    unsafe {
        shared.thread.store(GetCurrentThreadId(), Ordering::Release);
        if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
            reply.send(Status::Failed);
            return;
        }
        // Así el hilo de interfaz puede cortar una llamada colgada a `prevhost.exe`.
        let _ = CoEnableCallCancellation(None);
        NOTIFY.with(|n| n.set((job.parent, job.generation)));
        let opened = open(&job, &shared);
        let status = match &opened {
            Ok(_) if shared.cancelled.load(Ordering::Acquire) => Status::Failed,
            Ok(_) => Status::Ready,
            Err(s) => *s,
        };
        reply.send(status);
        if status == Status::Ready {
            if let Some(body) = shared_body(&shared) {
                let _ = SetTimer(Some(body), ALIVE_TIMER, ALIVE_MS, None);
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // Fin: descargar el visor y destruir la ventana del hilo. También si se lo abandonó: si
        // el visor estaba sano (se pasó de elemento mientras arrancaba) se libera todo; si está
        // colgado, este hilo se queda acá, vivo, y al cerrar xdir se sale sin destruir ventanas
        // (`must_exit_hard`).
        let handler = HANDLER.with(|h| h.borrow_mut().take()).or(opened.ok());
        if let Some(handler) = handler {
            let _ = handler.Unload();
            if let Ok(site) = handler.cast::<IObjectWithSite>() {
                let _ = site.SetSite(None);
            }
        }
        if let Some(body) = shared_body(&shared) {
            shared.body.store(0, Ordering::Release);
            let _ = DestroyWindow(body);
        }
        CoUninitialize();
    }
}

fn shared_body(shared: &Shared) -> Option<HWND> {
    match shared.body.load(Ordering::Acquire) {
        0 => None,
        h => Some(HWND(h as *mut _)),
    }
}

/// Crea el visor y lo muestra en una ventana nueva del hilo (oculta hasta que la ventana de la
/// vista rápida la muestre). Entre paso y paso mira si se lo canceló.
unsafe fn open(job: &Job, shared: &Shared) -> std::result::Result<IPreviewHandler, Status> {
    let cancelled = || shared.cancelled.load(Ordering::Acquire);
    let path = HSTRING::from(job.path.as_path());
    // Nunca se le pasa un archivo solo en la nube (el visor lo bajaría).
    if crate::metadata::cloud_only(&path) {
        return Err(Status::Cloud);
    }
    if cancelled() {
        return Err(Status::Failed);
    }
    unsafe {
        let body = create_body(job).ok_or(Status::Failed)?;
        shared.body.store(body.0 as isize, Ordering::Release);
        let handler: IPreviewHandler = CoCreateInstance(&job.clsid, None, CLSCTX_LOCAL_SERVER).map_err(|_| Status::Failed)?;
        // Desde acá, cualquier salida pasa por `Unload` y `SetSite(None)` (fin de `viewer_thread`).
        HANDLER.with(|h| *h.borrow_mut() = Some(handler.clone()));
        if cancelled() {
            return Err(Status::Failed);
        }
        initialize(&handler, &path).map_err(|_| Status::Failed)?;
        apply_visuals(&handler, job);
        if let Ok(site) = handler.cast::<IObjectWithSite>() {
            let frame: IPreviewHandlerFrame = Frame::new(job.main).into();
            let _ = site.SetSite(&frame);
        }
        if cancelled() {
            return Err(Status::Failed);
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(body, &mut rc);
        handler.SetWindow(body, &rc).map_err(|_| Status::Failed)?;
        WINDOW_SET.with(|w| w.set(true));
        handler.DoPreview().map_err(|_| Status::Failed)?;
        Ok(handler)
    }
}

/// Fondo, texto y fuente del tema para los visores que los aceptan (el de texto, el del SDK).
unsafe fn apply_visuals(handler: &IPreviewHandler, job: &Job) {
    let Ok(visuals) = handler.cast::<IPreviewHandlerVisuals>() else { return };
    let mut font = LOGFONTW {
        lfHeight: -(13 * job.dpi.max(96) as i32 / 96),
        lfWeight: FW_NORMAL.0 as i32,
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    for (dst, src) in font.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
        *dst = src;
    }
    unsafe {
        let _ = visuals.SetBackgroundColor(COLORREF(job.back));
        let _ = visuals.SetTextColor(COLORREF(job.text));
        let _ = visuals.SetFont(&font);
    }
}

/// El visor lee el archivo como prefiera: un flujo de solo lectura (lo recomendado), un
/// `IShellItem` o la ruta.
unsafe fn initialize(handler: &IPreviewHandler, path: &HSTRING) -> Result<()> {
    unsafe {
        if let Ok(init) = handler.cast::<IInitializeWithStream>() {
            let stream = SHCreateStreamOnFileEx(path, STGM_READ.0 | STGM_SHARE_DENY_NONE.0, 0, false, None)?;
            return init.Initialize(&stream, STGM_READ.0);
        }
        if let Ok(init) = handler.cast::<IInitializeWithItem>() {
            let item: IShellItem = SHCreateItemFromParsingName(path, None)?;
            return init.Initialize(&item, STGM_READ.0);
        }
        handler.cast::<IInitializeWithFile>()?.Initialize(path, STGM_READ.0)
    }
}

const BODY_CLASS: PCWSTR = w!("xdir.quicklook.viewer");

/// La ventana del hilo donde el visor pone la suya: hija de la de la vista rápida, oculta.
unsafe fn create_body(job: &Job) -> Option<HWND> {
    static REGISTER: Once = Once::new();
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        REGISTER.call_once(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(body_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: BODY_CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class);
        });
        let r = job.rect;
        CreateWindowExW(
            WS_EX_NOPARENTNOTIFY,
            BODY_CLASS,
            PCWSTR::null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            Some(HWND(job.parent as *mut _)),
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

/// La ventana del visor (en su hilo): el tamaño pasa al visor, el foco también, y cada tanto se
/// mira si la ventana del visor sigue ahí (si `prevhost.exe` murió, desaparece).
unsafe extern "system" fn body_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handler = || HANDLER.with(|h| h.borrow().clone());
    unsafe {
        match msg {
            WM_SIZE => {
                if let Some(h) = handler().filter(|_| WINDOW_SET.with(Cell::get)) {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let _ = h.SetRect(&rc);
                }
                LRESULT(0)
            }
            WM_SETFOCUS => {
                if let Some(h) = handler() {
                    let _ = h.SetFocus();
                }
                LRESULT(0)
            }
            // ¿Sigue vivo el visor? Su ventana desapareció (`prevhost.exe` cerrado) o no contesta
            // dos veces seguidas (colgado): se avisa y la ventana pasa a la imagen grande.
            WM_TIMER if wparam.0 == ALIVE_TIMER => {
                let child = GetWindow(hwnd, GW_CHILD).ok().filter(|c| !c.is_invalid());
                let responsive = child.is_some_and(|c| {
                    !IsHungAppWindow(c).as_bool()
                        && SendMessageTimeoutW(c, WM_NULL, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG | SMTO_BLOCK, HUNG_MS, None).0 != 0
                });
                let mut live = LIVENESS.with(Cell::get);
                let dead = live.tick(child.is_some(), responsive);
                LIVENESS.with(|l| l.set(live));
                if dead {
                    let _ = KillTimer(Some(hwnd), ALIVE_TIMER);
                    let (parent, generation) = NOTIFY.with(Cell::get);
                    let _ = PostMessageW(Some(HWND(parent as *mut _)), WM_QL_DIED, WPARAM(generation as usize), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_BODY_QUIT => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// En una máquina con el visor de texto (Windows 10/11 lo trae), `.txt` tiene uno; si no, se
    /// avisa y se saltea.
    #[test]
    fn viewer_for_txt_exists_or_skip() {
        match viewer_for(".txt") {
            Some(clsid) => {
                assert_ne!(clsid, GUID::zeroed());
                assert_eq!(viewer_for("txt"), Some(clsid), "con o sin punto");
            }
            None => eprintln!("esta máquina no tiene visor para .txt: se saltea"),
        }
    }

    #[test]
    fn viewer_for_unknown_ext_none() {
        assert_eq!(viewer_for(".xdirnada"), None);
        assert_eq!(viewer_for(""), None);
    }
}

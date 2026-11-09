//! Implementaciones falsas de los servicios del sistema, para los tests (de este crate y de
//! `xdir`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::Sender;

use super::{ConsoleInfo, FileOps, Handle, Services, SystemEnv, Waker};
use crate::format::thousands;
use crate::keys::Key;
use crate::ops::{Op, OpResult, WM_APP_OP_DONE};

/// Instala los servicios falsos. Se puede llamar desde todos los tests: solo la primera vez
/// instala, y si ya había servicios instalados no hace nada.
pub fn install_fakes() {
    super::install_once(|| Services {
        waker: Box::new(FakeWaker),
        env: Box::new(FakeEnv),
        file_ops: Box::new(FakeFileOps),
    });
}

/// Mensajes recibidos por el `FakeWaker`, por número de mensaje (de todo el proceso).
static POSTED: Mutex<BTreeMap<u32, usize>> = Mutex::new(BTreeMap::new());

/// `Waker` falso: no despierta a nadie, solo cuenta cuántos mensajes de cada número le llegaron.
pub struct FakeWaker;

impl FakeWaker {
    /// Cuántos mensajes `msg` se mandaron hasta ahora (en todo el proceso de tests: conviene
    /// usar un número de mensaje propio de cada test, o comparar contra el valor de antes).
    pub fn count(msg: u32) -> usize {
        POSTED.lock().unwrap_or_else(|e| e.into_inner()).get(&msg).copied().unwrap_or(0)
    }
}

impl Waker for FakeWaker {
    fn post(&self, _window: Handle, msg: u32, _wparam: usize, _lparam: isize) {
        *POSTED.lock().unwrap_or_else(|e| e.into_inner()).entry(msg).or_insert(0) += 1;
    }
}

thread_local! {
    /// Historial de rutas escritas del falso: en memoria y por hilo, para que los tests no se
    /// pisen.
    static TYPED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `SystemEnv` falso: tema claro, sin animaciones ni teclas apretadas, una carpeta de
/// configuración temporal por hilo, formatos deterministas y una sola consola.
pub struct FakeEnv;

impl SystemEnv for FakeEnv {
    fn system_dark(&self) -> bool {
        false
    }

    fn system_accent(&self) -> Option<u32> {
        None
    }

    fn animations_allowed(&self) -> bool {
        false
    }

    fn key_down(&self, _k: Key) -> bool {
        false
    }

    /// En pruebas la configuración nunca es la real: cada hilo de prueba tiene su propia carpeta
    /// temporal (`temp/xdir-tests/<pid>-<id del hilo>`).
    fn config_dir(&self) -> Option<PathBuf> {
        let name = format!("{}-{:?}", std::process::id(), std::thread::current().id());
        Some(std::env::temp_dir().join("xdir-tests").join(name.replace(['(', ')'], "")))
    }

    /// "AAAA-MM-DD" (UTC) del FILETIME; vacío si es 0.
    fn format_date(&self, filetime: u64) -> String {
        if filetime == 0 {
            return String::new();
        }
        let (y, m, d) = civil_from_days((filetime / 864_000_000_000) as i64 - DAYS_1601_TO_1970);
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// Como el Explorador de Windows: "1.234 KB" (redondea hacia arriba).
    fn format_size_kb(&self, bytes: u64) -> String {
        format!("{} KB", thousands(bytes.div_ceil(1024)))
    }

    /// Como Windows: "123 bytes", "1,23 KB", "12,3 MB", "476 GB" (tres cifras, truncadas).
    fn format_size_short(&self, bytes: u64) -> String {
        if bytes < 1024 {
            return format!("{bytes} bytes");
        }
        let units = ["KB", "MB", "GB", "TB", "PB", "EB"];
        let mut unit = 0;
        let mut scale = 1024u128;
        while (bytes as u128) >= scale * 1024 && unit + 1 < units.len() {
            scale *= 1024;
            unit += 1;
        }
        // Centésimas de la unidad, truncadas.
        let hundredths = bytes as u128 * 100 / scale;
        let text = match hundredths {
            0..1000 => format!("{},{:02}", hundredths / 100, hundredths % 100),
            1000..10000 => format!("{},{}", hundredths / 100, hundredths % 100 / 10),
            _ => format!("{}", hundredths / 100),
        };
        format!("{text} {}", units[unit])
    }

    fn typed_paths_load(&self) -> Vec<String> {
        TYPED.with(|t| t.borrow().clone())
    }

    fn typed_paths_save(&self, list: &[String]) {
        TYPED.with(|t| *t.borrow_mut() = list.to_vec());
    }

    fn consoles(&self) -> Vec<ConsoleInfo> {
        vec![ConsoleInfo { id: 0, label: "Terminal" }]
    }

    fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    fn open_console(&self, _owner: Handle, _id: u8, _dir: &Path, _verb: &str) -> bool {
        false
    }
}

/// Días entre 1601-01-01 (época de FILETIME) y 1970-01-01.
const DAYS_1601_TO_1970: i64 = 134_774;

/// Fecha civil (año, mes, día) de un número de días desde 1970-01-01 (algoritmo de Howard
/// Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

thread_local! {
    /// Operaciones recibidas por el falso y respuesta configurada: por hilo, para que los tests
    /// (que corren en paralelo, cada uno en su hilo) no se vean entre sí.
    static OPS: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
    static RESPONSE: RefCell<OpResult> = RefCell::new(OpResult::default());
}

/// `FileOps` falso: no toca el disco; anota cada `Op` recibida y responde por el canal, en el
/// acto, con el `OpResult` configurado (por defecto, éxito sin deshacer). El registro es del hilo
/// que llama a `spawn`, o sea el del test.
pub struct FakeFileOps;

impl FakeFileOps {
    /// Las operaciones pedidas desde este hilo hasta ahora (y vacía el registro).
    pub fn take_ops() -> Vec<Op> {
        OPS.with(|o| std::mem::take(&mut *o.borrow_mut()))
    }

    /// Respuesta de las próximas operaciones de este hilo.
    pub fn respond_with(result: OpResult) {
        RESPONSE.with(|r| *r.borrow_mut() = result);
    }
}

impl FileOps for FakeFileOps {
    fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op) {
        OPS.with(|o| o.borrow_mut().push(op));
        let _ = tx.send(RESPONSE.with(|r| r.borrow().clone()));
        owner.post(WM_APP_OP_DONE, 0, 0);
    }
}

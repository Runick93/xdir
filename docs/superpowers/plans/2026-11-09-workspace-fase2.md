# Workspace, fase 2: servicios del sistema, despertador y operaciones de archivos — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Las tres primeras interfaces del núcleo (`Waker`, `SystemEnv`, `FileOps`) definidas en `xdir-core`, implementadas por Windows y macOS dentro de `crates/xdir`, y los módulos que solo dependían de ellas movidos al núcleo, sin cambiar comportamiento.

**Architecture:** El núcleo define los traits y un **registro de servicios del proceso** (`xdir_core::sys`): cada plataforma instala sus implementaciones una vez al arrancar (`install`), y el código del núcleo las pide con funciones (`sys::env()`, `sys::waker()`, `sys::file_ops()`). Los tests instalan implementaciones falsas. Así no hay que pasar parámetros nuevos por cientos de llamadas. Lo que es por ventana (`Painter`, `WindowHost`) **no** va al registro; eso es de la fase 3. Las fachadas de `crates/xdir` (`platform`, `ops`, `format`, `console`, `typed`, `session`, `commands`) siguen exportando los mismos nombres, ahora llamando al registro, así que los archivos que las usan no cambian.

**Tech Stack:** Rust 2024, workspace de la fase 1 (`crates/xdir`, `crates/xdir-core`), CI de la fase 1.

**Spec:** `docs/superpowers/specs/2026-11-08-workspace-core-design.md` (paso 4: interfaces en orden `SystemEnv` → `Waker` → `FileOps` → …)

## Global Constraints

- Ningún cambio de comportamiento. `cargo test` en macOS: 130 tests o más (la suma de `xdir` + `xdir-core`) en cada tarea; `cargo check --all-targets --target x86_64-pc-windows-msvc` sin errores ni warnings; la CI (Windows con prueba de humo, macOS, Linux) en verde en cada PR.
- `xdir-core` sigue sin dependencias y sin `#[cfg(windows)]` / `#[cfg(target_os)]` en producción.
- Los traits del núcleo no mencionan tipos de sistema (`HWND`, `Retained<…>`, `windows::core::Result`…). Errores como `String`.
- Implementaciones por sistema en `crates/xdir/src/sys/{win32.rs, macos.rs}` (módulo nuevo), registradas desde `app/win.rs::run` y `app/mac.rs::run` **antes** de crear la `App`. El modo ayudante `--xtree-mft` de Windows no necesita el registro (sale antes).
- Tests: `xdir_core::sys::testing::install_fakes()` (idempotente, con `OnceLock`) instalado por los tests que lo necesiten; en `crates/xdir`, el `app()` de `app/tests.rs` y los tests de módulos que llamen servicios lo invocan.
- Rama por tarea, encadenadas desde `refactor/sistema` (sale de `origin/refactor/workspace`, que ya tiene la fase 1 completa). **Lección de la fase 1**: después de mergear un PR, cambiar la base del siguiente a `develop` antes de mergearlo.
- Castellano; Conventional Commits sin atribución a Claude.

## Review Focus

- **Registro sin instalar**: si algún camino llama `sys::env()` antes de `install` (tests de un módulo, el ayudante `--xtree-mft`, el `--probe`) → panic. `sys::*()` debe fallar con un mensaje claro («servicios no instalados») y cada task agrega un test que construye la `App` de prueba solo con `install_fakes()`.
- **`Handle::post` desde hilos de trabajo** (carga de carpetas, búsqueda, xtree, iconos, miniaturas, ops) pasa a llamar al `Waker` registrado: tiene que ser `Send + Sync` y no bloquear; en macOS sigue usando la cola principal de GCD. Prueba de humo de la CI + test con un `Waker` falso que cuenta mensajes desde varios hilos.
- **Tests que dependían del `config_dir` de `#[cfg(test)]`** (cada hilo con su carpeta temporal): el falso de `SystemEnv::config_dir` debe conservar exactamente ese comportamiento, o los tests de sesión, grupos y favoritos empiezan a pisarse entre sí.
- **Atajos y nombres de comandos en macOS** (`MAC_KEYS`, `MAC_NAMES`, `combo_label`): mover la tabla a la plataforma no debe cambiar qué atajo hace qué ni cómo se muestra (⌘⌥⇧, ⌫, ↩) → tests existentes de `commands.rs` más uno que compare la tabla completa antes y después.
- **Consolas** (`Console::ALL` difiere entre sistemas: 2 en Windows, 1 en macOS): el menú de terminales tiene que mostrar las mismas entradas y verbos alternativos que hoy en cada sistema.

---

### Task 1: Registro de servicios, `Waker` y `Handle` en el núcleo

**Files:**
- Create: `crates/xdir-core/src/sys/mod.rs` (traits, registro, `install`, accesores), `crates/xdir-core/src/sys/testing.rs` (falsos, `pub` para que `xdir` los use)
- Move: `Handle` de `crates/xdir/src/platform/mod.rs` → `crates/xdir-core/src/sys/mod.rs`; `platform/mod.rs` lo reexporta
- Create: `crates/xdir/src/sys/mod.rs`, `crates/xdir/src/sys/win32.rs`, `crates/xdir/src/sys/macos.rs` (implementaciones: `post_message` de `platform/{win32,macos}.rs` se muda acá)
- Modify: `crates/xdir/src/app/win.rs` y `crates/xdir/src/app/mac.rs` (`run` instala los servicios al empezar), `crates/xdir/src/app/tests.rs` (`app()` llama `install_fakes()`), `crates/xdir/src/main.rs` (`mod sys;`)

**Interfaces:**
- Produces (en `xdir_core::sys`):
  - `pub trait Waker: Send + Sync { fn post(&self, window: Handle, msg: u32, wparam: usize, lparam: isize); }`
  - `pub struct Services { pub waker: Box<dyn Waker>, pub env: Box<dyn SystemEnv>, pub file_ops: Box<dyn FileOps> }` (en esta tarea `SystemEnv` y `FileOps` existen como traits vacíos; se completan en Tasks 2 y 4)
  - `pub fn install(s: Services)` (una sola vez; la segunda es un no-op que se ignora en tests y un `debug_assert!` en producción) y `pub fn waker() -> &'static dyn Waker`, `pub fn env() -> &'static dyn SystemEnv`, `pub fn file_ops() -> &'static dyn FileOps`, que hacen `panic!("servicios del sistema no instalados")` si falta `install`.
  - `Handle(pub isize)` con `post(self, msg, wparam, lparam)` (llama `waker().post(self, …)`) e `is_null`.
  - `testing::install_fakes()` y `testing::FakeWaker` (cuenta mensajes por número, consultable desde los tests).

- [ ] **Step 1: Tests que fallan** (en `xdir-core`): `waker_receives_posts_from_many_threads` (8 hilos × 100 `Handle(1).post(…)` con `FakeWaker` → 800 mensajes contados); `missing_install_panics_with_message` (en un proceso de test separado no se puede «desinstalar»: probar la función interna `get_or_panic` con un `OnceLock` local vacío → `#[should_panic(expected = "servicios del sistema no instalados")]`).
- [ ] **Step 2:** `cargo test -p xdir-core` → FAIL.
- [ ] **Step 3:** Implementar el registro con `OnceLock<Services>`; mover `Handle`; implementaciones `Win32Waker` (`PostMessageW`) y `MacWaker` (cola principal → `HANDLER`, como hoy) en `crates/xdir/src/sys/`; instalar en `run`.
- [ ] **Step 4:** `cargo test` (≥ 130 + nuevos), check de Windows sin warnings, `cargo run` en Mac: navegar, abrir una carpeta grande (los mensajes de carga llegan), abrir xtree y analizar.
- [ ] **Step 5: Commit** `refactor(core): registro de servicios del sistema y despertador de la ventana`.

### Task 2: `SystemEnv` (tema, animaciones, teclas, carpeta de configuración, formatos, historial de rutas, consolas)

**Files:**
- Modify: `crates/xdir-core/src/sys/mod.rs` (métodos del trait), `crates/xdir-core/src/sys/testing.rs` (falso)
- Modify: `crates/xdir/src/sys/{win32,macos}.rs` (implementación, moviendo el código de `platform/{win32,macos}.rs`, `format.rs` (fechas y tamaños), `typed.rs` (registro / `rutas.txt`), `console.rs` (lista, nombre, verbos, abrir), `session.rs::config_dir`)
- Modify (fachadas que pasan a llamar `sys::env()` sin cambiar sus nombres públicos): `platform/mod.rs` (`system_dark`, `system_accent`, `animations_allowed`, `key_down`), `format.rs` (`date`, `size_kb`, `size_short`), `typed.rs` (`load`, `save` privados), `console.rs` (`Console` pasa a ser un índice opaco con `label`, `alt_verbs`, `open`, `open_as`, `all()`), `session.rs` (`config_dir`)
- Move a `xdir-core` lo que queda neutro: `format.rs` completo (fachada ya en el núcleo), `typed.rs` (sugerencias y `push`; el dibujo `Suggest::draw` usa `Painter` y queda en `xdir` hasta la fase 3: separarlo en `typed/draw.rs`), `console.rs` (el tipo y su lista)

**Interfaces:**
- Produces (métodos de `SystemEnv`, todos `&self`):
  - `fn system_dark(&self) -> bool`, `fn system_accent(&self) -> Option<u32>`, `fn animations_allowed(&self) -> bool`, `fn key_down(&self, k: Key) -> bool`
  - `fn config_dir(&self) -> Option<PathBuf>` (el modo portable `xdir.portable` lo resuelve el núcleo antes de llamar; la plataforma da `%APPDATA%\xdir` o `~/Library/Application Support/xdir`)
  - `fn format_date(&self, filetime: u64) -> String`, `fn format_size_kb(&self, bytes: u64) -> String`, `fn format_size_short(&self, bytes: u64) -> String`
  - `fn typed_paths_load(&self) -> Vec<String>`, `fn typed_paths_save(&self, list: &[String])`
  - `fn consoles(&self) -> Vec<ConsoleInfo>` con `pub struct ConsoleInfo { pub id: u8, pub label: &'static str }`, `fn console_alt_verbs(&self) -> &'static [(&'static str, &'static str)]`, `fn open_console(&self, owner: Handle, id: u8, dir: &Path, verb: &str) -> bool`
  - El falso: `config_dir` = carpeta temporal por hilo (idéntico al `#[cfg(test)] config_dir` de hoy), formatos deterministas (`date` → `"AAAA-MM-DD"` desde el filetime; tamaños con el formato de Windows de hoy), historial en memoria, una consola «Terminal».

- [ ] **Step 1: Tests que fallan**: `fake_config_dir_is_per_thread` (dos hilos → dos carpetas distintas, mismo hilo → la misma); `portable_mode_wins` (con `xdir.portable` junto a un ejecutable simulado, `core::session::config_dir()` usa esa carpeta sin llamar a la plataforma: separar la función pura `resolve_config_dir(exe_dir: Option<&Path>, platform: impl FnOnce() -> Option<PathBuf>)`); `consoles_menu_same_entries` (en `xdir`, con la implementación real: Windows → `["PowerShell", "CMD"]` y 2 verbos; macOS → `["Terminal"]` y 0 verbos, `cfg!` dentro del test).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar y mover. `#[cfg(test)] fn config_dir` de `session.rs` desaparece (lo reemplaza el falso).
- [ ] **Step 4:** `cargo test` (todos los tests de sesión, grupos, favoritos y paleta pasan), check de Windows, `cargo run`: modo oscuro y acento siguen al sistema; el menú de terminales muestra lo mismo; las fechas y tamaños en la lista se ven igual (comparar una captura de la vista de detalles antes y después en Mac).
- [ ] **Step 5: Commit** `refactor(core): entorno del sistema (tema, formatos, configuración, consolas) detrás de SystemEnv`.

### Task 3: Atajos y nombres de comandos por sistema

**Files:**
- Modify: `crates/xdir-core/src/sys/mod.rs` (`SystemEnv` suma `fn key_style(&self) -> KeyStyle` y `fn command_overrides(&self) -> &'static [(Command, &'static [&'static str], Option<&'static str>)]`, o la forma equivalente que quien implementa vea más simple, mientras la tabla de macOS viva en la plataforma)
- Move: `crates/xdir/src/commands.rs` → `crates/xdir-core/src/commands.rs` (con `MAC_KEYS` / `MAC_NAMES` movidos a `crates/xdir/src/sys/macos.rs` y `combo_label` parametrizado por `KeyStyle`); `crates/xdir/src` lo reexporta
- `available()`: la regla «en macOS no hay `Cmd` ni `ToggleQuickPin`» pasa a `SystemEnv` (`fn command_available(&self, c: Command) -> bool`); la de `ToggleTree` según la feature `arbol` queda donde está la feature

**Interfaces:**
- Produces: `xdir_core::commands::{Command, Spec, ALL, available, …}` con la misma API pública de hoy; `pub enum KeyStyle { Windows, Mac }`.

- [ ] **Step 1: Test que falla**: `mac_table_is_unchanged` — antes de mover, volcar en un test la lista completa `(id, keys(), title(), combo_label de cada tecla)` para macOS y para Windows como constantes esperadas; después de mover, el mismo test (en `xdir`, con la implementación real y `cfg!` para elegir la tabla esperada) tiene que dar idéntico. Escribirlo primero sobre el código actual (pasa), después mover (debe seguir pasando).
- [ ] **Step 2:** Mover y parametrizar.
- [ ] **Step 3:** `cargo test`, check de Windows, `cargo run` en Mac: la paleta muestra los mismos atajos con ⌘⌥⇧.
- [ ] **Step 4: Commit** `refactor(core): comandos en el núcleo con atajos por sistema`.

### Task 4: `FileOps` y tipos de operaciones en el núcleo

**Files:**
- Move: `Op`, `Undo`, `OpResult`, `WM_APP_OP_DONE` de `crates/xdir/src/ops/mod.rs` → `crates/xdir-core/src/ops.rs`; `crates/xdir/src/ops/mod.rs` los reexporta y su `spawn` pasa a `sys::file_ops().spawn(…)`
- Modify: `crates/xdir-core/src/sys/mod.rs` (`FileOps`), `crates/xdir/src/sys/{win32,macos}.rs` (registran `ops::win32::spawn` / `ops::macos::spawn` envueltos), `crates/xdir-core/src/sys/testing.rs` (`FakeFileOps` que registra la `Op` recibida y responde por el canal con un `OpResult` configurable)
- Test: `crates/xdir/src/app/tests.rs`

**Interfaces:**
- Produces: `pub trait FileOps: Send + Sync { fn spawn(&self, owner: Handle, tx: Sender<OpResult>, op: Op); }`.

- [ ] **Step 1: Tests que fallan** (en `app/tests.rs`, con el falso): `rename_sends_rename_op` (renombrar en línea → `Op::Rename` con el nombre nuevo); `undo_after_new_folder_sends_undo` (respuesta con `Undo::Created` → Ctrl+Z manda `Op::Undo(Undo::Created(..))`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Mover los tipos y conectar el trait.
- [ ] **Step 4:** `cargo test`, check de Windows, `cargo run` en Mac: copiar, mover, borrar a la Papelera, renombrar, carpeta nueva y deshacer funcionan como antes.
- [ ] **Step 5: Commit** `refactor(core): operaciones de archivos detrás de FileOps`.

### Task 5: PRs encadenados (con confirmación del usuario)

- [ ] **Step 1:** Mostrar el resumen al usuario y **pedir confirmación** antes del push.
- [ ] **Step 2:** Push de las cuatro ramas (`refactor/sistema`, `refactor/entorno`, `refactor/comandos`, `refactor/fileops`; cada una sale de la anterior) y PRs encadenados: el primero contra `refactor/workspace` si el PR #6 sigue abierto, o contra `develop` si ya se mergeó. En cada cuerpo, la lista corta de pruebas manuales en Windows (arranque, navegar, tema y acento, menú de terminales, fechas y tamaños en la lista, atajos de la paleta, copiar/mover/borrar/renombrar/deshacer) y el recordatorio: **al mergear uno, cambiar la base del siguiente a `develop`**.
- [ ] **Step 3:** Esperar la CI de cada PR y reportar al usuario.

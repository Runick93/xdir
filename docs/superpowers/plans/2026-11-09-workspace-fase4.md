# Workspace, fase 4: `App` al núcleo y una crate por sistema — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** La `App` (estado y lógica de la ventana) vive en `xdir-core`, y `crates/xdir` se reemplaza por `crates/xdir-windows` y `crates/xdir-mac`, adaptadores finos (arranque, mensajes, hosts y servicios del sistema) que generan el mismo `xdir.exe` y `xdir.app` de siempre.

**Architecture:** Primero se deja el código de `App` sin dependencias de `crates/xdir` (fachadas → `sys::*`, tipos neutros de arrastre al núcleo) y el código de plataforma (`messages.rs`, `win.rs`, `mac.rs`, `uia.rs`) hablándole a `App` solo por una API pública; después `app/` se mueve a `xdir_core::app`. Luego se separa macOS en `crates/xdir-mac` y lo que queda se renombra `crates/xdir-windows`; cada crate de plataforma compila a un `main` que avisa y sale fuera de su sistema. Los scripts de empaquetado copian el binario (`xdir-windows.exe` / `xdir-mac`) con el nombre de siempre.

**Tech Stack:** Rust 2024, workspace (`crates/*`), `windows` 0.62, `objc2` 0.6, Inno Setup, GitHub Actions (ci.yml, release.yml).

**Spec:** `docs/superpowers/specs/2026-11-08-workspace-core-design.md` (Restricciones; Arquitectura → Workspace; paso 5 del orden de trabajo)

## Global Constraints

- Ningún cambio de comportamiento ni visual. `cargo test` en la raíz del workspace pasa en macOS con ≥ 185 tests en cada tarea (hoy 91 en `xdir` + 94 en `xdir-core`; los que se mueven cuentan igual); `cargo check --all-targets --target x86_64-pc-windows-msvc` (también con `--features pruebas` y `--features arbol`) sin errores ni warnings nuevos; CI verde en cada PR, incluida la prueba de humo de Windows.
- `xdir-core` sin dependencias y sin `#[cfg(windows)]` / `#[cfg(target_os)]` en producción (sí `cfg!(feature = …)` y en tests).
- Cada crate de plataforma genera un binario con su propio nombre (`xdir-windows`, `xdir-mac`), así `cargo build --workspace` no produce dos `target/<perfil>/xdir` que se pisen. `scripts/package-mac.sh`, `scripts/package.ps1`, `installer/xdir.iss` y `release.yml` lo copian como `xdir` / `xdir.exe`: el usuario ve el mismo nombre de siempre.
- `cargo build` y `cargo test` en la raíz funcionan en Windows, macOS y Linux: las crates de otro sistema compilan vacías (un `main` que avisa y sale), con todo su código real bajo `#[cfg(…)]` en la raíz de la crate.
- `build.rs` (recursos de Windows) pasa a `xdir-windows`. `.cargo/config.toml` queda en la raíz: Cargo lee la configuración por carpeta de trabajo y no por paquete, y la regla ya es solo para `x86_64-pc-windows-msvc`.
- Features `arbol` y `pruebas`: las declara `xdir-core` (donde vive el código que las usa) y cada crate de plataforma las reexpone (`arbol = ["xdir-core/arbol"]`, `pruebas = ["xdir-core/pruebas"]`); `default = ["arbol"]` en las dos crates de plataforma.
- Paquetes «iguales»: mismos archivos dentro del zip, el dmg y el instalador, misma versión, y la app arranca. No idénticos byte a byte.
- Tests: toda prueba que llegue a un servicio llama `install_fakes()`; los de `App` usan `FakeHost`. Los que dependen del sistema real (`sys/tests.rs`, `fs/macos/xtree.rs`, `mft.rs`) quedan en su crate de plataforma.
- Prueba de humo en la Mac: copia aislada con `xdir.portable` + `XDIR_PRUEBAS`; nunca tocar `/Applications/xdir.app` ni su configuración; matar solo el PID propio.
- No correr `cargo fmt` sobre todo el repo. Castellano; Conventional Commits sin atribución a Claude.
- PRs **todos con base `develop`**, mergeados en orden: PR 1 = Task 1–2 en `refactor/app-core`; PR 2 = Task 3–5 en `refactor/crates-sistema` (sale de `refactor/app-core`). Cada cuerpo lleva la lista de pruebas de Windows para Runick93. Después del merge, borrar las ramas propias.

## Review Focus

- **Nombre del binario en todos lados**: un script, el instalador, `release.yml` o `humo.ps1` que siga buscando `target/…/xdir(.exe)` rompe el release o la CI sin que ningún test de Rust lo note → Task 4 verifica con `grep` que no quede ninguna ruta al binario viejo y la CI corre la prueba de humo con el exe nuevo; Task 5 arma los paquetes y compara su contenido.
- **Arranque con `--probe` y con una carpeta como argumento** (`xdir.exe C:\algo`): los argumentos los lee `startup_args`, que pasa al núcleo; tiene que seguir funcionando igual → test de `startup_args` en el núcleo con `--probe archivo carpeta` y con solo una carpeta (Task 2).
- **Ayudante elevado de xtree en Windows** (`--xtree-mft`): el `main` de `xdir-windows` tiene que salir por el ayudante antes de instalar servicios, como hoy → Task 4 lo deja primero en `main` y la lista para Runick93 incluye un escaneo de xtree de un disco entero.
- **`cargo build --workspace` en Linux**: las dos crates de plataforma compilan su stub y `xdir-core` sus tests → la CI de Linux pasa de `cargo test -p xdir-core` a `cargo test` en la raíz (Task 4).
- **Versión en el release**: `release.yml` sube la versión en `Cargo.lock` buscando `name = "xdir"`; con las crates nuevas tiene que actualizar `xdir-windows`, `xdir-mac` y `xdir-core`, o el `--locked` del build falla → Task 4 cambia el `sed` y Task 5 lo prueba con un `Cargo.lock` de prueba.

---

### Task 1: `App` sin dependencias de `crates/xdir`

**Files:**
- Create: `crates/xdir-core/src/drop.rs` (`DragKind`, `Effect`, `DragEvent`, `WM_APP_DRAG`, movidos de `crates/xdir/src/drop/mod.rs`; `drop/mod.rs` los reexporta y conserva el submódulo `win32`)
- Modify: `crates/xdir/src/app/{mod,actions,appearance,edit,events,favs,fileops,input,panes,paint,quick_jump,saved_groups}.rs`: las llamadas a fachadas de `crates/xdir` (`shell::*`, `fs::*`, `ops::spawn`, `icons::WM_APP_ICONS`, `console::Console`, `render::{Rect, rect, contains}`, `platform::*`) pasan a `xdir_core::sys::*` y a los tipos del núcleo; `icons::WM_APP_ICONS` pasa a `xdir_core::paint` (mismo valor, `WM_APP + 3`).
- Modify: `crates/xdir/src/app/{messages,win,mac,uia}.rs`: dejan de ser `impl App` y de leer campos privados. `messages.rs` pasa a `pub(super) fn handle(app: &mut App, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT`; lo que leían de `App` lo piden por la API de abajo.
- Modify: `crates/xdir/src/fs/mod.rs`, `shell/mod.rs`, `ops/mod.rs`, `icons/mod.rs`, `console.rs`: se borran las funciones de fachada que queden sin uso (quedan los submódulos de cada sistema).

**Interfaces:**
- Produces (en `App`, visibles para el código de plataforma; `pub` en vez de `pub(super)` donde haga falta):
  - Los métodos `on_*` que hoy llaman `messages.rs` y `mac.rs` (`on_key`, `on_char`, `on_alt_key`, `on_left_down`, `on_left_up`, `on_right_down`, `on_middle_click`, `on_mouse_move`, `on_mouse_left`, `on_wheel`, `on_wheel_h`, `on_back_forward`, `on_context_menu_request`, `on_drag`, `on_resized`, `on_closing`, `on_app_message`, `on_app_activated`, `on_system_theme_changed`, `on_text_changed`, `drives_changed`), más `paint`, `redraw`, `hit`/`cursor_at` (lo que use `WM_SETCURSOR`), `apply_window_theme`, `apply_animation_setting`, `sync_visibility`, `start_quick_access`, `set_accent`, `run_command`, y `test_hook` (feature `pruebas`).
  - `pub fn attach(&mut self, window: Handle, dpi: u32)` (fija `hwnd` y `dpi` y llama `host.attach`), `pub fn set_dpi(&mut self, dpi: u32)`, `pub fn window(&self) -> Handle`, `pub fn dpi(&self) -> u32`, `pub fn take_window_state(&mut self) -> Option<([i32; 4], bool)>`, `pub fn mark(&mut self, name: &'static str)` (agrega un hito de `--probe` con el tiempo desde el arranque), `pub fn dark_mode(&self) -> Option<bool>`.
  - `App` sigue con `hwnd` (es el dueño `Handle` que usan los servicios) y `dpi` (para `--probe` y las coordenadas del mouse); quedan documentados como «los fija la plataforma con `attach` / `set_dpi`».
- `drop`: `xdir_core::drop::{DragKind, Effect, DragEvent, WM_APP_DRAG}` con los mismos valores.

- [ ] **Step 1: Test que falla**: `attach_sets_window_and_dpi` en `crates/xdir/src/app/tests.rs` (`App::new(Init::blank(), FakeHost::new())`, `app.attach(Handle(7), 144)` → `app.window() == Handle(7)`, `app.dpi() == 144`, y el `FakeHost` registró el `attach` con `Handle(7)`; sumar `fn attached(&self) -> Option<Handle>` al `FakeHost`).
- [ ] **Step 2:** FAIL (no existen `attach` ni `window`).
- [ ] **Step 3:** Implementar la API, mover los tipos de arrastre, cambiar las fachadas por `sys::*` y reescribir `messages.rs`/`win.rs`/`mac.rs`/`uia.rs` contra la API.
- [ ] **Step 4:** PASS; Global Constraints; `grep -n "crate::\(shell\|fs\|ops\|icons\|console\|render\|platform\|drop\)" crates/xdir/src/app/{mod,actions,appearance,edit,events,favs,fileops,input,panes,paint,quick_jump,saved_groups,tests}.rs` vacío; ninguno de `messages.rs`, `win.rs`, `mac.rs`, `uia.rs` tiene `impl App`.
- [ ] **Step 5: Commit** `refactor(app): la App solo depende del núcleo y la plataforma le habla por su API`.

### Task 2: `App` al núcleo

**Files:**
- Move: `crates/xdir/src/app/{mod,actions,appearance,edit,events,favs,fileops,input,panes,paint,quick_jump,saved_groups,tests}.rs` → `crates/xdir-core/src/app/` (con `git mv`). En `crates/xdir/src/app/` quedan `win.rs`, `messages.rs`, `uia.rs`, `host_win.rs`, `mac.rs`, `host_mac.rs` y un `mod.rs` de plataforma que los declara y reexporta `xdir_core::app::App`.
- Modify: `crates/xdir-core/Cargo.toml` (feature `pruebas = []`); `crates/xdir/Cargo.toml` (`pruebas = ["xdir-core/pruebas"]`); `crates/xdir-core/src/lib.rs` (`pub mod app;`, `pub mod drop;`).
- Los `#[cfg(windows)]` / `#[cfg(not(windows))]` de `app/tests.rs` (líneas ~673–693) se quedan solo si son de tests; si un test depende del sistema real, pasa a `crates/xdir/src/sys/tests.rs`.

**Interfaces:**
- Consumes: Task 1.
- Produces: `xdir_core::app::{App, Init, startup_args}` con la API de la Task 1; `INITIAL_SIZE` y las constantes `WM_APP_*` de la `App` públicas donde la plataforma las use.

- [ ] **Step 1: Tests que fallan** (en el núcleo): los de `app/tests.rs` movidos, más `startup_args_reads_probe_and_folder` y `startup_args_folder_only` (para poder probarla sin `std::env::args`, separar `pub fn parse_args(args: &[String]) -> (bool, Option<String>, Option<PathBuf>)` —sesión limpia por `--probe`, archivo de `--probe`, carpeta si existe— y que `startup_args` la use: `["--probe", "p.txt", <tmp>]` → `(true, Some("p.txt"), Some(<tmp>))`; `[<tmp>]` → `(false, None, Some(<tmp>))`; `["--probe", "p.txt"]` → sin carpeta; una carpeta que no existe → `None`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Mover y conectar.
- [ ] **Step 4:** PASS; Global Constraints (≥ 185 + los nuevos); `cargo test -p xdir-core` y `cargo test -p xdir-core --features arbol,pruebas` solos; humo aislado en la Mac con las capturas de siempre (inicio, paleta, «Acerca de», colores, íconos grandes, oscuro, xtree) comparadas con las de `develop`.
- [ ] **Step 5: Commit** `refactor(core): la App pasa al núcleo`.
- [ ] **Step 6: Abrir PR 1** (`refactor/app-core`, base `develop`) con confirmación del usuario. Lista para Runick93: abrir y cerrar xdir, navegar, menú contextual, arrastrar archivos desde y hacia el Explorador, cajas de texto (barra, renombrar, filtro, paleta), menú de favoritos con clic derecho, Narrador básico; todo igual que en `develop`.

### Task 3: `crates/xdir-mac`

**Files:**
- Create: `crates/xdir-mac/Cargo.toml` (`[[bin]] name = "xdir-mac"`, `path = "src/main.rs"`; dependencias de macOS bajo `[target.'cfg(target_os = "macos")'.dependencies]` copiadas de `crates/xdir/Cargo.toml`; features como en Global Constraints).
- Move (con `git mv`) a `crates/xdir-mac/src/`: `app/{mac,host_mac}.rs`, y las partes `macos` de `fs`, `icons`, `ops`, `platform`, `render`, `shell`, `sys`, `thumbs`, con un `mod.rs` por carpeta que declara solo lo de macOS. Los tests del sistema real de macOS (`sys/tests.rs` en sus partes de macOS, `fs/macos/xtree.rs`) se mueven con su código.
- `crates/xdir-mac/src/main.rs`: `#[cfg(target_os = "macos")] fn main() -> Result<(), String> { app::run() }` y, fuera de macOS, `fn main() { eprintln!("xdir-mac solo funciona en macOS"); }`; los módulos bajo `#[cfg(target_os = "macos")]` en la raíz de la crate.
- Modify: `crates/xdir/` queda solo con lo de Windows (borrar el código `#[cfg(target_os = "macos")]` que se movió); `scripts/package-mac.sh` (`lipo` toma `target/<arch>/release/xdir-mac` y lo deja como `Contents/MacOS/xdir`; el build usa `-p xdir-mac`); `.github/workflows/release.yml` (paso de macOS) y `.github/workflows/ci.yml` (job de macOS si nombra el binario).

**Interfaces:**
- Consumes: `xdir_core::app::{App, Init, startup_args}` (Task 2).

- [ ] **Step 1:** Mover y armar la crate.
- [ ] **Step 2:** `cargo build -p xdir-mac` en la Mac deja `target/debug/xdir-mac`; `cargo test` en la raíz ≥ el número de la Task 2; `cargo check --all-targets --target x86_64-pc-windows-msvc` limpio (en Windows, `xdir-mac` compila su stub).
- [ ] **Step 3:** `scripts/package-mac.sh` arma `dist/xdir-<versión>-macos.dmg` y `.zip`; comparar la lista de archivos de la app (`find xdir.app`) y el `Info.plist` con los del mismo script corrido en `develop` (en un worktree): iguales. Humo aislado con `target/release/xdir-mac` (copia con `xdir.portable`) y capturas iguales a las de la Task 2.
- [ ] **Step 4: Commit** `refactor(mac): crate xdir-mac con el arranque y los servicios de macOS`.

### Task 4: `crates/xdir-windows`

**Files:**
- Move: `crates/xdir` → `crates/xdir-windows` (con `git mv`); `Cargo.toml` con `name = "xdir-windows"` y `[[bin]] name = "xdir-windows"`; `build.rs` va con la crate (las rutas `../../assets/…` no cambian de profundidad).
- `crates/xdir-windows/src/main.rs`: `#[cfg(windows)]` el `main` de hoy (con la salida temprana del ayudante `--xtree-mft` antes de instalar servicios) y, fuera de Windows, `fn main() { eprintln!("xdir-windows solo funciona en Windows"); }`; los módulos bajo `#[cfg(windows)]` en la raíz. `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` se conserva.
- Modify: `scripts/humo.ps1` (por defecto `target\debug\xdir-windows.exe`), `scripts/package.ps1` (por defecto `target\release\xdir-windows.exe`; el zip y el instalador lo copian como `xdir.exe`), `installer/xdir.iss` (la fuente es el exe nuevo y se instala como `xdir.exe`: `Source: {#SourceExe}; DestName: "xdir.exe"`), `.github/workflows/ci.yml` (humo con `target\debug\xdir-windows.exe`; Linux pasa a `cargo test --locked` en la raíz), `.github/workflows/release.yml` (exe nuevo; el `sed` de `Cargo.lock` actualiza `xdir-windows`, `xdir-mac` y `xdir-core`), `scripts/build.cmd` si nombra la crate.

**Interfaces:**
- Consumes: Task 2–3.

- [ ] **Step 1:** Renombrar, armar los stubs y ajustar scripts, instalador y workflows.
- [ ] **Step 2:** `cargo check --all-targets --target x86_64-pc-windows-msvc` (también `--features pruebas` y `arbol`) limpio; `cargo build --workspace` y `cargo test` en la Mac pasan (el stub de Windows compila); `grep -rn "release\\\\xdir.exe\|debug\\\\xdir.exe\|release/xdir\b\|crates/xdir/" scripts installer .github README.md docs/DISENO.md` no encuentra rutas viejas (las de `crates/xdir-core` no cuentan).
- [ ] **Step 3:** Probar el `sed` nuevo de `release.yml` sobre una copia de `Cargo.lock` con la versión cambiada: las 3 crates quedan con la versión nueva y `cargo metadata --locked --offline` sobre esa copia no falla (hacerlo en un worktree temporal).
- [ ] **Step 4: Commit** `refactor(windows): crate xdir-windows y el binario con el nombre de siempre en los paquetes`.

### Task 5: Verificación de paquetes, Linux y documentación

**Files:**
- Modify: `README.md` (arquitectura: `xdir-core`, `xdir-windows`, `xdir-mac`; cómo compilar cada uno: `cargo run -p xdir-windows` / `cargo run -p xdir-mac`), `docs/DISENO.md` si nombra `crates/xdir/`.

- [ ] **Step 1:** Desde la Mac: `rustup target add x86_64-unknown-linux-gnu` (si falta) y `cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu`: compilan `xdir-core` y los dos stubs, sin warnings. La CI de Linux del PR lo confirma corriendo `cargo test`.
- [ ] **Step 2:** Paquete de macOS ya comparado en la Task 3. Para Windows, la CI del PR corre build + test + humo con el exe nuevo; el zip y el instalador los arma `release.yml`, así que en el cuerpo del PR 2 va la lista para Runick93: `scripts\package.ps1` local → el zip contiene `xdir.exe` y `xdir.portable`; el instalador instala `xdir.exe`, accesos directos y desinstalación como antes.
- [ ] **Step 3:** Actualizar `README.md` y `docs/DISENO.md`.
- [ ] **Step 4: Commit** `docs: README con las crates por sistema`.
- [ ] **Step 5: Abrir PR 2** (`refactor/crates-sistema`, base `develop`) con confirmación del usuario. Lista para Runick93: `scripts\build.cmd build --release` y `scripts\package.ps1` → mismos archivos que antes en el zip y el instalador; instalar, abrir, actualizar sobre una versión anterior; `xdir.exe C:\carpeta` abre esa carpeta; xtree sobre un disco entero (ayudante elevado).

---

Después del merge: verificar en `develop`, borrar las ramas propias y el espacio de trabajo del plan, y anotar la fase en la nota del vault. Sigue el paso 6 del diseño (`xdir-linux`).

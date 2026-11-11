# Workspace, fase 1: CI, workspace y núcleo inicial — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Dejar a xdir con CI en cada PR (con prueba de humo en Windows), convertido en workspace de Cargo y con una primera crate `xdir-core` que compila y se prueba también en Linux, sin cambiar ningún comportamiento.

**Architecture:** Tres PRs encadenados (pasos 1, 2 y 3 del diseño): (1) workflow `ci.yml` + script de humo; (2) mover la crate a `crates/xdir` dentro de un workspace; (3) crear `crates/xdir-core` con lo que ya no depende de ninguna fachada y reexportarlo desde `xdir` para que el resto del código no cambie. Las interfaces (paso 4 en adelante) van en planes propios.

**Tech Stack:** Rust 2024, Cargo workspaces, GitHub Actions (windows-latest, macos-latest, ubuntu-latest), PowerShell 7 + C# por `Add-Type` para la prueba de humo.

**Spec:** `docs/superpowers/specs/2026-11-08-workspace-core-design.md`

## Global Constraints

- Ningún cambio de comportamiento: `cargo test` en macOS da los mismos 130 tests (o más) en cada tarea; `cargo check --all-targets --target x86_64-pc-windows-msvc` sin errores ni warnings.
- El paquete de macOS (`sh scripts/package-mac.sh`) contiene los mismos archivos antes y después de cada tarea (comparar `find dist/xdir.app -type f | sort`).
- Ramas encadenadas: Task 1 en `refactor/ci` (la rama actual `refactor/workspace-core` se renombra), Task 2 en `refactor/workspace` (sale de `refactor/ci`), Task 3 en `refactor/xdir-core` (sale de `refactor/workspace`). Sin push hasta la Task 4.
- `xdir-core` sin dependencias (`[dependencies]` vacío) y sin `#[cfg(windows)]` / `#[cfg(target_os)]` en código de producción.
- Versión única en `[workspace.package]` del `Cargo.toml` raíz; cada crate usa `version.workspace = true` y `edition.workspace = true`.
- `[profile.release]` y `.cargo/config.toml` quedan en la raíz del workspace (Cargo solo los lee ahí).
- `assets/` queda en la raíz (lo usan los scripts de empaquetado).
- Textos y comentarios en castellano; Conventional Commits sin atribución a Claude.

## Review Focus

- **`release.yml` sube la versión** con `sed` sobre la primera línea `version =` del `Cargo.toml` raíz y la entrada `name = "xdir"` de `Cargo.lock`: con workspace debe actualizar `[workspace.package]` y también la entrada de `xdir-core` en `Cargo.lock`, o el build con `--locked` falla → verificación en Task 3 (simular el `sed` localmente y correr `cargo build --locked`).
- **Rutas relativas que cambian al mover la crate**: `include_bytes!("../assets/runick-logo.bgra")` en `about.rs`, `assets/xdir.rc` en `build.rs`, y las rutas dentro de `assets/xdir.rc` → verificación en Task 2 (`cargo build` en Mac y `cargo check` para Windows; el `.rc` solo se compila en Windows: lo cubre la CI de la Task 1).
- **La prueba de humo puede fallar por el entorno del runner** (sin GPU, sesión no interactiva) y no por xdir → la Task 1 hace que el script distinga «no arrancó / no responde» (falla) de «la captura salió vacía» (aviso, con la imagen como artefacto).
- **`cargo test` en Linux** no puede compilar la crate `xdir` (no tiene `main` para Linux) → el job de Linux corre solo `-p xdir-core` (Task 3).
- **Reexportar módulos movidos** (`pub(crate) use xdir_core::toml;` etc.) tiene que dejar todas las rutas `crate::toml::…`, `crate::format::natural_cmp`, `crate::platform::Color`, `crate::xtree::tree::…` funcionando sin tocar sus usos → verificación en Task 3 (compila sin cambios en los archivos que los usan).

---

### Task 1: CI en cada PR con prueba de humo en Windows

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `scripts/humo.ps1`
- Modify: nada de código Rust.

**Interfaces:**
- Produces: workflow `CI` que corre en `pull_request` (cualquier rama base) y en `push` a `develop` y `main`; job `windows` que deja la captura de la prueba de humo como artefacto `humo-windows`.

- [ ] **Step 1:** `git branch -m refactor/workspace-core refactor/ci`.
- [ ] **Step 2: `scripts/humo.ps1`** (PowerShell 7). Parámetros: `-Exe` (por defecto `target\debug\xdir.exe`), `-Out` (carpeta para la captura, por defecto `humo`). Hace, en orden:
  1. Crea una carpeta temporal con 3 archivos y 2 subcarpetas, y otra para la configuración; pone `$env:APPDATA` en esta última (aísla `%APPDATA%\xdir`).
  2. `Start-Process $Exe -ArgumentList <carpeta temporal> -PassThru`.
  3. Espera hasta 30 s a `FindWindow("xdir.main", null)`; si no aparece → `exit 1` con «no apareció la ventana».
  4. Manda por `WM_COPYDATA` (`SendMessage`, `dwData = 0`, texto UTF-16 con NUL final), con 500 ms entre cada uno, y exige que cada `SendMessage` devuelva 1 y que el proceso siga vivo: `cmd layout_4`, `cmd layout_1`, `cmd paleta`, `close`, `cmd xtree`, (espera 3 s), `cmd xtree`, `notice humo`.
  5. Captura la ventana con `PrintWindow(hwnd, hdc, 2)` (PW_RENDERFULLCONTENT) a `$Out\humo.png`; si la imagen tiene menos de 8 colores distintos → `Write-Warning` «la captura salió vacía» (no falla).
  6. `PostMessage(WM_CLOSE)`, espera hasta 15 s a que el proceso termine; si no termina o el código de salida no es 0 → `exit 1`.
  Las llamadas Win32 (`FindWindow`, `SendMessage` con `COPYDATASTRUCT`, `PostMessage`, `PrintWindow`, `GetWindowRect`) van en un bloque C# con `Add-Type`.
- [ ] **Step 3: `.github/workflows/ci.yml`**:
  - `windows` (windows-latest): checkout, `rustup toolchain install stable --profile minimal`, `Swatinem/rust-cache@v2`, `cargo build --locked`, `cargo test --locked`, `cargo build --locked --features pruebas`, `pwsh scripts/humo.ps1 -Exe target\debug\xdir.exe -Out humo`, `actions/upload-artifact@v4` con `humo/` (`if: always()`).
  - `macos` (macos-latest): checkout, toolchain, cache, `cargo test --locked`.
  - `estilo` (ubuntu-latest, `continue-on-error: true`): `cargo fmt --check`. (clippy se suma cuando exista `xdir-core`, que compila en Linux.)
- [ ] **Step 4:** Verificar lo que se puede desde la Mac: `actionlint .github/workflows/ci.yml` si está instalado (si no, `brew install actionlint`); `pwsh -NoProfile -Command "[ScriptBlock]::Create((Get-Content scripts/humo.ps1 -Raw)) | Out-Null"` para chequear la sintaxis si `pwsh` está; si no hay `pwsh`, dejarlo anotado. La corrida real es en GitHub (Task 4).
- [ ] **Step 5: Commit** `ci: tests en Windows y macOS en cada PR y prueba de humo en Windows`.

### Task 2: Workspace con la app en `crates/xdir` (sin cambiar lógica)

**Files:**
- Move (git mv): `src/` → `crates/xdir/src/`, `build.rs` → `crates/xdir/build.rs`
- Create: `crates/xdir/Cargo.toml` (el `[package]`, `[[bin]]`, `[features]` y dependencias de hoy, con `version.workspace = true`, `edition.workspace = true`)
- Modify: `Cargo.toml` raíz (pasa a `[workspace] members = ["crates/*"]`, `resolver = "3"`, `[workspace.package] version = "<la de hoy>" edition = "2024"`, y `[profile.release]` de hoy)
- Modify: `crates/xdir/src/about.rs` (`include_bytes!("../../../assets/runick-logo.bgra")`), `crates/xdir/build.rs` (ruta a `assets/xdir.rc` y `rerun-if-changed` relativas a la raíz: `../../assets/…`; verificar las rutas dentro de `assets/xdir.rc`)
- Modify: `Cargo.lock` (lo regenera Cargo; sin cambiar versiones de dependencias: `cargo update -w` no, solo `cargo build`)
- Sin cambios esperados: `scripts/package-mac.sh`, `scripts/package.ps1`, `installer/xdir.iss`, `.github/workflows/*` (el binario sigue en `target/<perfil>/xdir`, la versión sigue siendo la primera línea `version =` del `Cargo.toml` raíz). Si alguno necesita cambio, se hace en esta tarea.

**Interfaces:**
- Produces: paquete `xdir` en `crates/xdir`; binario `target/<perfil>/xdir(.exe)` como hoy.

- [ ] **Step 1:** `git switch -c refactor/workspace`.
- [ ] **Step 2: Línea de base** (antes de mover): `cargo test 2>&1 | grep "test result"` (anotar totales) y `sh scripts/package-mac.sh && find dist/xdir.app -type f | sort > /tmp/<scratch>/antes.txt` (en el scratchpad de la sesión).
- [ ] **Step 3:** Mover y crear los `Cargo.toml` como se indica en Files.
- [ ] **Step 4:** `cargo build`, `cargo test` (mismos totales que la línea de base), `cargo check --all-targets --target x86_64-pc-windows-msvc` (sin warnings), `cargo build --features pruebas`.
- [ ] **Step 5:** `sh scripts/package-mac.sh` y `find dist/xdir.app -type f | sort > …/despues.txt`; `diff antes.txt despues.txt` vacío; la versión del dmg es la misma; abrir `dist/xdir.app` (`open`) y confirmar que arranca (`pgrep -f dist/xdir.app`), después cerrarla con `osascript -e 'quit app "xdir"'` — ojo: si el usuario tiene `/Applications/xdir.app` abierto, `quit app "xdir"` cierra ese; usar `kill <pid>` del proceso de `dist/`.
- [ ] **Step 6:** Simular el bump de `release.yml` en una copia temporal del árbol (o con `git stash` después): aplicar sus dos `sed` (sintaxis GNU: en la Mac usar `gsed` si está, o reproducir el efecto con `perl -pi`) y correr `cargo build --locked`; tiene que compilar. Si el `sed` sobre `Cargo.toml` ya no encuentra la versión (porque ahora está en `[workspace.package]`), ajustarlo en `release.yml`.
- [ ] **Step 7: Commit** `build: workspace de Cargo con la app en crates/xdir`.

### Task 3: Crate `xdir-core` con lo que ya es neutro, y Linux en la CI

**Files:**
- Create: `crates/xdir-core/Cargo.toml` (`[package] name = "xdir-core"`, `version.workspace`, `edition.workspace`, sin dependencias), `crates/xdir-core/src/lib.rs`
- Move (git mv) a `crates/xdir-core/src/`:
  - `crates/xdir/src/toml.rs` → `toml.rs`
  - `crates/xdir/src/xtree/{colors.rs, tree.rs, treemap.rs}` → `xtree/{colors.rs, tree.rs, treemap.rs}` (con `xtree/mod.rs` nuevo en el núcleo que declara solo esos tres)
- Create: `crates/xdir-core/src/geom.rs` con `Color`, `Rect`, `rect`, `contains` (movidos de `crates/xdir/src/platform/mod.rs`), y `crates/xdir-core/src/keys.rs` con `Key` y `pub mod keys { VK_* }` (movidos del mismo archivo). `WM_APP` también va al núcleo (`lib.rs` o `geom.rs`, a elección). `Handle` **no** se mueve (su `post` llama a la plataforma; va con la interfaz `Waker` en el plan siguiente).
- Create: `crates/xdir-core/src/format.rs` con `thousands`, `natural_cmp`, `caseless_cmp` y sus tests (movidos de `crates/xdir/src/format.rs`; `date`, `size_kb`, `size_short` quedan en `xdir`).
- Modify: `crates/xdir/Cargo.toml` (`xdir-core = { path = "../xdir-core" }`), `crates/xdir/src/main.rs` (`pub(crate) use xdir_core::toml;` en lugar de `mod toml;`), `crates/xdir/src/platform/mod.rs` (`pub use xdir_core::geom::{Color, Rect, rect, contains}; pub use xdir_core::keys::{Key, keys}; pub use xdir_core::WM_APP;`), `crates/xdir/src/format.rs` (`pub use xdir_core::format::{thousands, natural_cmp, caseless_cmp};`), `crates/xdir/src/xtree/mod.rs` (`pub use xdir_core::xtree::{colors, tree, treemap};` junto a `scan` y `view` que quedan)
- Modify: `.github/workflows/ci.yml` (job `linux` en ubuntu-latest: `cargo test --locked -p xdir-core`; job `estilo` suma `cargo clippy -p xdir-core -- -D warnings` con `continue-on-error`)
- Modify: `.github/workflows/release.yml` (el bump actualiza también la entrada `name = "xdir-core"` de `Cargo.lock`)

**Interfaces:**
- Produces (lo que usan los planes siguientes): crate `xdir_core` con `pub mod geom`, `pub mod keys`, `pub const WM_APP: u32`, `pub mod toml`, `pub mod format`, `pub mod xtree { pub mod colors; pub mod tree; pub mod treemap; }`. Los usos dentro de `xdir` no cambian gracias a las reexportaciones.

- [ ] **Step 1:** `git switch -c refactor/xdir-core`.
- [ ] **Step 2:** Mover los archivos y crear los reexports. Los tests que viven dentro de los módulos movidos se mueven con ellos.
- [ ] **Step 3:** Comprobar que ningún archivo que use esos módulos tuvo que cambiar: `git diff --stat` solo muestra los archivos listados en Files.
- [ ] **Step 4:** `cargo test` (la suma de tests de `xdir` + `xdir-core` es igual a la línea de base de la Task 2), `cargo test -p xdir-core` (pasa solo), `cargo check --all-targets --target x86_64-pc-windows-msvc` sin warnings, `cargo check -p xdir-core --target x86_64-unknown-linux-gnu` (con `rustup target add x86_64-unknown-linux-gnu` si falta; solo `check`, sin linkear).
- [ ] **Step 5:** `grep -rn "cfg(windows)\|target_os\|windows::\|objc2" crates/xdir-core/src` → solo coincidencias dentro de `#[cfg(test)]` (por ejemplo el test de `node_path`), ninguna en producción. `cargo tree -p xdir-core` sin dependencias.
- [ ] **Step 6:** Repetir la simulación del bump de versión de la Task 2 (Step 6) con el `release.yml` modificado: `cargo build --locked` compila.
- [ ] **Step 7:** Paquete de macOS igual a la línea de base (mismo `diff` vacío de la Task 2, Step 5).
- [ ] **Step 8: Commit** `refactor(core): crate xdir-core con geometría, teclas, toml, formato y datos de xtree`.

### Task 4: PRs encadenados (con confirmación del usuario)

- [ ] **Step 1:** Mostrar al usuario el resumen de las tres ramas y **pedir confirmación** antes de cualquier push. Recordarle que avise a Runick93 que mergee o pause lo suyo antes de que se mergee el PR del workspace.
- [ ] **Step 2:** Con la confirmación: `git push -u origin refactor/ci refactor/workspace refactor/xdir-core`.
- [ ] **Step 3:** `gh pr create --repo Runick93/xdir` encadenados: `refactor/ci` → base `feature/xtree-macos` (o `develop` si el PR #2 ya se mergeó); `refactor/workspace` → base `refactor/ci`; `refactor/xdir-core` → base `refactor/workspace`. Cada cuerpo: qué cambia, que no cambia comportamiento, cómo se verificó, y la lista corta de pruebas manuales en Windows (arrancar, navegar, cambiar layout, abrir la paleta y xtree, empaquetar con `scripts\package.ps1`). Sin atribución a Claude, sin `@` sueltos.
- [ ] **Step 4:** Esperar la CI del PR de `refactor/ci`; si la prueba de humo o algún job falla, corregir en `refactor/ci` y rebasar las ramas siguientes. Reportar al usuario los enlaces y el estado de la CI.

# Workspace, fase 3a: escaneo de xtree, íconos superpuestos, shell y sistema de archivos — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Las interfaces `DiskScan`, `Overlays`, `Shell`, `FileSystem` y `Watch` definidas en `xdir-core`, implementadas por Windows y macOS, con sus tipos neutros (y el código que deja de necesitar fachadas) movidos al núcleo, sin cambiar comportamiento.

**Architecture:** Igual que la fase 2: cada interfaz es un trait de `xdir_core::sys`, se suma a `Services`, cada plataforma la instala en `crates/xdir/src/sys/{win32,macos}.rs` y los tests usan falsos de `sys::testing`. Las fachadas de `crates/xdir` (`fs`, `shell`, `icons`) conservan los nombres públicos que usa el resto del código y llaman al registro. Lo que es por ventana (`Painter`, `WindowHost`, UIA) queda para la fase 3b.

**Tech Stack:** Rust 2024, workspace con `crates/xdir` y `crates/xdir-core`, CI de la fase 1.

**Spec:** `docs/superpowers/specs/2026-11-08-workspace-core-design.md` (paso 4, interfaces `Overlays`, `Shell`, `FileSystem` + `Watch`, `DiskScan`)

## Global Constraints

- Ningún cambio de comportamiento. `cargo test` en macOS ≥ 147 tests en cada tarea, también corridos de a un hilo por módulo (`cargo test -p xdir <módulo>:: -- --test-threads=1`); `cargo check --all-targets --target x86_64-pc-windows-msvc` sin errores ni warnings; CI verde en cada PR.
- `xdir-core` sin dependencias y sin `#[cfg(windows)]` / `#[cfg(target_os)]` en producción. Traits sin tipos de sistema; errores como `String`.
- Tests: toda prueba que llegue a un servicio llama `install_fakes()`. Rutas de la carpeta temporal en tests que corren en Windows: comparar con `fs::true_case(...)` (el runner da nombres cortos `RUNNER~1`).
- No correr `cargo fmt` sobre todo el repo.
- Ramas encadenadas: Task 1 `refactor/diskscan` (sale de `develop`), Task 2 `refactor/shell`, Task 3 `refactor/filesystem`, Task 4 `refactor/core-modulos`. **Después de mergear cada PR, borrar su rama («Delete branch») para que GitHub pase el siguiente a `develop`.**
- Castellano; Conventional Commits sin atribución a Claude.

## Review Focus

- **Hilos de trabajo y el registro**: `FileSystem::spawn_load`, la búsqueda, el escaneo de xtree y los íconos superpuestos corren en otros hilos; las implementaciones deben ser `Send + Sync` sin `unsafe impl` injustificados (en macOS, los objetos de AppKit no se pueden guardar en el servicio: se crean en el hilo que los usa, como hoy).
- **`Location::title` / `crumbs`** hoy tienen `#[cfg(windows)]` (unidades «Disco local (C:)» vs «Macintosh HD»): la barra de dirección y los títulos de pestaña tienen que verse igual en cada sistema → test con el entorno real de cada sistema (`cfg!`) que fije títulos y migas para `C:\Users` / `/Users`.
- **Errores de `shell`**: hoy `windows::core::Result` en Windows y `ShellError` en macOS, y `app/fileops.rs` usa `.message()`. Con `String` el texto que ve el usuario en el aviso tiene que ser el mismo.
- **Vigilancia de carpetas (`Watch`)**: el guardián se suelta al cerrar o cambiar de pestaña; un `Box<dyn Watch>` que no se suelte deja hilos vivos → test que crea y suelta 50 guardianes y verifica que el falso cuenta 50 bajas.
- **Ayudante elevado de xtree en Windows** (`--xtree-mft`): sigue saliendo antes de instalar servicios; `DiskScan::elevate` lo lanza igual que hoy.

---

### Task 1: `DiskScan` y `Overlays`, y el escáner de xtree al núcleo

**Files:**
- Modify: `crates/xdir-core/src/sys/{mod.rs,testing.rs}` (traits y falsos)
- Move: `crates/xdir/src/xtree/scan.rs` → `crates/xdir-core/src/xtree/scan.rs` (deja de llamar `fs::xtree_*`: usa `sys::disk_scan()`); `crates/xdir/src/xtree/mod.rs` lo reexporta
- Modify: `crates/xdir/src/fs/{win32,macos}/xtree.rs` (las funciones `xtree_*` pasan a implementar el trait en `crates/xdir/src/sys/{win32,macos}.rs`), `crates/xdir/src/xtree/view.rs` (usa `sys::disk_scan()`), `crates/xdir/src/icons/mod.rs` (`request_overlays` → `sys::overlays().request(…)`), `OverlayRequest` → núcleo
- Tests: los de `xtree::scan` se mueven con el archivo; los de `fs::macos::xtree` y `mft` quedan en su plataforma

**Interfaces:**
- Produces (en `xdir_core::sys`):
  - `pub trait DiskScan: Send + Sync { fn lister(&self, root: &Path) -> Box<dyn Lister>; fn fast_scan(&self, root: &Path, p: &Progress, cancel: &AtomicBool) -> FastScan; fn elevate(&self, owner: Handle, root: &Path, p: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>>; fn drives(&self) -> Vec<DriveEntry>; fn volume_space(&self, path: &Path) -> Option<(u64, u64)>; fn whole_volume(&self, path: &Path) -> bool; fn default_root(&self) -> PathBuf; fn permission_issue(&self, p: &Progress) -> bool; fn permission_hint(&self) -> &'static str; fn elevating_text(&self) -> &'static str; }`
  - `pub trait Overlays: Send + Sync { fn request(&self, req: OverlayRequest); }` con `OverlayRequest { hwnd: Handle, tx: Sender<LoadEvent>, panel, generation, items }` en el núcleo (si `LoadEvent` todavía no está en el núcleo en esta tarea, `Overlays` llega en la Task 3 junto con los tipos de `fs`; decidir según compile y anotarlo).
  - `Services` suma `disk_scan` y `overlays`; accesores `sys::disk_scan()`, `sys::overlays()`.
  - Falso `FakeDiskScan` (lister en memoria configurable, `drives` vacío, `default_root` = carpeta temporal, sin permisos pendientes).

- [ ] **Step 1: Tests que fallan**: `scan_uses_registered_lister` (en el núcleo: con un `FakeDiskScan` cuyo lister sirve un árbol en memoria, `scan::start` termina con ese árbol); `view_default_root_comes_from_disk_scan` (en `xdir`: abrir xtree desde «Este equipo» con los falsos usa `default_root()` del falso y no escanea el disco real).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar, mover `scan.rs` y conectar.
- [ ] **Step 4:** Verificación de las Global Constraints. En la Mac, prueba de humo aislada (copia con `xdir.portable` + `XDIR_PRUEBAS`, nunca tocar `/Applications/xdir.app`): abrir xtree sobre una carpeta temporal y capturar.
- [ ] **Step 5: Commit** `refactor(core): escaneo de xtree y overlays detrás de DiskScan y Overlays`.

### Task 2: `Shell`

**Files:**
- Move: `Item`, `Entry`, `UNPIN_QUICK` de `crates/xdir/src/shell/mod.rs` → `crates/xdir-core/src/shell.rs`; la fachada los reexporta
- Modify: `crates/xdir-core/src/sys/{mod.rs,testing.rs}`, `crates/xdir/src/sys/{win32,macos}.rs` (envuelven `shell::win32::*` / `shell::macos::*`), `crates/xdir/src/shell/mod.rs` (las funciones públicas pasan a `sys::shell()`), `crates/xdir/src/app/fileops.rs` (si `.message()` deja de existir: el error ya es `String`)
- Lo que solo existe en Windows (`data_object`, `hdrop_paths`, `handle_menu_message`, `OWN_VERBS`, `ops::item`) queda interno de la plataforma; `reveal` de macOS también.

**Interfaces:**
- Produces: `pub trait Shell: Send + Sync` con `copy_files(&[Item], cut) -> Result<(), String>`, `clipboard_files() -> Option<(Vec<PathBuf>, bool)>`, `paste_virtual(Handle, &Location) -> Result<bool, String>`, `clear_clipboard()`, `copy_text(Handle, &str) -> Result<(), String>`, `pick_folder(Handle, &str) -> Option<PathBuf>`, `open_item(Handle, &Item)`, `open_file(Handle, &Path, Option<&Path>)`, `properties(Handle, &[Item])`, `context_menu(Handle, &Location, &[Item], (f32, f32), bool, bool, &[Entry], bool) -> Option<String>`, `invoke_verb(Handle, &Location, &[Item], &str) -> Result<(), String>`, `pin_quick_access(Handle, &Path) -> Result<(), String>`, `unpin_quick_access(Handle, &Path) -> Result<(), String>`, `set_menu_mode(Option<bool>)`, `volume_of(&Path) -> Option<OsString>`. Como `Location` es de `fs` y todavía no está en el núcleo, esta tarea **mueve `Location` al núcleo** (es neutro salvo `title`/`crumbs`, que pasan a llamar `sys::env()` / la plataforma vía un método nuevo de `SystemEnv`: `fn drive_label(&self, root: &str) -> String` y `fn root_title(&self, path: &Path) -> Option<String>`); si eso crece demasiado, mover `Location` en la Task 3 y que `Shell` use rutas, y anotar la decisión.
- Falso `FakeShell` que registra llamadas por hilo y responde configurable.

- [ ] **Step 1: Tests que fallan**: `copy_paths_uses_shell` (comando copiar rutas → `FakeShell` recibe el texto esperado); `context_menu_own_entries_reach_shell` (el menú contextual de un panel pasa las entradas propias); `location_titles_per_os` (con el entorno real: `C:\` → «Disco local (C:)» en Windows; `/` → nombre del volumen en macOS; `C:\Users` → «Users», migas iguales a hoy).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** Verificación + humo en Mac: copiar rutas al portapapeles y comparar (`pbpaste`), abrir el menú contextual nativo no se puede capturar: verificar que no falla con el gancho de pruebas.
- [ ] **Step 5: Commit** `refactor(core): shell del sistema detrás de Shell`.

### Task 3: `FileSystem` y `Watch`

**Files:**
- Move a `crates/xdir-core/src/fs.rs`: `WM_APP_LOAD`, `WM_APP_CHANGED`, `Entry`, `LoadMsg`, `LoadEvent`, `QuickItem`, `normalize_lexical`, `is_zip`, `is_unc`, `spawn_search` (sus ganchos pasan a `sys::file_system()`), y `Location` si no se movió en la Task 2
- Modify: `crates/xdir-core/src/sys/{mod.rs,testing.rs}`, `crates/xdir/src/sys/{win32,macos}.rs`, `crates/xdir/src/fs/mod.rs` (fachada: mismos nombres públicos de hoy, llamando a `sys::file_system()`)
- `Watcher` pasa a ser `Box<dyn Watch>` creado por `FileSystem::watch`; `tab.rs`, `app/panes.rs` y `app/mod.rs` guardan ese tipo (único cambio en usuarios permitido)

**Interfaces:**
- Produces: `pub trait FileSystem: Send + Sync` con `spawn_load(Handle, Sender<LoadEvent>, usize, u64, Arc<AtomicU64>, Location)`, ganchos de búsqueda (`with_search_thread(&self, f: Box<dyn FnOnce() + Send>)`, `search_metadata`, `is_hidden`, `searchable_dir`, `is_mount_point`, `disk_entry`), `user_folder(usize)`, `drives()`, `drive_name`, `drive_label`, `forget_drive_labels`, `drive_missing`, `subfolders(&Location)`, `folder_icon()`, `shell_location(&str)`, `network_place()`, `linux_place()`, `shell_place_exists`, `shell_subfolders`, `quick_access()`, `is_quick_pinned`, `quick_access_watch_dir`, `long_path`, `true_case`, `connect_share(Handle, &Path)`, `same_path`, `watch(Handle, u32, usize, isize, &Path) -> Option<Box<dyn Watch>>`. `pub trait Watch: Send {}` (solo `Drop`).
- Falso `FakeFileSystem` sobre `std::fs` (sirve carpetas temporales reales, sin íconos, sin unidades) y `FakeWatch` que cuenta altas y bajas.

- [ ] **Step 1: Tests que fallan**: `search_runs_in_core_with_fake_fs` (los tests de búsqueda de hoy, ahora en el núcleo con `FakeFileSystem`); `watchers_are_released` (50 guardianes creados y soltados → 50 bajas); `tab_loads_through_file_system` (una pestaña que navega a una carpeta temporal recibe sus entradas por el canal).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar y mover.
- [ ] **Step 4:** Verificación + humo en Mac (navegar, buscar en subcarpetas, cambiar un archivo y ver que la lista se actualiza sola, árbol lateral con unidades).
- [ ] **Step 5: Commit** `refactor(core): sistema de archivos detrás de FileSystem y Watch`.

### Task 4: Mover al núcleo los módulos que ya no dependen de fachadas

**Files:**
- Candidatos (en este orden; cada uno se mueve solo si después de las Tasks 1–3 sus dependencias ya están todas en el núcleo; si no, se deja y se anota qué le falta): `jump.rs`, `favorites.rs`, `groups.rs`, `session.rs` (la parte de `Session`/parseo; el resto si `list`/`theme` lo impiden se anota), `anim.rs` (si `render` solo aparece por tipos ya movidos).
- Reexports en `crates/xdir/src/main.rs` para que los usuarios no cambien.

- [ ] **Step 1:** Para cada candidato: `grep` de sus `crate::` y decidir; anotar en el informe la tabla «módulo → movido / qué le falta».
- [ ] **Step 2:** Mover los que se puedan, con sus tests.
- [ ] **Step 3:** Verificación de las Global Constraints; `cargo test -p xdir-core` en Linux por CI.
- [ ] **Step 4: Commit** `refactor(core): favoritos, grupos, saltos y sesión en el núcleo` (ajustar el mensaje a lo que se movió).

### Task 5: PRs encadenados (con confirmación del usuario)

- [ ] **Step 1:** Mostrar el resumen al usuario y **pedir confirmación** antes del push.
- [ ] **Step 2:** Push y PRs encadenados (el primero contra `develop`), cada uno con su lista de pruebas en Windows y el recordatorio de **borrar la rama al mergear** para que el siguiente pase solo a `develop`.
- [ ] **Step 3:** Esperar la CI y reportar; corregir lo que falle en la rama que corresponda.

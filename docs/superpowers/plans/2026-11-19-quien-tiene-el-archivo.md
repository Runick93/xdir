# Quién tiene tomado el archivo — Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cuando una acción falla porque otro proceso tiene el archivo, abrir una ventana emergente propia que nombre el proceso y su usuario.

**Architecture:** El núcleo define `Holder` y un método `CopyPrims::holders` con valor por defecto vacío; el motor de copia y las operaciones sueltas lo consultan solo con el fallo `InUse` ya definitivo. Windows lo implementa con el Restart Manager. Una segunda ventana auxiliar (`AuxWindow::InUse`), independiente de la de copia, la dibuja el núcleo (`in_use_draw.rs`).

**Tech Stack:** Rust (workspace, edición 2024), crate `windows` 0.62.2 (feature nuevo `Win32_System_RestartManager`), pruebas con los falsos de `xdir_core::sys::testing`.

**Spec:** `docs/superpowers/specs/2026-11-19-quien-tiene-el-archivo-design.md`

## Global Constraints

- `ErrKind::InUse` **no cambia** (sigue siendo variante sin datos).
- La consulta `holders` se hace **solo después del fallo definitivo** (tras los reintentos del motor) y nunca en el hilo de interfaz.
- Ventana: **560** de ancho (como la de copia), alto del contenido (`aux_resize`), botones **Reintentar** (principal) y **Cerrar**; Esc cierra, Enter reintenta.
- **Una ventana por evento**, no por archivo: una operación suelta abre la suya al fallar; un trabajo del motor abre **una sola al terminar**, agrupada por proceso. Otro fallo con la ventana abierta **reemplaza** su contenido.
- Sin holders (vacío) → no se abre la ventana; queda el aviso de hoy. Trabajo cancelado → no se abre.
- Límites de la lista: **4** programas y **5** archivos por programa; el resto, «y N más».
- Textos: «Lo está usando otro programa:» (uno) / «Lo están usando otros programas:» (varios); cierre «Cerralo y reintentá.» / «Cerralos y reintentá.»; usuario no leído → «usuario desconocido»; línea del proceso «PID 4812 · usuario PC\Leandro».
- Linux y macOS: sin implementar (valor por defecto vacío). El código común compila y prueba en todos los sistemas.
- Commits: mensaje en español, `tipo(ámbito): resumen`, terminado con la línea `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. **No** agregar `Cargo.toml` raíz ni los archivos sueltos de `docs/` (hay cambios ajenos sin commitear): `git add` solo de los archivos de cada tarea.
- Las pruebas de `xdir-windows` corren solo en Windows (`cargo test -p xdir-windows`).

## Review Focus

- El proceso termina entre el fallo y la consulta → `holders` vacío → sin ventana, aviso de hoy (Task 3 lo prueba en el motor, Task 7 en la `App`).
- Un trabajo con muchos bloqueados (200 archivos de 3 programas) → una sola ventana, con «y N más», sin pasarse del alto de la pantalla (Task 5 y 6).
- Proceso del sistema o de otra sesión del que no se puede leer el usuario → «usuario desconocido», sin error (Task 2 y 5).
- Un segundo fallo con la ventana abierta → reemplaza el contenido, no abre otra (Task 7).
- El que tiene el archivo es el propio xdir (vista previa, miniaturas) → se muestra como cualquier otro proceso (Task 2).

---

### Task 1: `Holder`, `CopyError.holders` y `CopyPrims::holders`

**Files:**
- Modify: `crates/xdir-core/src/sys/copy.rs` (tipos, `CopyError`, trait)
- Modify: `crates/xdir-core/src/sys/mod.rs` (reexportar `Holder`)
- Modify: `crates/xdir-core/src/sys/testing/copy_fake.rs` (falso)
- Test: `crates/xdir-core/src/sys/tests.rs`

**Interfaces:**
- Produces:
  - `pub struct Holder { pub name: String, pub exe: Option<String>, pub pid: u32, pub user: Option<String> }` (`Clone, PartialEq, Eq, Debug`), en `xdir_core::sys`.
  - `CopyError` gana `pub holders: Vec<Holder>`; `CopyError::new` lo deja vacío.
  - `CopyPrims::holders(&self, _path: &Path) -> Vec<Holder>` con valor por defecto `Vec::new()`.
  - `FakeCopyPrims::set_holders(&self, path: &Path, holders: Vec<Holder>)` y `FakeCopyPrims::holders_calls(&self, path: &Path) -> u32`; su `CopyPrims::holders` devuelve lo cargado y cuenta la llamada.

- [ ] **Step 1: Write the failing test** en `sys/tests.rs`: `copy_error_new_has_no_holders` (`CopyError::new(ErrKind::InUse, p).holders.is_empty()`), `default_prims_know_no_holders` (un `CopyPrims` que no lo implementa, p. ej. el falso sin `set_holders`, devuelve `[]`) y `fake_prims_return_loaded_holders_and_count_calls` (`set_holders(p, vec![h])` → `holders(p) == vec![h]` y `holders_calls(p) == 1`).
- [ ] **Step 2: Run** `cargo test -p xdir-core sys::tests::copy_error_new_has_no_holders` — Expected: FAIL (no compila: `Holder` no existe).
- [ ] **Step 3: Implement** los tipos y el método del trait en `sys/copy.rs`; el campo en `CopyError`; corregir los literales `CopyError { kind, path }` que rompan la compilación con `holders: Vec::new()`; el falso en `copy_fake.rs`.
- [ ] **Step 4: Run** `cargo test -p xdir-core sys::` y `cargo build --workspace` — Expected: PASS y compila (macOS/Linux también: no cambian de comportamiento).
- [ ] **Step 5: Commit** (`feat(errors): el núcleo sabe nombrar quién tiene un archivo`).

---

### Task 2: Windows: `holders` con el Restart Manager

**Files:**
- Modify: `crates/xdir-windows/Cargo.toml` (sumar `"Win32_System_RestartManager"` a los features de `windows`)
- Create: `crates/xdir-windows/src/ops/holders.rs`
- Modify: `crates/xdir-windows/src/ops/mod.rs` (`mod holders;`) y `ops/prims.rs` (`impl CopyPrims for WinCopyPrims { fn holders }`)
- Test: en `holders.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `xdir_core::sys::Holder`, `CopyPrims::holders` (Task 1).
- Produces: `pub(crate) fn holders_of(path: &Path) -> Vec<Holder>` y `fn user_of(pid: u32) -> Option<String>` (formato `DOMINIO\usuario`).

- [ ] **Step 1: Write the failing tests** en `holders.rs`:
  - `file_open_without_sharing_names_this_process`: abre un archivo temporal con `OpenOptions::new().read(true).share_mode(0)`; `holders_of(&path)` contiene un `Holder` con `pid == std::process::id()` y `user == Some(..)` que termina en el usuario actual (`USERNAME`).
  - `free_file_has_no_holders`: un archivo sin abrir → `[]`.
  - `missing_file_has_no_holders`: ruta que no existe → `[]`, sin pánico.
  - `unreadable_user_is_none`: `user_of(4)` (proceso System) → `None`, sin pánico.
- [ ] **Step 2: Run** `cargo test -p xdir-windows holders` — Expected: FAIL (el módulo no existe).
- [ ] **Step 3: Implement** `holders_of`: `RmStartSession` → `RmRegisterResources` con la ruta (UTF-16, sin `\\?\`) → `RmGetList` (dos pasadas por el tamaño; se descarta lo de `RmGetList` con error) → por cada `RM_PROCESS_INFO`: `name = strAppName`, `exe` por `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `QueryFullProcessImageNameW` (solo el nombre del archivo), `user = user_of(pid)`; `RmEndSession` **siempre** (guardia con `Drop`). `user_of`: `OpenProcess` → `OpenProcessToken(TOKEN_QUERY)` → `GetTokenInformation(TokenUser)` → `LookupAccountSidW`; cualquier fallo → `None`. En `prims.rs`, `holders` llama a `holders_of`.
- [ ] **Step 4: Run** `cargo test -p xdir-windows holders` — Expected: PASS (4 pruebas).
- [ ] **Step 5: Commit** (`feat(windows): averiguar quién tiene un archivo con el Restart Manager`).

---

### Task 3: El motor de copia guarda los holders del fallo `InUse`

**Files:**
- Modify: `crates/xdir-core/src/transfer/mod.rs` (`JobError.holders`)
- Modify: `crates/xdir-core/src/transfer/engine.rs` (`fail`, literal de `JobError` en la línea ~1962)
- Modify: `crates/xdir-core/src/transfer/view_tests.rs` (literales de `JobError`)
- Test: `crates/xdir-core/src/transfer/engine_tests.rs`

**Interfaces:**
- Consumes: `CopyPrims::holders`, `FakeCopyPrims::set_holders/holders_calls` (Task 1).
- Produces: `JobError.holders: Vec<Holder>` (vacío salvo `InUse` con holders); `JobOutcome.errors[i].holders`.

- [ ] **Step 1: Write the failing tests** en `engine_tests.rs`, con el estilo de las pruebas que ya usan `disk.fail(path, ErrKind::InUse, n)`:
  - `in_use_error_carries_holders`: `disk.fail(a, InUse, u32::MAX)` + `set_holders(a, [excel])` → el `JobOutcome` trae un error con `holders == [excel]`.
  - `in_use_that_clears_up_is_not_queried`: `disk.fail(a, InUse, 1)` (falla una vez y se arregla durante los reintentos del renombrado final) → sin errores y `holders_calls(a) == 0`.
  - `other_errors_carry_no_holders`: `Denied` con `set_holders` cargado → `holders` vacío y `holders_calls == 0`.
  - `holders_gone_leave_the_error_plain`: `InUse` sin `set_holders` → error con `holders` vacío (el proceso terminó entre el fallo y la consulta).
- [ ] **Step 2: Run** `cargo test -p xdir-core transfer::engine_tests::in_use` — Expected: FAIL (no compila: falta `JobError.holders`).
- [ ] **Step 3: Implement** el campo (`Default` vacío en los tres literales del motor y en los de `view_tests.rs`); en `Engine::fail(&mut self, src, dst, mut e: CopyError)`: si `e.kind == ErrKind::InUse` y `e.holders.is_empty()`, `e.holders = self.prims.holders(&e.path)`, y ambos `JobError` que arma salen con `holders: e.holders.clone()`.
- [ ] **Step 4: Run** `cargo test -p xdir-core transfer::` — Expected: PASS (las existentes siguen).
- [ ] **Step 5: Commit** (`feat(copy): el motor guarda quién tiene los archivos en uso`).

---

### Task 4: Las operaciones sueltas devuelven los holders (`OpResult.in_use`)

**Files:**
- Create: `crates/xdir-core/src/in_use.rs` (por ahora solo los tipos) y `pub mod in_use;` en `lib.rs`
- Modify: `crates/xdir-core/src/ops.rs` (`OpResult.in_use`), `crates/xdir-core/src/transfer/engine.rs:~422` (literal de `OpResult`)
- Modify: `crates/xdir-core/src/sys/testing.rs` (`FakeFileOps::fail_in_use`)
- Modify: `crates/xdir-windows/src/ops/win32.rs` (`op_error` y `spawn`; literal de `OpResult` ~166)
- Test: `crates/xdir-core/src/ops.rs` (módulo `tests`) y, en Windows, `crates/xdir-windows/src/ops/win32.rs`

**Interfaces:**
- Consumes: `Holder`, `CopyPrims::holders` (Tasks 1–2).
- Produces:
  - `pub struct InUseItem { pub path: PathBuf, pub holders: Vec<Holder> }` y `pub struct InUseFailure { pub op: Op, pub items: Vec<InUseItem> }` (`Clone, Debug, PartialEq`), en `xdir_core::in_use`.
  - `OpResult.in_use: Option<InUseFailure>` (por defecto `None`).
  - `FakeFileOps::fail_in_use(holders: Vec<Holder>)`: las próximas operaciones de ese hilo fallan `InUse` con el aviso de siempre **y** `in_use: Some(..)` con el elemento de `op_path`.

- [ ] **Step 1: Write the failing tests:** en `ops.rs`, `fake_ops_in_use_failure_carries_the_op_and_holders` (con `FakeFileOps::fail_in_use([excel])` y un `Op::Rename`, el `OpResult` trae el aviso de hoy y `in_use == Some(InUseFailure { op, items: [InUseItem { path, holders: [excel] }] })`); en Windows, `rename_of_a_held_file_reports_the_holder` (archivo abierto con `share_mode(0)` desde la prueba; `Op::Rename` real por `spawn`; el `OpResult.in_use` nombra el PID propio).
- [ ] **Step 2: Run** `cargo test -p xdir-core ops::tests::fake_ops_in_use` — Expected: FAIL (no compila).
- [ ] **Step 3: Implement** los tipos y el campo; `FakeFileOps::fail_in_use`; en Windows, `op_error` devuelve además `Option<InUseFailure>`: si `failed.kind == ErrKind::InUse`, consulta `WinCopyPrims.holders(&failed.path)` y, si no está vacío, arma la falla con `op.clone()`; `spawn` lo pone en el `OpResult`. Los literales completos de `OpResult` suman `in_use: None`.
- [ ] **Step 4: Run** `cargo test -p xdir-core ops::` y `cargo test -p xdir-windows rename_of_a_held_file` — Expected: PASS.
- [ ] **Step 5: Commit** (`feat(ops): las operaciones sueltas informan quién tiene el archivo`).

---

### Task 5: El modelo de la ventana (agrupar y redactar)

**Files:**
- Modify: `crates/xdir-core/src/in_use.rs`
- Test: `crates/xdir-core/src/in_use.rs` (módulo `tests`)

**Interfaces:**
- Consumes: `InUseItem`, `InUseFailure` (Task 4), `JobError.holders` (Task 3), `ops::doing`.
- Produces (en `xdir_core::in_use`):
  - `pub enum InUseRetry { Op(Op), Job(JobId) }`
  - `pub struct InUseDialog { pub title: String, pub groups: Vec<HolderGroup>, pub more_groups: usize, pub retry: InUseRetry }` y `pub struct HolderGroup { pub holder: Holder, pub files: Vec<String>, pub more_files: usize }`
  - `InUseDialog::from_op(f: &InUseFailure) -> InUseDialog` — título «No se pudo {`ops::doing(op, path)`}» (con la inicial en mayúscula ya dada por `doing`: «No se pudo eliminar «informe.xlsx»»).
  - `InUseDialog::from_job(id: JobId, errors: &[JobError]) -> Option<InUseDialog>` — `None` si ningún error tiene holders; título ««a.xlsx» está en uso» (uno) o «N elementos están en uso».
  - `fn group(items: &[InUseItem]) -> Vec<HolderGroup>` (agrupa por `pid`, respeta los límites 4 y 5).
  - `pub fn intro(&self) -> &'static str`, `pub fn outro(&self) -> &'static str`, `pub fn holder_label(h: &Holder) -> String` («Microsoft Excel (EXCEL.EXE)»; solo `name` si no hay `exe` o es igual), `pub fn holder_line(h: &Holder) -> String` («PID 4812 · usuario PC\Leandro» / «PID 4812 · usuario desconocido»).

- [ ] **Step 1: Write the failing tests:** `group_merges_files_of_the_same_process`, `group_caps_programs_and_files` (7 procesos y 9 archivos en uno → `groups.len() == 4`, `more_groups == 3`, `files.len() == 5`, `more_files == 4`), `label_shows_exe_only_when_it_adds_something`, `line_says_unknown_user`, `from_op_title_names_the_action` (`Op::Delete` de `/u/informe.xlsx` → «No se pudo eliminar «informe.xlsx»»), `from_job_none_without_holders`, `from_job_titles_one_and_many`, `intro_outro_singular_and_plural` con los textos de Global Constraints. Cubre además el Review Focus de los 200 archivos: 200 `InUseItem` de 3 procesos → 3 grupos, cada uno con 5 archivos y su `more_files`.
- [ ] **Step 2: Run** `cargo test -p xdir-core in_use::` — Expected: FAIL.
- [ ] **Step 3: Implement** lo anterior (puro, sin pintar ni tocar la `App`).
- [ ] **Step 4: Run** `cargo test -p xdir-core in_use::` — Expected: PASS.
- [ ] **Step 5: Commit** (`feat(in-use): modelo de la ventana de archivo en uso`).

---

### Task 6: El dibujo de la ventana

**Files:**
- Create: `crates/xdir-core/src/in_use_draw.rs` y `pub mod in_use_draw;` en `lib.rs`
- Test: `crates/xdir-core/src/in_use_draw.rs` (módulo `tests`), con el mismo estilo que `transfer/view_tests.rs`

**Interfaces:**
- Consumes: `InUseDialog` y los textos de Task 5; el `Painter` y los estilos que usa `transfer::view::draw` (margen 16, botones 84×26, título y texto secundario del tema).
- Produces:
  - `pub const WIDTH: f32 = 560.0;`
  - `pub fn height(d: &InUseDialog) -> f32`
  - `pub enum InUseHit { Retry, Close }` (`Clone, Copy, PartialEq, Debug`)
  - `pub fn draw(p: &dyn Painter, r: Rect, d: &InUseDialog, hover: Option<InUseHit>, pressed: Option<InUseHit>)`
  - `pub fn hit(r: Rect, d: &InUseDialog, x: f32, y: f32) -> Option<InUseHit>`

- [ ] **Step 1: Write the failing tests:** `height_grows_with_groups_and_files` (un grupo < tres grupos con 5 archivos), `height_is_capped_by_the_limits` (200 archivos no cambia el alto máximo), `hit_finds_both_buttons_and_nothing_elsewhere`, `draw_shows_title_intro_process_and_buttons` (el texto pintado contiene el título, «Lo está usando otro programa:», «Microsoft Excel (EXCEL.EXE)», «PID 4812 · usuario PC\Leandro», «Cerralo y reintentá.», «Reintentar» y «Cerrar»; un `Painter` de prueba que acumule textos como el de `transfers_tests.rs::aux_texts`).
- [ ] **Step 2: Run** `cargo test -p xdir-core in_use_draw::` — Expected: FAIL.
- [ ] **Step 3: Implement** `draw`, `height` y `hit` con la maqueta de la spec: encabezado, intro, por cada grupo (etiqueta en negrita, línea del proceso en texto secundario, archivos con sangría, «y N más»), «y N programas más», cierre y botones alineados a la derecha; **Reintentar** es el principal.
- [ ] **Step 4: Run** `cargo test -p xdir-core in_use_draw::` — Expected: PASS.
- [ ] **Step 5: Commit** (`feat(in-use): dibujo de la ventana de archivo en uso`).

---

### Task 7: La `App`: abrir, atender y reintentar

**Files:**
- Modify: `crates/xdir-core/src/host.rs` (`AuxWindow::InUse`)
- Create: `crates/xdir-core/src/app/in_use.rs` (+ `mod in_use;` en `app/mod.rs`, campo `in_use: Option<InUseState>`)
- Modify: `crates/xdir-core/src/app/transfers.rs` (`paint_aux`, `aux_mouse`, `aux_key`, `aux_closed`: los `let AuxWindow::Transfers = w;` pasan a `match`; `on_job_done`), `crates/xdir-core/src/app/fileops.rs` (`on_op_done`)
- Modify: `crates/xdir-core/src/sys/testing.rs` (el falso de host registra por ventana: `aux_shows` ya existe; que distinga `AuxWindow`)
- Test: `crates/xdir-core/src/app/in_use_tests.rs` (+ `#[cfg(test)] mod in_use_tests;`)

**Interfaces:**
- Consumes: `InUseDialog`, `InUseRetry` (Task 5), `in_use_draw::{WIDTH, height, draw, hit, InUseHit}` (Task 6), `OpResult.in_use` (Task 4), `JobError.holders` (Task 3), `App::retry_job(id) -> Option<JobId>`, `sys::file_ops().spawn(hwnd, ops_tx, op)`.
- Produces: `AuxWindow::InUse`; `App::open_in_use(&mut self, d: InUseDialog)`; el despacho de las cuatro funciones públicas de ventana auxiliar por `AuxWindow`.

- [ ] **Step 1: Write the failing tests** en `in_use_tests.rs` (App de prueba con `FakeFileOps`, `FakeCopyPrims` y el host falso):
  - `failed_rename_in_use_opens_the_window`: `FakeFileOps::fail_in_use([excel])` + renombrar → `aux_shows()` registra `InUse` con el título «No se pudo renombrar «a.txt» como «b.txt»», y el aviso de la barra de estado sigue puesto.
  - `in_use_without_holders_opens_nothing`: `FakeFileOps::fail_with(ErrKind::InUse)` → sin `AuxWindow::InUse`.
  - `delete_job_with_locked_files_opens_one_window_at_the_end`: borrar 3 archivos bloqueados (`fail` + `set_holders`) → una sola apertura de `InUse`, después de `Finished`, con 3 elementos.
  - `cancelled_job_opens_nothing`.
  - `second_failure_replaces_the_window`: dos fallos seguidos → una ventana, con el contenido del segundo (`aux_resize` en vez de otro `aux_show` si ya estaba visible).
  - `retry_button_repeats_the_op_and_closes`: clic en Reintentar → `FakeFileOps::take_ops()` trae la misma `Op` otra vez y la ventana se oculta; con un trabajo, `retry_job` lanza el reintento.
  - `close_button_and_escape_hide_it`: Cerrar y `VK_ESCAPE` → `aux_hides()` sube y no hay reintento; Enter → Reintentar.
  - `copy_window_unchanged`: un trabajo con `InUse` sigue mostrando «Lo está usando otro programa» en la fila de la ventana de copia (la prueba existente de errores no cambia; se agrega una que además verifica que **la de copia** y la de «en uso» no se pisan: `aux_shows` tiene las dos).
- [ ] **Step 2: Run** `cargo test -p xdir-core app::in_use_tests` — Expected: FAIL.
- [ ] **Step 3: Implement:** la variante; `open_in_use` (reemplaza el estado, `aux_show` si no estaba visible o `aux_resize` + `aux_invalidate` si sí; si el host devuelve `false`, no hace nada); `on_op_done` abre con `InUseDialog::from_op` si `result.in_use` es `Some`; `on_job_done` abre con `from_job` cuando no `cancelled` y devuelve `Some`; `paint_aux`/`aux_mouse`/`aux_key`/`aux_closed` despachan por `w` a funciones de `app/in_use.rs` (hover, presionado, clic, Esc, Enter); el reintento: `InUseRetry::Op(op)` → `sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), op)`, `InUseRetry::Job(id)` → `self.retry_job(id)`; después oculta la ventana.
- [ ] **Step 4: Run** `cargo test -p xdir-core` — Expected: PASS entero (el núcleo, con las pruebas de la ventana de copia sin cambios).
- [ ] **Step 5: Commit** (`feat(in-use): la ventana de archivo en uso se abre sola y reintenta`).

---

### Task 8: Windows: dos ventanas auxiliares

**Files:**
- Modify: `crates/xdir-windows/src/auxwin.rs` (estado por ventana), `crates/xdir-windows/src/app/host_win.rs:~739` (los `aux_*` ya reenvían `w`: sin cambios de firma)
- Test: `crates/xdir-windows/src/auxwin.rs` (módulo `tests`)

**Interfaces:**
- Consumes: `AuxWindow::{Transfers, InUse}` (Task 7).
- Produces: `fn slot(w: AuxWindow) -> usize` (0 y 1); `AUX`, `RENDERER` y `PLACE` indexados por `slot`; `with(w, f)`, `with_renderer(w, f)`, `kind_of(hwnd) -> Option<AuxWindow>`; `ensure(main, w, theme)`. La barra de tareas y el avance (`set_progress`, `TaskbarButtonCreated`) siguen siendo solo de `Transfers`.

- [ ] **Step 1: Write the failing tests:** `slots_are_distinct_and_in_range` (`slot(Transfers) != slot(InUse)` y ambos `< COUNT`), `progress_only_applies_to_transfers` (la función pura que decide qué ventana lleva avance devuelve `true` solo para `Transfers`).
- [ ] **Step 2: Run** `cargo test -p xdir-windows auxwin::tests::slots` — Expected: FAIL.
- [ ] **Step 3: Implement:** `Aux` guarda su `kind`; `AUX`/`RENDERER` pasan a arreglos de dos casillas; `show/resize/set_title/invalidate_aux/hide/set_progress` reciben `w` y eligen casilla (se quitan los `let AuxWindow::Transfers = w;`); `aux_proc` y los auxiliares (`paint`, `mouse`, `wheel`) obtienen el tipo con `kind_of(hwnd)` y se lo pasan a `app.paint_aux/aux_mouse/aux_key/aux_closed`; `shutdown` suelta las dos; `WM_CLOSE` de la de «en uso» no toca el avance. Misma clase `xdir.aux`, mismo estilo, mismo centrado la primera vez; el lugar recordado es por ventana.
- [ ] **Step 4: Run** `cargo test -p xdir-windows` y `cargo build -p xdir-windows` — Expected: PASS y compila.
- [ ] **Step 5: Commit** (`feat(windows): una segunda ventana auxiliar para archivos en uso`).

---

### Task 9: Documentación y verificación entera

**Files:**
- Modify: `docs/DISENO.md` (fila nueva `X98`, «Ventana: archivo en uso», con la maqueta y `in_use_draw::draw`), `CHANGELOG.md` (en «Sin publicar ▸ Nuevo»)

- [ ] **Step 1: Documentar:** la fila X98 con el mismo formato que X45–X47 (ancho 560, margen 16, botones 84×26, qué hace cada control) y una línea de CHANGELOG: «Cuando un archivo no se puede borrar, mover, renombrar o copiar porque otro programa lo tiene, una ventana dice qué programa es y qué usuario lo ejecutó (en Windows)».
- [ ] **Step 2: Verificar el workspace:** `cargo test --locked` y `cargo build --locked --features pruebas -p xdir-windows` — Expected: todo en verde.
- [ ] **Step 3: Verificación manual en Windows** (anotar el resultado): desde PowerShell, `$f=[IO.File]::Open("$env:TEMP\a.txt",'OpenOrCreate','ReadWrite','None')`; en xdir, borrar, renombrar y mover `a.txt`: se abre la ventana con `powershell.exe`, el PID y tu usuario; copiar encima de él con un trabajo de varios archivos abre **una** ventana; `$f.Close()` y **Reintentar** lo completa; con un archivo en uso por un servicio del sistema, el usuario aparece o dice «usuario desconocido».
- [ ] **Step 4: Commit** (`docs(in-use): diseño de pantalla y novedades`).

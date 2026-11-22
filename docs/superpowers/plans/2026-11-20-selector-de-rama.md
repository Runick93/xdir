# Selector de rama de git Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un botón en el pie de cada panel con la rama actual de git, que abre un menú para cambiarla y, si git se niega por cambios sin commitear, pregunta si guardarlos en el stash o descartarlos.

**Architecture:** Un módulo `git` neutro en `xdir-core` (solo `std`): lee `.git/HEAD` en el propio programa y lanza el `git` de la consola solo para listar, cambiar y hacer stash. Cada `Tab` guarda su `RepoHead`; `Pane` dibuja el botón y resuelve el clic; `App` abre el menú nativo y corre los comandos de git con `spawn_task`/`TaskDone`.

**Tech Stack:** Rust edición 2024, `std::process::Command`; sin dependencias nuevas.

**Spec:** [docs/superpowers/specs/2026-11-20-selector-de-rama-design.md](../specs/2026-11-20-selector-de-rama-design.md)

## Global Constraints

- Sin dependencias nuevas (ni `git2` ni `gix`); solo `std`.
- Todo `git` se lanza con `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, `stdin` nulo, y en Windows `CREATE_NO_WINDOW` (`std::os::windows::process::CommandExt`, bajo `cfg(windows)`).
- Ningún proceso `git` corre en el hilo de la ventana: se usa `App::spawn_task` / `TaskDone`. `git::available()` y `read_head` sí corren ahí (no lanzan procesos).
- El botón solo existe si: `git::available()`, la pestaña es `Location::Path` y la carpeta o un padre es repositorio.
- Menú: máximo 40 ramas, las más recientes primero, entrada final deshabilitada «…y N más»; la rama actual va marcada y deshabilitada.
- Mensaje del stash, exacto: `xdir: antes de pasar a <rama>`. `stash push -u`. Nunca `stash pop` por cuenta propia.
- Descartar usa `git switch --discard-changes` y no toca los archivos sin seguimiento.
- Textos, comentarios y mensajes de commit en español, como el resto del proyecto (commits `feat(git): …`, `test(git): …`).
- `Cargo.toml` de la raíz tiene un cambio sin commitear que no es de esta función: **nunca `git add -A` ni `git add .`**; agregar solo los archivos de cada tarea.
- Los textos del cartel (Task 6) son exactos, copiados del spec.

## Review Focus

- Repositorio recién creado sin commits (HEAD apunta a una rama sin nacer): se muestra su nombre y el menú dice «(sin ramas)» en vez de abrir vacío. Test en Task 1 (`read_head`) y Task 5 (menú).
- Ruta con espacios y caracteres no ASCII (`C:\Users\Leandro-Dev\…`, `carpeta con ñ`): detección y cambio de rama funcionan. Test en Task 2.
- HEAD suelto: el botón muestra el hash corto y el menú no marca ninguna rama como actual. Test en Task 1 y Task 5.
- `.git` como archivo con ruta relativa (worktree): se detecta igual. Test en Task 1.
- El panel o la pestaña desaparece (o cambia de carpeta) mientras `git` corre: el resultado se aplica por **raíz del repo**, nunca por índice de panel viejo, y no hace panic. Test en Task 5.

---

## File Structure

- Create `crates/xdir-core/src/git.rs`: toda la lógica de git (detección, lectura de HEAD, ramas, cambio, stash, descartar). Con sus pruebas dentro (`#[cfg(test)] mod tests`).
- Create `crates/xdir-core/src/app/git_branch.rs`: clic del botón, menú, `TaskDone` de git y el cartel.
- Modify `crates/xdir-core/src/lib.rs`: `pub mod git;`.
- Modify `crates/xdir-core/src/tab.rs`: campo `git: Option<RepoHead>`.
- Modify `crates/xdir-core/src/pane.rs`: `branch_button`, `PaneHit::BranchButton`, dibujo y hit-test.
- Modify `crates/xdir-core/src/app/tooltip.rs`, `app/input.rs`, `app/tasks.rs`, `app/mod.rs`: tooltip, clic, `TaskDone`, `git_busy`.
- Modify `crates/xdir-core/src/sys/testing.rs` (`FakeHost`): guardar la posición de `popup_menu` y, si hace falta, una cola de respuestas (`set_answers`) para las pruebas de los Task 5 y 6.
- Modify `CHANGELOG.md`.

---

### Task 1: `git` — detección y lectura de la rama (sin procesos)

**Files:**
- Create: `crates/xdir-core/src/git.rs`
- Modify: `crates/xdir-core/src/lib.rs` (junto a `pub mod geom;`)

**Interfaces:**
- Produces:
  - `pub enum Head { Branch(String), Detached(String) }` con `pub fn label(&self) -> &str` (el nombre, o los 7 primeros caracteres del hash) y `#[derive(Clone, PartialEq, Eq, Debug)]`.
  - `pub struct RepoHead { pub root: PathBuf, pub head: Head }` (mismos derives).
  - `pub fn read_head(dir: &Path) -> Option<RepoHead>`
  - `#[cfg(test)] pub(crate) fn temp_repo(name: &str) -> Option<PathBuf>`: crea un repo en la carpeta temporal con un commit en `main` y una rama `feature`, y devuelve `None` si `git` no está instalado (el test que la usa hace `return` con un `eprintln!`). Se escribe en Task 2; en esta tarea las pruebas arman las carpetas a mano.

- [ ] **Step 1: Escribir las pruebas que fallan** en `git.rs` (`mod tests`), cada una arma una carpeta temporal con archivos a mano:
  - `read_head_branch`: `.git/HEAD` con `ref: refs/heads/feature/x\n` → `Head::Branch("feature/x")`, `root` = la carpeta.
  - `read_head_detached`: `HEAD` con 40 caracteres hexadecimales → `Head::Detached` con `label()` de 7 caracteres.
  - `read_head_from_subfolder`: llamarla con `dir = <repo>/a/b` devuelve el `root` del repo.
  - `read_head_gitfile_relative`: `.git` es un archivo con `gitdir: ../real/.git/worktrees/w`, y esa ruta tiene su `HEAD`.
  - `read_head_gitfile_absolute`: igual con ruta absoluta.
  - `read_head_unborn_repo`: `HEAD` con `ref: refs/heads/main` y sin ninguna ref → `Branch("main")`.
  - `read_head_none`: carpeta sin `.git` ni padres con `.git` → `None`; `HEAD` ilegible o vacío → `None`.
  - `read_head_path_with_spaces_and_unicode`: repo en `carpeta con ñ`.

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test -p xdir-core --lib git::`
Expected: error de compilación (`read_head` no existe).

- [ ] **Step 3: Implementar `read_head` y los tipos.** Sube por `dir.ancestors()`; en cada nivel mira `.git`: carpeta → su `HEAD`; archivo → primera línea `gitdir: <ruta>` (relativa a la carpeta que contiene el `.git`) y su `HEAD`. `ref: refs/heads/<n>` → `Branch`; cualquier otra línea hexadecimal de 40 (o 64) caracteres → `Detached`; el resto → `None`. Registrar `pub mod git;` en `lib.rs`.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test -p xdir-core --lib git::`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/xdir-core/src/git.rs crates/xdir-core/src/lib.rs
git commit -m "feat(git): leer la rama actual desde .git/HEAD"
```

---

### Task 2: `git` — disponibilidad y comandos (ramas, cambiar, stash, descartar)

**Files:**
- Modify: `crates/xdir-core/src/git.rs`

**Interfaces:**
- Consumes: `Head`, `RepoHead` de Task 1.
- Produces:
  - `pub fn available() -> bool` (`OnceLock<bool>` con `git --version`; en `cfg(test)` primero mira un `thread_local` fijado por `pub fn set_available_for_tests(v: Option<bool>)`).
  - `pub fn warm_up()`: lanza un hilo que llama a `available()` (se usa en Task 5; no se llama bajo `cfg(test)`).
  - `pub fn branches(root: &Path) -> Result<Vec<String>, String>`
  - `pub enum SwitchOutcome { Switched, Dirty { files: usize }, Failed(String) }` (`Clone, PartialEq, Debug`)
  - `pub fn switch(root: &Path, name: &str) -> SwitchOutcome`
  - `pub fn stash_and_switch(root: &Path, name: &str) -> SwitchOutcome`
  - `pub fn discard_and_switch(root: &Path, name: &str) -> SwitchOutcome`
  - `pub fn dirty_count(root: &Path) -> usize`
  - `pub fn stash_message(dest: &str) -> String` → `xdir: antes de pasar a <dest>`
  - `#[cfg(test)] pub(crate) fn temp_repo(name: &str) -> Option<PathBuf>` (ver Task 1).

- [ ] **Step 1: Escribir las pruebas que fallan** (cada una usa `temp_repo` y sale con `eprintln!` si devuelve `None`):
  - `branches_lists_recent_first`: tras un commit nuevo en `feature`, `branches` devuelve `["feature", "main"]`.
  - `branches_empty_repo`: repo con `git init` y sin commits → `Ok(vec![])`.
  - `switch_clean`: `switch(root, "feature")` → `Switched` y `read_head` da `Branch("feature")`.
  - `switch_branch_with_slash`: rama `feat/x` se puede elegir.
  - `switch_missing_branch`: `Failed(t)` con `t` no vacío.
  - `switch_dirty_conflict`: `a.txt` distinto en `main` y `feature`, modificado sin commitear en `main` → `Dirty { files: 1 }`, y el archivo sigue modificado.
  - `switch_dirty_without_conflict_carries_changes`: el cambio sin commitear no choca (archivo distinto) → `Switched` y el cambio sigue ahí.
  - `stash_and_switch_saves_changes`: en el caso de conflicto, `stash_and_switch` → `Switched`, `git stash list` contiene `xdir: antes de pasar a feature`, y un archivo nuevo sin seguimiento también quedó en el stash.
  - `discard_and_switch_keeps_untracked`: `discard_and_switch` → `Switched`, el cambio a `a.txt` se perdió, un archivo nuevo sin seguimiento sigue en disco.
  - `dirty_count_ignores_untracked`: 2 modificados + 1 nuevo sin seguimiento → `2`.
  - `works_in_path_with_spaces_and_unicode`: `temp_repo("carpeta con ñ")` y un `switch` limpio.
  - `available_override`: `set_available_for_tests(Some(false))` → `available()` es `false`; `None` restaura.

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test -p xdir-core --lib git::`
Expected: error de compilación (las funciones no existen).

- [ ] **Step 3: Implementar** un helper privado `fn run(root: &Path, args: &[&str]) -> Result<String, String>` que aplica las Global Constraints y devuelve stdout, o el stderr recortado si el código de salida no es 0. `branches` usa `for-each-ref --sort=-committerdate --format=%(refname:short) refs/heads`. `switch` usa `git switch <name>`; si falla y el stderr contiene `would be overwritten` o `commit your changes or stash them` → `Dirty { files: dirty_count(root) }`, si no `Failed(stderr)`. `dirty_count` cuenta líneas de `status --porcelain -uno`. `stash_and_switch`: `stash push -u -m <stash_message>`, luego `switch`; si el `switch` falla después del stash, `Failed("<texto de git> Tus cambios quedan en el stash (<mensaje>).")`. `discard_and_switch`: `switch --discard-changes <name>`. `temp_repo`: carpeta única en `std::env::temp_dir()`, `git init -b main`, usuario con `-c user.name=xdir -c user.email=xdir@example.com`, un commit con `a.txt`, rama `feature` con otro commit.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test -p xdir-core --lib git::`
Expected: PASS (si `git` no está instalado, los tests con repo se saltan y pasan).

- [ ] **Step 5: Commit**

```bash
git add crates/xdir-core/src/git.rs
git commit -m "feat(git): listar ramas, cambiar, stash y descartar con el git de la consola"
```

---

### Task 3: El estado de git en la pestaña

**Files:**
- Modify: `crates/xdir-core/src/tab.rs` (struct `Tab` ~línea 33, su constructor, y el brazo `LoadMsg::Listed` ~línea 679)
- Test: `crates/xdir-core/src/app/tests.rs`

**Interfaces:**
- Consumes: `git::read_head`, `git::available`, `git::RepoHead`.
- Produces: `pub git: Option<crate::git::RepoHead>` en `Tab`.

- [ ] **Step 1: Escribir las pruebas que fallan** en `app/tests.rs`, con `app_with`, una carpeta de `git::temp_repo` y `app::network_tests::wait_until` (`pub(super)`) para esperar a que termine la carga de la pestaña:
  - `tab_has_branch_inside_repo`: tras navegar la pestaña activa a la carpeta, `tab().git` es `Some` con `head == Head::Branch("main")`.
  - `tab_without_git_outside_repo`: carpeta temporal sin repo → `None`.
  - `tab_without_git_when_git_missing`: con `set_available_for_tests(Some(false))` dentro del repo → `None`.
  - `tab_without_git_on_unc_path`: una `Location::Path` que empieza con `\\` no consulta nada (se comprueba que `None` aunque `available` sea verdadero; se puede probar la función auxiliar directamente).
  - `tab_branch_refreshes_on_reload`: tras un `git switch` hecho por fuera y una recarga de la pestaña, `tab().git` refleja la rama nueva.

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test -p xdir-core --lib tab_has_branch tab_without_git tab_branch_refreshes`
Expected: error de compilación (`Tab::git` no existe).

- [ ] **Step 3: Implementar** el campo (`None` en el constructor) y, en `LoadMsg::Listed(result)` cuando el resultado es `Ok`, asignar `self.git` con una función `fn repo_head_of(loc: &Location) -> Option<RepoHead>`: solo `Location::Path(p)` con `git::available()` y que `p` no empiece con `\\`; si no, `None`. Si el listado falla, `self.git` no cambia.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test -p xdir-core --lib tab_has_branch tab_without_git tab_branch_refreshes`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/xdir-core/src/tab.rs crates/xdir-core/src/app/tests.rs
git commit -m "feat(git): cada pestaña recuerda en qué rama está"
```

---

### Task 4: El botón en el pie del panel

**Files:**
- Modify: `crates/xdir-core/src/pane.rs` (struct ~205, `new` ~255, `hit` ~485, `draw` ~717, enum `PaneHit` ~132)
- Modify: `crates/xdir-core/src/app/tooltip.rs` (~línea 80)
- Test: `crates/xdir-core/src/app/tests.rs` (junto a la prueba de los botones del pie, ~1225)

**Interfaces:**
- Consumes: `Tab::git` de Task 3.
- Produces: `PaneHit::BranchButton`; en `Pane`, `branch_button: Rect` (vacío si no hay botón) y `#[cfg(test)] pub fn branch_button_rect(&self) -> Rect`. Constante `BRANCH_BTN_MAX_W: f32 = 180.0`.

- [ ] **Step 1: Escribir las pruebas que fallan** en `app/tests.rs`:
  - `branch_button_drawn_left_of_new_folder`: con una pestaña con `git = Some(RepoHead{ Branch("main") })` asignada a mano y un cuadro dibujado, `branch_button_rect()` tiene `right <= new_folder.left`, la misma altura que `vb` y `host.drawn_texts()` contiene un texto con «main» dentro de ese rectángulo; `a.hit(center)` es `Hit::Pane(0, PaneHit::BranchButton)`.
  - `branch_button_absent_without_git`: sin `git`, `branch_button_rect()` es vacío y el centro de ese lugar no da `BranchButton`.
  - `branch_button_truncates_long_names`: nombre de 80 caracteres → ancho del rectángulo `<= BRANCH_BTN_MAX_W` y el texto dibujado termina en `…`.
  - `branch_button_shows_short_hash_when_detached`: `Detached("abc1234…")` muestra los 7 caracteres.
  - `branch_button_absent_on_pages`: en la pestaña de Configuración queda vacío.
  - `branch_tooltip`: `Hit::Pane(0, BranchButton)` da `Rama: main (clic para cambiar)`; HEAD suelto da `HEAD suelto en abc1234`.

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test -p xdir-core --lib branch_button branch_tooltip`
Expected: error de compilación (`PaneHit::BranchButton` no existe).

- [ ] **Step 3: Implementar** en `Pane::draw`, tras calcular `nb`: si `tab.git` es `Some`, rectángulo a la izquierda de `nb` (separación 6, mismo `vb_top` y `FOOTER_BTN_H`), ancho = texto medido con el mismo medidor que usa `pane.rs` para los títulos de pestañas, con tope `BRANCH_BTN_MAX_W`, nombre cortado con `…`; mismo estilo de hover/pulsado que `nb`; delante del nombre va un glifo de rama: el que ofrezca `FontId::Glyphs` (si Segoe no tiene uno, el texto «⎇» en la fuente normal); revisar también el mapa de glifos de `xdir-mac/src/render/macos.rs` por si hace falta una entrada. Al principio de `draw`, para las páginas, poner `branch_button` vacío junto a los otros. Añadir `PaneHit::BranchButton` con su comentario, y en `hit` el chequeo junto a los otros dos botones. El tooltip va en `tooltip.rs` junto a `ViewButton`; el texto del tooltip se arma desde `tab().git` del panel `p`.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test -p xdir-core --lib branch_button branch_tooltip`
Expected: PASS; además `cargo test -p xdir-core --lib footer` sigue en verde (la prueba existente de los botones del pie).

- [ ] **Step 5: Commit**

```bash
git add crates/xdir-core/src/pane.rs crates/xdir-core/src/app/tooltip.rs crates/xdir-core/src/app/tests.rs
git commit -m "feat(git): botón con la rama actual en el pie del panel"
```

---

### Task 5: El menú de ramas y el cambio de rama

**Files:**
- Create: `crates/xdir-core/src/app/git_branch.rs`
- Modify: `crates/xdir-core/src/app/mod.rs` (`mod git_branch;`, campo `git_busy: bool` en `App` e inicialización, `git::warm_up()` en `App::new` bajo `#[cfg(not(test))]`)
- Modify: `crates/xdir-core/src/app/tasks.rs` (variantes de `TaskDone` y su despacho en `on_task_done`)
- Modify: `crates/xdir-core/src/app/input.rs` (~línea 429: `PaneHit::BranchButton => self.open_branch_menu(i)`)
- Test: `crates/xdir-core/src/app/git_branch_tests.rs` (nuevo, `#[cfg(test)] mod git_branch_tests;` en `mod.rs`, como `in_use_tests.rs`)

**Interfaces:**
- Consumes: `git::{branches, switch, SwitchOutcome, RepoHead}`, `Tab::git`, `Pane::branch_button_rect`.
- Produces:
  - `TaskDone::GitBranches { pane: usize, root: PathBuf, result: Result<Vec<String>, String> }`
  - `TaskDone::GitSwitched { root: PathBuf, dest: String, how: GitHow, result: git::SwitchOutcome }`
  - `pub(super) enum GitHow { Plain, Stash, Discard }` (`Clone, Copy, PartialEq, Debug`)
  - `impl App { pub(super) fn open_branch_menu(&mut self, pane: usize); pub(super) fn on_git_branches(&mut self, pane: usize, root: PathBuf, result: Result<Vec<String>, String>); pub(super) fn start_switch(&mut self, root: PathBuf, dest: String, how: GitHow); pub(super) fn on_git_switched(&mut self, root: PathBuf, dest: String, how: GitHow, result: git::SwitchOutcome); }`

- [ ] **Step 1: Escribir las pruebas que fallan** (con `FakeHost`: `menus()`, `set_menu_choice`, `questions()`; esperar tareas con `network_tests::wait_until`):
  - `click_opens_menu_with_branches`: clic en `BranchButton` de un repo con `main`/`feature` → `host.menus()` tiene un menú con dos `Entry::Item`: `main` con `checked: true, enabled: false` y `feature` habilitada; el menú se pidió con `MenuPos::Above` (registrar la posición en `FakeHost` si hoy no se guarda).
  - `menu_caps_at_40_branches`: 45 ramas → 40 items + `…y 5 más` deshabilitado.
  - `menu_empty_repo_says_no_branches`: repo sin commits → un solo item deshabilitado `(sin ramas)`.
  - `menu_detached_head_marks_none`: HEAD suelto → ninguna rama con `checked`.
  - `choosing_branch_switches_and_reloads_all_panes`: dos paneles abiertos en el mismo repo, `set_menu_choice(Some(2))` (feature) → al terminar, `read_head` da `feature` y ambos `tab().git` dicen `Branch("feature")`.
  - `switch_failed_shows_notice`: elegir una rama cuyo cambio falla por otra causa → `a.notice` contiene el texto de git y `host.questions()` queda vacío (no hay cartel).
  - `click_ignored_while_busy`: con `git_busy = true`, el clic no abre menú.
  - `result_for_vanished_pane_does_not_panic`: aplicar `on_git_branches` con `pane` fuera de rango y `on_git_switched` con una raíz que ninguna pestaña tiene no hace panic ni abre menú.
  - `cancel_menu_does_nothing`: `set_menu_choice(None)` → no cambia la rama y `git_busy` queda en `false`.

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test -p xdir-core --lib git_branch_tests`
Expected: error de compilación (módulo y funciones no existen).

- [ ] **Step 3: Implementar.** `open_branch_menu`: si `git_busy` o la pestaña no tiene `git`, volver; releer `read_head` (actualiza `tab.git`), poner `git_busy = true` y `spawn_task` con `git::branches`. `on_git_branches`: `git_busy = false`; si el panel ya no existe o el `result` es `Err(t)` → `notice = Some(t)` y volver; armar las `Entry::Item` con id = posición+1 (reglas de Global Constraints y casos de las pruebas); `host.popup_menu(&entries, MenuPos::Above(r.left, r.top), None, None)` con `r = branch_button_rect()` del panel; si devuelve `Some(id)` válido, `start_switch(root, rama, GitHow::Plain)`. `start_switch`: `git_busy = true` y `spawn_task_or` (con `fallback` que devuelve `GitSwitched` con `Failed("git no respondió")`) que llama a `git::switch`, `stash_and_switch` o `discard_and_switch` según `how`. `on_git_switched`: `git_busy = false`; `Switched` → recargar (`reload(hwnd, &tx, true)`) toda pestaña de cualquier panel cuyo `git.root == root`, y si `how == Stash`, `notice = Some("Tus cambios están en el stash (xdir: antes de pasar a <dest>)")`; `Failed(t)` → `notice = Some(t)`; `Dirty` se resuelve en Task 6 (por ahora `notice` genérico). Seguir el patrón de `hwnd`/`tx` de `input.rs` (~línea 410).

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test -p xdir-core --lib git_branch_tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/xdir-core/src/app/git_branch.rs crates/xdir-core/src/app/git_branch_tests.rs crates/xdir-core/src/app/mod.rs crates/xdir-core/src/app/tasks.rs crates/xdir-core/src/app/input.rs crates/xdir-core/src/sys/testing.rs
git commit -m "feat(git): menú de ramas y cambio de rama desde el pie del panel"
```

---

### Task 6: El cartel de cambios sin commitear

**Files:**
- Modify: `crates/xdir-core/src/app/git_branch.rs` (rama `Dirty` de `on_git_switched`)
- Test: `crates/xdir-core/src/app/git_branch_tests.rs`

**Interfaces:**
- Consumes: `SwitchOutcome::Dirty { files }`, `start_switch`, `GitHow` de Task 5; `host.ask_yes_no_cancel` / `host.ask_yes_no`.
- Produces: `fn ask_dirty(&mut self, root: PathBuf, dest: String, files: usize)` (privada).

Textos exactos:
- Paso 1, título: `No se puede pasar a «<rama>»`. Texto: `Tienes cambios sin guardar que chocan con esa rama. ¿Guardarlos en el stash y cambiar? Sí: guardar en el stash y cambiar. No: descartar los cambios. Cancelar: quedarse aquí.`
- Paso 3, título: `Descartar cambios`. Texto: `Se perderán <N> archivos modificados. Esto no se puede deshacer. ¿Descartar y cambiar?` (con `N == 1`: `Se perderá 1 archivo modificado. …`).

- [ ] **Step 1: Escribir las pruebas que fallan** (repo con conflicto armado como en Task 2; `FakeHost::set_answer`; `host.questions()`):
  - `dirty_asks_stash_first`: tras elegir `feature` aparece una sola pregunta con el título y el texto exactos del paso 1.
  - `dirty_yes_stashes_and_switches`: `set_answer(Some(true))` → rama `feature`, el archivo del repo vuelve limpio, `git stash list` tiene `xdir: antes de pasar a feature` y `a.notice` dice `Tus cambios están en el stash (xdir: antes de pasar a feature)`.
  - `dirty_no_then_yes_discards`: la respuesta es `No` a la primera y `Sí` a la segunda (el `FakeHost` devuelve la misma respuesta a las dos; ampliarlo con una cola `set_answers(Vec<Option<bool>>)` si hace falta) → hay dos preguntas, la segunda con `Se perderá 1 archivo modificado…`, y la rama es `feature` con el cambio descartado y un archivo sin seguimiento intacto.
  - `dirty_no_then_no_does_nothing`: `No` y `No` → rama sigue `main`, el cambio sigue ahí, no hay `notice` de error.
  - `dirty_cancel_does_nothing`: `set_answer(None)` → una sola pregunta, nada cambia.
  - `dirty_count_in_plural`: 3 archivos modificados → `Se perderán 3 archivos modificados…`.
  - `stash_failure_reports_error_and_keeps_changes_message`: un `Failed` posterior al stash llega a `notice` con la frase «Tus cambios quedan en el stash».

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test -p xdir-core --lib dirty_`
Expected: FAIL (el `Dirty` todavía solo muestra un aviso genérico).

- [ ] **Step 3: Implementar** `ask_dirty` y llamarla desde la rama `Dirty` de `on_git_switched`: `Some(true)` → `start_switch(.., GitHow::Stash)`; `Some(false)` → segunda pregunta con `ask_yes_no` y, si es sí, `start_switch(.., GitHow::Discard)`; `None` → nada.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test -p xdir-core --lib dirty_ git_branch_tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/xdir-core/src/app/git_branch.rs crates/xdir-core/src/app/git_branch_tests.rs crates/xdir-core/src/sys/testing.rs
git commit -m "feat(git): preguntar si guardar en el stash o descartar cuando git se niega a cambiar"
```

---

### Task 7: Novedades y verificación final

**Files:**
- Modify: `CHANGELOG.md`
- Test: ninguno nuevo (se corre todo)

- [ ] **Step 1: Agregar la entrada** en la sección vigente de `CHANGELOG.md`, con el estilo de las entradas vecinas (una línea: botón con la rama de git en el pie del panel, menú para cambiarla y qué pasa con los cambios sin commitear).

- [ ] **Step 2: Correr todo**

Run: `cargo test -p xdir-core` y `cargo build --workspace`
Expected: todo en verde y compila en los tres crates de plataforma.

- [ ] **Step 3: Probar a mano en Windows** (no se puede automatizar): abrir xdir en este repo; verificar el botón en ambos paneles, el menú hacia arriba, un cambio limpio, un cambio con conflicto (editar un archivo que difiere entre dos ramas) con las tres respuestas, y que en `C:\` sin repo y en una unidad de red no aparece el botón. Anotar el resultado.

- [ ] **Step 4: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(git): novedades del selector de rama"
```

# Funciones del Explorador, tanda A — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Las 8 funciones de la tanda A (tamaño de la selección, selección por patrón, copiar como ruta, mostrar extensiones, más consolas, archivo de texto nuevo, historial de Atrás/Adelante y árbol que sigue al panel) en las 3 plataformas.

**Architecture:** Todo lo de interfaz y lógica en `xdir-core` (lista, paleta, barra de estado, árbol, comandos, sesión, configuración, `App`); en cada crate de plataforma solo lo que decide el sistema: crear el archivo (`FileOps`, `Op::NewFile`) y la lista de consolas (`SystemEnv::consoles` / `open_console`).

**Tech Stack:** Rust 2024, workspace `xdir-core` + `xdir-windows` + `xdir-mac` + `xdir-linux`.

**Spec:** `docs/superpowers/specs/2026-11-09-explorador-tanda-a-design.md`

## Global Constraints

- `xdir-core` sin dependencias ni `#[cfg]` de sistema en producción. Tests de `App` con `install_fakes()` y `FakeHost`.
- `cargo test` en la Mac (hoy 188) y en Linux con Docker (`sh /private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/dk.sh cargo test`, hoy 211) sin bajar; `cargo check --all-targets` sin errores ni warnings nuevos para la Mac, `x86_64-pc-windows-msvc` y `x86_64-unknown-linux-gnu`; CI verde (prueba de humo de Windows incluida).
- Textos visibles exactos de la spec: «N elementos seleccionados · 4,2 MB», «1 elemento seleccionado · …», «Seleccionar por patrón…», «Deseleccionar por patrón…», «Copiar como ruta de acceso», «Mostrar extensiones», «Archivo de texto nuevo», «Nuevo documento de texto.txt», `arbol_sigue_panel`.
- Atajos: `Ctrl+Más` / `Ctrl+Menos` (selección por patrón), `Ctrl+Alt+N` (archivo de texto nuevo); en macOS los equivalentes con ⌘ en la tabla de atajos de macOS (`crates/xdir-mac/src/sys/macos.rs`, `KEYS`).
- Comportamientos que no cambian: «Copiar las rutas como texto», «Carpeta nueva», el menú contextual del sistema.
- No `cargo fmt` del repo. Castellano; Conventional Commits sin atribución a Claude. Un PR: rama `feature/tanda-a`, base `develop`.

## Review Focus

- **Selección por patrón con nombres raros**: espacios, mayúsculas, sin extensión, `;` al final, patrón vacío → nada se rompe y el vacío no cambia la selección → tests en Task 2.
- **Extensiones ocultas y nombres sin extensión o que empiezan con punto** (`.gitignore`, `archivo.`, `a.tar.gz`): `.gitignore` se muestra completo, `a.tar.gz` → `a.tar`, carpetas completas; ordenar sigue usando el nombre completo → tests en Task 1.
- **Archivo nuevo cuando ya existe «Nuevo documento de texto.txt»** y en carpetas virtuales o sin permiso: sufijo « (2)»; error visible si no se puede crear, sin pánico → tests en Task 3.
- **Historial con muchas carpetas**: el menú muestra 15 como máximo, la más cercana primero, y elegir la 3.ª equivale a tres «Atrás» (Adelante queda con las salteadas) → test en Task 5.
- **Árbol que sigue a una carpeta que no está en el árbol** (red, zip, Este equipo, carpeta borrada): no se cuelga ni expande de más; solo se mueve si encuentra la rama → tests en Task 5.

---

### Task 1: Tamaño de la selección, copiar como ruta y mostrar extensiones

**Files:**
- Modify: `crates/xdir-core/src/tab.rs` (`status()`), `crates/xdir-core/src/list.rs` (nombre mostrado sin extensión; ordenar no cambia), `crates/xdir-core/src/session.rs` (`show_extensions: bool`, clave `extensiones`, por defecto `true`), `crates/xdir-core/src/ui.rs` (switch en la rueda dentada), `crates/xdir-core/src/app/{appearance,fileops,events}.rs` (switch, entrada del menú contextual, copiar), `crates/xdir-core/src/app/tests.rs`.

**Interfaces:**
- Produces:
  - `pub fn display_name(name: &str, is_dir: bool, show_ext: bool) -> &str` en `list.rs` (pura): sin extensión solo para archivos cuyo nombre tiene un `.` que no está en la posición 0; la extensión es lo que sigue al último `.`.
  - `FileList` gana `pub show_extensions: bool` (lo fija la `App` en todas las listas al cambiar el switch y al crear pestañas).
  - `pub fn selection_summary(count: usize, file_bytes: Option<u64>) -> String` en `tab.rs` (pura): `1` → «1 elemento seleccionado», `n` → «n elementos seleccionados», y si `file_bytes` es `Some(b)` agrega « · » + `format::size_short(b)`.
  - `pub fn quoted_paths(paths: &[PathBuf]) -> String` en `app/fileops.rs` (o donde vive «Copiar las rutas como texto»): cada ruta entre `"` y separadas por `\r\n` en Windows y `\n` en el resto — usar el mismo separador que ya usa «Copiar las rutas como texto».
  - Entrada nueva del menú contextual de elementos: «Copiar como ruta de acceso», junto a las entradas propias existentes.

- [ ] **Step 1: Tests que fallan**: `display_name_hides_extension` (`"foto.jpg"` → `"foto"`, `"a.tar.gz"` → `"a.tar"`, `".gitignore"` → `".gitignore"`, `"archivo."` → `"archivo"`, `"sin"` → `"sin"`, carpeta `"x.d"` → `"x.d"`, y con `show_ext = true` siempre igual), `selection_summary_texts` (`(1, Some(2048))` → `"1 elemento seleccionado · 2 KB"` con el formato del falso; `(3, None)` → `"3 elementos seleccionados"`), `status_shows_selection_size` (App con `FakeHost`: carpeta temporal con 2 archivos de 1000 y 3000 bytes y una carpeta; seleccionar los 3 → la barra de estado empieza con «3 elementos seleccionados» y contiene el tamaño de 4000 bytes; solo la carpeta → «1 elemento seleccionado» sin « · »), `copy_as_path_quotes` (con `FakeShell`: elegir «Copiar como ruta de acceso» sobre 2 elementos → el texto copiado son las 2 rutas entre comillas en líneas separadas), `extensions_switch_persists` (apagar el switch → `Session` guardada tiene `show_extensions == false` y al volver a cargar sigue apagado; el nombre dibujado en la lista con `RecordingPainter` es `foto` para `foto.jpg`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar. El renombrado en línea sigue abriendo con el nombre completo.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): tamaño de la selección, copiar como ruta y mostrar extensiones`.

### Task 2: Selección por patrón

**Files:**
- Modify: `crates/xdir-core/src/commands.rs` (`Command::SelectPattern`, `Command::DeselectPattern`, ids `seleccionar_patron` / `deseleccionar_patron`, atajos), `crates/xdir-core/src/palette.rs` (modo nuevo), `crates/xdir-core/src/list.rs`, `crates/xdir-core/src/app/{actions,quick_jump}.rs` (o donde la paleta aplica su resultado), `crates/xdir-mac/src/sys/macos.rs` (`KEYS`: `⌘+` / `⌘-` con el formato que usa esa tabla).

**Interfaces:**
- Produces:
  - `PaletteMode::Pattern { select: bool }`: la paleta muestra el texto escrito como única fila («Seleccionar los que coinciden con «*.jpg»» / «Deseleccionar…»), Enter aplica y cierra, Esc cierra sin cambios.
  - `pub fn matches_pattern(name: &str, pattern: &str) -> bool` en `list.rs` (pura): parte el patrón por `;`, ignora partes vacías y espacios alrededor, compara sin distinguir mayúsculas con `config::wildcard`; patrón sin partes → `false`.
  - `FileList::select_matching(&mut self, pattern: &str, select: bool) -> usize` (suma o quita de la selección los visibles que coinciden; devuelve cuántos cambió; no toca la selección si el patrón no tiene partes).

- [ ] **Step 1: Tests que fallan**: `pattern_matches_several_and_ignores_case` (`"FOTO.JPG"` con `"*.jpg;*.png"` → true; `"a.txt"` → false; `"nombre con espacios.jpg"` con `" *.jpg ; "` → true; `"sin"` con `"*"` → true; cualquier nombre con `""` o `";"` → false), `select_matching_adds_and_removes` (lista con `a.jpg`, `b.png`, `c.txt`: seleccionar `*.jpg;*.png` → 2 seleccionados; deseleccionar `*.png` → queda `a.jpg`; patrón vacío → no cambia), `palette_pattern_mode_selects_on_enter` (App: `run_command(SelectPattern)` abre la paleta en modo patrón; escribir `*.jpg` y Enter → selección con los `.jpg`; Esc → sin cambios), `pattern_commands_have_keys` (`Ctrl+Más` y `Ctrl+Menos` en la tabla por defecto; en la tabla de macOS, ⌘+ y ⌘-).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): seleccionar y deseleccionar por patrón`.

### Task 3: Archivo de texto nuevo

**Files:**
- Modify: `crates/xdir-core/src/ops.rs` (`Op::NewFile { parent: PathBuf }`), `crates/xdir-core/src/commands.rs` (`Command::NewTextFile`, id `archivo_texto_nuevo`, `Ctrl+Alt+N`; en macOS `⌘⌥N` en `KEYS`), `crates/xdir-core/src/app/actions.rs` (como `NewFolder`), `crates/xdir-windows/src/ops/win32.rs`, `crates/xdir-mac/src/ops/macos.rs`, `crates/xdir-linux/src/ops.rs`, y el falso `FakeFileOps` si hace falta.

**Interfaces:**
- Produces: `Op::NewFile { parent }` → crea un archivo vacío llamado «Nuevo documento de texto.txt» (o « (2)», « (3)»… si existe) y devuelve `OpResult { undo: Some(Undo::Created(vec![ruta])), focus: Some(ruta), rename_after: true, error: None }` — igual que `NewFolder` en cada plataforma (leer cómo lo resuelve cada una: Windows con `IFileOperation::NewItem` y `FILE_ATTRIBUTE_NORMAL` como hace con la carpeta; macOS y Linux con `OpenOptions::create_new`).
- La `App` abre el renombrado en línea al recibir el resultado, igual que con la carpeta nueva (ya lo hace por `rename_after`).

- [ ] **Step 1: Tests que fallan**: en el núcleo `new_text_file_sends_op` (App: `run_command(NewTextFile)` en una carpeta → `FakeFileOps::take_ops() == [Op::NewFile { parent }]`; en «Este equipo» no manda nada); en `xdir-linux` (Docker) `new_file_creates_and_renames_on_conflict` (dos veces en la misma carpeta → «Nuevo documento de texto.txt» y «Nuevo documento de texto (2).txt», vacíos, `rename_after`, `Undo::Created`), `new_file_error_without_permission` (carpeta 0o500, no root → `error` con texto, sin pánico); en `xdir-mac` el mismo par de tests en una carpeta temporal.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar en el núcleo y en las 3 plataformas.
- [ ] **Step 4:** PASS (Mac, Docker); `cargo check` para Windows; en el cuerpo del PR, prueba manual para Runick93.
- [ ] **Step 5: Commit** `feat: archivo de texto nuevo en las tres plataformas`.

### Task 4: Más consolas

**Files:**
- Modify: `crates/xdir-windows/src/sys/win32.rs` (consolas y `open_console`), `crates/xdir-mac/src/sys/macos.rs`, `crates/xdir-linux/src/env.rs`.

**Interfaces:**
- Produces:
  - Windows: `ConsoleInfo { id: WINDOWS_TERMINAL, label: "Windows Terminal" }` cuando `wt.exe` existe en el PATH o en `%LOCALAPPDATA%\Microsoft\WindowsApps\wt.exe`; se abre con `wt.exe -d <carpeta>` (verbos alternativos como las otras).
  - macOS: «iTerm» si existe `/Applications/iTerm.app` (o `~/Applications/iTerm.app`); se abre con `open -a iTerm <carpeta>`.
  - Linux: la primera que exista en el PATH de `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal`, `xterm`, con su nombre como etiqueta; se abre con el directorio de trabajo en la carpeta (`Command::current_dir`).
  - La detección se separa en funciones puras que reciben la lista de rutas existentes (o un cierre `exists: &dyn Fn(&Path) -> bool`) para poder probarlas.

- [ ] **Step 1: Tests que fallan**: `windows_terminal_listed_when_present` / `…_absent` (función pura de Windows con un `exists` falso; corre en la CI de Windows), `iterm_listed_when_present` (macOS, función pura), `linux_terminal_prefers_x_terminal_emulator` (función pura con un PATH simulado).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en la Mac y en Docker; `cargo check` para Windows.
- [ ] **Step 5: Commit** `feat: Windows Terminal, iTerm y la terminal de Linux en el menú de consolas`.

### Task 5: Historial de Atrás/Adelante y el árbol que sigue al panel

**Files:**
- Modify: `crates/xdir-core/src/tab.rs` (lista del historial e ir a una posición), `crates/xdir-core/src/app/input.rs` (clic derecho en `Button::Back` / `Button::Forward` → `popup_menu`), `crates/xdir-core/src/tree.rs` (revelar una carpeta), `crates/xdir-core/src/config.rs` (opción `arbol_sigue_panel`, por defecto `true`, en la sección `[opciones]` como `animaciones`), `crates/xdir-core/src/app/{panes,events}.rs` (llamar a revelar al cambiar la carpeta del panel activo), tests.

**Interfaces:**
- Produces:
  - `Tab::back_list(&self, max: usize) -> Vec<Location>` y `Tab::forward_list(&self, max: usize) -> Vec<Location>` (la más cercana primero); `Tab::go_back_n(&mut self, n: usize, hwnd: Handle, tx: &Sender<LoadEvent>)` y `go_forward_n` (equivalen a `n` veces atrás/adelante, en una sola carga).
  - Menú: `Entry::Item` con `location.title()` de cada una, máximo 15; elegir la i-ésima llama a `go_back_n(i + 1)`.
  - `Tree::reveal(&mut self, hwnd: Handle, path: &Path, show_hidden: bool)`: si la ruta está bajo una raíz del árbol (unidad o carpeta del usuario), despliega cada carpeta padre (pidiendo la carga de las que no están cargadas y continuando en `on_loaded` hasta llegar), y al llegar desplaza la vista para que la fila quede a la vista; si no está bajo ninguna raíz (red, zip, «Este equipo», carpeta inexistente), no hace nada. Una revelación nueva reemplaza a la pendiente.
  - `Config::tree_follows_pane: bool` (por defecto `true`; `arbol_sigue_panel = false` lo apaga).

- [ ] **Step 1: Tests que fallan**: `back_list_is_capped_and_nearest_first` (20 navegaciones → `back_list(15)` tiene 15 con la anterior primero), `go_back_n_keeps_forward` (A→B→C→D, `go_back_n(2)` → B, y `forward_list` = [C, D]), `back_button_right_click_shows_history` (App con `FakeHost`: clic derecho sobre el botón Atrás → `menus().last()` tiene las carpetas anteriores; con `set_menu_choice(Some(id de la 2.ª))` → la pestaña queda en esa carpeta), `tree_reveal_expands_ancestors` (con `install_fakes()` y una carpeta temporal `a/b/c` bajo una raíz del árbol: `reveal(a/b/c)` y procesar las cargas → los nodos `a` y `b` quedan expandidos y la fila de `c` dentro del área visible), `tree_reveal_ignores_paths_outside_roots` (`reveal` de una ruta que no está bajo ninguna raíz → nada cambia, sin pánico), `tree_follow_option` (`arbol_sigue_panel = false` → `Config::tree_follows_pane == false`; la App no llama a `reveal`). Los tests del árbol van con `#[cfg(feature = "arbol")]`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS (`cargo test`, `cargo test -p xdir-core --features arbol`); Global Constraints; humo aislado en la Mac (capturas de siempre con `run.sh`: deben seguir iguales salvo fechas) más una captura del árbol siguiendo al panel.
- [ ] **Step 5: Commit** `feat(core): historial de Atrás y Adelante y el árbol que sigue al panel`.

### Task 6: PR

- [ ] **Step 1:** Verificación final: `cargo test` (Mac), Docker, `cargo check` de los 3 sistemas.
- [ ] **Step 2:** Abrir el PR (`feature/tanda-a`, base `develop`) con confirmación del usuario. Lista para Runick93 en Windows: barra de estado con la selección; `Ctrl+Más` con `*.jpg`; «Copiar como ruta de acceso» y pegar en el Bloc de notas; apagar «Mostrar extensiones»; `Ctrl+Alt+N` y renombrar; Windows Terminal en el menú de consolas (si lo tiene); clic derecho en Atrás; el árbol (con `--features arbol`, que viene por defecto) siguiendo al panel.

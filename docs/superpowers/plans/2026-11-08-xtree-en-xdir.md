# xtree dentro de xdir — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** xtree (mapa de uso de disco) pasa a ser una vista de pantalla completa dentro de xdir, en Windows y en macOS, y se elimina el programa `xtree` aparte.

**Architecture:** Módulo neutro `src/xtree/` (árbol, escáner con cola de hilos, treemap, colores, vista que dibuja con el `Painter` de xdir) sin `#[cfg]` de sistema. Lo de cada sistema va en `fs::win32` / `fs::macos` con las mismas funciones, reexportadas desde `fs`, como el resto de xdir. El código actual de `src/bin/xtree/win.rs` se reparte; `mft.rs` se mueve sin cambiar su lógica.

**Tech Stack:** Rust 2024, crate `windows` 0.62, `objc2` 0.6 / `objc2-*` 0.3, `libc` 0.2 (macOS), FigCraft (`~/bin/figcraft-cli-0.12.1/figcraft-cli`).

**Spec:** `docs/superpowers/specs/2026-11-08-xtree-macos-design.md`

## Global Constraints

- Un solo crate y un solo binario (`xdir`); se borran `src/bin/xtree/`, el `[[bin]]` y la feature `xtree`.
- Sin `#[cfg(windows)]` / `#[cfg(target_os = "macos")]` dentro de `src/xtree/`.
- Verificación en cada tarea, desde la Mac: `cargo test` y `cargo check --all-targets --target x86_64-pc-windows-msvc` sin errores ni warnings nuevos (`export PATH="$HOME/.cargo/bin:$PATH"`).
- macOS mínimo 12.0. Textos de interfaz y comentarios en castellano.
- Paridad con xtree al commit `879b0e9`: medidas (`MARGIN 8`, `TOP_H 40`, `STATUS_H 58`, `LIST_W 372`, `TABLE_H 34`, `ROW_H 32`, `CARD_H 76`, `LEGEND_H 40`, `GAP 6`, `MENU_ROW 34`, `MENU_W 400`), `MAX_FILES_PER_DIR = 1500`, `MAX_CELLS = 400_000`, refresco en vivo 250 ms, hilos `available_parallelism().clamp(4, 8)`.
- Chrome de la vista con el `Theme` de xdir; rueda `WHEEL` y categorías `CATS` sin cambios.
- Commits con Conventional Commits (skill `conventional-commits`), sin atribución a Claude.
- `docs/` está en `.gitignore` (decisión de Runick93): no se fuerza con `-f` salvo que el usuario lo decida.

## Review Focus

- **Escaneo en curso y el usuario cierra la vista, cambia de carpeta o cierra xdir:** sin cuelgues, sin CPU colgada, los resultados viejos se descartan por `scan_id` → test en Task 3 (`cancel_stops_workers_and_late_result_is_ignored`).
- **Carpeta raíz vacía o de un solo archivo de 0 bytes:** la vista muestra el estado vacío sin panic (división por cero en `%`, `squarify` con área 0) → test en Task 2 (`layout_of_empty_folder_has_no_cells`) y Task 4 (`paint_with_empty_tree_does_not_panic`).
- **Nombres no UTF-8 o con caracteres raros (Mac permite bytes arbitrarios):** se muestran con `to_string_lossy` y `node_path` devuelve la ruta real → test en Task 6 (`lists_non_utf8_names`).
- **Enlace simbólico que apunta a una carpeta padre (ciclo):** no se sigue → test en Task 6 (`does_not_follow_symlinks`).
- **«Abrir en el panel» con el panel activo en «Este equipo» o en una ubicación del Shell:** navega igual a la ruta → test en Task 4 (`open_in_panel_navigates_active_pane`).

---

### Task 1: Pantallas en FigCraft y `docs/DISENO.md` (puerta: aprobación del usuario)

**Files:**
- Create: `design/xdir.figcraft`
- Create: `docs/DISENO.md`
- Create (fuera del repo): scripts de `figcraft-cli run` en el scratchpad

**Interfaces:**
- Produces: las pantallas aprobadas que Tasks 4–6 reproducen.

- [ ] **Step 1:** Leer `/Volumes/Javier/Projects/javier/cromos/docs/DISENO.md` (forma del documento) y `figcraft-cli commands` (comandos de páginas, variables, componentes y texto).
- [ ] **Step 2:** Página **Tokens**: variables con los colores de `src/theme.rs` (modo claro y oscuro, los 12 `PRESETS` de acento), `WHEEL` y `CATS` de `src/bin/xtree/win.rs`, y medidas de las Global Constraints. Estilos de texto: `ui` 13, `ui_bold` 13 semibold, `title` 16 semibold, `small` 11.5, `small_bold` 12.5 semibold.
- [ ] **Step 3:** Página **Componentes**, con el nombre de la función de Rust que lo dibuja: `ui::draw_status` (barra de estado con botón «xtree» encendido), `xtree::view::table_row`, `xtree::view::folder_block`, `xtree::view::file_block`, `xtree::view::card` (botones «Abrir en el panel», «Copiar ruta», «Explorar aquí»), `xtree::view::drive_menu`, `xtree::view::disk_bar`, `xtree::view::legend`.
- [ ] **Step 4:** Página **Pantallas · xtree**, 1400×820, en tema oscuro y claro: (1) escaneando con mapa en vivo, (2) escaneando, fase MFT con % (Windows), (3) completo con color por carpeta, (4) color por tipo con leyenda, (5) menú de discos abierto, (6) aviso «N carpetas sin permiso… (tecla A)» (Mac), (7) aviso de administrador (Windows).
- [ ] **Step 5:** Renderizar fuera del repo: `figcraft-cli render design/xdir.figcraft -o <scratchpad>/xtree-<n>.png --page "Pantallas · xtree"`. Revisar las imágenes.
- [ ] **Step 6:** Escribir `docs/DISENO.md` con: qué páginas hay y a qué módulo corresponde cada una, archivos de `design/` (solo el `.figcraft`, sin imágenes), cómo se edita (`figcraft-cli run` / `render`, MCP opcional con `claude mcp add -s local figcraft -- ~/bin/figcraft-cli-0.12.1/figcraft-cli mcp`), y diferencias a propósito (fuente Inter en FigCraft vs Segoe UI / San Francisco).
- [ ] **Step 7:** Mostrar los renders al usuario y **esperar su aprobación**. Los cambios que pida se hacen en el `.figcraft` antes de seguir.
- [ ] **Step 8: Commit** `design/xdir.figcraft` (y `docs/DISENO.md` solo si el usuario destrabó `docs/` del `.gitignore`): `docs(design): pantallas de xtree dentro de xdir en FigCraft`.

### Task 2: Núcleo de datos: árbol, treemap y colores

**Files:**
- Create: `src/xtree/mod.rs`, `src/xtree/tree.rs`, `src/xtree/treemap.rs`, `src/xtree/colors.rs`
- Modify: `src/main.rs` (agregar `mod xtree;`)
- Test: `#[cfg(test)] mod tests` en cada archivo

**Interfaces:**
- Produces:
  - `tree.rs`: `pub struct Node { pub name: Box<str>, pub parent: u32, pub size: u64, pub is_dir: bool, pub small: bool, pub children: Vec<u32> }`, `pub const NO_PARENT: u32`, `pub type Tree = Arc<RwLock<Vec<Node>>>`, `pub fn rows_of(arena: &[Node], current: u32) -> Vec<Option<u32>>`, `pub fn node_path(arena: &[Node], root: &Path, id: u32) -> PathBuf`, `pub fn sort_children(arena: &mut [Node])`, `pub fn encode(arena: &[Node]) -> Vec<u8>`, `pub fn decode(bytes: &[u8]) -> Option<Vec<Node>>`.
  - `treemap.rs`: `pub struct Cell { pub r: Rect, pub node: u32, pub top: u32, pub dir: bool, pub depth: u8, pub hue: f32, pub small: bool }`, `pub fn squarify(areas: &[f64], r: Rect) -> Vec<Rect>`, `pub fn header_h(depth: u8, w: f32, h: f32) -> f32`, `pub fn layout(arena: &[Node], id: u32, r: Rect, out: &mut Vec<Cell>)`.
  - `colors.rs`: `pub fn position_color(hue: f32, depth: u8, dir: bool) -> Color`, `pub fn file_color(name: &str) -> Color`, `pub fn category(name: &str) -> usize`, `pub const CATS`, `pub fn lighten(c: Color, k: f32) -> Color`, `pub fn text_on(c: Color) -> Color`, `pub fn fmt_size(n: u64) -> String`.
  - `Rect` y `Color` son los de `crate::platform` (no los de Direct2D).

- [ ] **Step 1: Tests que fallan**
  - `squarify_fills_the_rect`: áreas `[600.0, 300.0, 100.0]` en `rect(0,0,40,25)` → 3 rects, la suma de sus áreas es 1000 ± 0.01 y todos están dentro.
  - `layout_of_empty_folder_has_no_cells`: raíz sin hijos o con `size == 0` → `out` vacío, sin panic.
  - `layout_nests_folders`: raíz con carpeta A (archivos de 70 y 30) y archivo B (100) en `rect(0,0,400,300)` → hay una `Cell` dir de A, dos de archivo con `top == A`, una de B con `top == B`.
  - `rows_of_puts_up_first_and_sorts_by_size`: hijo de la raíz con 3 hijos desordenados → `[None, mayor, medio, menor]`; en la raíz no hay `None`; los de tamaño 0 no aparecen.
  - `node_path_joins_from_root`: raíz `/tmp/r` (Mac) o `C:\r` (Windows, con `cfg!(windows)` en el test para elegir la raíz) más `a/b` → `root.join("a").join("b")`.
  - `encode_decode_roundtrip`: un árbol de 5 nodos → `decode(&encode(&t)) == t` campo por campo; `decode(&[1,2,3]) == None`.
  - `fmt_size_matches_windows`: `0 → "0 B"`, `1536 → "1,50 KB"`, `5 * 1024^3 → "5,00 GB"`.
  - `category_by_extension`: `"a.MP4"` → índice de «Video»; `"sin_ext"` → último.
- [ ] **Step 2:** `cargo test xtree::` → FAIL (módulos no existen).
- [ ] **Step 3:** Mover desde `src/bin/xtree/win.rs` (líneas 95–105, 452–726): `squarify`, `header_h`, `layout`, colores, `fmt_size`, `rows_of`, cambiando `D2D1_COLOR_F` por `Color` y `D2D_RECT_F` por `Rect`. `layout` público sin los parámetros `top`/`depth` (un `layout_at` privado los lleva). `node_path` arma un `PathBuf` desde `root` (el nombre del nodo raíz es solo para mostrar). `encode`/`decode`: formato binario propio, little endian: `u32` cantidad, y por nodo `u32 len + bytes del nombre, u32 parent, u64 size, u8 flags (dir=1, small=2), u32 n hijos + u32 por hijo`; `decode` devuelve `None` ante cualquier lectura fuera de rango.
- [ ] **Step 4:** `cargo test xtree::` → PASS. `cargo check --all-targets --target x86_64-pc-windows-msvc` → OK (warnings de código sin usar: `#[allow(dead_code)]` temporal en `mod xtree;` hasta la Task 4).
- [ ] **Step 5: Commit** `refactor(xtree): núcleo neutro con árbol, treemap y colores`.

### Task 3: Núcleo del escáner

**Files:**
- Create: `src/xtree/scan.rs`
- Modify: `src/fs/win32.rs`, `src/fs/macos.rs` (stubs de las funciones de la interfaz; las implementan Tasks 5 y 6)

**Interfaces:**
- Consumes: `Node`, `Tree`, `NO_PARENT`, `sort_children` (Task 2).
- Produces (en `scan.rs`):
  - `pub struct DirListing { pub dirs: Vec<OsString>, pub files: Vec<FileEntry> }`, `pub struct FileEntry { pub name: Box<str>, pub size: u64, pub id: Option<u64> }` (`id` solo si el archivo puede tener enlaces duros).
  - `pub enum ListError { Denied, Other }`.
  - `pub trait Lister: Sync { fn list(&self, path: &Path) -> Result<DirListing, ListError>; }`
  - `pub struct Progress { pub files: AtomicU64, pub bytes: AtomicU64, pub phase: AtomicU32, pub mft_done: AtomicU64, pub mft_total: AtomicU64, pub denied: AtomicU64, pub need_admin: AtomicBool }` (fase: 0 recorriendo, 1 leyendo la MFT, 2 armando el árbol).
  - `pub struct Scan { pub id: usize, pub live: Tree, pub progress: Arc<Progress>, pub cancel: Arc<AtomicBool>, pub result: Arc<Mutex<Option<Vec<Node>>>> }`.
  - `pub fn walk(root: &Path, label: &str, lister: &dyn Lister, live: &Tree, progress: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>>` (bloqueante; `None` si se canceló).
  - `pub fn start(hwnd: Handle, id: usize, root: PathBuf) -> Scan`: hilo que primero prueba `fs::xtree_fast_scan`, si no corresponde usa `walk` con `fs::xtree_lister(&root)`, guarda en `result` y avisa con `hwnd.post(WM_APP_XTREE, id, 1)`; otro hilo avisa `hwnd.post(WM_APP_XTREE, id, 0)` cada 250 ms mientras no terminó ni se canceló.
  - `pub const WM_APP_XTREE: u32 = WM_APP + 22;`
- Produces (interfaz por sistema, en `fs::win32` y `fs::macos`, reexportada por `fs`):
  - `pub fn xtree_lister(root: &Path) -> impl Lister + 'static`
  - `pub enum FastScan { NotApplicable, Done(Vec<Node>), NeedsAdmin }` (definido en `scan.rs`, usado por los dos) y `pub fn xtree_fast_scan(root: &Path, progress: &Progress, cancel: &AtomicBool) -> FastScan`
  - `pub fn xtree_elevate(root: &Path, progress: &Progress, cancel: &AtomicBool) -> Option<Vec<Node>>`
  - `pub struct DriveEntry { pub path: PathBuf, pub label: String, pub detail: String }` (en `scan.rs`) y `pub fn xtree_drives() -> Vec<DriveEntry>`
  - `pub fn volume_space(path: &Path) -> Option<(u64, u64)>`, `pub fn whole_volume(path: &Path) -> bool`, `pub fn xtree_default_root() -> PathBuf`
  - `pub const XTREE_PERMISSION_HINT: &str`
  - En esta tarea, todas son stubs (`NotApplicable`, `None`, `vec![]`, lister que devuelve `Err(Other)`).

- [ ] **Step 1: Tests que fallan** (con un `FakeLister` en el test que sirve un árbol en memoria)
  - `walk_builds_tree_and_totals`: raíz con `a/` (archivos 10 y 20) y `b.bin` (5) → raíz `size == 35`, `a` `size == 30`, hijos ordenados de mayor a menor, `progress.files == 3`, `progress.bytes == 35`.
  - `walk_groups_small_files`: carpeta con 1503 archivos de tamaño `i+1` → 1500 nodos propios y uno `small` con nombre `"… 3 archivos pequeños"` y la suma de los 3 menores.
  - `walk_counts_hard_links_once`: dos archivos con el mismo `id = Some(7)` y tamaño 100 → el total suma 100.
  - `walk_counts_denied_folders`: una subcarpeta que devuelve `Err(Denied)` → `progress.denied == 1`, el resto del árbol está completo.
  - `cancel_stops_workers_and_late_result_is_ignored`: `FakeLister` que demora 50 ms por carpeta y 200 carpetas; se cancela a los 20 ms → `walk` devuelve `None` en menos de 1 s.
- [ ] **Step 2:** `cargo test xtree::scan` → FAIL.
- [ ] **Step 3:** Implementar moviendo `Ctx`, `worker`, `insert_listing` y el orden final de `win.rs` (líneas 138–160, 284–358, 435–444): la cola lleva `PathBuf` en vez de `Vec<u16>`; la deduplicación por `id` (64 shards) y el agrupado pasan de `list_dir` a `insert_listing`. Un pánico en un `list` se trata como `Err(Other)` (`catch_unwind`, como hoy).
- [ ] **Step 4:** `cargo test xtree::scan` → PASS; check de Windows OK.
- [ ] **Step 5: Commit** `feat(xtree): escáner neutro con cola de hilos y listado por sistema`.

### Task 4: La vista dentro de xdir

**Files:**
- Create: `src/xtree/view.rs`
- Modify: `src/render/win32.rs` y `src/render/macos.rs` (`Fonts`: agregar `title` 16 semibold, `small` 11.5, `small_bold` 12.5 semibold)
- Modify: `src/render/macos.rs` (`symbol_name`: confirmar `E70D`, `E838`, `E8C8`, `E8A7`, agregar las que falten)
- Modify: `src/app/mod.rs` (`xtree: Option<Xtree>` y `xtree_open: bool` en `App`), `src/app/input.rs` (`Hit::Xtree` abre/cierra; con la vista abierta, el mouse sobre el área de paneles va a la vista), `src/app/paint.rs` (con vista abierta: dibuja `xtree.paint` en `ui::panes_area` ampliada sobre la franja de grupos, oculta el árbol, enciende el botón), `src/app/events.rs` (`WM_APP_XTREE` → `xtree.on_message`; rueda), `src/app/actions.rs` (`on_key`: si la vista está abierta, primero `xtree.on_key`; `Command::Xtree`), `src/app/saved_groups.rs` (borrar `open_xtree`), `src/commands.rs` (`Command::Xtree`, id `"xtree"`, nombre `"Mapa de uso de disco (xtree)"`, sin teclas), `src/ui.rs` (`xtree_rect` siempre con ancho `XTREE_W`; `draw_status` con el botón encendido según un parámetro nuevo), `src/shell/win32.rs` y `src/shell/macos.rs` (`pub fn pick_folder(owner: Handle, title: &str) -> Option<PathBuf>`; Windows con `IFileOpenDialog` + `FOS_PICKFOLDERS`, Mac con `NSOpenPanel` `canChooseDirectories`)
- Test: `src/app/tests.rs`, `#[cfg(test)]` en `view.rs`

**Interfaces:**
- Consumes: Tasks 2 y 3; `Painter`, `Theme`, `shell::copy_text`, `shell::pick_folder`.
- Produces:
  - `pub struct Xtree` con `pub fn new(hwnd: Handle, root: PathBuf) -> Xtree` (arranca el escaneo).
  - `pub fn paint(&mut self, p: &Painter, area: Rect)`.
  - `pub fn on_mouse_move(&mut self, area: Rect, x: f32, y: f32) -> bool` (true = repintar).
  - `pub fn on_click(&mut self, area: Rect, x: f32, y: f32, double: bool) -> Action`, `pub fn on_right_click(&mut self) -> Action`, `pub fn on_wheel(&mut self, area: Rect, lines: f32) -> bool`, `pub fn on_key(&mut self, k: Key) -> Action`, `pub fn on_message(&mut self, wparam: usize, lparam: isize) -> bool`, `pub fn cancel(&self)`.
  - `pub enum Action { None, Redraw, Close, OpenInPanel { folder: PathBuf, select: Option<OsString> } }`.
  - Teclas: Retroceso sube, Esc cierra el menú de discos o, si no hay menú, `Close`; T alterna color; F5 re-analiza; A con `progress.need_admin || progress.denied > 0` lanza `fs::xtree_elevate` en un hilo y, si devuelve un árbol, lo toma como resultado.

- [ ] **Step 1: Tests que fallan**
  - `xtree_button_toggles_view`: clic en el centro de `ui::xtree_rect` → `a.xtree.is_some() && a.xtree_open`; otro clic → `!a.xtree_open`, `a.xtree` sigue existiendo y su escaneo no se canceló.
  - `closing_xdir_cancels_scan`: con la vista abierta, `a.on_closing()` → `scan.cancel` en true.
  - `xtree_view_swallows_pane_clicks`: con la vista abierta, un clic en el centro del panel 0 no cambia `a.active` ni la selección.
  - `open_in_panel_navigates_active_pane`: con el panel activo en `Location::Computer`, `Action::OpenInPanel { folder: tmp, select: None }` → la vista se cierra y la pestaña activa está en `Location::Path(tmp)`.
  - `command_xtree_is_listed`: `commands::all()` incluye `Command::Xtree` con id `"xtree"`.
  - `paint_with_empty_tree_does_not_panic` (en `view.rs`): `layout_ui` + hit-tests con árbol vacío y con un solo nodo de tamaño 0 → sin panic; `row_at` devuelve `None`.
  - `escape_closes_menu_then_view`: con el menú abierto, Esc → `Action::Redraw` y menú cerrado; Esc otra vez → `Action::Close`.
- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Mover a `view.rs` el estado y la lógica de `App` de `win.rs` (líneas 749–1268: `Ui`, `ui_layout`, `menu_rect`, `rows`, `row_at`, `cell_at`, `button_at`, `menu_row_at`, `choose`, `scan_pick`, `pick_label`, `target`, eventos) y `paint` (líneas 1270–1618) contra `Painter`: `rfill → round_fill`, `rstroke → round_stroke_w`, `stroke → round_stroke_w` con radio 0, `dot → circle`, `text(fmt…)` con las fuentes de `Fonts` (`big→title`, `head→small_bold`, `right→ui_right`, `center→ui_center`, `glyph→glyphs_small`), `clip/unclip → p.clip(r, || …)`. Las coordenadas llegan en DIPs relativas a la ventana: se quita `to_dips`. Los colores fijos de chrome (`BG`, `PANEL`, `PANEL_DEEP`, `BORDER`, `TEXT`, `DIM`, `ACCENT`, `SELECT`) pasan a `theme.background`, `surface`, `surface_alt`, `border`, `text`, `text_dim`, `accent` y `accent` respectivamente; `FOLDER_ICON` y `GREEN` quedan fijos. Botón 0: «Abrir en el panel» devuelve `OpenInPanel` (carpeta del nodo; si es archivo, su carpeta y `select = Some(nombre)`). Raíz al abrir: carpeta del panel activo si es `Location::Path`, si no `fs::xtree_default_root()`. Si la vista ya existía (cerrada con el escaneo en segundo plano), se reabre sin re-analizar: cerrar la vista la oculta (`App.xtree_open = false`) y no la destruye. `App::on_closing` llama `xtree.cancel()`.
- [ ] **Step 4:** `cargo test` → PASS; check de Windows OK. Correr xdir en Mac (`cargo run`), abrir la vista y verificar que dibuja con los stubs (mapa vacío, mensaje «Escaneando…» y luego vacío).
- [ ] **Step 5: Commit** `feat(xtree): vista de pantalla completa dentro de xdir`.

### Task 5: Windows: listado, MFT, ayudante elevado y limpieza del binario

**Files:**
- Create: `src/fs/win32/xtree.rs` (o funciones al final de `src/fs/win32.rs`, a elección)
- Move: `src/bin/xtree/win/mft.rs` → `src/fs/win32/mft.rs` (solo cambian los `use super::…` por `crate::xtree::{tree::*, scan::*}`)
- Modify: `src/app/win.rs` (`run`: si los argumentos son `--xtree-mft <unidad> <archivo>`, llamar `fs::xtree_helper_main` y salir antes de crear ventana, sesión o config)
- Delete: `src/bin/xtree/`, `[[bin]]` y feature `xtree` de `Cargo.toml` (y el comentario de `--no-default-features`), `scripts/package.ps1` (parámetro y copia de `xtree.exe`), `installer/xdir.iss` (`XtreeExe`, `Source:` y `taskkill /IM xtree.exe`); `[InstallDelete]` del `.iss` borra `{app}\xtree.exe` de instalaciones viejas.

**Interfaces:**
- Consumes: interfaz de Task 3.
- Produces: implementaciones Windows de `xtree_lister` (`NtQueryDirectoryFile` con `FILE_ID_FULL_DIR_INFO`, mismos atributos y `disk_size` que hoy; `id` = file id), `xtree_fast_scan` (raíz de unidad NTFS → `mft::scan`; `Fail::Denied` → `NeedsAdmin`), `xtree_elevate` (`ShellExecuteW` `runas` sobre `current_exe()` con `--xtree-mft C: %TEMP%\xdir-xtree-<pid>.bin`, espera el proceso, `decode` del archivo y lo borra), `pub fn xtree_helper_main(args: &[String]) -> i32` (`mft::scan` + `encode` al archivo), `xtree_drives`, `volume_space`, `whole_volume` (`C:\`), `xtree_default_root` (`%SystemDrive%\`), `XTREE_PERMISSION_HINT = "Escaneo rápido de la MFT: requiere administrador (tecla A)"`.

- [ ] **Step 1:** Mover los `mft::tests` con el archivo; `cargo test --target x86_64-pc-windows-msvc --no-run` no se puede linkear desde Mac: verificar con `cargo check --all-targets --target x86_64-pc-windows-msvc`.
- [ ] **Step 2:** Implementar las funciones moviendo el código de `win.rs` (líneas 115–282, 360–450, 773–849, 913–930).
- [ ] **Step 3:** Borrar el binario y el empaquetado. `grep -rn xtree.exe scripts installer src` → solo el `[InstallDelete]`.
- [ ] **Step 4:** `cargo test` (Mac) y check de Windows sin errores ni warnings.
- [ ] **Step 5: Commit** `feat(xtree): escaneo de Windows dentro de xdir y fin del programa aparte`.

### Task 6: macOS: escáner, discos y permisos

**Files:**
- Create: `src/fs/macos/xtree.rs` (o funciones al final de `src/fs/macos.rs`)
- Test: `#[cfg(test)]` en ese archivo (corre solo en Mac)

**Interfaces:**
- Consumes: interfaz de Task 3.
- Produces: implementaciones Mac de:
  - `xtree_lister`: `getattrlistbulk` con `ATTR_CMN_NAME | ATTR_CMN_OBJTYPE | ATTR_CMN_FILEID | ATTR_CMN_FLAGS | ATTR_CMN_DEVID`, `ATTR_FILE_LINKCOUNT | ATTR_FILE_ALLOCSIZE`, y `ATTR_CMN_RETURNED_ATTRS`.
    - No sigue `VLNK`.
    - Saltea archivos con `SF_DATALESS`.
    - `id = Some(fileid)` solo si `linkcount > 1`.
    - `EPERM` o `EACCES` al abrir la carpeta → `ListError::Denied`.
    - Dispositivos permitidos: el de la raíz y, si la raíz es `/`, también el de `/System/Volumes/Data`. Cuando la raíz es `/`, saltea la ruta `/System/Volumes`.
  - `xtree_fast_scan` → `NotApplicable`.
  - `xtree_elevate`: abre `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles` con `NSWorkspace` y devuelve `None`.
  - `xtree_drives`: `/` con `drive_name` y cada volumen visible de `/Volumes` (sin los ocultos ni los que apuntan a `/`), con `detail` `"<fs> · <tamaño>"` de `statfs.f_fstypename`.
  - `volume_space`: `statfs`, con `f_blocks * f_bsize` y `f_bavail * f_bsize`.
  - `whole_volume`: `/` o `/Volumes/X`.
  - `xtree_default_root`: `/`.
  - `XTREE_PERMISSION_HINT = "{n} carpetas sin permiso: dale a xdir Acceso total al disco (tecla A)"` (la vista reemplaza `{n}`).

- [ ] **Step 1: Tests que fallan** (en un tmp creado por el test)
  - `lists_sizes_and_dirs`: carpeta con `sub/` y un archivo de 10 000 bytes → `dirs == ["sub"]`, un `FileEntry` con `size >= 10_000` (tamaño asignado).
  - `hard_links_have_id`: `std::fs::hard_link` de un archivo → los dos `FileEntry` traen el mismo `Some(id)`; un archivo sin enlaces trae `None`.
  - `does_not_follow_symlinks`: `symlink("..", "loop")` → no aparece en `dirs`; `walk` sobre el tmp termina.
  - `lists_non_utf8_names`: archivo con nombre de bytes `b"a\xffb"` → aparece con `to_string_lossy` y `node_path` + `exists()` es true.
  - `denied_folder_is_reported`: carpeta con permisos `0o000` → `Err(ListError::Denied)` (restaurar permisos al final).
  - `root_allows_data_volume`: con raíz `/`, la regla de dispositivos acepta el `st_dev` de `/System/Volumes/Data` y rechaza el de `/Volumes/<otro>` si existe uno montado (si no hay, el test solo cubre el primero).
- [ ] **Step 2:** `cargo test fs::` → FAIL.
- [ ] **Step 3:** Implementar con `libc` (`open` con `O_RDONLY | O_DIRECTORY`, `getattrlistbulk` en un buffer de 256 KiB, parseo de atributos según `ATTR_CMN_RETURNED_ATTRS`).
- [ ] **Step 4:** `cargo test` → PASS. Correr xdir, analizar `~` y `/`: el total de `/` queda por debajo de «Utilizado» y no tiene `/System/Volumes/Data` como hijo; probar la tecla A con el aviso visible.
- [ ] **Step 5: Commit** `feat(xtree): escaneo de macOS con getattrlistbulk, discos y permisos`.

### Task 7: Cierre: lista de pruebas, nota del vault y PR

**Files:**
- Modify: `README.md` (si menciona `xtree.exe` como programa aparte)
- Modify: `/Volumes/Javier/Vault/Proyectos/Javier/xdir.md` (sección Notas)

- [ ] **Step 1:** `sh scripts/package-mac.sh` arma `xdir.app` sin `xtree` y la app abre la vista.
- [ ] **Step 2:** Escribir la lista de pruebas de Windows para el cuerpo del PR (dirigida a Runick93):
  - abrir y cerrar la vista desde el botón y desde la paleta;
  - escaneo de `C:` con MFT sin ser administrador, con la tecla A y el ayudante, aceptando y cancelando el pedido de Windows;
  - escaneo de una carpeta y de un pendrive FAT32;
  - color por tipo (T), F5 y el menú de discos con «Seleccionar carpeta…»;
  - «Abrir en el panel», «Copiar ruta» y «Explorar aquí»;
  - temas claro y oscuro;
  - el instalador borra el `xtree.exe` viejo.
- [ ] **Step 3:** Agregar en la nota del vault: xtree integrado en xdir, escáner por sistema y ayudante elevado.
- [ ] **Step 4:** `git push -u origin feature/xtree-macos` y PR a `develop` con `gh pr create` (sin atribución a Claude), **con confirmación del usuario antes del push**.

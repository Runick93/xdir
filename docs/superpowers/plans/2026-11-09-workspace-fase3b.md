# Workspace, fase 3b: dibujo, ventana y accesibilidad — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Las interfaces `Painter`, `WindowHost` y `Accessibility` definidas en `xdir-core` e implementadas por Windows y macOS, con todo el código que dibuja (lista, paneles, pestañas, árbol, paleta, xtree, «Acerca de», barra de estado) y la configuración y la sesión movidos al núcleo, y con `App` sin tipos de sistema salvo su arranque, sin cambiar comportamiento.

**Architecture:** `Painter` es un trait object-safe que el renderizador de cada plataforma implementa para un cuadro; el núcleo dibuja con `&dyn Painter` y nombra las fuentes con `FontId`. `WindowHost` es por ventana y vive en el hilo de interfaz: no va al registro `sys` (que es `Send + Sync`), sino que `App` lo guarda como `Rc<dyn WindowHost>` y lo recibe en `App::new`. El host de cada plataforma absorbe las funciones sueltas de `app/win.rs` / `app/mac.rs`, el estado `Sys`, las cajas de texto, los menús y el renderizador. La accesibilidad se parte en un modelo de solo lectura en el núcleo (`a11y`) y el pegamento COM de UIA que queda en Windows. `App` sigue en `crates/xdir` (pasa al núcleo en la fase 4), pero al terminar esta fase ya no tiene campos de sistema.

**Tech Stack:** Rust 2024, workspace con `crates/xdir` y `crates/xdir-core`, `windows` 0.62 (Direct2D/DirectWrite, UIA), `objc2` 0.6 (AppKit, Core Graphics), CI de la fase 1.

**Spec:** `docs/superpowers/specs/2026-11-08-workspace-core-design.md` (paso 4, interfaces `Painter`, `WindowHost`, `Accessibility`; «Tests»; «Rendimiento»)

## Global Constraints

- Ningún cambio de comportamiento ni visual. `cargo test` (workspace) en macOS ≥ 159 tests en cada tarea (hoy 107 en `xdir` + 52 en `xdir-core`; los que se mueven cuentan igual); `cargo check --all-targets --target x86_64-pc-windows-msvc` sin errores ni warnings nuevos; CI verde en cada PR (incluida la prueba de humo de Windows).
- `xdir-core` sin dependencias y sin `#[cfg(windows)]` / `#[cfg(target_os)]` en producción (`cfg!(feature = "arbol")` sí). Traits sin tipos de sistema; errores como `String`.
- `Painter` object-safe: nada genérico en sus métodos (`clip` toma `&mut dyn FnMut()`). `FontId` = {Ui, UiRight, UiCenter, UiBold, Title, Small, SmallBold, Glyphs, GlyphsSmall, Label}, uno por cada campo de `Fonts` de hoy, con la misma fuente, tamaño, peso y alineación.
- `WindowHost` no es `Send`: se usa solo desde el hilo de interfaz. Para llamar al host con una clausura que toca `App`, clonar el `Rc` antes (`let host = self.host.clone();`).
- Rendimiento (spec): antes de la Task 1 y después de la Task 2 medir el arranque con `--probe` en la Mac sobre la misma carpeta (5 corridas, mediana); margen ±10 %. Números en el cuerpo del PR 1; para Windows, pedirle a Runick93 la misma medición en la lista del PR.
- Tests: toda prueba que llegue a un servicio llama `install_fakes()`; los de `App` usan `FakeHost` (no crean ventana ni renderizador reales al terminar la Task 6). Rutas temporales en tests que corren en Windows: comparar con `fs::true_case(...)`.
- Prueba de humo en la Mac: copia aislada con `xdir.portable` + `XDIR_PRUEBAS`; nunca tocar `/Applications/xdir.app` ni su configuración; matar solo el PID propio; si se usa el portapapeles, guardarlo y restaurarlo.
- No correr `cargo fmt` sobre todo el repo.
- PRs: **todos con base `develop`** (no encadenados) y se mergean en orden. PR 1 = Task 1–2 en `refactor/painter`; PR 2 = Task 3–4 en `refactor/dibujo-core` (sale de `refactor/painter`); PR 3 = Task 5–7 en `refactor/window-host` (sale de `refactor/dibujo-core`); PR 4 = Task 8 en `refactor/accesibilidad` (sale de `refactor/window-host`). Cada cuerpo avisa que muestra los commits de los anteriores hasta que se mergeen y trae la lista de pruebas manuales de Windows para Runick93. Después del merge, borrar las ramas propias (locales y remotas) verificando `git log --no-merges origin/develop..<rama>` vacío.
- Castellano; Conventional Commits sin atribución a Claude.

## Review Focus

- **Texto alineado y recortado igual que hoy**: el tamaño en la lista va a la derecha (`UiRight`), los botones centrados (`UiCenter`); un `FontId` mal mapeado no rompe ningún test de lógica → test con `RecordingPainter` que fija la fuente de la columna Tamaño y del título del diálogo «Acerca de» (Task 1).
- **Clic derecho dentro del menú de favoritos en Windows**, también dentro de una subcarpeta y sobre la propia subcarpeta: con el menú anidado (`Entry::Sub`) la posición tiene que seguir llevando al favorito correcto → test de `fav_path_at` con raíz, desborde y subcarpeta, y sobre «(vacía)» que no da nada (Task 7).
- **Cajas de texto**: perder el foco cancela la barra de dirección y aplica el renombrado; los mensajes `WM_APP_*_DONE` tienen que seguir llegando igual → tests de `App` con `FakeHost` que cierran cada tipo de caja (Task 7).
- **Color personalizado**: en Windows el selector es modal y devuelve el color; en macOS el panel es asíncrono y el color llega por `MSG_ACCENT`. `choose_color` → `None` no puede cambiar el acento → test (Task 5).
- **Accesibilidad tras desplazar o filtrar**: los elementos se identifican por posición entre los visibles; después de filtrar, el elemento N ya no es el mismo → test del modelo que filtra y verifica nombre y límites del elemento 0 (Task 8).

---

### Task 1: `Painter`, `FontId`, tema y configuración en el núcleo

**Files:**
- Create: `crates/xdir-core/src/paint.rs` (`Painter`, `FontId`), `crates/xdir-core/src/thumbs.rs` (`Source`, `Key`), `Pixels` en `crates/xdir-core/src/paint.rs`
- Move: `crates/xdir/src/theme.rs` → `crates/xdir-core/src/theme.rs` (sin `apply_to_window`, que queda como `fn apply_window_theme(theme: &Theme, hwnd: HWND) -> bool` en `crates/xdir/src/app/win.rs` hasta la Task 5); `crates/xdir/src/config.rs` → `crates/xdir-core/src/config.rs` (con `wildcard`, que sale de `list.rs` hacia `config.rs` y `list` la reexporta)
- Modify: `crates/xdir/src/render/{win32,macos}.rs` (`impl xdir_core::paint::Painter for Painter<'_>`; `Fonts` se indexa por `FontId`), `crates/xdir/src/{theme,config,thumbs/mod,icons/mod}.rs` (reexportan del núcleo), y todo el que dibuja (`ui`, `list`, `pane`, `tree`, `palette`, `about`, `xtree/view`, `typed/draw`, `app/paint`): `&Painter` → `&dyn Painter`, `p.fonts.x` → `FontId::X`, `p.theme` → `p.theme()`, `p.file_colors` → `p.file_colors()`
- Create: `RecordingPainter` en `crates/xdir-core/src/sys/testing.rs`

**Interfaces:**
- Produces (en `xdir_core::paint`):
  - `pub enum FontId { Ui, UiRight, UiCenter, UiBold, Title, Small, SmallBold, Glyphs, GlyphsSmall, Label }` (`Clone, Copy, PartialEq, Eq, Debug, Hash`)
  - `pub trait Painter { fn fill(&self, r: Rect, c: Color); fn round_fill(&self, r: Rect, radius: f32, c: Color); fn rounded_path(&self, pts: &[(f32, f32, f32)], closed: bool, fill: Option<Color>, stroke: Option<(Color, f32)>); fn circle(&self, cx: f32, cy: f32, radius: f32, fill: Color); fn circle_stroke(&self, cx: f32, cy: f32, radius: f32, c: Color, width: f32); fn round_stroke_w(&self, r: Rect, radius: f32, c: Color, width: f32); fn round_stroke(&self, r: Rect, radius: f32, c: Color); fn text(&self, font: FontId, s: &str, r: Rect, c: Color); fn measure(&self, font: FontId, s: &str) -> f32; fn shell_icon(&self, index: i32, overlay: u8, x: f32, y: f32, size: f32); fn thumbnail(&self, key: &thumbs::Key, r: Rect) -> bool; fn logo(&self, r: Rect); fn thumb_px(&self, dips: f32) -> u32; fn clip(&self, r: Rect, f: &mut dyn FnMut()); fn theme(&self) -> &Theme; fn file_colors(&self) -> &[ColorRule]; }`
  - `pub struct Pixels { pub width: u32, pub height: u32, pub data: Vec<u8> }`; `xdir_core::thumbs::{Source, Key}` tal como hoy.
  - `xdir_core::theme::{Theme, Accent, PRESETS, DEFAULT_ACCENT, alpha}`; `Theme::from_system` sigue usando `sys::env()`.
  - `xdir_core::config::{Config, ColorRule, color_for, wildcard, …}` con la misma API pública que hoy.
  - `RecordingPainter::new(theme: Theme) -> Self` con `fn texts(&self) -> Vec<(FontId, String, Rect)>` (registra cada `text`; `measure` = 7.0 por carácter; `clip` llama a la clausura; íconos y miniaturas no hacen nada y `thumbnail` da `false`).
- Los que hoy devuelven algo desde `clip` (`list.rs`, `tree.rs`, `xtree/view.rs`, `about.rs`) lo sacan por una variable capturada.

- [ ] **Step 1: Medir antes**: `cargo build --release` y 5 corridas de `xdir --probe <archivo> <carpeta con ~2000 archivos>` en la copia aislada; anotar la mediana del primer cuadro en el ledger.
- [ ] **Step 2: Tests que fallan** (en `xdir-core`): `font_ids_are_distinct` (los 10 `FontId` distintos en un `HashSet`); `recording_painter_records_text` (`p.text(FontId::UiRight, "12 KB", r, c)` → `texts() == [(FontId::UiRight, "12 KB", r)]`); `color_for_matches_wildcards` y los tests que hoy tiene `config.rs` se mueven con el archivo. En `xdir`: `list_size_column_is_right_aligned` (dibujar una `FileList` con un archivo de 2048 bytes en vista detalles con `RecordingPainter`: el texto del tamaño usa `FontId::UiRight`) y `about_title_uses_title_font` (el nombre «xdir» del diálogo usa `FontId::Title`).
- [ ] **Step 3:** FAIL (no existen `FontId` ni `RecordingPainter`).
- [ ] **Step 4:** Implementar: el trait, `Fonts` como `[Format; 10]` indexado por `FontId as usize` en cada plataforma (los mismos formatos de hoy), la implementación del trait en los dos `Painter`, el cambio de firmas en quienes dibujan, y mover `theme.rs` y `config.rs`.
- [ ] **Step 5:** PASS; Global Constraints (tests, `cargo check` para Windows).
- [ ] **Step 6: Commit** `refactor(core): Painter y FontId en el núcleo, con tema y configuración`.

### Task 2: Prueba visual y de rendimiento del dibujo

**Files:**
- Ninguno de producción (salvo que la comparación muestre una diferencia: en ese caso se corrige en el archivo que dibuja mal y el commit lo dice).

- [ ] **Step 1:** En la copia aislada de la Mac, con la build de `develop` y la de la rama, abrir la misma carpeta en layout de 4 paneles, la paleta, «Acerca de», el selector de colores y xtree; capturar cada pantalla con `screencapture -l <ventana>` y comparar las parejas (`compare -metric AE` si está ImageMagick, si no, a ojo pixel por pixel con zoom). Diferencias aceptadas: ninguna.
- [ ] **Step 2: Medir después**: las mismas 5 corridas de `--probe`; mediana dentro de ±10 % de la del Step 1 de la Task 1. Anotar ambos números en el ledger y en el cuerpo del PR 1.
- [ ] **Step 3:** Escribir en el cuerpo del PR 1 la lista para Runick93: abrir xdir, 4 paneles con archivos de varios tipos y miniaturas, la paleta, «Acerca de», el selector de colores en claro y oscuro, xtree; todo debe verse igual que en `develop`; y la medición de `--probe` antes/después.
- [ ] **Step 4: Abrir PR 1** (`refactor/painter`, base `develop`) con confirmación del usuario.

### Task 3: Configuración visual, lista, sesión y diálogos al núcleo

**Files:**
- Move a `crates/xdir-core/src/`: `list.rs`, `ui.rs`, `about.rs`, `typed/draw.rs` (como `typed_draw.rs` o dentro de `typed.rs`), el parseo y la escritura de `session.rs` (lo que quedaba en `crates/xdir`), con sus tests
- Modify: `crates/xdir/src/{list,ui,about,session,typed/mod}.rs` quedan como reexportaciones (o se borran si nadie usa la ruta vieja); las llamadas a fachadas (`crate::fs::…`, `crate::icons::…`, `crate::shell::…`) en lo movido pasan a `xdir_core::sys::file_system()` / `overlays()` / `shell()`
- Lo que `about.rs` toma del sistema (`logo_pixels` lee bytes embebidos con `include_bytes!` de un recurso de `crates/xdir`): el recurso se mueve con el archivo o la función queda en `crates/xdir`; anotar la decisión.

**Interfaces:**
- Consumes: Task 1 (`Painter`, `FontId`, `Theme`, `Config`).
- Produces: `xdir_core::{list, ui, about, session}` con la misma API pública que hoy en `crates/xdir`; `xdir_core::session::{Session, load, save}` completo.

- [ ] **Step 1: Tests que fallan**: los tests de esos módulos movidos al núcleo (fallan por imports hasta que se muevan los módulos); `session_roundtrip_in_core` (en el núcleo, con `install_fakes()` y `config_dir` del falso apuntando a una carpeta temporal: guardar y leer una `Session` con 2 paneles, columnas no por defecto y acento `Rgb(0xF85149)` da lo mismo).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Mover y conectar.
- [ ] **Step 4:** PASS; Global Constraints; el número total de tests no baja.
- [ ] **Step 5: Commit** `refactor(core): lista, sesión, barra de estado y diálogos en el núcleo`.

### Task 4: Paneles, pestañas, árbol, paleta y vista de xtree al núcleo

**Files:**
- Move a `crates/xdir-core/src/`: `tab.rs`, `pane.rs`, `tree.rs`, `palette.rs`, `xtree/view.rs` (a `xtree/view.rs` del núcleo), con sus tests
- Modify: fachadas → `sys::*` en lo movido (`tree.rs`: `drives`, `drive_label`, `quick_access`, `shell_place_exists`…; `tab.rs`: `spawn_load`, `watch`, `request_overlays`, `open_item`, `open_file`; `xtree/view.rs`: `shell`); `crates/xdir/src/{tab,pane,tree,palette}.rs` y `xtree/mod.rs` reexportan o se borran.
- `xdir-core` declara la feature `arbol` y `crates/xdir` la reexpone (`arbol = ["xdir-core/arbol"]`); `pruebas` igual si algo movido la usa.

**Interfaces:**
- Consumes: Task 1–3.
- Produces: `xdir_core::{tab, pane, tree, palette, xtree::view}` con la misma API pública.

- [ ] **Step 1: Tests que fallan**: los de esos módulos movidos; `tree_lists_fake_drives` (en el núcleo, con un `FakeFileSystem` que da dos unidades, el árbol muestra dos nodos con los nombres del falso).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Mover y conectar; `cargo test -p xdir-core --features arbol` también pasa.
- [ ] **Step 4:** PASS; Global Constraints; en la Mac, humo aislado: navegar, abrir paleta y xtree, capturar.
- [ ] **Step 5: Commit** `refactor(core): paneles, pestañas, árbol, paleta y xtree en el núcleo`.
- [ ] **Step 6: Abrir PR 2** (`refactor/dibujo-core`, base `develop`) con confirmación del usuario. Lista para Runick93: navegar, pestañas (abrir, arrastrar, cerrar), árbol con `--features arbol`, paleta y salto rápido, xtree; todo igual que en `develop`.

### Task 5: `WindowHost` — la ventana

**Files:**
- Create: `crates/xdir-core/src/host.rs` (trait `WindowHost`, `MenuPos`, `TextKind`, `TextBox`), `FakeHost` en `crates/xdir-core/src/sys/testing.rs`
- Create: `crates/xdir/src/app/host_win.rs` y `crates/xdir/src/app/host_mac.rs` (`Win32Host`, `MacHost`), que absorben las funciones sueltas de `app/win.rs` / `app/mac.rs` (`invalidate`, `client_size`, `cursor_dips`, `set_title`, `request_close`, `focus_main`, `focus_main_if_lost`, `capture_mouse`, `release_mouse`, `beep`, `ask_yes_no_cancel`, `open_url`, `open_in_editor`, `wheel_lines`, `expand_env`) y los métodos de `App` `apply_window_theme`, `track_mouse_leave`, `choose_custom_color`, `start_drag`, `window_placement`, `show`; y lo que necesitan de `Sys` (`tracking_mouse`, `custom_colors`, `drop_target`)
- Modify: `crates/xdir/src/app/mod.rs` (`App` gana `host: Rc<dyn WindowHost>`; `App::new(init: Init, host: Rc<dyn WindowHost>)`; se borra el campo `#[cfg(test)] client`), todos los `app/*.rs` que llamaban a esas funciones, `app/tests.rs` (`App::new(Init::blank(), FakeHost::new())`)

**Interfaces:**
- Consumes: Task 1 (`Theme`), `xdir_core::shell::Item`.
- Produces (en `xdir_core::host`):
  - `pub trait WindowHost { fn attach(&self, window: Handle); fn invalidate(&self); fn client_size(&self) -> (f32, f32); fn cursor_dips(&self) -> (f32, f32); fn set_title(&self, title: &str); fn request_close(&self); fn focus_main(&self); fn focus_main_if_lost(&self); fn capture_mouse(&self); fn release_mouse(&self); fn track_mouse_leave(&self); fn beep(&self); fn ask_yes_no_cancel(&self, title: &str, text: &str) -> Option<bool>; fn open_url(&self, url: &str); fn open_in_editor(&self, path: &Path); fn wheel_lines(&self) -> f32; fn expand_env(&self, s: &str) -> String; fn apply_theme(&self, theme: &Theme, follow_system: bool) -> bool; fn choose_color(&self, current: u32) -> Option<u32>; fn start_drag(&self, items: &[Item]) -> Result<(), String>; fn placement(&self) -> Option<([i32; 4], bool)>; fn show(&self, saved: Option<([i32; 4], bool)>); }`
  - `apply_theme` devuelve si quedó el fondo translúcido (Mica); `choose_color` devuelve el color elegido si el selector es modal (Windows) o `None` si el color llega después por `MSG_ACCENT` (macOS). Los tamaños están en DIPs; el host lee el DPI de su ventana.
  - `FakeHost::new() -> Rc<FakeHost>` con `client: Cell<(f32, f32)>` (1280×800), `fn titles(&self) -> Vec<String>`, `fn invalidations(&self) -> usize`, `fn set_color_choice(&self, c: Option<u32>)`, `fn set_answer(&self, a: Option<bool>)`, `fn beeps(&self) -> usize`.
- `Win32Host`/`MacHost` se instalan en un `thread_local!` de su plataforma además de pasarse a `App`, para que `app/messages.rs` y `app/mac.rs` lleguen al tipo concreto.

- [ ] **Step 1: Tests que fallan** (en `xdir`, con `FakeHost`): `title_follows_group_name` (cargar el grupo «Trabajo» y pintar → `titles().last() == "xdir - Trabajo"`); `custom_color_none_keeps_accent` (`set_color_choice(None)`, elegir «Personalizado…» → el acento no cambia); `custom_color_some_sets_accent` (`Some(0xF85149)` → acento `Rgb(0xF85149)`); `failed_drag_shows_notice` (un `FakeHost` con `start_drag` que da `Err("x")` → aviso «No se pudo arrastrar: x»).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar el trait, los dos hosts y el cambio en `App`.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `refactor(core): la ventana detrás de WindowHost`.

### Task 6: El renderizador dentro del host

**Files:**
- Modify: `crates/xdir-core/src/host.rs` (suma `draw` y `text_width`), `crates/xdir/src/app/{host_win,host_mac,paint,input,messages,mac,mod}.rs`: `Renderer` pasa de `App` al host; `messages.rs` (`WM_SIZE`, `WM_DPICHANGED`) y `mac.rs` (`set_size`) le hablan al host concreto del `thread_local!`; `App::paint` arma el cuadro con `host.draw(...)`.

**Interfaces:**
- Consumes: Task 5.
- Produces: en `WindowHost`: `fn draw(&self, theme: &Theme, file_colors: &[ColorRule], paint: &mut dyn FnMut(&dyn Painter, f32, f32)) -> Result<(), String>` (incluye lo que hoy hace `ensure_target` al primer cuadro y devuelve error si el destino se perdió, igual que hoy); `fn text_width(&self, s: &str, bold: bool) -> f32`. `FakeHost::draw` llama a la clausura con un `RecordingPainter` del tamaño `client`; `FakeHost::text_width` = 7.0 por carácter.
- `App` deja de tener `renderer` y `sys`; `SysResult` deja de usarse en `App::new` (el error de crear el renderizador lo maneja el arranque de cada plataforma al crear el host).

- [ ] **Step 1: Tests que fallan**: `paint_draws_with_host` (con `FakeHost`, `app.paint()` → el `RecordingPainter` registró el texto de la barra de estado del panel activo); `double_click_header_fits_column_with_host_width` (doble clic en el borde de la columna Nombre ajusta el ancho usando `text_width` del host: con 7.0 por carácter y el nombre más largo de 10 caracteres, el ancho resultante es el que da `fit_column_at` con esa medida).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; `grep -n "renderer\|Sys\b\|SysResult" crates/xdir/src/app/{mod,paint,input,events,actions,panes,edit,favs,fileops,saved_groups,quick_jump,appearance}.rs` no encuentra nada.
- [ ] **Step 5: Commit** `refactor(core): el renderizador pasa al host de la ventana`.

### Task 7: Cajas de texto y menús en el host

**Files:**
- Modify: `crates/xdir-core/src/host.rs` (cajas y menús), `crates/xdir/src/app/{host_win,host_mac}.rs` (absorben `TextBox`, `TextKind::done_msg`, `popup_menu`, `build_fav_menu`, `FavMenu`, `update_edit_resources`, `on_fav_menu_right_click`, `edit_font`/`edit_brush`), `crates/xdir/src/app/{edit,favs,input,events,actions,panes,saved_groups,quick_jump,mod}.rs` (usan el host), `app/messages.rs` (`WM_MENURBUTTONUP` y `WM_CTLCOLOREDIT` van al host concreto)
- El menú de favoritos se arma una sola vez en código común (`App::fav_menu(&source) -> (Vec<Entry>, Vec<FavPath>)`, lo que hoy hace `build` en `mac.rs`) y se muestra con `popup_menu` en los dos sistemas.

**Interfaces:**
- Consumes: Task 5–6; `xdir_core::shell::Entry`.
- Produces (en `xdir_core::host`):
  - `pub enum TextKind { Address, Rename, FavRename, Filter, Palette }`; `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub struct TextBox(pub u32)`.
  - `pub enum MenuPos { At(f32, f32), Above(f32, f32), Cursor, CursorNested }`.
  - En `WindowHost`: `fn text_create(&self, kind: TextKind, r: Rect, text: &str, theme: &Theme) -> Option<TextBox>; fn text_get(&self, b: TextBox) -> String; fn text_set(&self, b: TextBox, text: &str); fn text_select(&self, b: TextBox, from: usize, to: isize); fn text_focus(&self, b: TextBox); fn text_has_focus(&self, b: TextBox) -> bool; fn text_destroy(&self, b: TextBox); fn text_restyle(&self, theme: &Theme); fn popup_menu(&self, entries: &[Entry], pos: MenuPos, dot: Option<(u32, Color)>, on_context: Option<&mut dyn FnMut(&[usize]) -> bool>) -> Option<u32>;`
  - `on_context(posiciones)` se llama con el camino de posiciones dentro del menú anidado (por ejemplo `[2, 0]` = primer elemento de la tercera entrada) al hacer clic derecho sobre un elemento; si devuelve `true`, el host cierra el menú y `popup_menu` devuelve `None`. En macOS se ignora (como hoy).
  - Función pura en el núcleo, en `favorites.rs`: `pub fn fav_path_at(favs: &Favorites, base: &FavPath, first: usize, positions: &[usize]) -> Option<FavPath>` (`base` = carpeta mostrada, vacío para la raíz; `first` = desplazamiento del desborde, 0 si no hay; `None` si el camino no lleva a un favorito, como «(vacía)»).
  - `FakeHost`: cajas en un `RefCell<HashMap<u32, (TextKind, String, bool)>>`, `fn text_of(&self, kind: TextKind) -> Option<String>`, `fn set_menu_choice(&self, id: Option<u32>)`, `fn menus(&self) -> Vec<Vec<Entry>>`.

- [ ] **Step 1: Tests que fallan**: en el núcleo `fav_path_at_root`, `fav_path_at_overflow` (`first = 3`, posiciones `[0]` → `[3]`), `fav_path_at_nested_group` (`[1, 0]` dentro del grupo `[1]` → `[1, 0]`), `fav_path_at_group_itself` (`[1]` → `[1]`), `fav_path_at_empty_marker_is_none`. En `xdir` con `FakeHost`: `address_edit_cancel_on_focus_loss` (abrir la barra de dirección, `WM_APP_EDIT_DONE` con wparam 0 → no navega y la caja se destruye); `rename_applies_on_done` (renombrado en línea, texto cambiado en el falso, `WM_APP_RENAME_DONE` con 1 → `FakeFileOps` recibe el renombrado); `fav_folder_menu_is_nested` (una carpeta de favoritos con un grupo adentro → `menus().last()` tiene un `Entry::Sub` con el nombre del grupo).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; en la Mac, humo aislado: barra de dirección, renombrar, filtro, paleta, menú de favoritos con subcarpeta.
- [ ] **Step 5: Commit** `refactor(core): cajas de texto y menús detrás de WindowHost`.
- [ ] **Step 6: Abrir PR 3** (`refactor/window-host`, base `develop`) con confirmación del usuario. Lista para Runick93: escribir una ruta en la barra (y Esc), renombrar con F2 y haciendo clic afuera, filtro rápido, paleta (flechas, Enter, Esc), menú de favoritos con una subcarpeta y clic derecho dentro del menú (sobre un favorito y sobre la subcarpeta), menú contextual de archivos, color personalizado, arrastrar archivos al Explorador, restaurar la posición de la ventana al reabrir, cambio de DPI entre monitores.

### Task 8: `Accessibility` — modelo en el núcleo, UIA en Windows

**Files:**
- Create: `crates/xdir-core/src/a11y.rs` (`El`, `Info`, `A11yView`, `parent`, `kids`, `navigate`, `focus_el`, `info`, `bounds`, `A11yAction`)
- Modify: `crates/xdir/src/app/uia.rs` (deja de leer `App` por dentro: llama a las funciones de `a11y` con `&App` como `A11yView` y aplica `A11yAction` con un método de `App`), `crates/xdir/src/app/mod.rs` (`impl A11yView for App`, `fn a11y_apply(&mut self, a: A11yAction)`, `uia_focus` → `a11y_focus`)
- Create: `Accessibility` en `xdir_core::host` (`fn notify_focus(&self, el: El)`); `App` lo llama por `host.accessibility()`.

**Interfaces:**
- Produces (en `xdir_core::a11y`):
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum El { Root, Pane(usize), Address(usize), List(usize), Item(usize, usize), Status }` (los mismos que hoy en `uia.rs`).
  - `pub struct ItemInfo { pub name: String, pub kind: String, pub size: String, pub date: String, pub selected: bool, pub rect: Rect }`; `pub struct Info { pub name: Option<String>, pub value: Option<String>, pub selected: bool, … }` con los mismos campos que hoy usa `uia.rs`.
  - `pub trait A11yView { fn shown_panes(&self) -> Vec<usize>; fn active(&self) -> usize; fn item_count(&self, pane: usize) -> usize; fn focus_pos(&self, pane: usize) -> Option<usize>; fn item_info(&self, pane: usize, pos: usize) -> Option<ItemInfo>; fn pane_title(&self, pane: usize) -> String; fn address_text(&self, pane: usize) -> String; fn status(&self) -> String; fn pane_rects(&self, pane: usize) -> (Rect, Rect, Rect); fn client(&self) -> (f32, f32); }`
  - `pub fn parent(el: El) -> Option<El>; pub fn kids(v: &dyn A11yView, el: El) -> Vec<El>; pub fn navigate(v: &dyn A11yView, el: El, dir: Nav) -> Option<El>; pub fn exists(v: &dyn A11yView, el: El) -> bool; pub fn focus_el(v: &dyn A11yView) -> El; pub fn info(v: &dyn A11yView, el: El) -> Info; pub fn bounds(v: &dyn A11yView, el: El) -> Rect;` con `pub enum Nav { Parent, Next, Previous, FirstChild, LastChild }`.
  - `pub enum A11yAction { Focus(El), Select(usize, usize), AddToSelection(usize, usize), RemoveFromSelection(usize, usize), Invoke(usize, usize) }`.
  - `pub trait Accessibility { fn notify_focus(&self, el: El); }`; `WindowHost` suma `fn accessibility(&self) -> Option<&dyn Accessibility> { None }`.
  - `FakeA11y` (una vista en memoria con N paneles y elementos) en `sys::testing`.

- [ ] **Step 1: Tests que fallan** (en el núcleo, con `FakeA11y`): `kids_of_root_are_shown_panes_and_status`; `navigate_next_item_stops_at_end`; `focus_el_is_list_when_empty`; `info_of_item_reads_name_and_selection`; `bounds_after_filter_follow_visible_items` (vista con 3 elementos, se filtra a 1: `info(Item(0, 0))` da el nombre del que quedó y `exists(Item(0, 1))` es `false`).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar el modelo, `impl A11yView for App`, `a11y_apply`, y reducir `uia.rs` al pegamento COM (proveedores, `VARIANT`, `SAFEARRAY`, coordenadas de pantalla).
- [ ] **Step 4:** PASS; Global Constraints; `grep -n "app\.panes\|app\.active\|\.tab()" crates/xdir/src/app/uia.rs` no encuentra nada.
- [ ] **Step 5: Commit** `refactor(core): modelo de accesibilidad en el núcleo`.
- [ ] **Step 6: Abrir PR 4** (`refactor/accesibilidad`, base `develop`) con confirmación del usuario. Lista para Runick93: con Narrador (Win+Ctrl+Enter), moverse con flechas por una lista (lee nombre, tipo, tamaño y fecha), Tab entre paneles, barra de dirección, seleccionar con Espacio/Ctrl, abrir con Enter; Accessibility Insights (si lo tiene) muestra el mismo árbol que en `develop`.

---

Después del merge de los 4 PRs: verificar en `develop`, borrar las ramas propias, borrar `.superpowers/sdd/2026-11-09-workspace-fase3b`, y anotar la fase en la nota del vault. Sigue la fase 4 (`App` al núcleo y separar `xdir-windows` / `xdir-mac`).

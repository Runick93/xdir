# Funciones del Explorador, tanda C (vista previa, detalles, vista rápida y Galería) — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Columna de vista previa con detalles, vista Galería y vista rápida con la barra espaciadora, en Windows, macOS y Linux (Linux sin interfaz: servicios y núcleo listos).

**Architecture:** Un servicio nuevo `sys::Preview` lee en otro hilo lo que la columna necesita además de la imagen (texto, cantidad de elementos de una carpeta, espacio de un disco, último acceso, «solo en la nube», sin permiso), con descarte por `generation`; la lectura es código neutro del núcleo (`preview::answer`) y cada plataforma solo aporta su chequeo de nube. La imagen grande es una extensión del servicio de miniaturas: `Painter::big_image` con escalones de tamaño y una caché LRU por bytes propia (`thumbs::BigCache`, en el núcleo, usada por cada renderizador). La columna, la Galería y la vista rápida (estado, recorrido, barra de título y tarjeta de respaldo) se dibujan y prueban en `xdir-core`; lo nativo es `QLPreviewPanel` en macOS y una ventana propia con `IPreviewHandler` fuera de proceso en Windows, detrás de métodos nuevos de `WindowHost`.

**Tech Stack:** Rust 2024; `xdir-core` sin dependencias; `windows` 0.62 (features nuevas si hacen falta: `Win32_UI_Shell` ya trae `IPreviewHandler`/`IPreviewHandlerFrame`/`IInitializeWithStream`/`AssocQueryStringW`; `Win32_System_Com_StructuredStorage` ya está), `windows-core` `#[implement]` para `IPreviewHandlerFrame`; macOS `objc2` 0.6 + `objc2-quick-look-ui` (mismo grupo de versiones que `objc2-quick-look-thumbnailing` 0.3.2).

**Spec:** `docs/superpowers/specs/2026-11-09-explorador-tanda-c-design.md` (incluida «Decisiones tomadas al revisar el diseño»). Diseño: `docs/DISENO.md` (X39–X44 y «Medidas que fija el diseño») y componentes «Tanda C: vista previa» de `docs/design/xdir.figcraft`.

## Global Constraints

- `xdir-core` sin dependencias ni `#[cfg]` de sistema en producción; lo de cada sistema detrás de `xdir_core::sys` o `WindowHost`, con su falso en `sys/testing.rs`. Tests de `App` con `install_fakes()`, `FakeHost` y `RecordingPainter`.
- Todas las funciones en Windows, macOS y Linux. Linux (sin interfaz): instala `Preview` real (lectura neutra, sin nube) y compila y prueba; `WindowHost` no existe allí todavía.
- Lecturas (imagen grande, texto, carpeta, disco) fuera del hilo de interfaz; respuesta con `generation`; la de una selección vieja se descarta. Mover la selección con las flechas nunca espera una vista previa.
- Nunca descargar archivos solo en la nube: macOS `SF_DATALESS`, Windows `FILE_ATTRIBUTE_OFFLINE`/`RECALL_ON_OPEN`/`RECALL_ON_DATA_ACCESS` (mismos chequeos que `crates/xdir-{mac,windows}/src/metadata.rs::cloud_only`). En Windows la imagen grande de esos archivos se pide con `SIIGBF_INCACHEONLY`; en macOS no se pide.
- Texto: hasta 64 KB leídos, sin ajustar líneas (se cortan); un corte de UTF-8 en el límite cuenta como texto.
- Imagen grande: lado pedido = escalón 256, 512, 1024 o 2048 px físicos (el menor ≥ lo necesario; tope 2048); caché aparte de la de la lista, LRU por bytes con tope 64 MB; mientras llega el escalón justo se dibuja escalado el que haya de la misma fuente.
- Medidas del diseño: columna 320 al empezar, 240–640, nunca deja un panel por debajo de `ui::MIN_PANE_W` (280); 6 de separación; margen 16; vista previa 288×216 (4:3 del ancho) o 288×300 para texto; detalles 20 más abajo, filas de 22, valor en x = 104. Botón 30×22 a 10 del primer botón de layout; «Grupos» y «xtree» corridos 40 a la izquierda. Galería: tira de 112 (línea `border` arriba), celdas 88×88, miniatura 72, paso 96, enfocado centrado; imagen grande con margen 16. Vista rápida: 960×680 centrada sobre xdir, mínimo 480×360, barra de título 40.
- Fuente de ancho fijo nueva `FontId::Mono` (11,5; Consolas en Windows, Menlo en macOS) al final del enum; `FontId::COUNT` = 11.
- Atajos: `Alt+P` (Windows y Linux) / `⌥⌘P` (macOS) la columna; `Ctrl+Shift+7` / `⌥⌘7` la Galería; Espacio la vista rápida. `View::Gallery` va **al final** de `VIEWS` (índice 6) y después de Miniaturas solo en `CYCLE`.
- Vista rápida: Espacio abre (no durante la búsqueda por tipeo; sobre un encabezado de grupo sigue plegando; Ctrl+Espacio sin cambios); Esc o Espacio cierran; flechas recorren el orden visible (la selección si había varios); el visor de Windows fuera de proceso (`CLSCTX_LOCAL_SERVER`), los handlers con `DisableProcessIsolation` = sin visor, si no responde en 3 s se muestra la imagen grande; teclas de vuelta por `IPreviewHandlerFrame::TranslateAccelerator`; en macOS los elementos `SF_DATALESS` no van a `QLPreviewPanel`.
- Textos exactos: «Mostrar u ocultar la vista previa», «Vista previa (Alt+P)» / «Vista previa (⌥⌘P)», «N elementos seleccionados», «Disponible solo en línea», «No se puede leer este archivo», «No hay un visor para este tipo de archivo», «Abrir», «Abrir con…», «Vista rápida», «Vista: galería», «Galería», «k de n», «N elementos · tamaño», rótulos «Tipo», «Tamaño», «Creado», «Modificado», «Último acceso», «Dimensiones», «Duración», «Autor», «Etiquetas», «Artista», «Álbum», «Espacio libre», «Capacidad».
- `cargo test` en la Mac y en Linux con Docker (`sh /private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/dk.sh cargo test`) sin bajar; `cargo check --all-targets` sin errores ni warnings nuevos para la Mac, `x86_64-pc-windows-msvc` (también `--no-default-features`) y `x86_64-unknown-linux-gnu`; clippy sin avisos nuevos en tests; CI verde.
- Layout según FigCraft: en cada tarea de interfaz, descomprimir `docs/design/xdir.figcraft` en una carpeta temporal y leer los nodos del componente y la pantalla (ids en `scratchpad/figcraft-c-report.md`); colores de las variables Tema.
- Smoke en la Mac al final de cada tarea de interfaz: `cargo build --release -p xdir-mac --features pruebas`, copia aislada con `xdir.portable` en `/private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/vis/<tarea>/` y `vis/run.sh`, capturas con comandos `XDIR_PRUEBAS` comparadas con `scratchpad/png/n_X39_o.png`…`n_X44_c.png`; nunca tocar `/Applications/xdir.app` en las pruebas; nunca `osascript quit`.
- No `cargo fmt` del repo. Castellano. Conventional Commits (tipo y scope en inglés, descripción en castellano) sin atribución a Claude; `git add` por nombre.
- PRs contra `develop` por parte terminada (ver «PRs» al final); al mergear la última, versión de Mac instalada en `/Applications`.

## Review Focus

- **Flechas rápidas sobre una carpeta de fotos grandes o en red**: la selección se mueve sin trabarse; solo se dibuja lo del último pedido; respuestas viejas no pisan → `preview_stale_reply_ignored` y `arrows_never_wait_for_preview` (Task 2).
- **Archivo solo en la nube** (columna, Galería, vista rápida): nunca se lee su contenido ni se pide su imagen en macOS; ícono y «Disponible solo en línea» → `preview_cloud_only_not_read` (Task 1), `quick_look_cloud_item_uses_card` (Task 4), test de Windows `cloud_big_image_cache_only` (Task 5).
- **Visor de Windows que se cuelga o falla** (`prevhost` muerto, handler roto, `DisableProcessIsolation`): xdir responde y muestra la imagen grande → `handler_timeout_falls_back` y `in_process_handler_is_no_viewer` (Task 5).
- **La carpeta cambia con la vista rápida abierta** (se borra el elemento, se renombra, se vacía): pasa al siguiente o se cierra, sin pánico por índices viejos → `quick_look_item_deleted_moves_next` y `quick_look_folder_emptied_closes` (Task 4).
- **Ventana angosta o pantalla con DPI distinto**: prender la columna nunca deja un panel bajo su mínimo y cambiar el ancho o el DPI no pide una imagen por cada píxel → `column_width_respects_pane_min` (Task 2) y `big_side_steps` (Task 1).

---

### Task 1: Servicio de vista previa e imagen grande

**Files:**
- Create: `crates/xdir-core/src/preview/mod.rs` (tipos, `answer`), `crates/xdir-core/src/preview/text.rs` (extensiones y detección de texto).
- Modify: `crates/xdir-core/src/lib.rs`, `sys/mod.rs` (trait `Preview`, `Services::preview`, `sys::preview()`; `FileSystem::cloud_only` con implementación por defecto `false`), `sys/testing.rs` (`FakePreview`, `FakeFileSystem::set_cloud_only`, `RecordingPainter::big_image`), `thumbs.rs` (`big_side`, `BigCache`), `paint.rs` (`Painter::big_image`, `FontId::Mono`), `crates/xdir-windows/src/{thumbs/win32.rs,render/win32.rs,sys/win32.rs,metadata.rs}`, `crates/xdir-mac/src/{thumbs/macos.rs,render/macos.rs,sys/macos.rs,metadata.rs}`, `crates/xdir-linux/src/sys.rs`.

**Interfaces:**
- Produces (`preview/mod.rs`):
  - `pub enum PreviewTarget { File(PathBuf), Folder(PathBuf), Drive(PathBuf) }`.
  - `#[derive(Clone, Debug, PartialEq)] pub enum PreviewBody { Image, Text { lines: Vec<String> }, Folder { items: u64 }, Drive { total: u64, free: u64 }, CloudOnly, Unreadable, Nothing }` (`Image`: lo dibuja `big_image`; `Nothing`: ícono del tipo).
  - `#[derive(Clone, Debug, PartialEq)] pub struct PreviewInfo { pub body: PreviewBody, pub accessed: u64 /* FILETIME, 0 si no hay */ }`.
  - `pub struct PreviewRequest { pub hwnd: Handle, pub tx: Sender<PreviewReply>, pub generation: u64, pub current: Arc<AtomicU64>, pub target: PreviewTarget }` con `fn is_stale(&self) -> bool` y `fn send(self, info: PreviewInfo)` (no manda si quedó viejo; despierta con `WM_APP_PREVIEW = WM_APP + 24`).
  - `pub struct PreviewReply { pub generation: u64, pub info: PreviewInfo }`.
  - `pub fn answer(req: PreviewRequest, cloud_only: &dyn Fn(&Path) -> bool)`: lectura neutra con `std::fs` en el hilo del llamador (nube → `CloudOnly` sin abrir; sin permiso/bloqueado → `Unreadable`; carpeta → cuenta con `read_dir`; disco → `sys::disk_scan().volume_space`; archivo → texto o `Image`; vacío → `Nothing`; `accessed` de `Metadata::accessed()` llevado a FILETIME). `catch_unwind` como `MetadataRequest::answer_with`.
  - `pub const TEXT_MAX: usize = 65_536; pub const TEXT_LINES: usize = 200;`
- Produces (`preview/text.rs`): `pub fn is_text_ext(ext: &str) -> bool` (sin distinguir mayúsculas: txt md rs toml json yaml yml xml html htm css js ts tsx jsx py c h cpp hpp cs java kt swift go rb php sh ps1 bat cmd ini cfg conf log csv tsv sql gitignore; **no** svg) y `pub fn decode_text(bytes: &[u8], cut: bool) -> Option<Vec<String>>` (UTF-8 válido sin NUL; con `cut`, un carácter incompleto al final se descarta; partido por `\n`, sin `\r` final). Sin extensión de texto se mira el contenido: los primeros 4 KB; si decodifican, se lee hasta `TEXT_MAX`.
- Produces (`sys`): `pub trait Preview: Send + Sync { fn request(&self, req: PreviewRequest); }`; `fn cloud_only(&self, path: &Path) -> bool { false }` en `FileSystem`.
- Produces (`thumbs.rs`): `pub fn big_side(px: u32) -> u32`; `pub struct BigCache<V> { … }` con `new(cap_bytes: usize)`, `get(&mut self, key: &Key) -> Option<&V>` (marca uso), `insert(&mut self, key: Key, v: V, bytes: usize)` (expulsa los menos usados hasta entrar), `best_for(&mut self, src: &Source) -> Option<&V>` (el escalón más grande que haya), `pub const BIG_CACHE_BYTES: usize = 64 << 20`.
- Produces (`paint.rs`): `fn big_image(&self, key: &thumbs::Key, r: Rect) -> bool` (centrada sin deformar; si no está el escalón pide y dibuja `best_for` escalado; false si no hay nada); `FontId::Mono`.
- Plataformas: `thumbs::request_big(hwnd, key)` / `take_done_big()` en Windows (misma cola del hilo, `SIIGBF_RESIZETOFIT | SIIGBF_BIGGERSIZEOK`, más `SIIGBF_INCACHEONLY` si `cloud_only`) y macOS (`QLThumbnailGenerator`; si `cloud_only`, termina con `None` sin pedir). `metadata::cloud_only` pasa a `pub(crate)` y lo usan `FileSystem::cloud_only` y el servicio. `Preview` de Windows/macOS: un hilo por pedido (o trabajador reusado) que llama `preview::answer(req, &cloud_only)`; Linux: `answer(req, &|_| false)`.
- `FakePreview`: `serve(path, info)`, `take_requests() -> Vec<PreviewTarget>`, `set_delay(Option<Duration>)`; sin respuesta cargada usa `answer` real. `RecordingPainter::big_image` registra la clave (`big_requests()`) y devuelve lo fijado con `set_big_ready(bool)`.

- [ ] **Step 1: Tests que fallan**: `big_side_steps` (100→256, 256→256, 257→512, 900→1024, 1500→2048, 5000→2048), `big_cache_evicts_by_bytes` (tope 100 bytes, 3 inserciones de 40 → la menos usada sale; `get` renueva), `big_cache_best_for_largest_step`, `decode_text_utf8_and_cut` (`"a\r\nb"` → `["a","b"]`; «ñ» partida al final con `cut` → válido; sin `cut` → `None`; bytes con NUL → `None`), `text_ext_list` (`RS`, `toml` sí; `svg`, `jpg` no), `preview_text_file` (archivo temporal de 70 KB de texto → `Text` con las líneas de los primeros 64 KB), `preview_sniffs_unknown_ext` (`LICENSE` sin extensión → `Text`; un PNG → `Image`), `preview_folder_counts_items` (3 archivos + 1 carpeta → `Folder { items: 4 }`), `preview_drive_space` (`FakeDiskScan::set_volume(Some((100, 40)), true)` → `Drive { total: 100, free: 40 }`), `preview_cloud_only_not_read` (`cloud_only` siempre true sobre un archivo que no existe → `CloudOnly`, sin error), `preview_unreadable` (ruta inexistente → `Unreadable`), `preview_stale_request_not_sent` (`current` distinto de `generation` → nada en el canal), `font_ids_are_distinct` con 11.
- [ ] **Step 2:** `cargo test -p xdir-core preview thumbs paint` → FAIL (tipos no definidos).
- [ ] **Step 3:** Implementar núcleo, falsos y servicios de las 3 plataformas (más `FontId::Mono` en las tablas de fuentes: `crates/xdir-windows/src/render/win32.rs` `Fonts::new` con «Consolas» 11,5; `crates/xdir-mac/src/render/macos.rs` `Fonts::new` con Menlo 11,5).
- [ ] **Step 4:** PASS; tests de plataforma: macOS `mac_big_image_png` (PNG temporal → `request_big` termina con píxeles), Windows `windows_big_image_png` y `cloud_big_image_cache_only` (en CI de Windows: ruta local no cloud → flags sin `INCACHEONLY`; probar la función pura `big_flags(cloud: bool)`); Global Constraints.
- [ ] **Step 5: Commit** `feat(core): servicio de vista previa e imagen grande`.

### Task 2: Columna de vista previa

**Files:**
- Create: `crates/xdir-core/src/preview/draw.rs` (`draw`, `draw_details`), `crates/xdir-core/src/app/preview.rs` (estado de la columna en la `App`).
- Modify: `ui.rs` (`panes_area` con ancho a la derecha, `preview_column_rect`, `preview_rect`, `draw_preview_toggle`, `StatusButtons { preview_hot, preview_on }`, `groups_rect` a 10 de `preview_rect`, `MIN_PANE_W`), `app/{mod,input,events,paint,actions,panes,tooltip}.rs`, `list/meta.rs` (`want_meta`), `tab.rs` (`request_metadata` incluye lo pedido por la columna), `xtree/view.rs` (`size_of`), `session.rs`, `commands.rs`, `crates/xdir-mac/src/sys/macos.rs` (`KEYS`).

**Interfaces:**
- Consumes: Task 1 completa.
- Produces:
  - `ui::MIN_PANE_W: f32 = 280.0`; `ui::PREVIEW_W: f32 = 320.0`, `PREVIEW_MIN_W = 240.0`, `PREVIEW_MAX_W = 640.0`; `pub fn panes_area(w, h, left, top, right: f32) -> Rect` (todas las llamadas actuales pasan `self.preview_width()`); `pub fn preview_column_rect(w: f32, h: f32, top: f32, width: f32) -> Rect`; `pub fn clamp_preview_width(want: f32, w: f32, left: f32, panes_across: usize) -> f32`; `pub fn preview_rect(w: f32, h: f32) -> Rect` (30×22, 10 a la izquierda del primer layout); `pub fn draw_preview_toggle(p: &dyn Painter, r: Rect, on: bool, hot: bool)`.
  - `preview::draw(p: &dyn Painter, r: Rect, s: &Shown)` y `preview::draw_details(p: &dyn Painter, r: Rect, rows: &[(&'static str, String)], tags: &[String])`, con `pub enum Shown { Item { name, key: thumbs::Key, icon: i32, info: Option<PreviewInfo>, rows: Vec<(&'static str, String)>, tags: Vec<String> }, Many { count: usize, keys: Vec<thumbs::Key> /* hasta 3 */, bytes: u64, kind: Option<Arc<str>> }, Empty }` (`info: None` = cargando: ícono del tipo; al llegar, se reemplaza sin parpadeo).
  - `pub fn detail_rows(e: &Entry, meta: Option<&Meta>, accessed: u64, env: &dyn SystemEnv) -> Vec<(&'static str, String)>` (puro: Tipo, Tamaño, Creado, Modificado, Último acceso, y los de metadatos con valor; carpetas sin «Tamaño» salvo el de xtree).
  - `Xtree::size_of(&self, path: &Path) -> Option<u64>` (busca en `arena` por tramos; `None` si no está escaneada).
  - `FileList::want_meta(&mut self, idx: usize)` (entra en el próximo `take_meta_request` con `MetaField::ALL`).
  - `Command::TogglePreview` id `vista_previa`, nombre «Mostrar u ocultar la vista previa», `&["Alt+P"]`; macOS `KEYS`: `(Command::TogglePreview, &["Ctrl+Shift+P"])` y `(Command::Palette, &["Ctrl+Alt+P"])` (ver nota de conflicto en el PR).
  - Sesión: `preview_on: bool` (`vista_previa`, por defecto `false`), `preview_width: f32` (`vista_previa_ancho`, por defecto 320, recortado a 240–640 al leer).
  - `App`: `fn preview_subject(&self) -> Option<PreviewTarget>` (del último panel activo aunque el foco esté en el árbol: 0 seleccionados → carpeta abierta (`Location::Path`) o, en «Este equipo», el disco enfocado (`Drive`); 1 → el elemento; varios → sin pedido, `Shown::Many`); se pide solo si cambió el sujeto, con `generation` nueva; `on_app_message(WM_APP_PREVIEW)` descarta `generation` vieja. Con `xtree_open` la columna no se ve ni pide. `Drag::PreviewEdge` para el borde izquierdo; `Hit::PreviewToggle` con cartel `tooltip_for` «Vista previa (Alt+P)» / «Vista previa (⌥⌘P)» según `KeyStyle`.

- [ ] **Step 1: Tests que fallan**: `preview_toggle_rect_and_shift` (con w=1400: botón 30×22 a 10 del primer layout; `groups_rect` 40 más a la izquierda que antes), `toggle_preview_command_and_shortcuts` (`Alt+P` en la tabla; con entorno Mac `Ctrl+Shift+P` y la paleta `Ctrl+Alt+P`; sin choques en `Keymap`), `column_width_respects_pane_min` (ventana 1000 con 2 paneles lado a lado: pedir 640 → recortado para que cada panel quede ≥ 280; nunca < 240), `preview_session_roundtrip` (prendida y 410 → se guarda y vuelve; 9999 → 640), `preview_follows_active_pane` (App: dos paneles; seleccionar en el 1 → pedido de ese archivo; activar el 2 → pedido del suyo; foco en el árbol no cambia), `preview_stale_reply_ignored` (dos selecciones seguidas con `FakePreview::set_delay`; la primera respuesta llega después → se dibuja la segunda), `arrows_never_wait_for_preview` (con demora de 5 s, 20 ↓ vuelven enseguida y la lista avanzó 20), `preview_text_draws_mono_lines` (`RecordingPainter`: texto en `FontId::Mono`, primeras líneas en orden, área 288×300), `preview_image_uses_big_image` (clave con lado `big_side(thumb_px(288))`; 288×216), `preview_many_selected` («3 elementos seleccionados», tamaño total, tipo común; con tipos distintos sin tipo), `preview_nothing_selected_shows_folder` (detalles de la carpeta y «N elementos · tamaño» con `Xtree::size_of` cuando hay escaneo), `preview_computer_shows_drive` («Espacio libre», «Capacidad»), `preview_details_rows` (`detail_rows` de una imagen con `Meta` → Tipo, Tamaño, Creado, Modificado, Último acceso, Dimensiones «4032 × 3024»; sin `Meta` no aparecen rótulos vacíos), `preview_unreadable_note` («No se puede leer este archivo»), `preview_requests_meta_for_subject` (sin columnas de metadatos visibles, seleccionar → `FakeMetadata` recibe ese elemento), `preview_hidden_with_xtree`, `preview_edge_drag_resizes`.
- [ ] **Step 2:** `cargo test -p xdir-core preview` → FAIL.
- [ ] **Step 3:** Implementar (según X39–X41 y `preview::draw`, `preview::draw_details`, `ui::draw_preview_toggle`, `ui::draw_status`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: imagen, archivo `.rs`, carpeta, 3 seleccionados y archivo bloqueado (`chmod 000`); comparar con X39–X41 en oscuro y claro.
- [ ] **Step 5: Commit** `feat(core): columna de vista previa con detalles`.

### Task 3: Vista Galería

**Files:**
- Create: `crates/xdir-core/src/list/gallery.rs` (layout puro, dibujo y hit-test de la Galería).
- Modify: `list/mod.rs` (`View::Gallery`, `VIEWS`/`CYCLE`, teclado, rueda, clic, doble clic, `draw` despacha a `draw_gallery`), `commands.rs` (`Command::View(6)`), `crates/xdir-mac/src/sys/macos.rs` (`KEYS`), `folder_views.rs`/`session.rs` (solo tests del id nuevo).

**Interfaces:**
- Consumes: `Painter::big_image`, `thumbs::big_side` (Task 1).
- Produces:
  - `View::Gallery` en `VIEWS[6] = (View::Gallery, "Galería", "galeria", 72.0)`; `CYCLE` de 7 con `Gallery` después de `Thumbnails`; `is_grid()` false, `has_header()` false.
  - `pub const STRIP_H: f32 = 112.0; CELL: f32 = 88.0; CELL_STEP: f32 = 96.0; STRIP_THUMB: f32 = 72.0; GALLERY_PAD: f32 = 16.0`.
  - `pub fn strip_offset(count: usize, focus: usize, width: f32) -> f32` (centra el enfocado; tope en los extremos; si todo entra, centrado el conjunto).
  - `FileList::draw_gallery(&mut self, p: &dyn Painter, area: Rect, active: bool)`, `FileList::draw_strip_cell(&self, p: &dyn Painter, r: Rect, pos: usize, state: CellState)` con `pub enum CellState { Normal, Hover, Selected, SelectedFocus }` y casilla de `draw_checkbox` si `checkboxes` y (hover o seleccionada).
  - Imagen grande: `big_image` con clave del enfocado; carpetas y sin imagen: ícono de 96 centrado (en la tira, ícono de 48). La Galería no dibuja texto de archivos (para eso está la columna).
  - Teclado en Galería: ←/↑ anterior, →/↓ siguiente, Inicio/Fin, Enter abre, Shift/Ctrl como en la lista; con grupos, se recorre `visible` sin encabezados (no pasa por `on_key_grouped`). Rueda sobre la tira: anterior/siguiente. Clic en celda enfoca (Ctrl/Shift como en la lista); doble clic abre.
  - `Command::View(6)` id `vista_galeria`, «Vista: galería», `&["Ctrl+Shift+7"]`; macOS `(Command::View(6), &["Ctrl+Alt+7"])`.

- [ ] **Step 1: Tests que fallan**: `gallery_view_index_and_cycle` (`View::Gallery.index() == 6`; `CYCLE` = Detalles, Lista, Contenido, Íconos, Íconos grandes, Miniaturas, Galería; `from_id("galeria")`), `gallery_shortcuts` (`Ctrl+Shift+7` → `View(6)`; entorno Mac `Ctrl+Alt+7`), `view_ids_roundtrip_session_and_folders` con Galería, `strip_offset_centers_focus` (30 elementos, ancho 960, foco 15 → la celda 15 centrada; foco 0 → offset 0; foco 29 → última celda pegada al borde; 5 elementos → conjunto centrado), `gallery_keys_move_one` (→ y ↓ avanzan uno; ← y ↑ retroceden; Shift+→ extiende), `gallery_skips_group_headers` (agrupada por tipo con un grupo plegado: la tira solo tiene los visibles en orden; → no se detiene en encabezados), `gallery_filter_applies`, `gallery_click_focuses_ctrl_adds` (clic en la celda 3 → foco 3; Ctrl+clic en la 5 → 2 seleccionados), `gallery_double_click_opens_folder` (`Action::Open`), `gallery_checkbox_in_strip` (con casillas: clic en la casilla de una celda la suma), `gallery_draws_big_image_of_focus` (`RecordingPainter::big_requests` con la ruta enfocada), `gallery_folder_uses_icon`.
- [ ] **Step 2:** `cargo test -p xdir-core gallery` → FAIL.
- [ ] **Step 3:** Implementar (según X42, `list::FileList::draw_gallery`, `draw_strip_cell`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con una carpeta de fotos y una agrupada; comparar con X42.
- [ ] **Step 5: Commit** `feat(core): vista Galería`.

### Task 4: Vista rápida en el núcleo y en macOS

**Files:**
- Create: `crates/xdir-core/src/quicklook.rs` (estado, recorrido, barra de título, tarjeta de respaldo), `crates/xdir-mac/src/quicklook.rs` (`QLPreviewPanel`).
- Modify: `host.rs` (métodos de vista rápida), `sys/testing.rs` (`FakeHost` los registra), `commands.rs` (`Command::QuickLook`), `list/mod.rs` (`typeahead_active`, `header_focused`), `app/{mod,actions,events,fileops,paint,input}.rs`, `crates/xdir-mac/src/{main.rs,app/mac.rs,app/host_mac.rs}`, `crates/xdir-mac/Cargo.toml` (`objc2-quick-look-ui`), `crates/xdir-mac/src/sys/macos.rs` (`FileSystem::cloud_only`), `crates/xdir-windows/src/app/host_win.rs` (métodos con la implementación por defecto hasta la Task 5).

**Interfaces:**
- Consumes: `Painter::big_image`, `FileSystem::cloud_only` (Task 1).
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub struct QuickLookItem { pub path: Option<PathBuf>, pub pidl: Option<Arc<[u8]>>, pub name: String, pub icon: i32, pub index: usize /* 0-based */, pub total: usize, pub cloud_only: bool, pub key: thumbs::Key /* imagen grande */ }`.
  - `WindowHost`: `fn quick_look_show(&self, item: &QuickLookItem) -> bool { false }` (true: lo muestra el sistema/ventana propia; false: la `App` dibuja la tarjeta encima de los paneles), `fn quick_look_close(&self) {}`. La plataforma avisa con `App::on_quick_look_closed(&mut self)` (el usuario cerró la ventana o el panel) y le pasa las teclas a `App::on_key`.
  - `quicklook::QuickLook { pane: usize, names: Vec<OsString> /* recorrido */, cur: usize, native: bool }` con `pub fn open(list: &FileList, pane: usize) -> Option<QuickLook>` (seleccionados > 1 → solo esos en orden visible; si no, toda la carpeta visible; empieza en el enfocado), `pub fn step(&mut self, delta: isize) -> bool` (sin dar la vuelta), `pub fn resync(&mut self, list: &FileList) -> bool` (por nombre; si el actual desapareció, el que quedó en su posición; `false` si no queda ninguno → cerrar), `pub fn item(&self, tab: &Tab, painter_px: &dyn Fn(f32) -> u32) -> Option<QuickLookItem>`.
  - `pub fn draw_title_bar(p: &dyn Painter, r: Rect, item: &QuickLookItem, hover: Option<TitleHit>)`, `pub fn title_hit(r: Rect, x: f32, y: f32) -> Option<TitleHit>` con `pub enum TitleHit { Prev, Next, Open, Close, Drag }` (barra de 40: ícono, nombre, ‹ «k de n» ›, «Abrir», cerrar); `pub fn draw_fallback(p: &dyn Painter, r: Rect, item: &QuickLookItem, viewer_missing: bool, hover: Option<FallbackHit>)` (imagen grande; con `cloud_only`, ícono y «Disponible solo en línea»; con `viewer_missing`, «No hay un visor para este tipo de archivo» y botones «Abrir con…» y «Abrir»), `pub fn fallback_hit(r: Rect, viewer_missing: bool, x: f32, y: f32) -> Option<FallbackHit>` con `pub enum FallbackHit { OpenWith, Open }`; `pub fn overlay_rect(w: f32, h: f32) -> Rect` (960×680 centrado, recortado al área, mínimo 480×360).
  - `Command::QuickLook` id `vista_rapida`, «Vista rápida», `&["Espacio"]`; entrada propia «Vista rápida» primera en el menú de clic derecho de archivos (`own` en `app/fileops.rs`).
  - `FileList::typeahead_active(&self) -> bool` (hay texto y pasaron < 900 ms), `FileList::header_focused(&self) -> bool`.
  - `App::on_key`, antes de la tabla de atajos: con vista rápida abierta, Espacio/Esc cierran y ←/↑ / →/↓ recorren (la lista acompaña: `select_only` si recorre la carpeta, solo foco si recorre la selección); cerrada, Espacio sin modificadores → `false` si `typeahead_active`, a la lista si `header_focused`, `false` si no hay selección, y si no `Command::QuickLook`. Si Espacio se usó, el `' '` que llega después a `on_char` se descarta (`App::swallow_space: bool`). Tras cada carga o cambio de la carpeta del panel, `resync`.
  - Con `host.quick_look_show` = false o `cloud_only`, la `App` dibuja en `paint` la tarjeta (`overlay_rect`, `draw_title_bar` + `draw_fallback`) y resuelve sus clics («Abrir» → `sys::shell().open_item`; «Abrir con…» → `Shell::invoke_verb(…, "openas")`).
  - macOS `quicklook.rs`: `pub fn show(item: &QuickLookItem) -> bool` (`cloud_only` → `false`), `pub fn close()`; data source/delegate con un solo elemento (el actual: `numberOfPreviewItems` = 1, `reloadData` al cambiar) para que el orden y los saltos los decida xdir; `previewPanel:handleEvent:` manda keyDown de Espacio, Esc y flechas a `App::on_key`; `acceptsPreviewPanelControl:`/`begin…`/`end…` en la vista de `app/mac.rs`; `windowWillClose` → `on_quick_look_closed`.

- [ ] **Step 1: Tests que fallan**: `quick_look_command_and_menu` (id, título, «Espacio» en la paleta; menú de clic derecho de un archivo con «Vista rápida» primero y elegirlo abre), `space_opens_quick_look` (App con `FakeHost`: un archivo seleccionado, Espacio → `FakeHost` registró `quick_look_show` con ese nombre y «1 de n»), `space_during_typeahead_types` (`on_char('a')` y enseguida Espacio → no abre; `on_key` devuelve false), `space_on_group_header_folds` (sin abrir), `ctrl_space_unchanged`, `space_char_swallowed_after_open` (el `' '` siguiente no mueve el foco), `esc_and_space_close`, `quick_look_arrows_walk_visible_order` (agrupada y filtrada: → salta encabezados y ocultos; la lista acompaña con `select_only`), `quick_look_walks_selection_only` (3 seleccionados no contiguos: → recorre esos 3, la selección no cambia y el foco acompaña; no da la vuelta al final), `quick_look_item_deleted_moves_next` (recarga sin el abierto → muestra el que quedó en su lugar), `quick_look_folder_emptied_closes`, `quick_look_cloud_item_uses_card` (`FakeFileSystem::set_cloud_only` → `quick_look_show` no se llama; la `App` dibuja «Disponible solo en línea»), `quick_look_overlay_when_host_has_none` (`FakeHost` devuelve false: dibuja `draw_title_bar` con «2 de 12» y la imagen grande; clic en «Abrir» → `ShellCall` de abrir), `title_bar_layout` (`RecordingPainter`: nombre, «2 de 12», «Abrir» en 960 de ancho), `fallback_texts` (sin visor: «No hay un visor para este tipo de archivo», «Abrir con…», «Abrir»), `overlay_rect_clamps` (ventana 800×500 → 480×360 mínimo dentro del área).
- [ ] **Step 2:** `cargo test -p xdir-core quick_look` → FAIL.
- [ ] **Step 3:** Implementar núcleo y macOS.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac (copia aislada): Espacio sobre un PDF de varias páginas, un video y un `.docx` → panel del sistema; flechas pasan y la lista acompaña; Esc cierra; búsqueda por tipeo con espacio («mi a») no abre; elemento de iCloud con `brctl evict` → tarjeta «Disponible solo en línea» sin descargar (`ls -lO` sigue con `dataless`).
- [ ] **Step 5: Commit** `feat: vista rápida con la barra espaciadora (núcleo y macOS)`.

### Task 5: Vista rápida en Windows

**Files:**
- Create: `crates/xdir-windows/src/quicklook/mod.rs` (`show`/`close`, estado del hilo de interfaz), `crates/xdir-windows/src/quicklook/window.rs` (ventana, dibujo con un `Renderer` propio y las funciones de `xdir_core::quicklook`), `crates/xdir-windows/src/quicklook/handler.rs` (búsqueda del handler, hilo STA, `IPreviewHandlerFrame`).
- Modify: `crates/xdir-windows/src/main.rs`, `app/host_win.rs` (`quick_look_show`/`quick_look_close`), `app/messages.rs` (avisos de la ventana a la `App`), `sys/win32.rs` (`FileSystem::cloud_only`), `Cargo.toml` (features que falten).

**Interfaces:**
- Consumes: `QuickLookItem`, `draw_title_bar`, `title_hit`, `draw_fallback`, `fallback_hit` (Task 4); `Painter::big_image` (Task 1).
- Produces:
  - Ventana `xdir.quicklook`: `WS_POPUP | WS_THICKFRAME`, dueña la principal (se cierra con ella), 960×680 centrada sobre xdir, `WM_GETMINMAXINFO` 480×360, `WM_NCHITTEST` → `HTCAPTION` en `TitleHit::Drag` y bordes para redimensionar, esquinas redondeadas DWM; barra de 40 dibujada por el núcleo; el visor ocupa el resto.
  - `handler::viewer_for(ext: &str) -> Option<GUID>` (`AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, ext, "{8895b1c6-b41f-4c1c-a562-0d564250836f}")`; `None` si `HKCR\CLSID\{clsid}` tiene `DisableProcessIsolation` = 1). Función pura para el registro: `fn isolation_disabled(value: Option<u32>) -> bool`.
  - Hilo STA por visor: crea con `CoCreateInstance(clsid, None, CLSCTX_LOCAL_SERVER)`, inicializa con `IInitializeWithStream` (`SHCreateStreamOnFileEx` de solo lectura) o `IInitializeWithFile`/`IInitializeWithItem`, `IObjectWithSite::SetSite` con el frame, `SetWindow` + `SetRect` + `DoPreview`; avisa a la ventana con un mensaje propio. Si no avisa en `HANDLER_TIMEOUT = 3 s` o falla, la ventana pasa a `draw_fallback` con `viewer_missing = true` y abandona ese hilo (el siguiente elemento usa uno nuevo). Al pasar de elemento: `Unload` en el hilo del visor.
  - `IPreviewHandlerFrame` (`#[implement]`): `TranslateAccelerator` devuelve `S_OK` y reenvía a la ventana Espacio, Esc y flechas (las demás `S_FALSE`); `GetWindowContext` sin aceleradores.
  - Teclas en la ventana (`WM_KEYDOWN`) → `App::on_key`; botones de la barra y del respaldo → `App` («Abrir» → `open_item`; «Abrir con…» → `SHOpenWithDialog`); cerrar → `App::on_quick_look_closed`. Archivos `cloud_only`: no se crea visor; `draw_fallback` con la imagen de caché o el ícono.
- [ ] **Step 1: Tests que fallan** (corren en la CI de Windows): `in_process_handler_is_no_viewer` (`isolation_disabled(Some(1))` true; `None`/`Some(0)` false), `viewer_for_txt_exists_or_skip` (en una máquina con el visor de texto, `.txt` da un CLSID; si no, se indica y se saltea), `viewer_for_unknown_ext_none` (`.xdirnada` → `None`), `handler_timeout_falls_back` (con un visor falso inyectado que no responde — `handler::run_with(timeout, f)` probado con un cierre que duerme 5 s y timeout de 100 ms → `Timeout` y el hilo de prueba sigue), `frame_forwards_keys` (`forwarded(vk)` puro: Espacio, Esc y las cuatro flechas sí; Tab y letras no).
- [ ] **Step 2:** `cargo test -p xdir-windows quicklook` en la CI de Windows (o `cargo check --target x86_64-pc-windows-msvc --all-targets` local) → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en CI; `cargo check` local para Windows (también `--no-default-features`) sin warnings; Global Constraints. Prueba manual en la lista del PR para Runick93 (PDF, `.docx`, `.txt`, video sin visor con «Abrir», archivo de OneDrive solo en línea, matar `prevhost.exe` con la ventana abierta).
- [ ] **Step 5: Commit** `feat(windows): vista rápida con el visor del panel de vista previa`.

### Task 6: Atajos en la wiki, PRs y versión de Mac

**Files:**
- Modify: wiki del repo en GitHub (clonar el `.wiki.git` del remoto `origin` en el scratchpad; página de atajos), `docs/DISENO.md` (X39–X44 pasan de «sin programar» a programadas).

- [ ] **Step 1:** Wiki: Alt+P / ⌥⌘P (vista previa), Ctrl+Shift+7 / ⌥⌘7 (Galería), Espacio (vista rápida; Esc o Espacio cierran; flechas pasan); más los pendientes de la tanda A y B si faltan (Ctrl+Shift+5/6, ⌥⌘5/6, Ctrl+Alt+N). Commit en la wiki `docs: atajos de la tanda C`.
- [ ] **Step 2:** `docs/DISENO.md` actualizado; commit `docs: pantallas de la tanda C programadas`.
- [ ] **Step 3:** Verificación final: Mac, Docker, checks de los 3 sistemas, capturas de X39–X42 comparadas con el diseño.
- [ ] **Step 4:** Al mergear la última parte: `develop` actualizado, `sh scripts/package-mac.sh` y reemplazar `/Applications/xdir.app` (avisar si xdir está abierto).

## PRs

Rama `feature/tanda-c`, base `develop`; después de cada merge se trae `develop` y se sigue en la misma rama.

1. **Tasks 1–2** — columna de vista previa (`feat: columna de vista previa`). Lista de pruebas para Windows: imagen, texto, carpeta, varios, «Este equipo», OneDrive solo en línea, archivo bloqueado, ancho y Alt+P.
2. **Task 3** — Galería (depende de 1 por `big_image`).
3. **Task 4** — vista rápida núcleo + macOS (depende de 1; en Windows queda la tarjeta del núcleo hasta el PR 4).
4. **Task 5 + Task 6** — vista rápida de Windows, wiki y versión de Mac.

La paleta sigue en ⇧⌘P en macOS (sin cambios); la columna usa ⌥⌘P en macOS para no chocar.

# Funciones del Explorador, tanda B (organizar la lista) — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Memoria de vista por carpeta, agrupar, filtros por columna, vistas Lista y Contenido, casillas de selección y columnas de metadatos, en las 3 plataformas.

**Architecture:** Todo el modelo y el dibujo en `xdir-core` (`list.rs` crece con submódulos para grupos, filtros y vistas; un módulo nuevo para la memoria por carpeta); un servicio nuevo `sys::Metadata` para leer metadatos en segundo plano, implementado en Windows (`IPropertyStore`) y macOS (Spotlight), vacío en Linux. El layout de cada pieza nueva sigue las pantallas X32–X37 y los componentes «Tanda B: organizar la lista» de `docs/design/xdir.figcraft` (ver `docs/DISENO.md`).

**Tech Stack:** Rust 2024, workspace `xdir-core` + `xdir-windows` + `xdir-mac` + `xdir-linux`; `windows` 0.62 (`Win32_UI_Shell_PropertiesSystem`), `objc2` 0.6 (Core Services / Spotlight `MDItem` vía `objc2-core-services` u otra crate `objc2-*` del mismo grupo de versiones si existe; si no, FFI mínima a `MDItemCreate`/`MDItemCopyAttribute`).

**Spec:** `docs/superpowers/specs/2026-11-09-explorador-tanda-b-design.md` (incluida la sección «Decisiones tomadas por quien diseña»)

## Global Constraints

- `xdir-core` sin dependencias ni `#[cfg]` de sistema en producción. Tests de `App` con `install_fakes()`, `FakeHost` y `RecordingPainter`.
- Nada bloqueante en el hilo de interfaz: metadatos en un hilo aparte (canal de la pestaña + `Waker`), como `Overlays`.
- `cargo test` en la Mac (hoy 216) y en Linux con Docker (`sh /private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/dk.sh cargo test`) sin bajar; `cargo check --all-targets` sin errores ni warnings nuevos para la Mac, `x86_64-pc-windows-msvc` (también `--no-default-features`) y `x86_64-unknown-linux-gnu`; CI verde (prueba de humo de Windows incluida).
- Textos visibles exactos de la spec (grupos, «Agrupar por», «(Ninguno)», «Quitar el filtro», «Quitar los filtros», «(filtrado)», «Usar esta vista en todas las carpetas», confirmación «¿Usar esta vista en todas las carpetas? Se pierden las vistas guardadas de cada carpeta.», «Restablecer la vista de esta carpeta», «Lista», «Contenido», «Casillas de selección», «Leyendo…», «Leyendo datos…», nombres de columnas «Dimensiones», «Duración», «Autor», «Etiquetas», «Artista», «Álbum»).
- Atajos nuevos: `Ctrl+Shift+5` (Lista), `Ctrl+Shift+6` (Contenido); en macOS los equivalentes en la tabla `KEYS` de `crates/xdir-mac/src/sys/macos.rs`. Las tablas de atajos `MAC_TABLE` / `WINDOWS_TABLE` se actualizan con cada comando nuevo.
- Compatibilidad: sin `carpetas.toml`, la vista y columnas de la sesión de hoy se copian como vista por defecto; la vista por defecto de fábrica es la de hoy (Detalles, por nombre, sin agrupar, columnas de siempre). Una `carpetas.toml` corrupta no impide arrancar (se ignora la entrada o el archivo y se anota en `notice`).
- Layout según FigCraft: en cada tarea de interfaz, leer el componente y la pantalla correspondientes del archivo de diseño (descomprimir `docs/design/xdir.figcraft` en una carpeta temporal y buscar los nodos por nombre) y usar sus medidas, colores (variables Tema) y fuentes (`FontId`).
- Smoke en la Mac al final de cada tarea de interfaz: `cargo build --release -p xdir-mac --features pruebas`, copia aislada con `xdir.portable` en `/private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/vis/<tarea>/` y `vis/run.sh`, más capturas de la función nueva con comandos `XDIR_PRUEBAS`; nunca tocar `/Applications/xdir.app` en las pruebas; nunca `osascript quit`.
- No `cargo fmt` del repo. Castellano; Conventional Commits sin atribución a Claude. Un PR: rama `feature/tanda-b`, base `develop`; al mergear, versión de Mac instalada en `/Applications`.

## Review Focus

- **Carpeta grande (10 000 elementos) agrupada o filtrada**: la lista sigue fluida; agrupar/filtrar no recalcula todo en cada cuadro → test de que el modelo de filas se recalcula solo al cambiar datos, orden, grupos o filtros (contador en el test), Task 2.
- **`carpetas.toml` roto o enorme**: línea inválida, archivo truncado, 5000 carpetas → arranca, ignora lo malo con aviso, recorta a 2000 → tests en Task 1.
- **Selección y teclado con grupos plegados y filtros**: el foco nunca queda en un elemento oculto; ↑/↓, Inicio/Fin, Shift+rango y Ctrl+A solo sobre lo visible; al plegar el grupo del foco, el foco pasa al encabezado → tests en Task 2 y 3.
- **Navegar entre carpetas con vistas distintas**: al volver con Atrás, la carpeta recupera su vista y su posición de desplazamiento y selección como hoy; cambiar de vista no pierde la selección → test en Task 1.
- **Metadatos que llegan tarde o para una carpeta vieja**: un lote de metadatos de una carga anterior (otra `generation`) se descarta; la lista no salta ni cambia el foco al llegar → test en Task 6.

---

### Task 1: Memoria de vista por carpeta

**Files:**
- Create: `crates/xdir-core/src/folder_views.rs` (modelo, claves, `carpetas.toml`, LRU).
- Modify: `crates/xdir-core/src/lib.rs`, `list.rs` (aplicar y leer `ViewSettings`), `tab.rs` (al terminar de navegar, aplicar la vista de la carpeta), `session.rs` (migración), `commands.rs` (`Command::ApplyViewToAll` id `vista_en_todas`, `Command::ResetFolderView` id `restablecer_vista`), `app/{actions,panes,mod}.rs`, tablas de atajos de Mac y Windows (sin atajo por defecto para estos dos).

**Interfaces:**
- Produces (en `folder_views.rs`):
  - `#[derive(Clone, Debug, PartialEq)] pub struct ViewSettings { pub view: View, pub sort: Column, pub descending: bool, pub columns: Vec<ColumnSpec>, pub group_by: Option<Column>, pub collapsed: Vec<String>, pub filters: Vec<(Column, Vec<String>)> }` (`group_by`, `collapsed` y `filters` quedan en `None`/vacío hasta las Tasks 2–3, que los usan; `Default` = vista de fábrica).
  - `pub fn folder_key(loc: &Location, search_root: Option<&Path>) -> String` (ruta normalizada con barra final quitada y en minúsculas en Windows/macOS según `sys::file_system().same_path`-equivalente — usar una normalización pura que dé la misma clave para rutas que `same_path` considera iguales; `equipo`; `shell:<parsing>`; `busqueda:<ruta>`).
  - `pub struct FolderViews { … }` con `pub fn load() -> FolderViews` (lee `carpetas.toml` de `session::config_dir()`), `pub fn get(&self, key: &str) -> ViewSettings` (la de la carpeta o la por defecto), `pub fn set(&mut self, key: &str, v: ViewSettings)`, `pub fn default_view(&self) -> &ViewSettings`, `pub fn apply_to_all(&mut self, v: ViewSettings)`, `pub fn reset(&mut self, key: &str)`, `pub fn save_if_dirty(&mut self)`; LRU de 2000 entradas por último uso.
  - `pub fn parse(text: &str) -> (FolderViews, Vec<String>)` y `pub fn write(&FolderViews) -> String` puras (errores por línea).
- `App` guarda un `FolderViews`; cada cambio de vista/orden/columnas/anchos en una lista llama `set` para la clave de la carpeta del panel y marca para guardar (diferido 1 s con el temporizador existente o al cerrar).

- [ ] **Step 1: Tests que fallan**: `folder_views_roundtrip` (2 carpetas + por defecto con columnas y anchos → `write` → `parse` igual), `folder_views_ignores_bad_lines` (una línea inválida y una entrada con columna desconocida → se ignoran con error, el resto carga), `folder_views_truncated_file_loads` (texto cortado a la mitad → carga lo que pudo, sin pánico), `folder_views_lru_caps_at_2000` (5000 `set` → quedan 2000, las últimas usadas), `folder_key_same_for_equal_paths` (`C:\Users\A` y `c:\users\a\` en Windows-like; en el núcleo probar con `FakeFileSystem` que compara como el falso), `migration_copies_session_view` (sin `carpetas.toml`, sesión con vista Íconos y columnas no de fábrica → la por defecto queda igual a eso), `navigating_restores_folder_view` (App: carpeta A en Detalles, cambiar B a Íconos, ir a A → Detalles; Atrás a B → Íconos; selección y desplazamiento de A como hoy al volver), `apply_view_to_all_confirms_and_clears` (con `FakeHost::set_answer(Some(true))` → todas las entradas borradas y la por defecto = la actual; con `Some(false)` → nada), `reset_folder_view_uses_default`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): cada carpeta recuerda su vista`.

### Task 2: Agrupar

**Files:**
- Create: `crates/xdir-core/src/list/groups.rs` (si `list.rs` no es módulo con carpeta, convertirlo a `list/mod.rs` en un commit previo de solo movimiento) — claves y nombres de grupos.
- Modify: `list.rs` (modelo de filas con encabezados, plegado, teclado, dibujo `draw_group_header` en todas las vistas), `commands.rs` (`Command::GroupBy(u8)` con ids `agrupar_nombre`, `agrupar_modificado`, `agrupar_creado`, `agrupar_tipo`, `agrupar_tamano`, `no_agrupar` y títulos «Agrupar por nombre», …, «No agrupar»), `app/fileops.rs` (submenú «Agrupar por» en el menú del fondo, con «(Ninguno)»), `folder_views.rs` (usa `group_by` y `collapsed`), tablas de atajos.

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)] pub struct GroupKey { pub order: i64, pub label: String }`.
  - `pub fn group_key(col: Column, e: &Entry, meta: Option<&Meta>, now_local: LocalNow) -> GroupKey` (pura; `LocalNow { day: i64 /* días desde 1970 en hora local */, weekday: u8 /* 0 = lunes */, month: u8, year: i32 }` armado por la `App` con `sys::env()`; `Meta` es de la Task 6 — en esta tarea el parámetro existe y las columnas de metadatos todavía no se agrupan) con los grupos y textos exactos de la spec (incluye «Sin fecha», «Carpetas», «Otros»).
  - `FileList::set_group(&mut self, col: Option<Column>)`, `FileList::toggle_group(&mut self, label: &str)`, `FileList::rows(&self) -> &[Row]` con `pub enum Row { Header { key: GroupKey, count: usize, collapsed: bool }, Item(usize) }` recalculado solo cuando cambian entradas, orden, grupo, plegado o filtros (guardar una marca de «sucio»).
  - `SystemEnv` gana `fn local_now(&self) -> LocalNow` con implementación por defecto que usa UTC (las plataformas la sobreescriben con hora local: Windows `GetLocalTime`/`SystemTimeToTzSpecificLocalTime`, macOS y Linux `localtime_r`). El falso devuelve una fecha fija configurable.

- [ ] **Step 1: Tests que fallan**: `group_key_dates` (con un `LocalNow` fijo un miércoles: archivos de hoy, ayer, el lunes, la semana pasada, este mes, el mes pasado, este año, hace dos años y fecha 0 → los 9 grupos en orden), `group_key_sizes` (0, 10 KB, 500 KB, 50 MB, 500 MB, 2 GB, 5 GB y una carpeta → los 8 grupos), `group_key_names` («1abc», «Álvaro», «ivan», «zeta», «_x» → 0–9, A–H, I–P, Q–Z, Otros), `group_key_types` (carpetas primero), `rows_group_and_collapse` (8 elementos en 3 grupos; plegar el del medio → sus filas no aparecen y el encabezado muestra la cuenta), `keyboard_skips_collapsed` (foco en el último del grupo 1, ↓ con el grupo 2 plegado → foco en el encabezado del 2; → lo despliega; ↓ entra), `ctrl_a_selects_visible_only`, `fold_focused_group_moves_focus_to_header`, `rows_not_recomputed_each_frame` (dibujar 10 cuadros sin cambios → el contador de recálculos no sube), `group_menu_in_background_menu` (App: menú del fondo con «Agrupar por» y sus 6 entradas; elegir «Tipo» → la lista queda agrupada y la vista de la carpeta lo recuerda), `group_header_drawn_like_design` (con `RecordingPainter`: el texto «Hoy (3)» con la fuente del diseño).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar (dibujo según `list::FileList::draw_group_header` de FigCraft).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con una carpeta temporal de archivos con fechas variadas (`touch -t`), agrupada por fecha y con un grupo plegado; comparar a ojo con X32.
- [ ] **Step 5: Commit** `feat(core): agrupar la lista por nombre, fecha, tipo y tamaño`.

### Task 3: Filtros por columna

**Files:**
- Create: `crates/xdir-core/src/list/filters.rs`.
- Modify: `list.rs` (flecha del encabezado al pasar el mouse, desplegable `draw_filter_menu` con casillas y cuentas, embudo, aplicación en el orden de la spec), `tab.rs` (`status()` agrega « (filtrado)»), `commands.rs` (`Command::ClearFilters` id `quitar_filtros`, título «Quitar los filtros»), `app/{input,events}.rs` (clics del desplegable, cerrar con Esc o clic afuera), `folder_views.rs` (`filters`), tablas de atajos.

**Interfaces:**
- Consumes: `group_key` de la Task 2.
- Produces: `FileList::filter_options(&self, col: Column) -> Vec<(GroupKey, usize, bool /* tildado */)>`, `FileList::set_filter(&mut self, col: Column, labels: Vec<String>)` (vacío = quitar), `FileList::is_filtered(&self) -> bool`; hit-test `ListHit::HeaderFilter(Column)` y `ListHit::FilterOption(usize)` / `FilterClear`.

- [ ] **Step 1: Tests que fallan**: `filter_options_counts` (carpeta con 4 carpetas, 12 jpg, 3 png, 2 pdf → opciones de Tipo con esas cuentas), `filter_keeps_any_checked` (tildar JPEG y PNG → 15 visibles), `filters_combine_across_columns` (Tipo=JPEG y Fecha=Hoy → solo los JPEG de hoy), `filter_then_quick_filter` (filtro de columna + filtro rápido «2036» → 1), `status_says_filtered` («(filtrado)» al final solo con filtros de columna), `clear_filters_command`, `filter_hides_focused_moves_focus` (el foco estaba en un PDF y se filtra → el foco pasa al primer visible), `filter_dropdown_click_flow` (App con `FakeHost`: pasar el mouse por el encabezado «Tipo», clic en la flecha, clic en «Imagen JPEG» → filtrado; Esc cierra; la carpeta recuerda el filtro al volver).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar (según X33 y `draw_filter_menu`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con el desplegable abierto; comparar con X33.
- [ ] **Step 5: Commit** `feat(core): filtros por columna en el encabezado`.

### Task 4: Vistas Lista y Contenido

**Files:**
- Modify: `list.rs` (`View::List` id `lista`, `View::Content` id `contenido`; layout, dibujo `draw_list_cell` / `draw_content_row`, hit-test, teclado: en Lista ←/→ cambian de columna y ↑/↓ de fila, desplazamiento horizontal con la rueda vertical), `commands.rs` (`Command::View(4)` «Vista: lista», `Command::View(5)` «Vista: contenido», ids `vista_lista` / `vista_contenido`, atajos `Ctrl+Shift+5` / `Ctrl+Shift+6`), `session.rs` y `folder_views.rs` (ids nuevos de vista), tablas de atajos.

- [ ] **Step 1: Tests que fallan**: `list_view_fills_columns_top_to_bottom` (30 elementos con alto de área para 10 filas → 3 columnas; el elemento 11 está arriba de la 2.ª columna), `list_view_keyboard` (→ desde el elemento 1 va al 11; ↓ al 2), `list_view_wheel_scrolls_horizontally`, `content_view_row_texts` (con `RecordingPainter`: nombre, tipo, fecha y tamaño de un archivo en las posiciones del diseño), `content_view_groups_headers` (agrupar en Contenido dibuja encabezados de fila completa), `view_shortcuts` (`Ctrl+Shift+5` y `+6` en la tabla), `view_ids_roundtrip_session_and_folders`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar (según X34, X35 y los componentes).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con las dos vistas; comparar con X34/X35.
- [ ] **Step 5: Commit** `feat(core): vistas Lista y Contenido`.

### Task 5: Casillas de selección

**Files:**
- Modify: `list.rs` (casilla a la izquierda del ícono si está el switch y la fila está bajo el mouse o seleccionada; clic en la casilla = Ctrl+clic; casilla del encabezado en Detalles con estado parcial; `draw_checkbox`), `ui.rs` (switch «Casillas de selección» en la rueda dentada, bajo «Mostrar extensiones»), `session.rs` (`checkboxes: bool`, clave `casillas`, por defecto `false`), `app/appearance.rs`.

- [ ] **Step 1: Tests que fallan**: `checkbox_click_toggles_only_that_item` (2 seleccionados, clic en la casilla de un tercero → 3 seleccionados), `header_checkbox_selects_all_visible_and_clears` (con un grupo plegado: selecciona solo lo visible; segundo clic quita todo), `header_checkbox_partial_state` (dibuja el estado parcial con 1 de 5), `checkboxes_switch_persists`, `checkboxes_hidden_when_off`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar (según X36 y `draw_checkbox`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac; comparar con X36.
- [ ] **Step 5: Commit** `feat(core): casillas de selección`.

### Task 6: Columnas de metadatos en el núcleo

**Files:**
- Modify: `crates/xdir-core/src/sys/{mod,testing}.rs` (trait `Metadata`, `FakeMetadata`), `fs.rs` (`LoadMsg::Metadata`), `list.rs` (columnas nuevas, caché por ruta y fecha, pedido de lotes, celdas, grupos «Pequeña/Mediana/Grande/Muy grande», «Muy corta…», «Sin valor», «Leyendo…»), `list/groups.rs`, `list/filters.rs`, `tab.rs`.

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct Meta { pub width: Option<u32>, pub height: Option<u32>, pub duration_ms: Option<u64>, pub authors: Vec<String>, pub tags: Vec<String>, pub artist: Option<String>, pub album: Option<String> }`.
  - `pub enum MetaField { Dimensions, Duration, Authors, Tags, Artist, Album }`.
  - `pub struct MetadataRequest { pub hwnd: Handle, pub tx: Sender<LoadEvent>, pub panel: usize, pub generation: u64, pub items: Vec<(usize, PathBuf)>, pub fields: Vec<MetaField> }`.
  - `pub trait Metadata: Send + Sync { fn request(&self, req: MetadataRequest); }` en `sys`, en `Services` como `metadata`, accesor `sys::metadata()`. Respuesta: `LoadMsg::Metadata(Vec<(usize, Meta)>)` en lotes, despertando con el mismo mensaje de carga.
  - `Column::{Dimensions, Duration, Authors, Tags, Artist, Album}` con ids de sesión `dimensiones`, `duracion`, `autor`, `etiquetas`, `artista`, `album` y anchos iniciales del diseño (X37).
  - Pedido: al mostrar o usar una columna de metadatos se piden los visibles (o toda la carpeta si agrupa o filtra por ella), en lotes de 200, sin repetir lo pedido; lotes de otra `generation` se descartan.
  - `FakeMetadata`: devuelve lo que el test le cargue por ruta, en el mismo hilo o con demora configurable.

- [ ] **Step 1: Tests que fallan**: `metadata_only_requested_when_column_visible` (sin columnas nuevas → `FakeMetadata` no recibe pedidos), `metadata_cells_fill_when_arriving` (con «Dimensiones» visible → pedido de los visibles; al llegar, la celda dice «4032 × 3024»), `duration_format` (`201000` ms → «0:03:21», `3723000` → «1:02:03»), `group_by_duration_requests_whole_folder_and_shows_reading` (antes de llegar, los elementos están en «Leyendo…»; después, en sus grupos), `stale_metadata_batch_ignored` (lote con `generation` vieja no cambia nada), `metadata_arrival_keeps_focus_and_scroll`, `tags_group_first_tag_and_filter_any`, `filter_dropdown_shows_reading_note`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints (Linux instala un `Metadata` vacío en `xdir-linux`; Windows y Mac, provisorio vacío hasta la Task 7 para que compilen).
- [ ] **Step 5: Commit** `feat(core): columnas de dimensiones, duración, autor, etiquetas, artista y álbum`.

### Task 7: Metadatos en Windows y macOS

**Files:**
- Create: `crates/xdir-windows/src/metadata.rs`, `crates/xdir-mac/src/metadata.rs`.
- Modify: `crates/xdir-{windows,mac}/src/sys/*.rs` (instalar el servicio), `Cargo.toml` de cada crate si hace falta una feature/crate (`windows`: `Win32_UI_Shell_PropertiesSystem`, `Win32_System_Com_StructuredStorage` ya está; macOS: la crate `objc2-*` de Core Services o FFI mínima), `crates/xdir-linux/src/sys.rs` (servicio vacío explícito, si no lo dejó la Task 6).

**Interfaces:**
- Windows: un hilo con COM inicializado (`CoInitializeEx` MTA) por pedido o un hilo trabajador reutilizado; por elemento `SHGetPropertyStoreFromParsingName(path, GPS_DEFAULT | GPS_FASTPROPERTIESONLY)` y las claves `PKEY_Image_HorizontalSize`/`VerticalSize` (o `PKEY_Video_FrameWidth/Height`), `PKEY_Media_Duration` (100 ns), `PKEY_Author`, `PKEY_Keywords`, `PKEY_Music_Artist`, `PKEY_Music_AlbumTitle`.
- macOS: `MDItemCreateWithURL` + `MDItemCopyAttribute` con `kMDItemPixelWidth/Height`, `kMDItemDurationSeconds`, `kMDItemAuthors`, `kMDItemUserTags` (o `NSURLTagNamesKey` del archivo, que no depende de la indexación), `kMDItemAuthors`/`kMDItemAlbum` para audio; si Spotlight no tiene el archivo, se devuelve `Meta::default()`.
- Respuestas en lotes de 50 o cada 100 ms.

- [ ] **Step 1: Tests que fallan**: macOS `mac_reads_png_dimensions` (crear un PNG de 3×2 en una carpeta temporal con bytes fijos; si Spotlight no lo tiene indexado, el test lo indica y se saltea, pero las etiquetas por `NSURLTagNamesKey` se prueban siempre: `mac_reads_finder_tags` escribiendo una etiqueta con `setResourceValue`), Windows `windows_reads_png_dimensions` (mismo PNG; corre en la CI de Windows), y en ambos `metadata_unknown_file_is_empty`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en la Mac; `cargo check` para Windows y Linux sin warnings; smoke en la Mac con una carpeta de imágenes y la columna «Dimensiones»; comparar con X37.
- [ ] **Step 5: Commit** `feat: metadatos de archivos en Windows y macOS`.

### Task 8: PR y versión de Mac

- [ ] **Step 1:** Verificación final (Mac, Docker, checks de los 3 sistemas, capturas de X32–X37 comparadas con el diseño).
- [ ] **Step 2:** PR (`feature/tanda-b`, base `develop`) con la lista de pruebas para Runick93 en Windows: agrupar en cada criterio y plegar; filtros por columna combinados; vistas Lista y Contenido; casillas; columnas Dimensiones/Duración/Autor/Etiquetas/Artista/Álbum en una carpeta de fotos y de música; que cada carpeta recuerde su vista al cerrar y abrir xdir; «Usar esta vista en todas las carpetas»; prueba de humo.
- [ ] **Step 3:** Al mergear: `develop` actualizado, `sh scripts/package-mac.sh` y reemplazar `/Applications/xdir.app` (avisar si xdir está abierto).

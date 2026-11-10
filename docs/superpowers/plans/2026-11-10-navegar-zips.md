# Navegar dentro de archivos comprimidos — plan de implementación

**Goal:** abrir un zip / tar.gz como carpeta de solo lectura en el mismo panel (migas, subir, Atrás/Adelante, pestaña nueva, sesión, búsqueda), abrir y previsualizar lo de adentro con una extracción temporal propia, sacar cosas con el motor de copia, y todo lo que escribe deshabilitado con el aviso «El contenido de un archivo comprimido es solo de lectura».

**Spec:** `docs/superpowers/specs/2026-11-10-navegar-zips-design.md`. Escribir adentro queda fuera (tanda futura).

**Architecture:** una variante nueva `Location::Archive { file, nested, inner }` (archivo del disco, cadena de archivos anidados como rutas interiores, carpeta interior con `/`). El núcleo lista con su propio hilo (`archive_nav::spawn_load`) usando `sys::archives().list` (no descomprime un zip para listar), arma las carpetas implícitas (`archive_nav::children`) y le pide a la plataforma solo el ícono y el tipo (`FileSystem::archive_look`, con defecto). Las extracciones de un solo elemento (abrir, vista previa, zip anidado) van a una carpeta temporal propia por ejecución (`archive_nav::temps`) con un candado (`File::try_lock`) que permite borrar al arrancar las de ejecuciones que ya no están y al salir la propia. Sacar cosas es un trabajo nuevo del motor, `JobSpec::ExtractEntries`, que extrae lo elegido en la carpeta oculta del motor junto al destino (`Archives::extract_some`) y lo lleva con los conflictos de siempre (como «Extraer aquí»).

**Tech Stack:** Rust 2024; `xdir-core` sin dependencias ni `cfg` de sistema; `xdir-archive` (zip 9, tar, flate2).

## Restricciones

- El hilo de interfaz nunca lista ni extrae: listar, resolver un zip anidado y extraer para abrir / vista previa corren en hilos; la respuesta vuelve por `WM_APP_LOAD` (listado) o `WM_APP_ARCHIVE` (extracción).
- Nombres validados con `safe_entry_path` (mismas reglas que al extraer); los inválidos se muestran en la raíz con su nombre crudo y no se abren ni copian. Los enlaces se muestran y no se siguen.
- Límite de entradas: `transfer::engine::BIG_ENTRIES` (100.000), igual que el motor.
- Fakes en `sys::testing` (`FakeArchives::extract_some`); rutas de prueba con `Path::join`.

## Tareas

### T1 — Tamaño comprimido y cifrado en `ArchiveEntry`
- `archive.rs`: `ArchiveEntry { packed: u64, encrypted: bool, .. }`. `xdir-archive/zip.rs`: `compressed_size()`, `encrypted()`; `targz.rs`: `packed = 0`, `encrypted = false`. Fake: `packed` = largo / 2, `encrypted` si `set_password`.
- Tests: `xdir-archive::tests::list_reports_packed_and_encrypted`.

### T2 — `Archives::extract_some`
- `fn extract_some(&self, archive, root, want: &dyn Fn(&str) -> bool, progress, failed) -> Result<usize, CopyError>` (defecto: `UNSUPPORTED`). `xdir-archive`: `zip::extract_all` y `targz::extract_all` reciben `want`. Fake igual.
- Tests: `xdir-archive::tests::extract_some_only_wanted_zip`, `extract_some_only_wanted_targz`; `sys::testing` `fake_extract_some_only_wanted`.

### T3 — `Location::Archive`
- `fs.rs`: variante, `parent` (sube por `inner`, después a la carpeta interior del anidado, después a la carpeta del archivo), `title`, `crumbs_in` (las del archivo + un tramo por archivo anidado y por carpeta), `target() = None`, `archive_place()`. `session.rs`: `comprimido:<archivo>//<anidado>//…//<interior>`. `folder_views.rs`: clave única `comprimido` con vista de fábrica Nombre / Tamaño / Tamaño comprimido / Fecha. Barra de dirección: texto `archivo/interior`.
- Tests: `fs::tests::archive_parent_title_crumbs`, `session::tests::archive_location_roundtrip`, `folder_views::tests::archive_default_view`.

### T4 — `archive_nav::children` (puro)
- `children(entries: &[ArchiveEntry], inner: &str) -> Vec<Child>`; `Child { name, is_dir, size, packed, modified, link, bad, archive }`. Carpetas implícitas, duplicados, `__MACOSX` fuera, inválidos en la raíz con `bad`, enlaces con `link`.
- Tests: `implicit_folders_listed_once`, `invalid_names_flagged_at_root`, `symlinks_are_links_not_dirs`, `mac_metadata_hidden`, `capped_at_big_entries`.

### T5 — Columna «Tamaño comprimido»
- `Entry.packed` (0 = vacío), `Column::Packed` (al final de `COLUMNS`), orden y texto de celda. `ATTR_ARCHIVE_BAD`.
- Tests: `list::tests::packed_column_text_and_sort`.

### T6 — Temporales (`archive_nav::temps`)
- `Temps::start(base)` (borra las de ejecuciones sin candado), `run_dir`, `entry_path(file, nested, rel)`, `ensure(file, nested, rel) -> Result<PathBuf, CopyError>` (serializado, con `copy_prims` para mirar y crear, reextrae si cambió el archivo), `cleanup()` al salir. Caché de los últimos listados.
- Tests: `stale_run_dirs_removed_live_kept`, `ensure_extracts_once`, `nested_chain_resolves`, `changed_archive_reextracts`.

### T7 — Listar en la pestaña
- `Tab::start` con `Location::Archive` → `archive_nav::spawn_load` (hilo, «Cargando…», cancelación por generación); búsqueda del panel filtra en el lugar; `entry_path` = ruta temporal; sin vigilancia ni metadatos.
- Tests: `tab_lists_archive_on_worker`, `search_in_archive_filters_in_place`, `up_from_archive_root_goes_to_folder`.

### T8 — Abrir
- Doble clic / Enter en un archivo comprimido de disco → `Location::Archive`; con Alt/⌥ → el sistema. Adentro: carpeta → navega; zip anidado → navega; archivo → extracción temporal en hilo y `open_file`; enlace o nombre inválido → aviso. Contraseña o dañado → vuelve y pregunta «¿Abrir con el sistema?».
- Tests (`app/archive_nav_tests.rs`): `double_click_zip_enters_it`, `alt_double_click_opens_with_system`, `open_file_inside_extracts_and_opens`, `nested_zip_navigable`, `password_zip_offers_system`, `bad_name_not_opened`.

### T9 — Solo lectura
- Comandos que escriben (Eliminar, Papelera, Renombrar, Renombrar varios, Cortar, Pegar, Carpeta nueva, Archivo nuevo, Duplicar, Comprimir, Carpeta con la selección, Acceso directo, Extraer) → aviso. Menú contextual propio con esas entradas deshabilitadas. Soltar en un panel dentro de un zip → no.
- Tests: `writing_commands_show_read_only_notice`, `context_menu_disables_writing_entries`, `drop_into_archive_refused`.

### T10 — Sacar cosas
- `JobSpec::ExtractEntries { archive, entries, dest }` en el motor (carpeta del motor junto al destino, `extract_some`, conflictos como «Extraer aquí», enlaces revisados). Copiar adentro guarda un portapapeles propio; Pegar en otro panel lo usa. Arrastrar desde un zip a otro panel de xdir extrae ahí.
- Tests: `engine::extract_entries_copies_selected_with_conflicts`, `app: copy_paste_out_of_archive_extracts`, `drag_out_of_archive_extracts`.

### T11 — Vista previa, Galería y vista rápida
- El elemento enfocado de un zip (hasta `thumbs::BIG_CACHE_BYTES`) se extrae a la temporal y, al llegar, la columna, la Galería y la vista rápida lo vuelven a pedir.
- Tests: `preview_of_file_inside_archive_extracts_then_shows`, `big_entry_not_extracted_for_preview`.

### T12 — Sesión y pestañas
- Al restaurar, un `Location::Archive` cuyo archivo ya no está → su carpeta. Pestaña nueva y clic medio en una carpeta del zip.
- Tests: `restore_archive_tab_falls_back_to_folder`, `middle_click_folder_in_archive_opens_tab`.

### T13 — Plataformas
- `archive_look` en macOS, Windows y Linux; `Temps` al arrancar y al salir. `cargo check` Windows (± `--no-default-features`) y Linux.

### T14 — Cierre
- `docs/DISENO.md` (pantallas X68–X71 si existen), smoke en la Mac, informe.

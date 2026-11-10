# Funciones del Explorador, tanda D (operaciones con archivos) — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Duplicar, carpeta nueva con la selección, Devolver de la Papelera, Expulsar, accesos directos, un motor de copia propio con su ventana del sistema (cola, conflictos, errores, gráfico de velocidad), comprimir y extraer, Renombrar varios y un panel de información, en Windows, macOS y Linux (Linux sin interfaz: núcleo y servicios listos).

**Architecture:** Las operaciones chicas (PR 1) se suman como variantes nuevas de `Op` que cada plataforma resuelve en su `FileOps::spawn`, con los nombres («copia», « 2», «Carpeta nueva con N elementos», «alias») calculados por funciones puras del núcleo (`names.rs`). El motor de copia (`transfer/`) es código neutro del núcleo: recorre con metadatos, decide conflictos, copia archivo por archivo con un servicio nuevo de primitivas (`sys::CopyPrims`, falso en memoria en `sys::testing`), corre cada trabajo en su hilo y manda `Update`s a la `App` por canal con `WM_APP_TRANSFER`; la ventana de copia es una ventana secundaria del sistema (`WindowHost::aux_*`) cuyo contenido dibuja el núcleo (`transfer::view`). Zip y tar viven en la crate nueva `xdir-archive`, detrás de `sys::Archives`. Renombrar varios y la información ocupan el lugar de la columna de vista previa (panel derecho), con lógica pura en el núcleo y un servicio `sys::Info` por plataforma.

**Tech Stack:** Rust 2024; `xdir-core` sin dependencias; `windows` 0.62 (features nuevas: `Win32_Devices_DeviceAndDriverInstallation`, `Win32_System_Ioctl`, `Win32_System_EventLog`, `Win32_Security_Authorization` si faltan); macOS `objc2` 0.6 (`objc2-app-kit`, `objc2-foundation`), `libc`; Linux `libc`; crate nueva `xdir-archive` con `zip` (solo store + deflate con `flate2`/`miniz_oxide`, sin `bzip2`/`zstd`/`aes`), `flate2` (`rust_backend`), `tar` (la versión estable más nueva de cada una al crearla; anotarla en el commit).

**Spec:** `docs/superpowers/specs/2026-11-10-explorador-tanda-d-design.md`. La sección final «Decisiones del usuario (2026-11-10)» manda sobre todo lo anterior: información = panel derecho como la columna de vista previa (sigue al panel activo); diálogo de copia = ventana aparte del sistema con cola y gráfico de velocidad; Renombrar varios = panel derecho + vista previa en la lista; zip con una crate común, con avance en la ventana de copia. Diseño: pantallas «Tanda D» de `docs/design/xdir.figcraft` y su sección en `docs/DISENO.md` (las está terminando otro agente; leerlas al empezar cada tarea de interfaz, no editarlas).

## Global Constraints

- `xdir-core` sin dependencias ni `#[cfg]` de sistema en producción; lo de cada sistema detrás de `xdir_core::sys` o `WindowHost`, con su falso en `sys/testing.rs`. Zip y tar solo en `xdir-archive`. Tests de `App` con `install_fakes()`, `FakeHost` y `RecordingPainter`.
- Las 3 plataformas: cada servicio nuevo se implementa en Windows, macOS y Linux; Linux (sin interfaz) compila y prueba servicios y lógica; `WindowHost` no existe allí todavía.
- El hilo de interfaz nunca espera al disco: toda E/S (copiar, saber si algo es expulsable, resolver un acceso directo, leer la Papelera, leer la información) corre en hilos de trabajo y vuelve por canal + mensaje de la aplicación. Mensajes nuevos: `WM_APP_TASK = WM_APP + 26`, `WM_APP_TRANSFER = WM_APP + 27`, `WM_APP_TRANSFER_DUE = WM_APP + 28`, `WM_APP_INFO = WM_APP + 29`.
- Nunca perder datos: lo a medio copiar se borra; «Reemplazar» escribe a un temporal oculto en la carpeta destino y recién al terminar bien reemplaza atómicamente; lo reemplazado va a la Papelera si el disco tiene; mover entre discos = copiar, verificar tamaño, borrar el original archivo por archivo; nunca pisar sin preguntar (tampoco un archivo sobre sí mismo).
- Archivos solo en la nube (`FileSystem::cloud_only`, mismos chequeos que la tanda C): el recorrido inicial, mover en el mismo disco, renombrar, la información, crear accesos directos y expulsar no leen contenido; copiar y comprimir sí.
- Nombres: Duplicar «nombre copia.ext», «nombre copia 2.ext»…; Conservar ambos y libres « 2», « 3» antes de la extensión; «Carpeta nueva con N elementos»; las mismas reglas en los 3 sistemas.
- Atajos (tabla §10 del spec): `duplicar` Ctrl+D / ⌘D; `favorito` Ctrl+Shift+D / ⇧⌘D; `carpeta_con_seleccion` Ctrl+Alt+Shift+N / ⌃⌘N («Meta+Ctrl+N» en la tabla `KEYS` de la Mac); `propiedades` Alt+Enter / ⌘I; `invertir_seleccion` sin cambios; F2 con varios = Renombrar varios; Comprimir, Extraer, accesos directos, Expulsar, Devolver (⌘⌫ dentro de la Papelera en macOS), Renombrar varios y Mostrar operaciones sin atajo, en paleta y menús. Quien tenga su atajo en `[atajos]` lo conserva.
- Castellano en toda la interfaz y los errores («Lo está usando otro programa», «No tenés permiso», «Ya no existe», «El disco está lleno»…).
- `cargo test` en la Mac y en Linux con Docker (`sh /private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/dk.sh cargo test`; si el script no existe, recrearlo como en la tanda C) sin bajar; `cargo check --all-targets` sin errores ni warnings nuevos para la Mac, `x86_64-pc-windows-msvc` (también `--no-default-features`) y `x86_64-unknown-linux-gnu`; clippy sin avisos nuevos; CI verde.
- Smoke en la Mac al final de cada tarea de interfaz: `cargo build --release -p xdir-mac --features pruebas`, copia aislada con `xdir.portable` en `…/scratchpad/vis/<tarea>/` y `vis/run.sh`, capturas comparadas con las pantallas de la tanda D; carpetas de prueba en el scratchpad; nunca tocar `/Applications/xdir.app` en las pruebas; nunca `osascript quit`.
- No `cargo fmt` del repo. Conventional Commits (tipo y scope en inglés, descripción en castellano) sin atribución a Claude; `git add` por nombre. Un PR por parte contra `develop` (ver «PRs»); al mergear la última, versión de Mac instalada.

## Review Focus

- **Cancelar o fallar a mitad de un «Reemplazar»** (archivo grande, disco lleno, cancelar): el archivo que ya estaba queda intacto y no queda el temporal → `replace_cancel_keeps_original` y `replace_disk_full_keeps_original` (Task 7).
- **Mover entre discos con un archivo bloqueado o que se copió mal**: en el origen queda exactamente lo que no se movió; nada desaparece de los dos lados → `move_cross_volume_keeps_failed_sources` y `move_size_mismatch_keeps_source` (Task 7).
- **Mayúsculas en un disco que no las distingue** («Foto.JPG» sobre «foto.jpg»; renombrar «a»→«A»): es conflicto al copiar y no es choque al renombrar solo mayúsculas → `conflict_case_insensitive_volume` (Task 7), `rename_many_case_only_change` (Task 16).
- **Zip malicioso o raro** (rutas absolutas, `..`, enlace que apunta afuera, contraseña, nombres CP437): nada se escribe fuera de la carpeta destino y el error es claro → `safe_entry_rejects_escapes`, `symlink_inside_only` (Task 14), `extract_skips_escaping_entries` (Task 15).
- **Elementos solo en la nube** al mover en el mismo disco, preparar una copia o abrir la información: nunca se piden datos → `walk_does_not_read_content` (Task 6), `move_same_volume_never_copies` (Task 7), `info_cloud_only_not_read` (Task 18).

---

## PR 1 — Operaciones chicas

### Task 1: Duplicar y favoritos a Ctrl+Shift+D

**Files:**
- Create: `crates/xdir-core/src/names.rs` (nombres puros).
- Modify: `crates/xdir-core/src/lib.rs`, `ops.rs` (`Op::Duplicate`, `OpResult::select`), `commands.rs` (`Command::Duplicate`, `favorito` a Ctrl+Shift+D), `app/actions.rs`, `app/fileops.rs` (`on_op_done` selecciona `select`; entrada «Duplicar» en `item_entries`), `tab.rs` (`select_after_load`), `sys/testing.rs`; `crates/xdir-mac/src/ops/macos.rs` (usa `names`, borra su `free_name`), `crates/xdir-mac/src/sys/macos.rs` (`KEYS`: `(Command::ToggleFavorite, &["Ctrl+Shift+D"])`; ⌘D sale de la tabla común); `crates/xdir-linux/src/ops.rs` (ídem); `crates/xdir-windows/src/ops/win32.rs` (`CopyItem` con `pszCopyName`).

**Interfaces:**
- Produces (`names.rs`):
  - `pub fn split_ext(name: &str, is_dir: bool) -> (&str, &str)` (extensión con el punto; carpetas y nombres que empiezan con punto no tienen; solo la última extensión: «a.tar.gz» → («a.tar», «.gz»)).
  - `pub fn copy_name(name: &str, is_dir: bool, n: u32) -> String` (n=1 «informe copia.pdf», n≥2 «informe copia 2.pdf»).
  - `pub fn numbered_name(name: &str, is_dir: bool, n: u32) -> String` (n=1 el mismo, n≥2 «informe 2.pdf»).
  - `pub fn free_name(dir: &Path, make: &dyn Fn(u32) -> String, taken: &dyn Fn(&Path) -> bool) -> PathBuf` (prueba n = 1, 2, …; `taken` lo da quien llama: `symlink_metadata().is_ok()` en las plataformas, un conjunto en los tests).
- Produces (`ops.rs`): `Op::Duplicate { sources: Vec<PathBuf> }`; `OpResult { …, pub select: Vec<PathBuf> }` (con `Default`; las plataformas pasan a construir con `..Default::default()`).
- Produces: `Command::Duplicate`, spec `spec(Command::Duplicate, "duplicar", "Duplicar", &["Ctrl+D"])` justo antes de `favorito`; `favorito` = `&["Ctrl+Shift+D"]`. `Tab::select_after_load(&mut self, names: Vec<OsString>)` (al recargar deja seleccionados esos y el foco en el último). `on_op_done`: si `select` no está vacío, cada pestaña que muestra la carpeta de los elementos los selecciona al recargar.
- Plataformas: destino de cada fuente = `names::free_name(parent, &|n| copy_name(name, is_dir, n), &taken)`; copiar como su `Op::Copy` (macOS `copy`, Linux `copy_item`, Windows `CopyItem(src, parent, pszCopyName)` sin `FOF_RENAMEONCOLLISION`). Resultado: `undo: Undo::Created(copias)`, `select: copias`, `focus: última`.

- [ ] **Step 1: Tests que fallan**: `copy_names` (`copy_name("informe.pdf", false, 1) == "informe copia.pdf"`, n=3 → «informe copia 3.pdf»; carpeta «Fotos.2024» → «Fotos.2024 copia»; «.bashrc» → «.bashrc copia»; «a.tar.gz» → «a.tar copia.gz»), `numbered_names` («informe.pdf», 2 → «informe 2.pdf»), `free_name_skips_taken` (tomados «a copia.txt» y «a copia 2.txt» → «a copia 3.txt»), `duplicate_command_and_favorite_moved` (`duplicar` en Ctrl+D; `favorito` en Ctrl+Shift+D; `combo_label_in(Mac, …)` «⌘D» y «⇧⌘D»; `defaults_are_consistent` sigue pasando), `duplicate_spawns_op_with_selection` (App: 2 seleccionados, Ctrl+D → `FakeFileOps::take_ops() == [Op::Duplicate { sources }]`; sin selección no hace nada; en `Location::Shell` no hace nada), `duplicate_selects_copies_after_reload` (`respond_with(OpResult { select: [a copia, b copia], .. })` → tras la recarga, esos 2 seleccionados), `item_menu_has_duplicate`; macOS y Linux: `duplicate_creates_copy_names` (carpeta temporal: «a.txt» dos veces → «a copia.txt», «a copia 2.txt», contenido igual, `undo` con las dos).
- [ ] **Step 2:** `cargo test -p xdir-core names duplicate` → FAIL (no definidos).
- [ ] **Step 3:** Implementar núcleo y las 3 plataformas.
- [ ] **Step 4:** PASS (también `cargo test -p xdir-mac duplicate` y Linux en Docker); Global Constraints; smoke en la Mac: ⌘D sobre un archivo y una carpeta, ⇧⌘D agrega a favoritos, ⌘Z manda las copias a la Papelera.
- [ ] **Step 5: Commit** `feat: duplicar con Ctrl+D / ⌘D (favoritos pasa a Ctrl+Shift+D)`.

### Task 2: Modificador Meta y Carpeta nueva con la selección

**Files:**
- Modify: `crates/xdir-core/src/keys.rs` (`VK_LWIN = Key(0x5B)`), `commands.rs` (`Combo::meta`, `parse_combo`, `combo_label_in`, `Keymap::lookup`, `Command::NewFolderWithSelection`), `app/actions.rs` (`on_key` lee Meta), `app/events.rs` (`on_alt_key`), `config.rs` (tests de `[atajos]`), `ops.rs` (`Op::NewFolderWith`, `Undo::Batch`, `Undo::CreatedFolder`), `names.rs` (`folder_with_name`), `app/fileops.rs` (entrada de menú con ≥ 2), `sys/testing.rs` (`FakeEnv` responde `key_down(VK_LWIN)` según un `Cell` del hilo); `crates/xdir-mac/src/sys/macos.rs` (`key_down(0x5B)` = `NSEventModifierFlags::Control`; `KEYS`: `(Command::NewFolderWithSelection, &["Meta+Ctrl+N"])`), `crates/xdir-mac/src/app/mac.rs` (con ⌃ apretada, `keyDown` sigue llamando `on_key`), `crates/xdir-windows/src/sys/win32.rs` (`key_down(0x5B)` = `VK_LWIN` o `VK_RWIN`), `crates/xdir-linux/src/env.rs`; `ops` de las 3 plataformas.

**Interfaces:**
- Produces: `Combo { vk, ctrl, shift, alt, meta: bool }`; `Keymap::lookup(&self, vk: Key, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Option<Command>` (todas las llamadas actuales pasan `key_down(VK_LWIN)`, los tests `false`). `parse_combo` acepta «Meta», «Win» y «Super» (sin distinguir mayúsculas). Texto: Windows «Win+Ctrl+N» (orden Win, Ctrl, Shift, Alt); Mac «⌃⌥⇧⌘» (Meta = ⌃ primero). El error de modificador desconocido pasa a decir «(usá Ctrl, Shift, Alt o Meta)».
- Produces: `Command::NewFolderWithSelection`, `spec(…, "carpeta_con_seleccion", "Carpeta nueva con la selección", &["Ctrl+Alt+Shift+N"])` después de `archivo_texto_nuevo`.
- Produces (`ops.rs`): `Op::NewFolderWith { parent: PathBuf, items: Vec<PathBuf> }`; `Undo::Batch(Vec<Undo>)` (se deshace en orden inverso; las plataformas recursan en su `undo_op` juntando errores); `Undo::CreatedFolder(PathBuf)` (a la Papelera solo si quedó vacía; si no, se deja).
- Produces (`names.rs`): `pub fn folder_with_name(count: usize) -> String` → «Carpeta nueva con 3 elementos» (con `numbered_name` para « 2»).
- Plataformas: crea la carpeta libre en `parent`, mueve cada elemento adentro (mismo disco: renombrar; otro disco: su mover de hoy), `undo: Batch([CreatedFolder(carpeta), Moved(pares)])`, `focus: carpeta`, `rename_after: true`; si no se pudo mover ninguno, borra la carpeta vacía y devuelve el error.
- `App`: `Command::NewFolderWithSelection` con ≥ 1 seleccionado y `location.target()` (en búsqueda: la carpeta buscada); el menú del clic derecho lo muestra con ≥ 2.

- [ ] **Step 1: Tests que fallan**: `meta_combos_parse_and_print` (`parse_combo("Meta+Ctrl+N")` con `meta`; `combo_label_in(Mac, …) == "⌃⌘N"`; Windows «Win+Ctrl+N»; `"Meta+X"` ya no es error — actualizar `combos_parse_and_print`), `meta_is_part_of_lookup` (Ctrl+N ≠ Meta+Ctrl+N), `new_folder_with_selection_shortcut` (Ctrl+Alt+Shift+N → comando; atajo en `[atajos]` con «Meta+Ctrl+K» se lee de `config.toml`), `folder_with_name_text`, `new_folder_with_spawns_op` (2 seleccionados → `Op::NewFolderWith { parent, items }`; el resultado con `rename_after` abre el renombrado sobre la carpeta), `menu_new_folder_with_needs_two`; macOS y Linux: `new_folder_with_moves_and_undoes` (3 archivos → carpeta «Carpeta nueva con 3 elementos» con los 3; segunda vez → «… 2»; `undo_op(Batch)` los devuelve y manda la carpeta vacía a la Papelera), `undo_created_folder_keeps_non_empty` (con un archivo nuevo adentro, la carpeta queda).
- [ ] **Step 2:** `cargo test -p xdir-core meta new_folder_with` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: ⌃⌘N con 3 archivos, la carpeta queda en renombrado; ⌘Z; la paleta muestra «⌃⌘N».
- [ ] **Step 5: Commit** `feat: carpeta nueva con la selección y modificador Meta en los atajos`.

### Task 3: Devolver desde la Papelera

**Files:**
- Create: `crates/xdir-mac/src/trash.rs` (lector de `.DS_Store` y registro propio).
- Modify: `crates/xdir-core/src/sys/mod.rs` (`FileSystem::is_trash`), `ops.rs` (`Op::Restore`), `commands.rs` (`Command::Restore`), `pane.rs` (botón «Devolver» en la barra), `app/{actions,fileops,input,paint}.rs`, `sys/testing.rs` (`FakeFileSystem::set_trash`); `crates/xdir-mac/src/{ops/macos.rs,sys/macos.rs,main.rs}` (el `trash()` de hoy anota en el registro), `crates/xdir-linux/src/{trash.rs,ops.rs,sys.rs}`, `crates/xdir-windows/src/sys/win32.rs`.

**Interfaces:**
- Produces: `fn is_trash(&self, loc: &Location) -> bool { false }` en `FileSystem` (sin E/S): Windows `Location::Shell` con `::{645FF040-5081-101B-9F08-00AA002F954E}` (sin distinguir mayúsculas); macOS `~/.Trash` o `/Volumes/*/.Trashes/<uid>`; Linux `<data_home>/Trash/files`.
- Produces: `Op::Restore { items: Vec<PathBuf> }` (macOS y Linux); `Command::Restore` id `devolver`, «Devolver», sin atajo. En Windows `Restore` sobre la Papelera llama `sys::shell().invoke_verb(hwnd, &loc, &items, "undelete")` y recarga (no pasa por `FileOps`).
- Plataformas (en el hilo de la operación): cada elemento vuelve a su origen; si la carpeta de origen no existe se crea (`create_dir_all`); si ya hay algo con ese nombre, no se pisa: error «Ya existe «x» en «carpeta»» (el conflicto con diálogo llega en la Task 11). Sin origen conocido: error «No se sabe de dónde vino «x»». Resultado: `undo: Undo::Created(devueltos)` (deshacer los manda otra vez a la Papelera), `select` = devueltos.
- macOS `trash.rs`: `pub fn parse_ds_store(bytes: &[u8]) -> HashMap<String, (String, String)>` (nombre en la Papelera → (`ptbL`, `ptbN`)); `pub fn origin(in_trash: &Path) -> Option<PathBuf>` (primero el registro propio, después el `.DS_Store` de esa Papelera; `ptbL` es la carpeta relativa a la raíz del volumen, sin «/» inicial: «/» + ptbL + ptbN en el disco de arranque, «/Volumes/X/» + ptbL + ptbN en otro); `pub fn log_trashed(in_trash: &Path, original: &Path)` / `pub fn forget(in_trash: &Path)` sobre `<config_dir>/papelera.tsv` (líneas `ruta_en_papelera\truta_original`; al reescribir se descartan las que ya no existen).
- Formato `.DS_Store` (solo lectura, big endian): bytes 0..4 = 1, 4..8 = «Bud1»; los desplazamientos del archivo se suman 4. En +8 el desplazamiento del bloque raíz: cantidad de bloques, direcciones (`addr & !0x1f` = posición, `1 << (addr & 0x1f)` = tamaño), relleno hasta 256, luego la tabla de nombres (u8 largo, nombre, u32 id): «DSDB» → bloque con (raíz, niveles, registros, nodos, tamaño de página). Nodo: u32 `P` (0 = hoja) y u32 `n`; hoja: `n` registros; interno: `n` × (u32 hijo, registro) y `P` es el hijo de la derecha. Registro: u32 largo en UTF-16, nombre UTF-16BE, 4 bytes de estructura (`ptbL`, `ptbN`…), 4 de tipo: `bool` 1, `long`/`shor`/`type` 4, `comp`/`dutc` 8, `blob` u32 + bytes, `ustr` u32 largo + UTF-16BE. Cualquier inconsistencia → mapa vacío (nunca pánico).
- Linux: `pub fn origin(name: &str, trash: &Path) -> io::Result<PathBuf>` (lee `Path=` del `.trashinfo` y lo decodifica) y `restore` ya existente.
- `App`: «Devolver» primero en el menú de los elementos cuando `is_trash(location)`; botón «Devolver» en la barra de la carpeta (`PaneHit::Restore`, apagado sin selección) con la Papelera a la vista; en macOS (`KeyStyle::Mac`) `Command::Recycle` dentro de la Papelera devuelve (⌘⌫ como el Finder).

- [ ] **Step 1: Tests que fallan**: núcleo `restore_menu_and_button_in_trash` (`FakeFileSystem::set_trash(ruta)`: menú con «Devolver» primero; botón visible y apagado sin selección; clic con 2 seleccionados → `Op::Restore`), `recycle_in_trash_restores_on_mac` (entorno Mac: ⌘⌫ → `Op::Restore`; entorno Windows: sigue siendo borrar), `restore_not_offered_outside_trash`; macOS `ds_store_reads_ptb_records` (un `.DS_Store` armado por un ayudante del test con una hoja y dos registros, y otro con un nodo interno → las dos entradas; bytes cortados o «Bud1» falso → vacío), `origin_prefers_own_log`, `restore_creates_missing_parent_and_refuses_overwrite`; Linux `trashinfo_origin_decodes_path` («%20» → espacio), `restore_op_returns_undo_created`.
- [ ] **Step 2:** `cargo test -p xdir-core restore` y `cargo test -p xdir-mac ds_store` → FAIL.
- [ ] **Step 3:** Implementar (Windows: `is_trash` y el verbo; macOS y Linux: `Op::Restore`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: borrar desde el Finder y desde xdir, abrir `~/.Trash` en xdir, «Devolver» y ⌘⌫ los devuelven a su carpeta; ⌘Z los vuelve a mandar.
- [ ] **Step 5: Commit** `feat: devolver desde la Papelera`.

### Task 4: Crear acceso directo y Mostrar el original

**Files:**
- Create: `crates/xdir-core/src/app/tasks.rs` (trabajos sueltos en otro hilo).
- Modify: `crates/xdir-core/src/names.rs` (`shortcut_name`), `ops.rs` (`Op::Shortcuts`), `sys/mod.rs` (`Shell::link_target`, `Shell::link_style`), `commands.rs`, `app/{mod,events,fileops,actions}.rs`, `sys/testing.rs`; `crates/xdir-windows/src/{ops/win32.rs,shell/win32.rs}` (`IShellLinkW`, `IPersistFile`), `crates/xdir-mac/src/{ops/macos.rs,shell/macos.rs}` (bookmark de NSURL), `crates/xdir-linux/src/{ops.rs,shell.rs}`; los cargadores de las 3 plataformas si no marcan los enlaces (ver abajo).

**Interfaces:**
- Produces (`tasks.rs`): `pub const WM_APP_TASK: u32 = WM_APP + 26;` `pub(super) enum TaskDone { Original { link: PathBuf, target: Option<PathBuf> } }` (la Task 5 suma `Ejectable(Vec<PathBuf>)` y `Ejected { root: PathBuf, label: String, result: Result<(), EjectError> }`; la Task 17, `Renamed`); `App::spawn_task(&self, f: impl FnOnce() -> TaskDone + Send + 'static)` (hilo, canal `task_tx`, `post(WM_APP_TASK)`); `App::on_task_done(&mut self, t: TaskDone)`.
- Produces (`names.rs`): `pub enum LinkStyle { Lnk, Alias, Symlink }`; `pub fn shortcut_name(name: &str, style: LinkStyle, n: u32) -> String`: Lnk «informe.pdf - Acceso directo.lnk», n≥2 «informe.pdf - Acceso directo (2).lnk» (como el Explorador); Alias «informe.pdf alias», «informe.pdf alias 2»; Symlink «Enlace a informe.pdf», «Enlace a informe.pdf 2».
- Produces (`sys`): `Shell::link_style(&self) -> LinkStyle` (sin defecto); `Shell::link_target(&self, path: &Path) -> Option<PathBuf>` (bloqueante, solo en hilos de trabajo; `None` si no es acceso o no se resuelve; no lee el destino). `Op::Shortcuts { targets: Vec<PathBuf> }` → crea cada uno junto a su destino; `undo: Created`, `select` = creados; con uno solo, `focus` + `rename_after`.
- Menú: «Crear acceso directo» para cualquier selección con ruta; «Mostrar el original» solo con un elemento cuyo `Entry` es enlace: `attrs & ATTR_LINK != 0` (`pub const ATTR_LINK: u32 = 0x400` en `fs.rs`; las cargas — en sus hilos — lo ponen para enlaces simbólicos y, en macOS, alias de `NSURLIsAliasFileKey`) o extensión `lnk` en Windows. Elegirlo: `spawn_task` con `link_target`; al volver, abre la carpeta del original en el panel activo con el original enfocado (`focus_after_load`), o aviso «No se encuentra el original de "nombre"».
- Comandos `crear_acceso_directo` «Crear acceso directo», `mostrar_original` «Mostrar el original», sin atajo.

- [ ] **Step 1: Tests que fallan**: `shortcut_names` (las 3 formas y n=2), `create_shortcut_spawns_op`, `show_original_menu_only_for_links` (una entrada con `ATTR_LINK` sí; un archivo común no; dos seleccionados no), `show_original_navigates` (`FakeShell::set_link_target(link, Some(destino))` → tras `WM_APP_TASK` la pestaña activa va a la carpeta y enfoca el nombre), `show_original_missing_notice` (`None` → aviso exacto); macOS `alias_roundtrip` (crea el alias de un archivo temporal, `link_target` lo resuelve; el alias se llama «a.txt alias»); Linux `symlink_shortcut_and_target`; Windows (CI) `lnk_roundtrip`.
- [ ] **Step 2:** `cargo test -p xdir-core shortcut original` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: alias de una carpeta, «Mostrar el original»; mover el original y probar el aviso.
- [ ] **Step 5: Commit** `feat: crear acceso directo y mostrar el original`.

### Task 5: Expulsar

**Files:**
- Modify: `crates/xdir-core/src/shell.rs` (`EjectError`), `sys/mod.rs` (`Shell::ejectable`, `Shell::eject`), `commands.rs` (`Command::Eject`), `list/mod.rs` (marca y clic de ⏏ en «Este equipo», vistas Detalles e Íconos), `tree.rs` (⏏ en nodos `Kind::Drive`), `app/{tasks,fileops,actions,input,panes}.rs`, `sys/testing.rs`; `crates/xdir-windows/src/shell/win32.rs` (+ `Cargo.toml` features), `crates/xdir-mac/src/shell/macos.rs`, `crates/xdir-linux/src/shell.rs`.

**Interfaces:**
- Produces: `#[derive(Clone, Debug, PartialEq)] pub struct EjectError { pub busy_by: Option<String>, pub busy: bool, pub message: String }`; `Shell::ejectable(&self, roots: &[PathBuf]) -> Vec<PathBuf>` y `Shell::eject(&self, root: &Path) -> Result<(), EjectError>` (los dos bloqueantes, en hilos de trabajo).
- Qué es expulsable y cómo, por sistema: lo del spec §7 al pie de la letra (Windows `GetDriveTypeW` + `IOCTL_STORAGE_QUERY_PROPERTY` con `BusTypeUsb/Sd/Mmc/1394/FileBackedVirtual` y CD; `CM_Request_Device_EjectW` sobre el padre del disco del volumen — `IOCTL_STORAGE_GET_DEVICE_NUMBER` + `SetupDiGetClassDevsW(GUID_DEVINTERFACE_DISK)` para encontrarlo —; veto → `busy`, y el programa del evento 225 de Kernel-PnP con `EvtQuery` como mejor esfuerzo. macOS: claves `NSURLVolumeIsEjectableKey`/`…IsRemovableKey`/`…IsInternalKey`/`…IsLocalKey`; `NSWorkspace unmountAndEjectDeviceAtURL:error:`; ocupado → `lsof -F pc <volumen>`. Linux: `/sys/block/*/removable` o montajes bajo `/media`, `/run/media`; `udisksctl unmount -b` + `power-off`, si no `gio mount -e`, si no `umount`; ocupado → `/proc/*/fd` y `/proc/*/cwd`).
- Funciones puras probadas por plataforma: macOS `parse_lsof(out: &str) -> Vec<String>` (nombres `c…` sin repetir); Linux `holders(proc_root: &Path, mount: &Path) -> Vec<String>` (lee `comm`); Windows `bus_is_external(bus: u32) -> bool`.
- `App`: cache `ejectable: HashSet<PathBuf>`, renovada con `spawn_task(TaskDone::Ejectable)` al cargar «Este equipo» y al refrescar las unidades del árbol; `FileList::eject_rows: Vec<usize>` y `FileList::eject_hit(x, y) -> Option<usize>`; `Tree::eject_hit(x, y) -> Option<usize>` (nodo). `Command::Eject` id `expulsar` «Expulsar» (sobre el disco enfocado en «Este equipo» o el de la carpeta del panel) y entrada «Expulsar» del menú del disco.
- Antes de expulsar (`App::release_volume(root: &Path)`): toda pestaña de cualquier panel cuya carpeta está en ese disco pasa a «Este equipo» (suelta guardián y lectura), se cierra xtree si su raíz está ahí, se vacía el sujeto de la vista previa; después `spawn_task` con `eject`. Resultado: aviso en la barra «Se puede quitar "USB (E:)" con seguridad»; ocupado: `host.ask_yes_no_cancel("Expulsar", "No se pudo expulsar \"USB (E:)\": lo está usando Word. ¿Reintentar?")` (sin nombre: «lo está usando otro programa»); Sí reintenta.

- [ ] **Step 1: Tests que fallan**: `eject_button_only_on_ejectable` (`FakeShell::set_ejectable([E:])` → solo esa fila dibuja ⏏ en Detalles e Íconos; `RecordingPainter`), `eject_click_releases_tabs_first` (dos pestañas en `E:\fotos` y una en `C:\` → las de E: pasan a «Este equipo» antes de `FakeShell` registrar `eject(E:)`), `eject_success_notice`, `eject_busy_offers_retry` (`EjectError { busy_by: Some("Word"), busy: true, .. }` → `FakeHost` registró la pregunta con el texto exacto; responder Sí → segundo `eject`), `eject_from_tree_node`, `eject_command_uses_focused_drive`; macOS `parse_lsof_names`; Linux `holders_finds_cwd_and_fd` (un `/proc` falso en carpeta temporal); Windows (CI) `bus_types`.
- [ ] **Step 2:** `cargo test -p xdir-core eject` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con un pendrive o una imagen `hdiutil attach` del scratchpad: ⏏ en «Este equipo» y en el árbol; con un archivo abierto en Vista Previa, aviso de ocupado con el nombre.
- [ ] **Step 5: Commit** `feat: expulsar discos desde Este equipo, el árbol y el menú`.

---

## PR 2 — Motor de copia en el núcleo

### Task 6: Tipos, primitivas y recorrido

**Files:**
- Create: `crates/xdir-core/src/transfer/mod.rs` (tipos, `Control`), `transfer/walk.rs` (recorrido con metadatos), `transfer/speed.rs` (velocidad y tiempo restante), `transfer/tests.rs`.
- Modify: `crates/xdir-core/src/lib.rs`, `sys/mod.rs` (trait `CopyPrims`, `Services::copy_prims`, `sys::copy_prims()`), `sys/testing.rs` (`FakeCopyPrims`), los `install` de las 3 plataformas (un stub que responde `ErrKind::Other("sin implementar")` hasta las Tasks 9–10).

**Interfaces:**
- Produces (`sys`):
  ```rust
  pub trait CopyPrims: Send + Sync {
      fn stat(&self, p: &Path) -> Result<ItemStat, CopyError>;          // sin seguir enlaces ni leer contenido
      fn read_dir(&self, p: &Path) -> Result<Vec<OsString>, CopyError>;
      fn volume(&self, p: &Path) -> Option<VolumeInfo>;
      /// Crea `dst` (falla con Exists si ya está); `progress(bytes_copiados)` en cada bloque; si
      /// devuelve `Flow::Stop`, corta, borra `dst` y devuelve `ErrKind::Cancelled`.
      fn copy_file(&self, src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError>;
      fn copy_symlink(&self, src: &Path, dst: &Path) -> Result<(), CopyError>;
      fn create_dir(&self, dst: &Path) -> Result<(), CopyError>;
      fn finish_dir(&self, dst: &Path, like: &Path) -> Result<(), CopyError>;   // atributos y fechas al final
      fn rename_noreplace(&self, from: &Path, to: &Path) -> Result<(), CopyError>;
      fn replace(&self, from: &Path, to: &Path) -> Result<(), CopyError>;        // atómico
      fn remove(&self, p: &Path) -> Result<(), CopyError>;                       // archivo, enlace o carpeta vacía
      fn trash(&self, p: &Path) -> Result<String, CopyError>;                    // como `Undo::Recycled`
      fn trash_origin(&self, in_trash: &Path) -> Option<PathBuf> { None }
      fn untrash(&self, in_trash: &str, original: &Path) -> Result<(), CopyError>; // deshace `trash`
  }
  pub enum ItemKind { File, Dir, Symlink }
  pub struct ItemStat { pub kind: ItemKind, pub size: u64, pub modified: u64, pub cloud_only: bool }
  pub struct VolumeInfo { pub id: OsString, pub label: String, pub case_insensitive: bool, pub has_trash: bool, pub max_file: Option<u64>, pub free: u64 }
  pub enum Flow { Continue, Stop }
  pub enum ErrKind { InUse, Denied, NotFound, DiskFull, TooBig, Exists, Cancelled, Other(String) }
  pub struct CopyError { pub kind: ErrKind, pub path: PathBuf }
  ```
  `CopyError::message(&self) -> String`: «Lo está usando otro programa», «No tenés permiso», «Ya no existe», «El disco está lleno», «Es demasiado grande para el formato del disco de destino (FAT32)», «Ya existe», «Cancelado», el texto de `Other`.
- Produces (`transfer/mod.rs`): `pub type JobId = u64;` `pub struct Control` con `new() -> Arc<Control>`, `pause()`, `resume()`, `cancel()`, `is_cancelled()`, `answer(a: Answer)`, `pub(crate) fn checkpoint(&self) -> Flow` (bloquea mientras está en pausa; `Stop` si se canceló) y `pub(crate) fn wait_answer(&self) -> Option<Answer>` (`None` si se canceló) — `Mutex` + `Condvar`. `pub enum Choice { Replace, KeepBoth, Skip, Merge }`, `pub struct Answer { pub choice: Choice, pub all: bool }`.
- Produces (`walk.rs`): `pub struct Planned { pub src: PathBuf, pub rel: PathBuf, pub stat: ItemStat }`, `pub fn walk(prims: &dyn CopyPrims, root: &Path, ctl: &Control, seen: &mut dyn FnMut(u64)) -> Result<Vec<Planned>, CopyError>` (preorden; carpetas antes que su contenido; enlaces no se siguen; `seen(n)` cada 200 elementos para «Preparando… N elementos»; no lee contenido) y `pub fn totals(items: &[Planned]) -> (u64 /*elementos*/, u64 /*bytes*/)`.
- Produces (`speed.rs`): `pub struct Speed` con `new(start: Instant, total: u64)`, `push(&mut self, now: Instant, done: u64)`, `rate(&self) -> Option<f64>` (bytes/s, promedio de los últimos 5 s), `eta(&self, now: Instant) -> Option<Duration>` (`None` hasta tener 2 s de datos → «Calculando…»), `average(&self) -> Option<f64>` (desde el principio) y `graph(&self) -> &[f32]` (una muestra de velocidad por cada 1 % de avance, máximo 100 — el eje x del gráfico es el avance, como en Windows).
- `FakeCopyPrims` (proceso entero, rutas propias por test como `FAKE_DIRS`): `add_dir`, `add_file(path, bytes: Vec<u8>)`, `add_symlink`, `set_cloud_only`, `set_volume(prefix, VolumeInfo)` (por prefijo de ruta), `fail(path, ErrKind, times: u32)`, `fail_after(path, bytes, ErrKind)`, `set_block(n)` (tamaño de bloque, para avisos), `set_delay(Option<Duration>)` (demora por bloque), `contents(path) -> Option<Vec<u8>>`, `exists(path)`, `reads(path) -> u32` (veces que se leyó contenido), `trashed() -> Vec<(String, PathBuf)>` (`untrash` devuelve de ahí); compara nombres sin mayúsculas en volúmenes `case_insensitive`.

- [ ] **Step 1: Tests que fallan**: `walk_preorder_with_totals` (árbol de 2 carpetas y 3 archivos → orden y `totals == (5, suma)`), `walk_does_not_read_content` (`reads == 0`; un archivo `cloud_only` cuenta su tamaño sin leerlo), `walk_does_not_follow_symlinks`, `walk_cancel_stops`, `speed_eta_after_two_seconds` (pushes a 0,5 s → `eta` `None`; a 2,5 s → `Some`), `speed_moving_average_five_seconds` (10 MB/s y luego 20 MB/s durante 6 s → `rate` ≈ 20 MB/s), `speed_graph_one_point_per_percent` (≤ 100 muestras), `control_pause_blocks_until_resume` (un hilo en `checkpoint` no sigue hasta `resume`; `cancel` lo libera con `Stop`), `copy_error_messages` (los textos exactos).
- [ ] **Step 2:** `cargo test -p xdir-core transfer` → FAIL.
- [ ] **Step 3:** Implementar núcleo, falso y stubs.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): primitivas de copia, recorrido y velocidad del motor`.

### Task 7: El motor: copiar, mover, conflictos, errores y deshacer

**Files:**
- Create: `crates/xdir-core/src/transfer/engine.rs`, `transfer/engine_tests.rs`.
- Modify: `transfer/mod.rs` (`JobSpec`, `Update`, `JobOutcome`), `ops.rs` (sin cambios de tipos: usa `Undo::Batch`, `Created`, `Moved`, `Recycled`).

**Interfaces:**
- Consumes: Task 6 completa; `names::{copy_name, numbered_name, free_name}`, `names::folder_with_name` (Tasks 1–2).
- Produces:
  ```rust
  pub enum JobSpec {
      Copy { sources: Vec<PathBuf>, dest: PathBuf },
      Move { sources: Vec<PathBuf>, dest: PathBuf },
      Duplicate { sources: Vec<PathBuf> },
      NewFolderWith { parent: PathBuf, items: Vec<PathBuf> },
      Restore { items: Vec<PathBuf> },                    // origen por `trash_origin`
      Undo(Undo),                                        // Moved y Batch (ver abajo)
      Retry { moving: bool, pairs: Vec<(PathBuf, PathBuf)> }, // (origen, destino ya decidido)
  }
  pub enum Update {
      Preparing { items: u64 },
      Started { title: String, items: u64, bytes: u64 },
      Waiting,                                            // «En cola» (Task 8)
      Progress { items: u64, bytes: u64, current: String },
      Conflict(ConflictInfo),
      Finished(JobOutcome),
  }
  pub struct ConflictInfo { pub name: String, pub folder: String, pub is_dir: bool, pub can_merge: bool, pub incoming: ItemStat, pub existing: ItemStat, pub remaining: usize }
  pub struct JobError { pub src: PathBuf, pub dst: PathBuf, pub message: String }
  pub struct JobOutcome { pub result: OpResult, pub errors: Vec<JobError>, pub stopped: Option<String>, pub cancelled: bool }
  pub fn run(spec: JobSpec, prims: &dyn CopyPrims, ctl: &Control, out: &mut dyn FnMut(Update));
  pub fn title(spec: &JobSpec, items: u64) -> String;
  ```
- Reglas (todas del spec §1; las que el spec deja abiertas, fijadas acá):
  - Títulos: «Copiando N elementos de "A" a "B"», «Moviendo N elementos de "A" a "B"» (con 1: «Copiando «x» a "B"»), «Duplicando N elementos», «Devolviendo N elementos», «Deshaciendo».
  - `JobSpec::Undo`: `Moved` vuelve cada par con las reglas de mover; `Batch` en orden inverso; `Created` → `trash`; `CreatedFolder` → `trash` si quedó vacía; `Recycled` → `untrash`; `Rename` → `rename_noreplace`. Lo que no se pudo deshacer va a `errors`; `result.undo` = `None`.
  - Antes de copiar: bytes que hay que escribir (todo salvo mover en el mismo volumen) > `free` → no empieza, `stopped: Some("Faltan 3,2 GB en \"Backup (E:)\"")` (tamaño con `sys::env().format_size_short`). Archivos > `max_file` → `JobError` «Es demasiado grande…» de entrada, y se siguen los demás.
  - Copiar una carpeta dentro de sí misma: error «No se puede copiar una carpeta dentro de sí misma». Copiar a la misma carpeta → nombres de Duplicar. Mover a la misma carpeta → nada.
  - Mismo volumen y `Move`: `rename_noreplace` del elemento de arriba (no lee nada); si existe → conflicto; «Combinar» entra y renombra hijo por hijo.
  - Otro volumen y `Move`: por archivo `copy_file` → `stat(dst).size == stat(src).size` → `remove(src)`; si no coincide: error «La copia no quedó igual» y el origen queda; carpetas de origen se `remove` al final si quedaron vacías (si no, se dejan).
  - Conflicto (el destino existe): manda `Update::Conflict`, espera `wait_answer`; `all` fija la elección para el resto de **ese trabajo**, por separado para archivos y para carpetas. `can_merge` solo si los dos son carpetas. «Conservar ambos» → `numbered_name` libre. «Reemplazar» archivo → copia a `.xdir-<pid>-<n>.tmp` en la carpeta destino, `trash(existente)` si `has_trash` (si no, se borra después del reemplazo), `replace(tmp, dst)`; carpeta → la existente entera a la Papelera y se copia. «Omitir» → nada.
  - Pausa/cancelar: el aviso de progreso llama `ctl.checkpoint()`; cancelar borra el parcial (`copy_file` ya lo hace) y el temporal de un reemplazo; lo completo queda y entra en el deshacer.
  - Errores: cada `CopyError` se anota (`JobError`) y se sigue; `DiskFull` detiene el trabajo con `stopped: Some("El disco está lleno")`.
  - Progreso: a lo sumo uno cada 100 ms por trabajo, más el final con los totales.
  - Deshacer (`result.undo`): `Batch` en el orden en que pasó, de `Recycled(reemplazados)` y `Created(creados de arriba y archivos nuevos dentro de carpetas combinadas)` / `Moved(pares)`; vacío → `None`. `select` = elementos de arriba creados o movidos; `focus` el último.
  - `NewFolderWith`: crea la carpeta con `folder_with_name` libre y mueve como `Move`; `rename_after: true`; deshacer `Batch([CreatedFolder, Moved])`.
  - `Restore`: destino = `trash_origin`; crea la carpeta de origen si falta; conflicto como copiar; sin origen → `JobError` «No se sabe de dónde vino».

- [ ] **Step 1: Tests que fallan** (en `engine_tests.rs`, con `FakeCopyPrims` y respuestas desde el `out` del test): `copy_tree_and_undo` (contenido igual, `undo == Batch([Created([...])])`, `select` las de arriba), `copy_into_same_folder_duplicates`, `copy_into_itself_refused`, `move_same_volume_never_copies` (`reads == 0`, origen ya no existe, `Moved`), `move_cross_volume_verifies_then_deletes`, `move_cross_volume_keeps_failed_sources` (un archivo `InUse` → queda solo él en el origen y su carpeta; error con «Lo está usando otro programa»), `move_size_mismatch_keeps_source` (`fail_after` con tamaño distinto), `conflict_keep_both_names` («informe 2.pdf»), `conflict_replace_trashes_old` (`trashed` tiene el viejo; `undo` lo devuelve), `conflict_replace_without_trash_deletes`, `replace_cancel_keeps_original` (cancelar a mitad → contenido viejo intacto, sin `.tmp`), `replace_disk_full_keeps_original`, `conflict_merge_folders` (carpeta con 2 archivos sobre otra con 1 en común → entra y pregunta solo por el común), `apply_to_all_separate_for_files_and_dirs`, `conflict_case_insensitive_volume` («Foto.JPG» sobre «foto.jpg» → conflicto; en volumen que distingue → dos archivos), `errors_collected_and_continue`, `disk_full_stops`, `not_enough_space_refuses` (texto exacto con `FakeEnv` «3,2 GB» y la etiqueta), `fat32_big_file_error_upfront`, `pause_and_cancel_midfile` (con bloque chico: pausa detiene los avisos; cancelar borra el parcial y deja los completos en `Created`), `retry_only_failed_pairs`, `progress_throttled_and_final`, `new_folder_with_job`, `restore_job_uses_trash_origin`, `undo_job_moved_and_batch_reverse` (Batch de `Recycled` + `Created` → primero a la Papelera lo creado y después vuelve lo reemplazado).
- [ ] **Step 2:** `cargo test -p xdir-core engine` → FAIL.
- [ ] **Step 3:** Implementar `run` (un recorrido por raíz con `walk`, una función por modo; nada de E/S fuera de `prims`).
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): motor de copia con conflictos, errores y deshacer`.

### Task 8: Cola y controlador en la App

**Files:**
- Create: `crates/xdir-core/src/transfer/sched.rs` (quién espera a quién), `crates/xdir-core/src/app/transfers.rs` (estado de los trabajos en la `App`), `app/transfers_tests.rs`.
- Modify: `transfer/mod.rs` (`JobView`), `app/{mod,events,actions,fileops}.rs`; `ops.rs` y las 3 plataformas (se quitan `Op::Duplicate` y `Op::NewFolderWith` con su código: pasan al motor).

**Interfaces:**
- Consumes: `run`, `JobSpec`, `Update`, `Control` (Task 7).
- Produces (`sched.rs`): `pub struct Scheduler` (`Mutex` + `Condvar`) con `acquire(&self, id: JobId, vols: &[OsString], ctl: &Control) -> bool` (bloquea mientras un trabajo anterior en orden de llegada, corriendo o esperando, comparte un volumen; `false` si se canceló esperando) y `release(&self, id: JobId)`. El hilo del trabajo calcula sus volúmenes con `prims.volume` (origen y destino), manda `Update::Waiting` si tiene que esperar.
- Produces (`transfer/mod.rs`): `pub struct JobView { pub id: JobId, pub title: String, pub state: JobState, pub items: (u64, u64), pub bytes: (u64, u64), pub current: String, pub speed: Speed, pub conflict: Option<ConflictInfo>, pub errors: Vec<JobError>, pub stopped: Option<String>, pub paused: bool, pub started: Instant }` y `pub enum JobState { Preparing, Waiting, Running, Conflict, Done }`.
- Produces (`app/transfers.rs`): `App::start_transfer(&mut self, spec: JobSpec) -> JobId` (hilo por trabajo con `sys::copy_prims()`; manda `(JobId, Update)` por `transfer_tx` y `post(WM_APP_TRANSFER)`; nunca bloquea), `App::on_transfer_updates(&mut self)` (vacía el canal; `Finished` → `on_op_done(result)` como hoy, más errores/`stopped` en `JobView`), `pause_job(id)`, `resume_job(id)`, `cancel_job(id)`, `answer_job(id, Answer)`, `retry_job(id)` (`JobSpec::Retry` con los pares fallados), `running_jobs(&self) -> usize`, `jobs_on_volume(&self, root: &Path) -> Vec<JobId>`.
- En este PR van por el motor: `Command::Duplicate` (`JobSpec::Duplicate`) y `Command::NewFolderWithSelection` (`JobSpec::NewFolderWith`); copiar, mover, pegar, soltar y Devolver siguen como hoy hasta la Task 11. Sin ventana todavía: un conflicto o error se ve como aviso en la barra («Duplicar terminó con N errores»); Duplicar y carpeta nueva no generan conflictos.
- Expulsar (Task 5): si `jobs_on_volume(root)` no está vacío, pregunta «Hay N operaciones sobre "USB (E:)". ¿Cancelarlas y expulsar?» antes de soltar el disco.

- [ ] **Step 1: Tests que fallan**: `scheduler_parallel_on_distinct_volumes`, `scheduler_fifo_on_shared_volume` (B y C esperan a A; C no se adelanta a B si comparte con B), `scheduler_cancel_while_waiting`, `duplicate_goes_through_engine` (App con `FakeCopyPrims`: Ctrl+D → copia creada, `FakeFileOps::take_ops()` vacío, copias seleccionadas al recargar, Ctrl+Z las manda a la Papelera), `new_folder_with_goes_through_engine` (renombrado abierto sobre la carpeta), `conflict_waits_only_its_job` (dos trabajos en volúmenes distintos; uno en conflicto, el otro termina), `paste_returns_immediately` (con `FakeCopyPrims::set_delay`, `start_transfer` vuelve enseguida y `on_key` sigue respondiendo), `undo_of_job_is_one_step`, `eject_asks_when_jobs_on_volume`.
- [ ] **Step 2:** `cargo test -p xdir-core sched transfers` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints (las plataformas siguen compilando sin las dos variantes quitadas).
- [ ] **Step 5: Commit** `feat(core): cola de trabajos de copia en la App`.

### Task 9: Primitivas de copia en macOS y Linux

**Files:**
- Create: `crates/xdir-mac/src/ops/prims.rs`, `crates/xdir-linux/src/prims.rs`.
- Modify: `crates/xdir-mac/src/{ops/mod.rs,sys/macos.rs}`, `crates/xdir-linux/src/{sys.rs,main.rs}`, `crates/xdir-mac/src/ops/macos.rs` (`trash` devuelve también lo que necesita `log_trashed`).

**Interfaces:**
- Consumes: `CopyPrims` y tipos (Task 6); `trash::origin` (Task 3).
- macOS: `copy_file` con `copyfile(COPYFILE_ALL | COPYFILE_CLONE | COPYFILE_EXCL | COPYFILE_NOFOLLOW_SRC)` y un `copyfile_state_t` con `COPYFILE_STATE_STATUS_CB` (en `COPYFILE_PROGRESS` lee `COPYFILE_STATE_COPIED` y llama `progress`; `Stop` → `COPYFILE_QUIT`); `copy_symlink` con `COPYFILE_NOFOLLOW`; `create_dir` + `finish_dir` con `copyfile(COPYFILE_METADATA)` de la carpeta origen; `rename_noreplace` con `renamex_np(RENAME_EXCL)`; `replace` con `rename`; `volume` con `statfs` (`f_fstypename` «msdos» → `max_file = 4 GiB - 1`) y `pathconf(_PC_CASE_SENSITIVE)`; `has_trash` true salvo volúmenes de solo lectura o de red; `stat` con `lstat` y `SF_DATALESS`; `trash` = el de hoy (con registro); `trash_origin` = `trash::origin`; `untrash` = mover desde la Papelera (+ `trash::forget`).
- Linux: `copy_file` con `ioctl(FICLONE)` y si falla `copy_file_range` en bloques de 8 MiB (y `read`/`write` si tampoco), abriendo el destino con `O_CREAT | O_EXCL`; después permisos, `utimensat` y atributos `user.*`; `rename_noreplace` con `renameat2(RENAME_NOREPLACE)` y, si da `EINVAL`/`ENOSYS`, `symlink_metadata` + `rename`; `volume` con `statfs` (`MSDOS_SUPER_MAGIC` → FAT32) y `/proc/self/mountinfo` para el id; nunca `case_insensitive`; `trash`/`trash_origin`/`untrash` con `trash.rs` (`recycle`, `origin`, `restore`).
- Errores del sistema → `ErrKind`: `EBUSY`/`ETXTBSY` → `InUse`, `EACCES`/`EPERM` → `Denied`, `ENOENT` → `NotFound`, `ENOSPC`/`EDQUOT` → `DiskFull`, `EFBIG` → `TooBig`, `EEXIST` → `Exists`; función pura `pub(crate) fn err_kind(errno: i32) -> ErrKind` en cada una.

- [ ] **Step 1: Tests que fallan** (carpetas temporales): `prims_copy_file_progress_and_cancel` (archivo de 20 MB: avisos crecientes; `Stop` a la mitad → `Cancelled` y sin destino), `prims_copy_refuses_existing`, `prims_rename_noreplace_refuses`, `prims_replace_is_atomic`, `prims_symlink_copied_as_link`, `prims_finish_dir_keeps_mtime`, `prims_err_kind_mapping`, y el motor de punta a punta con primitivas reales: `engine_real_copy_tree` (macOS y Linux).
- [ ] **Step 2:** `cargo test -p xdir-mac prims` y en Docker `cargo test -p xdir-linux prims` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: ⌘D de una carpeta de 2 GB (clona al instante en APFS) y de un pendrive FAT32 a otro disco.
- [ ] **Step 5: Commit** `feat: primitivas de copia en macOS y Linux`.

### Task 10: Primitivas de copia en Windows

**Files:**
- Create: `crates/xdir-windows/src/ops/prims.rs`.
- Modify: `crates/xdir-windows/src/{ops/mod.rs,sys/win32.rs,Cargo.toml}`.

**Interfaces:**
- Consumes: `CopyPrims` (Task 6).
- `copy_file` con `CopyFile2` (`COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK`; `COPYFILE2_EXTENDED_PARAMETERS` con `pProgressRoutine` y `pfCancel`; en `COPYFILE2_CALLBACK_CHUNK_FINISHED` llama `progress(uliTotalBytesTransferred)` y devuelve `COPYFILE2_PROGRESS_CANCEL` con `Stop`; clona en ReFS solo); rutas largas con `\\?\` (`fs::wide_path` si ya lo hace); `rename_noreplace` = `MoveFileExW(…, MOVEFILE_COPY_ALLOWED = no, sin REPLACE)`; `replace` = `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`; `create_dir` = `CreateDirectoryExW(like)` y `finish_dir` = `SetFileTime` + atributos; `stat` con `GetFileAttributesExW` (+ `FILE_ATTRIBUTE_OFFLINE`/`RECALL_*` = nube; `REPARSE_POINT` con tag de enlace = `Symlink`); `volume` con `GetVolumeInformationW` (nombre del sistema de archivos «FAT32» → `max_file`; `FILE_CASE_SENSITIVE_SEARCH` ausente → `case_insensitive`), `GetDiskFreeSpaceExW`, id = `GetVolumePathNameW`; `has_trash` = disco fijo o extraíble local (no de red); `trash` = `IFileOperation` `DeleteItem` con `FOFX_RECYCLEONDELETE` + `FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI` devolviendo el nombre en la Papelera (lo que anota hoy el `Recorder`); `untrash` = `restore_from_recycle_bin`.
- `pub(crate) fn err_kind(code: u32) -> ErrKind`: `ERROR_SHARING_VIOLATION`/`ERROR_LOCK_VIOLATION` → `InUse`, `ERROR_ACCESS_DENIED` → `Denied`, `ERROR_FILE_NOT_FOUND`/`ERROR_PATH_NOT_FOUND` → `NotFound`, `ERROR_DISK_FULL`/`ERROR_HANDLE_DISK_FULL` → `DiskFull`, `ERROR_FILE_TOO_LARGE` → `TooBig`, `ERROR_FILE_EXISTS`/`ERROR_ALREADY_EXISTS` → `Exists`, `ERROR_REQUEST_ABORTED` → `Cancelled`.

- [ ] **Step 1: Tests que fallan** (CI de Windows): los mismos `prims_*` de la Task 9 y `prims_long_path` (ruta de 300 caracteres), `prims_err_kind_mapping`, `engine_real_copy_tree`.
- [ ] **Step 2:** `cargo check --target x86_64-pc-windows-msvc --all-targets` y CI → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en CI; check local también `--no-default-features`; Global Constraints. Lista de prueba manual para Runick93 en el PR: Ctrl+D de una carpeta grande, archivo abierto en Word.
- [ ] **Step 5: Commit** `feat(windows): primitivas de copia con CopyFile2`.

---

## PR 3 — Ventana de copia

### Task 11: Ventana secundaria en el host, vista de la cola y todo el tráfico por el motor

**Files:**
- Create: `crates/xdir-core/src/transfer/view.rs` (layout, dibujo y hit-test de la ventana: filas, gráfico, conflicto, errores), `transfer/view_tests.rs`.
- Modify: `crates/xdir-core/src/host.rs` (`AuxWindow`, métodos `aux_*`), `sys/testing.rs` (`FakeHost` los registra), `ui.rs` (indicador en la barra de estado), `commands.rs` (`Command::ShowTransfers`), `app/{transfers,fileops,actions,events,input,paint,mod}.rs`; `crates/xdir-windows/src/app/messages.rs` y `crates/xdir-mac/src/app/mac.rs` (preguntar antes de cerrar).

**Interfaces:**
- Consumes: `JobView`, `Speed::graph`, `Answer`, `Choice` y los métodos de la Task 8.
- Produces (`host.rs`): `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum AuxWindow { Transfers }`; en `WindowHost` con implementación por defecto: `fn aux_show(&self, _w: AuxWindow, _title: &str, _size: (f32, f32)) -> bool { false }` (crea o trae al frente; `size` en DIPs del área), `fn aux_resize(&self, _w: AuxWindow, _size: (f32, f32)) {}`, `fn aux_invalidate(&self, _w: AuxWindow) {}`, `fn aux_hide(&self, _w: AuxWindow) {}`. La plataforma llama `App::paint_aux(&mut self, w: AuxWindow, p: &dyn Painter, width: f32, height: f32)`, `App::aux_mouse(&mut self, w: AuxWindow, x: f32, y: f32, ev: AuxMouse)` con `pub enum AuxMouse { Move, Down, Up, Leave }`, `App::aux_key(&mut self, w: AuxWindow, vk: Key) -> bool` y `App::aux_closed(&mut self, w: AuxWindow)` (el usuario la cerró: se oculta, los trabajos siguen y queda el indicador).
- Produces (`view.rs`): `pub fn content_height(jobs: &[JobView], expanded: Option<JobId>) -> f32`; `pub fn draw(p: &dyn Painter, r: Rect, jobs: &[JobView], expanded: Option<JobId>, hover: Option<ViewHit>, env: &dyn SystemEnv)`; `pub fn hit(r: Rect, jobs: &[JobView], expanded: Option<JobId>, x: f32, y: f32) -> Option<ViewHit>` con `pub enum ViewHit { Pause(JobId), Resume(JobId), Cancel(JobId), Expand(JobId), Choose(JobId, Choice), ApplyAll(JobId), Retry(JobId), Dismiss(JobId) }`; `pub fn draw_graph(p: &dyn Painter, r: Rect, graph: &[f32], average: Option<f64>, remaining: Option<Duration>)`. Medidas y colores de las pantallas «Tanda D» (si falta alguna: ancho 480, fila 72, gráfico 96 de alto).
- Textos: fila «58 %», «48,2 MB/s», «quedan 2 min» / «Calculando…», «En cola», «Preparando… N elementos», «En pausa»; botones «Pausa», «Reanudar», «Cancelar»; conflicto «Ya existe «x» en «carpeta»», «El que se copia», «El que ya está» (tamaño y fecha), «Reemplazar», «Conservar ambos», «Omitir», «Combinar», «Hacer esto con todos (quedan N conflictos)»; fin «Terminó con N errores» con la lista (nombre — motivo), «Reintentar», «Cerrar»; espacio: el `stopped` tal cual; gráfico: «Velocidad: 48,2 MB/s» y la línea del promedio.
- `App`: la ventana aparece (`aux_show`) cuando un trabajo lleva más de 1 s (`WM_APP_TRANSFER_DUE` como `WM_APP_PREVIEW_DUE`) o enseguida si llega un conflicto o un error; el expandido (con gráfico) es el primero que está corriendo salvo que se elija otro; se redimensiona con `content_height`; con la cola vacía y sin errores se oculta sola. Barra de estado: `StatusButtons::transfers: Option<f32>` (avance total) con clic que vuelve a mostrarla; `Command::ShowTransfers` id `mostrar_operaciones` «Mostrar operaciones». Sin ventana del host (`aux_show` false, Linux): solo el indicador.
- **Todo pasa por el motor desde acá**: `paste` (lo cortado → `Move`), `on_drag` al soltar, `Command::Restore` en macOS y Linux (`JobSpec::Restore`) y `Command::Undo` con `Undo::Moved` o `Undo::Batch` (`JobSpec::Undo`; los demás deshacer siguen por `FileOps`). `Location::Shell` y `paste_virtual` siguen con el Shell. `Op::Copy`, `Op::Move` y `Op::Restore` quedan en las plataformas sin uso desde la `App` (respaldo; borrarlos es otra limpieza).
- Cerrar: `App::confirm_close(&mut self) -> bool`: con trabajos en curso, `ask_yes_no_cancel("xdir", "Hay N operaciones en curso. ¿Cancelarlas y salir?")`; Sí → cancela todos y espera hasta 5 s a que limpien → true; otro → false. Windows: `WM_CLOSE` no destruye si es false; macOS: `windowShouldClose` devuelve false.

- [ ] **Step 1: Tests que fallan**: `window_appears_after_one_second` (trabajo que sigue a 1 s → `FakeHost` registró `aux_show(Transfers, "Copiando…")`; uno que termina antes → nunca), `conflict_shows_window_now`, `rows_draw_progress_speed_eta` (`RecordingPainter`: «58 %», «48,2 MB/s», «quedan 2 min»; antes de 2 s «Calculando…»), `queued_row_says_en_cola`, `graph_drawn_for_expanded`, `conflict_view_buttons` (carpeta → con «Combinar»; archivo → sin), `conflict_answer_reaches_job` (clic en «Conservar ambos» con «Hacer esto con todos» → `answer_job(id, Answer { KeepBoth, all: true })`), `errors_view_and_retry`, `not_enough_space_text`, `pause_resume_cancel_buttons`, `closing_window_keeps_jobs_and_indicator`, `indicator_click_reopens`, `queue_empty_hides_window`, `paste_uses_engine`, `drop_uses_engine`, `undo_move_uses_engine`, `restore_uses_engine_on_mac`, `confirm_close_with_jobs`.
- [ ] **Step 2:** `cargo test -p xdir-core transfer view` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): ventana de copia con cola, conflictos, errores y gráfico de velocidad`.

### Task 12: Ventana de copia en Windows

**Files:**
- Create: `crates/xdir-windows/src/auxwin.rs` (siguiendo `quicklook/window.rs`).
- Modify: `crates/xdir-windows/src/{main.rs,app/host_win.rs,app/messages.rs}`.

**Interfaces:**
- Consumes: `AuxWindow`, `App::{paint_aux, aux_mouse, aux_key, aux_closed}` (Task 11).
- Ventana `xdir.aux`: título del sistema con cerrar y minimizar (`WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX`), dueña la principal, sin redimensionar (el alto lo pone `aux_resize`), modo oscuro del título con DWM como `quicklook::apply_dwm`, posición inicial abajo a la derecha del monitor de xdir (como la de Windows) y recordada mientras dure la sesión; un `Renderer` propio que llama `App::paint_aux`; `WM_MOUSEMOVE`/`LBUTTONDOWN`/`LBUTTONUP`/`MOUSELEAVE` → `aux_mouse`; `WM_KEYDOWN` → `aux_key` (Esc = cerrar); `WM_CLOSE` → `aux_closed` + `ShowWindow(SW_HIDE)`; `WM_DPICHANGED` y tema como la vista rápida. Se destruye al cerrar xdir (`shutdown`).
- [ ] **Step 1: Tests que fallan** (CI de Windows, funciones puras): `initial_position_bottom_right` (`fn initial_rect(work: RECT, size: SIZE) -> RECT`, margen 16), `client_to_window_size` (con el borde del sistema).
- [ ] **Step 2:** `cargo check --target x86_64-pc-windows-msvc --all-targets` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en CI; check también `--no-default-features`; lista de prueba manual en el PR (copia de 5 GB a un pendrive con pausa, cancelar, conflicto con «Combinar», minimizar y reabrir desde la barra, cerrar xdir copiando).
- [ ] **Step 5: Commit** `feat(windows): ventana de copia`.

### Task 13: Ventana de copia en macOS

**Files:**
- Create: `crates/xdir-mac/src/auxwin.rs`.
- Modify: `crates/xdir-mac/src/{main.rs,app/host_mac.rs,app/mac.rs}`.

**Interfaces:**
- Consumes: lo mismo que la Task 12.
- `NSWindow` con `Titled | Closable | Miniaturizable`, `setReleasedWhenClosed(false)`, `setHidesOnDeactivate(false)`, apariencia del tema de xdir; vista propia `AuxView` (`define_class!` como `XdirView`) que dibuja con el `Renderer` de macOS llamando `App::paint_aux`, con `NSTrackingArea` para el hover, `mouseDown/Up/Moved/Exited` → `aux_mouse`, `keyDown` → `aux_key`; delegado `windowShouldClose` → `aux_closed` + `orderOut` y devuelve false; posición inicial abajo a la derecha de la pantalla de xdir, margen 16.
- [ ] **Step 1: Tests que fallan**: `initial_frame_bottom_right` (función pura `initial_frame(visible: NSRect, size) -> NSRect` en coordenadas de AppKit).
- [ ] **Step 2:** `cargo test -p xdir-mac auxwin` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac (copia aislada): copiar 3 GB del scratchpad a una imagen `hdiutil` (otro volumen) con pausa y reanudar; conflicto de carpeta con «Combinar» y «Hacer esto con todos»; mover entre discos y verificar que el origen quede vacío; cerrar la ventana y reabrir desde la barra; cerrar xdir copiando → pregunta; capturas contra las pantallas 1–3 de la tanda D.
- [ ] **Step 5: Commit** `feat(mac): ventana de copia`.

---

## PR 4 — Comprimir y extraer

### Task 14: Crate `xdir-archive` y reglas de seguridad

**Files:**
- Create: `crates/xdir-archive/Cargo.toml`, `crates/xdir-archive/src/lib.rs`, `src/zip.rs`, `src/targz.rs`, `src/tests.rs`; `crates/xdir-core/src/archive.rs` (tipos y reglas puras).
- Modify: `crates/xdir-core/src/{lib.rs,sys/mod.rs,sys/testing.rs}` (`sys::Archives`, `FakeArchives`).

**Interfaces:**
- Produces (núcleo, `archive.rs`): `pub enum ArchiveFormat { Zip, TarGz }` y `pub fn format_of(name: &str) -> Option<ArchiveFormat>` (`.zip`; `.tar.gz`/`.tgz` sin distinguir mayúsculas); `pub struct ArchiveEntry { pub index: usize, pub name: String, pub size: u64, pub kind: ItemKind, pub link: Option<String>, pub modified: u64 }`; `pub fn safe_entry_path(name: &str) -> Option<PathBuf>` (`None` para absolutas, con unidad o UNC, con `..` que salga, vacías; `\` cuenta como separador); `pub fn symlink_inside(entry: &Path, target: &str) -> bool`; `pub fn archive_name(names: &[&str]) -> String` (uno → «informe.pdf.zip» / «Fotos.zip», como el Finder; varios → «Archivo.zip»); `pub fn extract_folder_name(archive: &str) -> String` («fotos.zip» → «fotos», «a.tar.gz» → «a»).
- Produces (`sys`): `pub trait Archives: Send + Sync { fn formats(&self) -> &'static [ArchiveFormat]; fn list(&self, archive: &Path) -> Result<Vec<ArchiveEntry>, CopyError>; fn extract(&self, archive: &Path, entry: &ArchiveEntry, dst: &Path, progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError>; fn create(&self, out: &Path, items: &[(PathBuf, String)], progress: &mut dyn FnMut(u64) -> Flow) -> Result<(), CopyError>; }` (`create`: `items` = (archivo en disco, nombre dentro con `/`); carpetas vacías con nombre terminado en `/`).
- Produces (`xdir-archive`, depende de `xdir-core`): `pub struct Service { pub tar_gz: bool }` que implementa `Archives` (Windows `tar_gz: false`). Zip: store + deflate, zip64 para > 4 GB, nombres UTF-8 si traen la marca y si no CP437 (lo hace la crate `zip`; verificarlo con el test), contraseña → `ErrKind::Other("Este zip tiene contraseña")`, método no soportado → `Other("Formato no soportado")`; `extract` de un `.tar.gz` recorre hasta la entrada (aceptable: se extrae en orden). `create` escribe en `out` creado con `create_new`; `Stop` → borra `out`.
- Errores de E/S de la crate → `ErrKind` con la misma tabla que las primitivas (función `kind_of(io::Error)`).

- [ ] **Step 1: Tests que fallan**: núcleo `safe_entry_rejects_escapes` («/etc/x», «C:\\x», «\\\\srv\\x», «a/../../x», «..», «» → `None`; «a/./b», «a/b/../c» → `Some`), `symlink_inside_only`, `archive_and_folder_names`, `format_detection`; crate `zip_roundtrip` (carpeta con 3 archivos y una subcarpeta vacía → `create` → `list` → `extract` igual), `zip_progress_and_stop`, `zip_cp437_names` (un zip mínimo armado en el test sin la marca UTF-8 con «Ñ» en CP437 → «Ñ»), `zip_password_error`, `zip64_large_entry_listed` (entrada marcada > 4 GB en un zip armado a mano, solo `list`), `targz_roundtrip` (con enlace interno).
- [ ] **Step 2:** `cargo test -p xdir-core archive` y `cargo test -p xdir-archive` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS (también en Docker y `cargo check` para Windows); Global Constraints.
- [ ] **Step 5: Commit** `feat: crate xdir-archive para zip y tar.gz`.

### Task 15: Comprimir y extraer en el motor y en los menús

**Files:**
- Modify: `crates/xdir-core/src/transfer/{mod.rs,engine.rs,engine_tests.rs}` (`JobSpec::Compress`, `JobSpec::Extract`), `commands.rs`, `app/{fileops,actions,transfers}.rs`; `crates/{xdir-windows,xdir-mac,xdir-linux}/Cargo.toml` y sus `install` (`archives: Box::new(xdir_archive::Service { tar_gz: … })`).

**Interfaces:**
- Consumes: `Archives`, `safe_entry_path`, `symlink_inside`, `archive_name`, `extract_folder_name` (Task 14); el motor (Task 7).
- Produces: `JobSpec::Compress { sources: Vec<PathBuf> }` (zip en la carpeta del primero, nombre `archive_name` libre con `numbered_name`; título «Comprimiendo N elementos»; al terminar `select` el zip, `undo: Created([zip])`); `JobSpec::Extract { archive: PathBuf, into_folder: bool }` (título «Extrayendo «x»»; `into_folder` crea la carpeta `extract_folder_name` libre (« 2») y extrae ahí sin conflictos; si no, extrae en la carpeta del archivo con los conflictos del motor; entradas con `safe_entry_path == None` o enlaces que salen → `JobError` «Ruta no permitida dentro del archivo» y se siguen; los enlaces del archivo los crea `Archives::extract` solo fuera de Windows y solo si `symlink_inside` (en Windows el `Service` devuelve `ErrKind::Other("Enlace omitido")` y el motor lo anota como `JobError`); cancelar borra la entrada a medias; `undo: Created(lo de arriba)`).
- Menú y paleta: «Comprimir» (cualquier selección con ruta); con un solo archivo de formato soportado en este sistema, «Extraer aquí» y «Extraer en "nombre"» (con el nombre real; sin puntos suspensivos). Comandos `comprimir` «Comprimir», `extraer_aqui` «Extraer aquí», `extraer_en` «Extraer en una carpeta» (la paleta no sabe el nombre). Zip con contraseña o formato no soportado: el error de la fila y aviso «Abrir con el sistema» que llama `sys::shell().open_item`.

- [ ] **Step 1: Tests que fallan** (con `FakeArchives` y `FakeCopyPrims`): `compress_one_and_many_names`, `compress_cancel_removes_partial`, `extract_into_folder_numbered`, `extract_here_conflicts` (una entrada que ya existe → `Update::Conflict`), `extract_skips_escaping_entries` (nada fuera del destino; `JobError` con el texto), `extract_progress_by_bytes`, `menu_extract_shows_real_name` («Extraer en "fotos"»), `targz_not_offered_on_windows_style` (`FakeArchives` con solo `Zip`), `password_zip_offers_system_open`.
- [ ] **Step 2:** `cargo test -p xdir-core compress extract` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: comprimir una carpeta de 1 GB (fila «Comprimiendo…» con pausa), «Extraer en "x"» y «Extraer aquí» con un conflicto, un `.tar.gz`; abrir el zip con el Finder para confirmar que es válido.
- [ ] **Step 5: Commit** `feat: comprimir y extraer con avance en la ventana de copia`.

---

## PR 5 — Renombrar varios

### Task 16: Cálculo de nombres y choques

**Files:**
- Create: `crates/xdir-core/src/rename_many.rs`.
- Modify: `crates/xdir-core/src/lib.rs`, `transfer/engine.rs` (`rename_all`), `ops.rs` (`Undo::Renames`); las 3 plataformas (rama de `Undo::Renames` que devuelve error «se deshace en el núcleo»: nunca les llega, ver Task 17).

**Interfaces:**
- Produces: `pub enum Mode { Replace { find: String, with: String }, Add { text: String, before: bool }, Format { style: FormatStyle, base: String, before: bool, start: u32 } }`, `pub enum FormatStyle { Number, Counter, Date }`; `pub struct Source { pub name: String, pub is_dir: bool, pub modified_day: (i32, u8, u8) }`; `pub enum Clash { Duplicate, Exists, Empty, Invalid(char) }`; `pub fn compute(items: &[Source], mode: &Mode) -> Vec<String>` (sobre el nombre sin extensión — `names::split_ext` —, la extensión igual; Reemplazar: todas las apariciones, distinguiendo mayúsculas; Número: «base 1», Contador: «base 001» con ceros hasta el ancho de `start + n - 1` y mínimo 3, Fecha: «base 2026-11-10»; `before` pone el número/fecha antes: «1 base»; `base` vacío en Formato = nombre original); `pub fn clashes(old: &[String], new: &[String], others: &[String], case_insensitive: bool, invalid: &[char]) -> Vec<Option<Clash>>` (`others` = nombres de la carpeta que no se renombran; un nombre que solo cambia mayúsculas no choca consigo mismo); `pub const INVALID_WINDOWS: &[char] = &['\\','/',':','*','?','"','<','>','|']; pub const INVALID_UNIX: &[char] = &['/'];` (cuál usar lo da `FileSystem::invalid_name_chars(&self) -> &'static [char]`, nuevo, por defecto `INVALID_UNIX`).
- Produces (motor): `pub fn rename_all(prims: &dyn CopyPrims, pairs: &[(PathBuf, String)]) -> Result<Undo, String>`: primero cada uno a un temporal `.xdir-ren-<pid>-<i>` en su carpeta con `rename_noreplace`, después al final; si algo falla, deshace lo hecho en orden inverso y devuelve el error. Devuelve `Undo::Renames(Vec<(PathBuf /*ahora*/, String /*nombre anterior*/)>)`.

- [ ] **Step 1: Tests que fallan**: `replace_text_all_occurrences` («IMG_2041.jpg», buscar «IMG_» con «Viaje-» → «Viaje-2041.jpg»; mayúsculas distintas no reemplaza), `add_text_before_after`, `format_number_counter_date` (contador desde 1 con 12 elementos → «base 001»…«base 012»; desde 998 con 5 → ancho 4), `extension_kept_and_dirs_without_ext`, `clash_duplicate_and_exists`, `clash_empty_and_invalid` (Windows «a:b» → `Invalid(':')`), `rename_many_case_only_change` (`case_insensitive` y «a.txt» → «A.txt» sin choque), `rename_all_cross_names` (a→b y b→a con `FakeCopyPrims` → intercambiados), `rename_all_rolls_back` (falla el tercero → los dos primeros vuelven), `undo_renames_roundtrip`.
- [ ] **Step 2:** `cargo test -p xdir-core rename_many` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): cálculo de Renombrar varios con choques y cruce seguro`.

### Task 17: Panel de Renombrar varios y vista previa en la lista

**Files:**
- Create: `crates/xdir-core/src/app/rename_many.rs` (estado y teclas), `app/rename_many_tests.rs`, `crates/xdir-core/src/rename_many_draw.rs` (dibujo y hit-test del panel).
- Modify: `app/preview.rs` (el lugar de la derecha pasa a tener dueño: `RightPanel`), `list/mod.rs` (columna Nombre con el nombre nuevo), `commands.rs` (`Command::RenameMany`), `app/{actions,fileops,input,paint,events}.rs`, `session.rs` (nada nuevo), `sys/mod.rs` (`invalid_name_chars`) y las plataformas.

**Interfaces:**
- Consumes: `compute`, `clashes`, `rename_all`, `Undo::Renames` (Task 16).
- Produces (`app/preview.rs`): `pub(super) enum RightPanel { Preview, Info, RenameMany }`; el panel derecho se ve si la vista previa está prendida o si el dueño no es `Preview`; usa el mismo ancho y borde (`preview_width`, `Drag::PreviewEdge`); al cerrar Info o Renombrar varios vuelve la vista previa si estaba prendida.
- Produces: `FileList::rename_preview: Option<Vec<(usize, String, Option<Clash>)>>` (posición en `entries`, nombre nuevo, choque): la columna Nombre (o la etiqueta en las vistas de íconos) dibuja el nombre nuevo con lo que cambia resaltado (`accent`), el viejo tachado al lado y, con choque, la fila en rojo (`danger` del tema) con el motivo en el cartel.
- Panel (`rename_many_draw.rs`): `pub fn draw(p, r, s: &PanelState, hover)`, `pub fn hit(r, s, x, y) -> Option<PanelHit>`; selector de modo («Reemplazar texto», «Agregar texto», «Formato») con `host.popup_menu`; campos con `host.text_create` (cada cambio — `WM_APP_TEXT_CHANGED` — recalcula); casillas «antes del nombre» / «después del nombre»; «Número inicial»; «Cancelar» y «Renombrar» (apagado con choques o sin cambios); título «Renombrar N elementos».
- Abrir: F2 (`Command::Rename`) con más de un seleccionado, menú «Renombrar N elementos…» y paleta (`renombrar_varios` «Renombrar varios…»). Los elementos y su orden son los de la lista visible en ese momento (con su orden y agrupación); si la carpeta se recarga, los que desaparecieron salen. Esc o «Cancelar» cierran; «Renombrar» corre `rename_all` en un hilo (`spawn_task`, nuevo `TaskDone::Renamed { result: Result<Undo, String> }`) y apila el deshacer; Ctrl+Z con `Undo::Renames` también va por `rename_all` en un hilo (no por `FileOps`).
- [ ] **Step 1: Tests que fallan**: `f2_with_many_opens_panel` (con uno, renombrado en línea como hoy), `panel_replaces_preview_and_restores_it`, `typing_updates_list_preview` (escribir en «Buscar» → `rename_preview` con los nuevos; `RecordingPainter` dibuja el nuevo y el viejo tachado), `clash_marks_row_and_disables_apply`, `numbering_follows_visible_order` (agrupada por tipo), `apply_renames_and_one_undo` (con `FakeCopyPrims`: Renombrar → nombres nuevos; Ctrl+Z → todos vuelven en un paso), `escape_cancels_and_clears_preview`, `rename_many_menu_label` («Renombrar 3 elementos…»).
- [ ] **Step 2:** `cargo test -p xdir-core rename_many` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 4 de la tanda D).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: 24 fotos con los tres modos, un choque marcado, aplicar y ⌘Z; comparar con las capturas.
- [ ] **Step 5: Commit** `feat: Renombrar varios en el panel derecho con vista previa en la lista`.

---

## PR 6 — Panel de información

### Task 18: Servicio de información y panel en el núcleo

**Files:**
- Create: `crates/xdir-core/src/info/mod.rs` (tipos, pedido, cambios, `merge`), `info/size.rs` (tamaño de carpetas), `info/draw.rs` (secciones, hit-test), `crates/xdir-core/src/app/info.rs`, `app/info_tests.rs`.
- Modify: `lib.rs`, `sys/mod.rs` (`trait Info`, `Services::info`, `sys::info()`), `sys/testing.rs` (`FakeInfo`), `commands.rs` (`Command::Properties` abre/cierra el panel), `session.rs` (`informacion_secciones`), `app/{actions,fileops,events,input,paint,preview}.rs`; `install` de las 3 plataformas con una implementación mínima (solo General con `std::fs`) hasta las Tasks 19–21.

**Interfaces:**
- Consumes: `RightPanel` (Task 17); `Painter::big_image` y `preview::Shown` de la tanda C para el encabezado; `Xtree::size_of`.
- Produces (`info/mod.rs`):
  - `pub enum Tri { Off, On, Mixed }`; `pub enum Flag { Locked, Hidden, ReadOnly }`.
  - `pub struct ItemInfo { pub path: PathBuf, pub kind_name: String, pub size: u64, pub on_disk: u64, pub created: u64, pub modified: u64, pub last_opened: u64, pub last_opened_is_access: bool, pub is_dir: bool, pub cloud_only: bool, pub tags: Option<Vec<Tag>> /* None: no admite */, pub comment: Option<String>, pub hidden_ext: Option<bool> /* solo macOS */, pub flags: Vec<(Flag, bool, bool /* editable */)>, pub open_with: Option<OpenWith>, pub perms: Option<Perms>, pub more: Vec<(&'static str, String)>, pub volume: Option<VolumeData> }`; `pub struct Tag { pub name: String, pub color: Option<u32> }`; `pub struct OpenWith { pub current: Option<AppRef>, pub apps: Vec<AppRef>, pub per_file: bool /* elegir queda para el archivo (macOS) o abre una vez */ }`; `pub struct AppRef { pub id: String, pub name: String, pub icon: i32 }`; `pub struct Perms { pub owner: String, pub group: String, pub mode: u32, pub editable: bool, pub can_read: bool, pub can_write: bool }`; `pub struct VolumeData { pub total: u64, pub free: u64, pub format: String }`.
  - `pub struct InfoRequest { pub hwnd: Handle, pub tx: Sender<InfoReply>, pub generation: u64, pub current: Arc<AtomicU64>, pub paths: Vec<PathBuf> }` con `is_stale()` y `send()` (como `PreviewRequest`; `WM_APP_INFO`).
  - `pub enum InfoReply { Items { generation: u64, items: Vec<ItemInfo> }, FolderSize { generation: u64, path: PathBuf, bytes: u64, files: u64, folders: u64, done: bool }, Applied { generation: u64, section: Section, error: Option<String> } }`; `pub enum Section { Tags, General, More, Name, Comments, OpenWith, Perms }` (orden de dibujo; también el bit de `informacion_secciones`).
  - `pub enum InfoChange { Tags { paths: Vec<PathBuf>, add: Vec<String>, remove: Vec<String> }, Comment { path: PathBuf, text: String }, Flag { paths: Vec<PathBuf>, flag: Flag, on: bool }, HideExt { path: PathBuf, on: bool }, OpenWith { paths: Vec<PathBuf>, app: String }, ChangeAll { path: PathBuf, app: String }, Perms { paths: Vec<PathBuf>, mode: u32, recursive: bool } }`.
  - `pub fn merge(items: &[ItemInfo]) -> Summary` (puro: «N elementos», tamaños sumados, tipo si es el mismo, cada casilla `Tri`, etiquetas comunes) con `pub struct Summary { pub count: usize, pub size: u64, pub on_disk: u64, pub kind: Option<String>, pub flags: Vec<(Flag, Tri, bool)>, pub tags: Vec<Tag> }`.
- Produces (`info/size.rs`): `pub fn folder_size(path: &Path, cloud_only: &dyn Fn(&Path) -> bool, ctl: &AtomicBool, step: &mut dyn FnMut(u64, u64, u64))` (sin entrar en carpetas solo en la nube; `step` cada 500 elementos para «Calculando… N elementos»).
- Produces (`sys`): `pub trait Info: Send + Sync { fn request(&self, req: InfoRequest); fn apply(&self, owner: Handle, tx: Sender<InfoReply>, generation: u64, change: InfoChange); fn more_in_system(&self, owner: Handle, items: &[Item]) -> bool; }` (`request` y `apply` en otro hilo; `more_in_system` = lo que hoy hace `Shell::properties`, false en Linux: el botón no se muestra).
- `App`: `Command::Properties` abre el panel (`RightPanel::Info`) o lo cierra si estaba abierto; sigue al panel activo como la vista previa (sujeto: la selección; sin selección, la carpeta; en «Este equipo», el disco enfocado), con `generation` y la pausa de 50 ms de la tanda C; ✕ y Esc (con foco en el panel) cierran. Tamaño de carpeta: al instante con `Xtree::size_of`, si no `folder_size` en un hilo. Secciones plegables con estado guardado; editar el nombre manda `Op::Rename`; los demás cambios `sys::info().apply` y el error se muestra en la sección. Nombre: «Propiedades» / «Obtener información» (macOS) como hoy; título «Información de «x»» / «Información de N elementos».

- [ ] **Step 1: Tests que fallan**: `properties_toggles_info_panel` (Alt+Enter abre, otra vez cierra; con la vista previa prendida vuelve la vista previa), `info_follows_active_pane`, `info_stale_reply_ignored`, `info_draws_all_sections` (`RecordingPainter` con un `ItemInfo` completo: «Etiquetas», «General», «Más información», «Nombre y extensión», «Comentarios», «Abrir con», «Compartir y permisos», «Más en el sistema…»), `info_tags_unsupported_text` («Este tipo de archivo no admite etiquetas»), `info_folder_size_calculating` («Calculando… 1.200 elementos» y luego el total; con xtree, al instante), `info_drive_shows_capacity`, `info_multi_merge_tri` (dos archivos, uno oculto → casilla `Mixed`; etiquetas comunes), `info_cloud_only_not_read` (`FakeFileSystem::set_cloud_only` → el falso no lee contenido y la fila dice «Disponible solo en línea»), `info_sections_persist` (sesión), `info_rename_from_panel` (`Op::Rename`), `info_apply_error_in_section`, `info_more_in_system_hidden_when_unsupported`, `folder_size_skips_cloud_dirs`.
- [ ] **Step 2:** `cargo test -p xdir-core info` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 5 de la tanda D).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con la implementación mínima (General, varios, carpeta, disco).
- [ ] **Step 5: Commit** `feat(core): panel de información`.

### Task 19: Información en macOS

**Files:**
- Create: `crates/xdir-mac/src/info.rs`.
- Modify: `crates/xdir-mac/src/{sys/macos.rs,main.rs,Cargo.toml}`.

**Interfaces:**
- Consumes: `Info`, `InfoRequest`, `InfoChange` (Task 18).
- Lectura: claves de `NSURL` (`NSURLLocalizedTypeDescriptionKey`, `…TotalFileAllocatedSizeKey`, `…CreationDateKey`, `…ContentModificationDateKey`, `NSURLTagNamesKey` + colores de las etiquetas del Finder, `NSURLHasHiddenExtensionKey`, `NSURLIsUserImmutableKey`, `NSURLIsHiddenKey`, volumen: `…VolumeTotalCapacityKey`, `…AvailableCapacityKey`, `…LocalizedFormatDescriptionKey`); `MDItemCopyAttribute` para `kMDItemFinderComment` y `kMDItemLastUsedDate`; «Más información» de los metadatos de la tanda B; permisos con `lstat` y `getpwuid`/`getgrgid`; «Abrir con»: `NSWorkspace URLForApplicationToOpenURL:` y `URLsForApplicationsToOpenURL:`. Nada de esto lee contenido (`SF_DATALESS` → «Disponible solo en línea» en lo que lo necesitaría).
- Escritura: etiquetas `setResourceValue:forKey:NSURLTagNamesKey`; Bloqueado `chflags(UF_IMMUTABLE)`; Oculto `chflags(UF_HIDDEN)`; ocultar extensión `NSURLHasHiddenExtensionKey`; comentario con `osascript` (`tell application "Finder" to set comment of (POSIX file "…" as alias) to "…"`, con el texto escapado por una función pura `applescript_string(s) -> String`); «Abrir con» por archivo: atributo `com.apple.LaunchServices.OpenWith` no es público — se usa `NSWorkspace setDefaultApplicationAtURL:toOpenFileAtURL:completionHandler:` (macOS 12+); «Cambiar todos…» `setDefaultApplicationAtURL:toOpenContentType:completionHandler:`; permisos `chmod`, recursivo en otro hilo juntando errores. «Más en el sistema…» = el `osascript` de hoy.
- [ ] **Step 1: Tests que fallan**: `applescript_string_escapes` (comillas y barras), `mac_info_reads_general` (archivo temporal: tamaño, fechas, tipo), `mac_tags_roundtrip`, `mac_hidden_flag_roundtrip`, `mac_perms_chmod_recursive` (carpeta temporal), `mac_volume_info_root`.
- [ ] **Step 2:** `cargo test -p xdir-mac info` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: ⌘I sobre un PDF (todas las secciones), etiqueta roja visible después en el Finder, comentario que el Finder muestra, «Cambiar todos…», carpeta de iCloud con elementos `dataless` sin bajarlos (`ls -lO` sigue `dataless`); capturas contra la pantalla 5.
- [ ] **Step 5: Commit** `feat(mac): información con etiquetas, comentarios, abrir con y permisos`.

### Task 20: Información en Windows

**Files:**
- Create: `crates/xdir-windows/src/info.rs`.
- Modify: `crates/xdir-windows/src/{sys/win32.rs,main.rs,Cargo.toml}`.

**Interfaces:**
- Consumes: lo mismo que la Task 19.
- Lectura: `GetFileAttributesExW`; tamaño en disco `GetCompressedFileSizeW` redondeado al clúster (`GetDiskFreeSpaceW`) — función pura `round_to_cluster(size, cluster) -> u64`; etiquetas `System.Keywords` y comentario `System.Comment` por `SHGetPropertyStoreFromParsingName` (`GPS_DEFAULT` para leer, `GPS_READWRITE` para escribir; nunca `GPS_OPENSLOWITEM`) — si el formato no las admite (`IPropertyStoreCapabilities::IsPropertyWritable` = S_FALSE) → `tags: None` / `comment: None`; nube (`OFFLINE`/`RECALL_*`) → no se abre el property store; último acceso como «Último acceso» (`last_opened_is_access`); volumen `GetVolumeInformationW` + `GetDiskFreeSpaceExW`; dueño `GetNamedSecurityInfoW` + `LookupAccountSidW`; lectura/escritura del usuario con `GetEffectiveRightsFromAclW` sobre el SID del token (sin abrir el archivo); «Abrir con» `SHAssocEnumHandlers(ASSOC_FILTER_RECOMMENDED)` (`per_file: false`).
- Escritura: Oculto y Solo lectura con `SetFileAttributesW`; etiquetas y comentario con `IPropertyStore::SetValue` + `Commit`; «Abrir con» elegir = `IAssocHandler::Invoke` con el `IDataObject` del archivo (abre una vez); «Cambiar todos…» = `SHOpenWithDialog` con `OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_FORCE_REGISTRATION | OAIF_EXEC`; permisos solo lectura (`editable: false`). «Más en el sistema…» = `SHObjectProperties`/`SHMultiFileProperties` de hoy.
- [ ] **Step 1: Tests que fallan** (CI de Windows): `round_to_cluster_values`, `win_info_reads_general`, `win_hidden_readonly_roundtrip`, `win_keywords_unsupported_on_txt` (`.txt` → `tags: None`), `win_owner_present`.
- [ ] **Step 2:** `cargo check --target x86_64-pc-windows-msvc --all-targets` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en CI; check también `--no-default-features`; lista de prueba manual en el PR (`.docx` con etiquetas y comentario, `.txt` sin etiquetas, archivo de OneDrive solo en línea, Alt+Enter en «Este equipo» sobre un disco, «Cambiar todos…»).
- [ ] **Step 5: Commit** `feat(windows): información con etiquetas, comentarios y abrir con`.

### Task 21: Información en Linux

**Files:**
- Create: `crates/xdir-linux/src/info.rs`.
- Modify: `crates/xdir-linux/src/{sys.rs,main.rs}`.

**Interfaces:**
- Consumes: lo mismo que la Task 19.
- Lectura: `lstat` (`st_blocks × 512` en disco); etiquetas `getxattr("user.xdg.tags")` (separadas por coma), comentario `user.xdg.comment` (sin el atributo → vacío; `ENOTSUP` → `None`); Oculto informativo (nombre con punto, no editable); Solo lectura = falta la escritura del dueño (editable); volumen `statvfs` + formato de `/proc/self/mountinfo` (función pura `fs_type_for(mountinfo: &str, path: &Path) -> Option<String>`); «Abrir con»: `xdg-mime query default <mime>` y las `.desktop` con ese tipo en `mimeinfo.cache` (`per_file: false`); dueño y grupo `getpwuid`/`getgrgid`.
- Escritura: `setxattr`/`removexattr`; `chmod` (recursivo juntando errores); «Cambiar todos…» `xdg-mime default <app>.desktop <mime>`; elegir una app = `gio launch` o `gtk-launch <app> <archivo>`. Sin «Más en el sistema…».
- [ ] **Step 1: Tests que fallan** (Docker): `linux_tags_xattr_roundtrip` (si el sistema de archivos del contenedor no soporta `user.*`, se indica y se saltea), `linux_readonly_toggles_owner_write`, `linux_fs_type_from_mountinfo`, `linux_mime_cache_apps` (un `mimeinfo.cache` de prueba).
- [ ] **Step 2:** `sh …/dk.sh cargo test -p xdir-linux info` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(linux): servicio de información`.

---

## Cierre

### Task 22: Atajos en la wiki, verificación final y versión de Mac

**Files:**
- Modify: wiki del repo (clonar el `.wiki.git` del remoto `origin` en el scratchpad; página de atajos); `docs/DISENO.md` (pantallas de la tanda D pasan a programadas — solo cuando el agente de diseño ya no la esté editando).

- [ ] **Step 1:** Wiki: Ctrl+D / ⌘D Duplicar; Ctrl+Shift+D / ⇧⌘D favoritos (antes Ctrl+D); Ctrl+Alt+Shift+N / ⌃⌘N carpeta nueva con la selección; Alt+Enter / ⌘I información (ahora panel); F2 con varios; ⌘⌫ en la Papelera devuelve; el modificador «Meta» en `[atajos]`; los comandos sin atajo. Commit en la wiki `docs: atajos de la tanda D`.
- [ ] **Step 2:** `docs/DISENO.md` actualizado; commit `docs: pantallas de la tanda D programadas`.
- [ ] **Step 3:** Verificación final: Mac, Docker, checks de los 3 sistemas, capturas de las 8 pantallas comparadas con el diseño, smoke del spec (copia grande con pausa y cancelar, conflicto, mover entre discos con un pendrive, expulsar).
- [ ] **Step 4:** Al mergear la última parte: `develop` actualizado, `sh scripts/package-mac.sh` y reemplazar `/Applications/xdir.app` (avisar si xdir está abierto).

## PRs

Rama `feature/tanda-d`, base `develop`; después de cada merge se trae `develop` y se sigue en la misma rama.

1. **Tasks 1–5** — Duplicar, carpeta nueva con la selección (+ Meta), Devolver, accesos directos, Expulsar (`feat: operaciones chicas de la tanda D`). Se puede cortar antes: Tasks 1–2 solas ya son un PR completo si la sesión no da para más; 3–5 van en un PR 1b.
2. **Tasks 6–10** — motor de copia y primitivas; Duplicar y carpeta nueva ya lo usan, el resto sigue con lo de hoy.
3. **Tasks 11–13** — ventana de copia; copiar, mover, pegar, soltar, Devolver y deshacer un movimiento pasan al motor.
4. **Tasks 14–15** — comprimir y extraer (`xdir-archive`).
5. **Tasks 16–17** — Renombrar varios.
6. **Tasks 18–21 + Task 22** — panel de información, wiki y versión de Mac.

Cada PR lleva la lista de pruebas manuales de Windows para Runick93.

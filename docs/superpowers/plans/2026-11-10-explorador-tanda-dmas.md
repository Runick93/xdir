# Tanda D+ (configuración y red) — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Una pestaña de Configuración propia (buscador, secciones, editor de atajos) que escribe `config.toml` sin perder lo escrito a mano, y carpetas de red de primera clase (Conectar a un servidor, unidades de red de Windows, «Red» en el árbol, desconectar con ⏏) sin que la red trabe nunca la ventana, en Windows, macOS y Linux (Linux sin interfaz: núcleo y servicios listos).

**Architecture:** Dos corrientes independientes, cada una en su rama y su PR, más un cierre. **S (configuración)**: un editor de TOML por líneas en el núcleo (`toml_edit.rs`), una `Location::Settings` que la pestaña dibuja con `settings::draw` en vez de su `FileList`, una tabla de opciones en el núcleo (de ella salen las secciones y el buscador) y el editor de atajos sobre `Keymap`; cada cambio se aplica en memoria y se escribe en un hilo, en cola. **N (red)**: tipos puros `net::Address`, `NetError`, `red.toml` y un tiempo límite genérico en el núcleo; un servicio `sys::Network` (NetFS + Network.framework en macOS, WNet en Windows, `gio` en Linux) que solo corre en hilos de trabajo, salvo `is_remote` (sin E/S), que usa la auditoría del hilo de interfaz; paneles dibujados para conectar y para las unidades de Windows; «Red» en el árbol; desconectar por el flujo de Expulsar de la tanda D. La sección «Red» de la configuración la escribe N como tres funciones con la firma de la API de páginas de S; el cierre las enchufa.

**Tech Stack:** Rust 2024; `xdir-core` sin dependencias; `windows` 0.62 (`Win32_NetworkManagement_WNet` ya está); macOS `objc2` 0.6, `block2`, `dispatch2`, NetFS.framework y Network.framework enlazados con `#[link(kind = "framework")]` y `extern "C"` escritos a mano (un crate de bindings nuevo solo preguntando antes); Linux `gio` como proceso hijo (sin enlazar).

**Spec:** `docs/superpowers/specs/2026-11-10-explorador-tanda-dmas-design.md` (manda su sección «Decisiones»). Diseño: pantallas «Tanda D+» de `docs/design/xdir.figcraft` y su sección en `docs/DISENO.md` (las está terminando otro agente: leerlas al empezar cada tarea de interfaz, no editarlas hasta la Task F1).

## Global Constraints

- `xdir-core` sin dependencias ni `#[cfg]` de sistema en producción; lo de cada sistema detrás de `xdir_core::sys` (`Network` nuevo, con `FakeNetwork` en `sys/testing`) o `WindowHost`. `toml_edit`, `net::Address` y `red.toml` son lógica pura del núcleo con tests. Tests de `App` con `install_fakes()`, `FakeHost`, `RecordingPainter`.
- Las 3 plataformas: cada servicio se implementa en Windows, macOS y Linux; Linux (sin interfaz) compila y prueba servicios y lógica.
- El hilo de interfaz nunca espera a la red ni al disco: conectar, listar conexiones, desconectar, descubrir, recursos de una máquina y escribir `config.toml` / `red.toml` corren en hilos de trabajo (`spawn_task`) y vuelven por `TaskDone`. Tiempos límite: 30 s conectar, 10 s `shares` y `connections`, 15 s desconectar; al vencer, el hilo se abandona y el resultado se descarta. Al salir no se espera a ningún hilo de red.
- Nunca guardar contraseñas: ni en `red.toml`, ni en la sesión, ni en avisos o mensajes de error; `usuario:clave@` se guarda como `usuario@`. En Linux la clave va del campo al `stdin` de `gio` y la memoria se pisa al soltarla.
- Nunca perder lo escrito a mano en `config.toml`: solo cambian las líneas de lo que se tocó; comentarios, orden, claves y secciones desconocidas y líneas con error quedan byte por byte; se conserva `\n`/`\r\n` y la falta de salto final; escritura atómica (temporal en la misma carpeta + renombrar).
- Castellano en toda la interfaz y los errores (textos exactos del spec: «No se encuentra el servidor "nas"», «Usuario o contraseña incorrectos», «El recurso "fotos" no existe en "nas"», «Se agotó el tiempo esperando a "nas"», «Ya hay una conexión con "nas" con otro usuario», «Hace falta gvfs para conectarse a servidores», «En este sistema no se puede conectar a direcciones sftp://»).
- Atajos (spec §C): `configuracion` Ctrl+Coma (⌘, en macOS, sin tocar `KEYS`); `configuracion_atajos` sin atajo; `abrir_configuracion` pasa a llamarse «Abrir config.toml» (mismo id); `conectar_servidor` Ctrl+K (⌘K); `conectar_unidad_red` y `desconectar_unidad_red` sin atajo y solo Windows (`command_available` false en macOS y Linux); `expulsar` también desconecta. Lo puesto en `[atajos]` sigue ganando.
- `cargo test` en la Mac y en Linux con Docker (`sh /private/tmp/claude-501/-Volumes-Javier-Projects-javier-xdir/9ac1facf-f0c7-4c38-86cd-65b40a7300de/scratchpad/dk.sh cargo test`; si no existe, recrearlo como en la tanda C) sin bajar; `cargo check --all-targets` sin errores ni warnings nuevos para la Mac, `x86_64-pc-windows-msvc` (también `--no-default-features`) y `x86_64-unknown-linux-gnu`; clippy sin avisos nuevos; CI verde.
- Smoke en la Mac al final de cada tarea de interfaz: `cargo build --release -p xdir-mac --features pruebas`, copia aislada con `xdir.portable` en `…/scratchpad/vis/<tarea>/` y `vis/run.sh`, capturas comparadas con las pantallas de la tanda D+; nunca `/Applications/xdir.app` ni la configuración real del usuario; nunca `osascript quit`. Para red: el NAS de la casa (credenciales del Llavero, nunca escritas en archivos ni en el chat).
- No `cargo fmt` del repo. Conventional Commits (tipo y scope en inglés, descripción en castellano) sin atribución a Claude; `git add` por nombre.

## Review Focus

- **`config.toml` editado a mano con la pestaña abierta** (cambió en disco entre dos cambios de la pestaña, con `\r\n`, sin salto final y `[atajos]` repetida): lo del usuario y lo de la pestaña quedan los dos → `edit_after_external_change_keeps_both` (Task S3), `crlf_and_no_final_newline_kept`, `repeated_section_edits_last` (Task S1).
- **Varios cambios seguidos** (prender y apagar un switch rápido, reasignar dos atajos al hilo): el archivo final refleja el último estado, sin escrituras cruzadas → `config_writes_are_serialized` (Task S3).
- **Grabar la misma combinación que ya tiene la fila, o una de su propio otro chip**: no es choque ni pregunta, no cambia nada → `record_own_combo_is_noop` (Task S5).
- **Una dirección con contraseña** (`smb://ana:secreto@nas/x`) escrita en el panel, en la barra o en `red.toml` a mano: nunca aparece en `red.toml`, recientes, sesión, avisos ni errores → `password_never_persisted` (Task N1), `connect_dialog_strips_password` (Task N4).
- **La red se cae con una carpeta remota abierta** (y otra pestaña, xtree o la vista previa sobre ella): la ventana responde, la pestaña dice «"nas" no responde», «Cerrar la pestaña» funciona y desconectar suelta todo → `hung_load_shows_not_responding_and_close_works` (Task N5), `disconnect_releases_tabs_xtree_preview` (Task N6).

---

## Corrientes, ramas y archivos compartidos

- **S** en `feature/dmas-config`, **N** en `feature/dmas-red`, las dos desde `develop` y en paralelo; un PR cada una contra `develop` (se pueden cortar en dos: S1–S4 / S5–S6; N1–N6 / N7–N11). La que se mergea **segunda** trae `develop`, resuelve los choques de abajo y agrega lo que falte de «Interfaces S↔N». **F1** va al final, en `feature/dmas-cierre`, con las dos mergeadas.
- **Archivos de S**: `toml_edit.rs`, `settings/**`, `app/settings.rs`, `app/settings_pages.rs`, `app/settings_tests.rs`, `config.rs`, `session.rs`, `folder_views.rs`, `fs.rs` (`Location`), `pane.rs`, `ui.rs` (rueda), `xtree/view.rs`, `crates/xdir-mac/src/app/mac.rs`.
- **Archivos de N**: `net/**`, `app/network.rs`, `app/network_page.rs`, `app/network_tests.rs`, `tree.rs`, `list/mod.rs`, `app/edit.rs`, `app/transfers.rs`, `app/saved_groups.rs`, `palette.rs`, `sys/testing.rs` (+ `sys/testing/net_fake.rs`), los `net` de las 3 plataformas, sus `fs` (quitar `connect_share`), `scripts/package-mac.sh`.
- **Compartidos y cómo no chocar** (cada corriente escribe en su propio bloque):
  - `commands.rs`: S agrega `Settings`, `SettingsKeys` después de `OpenConfig` en el `enum` y sus `spec` justo después de la de `abrir_configuracion`; N agrega `ConnectServer`, `MapNetworkDrive`, `DisconnectNetworkDrive` después de `Eject` en el `enum` y sus `spec` justo después de la de `expulsar`. La tabla de grupos (`group`) es de S.
  - `app/mod.rs`: S declara `mod settings; mod settings_pages;` (+ tests) después de `mod saved_groups;` y sus campos después de `about_open`; N declara `mod network; mod network_page;` (+ tests) después de `mod info;` y su campo `net: network::NetState` después de `transfers`.
  - `app/tasks.rs`: S agrega su variante **primera** en `TaskDone` y su brazo primero en `on_task_done`; N agrega las suyas **al final**.
  - `app/actions.rs` (`run_command`, `on_key`): S pone sus brazos junto al de `Command::OpenConfig` y la captura de teclas al principio de `on_key`; N pone los suyos junto al de `Command::Eject`.
  - `app/events.rs`: N suma brazos de mensajes; S no.
  - `tab.rs`: S agrega `pub settings: Option<SettingsView>` después de `pub search` y toca `start`/`set_visible`; N agrega sus campos después de `left_view` y toca `new`, `navigate`, `open`, `retry_with_credentials`.
  - `sys/mod.rs`: S suma métodos con defecto a `SystemEnv`; N suma el trait `Network`, `Services::network` y `sys::network()`, y quita `FileSystem::connect_share`.
  - `crates/xdir-mac/src/sys/macos.rs`: S solo el `impl SystemEnv` (métodos nuevos); N `command_available` y el `install`.
  - `Location`: N nunca hace `match` exhaustivo sobre `Location` (usa `if let Location::Path` o brazo `_`), así `Location::Settings` no le rompe nada.

### Interfaces S↔N

- **Páginas extra de la configuración** (S produce en `app/settings_pages.rs`, N consume las firmas):
  ```rust
  pub(super) struct ExtraPage {
      pub id: &'static str,          // "red" (también el ancla de la sesión: xdir:configuracion#red)
      pub title: &'static str,       // "Red"
      pub after: Section,            // va después de esta sección en la lista
      pub search: fn(&App) -> Vec<SearchItem>,
      pub draw: fn(&mut App, &dyn Painter, Rect, Option<(f32, f32)>) -> f32,   // alto del contenido
      pub click: fn(&mut App, f32, f32) -> bool,                               // true: lo atendió
      pub field_done: fn(&mut App, TextBox, bool /* Enter */) -> bool,          // campos propios de la página
  }
  pub(super) const PAGES: &[ExtraPage] = &[];
  ```
  N escribe en `app/network_page.rs` `pub(super) fn search(app: &App) -> Vec<SearchItem>`, `draw`, `click`, `field_done` con esas firmas (sin depender de `ExtraPage`); si S ya está en `develop` cuando N termina, N mismo agrega `ExtraPage { id: "red", title: "Red", after: Section::Keys, … }` a `PAGES`; si no, lo hace F1. `SearchItem` (de S, `settings/mod.rs`): `pub struct SearchItem { pub title: String, pub desc: String, pub keywords: Vec<String> }`; si S no está, N define un `struct SearchItem` local con los mismos campos en `network_page.rs` y F1 lo reemplaza por el de S.
- **Grupos de atajos**: `commands::group(c: Command) -> Group` es un `match` exhaustivo (sin `_`). La corriente que se mergea segunda agrega `Command::ConnectServer | Command::MapNetworkDrive | Command::DisconnectNetworkDrive => Group::Network`.
- **`App::eject(root)`** (N la cambia): sobre una conexión de red desconecta. S no la toca.

---

## Corriente S — Configuración

### Task S1: Editor de `config.toml` por líneas

**Files:**
- Create: `crates/xdir-core/src/toml_edit.rs`.
- Modify: `crates/xdir-core/src/lib.rs`, `toml.rs` (nada de la API; solo si hace falta exponer `strip_comment` como `pub(crate)`).

**Interfaces:**
- Produces:
  - `pub fn set(text: &str, section: &str, key: &str, value: &str) -> String` (`value` ya en TOML: `"\"Ctrl+K\""`, `"true"`, `"[\"Ctrl+K\", \"F3\"]"`). Reglas del spec §A.9 en orden: (1) última línea activa `key = …` de la última `[section]` (no `[[section]]`): se cambia solo el valor, quedan sangría, comillas de la clave y comentario final (con los espacios que lo separaban); (2) si no, debajo del ejemplo comentado `# key = …` de esa sección (el ejemplo queda); (3) si no, después de la última línea activa de la sección o del encabezado; (4) sin sección: al final `\n[section]\n` y la línea. Clave escrita como en `toml::parse` la entiende: suelta si es `[A-Za-z0-9_-]+`, si no con `toml::quote`.
  - `pub fn remove(text: &str, section: &str, key: &str) -> String` (todas las líneas activas de esa clave en la sección; el ejemplo comentado queda).
  - `pub fn remove_all(text: &str, section: &str) -> String` (todas las líneas activas de la sección; comentarios y encabezado quedan).
  - `pub fn str_value(items: &[String]) -> String`: 0 → `""` entre comillas (`"\"\""`), 1 → `"\"x\""`, varios → `["a", "b"]`, con `toml::quote`.
  - Fin de línea: el del archivo (`\r\n` si la primera línea lo tiene); sin salto final se agrega la línea sin dejar uno nuevo al final del texto original. Líneas con error de `parse` y secciones desconocidas nunca cambian.

- [ ] **Step 1: Tests que fallan** (`toml_edit::tests`): `set_replaces_value_keeps_comment` (`"[opciones]\nanimaciones = true   # mío\n"`, `set(…, "opciones", "animaciones", "false")` → `"[opciones]\nanimaciones = false   # mío\n"`), `set_quoted_key` (`"\"*.pdf\" = \"#E5534B\""` en `[colores]`), `set_below_commented_example` (plantilla: `set(t, "atajos", "duplicar", "\"Ctrl+Shift+K\"")` → la línea nueva justo debajo de `# duplicar …`, el ejemplo sigue), `set_appends_in_section` y `set_creates_section_at_end`, `crlf_and_no_final_newline_kept` (texto `\r\n` sin salto final → todas las líneas con `\r\n`, sigue sin salto final), `repeated_section_edits_last` (dos `[atajos]` con la clave en las dos → cambia la de la segunda), `error_lines_untouched` (`"[atajos]\nbasura sin igual\nx = 1\n"` → `basura sin igual` igual byte por byte), `remove_then_original` (`remove(set(t, s, k, v), s, k) == t` para la plantilla con un atajo nuevo), `remove_all_keeps_comments`, `template_roundtrip_one_change` (`Config::parse(set(template(), "atajos", "duplicar", "\"Ctrl+Shift+K\""))` = la de la plantilla salvo `duplicar`), `str_value_forms`.
- [ ] **Step 2:** `cargo test -p xdir-core toml_edit` → FAIL (módulo no existe).
- [ ] **Step 3:** Implementar sobre las líneas del texto, usando `toml::parse` para ubicar secciones y claves (sus números de línea) y la detección de `# key =` con el mismo recorte de comentario que `toml.rs`.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): editor de config.toml que conserva lo escrito a mano`.

### Task S2: La pestaña Configuración (`Location::Settings`)

**Files:**
- Create: `crates/xdir-core/src/settings/mod.rs` (`Section`, `SettingsView`, `SearchItem`), `settings/draw.rs` (marco: buscador, lista de secciones, contenido con desplazamiento; `SettingsHit`), `crates/xdir-core/src/app/settings.rs`, `app/settings_pages.rs`, `app/settings_tests.rs`.
- Modify: `fs.rs` (`Location::Settings`: `parent` None, `title` «Configuración», `crumbs` [«Configuración», sección], `target` None), `session.rs`, `folder_views.rs` (`folder_key` → `"configuracion"`, nunca se guarda vista), `tab.rs` (`start`/`set_visible`: con `Settings` no carga ni vigila; `settings` se crea al construir), `pane.rs` (dibuja `settings::draw` en vez de la lista; migas a la sección; barra de dirección no editable; Atrás/Adelante/Subir apagados; ícono de engranaje en la pestaña), `commands.rs`, `ui.rs` (`PickerHit::MoreOptions` «Más opciones…» al pie, al lado de «Acerca de»), `app/{mod,actions,input,paint,panes}.rs`, `crates/xdir-mac/src/app/mac.rs` (menú xdir → «Configuración…» corre `Command::Settings`).

**Interfaces:**
- Produces: `Location::Settings`; `pub enum Section { Appearance, ListViews, Tree, Preview, Keys, Xtree, Advanced }` con `id()` (`"apariencia"`, `"lista"`, `"arbol"`, `"vista_previa"`, `"atajos"`, `"xtree"`, `"avanzado"`) y `title()` («Apariencia», «Lista y vistas», «Árbol», «Vista previa», «Atajos de teclado», «xtree», «Avanzado»); la lista de la izquierda = estas en orden con las `PAGES` insertadas después de su `after` («Red» queda entre Atajos y xtree). `pub struct SettingsView { pub page: PageId, pub scroll: f32, pub query: String, pub recording: Option<keys::Recording> /* S5 */ }`, `pub enum PageId { Builtin(Section), Extra(&'static str) }`.
- Produces (sesión): `location_str(Settings)` = `"xdir:configuracion"`; `Session::settings_page: Option<String>` se escribe pegado como `"xdir:configuracion#atajos"` y `location_from` lo separa (página desconocida → Apariencia). Ruling: `Location` queda sin datos y la sección viaja aparte, con el texto en disco que pide el spec.
- Produces (`App`): `pub(super) fn open_settings(&mut self, page: Option<PageId>)` — busca una pestaña `Settings` en todos los paneles: si hay, activa ese panel y pestaña (y la página, si se pidió); si no, pestaña nueva en el panel activo (no reemplaza la carpeta). Comandos `spec(Command::Settings, "configuracion", "Configuración", &["Ctrl+Coma"])`, `spec(Command::SettingsKeys, "configuracion_atajos", "Configuración: Atajos de teclado", &[])`; `abrir_configuracion` se renombra «Abrir config.toml». `Ctrl+L` / `EditAddress` con la pestaña activa enfoca el buscador. Los comandos de carpeta (copiar, pegar, vistas, ordenar, filtros, vista previa…) no hacen nada con ella (`Location::target()` None ya cubre la mayoría; revisar `run_command`).
- Produces: el buscador es `host.text_create(TextKind::Filter, …)` mientras la pestaña está a la vista en su panel; cambia `SettingsView::query` (S6 usa el texto).
- Produces: `ExtraPage` y `PAGES` vacía (ver «Interfaces S↔N»).

- [ ] **Step 1: Tests que fallan** (`app/settings_tests.rs`): `settings_opens_once` (`run_command(Settings)` dos veces → una sola pestaña `Settings`; con la carpeta del panel intacta en la otra pestaña), `settings_focuses_existing_in_other_pane` (está en el panel 2 y el activo es el 0 → activo pasa a 2 con esa pestaña), `settings_moves_not_duplicates` (`move_tab_to_pane` y abrir otra vez → sigue una), `settings_close_and_reopen` (Ctrl+W la cierra; «Reabrir» la vuelve a abrir), `settings_not_loaded_nor_watched` (`FakeFileSystem` sin `spawn_load` ni `watch` para ella), `settings_session_roundtrip` (`xdir:configuracion#atajos` en `sesion.toml` → al restaurar, `Settings` en la página Atajos; `"xdir:configuracion#nada"` → Apariencia), `settings_title_and_crumbs` («Configuración»; migas «Configuración › Atajos de teclado»; `can_back/can_up` false), `settings_keys_command_opens_keys_page`, `picker_more_options_opens_settings` (clic en `PickerHit::MoreOptions` → rueda cerrada y pestaña abierta), `folder_commands_ignored_in_settings` (Ctrl+C, Ctrl+Shift+1 → nada), `settings_command_shortcuts` (`combo_label_in(Mac, …)` de `configuracion` = «⌘,»; Windows «Ctrl+,»), `draw_settings_frame` (`RecordingPainter`: «Buscar en la configuración», las 7 secciones, título de la página).
- [ ] **Step 2:** `cargo test -p xdir-core settings` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 1 de la tanda D+, contenido de cada sección todavía vacío). Revisar cada `match` sobre `Location` del núcleo y de las 3 plataformas (`grep -rn "Location::Computer =>" crates`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: ⌘,, «Más opciones…», menú xdir → «Configuración…», arrastrar la pestaña al otro panel, ⌘W, reabrir xdir y que vuelva en la misma sección.
- [ ] **Step 5: Commit** `feat: pestaña de configuración`.

### Task S3: Apariencia, Lista y vistas, Árbol, Vista previa y la escritura de `config.toml`

**Files:**
- Create: `crates/xdir-core/src/settings/controls.rs` (switch, segmentado, deslizador, desplegable, fila de opción; dibujo + hit).
- Modify: `settings/{mod,draw}.rs`, `app/settings.rs`, `app/tasks.rs` (`TaskDone::ConfigSaved`, primera variante), `config.rs`, `folder_views.rs` (`set_default_view`), `app/{appearance,panes}.rs`.

**Interfaces:**
- Consumes: `toml_edit::{set, remove, remove_all, str_value}` (S1).
- Produces (`config.rs`): `pub enum ConfigEdit { Set { section: &'static str, key: String, value: String }, Remove { section: &'static str, key: String }, RemoveAll { section: &'static str } }`; `pub fn apply_edits(text: &str, edits: &[ConfigEdit]) -> String`; `pub fn write_edits(edits: &[ConfigEdit]) -> Result<Config, String>` (bloqueante, para hilos: lee el archivo — o `template()` si no existe —, aplica, escribe `config.toml.tmp` + `rename`, y devuelve `Config::parse` del texto escrito con su `modified`).
- Produces (`App`): `pub(super) fn edit_config(&mut self, edits: Vec<ConfigEdit>)` — una sola escritura en vuelo: si hay otra, se encolan (`config_pending`) y se escriben juntas al llegar `TaskDone::ConfigSaved { result: Result<Config, String> }`; con `Ok(c)` → `self.config = c` (así `reload_config` no avisa «Configuración aplicada» por algo propio y lo cambiado a mano entre medio entra); con `Err(e)` → la pestaña muestra «No se pudo guardar config.toml: {e}» y lo de memoria queda.
- Opciones (tabla del spec §A.3–A.6, controles y destinos al pie de la letra): Tema segmentado Claro/Oscuro/Sistema → `dark_mode` `Some(false)/Some(true)/None`; Color de acento: 12 `theme::PRESETS`, «Del sistema», «Otro…» (`host.choose_color`); Animaciones Sistema/Sí/No → `[opciones] animaciones` (Sistema = `Remove`) y `apply_animation_setting`; switches de Lista y vistas (`show_extensions`, `show_hidden` — mismo efecto que Ctrl+H —, `checkboxes`, `group_bar`, `file_colors` con las reglas activas debajo: propias primero, después las de fábrica, con su muestra y «Editar en config.toml» = `OpenConfig`); Vista por defecto: desplegable de `list::VIEWS` → `FolderViews::set_default_view(v: View)` (cambia la por defecto; las carpetas con vista propia la conservan), «Usar la vista de "<carpeta>" en todas» (la pestaña de carpeta que estaba activa en ese panel al abrir la configuración; apagado si ya no existe) y «Restablecer las vistas…» (`ask_yes_no` «¿Volver todas las carpetas a la vista de fábrica?» → `apply_to_all(ViewSettings::default())`); Árbol (`show_tree` solo con `arbol`, `tree_pinned`, `tree_follow` y al pie «De fábrica: lo que diga arbol_sigue_panel en config.toml» con «Volver a lo de config.toml» → `tree_follow = None`); Vista previa (`preview_on`, ancho 240–640 con el número). Los campos de sesión se guardan como hoy (la sesión se escribe al cerrar / con su mecanismo actual); la rueda dentada y la pestaña leen los mismos campos de `App`.

- [ ] **Step 1: Tests que fallan**: `theme_three_states` (clic en Sistema → `dark_mode == None`; Oscuro → `Some(true)`; el switch de la rueda sigue fijando), `accent_preset_and_system`, `animations_writes_and_removes_line` (Sí → `config.toml` con `animaciones = true` debajo del ejemplo comentado; Sistema → la línea ya no está y el ejemplo sí), `config_writes_are_serialized` (tres cambios seguidos sin vaciar el canal → una escritura en vuelo; al terminar, el archivo tiene el último valor), `edit_after_external_change_keeps_both` (escribir a mano `[colores] "*.log" = "#112233"` y después cambiar Animaciones → el archivo tiene las dos cosas y `config.colors` incluye la regla), `config_write_error_shown` (carpeta de configuración de solo lectura → texto «No se pudo guardar config.toml:» en la página), `own_write_not_reported_as_external` (después de `ConfigSaved`, `reload_config` no pone «Configuración aplicada»), `list_switches_mirror_picker` (prender casillas desde la pestaña → `PickerSwitches.checkboxes` true), `default_view_keeps_custom_folders` (una carpeta con vista propia Íconos, por defecto → Lista: esa sigue en Íconos, otra nueva abre en Lista), `tree_follow_back_to_config`, `preview_width_slider_clamps` (arrastre fuera de rango → 240 y 640), `color_rules_listed_own_first`.
- [ ] **Step 2:** `cargo test -p xdir-core settings config` → FAIL.
- [ ] **Step 3:** Implementar (pantallas 1 y 5).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac con un `config.toml` con comentarios propios: cambiar Animaciones y Tema, `diff` contra la copia de antes (solo la línea), la rueda dentada refleja los cambios.
- [ ] **Step 5: Commit** `feat: secciones de apariencia, lista, árbol y vista previa en la configuración`.

### Task S4: xtree y Avanzado

**Files:**
- Modify: `config.rs` (`[opciones] xtree_por_tipo`, `xtree_abrir`; plantilla con sus ejemplos comentados), `session.rs` (`xtree_ultimo: Option<PathBuf>`), `xtree/view.rs` (`Xtree::new` recibe `by_type`), `app/actions.rs` (`toggle_xtree`), `settings/{mod,draw}.rs`, `app/settings.rs`, `sys/mod.rs` (`SystemEnv::portable_supported(&self) -> bool { true }`, `SystemEnv::file_manager_name(&self) -> &'static str { "el Explorador" }`), `crates/xdir-mac/src/sys/macos.rs` (false, «el Finder»), `crates/xdir-linux/src/env.rs` («la carpeta»), `sys/testing.rs` (`FakeEnv::set_portable_supported`).

**Interfaces:**
- Produces: `Config::xtree_by_type: bool` (defecto false) y `Config::xtree_open: XtreeOpen` con `pub enum XtreeOpen { System, Pane, Last }` (`"sistema"`, `"panel"`, `"ultimo"`; valor desconocido → error de línea como las otras opciones). **Ruling**: sin la opción manda `Pane`, que es lo que hace hoy `toggle_xtree` (la carpeta del panel; si no es carpeta, `default_root`); el spec atribuyó «como hoy» a «El del sistema» por error. `Last` usa `xtree_ultimo` (si no hay o ya no existe, `Pane`); cada análisis nuevo anota su raíz en `xtree_ultimo`. La tecla T sigue cambiando solo la vista abierta (no escribe `config.toml`); el switch decide cómo arranca.
- xtree también muestra, solo para ver, sus teclas (T, F5, ⌫, A, Esc) con un enlace «Ver los atajos» que lleva a la página Atajos.
- Avanzado: «Abrir config.toml» (`OpenConfig`); carpeta de configuración (`session::config_dir`) con «Mostrar en {file_manager_name}» → `sys::shell().open_file(hwnd, &dir, None)`; la tabla de archivos del spec §A.8 (con `red.toml` y `rutas.txt` solo si `KeyStyle::Mac`); Modo portable: estado («Los archivos se guardan junto a xdir» / «…en la carpeta del usuario»); con `portable_supported()`: «Activar el modo portable» (`ask_yes_no`; en un hilo copia los archivos de la tabla que existan a la carpeta del ejecutable y crea `xdir.portable`; error → «No se pudo escribir en <carpeta>: …») y «Desactivar» (borra `xdir.portable`), los dos con «Se aplica al volver a abrir xdir»; sin soporte (macOS), solo el texto.

- [ ] **Step 1: Tests que fallan**: `xtree_options_parse` (`xtree_por_tipo = true`, `xtree_abrir = "ultimo"`; `"otro"` → error «línea N: …»), `template_has_xtree_examples` (`template_parses_clean_and_keeps_defaults` sigue pasando), `xtree_open_uses_option` (`Pane` con carpeta → esa; `System` → `FakeDiskScan::default_root`; `Last` con `xtree_ultimo` → esa), `xtree_starts_by_type_from_config`, `xtree_switch_writes_option`, `advanced_lists_files_and_dir`, `portable_toggle_creates_and_removes_marker` (carpeta temporal como «ejecutable»: activar copia `config.toml` y crea `xdir.portable`; desactivar lo borra y deja `config.toml`), `portable_info_only_when_unsupported` (`FakeEnv::set_portable_supported(false)` → sin botones).
- [ ] **Step 2:** `cargo test -p xdir-core xtree_open portable advanced` → FAIL.
- [ ] **Step 3:** Implementar; la ruta del ejecutable sale de `std::env::current_exe` como `config_dir` (en tests, un parámetro de la función pura `portable_files(from: &Path, to: &Path) -> Result<(), String>`).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: xtree con «por tipo» prendido abre coloreado; «Mostrar en el Finder»; Modo portable solo informa.
- [ ] **Step 5: Commit** `feat: opciones de xtree y sección avanzada en la configuración`.

### Task S5: Editor de atajos

**Files:**
- Create: `crates/xdir-core/src/settings/keys.rs` (clasificar una tecla grabada, estado de la fila).
- Modify: `commands.rs` (`Group`, `group`, `combo_text`, `Keymap::{combos, owner}`, `Config::key_errors`), `config.rs`, `settings/draw.rs`, `app/settings.rs`, `app/actions.rs` (captura al principio de `on_key` y `on_char`), `app/input.rs` (clic afuera cancela), `crates/xdir-mac/src/app/mac.rs` (`performKeyEquivalent:` del `XdirView`: si `app.capturing_keys()`, manda la tecla a `on_key` y devuelve true, así ⌘W/⌘, no los toma el menú).

**Interfaces:**
- Produces (`commands.rs`): `pub enum Group { Navigation, TabsPanes, Selection, Files, Views, Preview, Network, Other }` con `title()` («Navegación», «Pestañas y paneles», «Selección», «Archivos», «Vistas», «Vista previa y vista rápida», «Red», «Otros»); `pub fn group(c: Command) -> Group` (`match` exhaustivo; `Eject` en Archivos; `Settings`, `SettingsKeys`, `OpenConfig`, `About` en Otros); `pub fn combo_text(c: Combo) -> String` (nombre de `config.toml`, orden Meta, Ctrl, Shift, Alt; teclas «Coma», «Punto», «Izquierda», «RePag», «Más», «MásNum»…; letras en mayúscula; `parse_combo(&combo_text(c)) == Ok(c)`); `Keymap::combos(&self, c: Command) -> Vec<Combo>` (en orden); `Keymap::owner(&self, k: Combo) -> Option<Command>`; `pub fn markdown_table(env: &dyn SystemEnv) -> String` (tabla de la wiki por grupo, en orden de `ALL`, solo `available_in`). `Config::key_errors: Vec<String>` («config.toml, línea 12: tecla desconocida "Hiper"») además de `errors`.
- Produces (`settings/keys.rs`): `pub enum Capture { Wait, Cancel, Reject(&'static str), Accept(Combo) }`; `pub fn classify(vk: Key, ctrl: bool, shift: bool, alt: bool, meta: bool, style: KeyStyle) -> Capture`: Esc solo → `Cancel`; modificador solo (Shift, Ctrl, Alt, Win) → `Wait`; teclas de la lista sin Ctrl/Alt/Meta (flechas, Enter, Retroceso, Inicio, Fin, RePág, AvPág, letras, números y signos — con o sin Shift) → `Reject("Esa tecla ya la usa la lista")`; del sistema → `Reject("La usa el sistema")`: Mac ⌘Q, ⌘H, ⌘⌥H, ⌘Tab, ⌘Espacio, ⌃⌘Q (en `Combo`: Ctrl+Q, Ctrl+H, Ctrl+Alt+H, Ctrl+Tab, Ctrl+Espacio, Meta+Ctrl+Q); Windows y Linux (comparten `KeyStyle::Windows`; ruling) Alt+F4, Alt+Tab, Ctrl+Alt+Supr y todo lo que lleva Meta. Lo demás → `Accept`.
- Produces (`keys.rs`): `pub struct Recording { pub command: Command, pub replace: Option<usize> /* chip; None = agregar */, pub conflict: Option<(Combo, Command)>, pub message: Option<&'static str> }`.
- `App`: `pub fn capturing_keys(&self) -> bool`. Fila: ✎ graba reemplazando el chip elegido (sin chips, el primero), + agrega, ✕ en el chip (al pasar el mouse) lo quita, ↺ (solo si difiere de fábrica) quita la línea. Aceptar: si `owner(k)` es otro comando → `conflict` y la fila muestra «⌘K ya es de "Conectar a un servidor…"» con «Reasignar» / «Cancelar»; si es el mismo comando → nada (no-op). Aplicar = `config.keymap.set(cmd, &nuevos)` y, por cada desplazado que devuelve, su línea con lo que le quedó (`""` si nada): `edit_config` con `Set { "atajos", id, str_value(combo_text…) }`. «Restablecer todos» → `ask_yes_no("Atajos", "¿Volver todos los atajos a los de fábrica? Se borran N atajos propios.")` (N = líneas activas de `[atajos]`) → `RemoveAll { "atajos" }` y `Keymap::default()` + los de `config.toml` que no sean atajos. «(cambiado)» si `combos(c) != Keymap::default().combos(c)`. Arriba, `key_errors` con «Abrir config.toml». Solo comandos `available()`, por `group` y en el orden de `ALL`, con el nombre `title()`.

- [ ] **Step 1: Tests que fallan**: `combo_text_roundtrip` (todos los atajos de fábrica: `parse_combo(&combo_text(c)) == Ok(c)`; `Ctrl+Coma`, `Meta+Ctrl+N`), `every_command_has_group` (compila sin `_`; `group(Command::Duplicate) == Group::Files`), `markdown_table_order` (los títulos de grupo en el orden de `Group`), `classify_rules` (Esc → `Cancel`; Shift solo → `Wait`; «A» y Shift+A → lista; Ctrl+A → `Accept`; Mac Ctrl+Q → sistema; Windows Alt+F4 y Meta+E → sistema; F3 → `Accept`), `record_replaces_chip_and_writes_line` (✎ en `duplicar`, Ctrl+Shift+K → `config.toml` con `duplicar = "Ctrl+Shift+K"` y el keymap la usa), `record_conflict_offers_reassign` (Ctrl+T en `duplicar` → `conflict == Some((Ctrl+T, NewTab))`, nada escrito; «Reasignar» → dos líneas: `duplicar = "Ctrl+T"` y `pestana_nueva = ""`), `record_own_combo_is_noop` (grabar Ctrl+D en `duplicar` → sin conflicto, sin escritura), `add_second_shortcut` (+ → lista de dos), `remove_chip_leaves_empty_string`, `reset_one_removes_line`, `reset_all_asks_and_clears_active_lines` (texto exacto con N; comentarios intactos), `recording_swallows_keys` (grabando, Ctrl+W y Tab no cierran ni cambian de panel; clic afuera cancela), `escape_cancels_recording`, `key_errors_shown_on_top`, `keys_page_mac_labels` (`FakeEnv::set_key_style(Mac)` → chips «⌘P», `NewTab` «⌘T»).
- [ ] **Step 2:** `cargo test -p xdir-core combo_text classify record keys_page` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 3: normal, grabando, choque, rechazo).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: grabar ⇧⌘K en Duplicar, choque con ⌘T y Reasignar, ⌘W mientras graba no cierra, ⌘Q rechazado; `config.toml` con sus comentarios intactos (`diff`).
- [ ] **Step 5: Commit** `feat: editor de atajos de teclado en la configuración`.

### Task S6: Buscador

**Files:**
- Create: `crates/xdir-core/src/settings/options.rs` (tabla de opciones).
- Modify: `settings/{mod,draw}.rs`, `app/settings.rs`.

**Interfaces:**
- Produces: `pub struct OptionSpec { pub id: &'static str, pub section: Section, pub title: &'static str, pub desc: &'static str, pub keywords: &'static [&'static str] }`; `pub const OPTIONS: &[OptionSpec]` (una por fila de S3/S4; p. ej. Tema: «oscuro», «claro», «modo», «noche»); las secciones de S3/S4 se dibujan en el orden de esta tabla. `pub fn fold(s: &str) -> String` (minúsculas, sin tildes ni diéresis: á→a, é→e, í→i, ó→o, ú/ü→u, ñ queda). `pub fn matches(query: &str, item: &SearchItem) -> bool` (todas las palabras de la búsqueda en título, descripción o palabras clave, plegadas). Atajos: un `SearchItem` por comando con su título y, como palabras clave, `combo_label` y `combo_text` de sus atajos y su texto sin símbolos (`"⌘K"`, `"ctrl+k"`). Páginas extra: `ExtraPage::search`.
- Vista con texto: la derecha muestra todo lo que coincide de todas las secciones, agrupado bajo el nombre de la sección; la izquierda pone la cantidad al lado de cada sección y atenúa las que no tienen; sin texto vuelve a la sección elegida.

- [ ] **Step 1: Tests que fallan**: `fold_strips_accents` («Árbol» → «arbol»), `search_tema_by_keyword` («noche» → Tema), `search_across_sections_counts` («mostrar» → Lista y vistas y Árbol con sus cantidades; Avanzado atenuada), `search_keys_by_label` («⌘k» con estilo Mac y «ctrl+k» con Windows encuentran el comando que tenga ese atajo; «atajo» encuentra la sección Atajos), `search_empty_returns_to_page`, `search_draws_groups` (`RecordingPainter`: los nombres de sección arriba de cada grupo), `extra_page_items_searchable` (una `ExtraPage` de prueba con un `SearchItem` aparece).
- [ ] **Step 2:** `cargo test -p xdir-core search_` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 2).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: «oscuro», «atajo», «⌘k».
- [ ] **Step 5: Commit** `feat: buscador de la configuración`.

---

## Corriente N — Red

### Task N1: Direcciones, `red.toml` y tiempo límite

**Files:**
- Create: `crates/xdir-core/src/net/mod.rs` (`Address`, `NetError`, `NetConnection`, `ConnectOptions`, `Credentials`, `Secret`, `with_timeout`), `net/places.rs` (`red.toml`).
- Modify: `crates/xdir-core/src/lib.rs`.

**Interfaces:**
- Produces:
  - `pub struct Address { pub scheme: String, pub user: Option<String>, pub host: String, pub port: Option<u16>, pub share: Option<String>, pub rest: Vec<String> }`; `Address::parse(text: &str) -> Result<Address, String>` (acepta `smb://srv/rec/c`, `\\srv\rec\c`, `//srv/rec`, `afp://srv/vol`, `sftp://[u@]srv[:p]/ruta`, nombre suelto o IP → `smb://nombre`; esquema en minúsculas; sin barra final; `u:clave@` → `u@`; error: «Escribí una dirección como smb://servidor/recurso»); `to_url(&self) -> String`; `to_unc(&self) -> Option<String>` (solo `smb`: `\\srv\rec\c`); `display(&self, style: KeyStyle) -> String` (Windows UNC si se puede; si no URL); `same(&self, o: &Address) -> bool` (host y recurso sin mayúsculas).
  - `pub enum NetError { NotFound { host: String }, BadCredentials, NoShare { host: String, share: String }, Timeout { host: String }, CredentialConflict { host: String }, OpenFiles { by: Option<String> }, Unsupported { scheme: String }, NeedsCredentials { user: bool, domain: bool }, NoGvfs, Cancelled, Other(String) }` con `message(&self) -> String` (textos de Global Constraints; `OpenFiles` «lo está usando {by|otro programa}»).
  - `pub struct NetConnection { pub address: String, pub local: PathBuf, pub letter: Option<char>, pub reconnect: bool, pub connected: bool /* false: unidad recordada desconectada */ }`.
  - `pub struct ConnectOptions { pub drive_letter: Option<char>, pub reconnect: bool, pub other_credentials: bool, pub credentials: Option<Credentials> }` (`Default`); `pub struct Credentials { pub user: String, pub domain: Option<String>, pub password: Secret }`; `pub struct Secret(Vec<u8>)` sin `Debug`/`Clone`, con `Drop` que pisa los bytes con ceros y `expose(&self) -> &[u8]`.
  - `pub fn with_timeout<T: Send + 'static>(limit: Duration, host: &str, f: impl FnOnce() -> Result<T, NetError> + Send + 'static) -> Result<T, NetError>` (hilo + `recv_timeout`; al vencer, `Timeout { host }` y el hilo queda abandonado).
- Produces (`net/places.rs`): `pub struct NetPlaces { pub favorites: Vec<NetFavorite>, pub recents: Vec<NetRecent> }`, `NetFavorite { name: String, address: String }`, `NetRecent { address: String, date: i64 }`; `parse(text) -> NetPlaces` (`[[favorito]] nombre, direccion`; `[[reciente]] direccion, fecha`; cada dirección pasa por `Address::parse` y se guarda con `to_url`/la forma escrita sin clave; las inválidas se descartan), `write(&self) -> String`, `load() -> NetPlaces`, `save(&self) -> io::Result<()>` (atómico, en hilo), `add_recent(&mut self, addr: &Address, now: i64)` (al principio, sin repetir con `same`, máximo 10), `is_favorite(&Address) -> bool`, `toggle_favorite(&mut self, addr: &Address, name: Option<String>)`, `move_favorite(&mut self, i: usize, up: bool)`, `remove_favorite(i)`, `remove_recent(i)`, `clear_recents()`.

- [ ] **Step 1: Tests que fallan** (`net::tests`, `net::places::tests`): `parse_forms` (las 6 formas del spec dan host/share/rest; `nas` → `smb://nas`; `192.168.1.10` → `smb://192.168.1.10`), `normalize` (`SMB://Nas/Fotos/` → `smb://Nas/Fotos`), `unc_and_url` (`\\srv01\compartido` ↔ `smb://srv01/compartido`; `afp` sin UNC), `sftp_user_port` (`sftp://ana@vps:2222/var/www`), `password_never_persisted` (`smb://ana:secreto@nas/x` → `to_url` `smb://ana@nas/x`; `add_recent` + `write` no contienen «secreto»; un `red.toml` escrito a mano con la clave se lee sin ella), `error_messages` (cada texto exacto), `recents_front_dedupe_max_ten`, `favorites_order_and_toggle`, `places_roundtrip` (`parse(write(p)) == p`), `timeout_abandons_hung_call` (`with_timeout(50 ms, "nas", || { sleep(5 s); Ok(()) })` vuelve en < 1 s con `Timeout { host: "nas" }`), `secret_wipe_zeroes` (`Secret::wipe(&mut self)`, que llama `Drop`, deja todos los bytes en 0 y `Debug` no existe).
- [ ] **Step 2:** `cargo test -p xdir-core net` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(core): direcciones de red, red.toml y tiempo límite`.

### Task N2: Servicio `sys::Network`, falso y stubs

**Files:**
- Create: `crates/xdir-core/src/sys/testing/net_fake.rs`.
- Modify: `crates/xdir-core/src/sys/mod.rs` (trait, `Services::network`, `sys::network()`; quitar `FileSystem::connect_share`), `sys/testing.rs` (`install_fakes`; `FakeFileSystem` sin `connect_share`), `tab.rs` (`retry_with_credentials` devuelve la ruta en vez de conectar), `app/network.rs` (nuevo: `NetState`, conectar con credenciales en un hilo después de `retry_with_credentials`), `app/{mod,events,tasks}.rs`; los `install` de las 3 plataformas con un stub (`supports` false, `connect` → `Other("sin implementar")`, `is_remote` por prefijo UNC) hasta N3/N9/N11; quitar `connect_share` de `crates/xdir-windows/src/fs/win32/mod.rs` + `sys/win32.rs`, `crates/xdir-linux/src/fs.rs`, `crates/xdir-mac/src/fs`.

**Interfaces:**
- Produces:
  ```rust
  pub trait Network: Send + Sync {
      fn supports(&self, scheme: &str) -> bool;
      /// Bloqueantes: solo en hilos de trabajo.
      fn connect(&self, owner: Handle, addr: &Address, opts: ConnectOptions, cancel: &AtomicBool) -> Result<PathBuf, NetError>;
      fn connections(&self) -> Result<Vec<NetConnection>, NetError>;
      fn disconnect(&self, local: &Path, force: bool) -> Result<(), NetError>;
      fn shares(&self, host: &str) -> Result<Vec<String>, NetError>;
      /// Manda `HostEvent`s a medida que aparecen y desaparecen máquinas, avisando con `WM_APP_NET`; corre hasta `cancel`.
      fn browse(&self, hwnd: Handle, tx: Sender<HostEvent>, cancel: Arc<AtomicBool>);
      /// Sin E/S: mira la lista que mantiene el servicio (UNC y letras de red; puntos de montaje de red).
      fn is_remote(&self, path: &Path) -> bool;
      /// Vuelve a leer esa lista (en un hilo de trabajo, al cambiar los discos).
      fn refresh_remote(&self);
      fn free_drive_letters(&self) -> Vec<char> { Vec::new() }
      fn can_force(&self) -> bool { false }
      fn local_network_denied(&self) -> bool { false }
      fn open_credentials_place(&self, owner: Handle) -> bool { false }
      fn open_local_network_settings(&self, owner: Handle) {}
  }
  pub enum HostEvent { Up(String), Down(String) }
  ```
  `pub const WM_APP_NET: u32 = WM_APP + 33;` (eventos de `browse`) y `pub const WM_APP_NET_TICK: u32 = WM_APP + 34;` (redibujo de «Conectando…», N5), en `net/mod.rs`.
- `FakeNetwork` (estado de todo el proceso, como `FakeShell`, porque se llama desde hilos; cada test usa hosts y rutas propios): `set_supports(&[&str])`, `set_connect(host, Result<PathBuf, NetError>)`, `connects() -> Vec<(String /*url*/, ConnectOptions sin la clave)>`, `set_connections(Vec<NetConnection>)` (por host; `connections()` devuelve las de todos), `set_disconnect(local, Result)`, `disconnects() -> Vec<(PathBuf, bool)>`, `set_hosts(Vec<String>)` (lo que manda `browse`), `set_can_force(bool)`, `set_local_network_denied(bool)`, `set_free_letters(Vec<char>)`, `set_remote(prefix: &Path)`, `hang(host_or_prefix, Duration)` (las llamadas bloqueantes sobre eso duermen), `mark_ui_thread()` y `ui_io_calls() -> Vec<String>` (N5).
- `App`: `NetState { places: NetPlaces, connections: Vec<NetConnection>, hosts: Vec<String>, browse_cancel: Option<Arc<AtomicBool>>, … }` cargado al arrancar (`NetPlaces::load` en un hilo), `connections` pedidas con `spawn_task` + `with_timeout(10 s)` (lo mismo `shares`) al arrancar y al cambiar los discos (`TaskDone::NetConnections(Result<Vec<NetConnection>, NetError>)`, al final del `enum`), `refresh_remote` en el mismo hilo. `TaskDone::Connected { pane: usize, tab: Option<usize>, addr: Address, result: Result<PathBuf, NetError> }` (lo usan N4 y la reconexión de `retry_with_credentials`).

- [ ] **Step 1: Tests que fallan** (`app/network_tests.rs`): `unc_auth_failure_connects_in_worker` (la carga de `\\nas\fotos` falla con «os error 1326» → `FakeNetwork::connects()` tiene `smb://nas/fotos` y la llamada no ocurrió en el hilo del test; al volver `Connected Ok`, la pestaña recarga; una sola vez por navegación), `connections_requested_at_start`, `fake_network_hang_sleeps` (con `hang("lento", 200 ms)`, `connect` tarda ≥ 200 ms), `connect_share_removed` (compila sin `FileSystem::connect_share`: verificación con `cargo check` de las 3 plataformas).
- [ ] **Step 2:** `cargo test -p xdir-core network` → FAIL.
- [ ] **Step 3:** Implementar núcleo, falso y stubs.
- [ ] **Step 4:** PASS; Global Constraints (los 3 `cargo check`).
- [ ] **Step 5: Commit** `feat(core): servicio de red del sistema`.

### Task N3: Red en macOS

**Files:**
- Create: `crates/xdir-mac/src/net/mod.rs` (`MacNetwork`), `net/bonjour.rs` (`nw_browser`).
- Modify: `crates/xdir-mac/src/sys/macos.rs` (`install` con `MacNetwork`; `command_available`: false para `MapNetworkDrive`, `DisconnectNetworkDrive` — se agregan en N4), `crates/xdir-mac/src/shell/macos.rs` (`ejectable` deja fuera los volúmenes con `NSURLVolumeIsLocalKey = false`: su ⏏ sale de `Network::connections`; ruling, como Windows en la decisión 14), `scripts/package-mac.sh`.

**Interfaces:**
- Consumes: trait `Network` (N2), `Address`, `NetError` (N1).
- `supports`: `smb`, `afp`. `connect`: `NetFSMountURLSync(url, NULL, NULL, NULL, open_options{kNAUIOptionKey: kNAUIOptionAllowUI}, NULL, &mountpoints)` con `extern "C"` y `#[link(name = "NetFS", kind = "framework")]`; devuelve el primer punto de montaje (`/Volumes/<recurso>`); `cancel` no interrumpe (el hilo se abandona). Función pura `pub fn mac_error(code: i32, host: &str, share: &str) -> NetError` (`EAUTH` 80 → `BadCredentials`; `ENOENT` 2 → `NoShare`; `EHOSTUNREACH` 65, `ENETUNREACH` 51, `ECONNREFUSED` 61 → `NotFound`; `ETIMEDOUT` 60 → `Timeout`; `ECANCELED` 89 y `-128` → `Cancelled`; otro → `Other("error {code}")`).
- `connections`: `NSFileManager mountedVolumeURLsIncludingResourceValuesForKeys` con `NSURLVolumeIsLocalKey = false`, dirección de `NSURLVolumeURLForRemountingKey` (sin clave). `disconnect`: `unmountAndEjectDeviceAtURL` como `Shell::eject`; ocupado → `OpenFiles { by }` con `parse_lsof`; `force` → `unmount(path, MNT_FORCE)`; `can_force` true.
- `is_remote`: lista de puntos de montaje de red en un `RwLock`, cargada en `install` con `getfsstat(MNT_NOWAIT)` (local, sin red; tipos `smbfs`, `afpfs`, `nfs`, `webdav`) y en `refresh_remote`; función pura `pub fn remote_roots(mounts: &[(String /*tipo*/, PathBuf)]) -> Vec<PathBuf>`.
- `browse`: `nw_browse_descriptor_create_bonjour_service("_smb._tcp", NULL)` y `"_afpovertcp._tcp"`, `nw_browser_set_browse_results_changed_handler` (bloque `block2`) en una cola `dispatch2`; nombres de `nw_endpoint_get_bonjour_service_name` → `HostEvent::Up/Down` sin repetir entre los dos servicios; estado `waiting` con `kDNSServiceErr_PolicyDenied` (-65570) → `local_network_denied = true` y no manda nada; `cancel` → `nw_browser_cancel`. `shares`: `Ok(vec![])` (macOS elige el recurso en su ventana al conectar a `smb://máquina`).
- `open_credentials_place`: `open -b com.apple.keychainaccess`; `open_local_network_settings`: `x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork`.
- `package-mac.sh`: `NSLocalNetworkUsageDescription` «xdir muestra las computadoras de tu red local en "Red"» y `NSBonjourServices` (`_smb._tcp`, `_afpovertcp._tcp`).

- [ ] **Step 1: Tests que fallan** (`cargo test -p xdir-mac net`): `mac_error_codes` (cada código), `remote_roots_filters_types` (`apfs` no; `smbfs` sí), `bonjour_dedupes_services` (función pura que junta resultados de los dos servicios), `plist_has_local_network_keys` (lee `scripts/package-mac.sh` y busca las dos claves y los dos servicios), `local_volumes_not_network_ejectable` (`ejectable` sobre `/` y una imagen `hdiutil` del scratchpad no cambia; prueba con la función pura del filtro).
- [ ] **Step 2:** → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; prueba manual con el test `#[ignore]` `mac_connect_nas_manual` (conecta a la URL de `XDIR_NAS_URL`, lista `connections` y desconecta), con la contraseña en el Llavero.
- [ ] **Step 5: Commit** `feat(mac): conectar, listar, desconectar y descubrir servidores`.

### Task N4: Conectar a un servidor…

**Files:**
- Create: `crates/xdir-core/src/net/dialog.rs` (dibujo y hit del panel).
- Modify: `commands.rs` (`ConnectServer`, y `MapNetworkDrive`, `DisconnectNetworkDrive` ya declarados para N9/N10), `app/network.rs`, `app/{actions,input,paint,events,edit,fileops}.rs` (menú del fondo de «Este equipo»), `crates/xdir-mac/src/sys/macos.rs` y `crates/xdir-linux/src/env.rs` (`command_available` false para los dos de Windows).

**Interfaces:**
- Consumes: `Network::connect`, `NetPlaces`, `Address`, `with_timeout`, `TaskDone::Connected` (N1–N2).
- Produces: `spec(Command::ConnectServer, "conectar_servidor", "Conectar a un servidor…", &["Ctrl+K"])`, `spec(Command::MapNetworkDrive, "conectar_unidad_red", "Conectar unidad de red…", &[])`, `spec(Command::DisconnectNetworkDrive, "desconectar_unidad_red", "Desconectar unidad de red…", &[])`.
- Panel (`net/dialog.rs`): `pub struct ConnectDialog { pub field: Option<TextBox>, pub error: Option<String>, pub busy: Option<u64 /* generación */>, pub selected: Option<usize>, pub naming: Option<TextBox> }`; `pub fn draw(p, r, d: &ConnectDialog, places: &NetPlaces, style: KeyStyle, hover)`, `pub fn hit(r, d, places, x, y) -> Option<DialogHit>` con `DialogHit { Field, Star, Favorite(usize), Recent(usize), RemoveRecent(usize), Cancel, Connect, Close }`. Ejemplo bajo el campo: Windows «Por ejemplo: smb://servidor/recurso o \\servidor\recurso»; macOS suma «, afp://…»; Linux «, sftp://…». Centrado sobre los paneles; modal como «Acerca de».
- `App`: `open_connect_dialog()` (campo con la dirección de la carpeta del panel si `is_remote`, si no vacío; foco en el campo); Enter/Conectar → `Address::parse` (error en rojo, no cierra) → `supports(scheme)` (si no: «En este sistema no se puede conectar a direcciones sftp://») → `busy`, botón «Conectando…» + «Cancelar», `spawn_task(with_timeout(30 s, connect))`; al volver con la generación vigente: cierra, abre la ruta **en una pestaña nueva del panel activo**, `add_recent` + `save` en un hilo; error: queda en el panel. Cancelar con `busy` descarta el resultado (generación nueva). Clic en favorito/reciente pone la dirección; doble clic conecta; ✕ quita el reciente; ☆ agrega/quita favorito pidiendo el nombre en línea (Enter vacío = la dirección). Barra de dirección: `go_to_text` con `smb://`, `afp://`, `sftp://` → misma conexión sin panel, en la misma pestaña (ruling: la barra navega donde se escribe), con la lista en «Conectando con "nas"…» (N5) y el error en `tab.error`. Menú del fondo de «Este equipo»: «Conectar a un servidor…» (+ los dos de Windows si `available`).

- [ ] **Step 1: Tests que fallan**: `ctrl_k_opens_dialog_with_focus` (y `combo_label_in(Mac)` «⌘K»), `dialog_prefills_remote_folder`, `invalid_address_keeps_dialog` (texto exacto), `unsupported_scheme_message` (`FakeNetwork::set_supports(&["smb"])` + `sftp://x` → «En este sistema no se puede conectar a direcciones sftp://»), `connect_opens_new_tab_and_adds_recent` (`set_connect("nas", Ok("/Volumes/fotos"))` → pestaña nueva en el panel activo con esa ruta, la otra pestaña intacta, `places.recents[0]` `smb://nas/fotos`, `red.toml` escrito), `connect_error_stays_in_dialog` (`BadCredentials` → «Usuario o contraseña incorrectos» y el panel abierto), `cancel_discards_late_result`, `connect_dialog_strips_password` (`smb://ana:secreto@nas/x` → `connects()` lleva la URL completa al servicio solo en memoria del hilo; recientes, `red.toml`, avisos y `tab.error` sin «secreto»), `star_toggles_favorite_with_name`, `double_click_recent_connects`, `address_bar_url_connects_in_place`, `computer_background_menu_has_connect`, `windows_only_commands_hidden_on_mac` (`available_in` con el entorno de la Mac → false; `FakeEnv` → true).
- [ ] **Step 2:** `cargo test -p xdir-core connect dialog` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 6: vacío, con favoritos y recientes, conectando, con error).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: ⌘K al NAS con la contraseña del Llavero (la ventana del sistema queda encima), reciente y favorito, error con un host inexistente.
- [ ] **Step 5: Commit** `feat: conectar a un servidor`.

### Task N5: Que la red nunca trabe la ventana

**Files:**
- Modify: `tab.rs` (`new`/`navigate`/`open` sin `long_path`, `is_dir`, `exists`, `is_file` para rutas remotas: usan `Entry::is_dir` y navegan; guardián remoto en un hilo: `Tab::set_watcher(nav_id, Option<Box<dyn Watch>>)`; `load_started: Option<Instant>`, `first_batch: bool`, `pub fn slow_state(&self, now: Instant) -> Slow`), `list/mod.rs` (`FileList::placeholder: Option<Placeholder>` dibujado en lugar de la lista vacía, con sus botones en `hit`), `app/{edit,saved_groups,quick_jump,favs,fileops,tasks,mod,paint,events}.rs`, `palette.rs`, `favorites.rs`, `tree.rs` (`refresh_drives`: nombre de una unidad remota sin `drive_label` en el hilo — se pide en un hilo y se completa al volver), `sys/testing.rs` (`FakeFileSystem` y `FakeDiskScan` anotan en `FakeNetwork::ui_io_calls` las llamadas sobre rutas remotas hechas en el hilo marcado), las cargas de miniaturas y metadatos de las 3 plataformas (tiempo límite por elemento remoto: `with_timeout(5 s)`; sin respuesta, el elemento queda sin miniatura/metadato y la cola sigue).

**Interfaces:**
- Consumes: `Network::is_remote`, `FakeNetwork::{set_remote, mark_ui_thread, ui_io_calls}` (N2).
- Produces: `pub fn fs::probe_dir(p: &Path) -> Option<bool>` (`None` si `sys::network().is_remote(p)` sin tocar el disco; si no `Some(p.is_dir())`) y lo mismo `probe_file`, `probe_exists`; con `None` cada llamador navega o deja decidir a la carga en su hilo. `pub enum Slow { No, Connecting { host: String }, NotResponding { host: String } }` (remota, cargando y sin ningún lote: ≥ 0,5 s → `Connecting`, ≥ 20 s → `NotResponding`); `pub enum Placeholder { Connecting(String), NotResponding(String) }` con los textos «Conectando con "nas"…» y «"nas" no responde» y los botones «Reintentar» (recarga) y «Cerrar la pestaña»; `App` pone el `placeholder` de cada lista visible antes de dibujar y, al empezar una carga remota, `post_after` de `WM_APP_NET_TICK` a 0,5 s y 20 s. Lista auditada (cada una se saltea con `is_remote` o pasa a un hilo): `Tab::new`/`navigate` (`long_path`), `Tab::open` (`is_dir`, `is_file`, `exists`), `Tab::start` (`watch`), `edit.rs` `go_to_text` (`exists`, `is_dir`, `is_file`, `true_case`) y la sugerencia (`exists`, línea 148), `palette.rs:112` (`is_dir` + `true_case`), `tab.rs:763` y `tree.rs:1095` (`true_case`), `app/quick_jump.rs:97`, `app/favs.rs:294`, `app/fileops.rs:426`, `app/mod.rs:441` (ruta de arranque), `favorites.rs:64` (`long_path`), `saved_groups.rs:115` (`drive_missing`), `tasks.rs` `eject`/`eject_now` (`drive_label`), `tree.rs:409` (`drive_label`). Al terminar, `grep` de nuevo con la misma lista de llamadas (`long_path(`, `true_case(`, `drive_missing(`, `drive_label(`, `volume_space(`, `cloud_only(`, `case_insensitive(`, `.is_dir()`, `.exists()`, `.is_file()`, `metadata(`) sobre `crates/xdir-core/src/{app,tab.rs,tree.rs,palette.rs,favorites.rs}` y anotar en el commit las que quedan y por qué no tocan la red.

- [ ] **Step 1: Tests que fallan** (todos con `FakeNetwork::set_remote("/net/<test>")` y `mark_ui_thread()`): `remote_navigation_no_ui_io` (abrir la pestaña, navegar, Atrás, abrir una subcarpeta de la lista, escribir la ruta en la barra, saltar con la paleta, arrastrar a favoritos, restaurar la sesión y un grupo con esa ruta → `ui_io_calls()` vacío), `remote_watcher_created_off_thread`, `slow_state_thresholds` (`slow_state(t0 + 0,4 s) == No`; 0,6 s → `Connecting`; 21 s → `NotResponding`; con un lote → `No`), `hung_load_shows_not_responding_and_close_works` (carga que nunca responde: `RecordingPainter` dibuja «Conectando con "nas"…» y luego «"nas" no responde»; clic en «Cerrar la pestaña» la cierra; cambiar de panel funciona en el medio), `retry_reloads`, `late_result_discarded_after_close`, `local_paths_still_probe` (una carpeta temporal local sigue usando `is_dir`: escribir un archivo en la barra lo enfoca), `drive_label_of_remote_off_thread`.
- [ ] **Step 2:** `cargo test -p xdir-core remote slow_state hung` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 9).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: abrir una carpeta del NAS, apagar el Wi-Fi, recargar: la ventana responde, se ve «Conectando…» y después «no responde»; cerrar la pestaña; volver a prender.
- [ ] **Step 5: Commit** `fix: ninguna carpeta de red traba la ventana`.

### Task N6: Desconectar (⏏)

**Files:**
- Modify: `app/tasks.rs` (`eject`, `TaskDone::Disconnected` al final), `app/transfers.rs` (`eject_after_jobs` con el verbo), `app/{paint,input,fileops,network}.rs` (⏏ en «Este equipo» para conexiones), `list/mod.rs` (si `eject_rows` necesita la unión).

**Interfaces:**
- Consumes: `Network::{disconnect, can_force, connections}`, `NetState::connections`.
- Produces: `App::network_root(&self, p: &Path) -> Option<&NetConnection>` (la conexión cuya `local` contiene `p`, la más larga); `App::eject(root)`: si `network_root(&root)` es `Some` → `disconnect(root)`, si no el de la tanda D. `eject_after_jobs(&mut self, root, label, verb: Verb, then)` con `pub enum Verb { Eject, Disconnect }`; `pub fn disconnect_question(jobs, label) -> String` («Hay 1 operación sobre "fotos". ¿Cancelarla y desconectar?» / «Hay N operaciones … ¿Cancelarlas y desconectar?»). `disconnect`: `release_volume(root)` → `spawn_task(with_timeout(15 s, disconnect(root, false)))` → `TaskDone::Disconnected { root, label, force: bool, result: Result<(), NetError> }`: Ok → barra «Se desconectó "fotos"», `connections` se vuelve a pedir; `OpenFiles { by }` → `ask_yes_no_cancel("Desconectar", "No se pudo desconectar \"fotos\": lo está usando Word. ¿Reintentar?")`, y si `can_force()` y no era forzado, una segunda pregunta «¿Desconectar igual? Lo que esté abierto en "fotos" puede perder cambios.» → `disconnect(root, true)`. La etiqueta es el último tramo de la dirección (no `drive_label`). ⏏ en «Este equipo» y `Command::Eject` cubren conexiones: el conjunto de ⏏ de la lista es `ejectable ∪ {c.local}`.

- [ ] **Step 1: Tests que fallan**: `disconnect_releases_tabs_xtree_preview` (pestañas en `/net/x/fotos` de dos paneles pasan a «Este equipo», xtree con raíz ahí se cierra, la vista previa se vacía, **después** `FakeNetwork::disconnects()` tiene `(/net/x/fotos, false)`), `disconnect_with_copy_asks` (un trabajo de copia sobre esa ruta → `FakeHost` registró «Hay 1 operación sobre "fotos". ¿Cancelarla y desconectar?»; No → nada), `disconnect_success_notice`, `disconnect_busy_retry_then_force` (`OpenFiles { by: Some("Word") }` → pregunta exacta; Sí → segundo intento; con `can_force` y otra vez ocupado → «¿Desconectar igual?…» → `(root, true)`), `eject_command_on_remote_folder_disconnects`, `computer_list_shows_eject_for_connections` (`RecordingPainter`), `eject_disk_flow_unchanged` (los tests de la tanda D siguen pasando).
- [ ] **Step 2:** `cargo test -p xdir-core disconnect` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: ⏏ del NAS en «Este equipo» con un PDF abierto en Vista Previa (aviso con el nombre, «Desconectar igual»).
- [ ] **Step 5: Commit** `feat: desconectar carpetas de red con ⏏`.

### Task N7: «Red» en el árbol

**Files:**
- Modify: `tree.rs` (`Kind::NetFavorite`, `NetConnected`, `NetHost`; `Kind::Network` en los 3 sistemas; ⏏ de conexiones), `app/{panes,network,input}.rs` (clic, menú del clic derecho), `app/events.rs` (`WM_APP_NET`).

**Interfaces:**
- Consumes: `NetState::{places, connections, hosts}`, `Network::browse`, `HostEvent`, flujo de N4 y N6.
- Produces: `Tree::set_network(&mut self, favorites: Vec<(String /*nombre*/, String /*dirección*/)>, connected: Vec<(String /*título*/, PathBuf)>, hosts: Vec<String>)` (reconstruye los hijos de «Red» conservando lo expandido de cada conectado); «Red» siempre al final, con `path` = `network_place()` (vacío en macOS y Linux); su `location()` es la Red del Shell solo si `network_place()` no está vacío (macOS y Linux solo despliegan). Grupos: favoritos (★), conectados (título «nas.local / fotos» o «Z: compartido (\\srv01)», ⏏ al pasar el mouse y en el seleccionado; se expanden como carpetas), separador y «en la red local»: Windows = hijos de `shell_subfolders(NETWORK_PLACE)` leídos al expandir (como hoy); macOS y Linux = `hosts` de `browse` (se agregan y quitan a medida que llegan; sin nada, el grupo no aparece). `TreeHit::ConnectFavorite(String)`, `TreeHit::ConnectHost(String)` (conecta `smb://máquina`), y ⏏ de un `NetConnected` → `App::eject(local)`. Clic en favorito: si ya hay una conexión con ese host/recurso abre su ruta; si no, conecta con el flujo de N4 sin panel y abre en el panel activo; si falla, abre el panel con el error. Menú: «Conectar a un servidor…», «Desconectar», «Agregar a favoritos» / «Quitar de favoritos», «Copiar la dirección» (`shell().copy_text`). `browse` arranca con el árbol visible y se cancela al ocultarlo.

- [ ] **Step 1: Tests que fallan**: `tree_network_root_on_all_systems` (con `network_place` vacío igual aparece «Red» y no abre nada en el panel), `tree_network_groups` (2 favoritos, 1 conectado, 2 hosts → filas en ese orden con el separador; `RecordingPainter` dibuja ★ y ⏏ solo al pasar el mouse o seleccionado), `host_up_down_updates_tree`, `favorite_click_connects_or_opens` (sin conexión → `connects()`; con conexión → navega sin conectar), `host_click_connects_smb`, `connected_eject_runs_disconnect_flow`, `tree_network_menu_entries` (los 4 textos), `windows_shell_children_kept` (con `network_place` no vacío, expandir pide `shell_subfolders` como hoy), `browse_cancelled_when_tree_hidden`.
- [ ] **Step 2:** `cargo test -p xdir-core tree_network host_ favorite_click` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 8).
- [ ] **Step 4:** PASS; Global Constraints; smoke en la Mac: «Red» con el NAS conectado (⏏), un favorito y las máquinas de Bonjour (aceptar el permiso de red local; negarlo y ver que el grupo no aparece).
- [ ] **Step 5: Commit** `feat: Red en el árbol con favoritos, conexiones y la red local`.

### Task N8: Sección «Red» de la configuración

**Files:**
- Create: `crates/xdir-core/src/app/network_page.rs`.
- Modify: `app/network.rs`, `app/settings_pages.rs` (solo si S ya está en `develop`: agrega la `ExtraPage`), `app/saved_groups.rs` (`reload_config` relee `red.toml` si cambió su fecha, como `grupos.toml`; la fecha se mira en un hilo).

**Interfaces:**
- Produces (firmas de «Interfaces S↔N»): `search` (Favoritos, Recientes, Conectados ahora, Contraseñas; palabras clave «servidor», «smb», «nas», «contraseña», «llavero»), `draw`, `click`, `field_done`. Contenido: Favoritos con nombre y dirección editables en línea (`TextKind::Field`; la dirección se valida con `Address::parse` y el error queda en la fila), ↑/↓, ✕, «Agregar…» (fila vacía); Recientes con la fecha («hace 2 días», función pura `ago(now, date) -> String`), ★ (pasa a favoritos), ✕, «Borrar los recientes»; Conectados ahora (`connections` pedidas en un hilo al abrir la página) con ⏏ (`App::eject`); el texto «xdir no guarda contraseñas: las guarda el sistema (Llavero / Administrador de credenciales / llavero del escritorio)» — la parte entre paréntesis según el sistema por `KeyStyle` y `open_credentials_place` — con el botón «Abrir» si `open_credentials_place` existe; con `local_network_denied()`, «Para ver las máquinas de la red local, permití "Red local" en Ajustes del Sistema → Privacidad» y el botón que llama `open_local_network_settings`. Cada cambio guarda `red.toml` en un hilo.
- Sin S en `develop`: los tests llaman a `draw`/`click` directamente sobre un `Rect` (la página no tiene dónde verse todavía; se ve en F1).

- [ ] **Step 1: Tests que fallan**: `red_page_lists_and_edits_favorites` (renombrar en línea, dirección inválida con error, ↑ cambia el orden de `places` y del árbol), `red_page_recents_star_and_clear`, `red_page_connected_eject`, `red_page_password_text` (texto exacto con «Llavero» en Mac), `red_page_local_network_hint` (`FakeNetwork` con `local_network_denied` → aviso y botón), `ago_texts` («hace 2 días», «hoy», «ayer»), `red_toml_reloaded_when_changed`, `red_page_search_items`.
- [ ] **Step 2:** `cargo test -p xdir-core red_page` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 4).
- [ ] **Step 4:** PASS; Global Constraints (smoke de la página en F1 si S no estaba).
- [ ] **Step 5: Commit** `feat: favoritos y recientes de red en la configuración`.

### Task N9: Red en Windows

**Files:**
- Create: `crates/xdir-windows/src/net.rs` (`WinNetwork`).
- Modify: `crates/xdir-windows/src/{sys/win32.rs,main.rs}`.

**Interfaces:**
- Consumes: trait `Network` (N2).
- `supports`: `smb`. `connect`: `WNetAddConnection3W(owner, &NETRESOURCEW{ RESOURCETYPE_DISK, lpRemoteName: \\srv\rec, lpLocalName: "Z:" o null }, null, null, CONNECT_INTERACTIVE | (CONNECT_PROMPT si other_credentials) | (CONNECT_UPDATE_PROFILE si reconnect))`; si falla por acceso (`5`, `1326`) sin `CONNECT_PROMPT`, se repite una vez con `CONNECT_PROMPT`; devuelve `Z:\` o `\\srv\rec\resto`. Pura: `pub fn win_error(code: u32, host: &str, share: &str) -> NetError` (53 → `NotFound`; 67 → `NoShare`; 5, 1326 → `BadCredentials`; 1219 → `CredentialConflict`; 1223 → `Cancelled`; 2401, 2404 → `OpenFiles { by: None }`; 85 → `Other("La letra ya está en uso")`; otro → `Other` con el mensaje del sistema).
- `connections`: `WNetOpenEnumW(RESOURCE_CONNECTED)` + `RESOURCE_REMEMBERED` (las recordadas que no están conectadas → `connected: false`); `reconnect` = está en las recordadas. `disconnect`: `WNetCancelConnection2W(nombre, CONNECT_UPDATE_PROFILE, force)`; `can_force` true. `free_drive_letters`: `GetLogicalDrives` + las recordadas, de Z a D. `is_remote`: UNC por forma; letras de red en un `RwLock` cargado en `install` con `GetDriveTypeW == DRIVE_REMOTE` (local, sin red) y en `refresh_remote`. `browse`: no manda nada (Windows usa la Red del Shell en el árbol). `shares`: `NetShareEnum` nivel 1 (solo `STYPE_DISKTREE`; `Win32_NetworkManagement_NetManagement` si falta la feature). `open_credentials_place`: `control /name Microsoft.CredentialManager`.

- [ ] **Step 1: Tests que fallan** (CI de Windows): `win_error_codes`, `free_letters_from_mask` (función pura `free_letters(mask: u32, remembered: &[char]) -> Vec<char>`: Z primero), `win_is_remote_unc`, `win_connections_enum_runs` (no falla en una máquina sin conexiones).
- [ ] **Step 2:** `cargo check --target x86_64-pc-windows-msvc --all-targets` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en CI; check también `--no-default-features`; lista de prueba manual para el PR (conectar `\\srv\rec` con credenciales guardadas y con «otras credenciales», `ERROR_SESSION_CREDENTIAL_CONFLICT`, desconectar con un archivo abierto y «Desconectar igual»).
- [ ] **Step 5: Commit** `feat(windows): conectar, listar y desconectar recursos de red`.

### Task N10: Conectar y desconectar unidad de red (Windows)

**Files:**
- Create: `crates/xdir-core/src/net/drives_dialog.rs`.
- Modify: `app/network.rs`, `app/{actions,input,paint,fileops}.rs` (menú del clic derecho de «Este equipo» y de «Red»).

**Interfaces:**
- Consumes: `free_drive_letters`, `connect` con `ConnectOptions { drive_letter, reconnect, other_credentials }`, `connections`, flujo de ⏏ (N6).
- Produces: `MapDialog { letter: char, folder: Option<TextBox>, reconnect: bool /* prendida */, other_credentials: bool, error: Option<String>, busy: Option<u64> }` y `DisconnectDialog { rows: Vec<(NetConnection, bool /* marcada */)> }` con `draw`/`hit` en `drives_dialog.rs`. Unidad: letras libres de Z hacia abajo, Z (o la primera libre) elegida, con `host.popup_menu`; Carpeta con autocompletar de favoritos y recientes (sugerencia por prefijo); «Examinar…» = `shell().pick_folder`; «Finalizar» → conectar en un hilo, abre la unidad en una pestaña nueva y la suma a recientes. «Desconectar unidad de red…»: letra, carpeta, «desconectada» para las recordadas; «Desconectar» corre el flujo de N6 para cada marcada; sin unidades: «No hay unidades de red».

- [ ] **Step 1: Tests que fallan** (con `FakeNetwork`; `FakeEnv` deja disponibles los comandos): `map_dialog_defaults` (letras libres `['Z','Y']` → Z elegida; «Conectar de nuevo al iniciar sesión» prendida), `map_finish_connects_with_letter` (`connects()` con `drive_letter: Some('Z')`, `reconnect: true`; pestaña nueva en `Z:\`; reciente agregado), `map_error_stays`, `map_autocomplete_from_places`, `disconnect_dialog_lists_and_runs_flow` (dos unidades, una «desconectada»; marcar una → `disconnects()` con esa), `disconnect_dialog_empty_text`, `drive_commands_in_computer_menu`.
- [ ] **Step 2:** `cargo test -p xdir-core map_ disconnect_dialog` → FAIL.
- [ ] **Step 3:** Implementar (pantalla 7).
- [ ] **Step 4:** PASS; Global Constraints (sin smoke en la Mac: los comandos no existen ahí; capturas con `FakeHost` + `RecordingPainter` en el PR).
- [ ] **Step 5: Commit** `feat(windows): conectar y desconectar unidades de red`.

### Task N11: Red en Linux

**Files:**
- Create: `crates/xdir-linux/src/net.rs` (`LinuxNetwork`).
- Modify: `crates/xdir-linux/src/{sys.rs,main.rs}`.

**Interfaces:**
- Consumes: trait `Network` (N2), `Credentials`, `Secret`.
- Sin `gio` en el `PATH` → todo devuelve `NetError::NoGvfs`. `supports`: `smb`, `sftp`, y `afp` si `gio mount --list-types` lo trae (leído una vez, en el primer `supports` desde un hilo; ruling: hasta entonces `afp` no). `connect`: `gio mount <url>` con `stdin`/`stdout` por tubos; si `gio` pregunta («User», «Domain», «Password») y no hay `credentials` → mata el hijo y `NeedsCredentials { user, domain }`; con `credentials` las escribe en el `stdin` y suelta el `Secret`. Ruta local: `gio info -a standard::target-uri,standard::local-path <url>` (o `$XDG_RUNTIME_DIR/gvfs/…`). `connections`: `gio mount -l` (solo `smb`, `sftp`, `afp`, `dav`). `disconnect`: `gio mount -u <url>`; ocupado → `holders` de la tanda D (`/proc/*/fd`) → `OpenFiles { by }`; `can_force` false. `is_remote`: `/proc/self/mounts` (`cifs`, `smb3`, `nfs`, `nfs4`, `fuse.sshfs`) + todo bajo `$XDG_RUNTIME_DIR/gvfs`, cargado en `install` y en `refresh_remote`. `browse`: `gio list network:///` cada 30 s mientras no se cancele, mandando `Up`/`Down` por diferencia. Errores de `gio` por texto: pura `pub fn gio_error(stderr: &str, host: &str, share: &str) -> NetError`.
- Puras: `pub fn gio_prompt(line: &str) -> Option<Prompt>` (`Prompt::{User, Domain, Password}`), `pub fn parse_mount_list(out: &str) -> Vec<NetConnection>`, `pub fn parse_network_list(out: &str) -> Vec<String>`, `pub fn remote_mounts(mounts: &str, gvfs: &Path) -> Vec<PathBuf>`.

- [ ] **Step 1: Tests que fallan** (Docker): `gio_prompts`, `gio_errors` (salidas reales de `gio` con «Connection refused», «No such file or directory», «Permission denied»), `parse_mount_list_network_only`, `parse_network_list_hosts`, `remote_mounts_types`, `no_gio_reports_needs_gvfs` (`PATH` vacío → «Hace falta gvfs para conectarse a servidores»), `credentials_written_once` (un `gio` falso — script en una carpeta temporal del test puesto primero en el `PATH` — que pide clave y escribe lo recibido a un archivo; el servicio responde `NeedsCredentials` sin credenciales y conecta con ellas).
- [ ] **Step 2:** `sh …/dk.sh cargo test -p xdir-linux net` → FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS; Global Constraints.
- [ ] **Step 5: Commit** `feat(linux): servicio de red con gio`.

---

## Cierre

### Task F1: Enchufar Red en la configuración, wiki, diseño, verificación y versión de Mac

**Files:**
- Modify: `crates/xdir-core/src/app/settings_pages.rs` (si N no la agregó: `ExtraPage { id: "red", title: "Red", after: Section::Keys, search: network_page::search, draw: network_page::draw, click: network_page::click, field_done: network_page::field_done }`; reemplazar el `SearchItem` local de N por el de S), `commands.rs` (brazos de `group` si faltan), `app/settings_tests.rs`; wiki (clonar `Runick93/xdir.wiki` en el scratchpad); `docs/DISENO.md` (solo cuando el agente de diseño ya no la esté editando).

- [ ] **Step 1: Tests que fallan**: `settings_red_page_in_list` (la lista dice «… Atajos de teclado, Red, xtree, Avanzado»; `xdir:configuracion#red` se restaura), `search_finds_red_items` («contraseña» → Red), `network_commands_grouped` (`group(Command::ConnectServer) == Group::Network`; la página de Atajos muestra el grupo «Red» con «Conectar a un servidor…» ⌘K).
- [ ] **Step 2:** `cargo test -p xdir-core settings_red network_commands` → FAIL (si F1 tiene algo que enchufar); implementar; PASS.
- [ ] **Step 3: Commit** `feat: sección Red en la configuración`.
- [ ] **Step 4:** Wiki (spec §D): **Configuracion.md** (la pestaña: cómo se abre, secciones, buscador, qué se guarda dónde; «xdir solo cambia las líneas de lo que tocás en la pestaña; comentarios y lo demás quedan»; `xtree_por_tipo` y `xtree_abrir`; `red.toml`; modo portable desde Avanzado; Linux en «Dónde se guarda»), **Atajos-de-teclado.md** (la tabla generada con `commands::markdown_table` en estilo Windows y Mac, Ctrl+, / ⌘, y Ctrl+K / ⌘K, el grupo Red, «Cambiar un atajo» desde la pestaña con `config.toml` como alternativa), **Red.md** nueva en `_Sidebar.md` (conectar, direcciones por sistema, favoritos y recientes, unidades de Windows, «Red» en el árbol y el permiso de red local de macOS, desconectar, «xdir no guarda contraseñas», carpetas que no responden), **Funciones.md** (una línea por función con enlace). Commit en la wiki `docs: configuración y red (tanda D+)`.
- [ ] **Step 5:** `docs/DISENO.md`: pantallas de la tanda D+ pasan a programadas; commit `docs: pantallas de la tanda D+ programadas`.
- [ ] **Step 6:** Verificación final: Mac, Docker, checks de los 3 sistemas, capturas de las 10 pantallas contra el diseño (oscuro y claro), smoke del spec (⌘,; cambiar un atajo y `diff` de `config.toml` con sus comentarios intactos; ⌘K al NAS con la contraseña del Llavero; ⏏; tirar la red con una carpeta remota abierta y que la ventana siga respondiendo).
- [ ] **Step 7:** Con todo mergeado: `develop` actualizado, `sh scripts/package-mac.sh` y reemplazar `/Applications/xdir.app` (avisar si xdir está abierto); comprobar que el `Info.plist` instalado trae `NSLocalNetworkUsageDescription`.

## PRs

1. **S** (`feature/dmas-config`): S1–S6 (`feat: pestaña de configuración y editor de atajos`); se puede cortar en S1–S4 y S5–S6.
2. **N** (`feature/dmas-red`): N1–N11 (`feat: conectar a servidores, Red en el árbol y desconectar`); se puede cortar en N1–N6 (macOS + núcleo) y N7–N11.
3. **Cierre** (`feature/dmas-cierre`): F1, con S y N en `develop`.

Cada PR lleva la lista de pruebas manuales de Windows para Runick93.

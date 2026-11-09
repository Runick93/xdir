# Workspace, paso 6: `xdir-linux` — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Una crate `crates/xdir-linux` (binario `xdir-linux`) que en Linux implementa los servicios del sistema del núcleo con lo mínimo (biblioteca estándar, `std::os::unix`, `libc` y `notify`), arma el núcleo con ellos y sale con un aviso, porque la interfaz gráfica de Linux es otro proyecto.

**Architecture:** Igual que `xdir-mac` y `xdir-windows`: en la raíz de la crate, todo el código real bajo `#[cfg(target_os = "linux")]` y, fuera de Linux, un `main` que avisa y sale. En Linux `main` instala `sys::Services` con las implementaciones de Linux, crea la `App` (`App::new(Init::load(…), host)`) con un `HeadlessHost` propio (implementa `WindowHost` sin ventana: no dibuja, tamaño fijo, sin cajas ni menús) y termina imprimiendo que la interfaz de Linux todavía no existe. Los servicios se prueban con tests que corren en la CI de Linux (`cargo test` en la raíz) y, desde la Mac, en Docker.

**Tech Stack:** Rust 2024, `libc` 0.2, `notify` 8 (las mismas versiones que `xdir-mac`), Docker `rust:1-slim` para probar desde la Mac.

**Spec:** `docs/superpowers/specs/2026-11-08-workspace-core-design.md` (Arquitectura → Workspace: «`crates/xdir-linux/` bin: por ahora solo arma el núcleo con implementaciones mínimas y sale con un aviso; la UI de Linux es otro proyecto»; paso 6 del orden de trabajo)

## Global Constraints

- `cargo build` y `cargo test` en la raíz funcionan en Windows, macOS y Linux: fuera de Linux `xdir-linux` compila a un `main` que avisa y sale. `cargo check --all-targets` sin errores ni warnings nuevos para `x86_64-pc-windows-msvc`, `aarch64-apple-darwin` (o el target de la Mac) y `x86_64-unknown-linux-gnu`.
- `xdir-core` no cambia, salvo que haga falta exponer algo ya neutral (anotarlo). Sin `#[cfg]` de sistema en el núcleo.
- Features: `default = ["arbol"]`, `arbol = ["xdir-core/arbol"]`, `pruebas = ["xdir-core/pruebas"]`, como las otras dos crates.
- `release.yml`: el `sed` de la versión también sube `xdir-linux` en `Cargo.lock`, o el `--locked` del release falla. No se empaqueta nada de Linux.
- Las implementaciones son mínimas pero correctas: lo que Linux no puede hacer sin interfaz (portapapeles, menú contextual del sistema, elegir carpeta, propiedades, pegar virtual, fijar en acceso rápido, ayudante elevado) devuelve `None` / `false` / `Err("No disponible en Linux")` sin entrar en pánico.
- Tests: los de la crate corren solo en Linux (`#[cfg(test)]` dentro del código `cfg(target_os = "linux")`); nunca escriben fuera de la carpeta temporal; no cambian variables de entorno del proceso (en Rust 2024 es `unsafe` y compite con otros tests): las funciones que leen `HOME` / `XDG_*` se separan en una versión pura que recibe los valores, y esa es la que se prueba.
- Verificación desde la Mac: `docker run --rm -v "$PWD":/w -w /w rust:1-slim cargo test` (con `CARGO_TARGET_DIR=/w/target/linux` para no mezclar con el `target` de la Mac). Si Docker no está, solo `cargo check` para Linux y lo confirma la CI.
- No correr `cargo fmt` sobre todo el repo. Castellano; Conventional Commits sin atribución a Claude.
- Un solo PR: rama `feature/xdir-linux`, base `develop`. Después del merge, borrar la rama.

## Review Focus

- **Carpetas con nombres no UTF-8 o sin permiso de lectura**: Linux permite nombres con bytes inválidos y carpetas sin permiso; la carga y el escaneo no pueden entrar en pánico ni cortar todo el listado → test en Task 2 con un archivo de nombre no UTF-8 y una subcarpeta con permisos `000` (el escaneo sigue y cuenta la denegada).
- **Enlaces simbólicos**: un enlace a una carpeta no se recorre dos veces en el escaneo de xtree ni hace un ciclo → test en Task 3 con un enlace que apunta a su carpeta padre.
- **Mover entre discos**: `rename` falla con `EXDEV` entre sistemas de archivos; mover tiene que caer a copiar y borrar → la Task 3 lo implementa y lo prueba forzando el camino de copia (función separada).
- **Papelera**: borrar con `recycle = true` sigue la especificación de freedesktop (`$XDG_DATA_HOME/Trash/files` + `.trashinfo`) y nunca borra definitivamente por error; dos archivos con el mismo nombre no se pisan en la papelera → test en Task 3.
- **Carpeta de configuración**: sin `XDG_CONFIG_HOME` usa `$HOME/.config/xdir`; con el modo portable (`xdir.portable` junto al ejecutable) manda la regla del núcleo → test de la función pura en Task 1.

---

### Task 1: Crate, arranque y servicios simples

**Files:**
- Create: `crates/xdir-linux/Cargo.toml` (`[[bin]] name = "xdir-linux"`; dependencias `libc` y `notify` bajo `[target.'cfg(target_os = "linux")'.dependencies]`; features como en Global Constraints), `crates/xdir-linux/src/main.rs` (stub fuera de Linux: `eprintln!("xdir-linux solo funciona en Linux")`), `crates/xdir-linux/src/{sys,env,host}.rs`.
- Modify: `.github/workflows/release.yml` (línea del `sed` para `xdir-linux`), `Cargo.lock`.

**Interfaces:**
- Produces:
  - `pub fn install() ` en `sys.rs`: arma `sys::Services` con `LinuxWaker`, `LinuxEnv`, `LinuxFileOps`, `LinuxDiskScan`, `LinuxShell`, `LinuxFileSystem`, `LinuxOverlays` y llama `xdir_core::sys::install`. En esta tarea `FileOps`, `DiskScan`, `Shell` y `FileSystem` pueden ser los de la Task 2–3 todavía vacíos (devuelven `None`/`Err("No disponible en Linux")`); las Tasks 2–3 los completan.
  - `LinuxWaker`: sin hilo de interfaz; `post` no hace nada (documentarlo).
  - `LinuxOverlays::request`: no hace nada (Linux no tiene superposiciones de íconos).
  - `LinuxEnv`: `system_dark` = `false`, `system_accent` = `None`, `animations_allowed` = `true`, `key_down` = `false`, `config_dir` = `config_dir_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf>` (`$XDG_CONFIG_HOME/xdir` si es una ruta absoluta, si no `$HOME/.config/xdir`), formatos con las funciones neutras de `xdir_core::format` (las mismas que usa el falso del núcleo), historial de rutas escritas en `<config_dir>/rutas.txt` (una por línea), `consoles` vacío, `open_console` = `false`, el resto por defecto del trait.
  - `HeadlessHost` en `host.rs`: `impl WindowHost` sin ventana; `client_size` = `(1280.0, 800.0)`, `draw` devuelve `Ok(())` sin llamar a la clausura, `text_create` = `None`, `popup_menu` = `None`, `choose_color` = `None`, `ask_yes_no_cancel` = `None`, `start_drag` = `Err("No disponible en Linux")`, `expand_env` expande `$VAR` / `${VAR}` con `std::env::var`, `wheel_lines` = 3.0, `text_width` = 7.0 por carácter, el resto no hace nada.
  - `main` en Linux: `sys::install()`, `App::new(Init::load(session, None, None, Instant::now(), Vec::new()), Rc::new(HeadlessHost))` (con `startup_args()` del núcleo), e imprime `xdir-linux: el núcleo arrancó con los servicios de Linux; la interfaz de Linux todavía no está hecha.` y sale con código 0.

- [ ] **Step 1: Tests que fallan** (en `env.rs`, `#[cfg(test)]`): `config_dir_uses_xdg_config_home` (`Some("/x/conf")`, `Some("/home/a")` → `/x/conf/xdir`), `config_dir_falls_back_to_home` (`None`, `Some("/home/a")` → `/home/a/.config/xdir`), `config_dir_ignores_relative_xdg` (`Some("rel")`, `Some("/home/a")` → `/home/a/.config/xdir`), `config_dir_none_without_home` (`None`, `None` → `None`); en `host.rs`: `headless_host_has_fixed_size_and_no_text_boxes`.
- [ ] **Step 2:** FAIL en Docker (`cargo test -p xdir-linux`).
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en Docker; `docker run … cargo run -p xdir-linux` imprime el aviso y sale con 0; checks de Windows, Mac y Linux sin warnings; `cargo test` en la Mac igual que antes (188).
- [ ] **Step 5: Commit** `feat(linux): crate xdir-linux que arma el núcleo con servicios mínimos`.

### Task 2: `FileSystem` y `Watch` de Linux

**Files:**
- Create: `crates/xdir-linux/src/fs.rs` (y `fs/` con submódulos si pasa de ~400 líneas).
- Modify: `crates/xdir-linux/src/sys.rs` (usa `LinuxFileSystem`).

**Interfaces:**
- Consumes: Task 1.
- Produces: `LinuxFileSystem: FileSystem`:
  - `spawn_load`: un hilo que lee la carpeta con `std::fs::read_dir` + `symlink_metadata` y manda los lotes por `tx` con el mismo protocolo que la Mac (`LoadEvent`/`LoadMsg`: leer `crates/xdir-mac/src/fs/macos/mod.rs` y `xdir_core::fs` para el orden de mensajes, el tamaño de lote y el uso de `generation`/`current` para descartar cargas viejas) y despierta con `hwnd.post(WM_APP_LOAD, …)`; `Location::Computer` lista `drives()`.
  - `disk_entry`: nombre, tamaño, fecha de modificación como `filetime` (la misma unidad que usa el núcleo: revisar `xdir_core::fs::Entry` y el `disk_entry` de la Mac), carpeta/archivo, oculto si empieza con `.`, ícono `None`/índice neutro.
  - `is_hidden`: nombre que empieza con `.`. `searchable_dir`: todo salvo `/proc`, `/sys` y `/dev` cuando se buscan desde la raíz (comparar por nombre como hace la Mac con sus carpetas del sistema). `is_mount_point`: `st_dev` distinto del padre (`std::os::unix::fs::MetadataExt::dev`).
  - `drives()`: `/` más los puntos de montaje de `/proc/self/mounts` bajo `/media`, `/mnt` y `/run/media` (función pura `mounts_from(text: &str) -> Vec<PathBuf>` probada); `drive_name`/`drive_label`: el último componente o «Sistema de archivos» para `/`; `root_title`: `drive_name` para raíces.
  - Carpetas del usuario (`user_folder(i)`): `$HOME/{Escritorio|Desktop, Documentos|Documents, Descargas|Downloads, Imágenes|Pictures, Música|Music, Vídeos|Videos}` en el mismo orden que la Mac, leyendo `~/.config/user-dirs.dirs` si existe (función pura `user_dirs_from(text: &str, home: &Path) -> Vec<(String, PathBuf)>` probada).
  - `watch`: `notify` (inotify) igual que la Mac, despertando con `hwnd.post(msg, panel, lparam)`; el guardián se suelta en `Drop`.
  - Lo del Shell de Windows (`shell_location`, `shell_place_exists`, `shell_subfolders`, `quick_access`, `is_quick_pinned`, `quick_access_watch_dir`, `connect_share`, `network_place`, `linux_place`) devuelve vacío / `None` / `false` / `""`; `long_path` y `true_case` devuelven la ruta tal cual; `same_path` compara `(dev, ino)`.

- [ ] **Step 1: Tests que fallan**: `load_lists_files_and_folders` (carpeta temporal con 2 archivos y 1 carpeta → `spawn_load` manda por el canal esos 3 elementos y el mensaje de fin), `load_skips_nothing_with_non_utf8_name` (un archivo de nombre con el byte `0xFF` aparece listado sin pánico), `dotfiles_are_hidden`, `mounts_from_keeps_media_and_mnt` (texto de `/proc/self/mounts` de ejemplo → `/media/usb` y `/mnt/datos`, no `/proc` ni `/sys`), `user_dirs_from_reads_xdg_file` (`XDG_DOWNLOAD_DIR="$HOME/Descargas"` → `/home/a/Descargas`), `watch_reports_new_file` (crear un archivo en la carpeta vigilada → llega un `post`; usar un `Waker` de prueba que cuente).
- [ ] **Step 2:** FAIL en Docker.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en Docker; checks de los tres sistemas sin warnings.
- [ ] **Step 5: Commit** `feat(linux): sistema de archivos y vigilancia de carpetas`.

### Task 3: `FileOps`, `Shell` y `DiskScan` de Linux

**Files:**
- Create: `crates/xdir-linux/src/{ops,trash,shell,scan}.rs`.
- Modify: `crates/xdir-linux/src/sys.rs`.

**Interfaces:**
- Consumes: Task 1–2.
- Produces:
  - `LinuxFileOps::spawn`: un hilo por operación que manda un `OpResult` y despierta con `owner.post(WM_APP_OP_DONE, …)`, con la misma semántica que `crates/xdir-mac/src/ops/macos.rs` (leerlo: nombres «copia» ante conflicto, `Undo` que devuelve cada operación, `focus`, `rename_after` de la carpeta nueva y su nombre «Nueva carpeta»). Mover usa `rename` y, si falla con `EXDEV`, `copy_then_remove` (función aparte). Borrar con `recycle = true` usa la papelera de `trash.rs`; con `false`, `remove_file` / `remove_dir_all`. `Undo::Recycled` restaura desde la papelera.
  - `trash.rs`: `pub fn trash_dir(data_home: &Path) -> PathBuf` (`<data_home>/Trash`), `pub fn recycle(path: &Path, trash: &Path) -> io::Result<String>` (mueve a `files/<nombre>` —con sufijo ` (2)`, ` (3)`… si existe— y escribe `info/<nombre>.trashinfo` con `Path=` (codificado en URL, como pide freedesktop) y `DeletionDate=` en hora local `AAAA-MM-DDTHH:MM:SS`; devuelve el nombre dentro de la papelera), `pub fn restore(name: &str, original: &Path, trash: &Path) -> io::Result<()>`. `data_home` = `$XDG_DATA_HOME` o `$HOME/.local/share`.
  - `LinuxShell`: `open_file` / `open_item` con `xdg-open` (proceso aparte, sin esperar), `volume_of` = punto de montaje que contiene la ruta; portapapeles, `paste_virtual`, `pick_folder`, `properties`, `context_menu`, `invoke_verb`, fijar/quitar de acceso rápido → `None`/`false`/`Err("No disponible en Linux")`; `set_menu_mode` no hace nada.
  - `LinuxDiskScan`: `lister` = listador con `read_dir` + `symlink_metadata` que no sigue enlaces y cuenta las carpetas sin permiso como denegadas (como el de la Mac); `fast_scan` = no hay escaneo rápido (devolver el valor «no disponible» del tipo `FastScan`, mirar cómo lo hace la Mac); `elevate` = `None`; `drives` y `volume_space` con `libc::statvfs`; `whole_volume`: la ruta es un punto de montaje; `default_root` = `$HOME`; `permission_issue` = hubo carpetas denegadas; textos de aviso en castellano («Algunas carpetas no se pudieron leer por permisos.»).

- [ ] **Step 1: Tests que fallan**: `copy_renames_on_conflict`, `move_falls_back_to_copy` (llamar `copy_then_remove` directo: el origen desaparece y el destino tiene el mismo contenido), `rename_undo_restores_name`, `new_folder_is_focused_for_rename`, `recycle_writes_trashinfo` (en una papelera temporal: aparecen `files/a.txt` e `info/a.txt.trashinfo` con `Path=` codificado y `DeletionDate=`), `recycle_same_name_twice_keeps_both`, `restore_puts_file_back`, `lister_does_not_follow_symlink_loops` (enlace a la carpeta padre: el escaneo termina y no lo cuenta como carpeta), `lister_counts_denied_dirs` (carpeta con permisos `000`; si los tests corren como root, donde los permisos no aplican, el test se salta solo con un aviso), `shell_unsupported_returns_error` (`copy_files` → `Err("No disponible en Linux")`).
- [ ] **Step 2:** FAIL en Docker.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS en Docker (también como usuario no root: `docker run --user $(id -u):$(id -g) …` con `HOME=/tmp`); checks de los tres sistemas sin warnings; `cargo test` en la Mac igual que antes.
- [ ] **Step 5: Commit** `feat(linux): operaciones de archivos, papelera, shell y escaneo de disco`.

### Task 4: Documentación y PR

**Files:**
- Modify: `README.md` (la sección de arquitectura suma `xdir-linux`: qué hace hoy y cómo probarla: `cargo run -p xdir-linux` en Linux), `docs/superpowers/specs/2026-11-08-workspace-core-design.md` no se toca.

- [ ] **Step 1:** Actualizar el `README`.
- [ ] **Step 2:** Verificación final: `cargo test` en la Mac, `docker run … cargo test` (Linux), checks de Windows/Mac/Linux, y el `sed` de `release.yml` probado sobre una copia (las 4 crates quedan con la versión nueva y `cargo metadata --locked` pasa).
- [ ] **Step 3: Commit** `docs: README con xdir-linux`.
- [ ] **Step 4: Abrir el PR** (`feature/xdir-linux`, base `develop`) con confirmación del usuario. No hace falta lista para Runick93 más allá de «la CI de Windows sigue verde»: en Windows `xdir-linux` es un stub.

---

Después del merge: borrar la rama y el espacio de trabajo del plan, y anotar en la nota del vault que la migración del diseño terminó.

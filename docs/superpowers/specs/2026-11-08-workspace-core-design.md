# xdir en workspace: `xdir-core` + una crate por sistema — diseño

Fecha: 2026-11-08 · Rama: `refactor/workspace-core` (sale de `feature/xtree-macos`, PR #2)

## Intención

Hoy xdir es una sola crate: el código de cada sistema se elige con `#[cfg]` dentro de módulos
«fachada» (`fs`, `shell`, `ops`, `icons`, `thumbs`, `render`, `platform`, `drop`, `app/win.rs` vs
`app/mac.rs`). Nada verifica que Windows y macOS ofrezcan lo mismo; coinciden por costumbre.

Queremos un **workspace de Cargo** con:

- **`xdir-core`**: toda la funcionalidad, sin APIs del sistema. Se compila y se prueba en cualquier
  sistema, sin ventana.
- **`xdir-windows`**, **`xdir-mac`** y después **`xdir-linux`**: cada uno con su ventana, dibujo,
  sistema de archivos, shell, íconos y nada más. Cada uno genera el ejecutable `xdir` de su sistema.

Un cambio en el núcleo sirve a los tres. Un cambio de plataforma no puede romper a las otras, porque
el compilador no lo permite. Es la base para sumar Linux.

## Objetivos

- `xdir-core` sin dependencias de sistema (`windows`, `objc2*`, `dispatch2`, etc.) ni `#[cfg]` de
  sistema en código de producción.
- El núcleo define **interfaces (traits)** y cada crate de plataforma las implementa; la plataforma
  arma la `App` con sus implementaciones (enfoque A, elegido por el usuario).
- `xdir-windows` y `xdir-mac` con el mismo comportamiento que hoy: misma interfaz, mismas funciones,
  mismo empaquetado (`xdir.exe` con instalador, `xdir.app` con dmg).
- CI en cada PR: Windows (build + test), macOS (build + test), Linux (build + test de `xdir-core`).
- `xdir-linux` existe como crate que compila en Linux y usa el núcleo; mostrar una ventana en Linux
  es un proyecto aparte (ver «No objetivos»).
- Tests del núcleo (incluidos los de `App`) que corren en los tres sistemas con implementaciones
  falsas de las interfaces.

## No objetivos

- La interfaz gráfica de Linux. Elegir GTK4 o winit + tiny-skia + cosmic-text y portar ventana,
  dibujo, íconos, miniaturas, papelera y arrastrar y soltar se diseña aparte, cuando este trabajo esté
  mergeado. Acá solo se deja la crate y el núcleo verificado en Linux.
- Funciones nuevas o cambios de aspecto.
- Reescribir lógica que hoy funciona: se mueve y se le cambia la forma de llamar al sistema.

## Anti-objetivos (sería un fracaso aunque «funcione»)

- Que algún comportamiento de Windows o macOS cambie sin quererlo (rendimiento incluido: dibujo,
  carga de carpetas, escaneo de xtree).
- Un PR gigante imposible de revisar, o una rama larga que acumule conflictos con lo que Runick93
  sigue haciendo.
- Que el núcleo siga dependiendo de una plataforma «por la puerta de atrás» (una crate de fachada
  elegida por `cfg`, `not(windows)` que en realidad significa macOS, etc.).
- Genéricos que se propagan por todo el núcleo (`App<P>`, `Pane<P>`…).

## Restricciones

- Rust 2024, `windows` 0.62, `objc2` 0.6 / `objc2-*` 0.3, como hoy.
- `cargo build` y `cargo test` en la raíz del workspace funcionan en Windows, macOS y Linux: las
  crates de otro sistema compilan vacías (stub) fuera de su sistema.
- Cada crate de plataforma genera un binario con su propio nombre (`xdir-windows`, `xdir-mac`,
  `xdir-linux`), así `cargo build --workspace` no produce dos `target/<perfil>/xdir` que se pisen.
  `scripts/package-mac.sh`, `scripts/package.ps1`, `installer/xdir.iss` y `release.yml` lo copian
  como `xdir` / `xdir.exe`: el usuario ve el mismo nombre de siempre (decisión de quien diseña).
- `build.rs` (recursos de Windows) y `.cargo/config.toml` (`+crt-static`) pasan a `xdir-windows`.
- Features `arbol` y `pruebas`: se conservan; las declara la crate donde viven y la de plataforma las
  reexpone.
- Cada paso termina mergeado en `develop` por PR, revisado por Runick93. Desde la Mac, Windows se
  verifica con `cargo check --target x86_64-pc-windows-msvc` y con la CI de Windows.
- Textos y comentarios en castellano. Conventional Commits sin atribución a Claude.

## Enfoques considerados

- **A. Traits definidos en el núcleo, implementados por cada plataforma y pasados como objetos
  (`&dyn` / `Box<dyn>` / `Arc<dyn>`)** — **elegido**. El compilador obliga a que las tres
  plataformas ofrezcan lo mismo, el núcleo se prueba con implementaciones falsas y el costo (una
  llamada indirecta por operación de dibujo) no se nota.
- B. Los mismos traits con genéricos (`App<P: Platform>`): sin costo en tiempo de ejecución, pero el
  parámetro se propaga a todo el núcleo; más difícil de leer y más lento de compilar.
- C. Mover las fachadas a una crate `xdir-platform` elegida por `cfg`: menos trabajo, pero el núcleo
  sigue dependiendo de la plataforma y no se gana lo que se busca.

## Estado de partida (mapa, 25.400 líneas)

- **Va al núcleo tal cual (~12.500):** `config`, `toml`, `groups`, `jump`, `favorites`, `palette`,
  `about`, `anim`, `ui`, `list`, `pane`, `tab`, `tree`, `xtree/*`, `app/{actions, appearance, edit,
  events, favs, fileops, input, panes, quick_jump, saved_groups, tests}`. Llaman a la plataforma por
  las fachadas o por `Painter`; esas llamadas pasan a ser llamadas a traits.
- **Hay que partir (~3.000):** los `mod.rs` de cada fachada (los tipos neutros van al núcleo),
  `format.rs`, `typed.rs`, `console.rs`, `theme.rs` (`apply_to_window`), `session.rs`
  (`config_dir`), `commands.rs` (atajos y nombres de macOS), `app/mod.rs`, `app/paint.rs` (UIA),
  `main.rs`.
- **Solo Windows (~6.000):** `fs/win32/*`, `shell`, `ops`, `icons`, `thumbs`, `render`, `platform`,
  `drop` en su versión win32, `app/{win, messages, uia}.rs`.
- **Solo macOS (~3.900):** las versiones `macos` de lo mismo y `app/mac.rs`.
- Los hilos de trabajo ya hablan con la ventana de forma neutra: `Handle::post(WM_APP + n, …)` como
  despertador y el contenido por canales `mpsc`.
- Acoples grandes: (1) `Painter` concreto por sistema y `p.fonts.<x>` cuyo tipo es
  `IDWriteTextFormat` o `TextFormat` (84 usos); (2) ~25 funciones de ventana (`TextBox`,
  `popup_menu`, `focus_main`, `invalidate`, `client_size`, `beep`…) que `app/` importa con
  `use win::*` / `use mac::*`, más 8 métodos de `App` escritos en `win.rs` / `mac.rs`.
- Diferencias hoy entre sistemas: el `Result` de `shell` y `render` es `windows::core::Result` en
  Windows y otro tipo en macOS; funciones que solo existen de un lado (`shell::reveal`,
  `platform::popup_menu`, `render::set_size`, `drop` solo en Windows…).
- `not(windows)` se usa para decir «macOS» en `fs/mod.rs`, `console.rs`, `typed.rs`, `session.rs` y
  tests (`open -a Terminal`, `~/Library/Application Support`, «Macintosh HD»).

## Arquitectura

### Workspace

```
Cargo.toml                 [workspace] members = ["crates/*"], dependencias compartidas
crates/xdir-core/          lib: App, paneles, lista, árbol, paleta, xtree, config, sesión, formatos,
                           tipos neutros y los traits de plataforma
crates/xdir-windows/       bin "xdir": ventana Win32, Direct2D, Shell, MFT, UIA, instalador
crates/xdir-mac/           bin "xdir": AppKit, Core Graphics, getattrlistbulk, notify
crates/xdir-linux/         bin "xdir": por ahora solo arma el núcleo con implementaciones mínimas
                           y sale con un aviso; la UI de Linux es otro proyecto
```

- Fuera de su sistema, cada crate de plataforma compila a un `main` que avisa y sale (todo su código
  real bajo `#[cfg(…)]` en la raíz de la crate). Así `cargo build/test --workspace` anda en todos.
- `xdir-core` no tiene dependencias de sistema. Puede usar crates portables si hace falta (ninguna
  prevista).

### Interfaces que define el núcleo

Nombres y métodos de partida (quien implementa puede ajustar firmas; el criterio es que el núcleo no
vea tipos de sistema):

| Trait | Qué cubre |
|---|---|
| `Waker` (Send + Sync) | Despertar el hilo de interfaz: `post(msg, wparam, lparam)`. Reemplaza la implementación de `Handle::post`; se conservan los números `WM_APP + n` y los canales actuales. |
| `FileSystem` (Send + Sync) | Carga de carpetas (`spawn_load`), ganchos de la búsqueda (`search_metadata`, `is_hidden`, `searchable_dir`, `is_mount_point`, `disk_entry`, `with_search_thread`; `spawn_search` pasa al núcleo), unidades y carpetas del usuario, ubicaciones del Shell, acceso rápido, `long_path`, `true_case`, `connect_share`, `same_path`, título y migas de la raíz (saca el `cfg` de `Location::title/crumbs`), y `watch(…) -> Option<Box<dyn Watch>>`. |
| `Watch` | Guarda de vigilancia de carpeta (solo `Drop`). |
| `DiskScan` | xtree: `lister`, `fast_scan`, `elevate`, `drives`, `volume_space`, `whole_volume`, `default_root`, `permission_issue`, textos del aviso. Reemplaza las funciones `fs::xtree_*` de hoy. |
| `Shell` | Portapapeles de archivos y texto, pegar virtual, elegir carpeta, abrir, propiedades, menú contextual, verbos, fijar/quitar de acceso rápido, modo oscuro de menús, `volume_of`. Errores como `SysError(String)` del núcleo. |
| `FileOps` | `spawn(waker, Sender<OpResult>, Op)`. |
| `Overlays` | `request(OverlayRequest)`. Íconos y miniaturas quedan dentro del renderizador de cada plataforma. |
| `Painter` (object-safe) | `fill`, `round_fill`, `rounded_path`, `circle`, `circle_stroke`, `round_stroke(_w)`, `text(FontId, …)`, `measure(FontId, …)`, `shell_icon`, `thumbnail`, `logo`, `thumb_px`, `clip(Rect, &mut dyn FnMut())`, `theme()`, `file_colors()`. `FontId` = {Ui, UiRight, UiCenter, UiBold, Title, Small, SmallBold, Glyphs, GlyphsSmall, Label}. |
| `WindowHost` (hilo de interfaz) | Repintar, tamaño, cursor, título, cerrar, mostrar, ubicación de la ventana; foco y mouse; `beep`, `ask_yes_no_cancel`, `open_url`, `open_in_editor`, elegir color; `popup_menu(&[Entry], MenuPos, dot)` (el menú de favoritos pasa a ser un `Entry` anidado); cajas de texto por id (`TextKind`, crear, texto, seleccionar, foco, destruir); tema de la ventana (absorbe `Theme::apply_to_window`); arrastrar (`start_drag`); `wheel_lines`, `expand_env`; `text_width`. |
| `SystemEnv` | Oscuro/acento del sistema, animaciones permitidas, `key_down`, carpeta de configuración, historial de rutas escritas, consolas (lista, nombre, abrir, verbos alternativos), formato de fecha y tamaño, atajos y nombres propios del sistema (lo de `MAC_KEYS`, `MAC_NAMES`, `combo_label`). |
| `Accessibility` (opcional) | `notify_focus` con un modelo de solo lectura que el núcleo expone (elementos, posiciones, selección) en vez de que `uia.rs` lea el interior de `App`. |

- La `App` del núcleo guarda una estructura `Platform` con esas implementaciones (`Arc<dyn …>` /
  `Box<dyn …>`), creada por la crate de plataforma al arrancar.
- `App::paint` recibe `&dyn Painter`; el renderizador vive en la plataforma, que llama
  `app.paint(p, w, h)` dentro de su `draw`.
- Los eventos que llegan del sistema siguen entrando por los métodos `on_*` de `App`, que pasan a
  ser públicos para las crates de plataforma.
- Los tipos que hoy cruzan y son neutros (`Color`, `Rect`, `Key`, `Handle`, `Location`, `Entry`,
  `LoadEvent`, `shell::Item`, `shell::Entry`, `Op`, `OpResult`, `DragEvent`, `Pixels`,
  `thumbs::Source`, tipos de xtree) pasan al núcleo sin cambios. `Entry.icon` sigue siendo un índice
  opaco que interpreta la plataforma.
- Las dependencias internas de cada plataforma (fs → icons, thumbs → icons, shell → ops, drop →
  shell, render → icons/thumbs) quedan dentro de su crate.

### Orden de trabajo (cada paso es un PR a `develop`)

1. **CI**: workflow en cada PR con Windows (`cargo build` + `cargo test` + prueba de humo) y macOS
   (`cargo test`). `fmt` y `clippy` solo como aviso, sin bloquear. Linux se suma en el paso 3.
2. **Workspace sin cambiar lógica**: mover todo a `crates/xdir` (una sola crate), ajustar scripts,
   `build.rs`, `.cargo/config.toml`, `release.yml` e instalador. Verificación: los paquetes de
   Windows y macOS salen iguales.
3. **Crear `xdir-core`** y pasarle lo que ya es puro y no depende de fachadas (`config`, `toml`,
   `groups`, `format` neutro, tipos de `platform`, `xtree` sin `view`, etc.). `xdir` depende de él.
4. **Una interfaz por PR**, en este orden (de menor a mayor riesgo): `SystemEnv` → `Waker` →
   `FileOps` → `Overlays` → `Shell` → `FileSystem` + `Watch` → `DiskScan` → `Painter` (incluye
   `FontId`) → `WindowHost` (incluye cajas de texto y menús) → `Accessibility`. Cada uno mueve al
   núcleo los módulos que dejan de necesitar la fachada.
5. **Separar `xdir-windows` y `xdir-mac`**: lo que queda en `crates/xdir` se reparte; se borra
   `crates/xdir`. El núcleo queda sin `#[cfg]` de sistema.
6. **`xdir-linux`**: crate que compila en Linux y arma el núcleo; CI de Linux corriendo los tests del
   núcleo.

### Tests

- Implementaciones falsas de cada trait en el núcleo (`#[cfg(test)]` o un módulo `testing` público
  para que las plataformas también las usen). Los tests de `App` (`App::new(Init::blank())`) dejan de
  crear un renderizador real.
- Cada paso: `cargo test` en macOS y Windows (CI) sin tests perdidos; el número de tests no baja.
- Los tests que hoy dependen del sistema (rutas `C:\`, `fs/macos/xtree.rs`, `mft.rs`) se quedan en
  su crate de plataforma.

### Coordinación y verificación

- **Con Runick93** (decisión del usuario): antes del paso 2 él mergea o pausa lo que tenga abierto.
  Los PRs se abren encadenados (cada uno sale del anterior) sin esperar cada merge; él los revisa
  en orden.
- **Windows** (decisión del usuario): cada PR lleva una lista corta de pruebas manuales que hace
  Runick93. Además la CI de Windows suma una prueba de humo que arranca xdir con la feature
  `pruebas` (comandos por WM_COPYDATA), navega a una carpeta, cambia de layout, abre la paleta y
  la vista de xtree, y verifica que dibuja sin errores. Esa prueba se agrega en el paso 1 y corre en
  todos los PRs siguientes.
- **Paquetes en el paso 2**: «iguales» quiere decir mismos archivos dentro del zip, el dmg y el
  instalador, misma versión y la app arranca; no idénticos byte a byte.
- **Rendimiento**: antes de los pasos `Painter` y `DiskScan` se mide el arranque con `--probe` y el
  escaneo de xtree (`walk` y MFT) sobre las mismas carpetas; se repite después. Margen aceptado:
  ±10 %. Los números van en el PR.
- **Linux en la CI**: entra en el paso 3, cuando existe `xdir-core`; antes no hay nada que compile
  en Linux.

## Decisiones tomadas por el usuario

- Workspace con `xdir-core` + `xdir-windows`, `xdir-mac`, `xdir-linux`.
- Enfoque A (traits como objetos).
- xtree cerrado antes de migrar (PR #2).
- Runick93 frena su trabajo antes del paso 2; PRs encadenados.
- Verificación de Windows: lista manual para Runick93 + prueba de humo en la CI.

## Lo que queda a criterio de quien implementa

- Firmas exactas de los traits, mientras el núcleo no vea tipos de sistema.
- Si los traits se agrupan en un solo `Platform` o se pasan por separado.
- Cómo compila cada crate de plataforma fuera de su sistema (stub en la raíz con `cfg`).
- El momento exacto de mover cada módulo neutro, dentro del orden propuesto.
- `fmt`/`clippy` en la CI como aviso o como error (propuesta: aviso).

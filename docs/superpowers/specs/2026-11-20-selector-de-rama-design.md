# Selector de rama de git — diseño

Fecha: 2026-11-20. Tamaño: cambio mediano (núcleo, módulo nuevo `git`, pie del panel; sin código por plataforma). Estado: pendiente de aprobación.

## Intención

Cuando se explora una carpeta que es un repositorio de git, hoy hay que salir a la terminal para saber
en qué rama se está o para cambiarla. Queremos que **el pie de cada panel lo diga y permita cambiarla**:
un botón con la rama actual que abre un menú con las ramas locales.

## Objetivos

- Un botón con el nombre de la rama actual, en el pie de cada panel, a la **izquierda** de «Carpeta nueva».
- Al hacer clic, un menú nativo (crece hacia arriba) con las ramas locales; elegir una cambia de rama.
- Solo aparece si se cumplen las tres condiciones: `git` está instalado, la pestaña es una ruta local
  (`Location::Path`) y la carpeta (o alguno de sus padres) es un repositorio. Si falta alguna, el botón
  no se dibuja ni reserva lugar.
- Usa el `git` de la consola. No se necesita `gh` ni que el repositorio tenga remoto.
- Si el cambio de rama falla por **cambios sin commitear que chocan**, se le pregunta al usuario
  (stash o descartar). Solo en ese caso.
- Funciona igual en Windows, macOS y Linux: el código vive en `xdir-core`.

## No objetivos

- Ramas remotas, crear, borrar o fusionar ramas, ni abrir PR (eso sí sería `gh`).
- Indicador de «sucio» o de adelantado/atrasado respecto al remoto.
- Vigilar `.git/HEAD` en vivo: si la rama cambia desde otra terminal, el botón se pone al día en la
  siguiente recarga de la pestaña, al abrir el menú o tras un cambio hecho desde xdir.
- Lista de worktrees, ni tratamiento especial de submódulos más allá de leer su rama.
- Ayudar a instalar git: sin git no hay botón y no se avisa de nada.
- Un cuadro propio con la lista de archivos que chocan (queda como posible mejora, ver Enfoques).

## Anti objetivos (sería un fracaso aunque «funcione»)

- **Perder cambios del usuario** sin que lo haya confirmado de forma explícita.
- Congelar la interfaz: ningún proceso `git` corre en el hilo de la ventana.
- Un proceso `git` por cada carpeta que se abre, o una consola que parpadea en Windows.
- Mostrar el botón en un lugar donde no puede funcionar (red, archivo comprimido, «Este equipo»).
- Mostrar el cartel de stash/descartar cuando git falló por otra causa.

## Enfoques considerados

**Cómo se obtiene el nombre de la rama**

| | Qué es | Pro | Contra |
|---|---|---|---|
| A1 | Lanzar `git` en cada navegación | Una sola vía | Un proceso por carpeta abierta; siempre asíncrono |
| **A2 (elegido)** | Leer `.git/HEAD` en el programa; `git` solo para listar, cambiar y stash | Instantáneo, no lanza nada al navegar | Hay que cubrir `.git` como archivo (worktrees, submódulos) y HEAD suelto |

**El cartel de cambios sin commitear**

| | Qué es | Pro | Contra |
|---|---|---|---|
| **B1 (elegido)** | Dos pasos con diálogos nativos del host (`ask_yes_no_cancel`, luego `ask_yes_no`) | Usa lo que existe, no toca ningún host, el descarte tiene su confirmación aparte | Dos cuadros en vez de uno |
| B2 | Ventana propia del núcleo con tres botones y la lista de archivos | Un solo cuadro, más informativo | Mucho más trabajo |

Eligió Leandro: **A2 + B1**. Si el cartel necesita la lista de archivos, se pasa a B2 sin rehacer lo
demás, porque la lógica de git queda separada de la presentación.

## Diseño

### 1. Módulo `git` (núcleo, solo `std`)

Un archivo nuevo `crates/xdir-core/src/git.rs`, sin dependencias nuevas.

- `available() -> bool`: ejecuta `git --version` **una sola vez** (`OnceLock`) y recuerda el
  resultado. Se calienta en un hilo al arrancar; hasta que responde, `false` (el botón aparece unos
  milisegundos después, nunca bloquea el arranque).
- `read_head(dir: &Path) -> Option<RepoHead>`: sube por los padres buscando `.git`. Si es una
  **carpeta**, lee `HEAD` ahí; si es un **archivo** (`gitdir: <ruta>`), sigue la ruta, que puede ser
  relativa. `RepoHead { root: PathBuf, head: Head }` con `Head::Branch(String)` (`ref: refs/heads/x`)
  o `Head::Detached(String)` (los 7 primeros caracteres del hash). Cualquier error de lectura
  devuelve `None`. No lanza procesos.
- `branches(root) -> Result<Vec<String>, GitError>`: `git for-each-ref --sort=-committerdate
  --format=%(refname:short) refs/heads`, las más recientes primero.
- `switch(root, name) -> SwitchOutcome`:
  - `Switched`;
  - `Dirty`: git se negó por cambios locales que chocan;
  - `Failed(String)`: cualquier otra causa, con el texto de git.
- `stash_and_switch(root, name)`: `git stash push -u -m "xdir: antes de pasar a <name>"` y luego
  `switch`.
- `discard_and_switch(root, name)`: `git switch --discard-changes <name>`. Solo descarta lo que ya
  tiene seguimiento; los archivos nuevos sin seguimiento no se tocan.
- `dirty_count(root) -> usize`: líneas de `git status --porcelain -uno`, para el texto de la
  confirmación de descarte.

**Cómo se lanza `git`** (un único helper privado, usado por todo lo anterior):
`Command::new("git").current_dir(root)` con `LC_ALL=C` y `GIT_TERMINAL_PROMPT=0` (para que los
mensajes no estén traducidos y nunca espere una contraseña), `stdin` nulo, y en Windows
`CREATE_NO_WINDOW` (`std::os::windows::process::CommandExt`, bajo `cfg(windows)`).

**Detectar el caso «sucio»**: `switch` devuelve `Dirty` solo si git falló y el error contiene
«would be overwritten» o «commit your changes or stash them» (con `LC_ALL=C` el texto es estable).
Cualquier otro fallo es `Failed(texto)`.

### 2. Estado por pestaña

`Tab` gana `git: Option<RepoHead>`. Se calcula con `git::read_head` cuando una carga de la pestaña
termina (también en una recarga), solo si la ubicación es `Location::Path` y `git::available()`;
en cualquier otro caso queda `None`. El botón del pie lee este campo.

Restricción: `read_head` toca el disco. No debe bloquear la ventana en unidades de red. El
constructor decide si va en el hilo de carga o se omite para rutas remotas, pero el resultado visible
es el mismo.

### 3. El botón del pie (`pane.rs`, `input.rs`, `tooltip.rs`)

- En `Pane::draw`, junto a `view_button` y `new_folder_button` ([pane.rs:719](crates/xdir-core/src/pane.rs:719)):
  si la pestaña tiene `git`, un `branch_button: Rect` a la izquierda de «Carpeta nueva», con el glifo
  de rama y el nombre. El ancho se ajusta al texto con un tope; si el nombre no cabe, se corta con
  puntos suspensivos. Con `Head::Detached`, se muestra el hash corto.
- Variante `PaneHit::BranchButton`, resuelta en el mismo `hit` que los otros dos botones
  ([pane.rs:486](crates/xdir-core/src/pane.rs:486)). Cuando no hay botón, su `Rect` queda vacío y no
  recibe clics (igual que en las páginas).
- Tooltip: «Rama: <nombre> (clic para cambiar)»; con HEAD suelto, «HEAD suelto en <hash>».

### 4. El menú y el cambio de rama (`app/`)

Un módulo nuevo `app/git_branch.rs` con la lógica del clic. Al hacer clic en `BranchButton`:

1. Releer `read_head` (por si cambió afuera) y pedir `git::branches` en un hilo.
2. Abrir `host.popup_menu(&entries, MenuPos::Above(r.left, r.top), ...)`. La rama actual va marcada y
   deshabilitada. Máximo 40 ramas (las más recientes); si hay más, una última entrada deshabilitada
   «…y N más».
3. Si el usuario elige una rama, correr `git::switch` en un hilo. Mientras corre, los clics sobre
   el botón se ignoran.
4. Según el resultado:
   - `Switched`: recargar **todas las pestañas del mismo repositorio** (en cualquier panel) y
     actualizar el botón.
   - `Failed(texto)`: mostrar el texto de git tal cual. Se reutiliza el mecanismo de aviso de errores
     que ya exista en `App` (barra de estado); es una decisión del constructor cuál.
   - `Dirty`: ver 5.

### 5. El cartel de cambios sin commitear (B1)

Solo tras `Dirty`, en este orden y con `host.ask_yes_no_cancel` / `host.ask_yes_no`:

1. `ask_yes_no_cancel("No se puede pasar a «<rama>»", "Tienes cambios sin guardar que chocan con esa
   rama. ¿Guardarlos en el stash y cambiar? Sí: guardar en el stash y cambiar. No: descartar los
   cambios. Cancelar: quedarse aquí.")`.
2. **Sí** → `stash_and_switch`. Al terminar bien, un aviso corto: «Tus cambios están en el stash
   (xdir: antes de pasar a <rama>)». El stash no se reaplica solo.
3. **No** → `ask_yes_no("Descartar cambios", "Se perderán <N> archivos modificados. Esto no se puede
   deshacer. ¿Descartar y cambiar?")`; solo si dice Sí, `discard_and_switch`.
4. **Cancelar** (o Esc) → no pasa nada.

Si el stash o el cambio fallan después de empezar, se muestra el error de git y se dice que los
cambios quedan en el stash (nunca se intenta recuperarlos con `stash pop` por cuenta propia).

## Pruebas

- `git`: contra repositorios reales creados en una carpeta temporal (`git init`, commits, ramas),
  con las pruebas omitidas si `git` no está instalado. Casos: rama normal, HEAD suelto, `.git`
  como archivo con ruta relativa, carpeta sin repo, `switch` limpio, `switch` con conflicto (`Dirty`),
  `stash_and_switch`, `discard_and_switch` (los archivos sin seguimiento se conservan),
  `dirty_count`, nombre de rama inexistente (`Failed`).
- Botón: `available()` falso → sin botón y sin `Rect`; ruta de red o archivo comprimido → sin botón;
  clic abre el menú con las ramas; elegir una cambia la rama y recarga las pestañas del repo.
- Cartel: el doble de pruebas del host ya registra menús y respuestas; casos Sí, No+Sí, No+No,
  Cancelar, y que `Failed` por otra causa no muestre el cartel.
- Hit-test y tooltip del botón, en el mismo estilo que los de «Carpeta nueva» y «Cambiar la vista»
  ([tests.rs:1235](crates/xdir-core/src/app/tests.rs:1235)).

## Lo que queda para el constructor

- Cómo (hilo de carga u omisión) se evita bloquear en rutas de red (sección 2).
- Qué mecanismo de aviso de errores usar para `Failed` (sección 4).
- El glifo de rama y el ancho máximo del botón.
- Cómo se inyecta `available()` en las pruebas (un interruptor de prueba o un parámetro).
- Si `GIT_OPTIONAL_LOCKS=0` conviene en `status` para no interferir con otra herramienta.

# xtree dentro de xdir (Windows y macOS) — diseño

Fecha: 2026-11-08 · Rama: `feature/xtree-macos`

## Intención

xtree es el mapa de uso de disco al estilo WizTree que hizo Runick93 (PR #1). Hoy es un programa
aparte, solo para Windows, que xdir abre desde el botón «xtree» de su barra de estado.

Lo queremos **dentro de xdir, como una función más**, en Windows y en macOS. El programa `xtree`
aparte se elimina en los dos sistemas.

Dos razones:
- Reutilizar lo máximo posible. xtree pasa a dibujar con el mismo `Painter`, el mismo tema y los
  mismos eventos que xdir, y la lógica se escribe una sola vez para los dos sistemas.
- Preparar la migración que viene después: un workspace con `xdir-core`, `xdir-windows`,
  `xdir-mac` y `xdir-linux` (fuera de este trabajo). Por eso todo lo que no dependa del sistema
  se escribe sin `#[cfg]`, en módulos que después pasan tal cual al núcleo.

## Objetivos

- **xtree en pantalla completa dentro de xdir.** El botón «xtree» de la barra de estado, o un
  comando de la paleta, reemplaza el área de los paneles por la vista de xtree. Se analiza la
  carpeta del panel activo, o el disco del sistema si el panel no está en una carpeta. El mismo
  botón o Esc vuelven a los paneles.
- **Paridad funcional con el xtree de hoy** (commit `879b0e9`):
  - mapa *squarified* anidado, con colores por posición o por tipo (T) y su leyenda;
  - tabla con «Subir», tamaño, % y barra;
  - tarjeta de detalle con tres acciones;
  - desplegable de discos, más «Seleccionar carpeta…» y «Analizar»;
  - barra de datos del disco: carpeta actual, espacio total, utilizado y libre, sin analizar;
  - mapa en vivo durante el escaneo, cancelación al volver a analizar y F5;
  - clic selecciona, doble clic entra, clic derecho o Retroceso suben;
  - lectura rápida de la MFT en Windows.
- **Acciones de la tarjeta adaptadas a estar dentro de xdir:**
  - «Mostrar en xdir» pasa a ser **«Abrir en el panel»**: vuelve a los paneles y el panel activo
    navega a esa carpeta (o a la carpeta del archivo, con el archivo seleccionado);
  - «Copiar ruta» queda igual;
  - «Explorar aquí» queda igual (entra en la carpeta dentro del mapa).
- **macOS:** un escáner propio (ver «Escáner en Mac») y el aviso de permisos.
- **Diseño en FigCraft antes de programar**, en `design/xdir.figcraft`, con `docs/DISENO.md`
  como en cromos (ver «Diseño en FigCraft»). La implementación arranca cuando el usuario apruebe
  esas pantallas.
- Se borran `src/bin/xtree/`, el `[[bin]]` de Cargo.toml y su feature, y el empaquetado de
  `xtree.exe` (`scripts/package.ps1`, `installer/xdir.iss`, incluido el taskkill).

## No objetivos

- La migración a workspace y Linux. Viene después, en otro diseño.
- Funciones nuevas de xtree: borrar desde el mapa, exportar, comparar escaneos, etc.
- Una ventana aparte o el mapa como vista de un solo panel (descartados por el usuario).

## Anti-objetivos (sería un fracaso aunque «funcione»)

- Que la lógica de xtree quede repartida con `#[cfg]` en vez de núcleo neutro más plataforma.
- Que en Mac `/` cuente dos veces el volumen de datos o sume discos externos.
- Que las carpetas protegidas por macOS queden en 0 sin ningún aviso.
- Perder la lectura de la MFT o el rendimiento del escáner en Windows.
- Que un escaneo en curso trabe la interfaz de xdir, o siga gastando CPU sin que nadie lo vea
  después de cerrar xdir.

## Restricciones

- Rust 2024, un solo crate y un solo binario (`xdir`). Dependencias de siempre: `windows` 0.62
  y `objc2` 0.6 / `objc2-*` 0.3.
- `cargo build` y `cargo test` tienen que andar en los dos sistemas. Desde la Mac se verifica
  además con `cargo check --all-targets --target x86_64-pc-windows-msvc`.
- La prueba funcional en Windows la hace **Runick93** al revisar el PR. El PR lleva una lista
  de qué probar.
- Feature `xtree`: se elimina y xtree queda siempre incluido. Es una decisión de quien
  implementa; si se prefiere conservarla, que siga apagando el botón.
- macOS 12 como mínimo. Textos y comentarios en castellano.

## Enfoques considerados

- **Pantalla completa dentro de xdir** — **elegido**. Ocupa el área de los paneles y conserva el
  layout de Runick93. Es la opción que menos cambia su diseño.
- Vista de un panel, que cambia la lista por el mapa, como la búsqueda. Se integra más, pero con
  3 o 4 paneles el mapa queda chico y hay que rediseñar.
- Ventana aparte del mismo proceso. Hoy xdir maneja una sola ventana; habría que agregar soporte
  para varias.
- (Antes de este cambio de rumbo) xtree como programa aparte con un núcleo compartido: quedó
  descartado al decidir integrarlo.

## Arquitectura

### Módulos

| Módulo | Qué tiene | ¿Depende del sistema? |
|---|---|---|
| `src/xtree/mod.rs` | Estado de la vista (`Xtree`): árbol, carpeta actual, selección, hover, scroll, `by_type`, menú de discos, escaneo en curso | No |
| `src/xtree/tree.rs` | `Node`, `NO_PARENT`, `rows_of`, `node_path` (con el separador del sistema), orden final | No |
| `src/xtree/scan.rs` | Cola, hilos, `insert_listing`, agrupado «… N archivos pequeños», deduplicación de enlaces duros, contadores y fase, cancelación, aviso de fin por el buzón de la ventana (`Handle::post`) | No |
| `src/xtree/treemap.rs` | `squarify`, `layout`, `Cell`, `header_h` | No |
| `src/xtree/colors.rs` | Rueda, HSL, `position_color`, categorías por extensión, `text_on`, `fmt_size` | No |
| `src/xtree/view.rs` | Layout de la vista, hit-tests, eventos (mouse, rueda, teclas) y `paint(&Painter)` | No |
| `src/fs/win32.rs` (o `fs/win32/scan.rs`) | Listado con `NtQueryDirectoryFile`, el `mft.rs` de hoy, discos, espacio del volumen | Windows |
| `src/fs/macos.rs` (o `fs/macos/scan.rs`) | Listado con `getattrlistbulk`, discos, espacio del volumen, firmlinks | Mac |

- La interfaz con cada sistema es la misma que ya usa xdir: funciones con el mismo nombre en
  `fs::win32` y `fs::macos`, reexportadas desde `fs`. Hacen falta `xtree_list_dir`,
  `xtree_fast_scan` (MFT en Windows; en Mac no hace nada), `drives`, `volume_space`,
  `whole_volume` y el texto o la acción de permisos. Así, en la migración siguiente, esas
  funciones pasan a ser el trait del núcleo.
- Elegir carpeta, copiar al portapapeles y los íconos usan lo que xdir ya tiene: el diálogo de
  carpeta, el portapapeles de `ops` y `shell`, y los glifos del `Painter`, que en Mac ya se
  traducen a SF Symbols.
- `mft.rs` pasa entero a `fs/win32/` y solo cambia su `use super::…` a los tipos nuevos. Su
  lógica no se toca.

### Dentro de xdir

- `App` suma `xtree: Option<Xtree>`. Con la vista abierta:
  - `paint` dibuja xtree en el área de los paneles, incluida la franja de grupos;
  - el árbol lateral se oculta;
  - la barra de estado de xdir queda, con el botón «xtree» encendido;
  - mouse y teclas van a la vista. Sus teclas (T, F5, Retroceso, Esc, A) tienen prioridad sobre
    los atajos de xdir, salvo los globales de ventana (cerrar, paleta).
- Al cerrar la vista, el escaneo sigue en segundo plano. Al volver a abrirla se ve el resultado
  sin analizar de nuevo. «Analizar» y F5 lo rehacen. Al cerrar xdir, el escaneo se cancela.
- El refresco en vivo (250 ms) usa el timer o la animación que ya tiene xdir.
- La vista usa los **colores del tema de xdir** (claro u oscuro, y el acento) para fondo, paneles,
  bordes, texto y botones. Los colores del mapa (la rueda y las categorías) quedan como están.
  Es decisión de quien implementa: hoy xtree es oscuro fijo. Las pantallas de FigCraft lo
  muestran para que el usuario lo apruebe.
- Comando nuevo en la paleta: «Mapa de uso de disco (xtree)». Sin atajo por defecto; se puede
  configurar como los demás.
- Se borra `open_xtree`, que lanzaba el programa aparte. El `cfg!(all(windows, …))` de
  `ui.rs` y `app/tests.rs` se va con la feature.

### Escáner en Mac

- `getattrlistbulk` por carpeta, pidiendo nombre, tipo, id de archivo, cantidad de enlaces,
  tamaño asignado y flags. No sigue enlaces simbólicos.
- Enlaces duros (`nlink > 1`): se cuentan una sola vez por id, igual que en Windows.
- Archivos que están solo en iCloud (`SF_DATALESS`): no suman, igual que los OFFLINE de Windows.
- **Analizar `/`:**
  - se recorre el volumen del sistema y el de datos;
  - se pasa por los firmlinks, porque el cambio de dispositivo entre esos dos volúmenes está
    permitido;
  - se saltea `/System/Volumes` y no se cruza a ningún otro dispositivo (discos externos, VM,
    Preboot, etc.);
  - las rutas que se muestran son las del usuario (`/Users/...`, `/Applications/...`).
  - Lo de Data que no tiene firmlink que lo exponga, más VM, Preboot e instantáneas, queda en
    «Sin analizar».
- **Una carpeta que no es `/`:** no se cruza a otro dispositivo.
- **Espacio del volumen:** `statfs`, tal cual. En APFS es el del contenedor. «Disco entero» es
  `/` o la raíz de `/Volumes/X`.
- **Discos del desplegable:** `/` (con el nombre del volumen) y cada volumen visible de
  `/Volumes`, con su sistema de archivos y su tamaño.
- **Permisos de macOS:**
  - los pedidos de Escritorio, Documentos y Descargas salen a nombre de xdir, que ya tiene esos
    textos en su Info.plist; el escaneo espera la respuesta;
  - las carpetas que fallan con `EPERM` o `EACCES` se cuentan;
  - si hubo alguna, la barra de datos muestra «N carpetas sin permiso: dale a xdir Acceso total
    al disco (tecla A)», y la tecla A abre Ajustes > Privacidad > Acceso total al disco. Es el
    lugar donde Windows muestra «ejecutá como administrador».
- **Windows, tecla A: ayudante elevado** (decisión del usuario).
  - xdir **no** se relanza ni se eleva. Lanza con `runas` una copia de sí mismo en modo ayudante,
    por ejemplo `xdir.exe --leer-mft C: <archivo>`.
  - El ayudante no abre ventana: lee la MFT con el `mft.rs` de hoy, escribe el árbol en un
    archivo temporal y termina.
  - El xdir abierto espera en segundo plano, lee el archivo, lo borra y muestra el resultado como
    si hubiera escaneado él. Mientras, la vista muestra la fase de lectura de la MFT; el progreso
    fino del ayudante es opcional.
  - Si el usuario cancela el pedido de Windows, o el ayudante falla, se queda el resultado del
    recorrido normal y el aviso sigue visible.
  - `startup_args` tiene que reconocer el modo ayudante antes que cualquier otra cosa: no carga
    sesión ni configuración y no abre ventana.
  - El formato del archivo es interno, sin compatibilidad entre versiones, y lo decide quien
    implementa. El nombre del argumento también.

### Tests

- Tests unitarios del núcleo que corren en los dos sistemas: `squarify`, `layout`, `rows_of`,
  `node_path`, `fmt_size`, colores, y el escáner con un listado falso (árbol, totales,
  agrupado, enlaces duros, cancelación).
- Escáner de Mac sobre una carpeta temporal armada en el test, con archivos, subcarpetas, un
  enlace duro y un enlace simbólico.
- Tests de `App`: el botón abre y cierra la vista y no responde a clics de los paneles con la
  vista abierta; «Abrir en el panel» navega el panel activo.
- Se mantienen los `mft::tests`.
- Ida y vuelta del archivo del ayudante: escribir un árbol y leerlo da el mismo árbol. Corre en
  los dos sistemas.

## Diseño en FigCraft

Sigue la forma de cromos (`cromos/docs/DISENO.md`), adaptada a una app de escritorio en Rust:

- `design/xdir.figcraft` es la fuente; se edita con `figcraft-cli` (`run` y `render`). En
  `design/` no van imágenes; los renders se hacen fuera del repo.
- `docs/DISENO.md` explica qué hay en el archivo, cómo se edita y cómo se mantiene sincronizado.
- Páginas:
  - **Tokens:** los colores de `theme.rs` (claro y oscuro, y los 12 acentos), los colores del
    mapa (rueda y categorías), medidas y textos;
  - **Componentes:** piezas de xdir que usa la vista (barra de estado, botones, píldoras) y las
    de xtree (fila de tabla, bloque de carpeta y de archivo, tarjeta, desplegable de discos,
    barra de datos del disco, leyenda);
  - **Pantallas · xtree:** escaneando (mapa en vivo, con la fase MFT en Windows), análisis
    completo (color por carpeta), color por tipo con leyenda, menú de discos abierto, aviso de
    permisos (Mac) y de administrador (Windows), en tema oscuro y claro.
- Los componentes llevan el nombre del módulo o la función de Rust que los dibuja. No hay
  «Connect to code», que es de React; si hace falta, se anota a mano qué función dibuja cada
  componente.
- Este trabajo abre `docs/`, que hoy no existe. Por ahora va solo `DISENO.md`; ONBOARDING,
  DECISIONS y CHANGELOG quedan para cuando el usuario los pida.

## Orden

1. Pantallas en FigCraft y `docs/DISENO.md`. **El usuario las revisa y aprueba.**
2. Núcleo neutro de xtree (`src/xtree/`) con sus tests, moviendo el código de `win.rs`.
3. Escáner de Windows en `fs/win32` (listado y MFT), la vista dentro de xdir en Windows y el
   ayudante elevado. Se borra el binario aparte.
4. Escáner de Mac y discos; la vista en Mac.
5. Empaquetado (Windows sin `xtree.exe`), lista de pruebas para Runick93 y PR a develop.

## Lo que queda a criterio de quien implementa

- Nombres exactos, y si los escáneres de cada sistema van en `fs/*` o en un submódulo propio.
- Cantidad de hilos del escáner en Mac.
- Ajuste fino de medidas y fuentes para que se vea bien en los dos sistemas.
- Cómo se reutiliza el diálogo de carpeta y el portapapeles que ya existen.

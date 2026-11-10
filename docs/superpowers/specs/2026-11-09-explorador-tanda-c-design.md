# Tanda C: vista previa, detalles, vista rápida y Galería — diseño

## Intención

Poder ver qué es un archivo sin abrirlo, con lo mejor del Explorador (panel de vista previa y de detalles) y del Finder (vista rápida con la barra espaciadora, vista Galería). Todo para Windows, macOS y Linux (Linux sin interfaz por ahora: el núcleo y los servicios quedan listos).

## Objetivos

1. **Columna de vista previa** a la derecha de la ventana, con la vista previa arriba y los detalles abajo (como el Finder).
2. **Vista rápida** con la barra espaciadora: el archivo "de verdad" (video que se reproduce, PDF con todas sus páginas, documentos de Office) en una ventana encima de xdir.
3. **Vista Galería**: una vista más del panel con una imagen grande y una tira de miniaturas.

## No objetivos

- Editar desde la vista previa (marcado, recortar, girar: eso es la tanda G).
- Reproducir video o recorrer PDF dentro de la columna o de la Galería (solo en la vista rápida).
- Vista previa de archivos dentro de zip (tanda D).
- Interfaz de Linux (al final del plan).

## Anti-objetivos

- Que mover la selección con las flechas se trabe esperando una vista previa (las vistas previas llegan solas, en otro hilo, y una selección vieja se descarta).
- Que la vista previa descargue archivos que están solo en la nube (iCloud, OneDrive, Dropbox): se muestra el ícono y los datos que no requieren descarga.
- Que un visor del sistema que se cuelga o falla tire abajo xdir.
- Dos paneles de detalles distintos (la Galería no trae el suyo: usa la columna de vista previa).

## Enfoques considerados

- **A. Imagen fija en todos lados** (servicio de miniaturas en tamaño grande + texto dibujado por xdir). Simple y parejo en las 3 plataformas, pero un video es un cuadro fijo y un PDF solo su primera página.
- **B. Visores del sistema en todos lados** (preview handlers de Windows, QuickLook de macOS). Lo más completo, pero ventanas nativas incrustadas en la columna y la Galería: mucho código por plataforma y casos raros de foco y tamaño; en Linux no hay equivalente.
- **C. Mezcla (elegido por el usuario).** Columna y Galería con imagen fija (A): rápidas, siguen la selección con las flechas. La vista rápida con el visor del sistema (B): es donde se espera ver el archivo completo, igual que en el Finder.

## Diseño

### 1. Columna de vista previa

```
┌árbol┐┌─ panel 1 ──────┐┌─ panel 2 ──────┐┌─ vista previa ─┐
│     ││ foto.jpg  ◀sel ││                ││   [ imagen ]   │
│     ││ doc.pdf        ││                ││ foto.jpg       │
│     ││                ││                ││ JPEG · 491 KB  │
│     ││                ││                ││ 4032 × 3024    │
│     ││                ││                ││ Modificado …   │
└─────┘└────────────────┘└────────────────┘└────────────────┘
```

- Una sola columna a la derecha de la ventana (después de los paneles), que muestra lo seleccionado en el **panel activo**; al cambiar de panel activo, cambia.
- Se prende y apaga con un botón en la barra de estado (junto a los de vistas, con cartel de ayuda), con un comando de la paleta («Mostrar u ocultar la vista previa») y con el atajo **Alt+P** en Windows y Linux y **⌥⌘P** en macOS (⇧⌘P ya es la paleta de comandos). El ancho se cambia arrastrando el borde (mínimo y máximo razonables; los paneles nunca quedan más angostos que su mínimo). Prendida y ancho se guardan en la sesión.
- **Arriba, la vista previa** según el tipo:
  - Imágenes, video, PDF, documentos y lo que el sistema sepa previsualizar: la imagen del servicio de miniaturas pedida al tamaño de la columna (más grande que las miniaturas de la lista, en su propia caché).
  - Texto y código (extensiones conocidas o contenido que es texto UTF-8 válido): las primeras líneas en letra de ancho fijo, hasta 64 KB leídos, sin ajustar líneas largas (se cortan).
  - Carpetas: el ícono grande y la cantidad de elementos; si xtree ya la escaneó, su tamaño.
  - Sin vista previa o solo en la nube: el ícono grande del tipo.
  - Mientras llega: el ícono del tipo; cuando llega, se reemplaza sin parpadeo.
- **Abajo, los detalles**: nombre (en varias líneas si hace falta), tipo, tamaño, fechas de creación, modificación y último acceso, y los metadatos de la tanda B (dimensiones, duración, autor, etiquetas, artista, álbum) que tengan valor. Con varios seleccionados: «N elementos seleccionados», el tamaño total y, si son todos del mismo tipo, el tipo.
- Con nada seleccionado: los detalles de la carpeta abierta.
- En «Este equipo»: los datos del disco enfocado (espacio libre y total).

### 2. Vista rápida (barra espaciadora)

- Con uno o más elementos seleccionados en la lista, **Espacio** abre una ventana grande encima de xdir con el elemento enfocado. **Espacio** o **Esc** la cierran. Con la vista rápida abierta, ↑/↓ y ←/→ pasan al anterior o siguiente (si había varios seleccionados, recorre solo esos; si no, la carpeta), y la selección de la lista acompaña.
- Excepciones: si se está tecleando el nombre de un archivo para saltar a él (búsqueda por tipeo en curso), Espacio sigue siendo parte del nombre. Sobre el encabezado de un grupo, Espacio sigue plegándolo. Ctrl+Espacio sigue como hoy.
- **macOS**: el panel de QuickLook del sistema (`QLPreviewPanel`): video que se reproduce, PDF con todas sus páginas, Office, etc.
- **Windows**: una ventana propia de xdir que aloja el visor del panel de vista previa del Explorador (`IPreviewHandler`) para el tipo; si el tipo no tiene visor (por ejemplo video), muestra la imagen grande del servicio de miniaturas y un botón «Abrir» que lo abre con su programa. El visor corre de forma que si falla o se cuelga, la ventana muestra la imagen grande en su lugar y xdir sigue respondiendo.
- **Linux** (cuando tenga interfaz): la imagen grande, como la columna.
- Archivos solo en la nube: no se descargan; se muestra el ícono con la leyenda «Disponible solo en línea».
- También desde el menú del clic derecho de un archivo («Vista rápida») y la paleta.

### 3. Vista Galería

```
┌─────────────────────────────────────────┐
│                                         │
│           [ imagen grande ]             │
│                                         │
├─────────────────────────────────────────┤
│ ▢ ▢ [▣] ▢ ▢ ▢ ▢ ▢ ▢  tira de miniaturas │
└─────────────────────────────────────────┘
```

- Una vista más del panel (`View::Gallery`), en el menú «Ver», en el ciclo de vistas (después de Miniaturas), con Ctrl+Shift+7 en Windows/Linux y ⌥⌘7 en macOS, y guardada por carpeta como las demás.
- Arriba, la imagen grande del elemento enfocado (misma fuente que la columna de vista previa); abajo, una tira horizontal de miniaturas con el enfocado resaltado y centrado cuando se pueda.
- ←/→ (y ↑/↓) pasan de uno a otro; clic en la tira lo enfoca; doble clic o Enter lo abre (carpetas: entra). Selección múltiple con Ctrl/⇧ en la tira como en la lista. Casillas de selección (tanda B) también en la tira si están prendidas.
- Carpetas y archivos sin vista previa: el ícono grande.
- Agrupar y filtrar siguen funcionando (la tira muestra los elementos visibles en el orden de la lista; los encabezados de grupo no se dibujan en la tira).
- Los detalles se ven en la columna de vista previa si está prendida (no hay un panel propio).

## Restricciones

- `xdir-core` sin dependencias ni `cfg` de sistema en producción; lo de cada sistema detrás de servicios en `xdir_core::sys` (con su versión falsa para los tests).
- Las lecturas (imagen grande, texto, carpeta) en otro hilo, con descarte de pedidos viejos (mismo patrón de generación que los metadatos), sin trabar la interfaz.
- Respetar archivos solo en la nube (mismas comprobaciones que los metadatos: `SF_DATALESS` en macOS, atributos `OFFLINE`/`RECALL_*` en Windows).
- Castellano en toda la interfaz; atajos documentados en la wiki al terminar.
- Antes de programar, las pantallas en FigCraft (`docs/design/xdir.figcraft`): columna de vista previa (imagen, texto, carpeta, varios seleccionados), botón de la barra de estado, vista rápida en Windows con visor y sin visor, y Galería; en oscuro y claro.
- Tests con `install_fakes()`, `FakeHost`, `RecordingPainter`; `cargo test` y `cargo check --all-targets` limpios en los 3 sistemas; prueba de humo en Mac.
- PRs propios contra `develop` a medida que se terminan las partes; versión de Mac instalada al terminar la tanda.

## Lo que queda para quien construya

- Medidas exactas (ancho inicial y límites de la columna, alto de la tira de la Galería, tamaño de la ventana de vista rápida): las define el diseño en FigCraft.
- La lista de extensiones de texto y la detección de "es texto".
- Orden de las partes en el plan (sugerido: columna de vista previa → Galería → vista rápida macOS → vista rápida Windows).

## Decisiones tomadas al revisar el diseño (llamadas de Claude, corregibles)

1. **Aislamiento del visor en Windows**: el `IPreviewHandler` se crea fuera de proceso (`CLSCTX_LOCAL_SERVER`), como hace el Explorador: Windows lo corre en su propio proceso sustituto (`prevhost.exe`), así que si el visor falla o se cuelga no tira abajo xdir. Los handlers que piden correr dentro del proceso (`DisableProcessIsolation`) se tratan como «sin visor» (imagen grande). Sin ejecutables nuevos ni cambios en el instalador. Si el visor no responde en un tiempo razonable, se muestra la imagen grande.
2. **Teclado en la vista rápida**: la rueda del mouse y el clic van al visor (para recorrer un PDF o un documento); Espacio, Esc y las flechas los maneja xdir. En Windows el visor le devuelve esas teclas a xdir por `IPreviewHandlerFrame::TranslateAccelerator`; en macOS, `QLPreviewPanel` ya delega en su data source/delegate la navegación. La ventana de vista rápida es una ventana normal de xdir (sin borde de sistema propio), dueña de la principal, que se cierra con ella.
3. **Índice de la Galería**: `View::Gallery` se agrega **al final** de `VIEWS` (índice 6, para no mover los guardados en sesión y `carpetas.toml`) y va después de Miniaturas solo en `CYCLE`. Atajos: Ctrl+Shift+7 en Windows/Linux y ⌥⌘7 en macOS, siguiendo a las otras vistas, que en Mac ya usan ⌥⌘1–6.
4. **Solo en la nube**: en macOS los elementos con `SF_DATALESS` no se pasan a `QLPreviewPanel`; xdir muestra su propia tarjeta «Disponible solo en línea». En Windows, para archivos `OFFLINE`/`RECALL_*` la imagen grande se pide solo de la caché (`SIIGBF_INCACHEONLY`) y si no está, el ícono.
5. **Imagen grande**: extiende el servicio de miniaturas existente con tamaños grandes; caché en memoria con tope (LRU por bytes, ~64 MB), separada de la de la lista. El tamaño pedido se redondea a escalones (256, 512, 1024, 2048 px físicos) para que redimensionar la columna o cambiar de DPI no pida una imagen por cada paso; mientras tanto se escala la que hay.
6. **Recorrido de la vista rápida**: sigue el orden visible de la lista (agrupada y filtrada), salteando encabezados. Si una actualización de la carpeta borra el elemento abierto, pasa al siguiente; si no queda ninguno, se cierra.
7. **Qué muestra la columna**: siempre lo del último panel activo, aunque el foco esté en el árbol. Los resultados de búsqueda se comportan como cualquier carpeta. Con la vista de xtree abierta (pantalla completa), la columna no se ve.
8. **Texto ilegible**: un corte de UTF-8 en el límite de 64 KB cuenta como texto válido. Archivos sin permiso o bloqueados: ícono del tipo y, en los detalles, «No se puede leer este archivo».

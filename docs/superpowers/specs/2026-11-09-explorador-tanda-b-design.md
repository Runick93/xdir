# Funciones del Explorador, tanda B: organizar la lista — diseño

## Intención

Que la lista de archivos de xdir se pueda organizar como en el Explorador de Windows: agrupar, filtrar por
columna, ver más datos de cada archivo, vistas Lista y Contenido, casillas de selección, y que cada carpeta
recuerde cómo se la ve. Es parte del plan de 25 funciones del Explorador en tandas (A hecha; esta es la B;
después C paneles laterales, D operaciones, E árbol y red). Las funciones son para las 3 plataformas
(Windows, macOS y Linux): la lógica y el dibujo viven en `xdir-core`; lo que depende del sistema (leer
metadatos de archivos) se implementa en cada crate de plataforma; Linux, sin interfaz todavía, queda con
los servicios listos o vacíos donde no hay de dónde leer.

Quién lo usa: quien trabaja con muchas carpetas a la vez (el público de xdir) y viene del Explorador, que
espera que Descargas quede agrupada por fecha y Fotos en miniaturas sin tener que rehacerlo cada vez.

## Decisiones del usuario

- Funciones generales para las 3 plataformas.
- **Memoria por carpeta, como el Explorador**: cada carpeta recuerda su vista, orden, agrupación, columnas
  (y anchos) y filtros.
- Las 5 funciones de la tanda tal como se propusieron (agrupar con los grupos del Explorador, filtros por
  columna con casillas, más columnas leídas en segundo plano, vistas Lista y Contenido, casillas opcionales).
- Antes de programar se diseñan las pantallas en FigCraft (claro y oscuro), en `docs/design/xdir.figcraft`.

## Objetivos

1. **Memoria por carpeta.** Cada carpeta recuerda vista, orden (columna y sentido), agrupación (y grupos
   plegados), columnas visibles con sus anchos y filtros. Las carpetas nunca tocadas usan la **vista por
   defecto**. Comando «Usar esta vista en todas las carpetas»: copia la configuración de la carpeta actual a
   la vista por defecto y borra las de las demás carpetas (el «Aplicar a las carpetas» del Explorador, con
   confirmación «¿Usar esta vista en todas las carpetas? Se pierden las vistas guardadas de cada
   carpeta.»). Comando «Restablecer la vista de esta carpeta» (vuelve a la por defecto). Las ubicaciones
   especiales (Este equipo, cada búsqueda — por carpeta buscada —, Papelera y demás ubicaciones del Shell)
   recuerdan la suya con su propia clave.
2. **Agrupar por** Nombre, Fecha de modificación, Fecha de creación, Tipo, Tamaño o cualquier columna
   nueva (punto 3), o «(Ninguno)». Encabezados de grupo con el nombre y la cantidad («Hoy (3)»), plegables
   con clic o con ←/→ cuando el foco está en el encabezado. Grupos:
   - Nombre: «0–9», «A–H», «I–P», «Q–Z», «Otros» (por la primera letra sin acento, sin distinguir
     mayúsculas).
   - Fechas: «Hoy», «Ayer», «Antes en esta semana», «La semana pasada», «Antes en este mes», «El mes
     pasado», «Antes en este año», «Hace mucho», y «Sin fecha» (fecha 0). Semana de lunes a domingo, en
     hora local.
   - Tipo: un grupo por `type_name`; las carpetas, en el grupo «Carpeta de archivos» / el tipo que ya da el
     sistema, primero.
   - Tamaño: «Vacío» (0 B), «Diminuto (0–16 KB)», «Pequeño (16 KB–1 MB)», «Mediano (1–128 MB)», «Grande
     (128 MB–1 GB)», «Enorme (1–4 GB)», «Gigantesco (> 4 GB)»; las carpetas en «Carpetas».
   - Columnas nuevas: por valor (Autor, Artista, Álbum, Etiquetas — un elemento con varias etiquetas va
     en cada una), dimensiones por tamaño de imagen («Pequeña», «Mediana», «Grande», «Muy grande»),
     duración por rangos («Muy corta (< 1 min)», «Corta (1–5 min)», «Media (5–30 min)», «Larga (> 30
     min)»), y «Sin valor».
   - Orden de los grupos: el natural de cada criterio (fechas de la más reciente a la más vieja, tamaños
     de chico a grande, nombre alfabético); el sentido de orden de la columna lo invierte. Dentro de cada
     grupo, el orden de la lista de siempre.
   - Funciona en las vistas Detalles, Lista, Contenido, Íconos y Miniaturas (en las de grilla el
     encabezado ocupa una fila completa).
   - Menú «Agrupar por» en el clic derecho del fondo (junto a «Ordenar por» y «Vista») y comandos en la
     paleta («Agrupar por nombre», …, «No agrupar»). El clic derecho en el encabezado de columnas trae
     también «Ordenar por ▸» y «Agrupar por ▸» (los mismos submenús, sobre el panel del encabezado),
     entre las columnas y «Columnas de fábrica».
3. **Más columnas**: Dimensiones (ancho × alto en píxeles, imágenes y videos), Duración (h:mm:ss, audio y
   video), Autor, Etiquetas, Artista y Álbum. Se agregan desde el menú de columnas del encabezado (como las
   de hoy). Los valores se leen en segundo plano solo para las columnas visibles (o la que agrupa o
   filtra), en lotes, y la lista se repinta al llegar; mientras tanto la celda está vacía. Se guardan en
   memoria por ruta y fecha de modificación mientras la carpeta está abierta. Origen:
   - Windows: el sistema de propiedades del Shell (`IPropertyStore`), las mismas propiedades que muestra el
     Explorador (`System.Image.Dimensions`, `System.Media.Duration`, `System.Author`, `System.Keywords`,
     `System.Music.Artist`, `System.Music.AlbumTitle`).
   - macOS: Spotlight (`MDItem`: `kMDItemPixelWidth/Height`, `kMDItemDurationSeconds`, `kMDItemAuthors`,
     etiquetas de Finder `kMDItemUserTags`, `kMDItemAuthors`/`kMDItemAlbum` de audio).
   - Linux: sin origen por ahora (el servicio devuelve vacío).
4. **Filtros por columna.** Al pasar el mouse por el encabezado de una columna aparece una flecha a la
   derecha; clic abre un desplegable con una casilla por cada grupo de esa columna que tenga elementos en la
   carpeta (los mismos grupos del punto 2), con la cantidad. Tildar varios muestra los elementos de
   cualquiera de ellos; filtros en varias columnas se combinan (tiene que cumplir todos). La columna filtrada
   muestra un embudo en el encabezado; «Quitar el filtro» en el desplegable. La barra de estado indica
   «(filtrado)». Se combina con el filtro rápido de texto que ya existe. Los filtros se recuerdan por carpeta
   (punto 1) y se borran con «Quitar los filtros» (comando nuevo).
5. **Vistas nuevas:**
   - **Lista**: ícono de 16, solo el nombre, en columnas que se llenan de arriba abajo; el desplazamiento es
     horizontal (rueda vertical desplaza en horizontal, como el Explorador).
   - **Contenido**: una fila alta por elemento (ícono de 32 o miniatura), nombre en la primera línea y el
     tipo debajo; a la derecha, fecha de modificación arriba y tamaño debajo.
   - Se suman a «Vista» del menú y de la paleta, con atajos `Ctrl+Shift+5` (Lista) y `Ctrl+Shift+6`
     (Contenido) (en macOS con ⌘ según la tabla de atajos).
6. **Casillas de selección**: switch «Casillas de selección» en la rueda dentada, apagado por defecto
   (global, se guarda en la sesión). Prendido: cada elemento muestra una casilla a la izquierda del ícono,
   en todas las filas (vacía o marcada); en oscuro el borde de la vacía usa `text_secondary` para que se
   distinga (decisión del usuario; se aparta de la maqueta X36, que la mostraba solo con el mouse encima o
   seleccionado). Clic en la casilla suma o quita ese elemento de la
   selección sin tocar los demás (como Ctrl+clic). En Detalles, el encabezado tiene una casilla para
   seleccionar o quitar todo.

## No objetivos

- Pilas («Apilar por») y búsqueda guardada del Explorador.
- Editar metadatos (autor, etiquetas) desde xdir.
- Columnas de metadatos en Linux (no hay de dónde leerlas sin dependencias de escritorio).
- Compartir la vista por tipo de carpeta («Documentos», «Imágenes», «Música») como hace el Explorador: la
  memoria es por carpeta y hay una sola vista por defecto.
- Interfaz de Linux.

## Anti-objetivos (sería un fracaso aunque «funcione»)

- Que leer metadatos trabe la lista o la ventana, o que abrir una carpeta grande sea más lento cuando las
  columnas nuevas no están visibles.
- Que una carpeta pierda su vista guardada sin que el usuario lo pida, o que una vista corrupta en
  `carpetas.toml` impida abrir xdir.
- Que la memoria por carpeta crezca sin límite.
- Cambiar cómo se ven las carpetas hoy para quien no toca nada: la vista por defecto de fábrica es la de
  hoy (Detalles, ordenado por nombre, sin agrupar, columnas de siempre).

## Restricciones

- Las de la migración: `xdir-core` sin dependencias ni `cfg` de sistema en producción; App y lista
  probadas con `install_fakes()`, `FakeHost` y `RecordingPainter`; `cargo test` y `cargo check
  --all-targets` limpios en Windows, macOS y Linux; CI verde con la prueba de humo de Windows.
- Nada bloqueante en el hilo de interfaz: los metadatos se leen en un hilo aparte con el mismo patrón que
  la carga de carpetas (canal + `Waker`).
- La sesión y la configuración actuales siguen cargando (compatibilidad hacia atrás): la vista y las
  columnas que hoy guarda la sesión pasan a ser la vista por defecto la primera vez.
- Castellano; Conventional Commits sin atribución a Claude; PRs contra `develop`; al mergear, versión de
  Mac instalada en `/Applications` para que el usuario pruebe.

## Enfoques considerados

**Dónde se guarda la memoria por carpeta.**
- A. **Archivo propio `carpetas.toml`** en la carpeta de configuración (con el parser `toml` del núcleo),
  con una entrada por carpeta y un límite de 2000 carpetas (se descartan las usadas hace más tiempo).
  Elegido: no mezcla con `sesion.toml` (que se reescribe seguido), se puede borrar para «resetear» y no
  depende del sistema.
- B. Dentro de `sesion.toml`: más simple, pero la sesión se reescribe al cerrar y con miles de carpetas
  crece y se vuelve frágil.
- C. Como el Explorador, en el sistema (registro de Windows / atributos extendidos): depende del sistema,
  no funciona en carpetas de solo lectura ni en Linux. Descartado.

**Cómo se leen los metadatos.**
- A. **Servicio nuevo `Metadata` en `xdir_core::sys`** (trait `Send + Sync`, como `FileSystem`), con
  `fn request(&self, req: MetadataRequest)` que lee en un hilo y devuelve por el canal de la pestaña
  (`LoadMsg::Metadata`), igual que las superposiciones de íconos (`Overlays`). Elegido: sigue el patrón
  existente y no traba la interfaz.
- B. Leerlos en `spawn_load` junto con la carga: más simple pero hace más lenta toda carga aunque las
  columnas no se vean (viola un anti-objetivo).

**Dónde viven los grupos y filtros.**
- **Funciones puras en el núcleo** que dado un `Entry` (y sus metadatos) devuelven la clave de grupo de una
  columna (`group_key(col, entry, now) -> GroupKey`); agrupar, el desplegable de filtros y el orden de los
  grupos usan la misma función. Única forma razonable: una sola fuente para grupos y filtros.

## Lo que queda a quien implementa

- Formato exacto de `carpetas.toml` (clave = ruta normalizada con `same_path`; para ubicaciones especiales
  una clave textual), nombres internos, cuándo se guarda (al cambiar algo, con escritura diferida).
- Layout fino de Lista y Contenido, del desplegable de filtros y de las casillas: lo fijan las pantallas de
  FigCraft que se diseñan antes.
- Tamaño de los lotes de metadatos y el límite de la caché en memoria.
- Detalles de propiedades por plataforma (qué API exacta de Spotlight/IPropertyStore).

## Decisiones tomadas por quien diseña (después del chequeo del constructor)

- **Pantallas primero.** Las pantallas de FigCraft (X32–X37, claro y oscuro) se hacen antes del plan y
  fijan el layout de Lista, Contenido, encabezados de grupo, desplegable de filtros y casillas. Ninguna
  tarea de interfaz arranca antes de que el usuario las vea.
- **Primera vez y modelo de datos.** «Primera vez» = no existe `carpetas.toml`: la vista y columnas de la
  sesión de hoy se copian como vista por defecto. `sesion.toml` sigue leyendo esos campos (compatibilidad)
  pero deja de usarlos después de la migración. Cada cambio que se hace en una pestaña (vista, orden,
  columnas, anchos, agrupación, grupos plegados, filtros) se guarda en la entrada de **la carpeta actual**
  (no hay ajuste propio de la pestaña); al navegar, la lista toma la entrada de la carpeta nueva o la vista
  por defecto. Escritura diferida (un segundo después del último cambio y al cerrar).
- **«Usar esta vista en todas las carpetas»** copia vista, orden, agrupación (sin los grupos plegados) y
  columnas con anchos a la vista por defecto, y borra todas las entradas de carpetas. No copia filtros. Las
  casillas son un ajuste global aparte.
- **Etiquetas múltiples.** Para agrupar, un elemento con varias etiquetas va en el grupo de **la primera
  etiqueta** en orden alfabético (una sola fila por elemento; la selección y la cuenta no cambian). Para
  filtrar, coincide si **cualquiera** de sus etiquetas está tildada.
- **Grupos plegados.** Sus elementos no se ven: el teclado los saltea, el rango con Shift no los incluye y
  Ctrl+A selecciona solo lo visible. Dentro de cada grupo, carpetas primero como hoy.
- **Columnas de metadatos sin leer.** Cuando una columna de metadatos se usa para agrupar o filtrar, se
  piden los metadatos de **toda la carpeta** (no solo lo visible). Mientras llegan, los elementos sin dato
  van a un grupo «Leyendo…» al final y el desplegable de filtros muestra las cuentas parciales con la nota
  «Leyendo datos…», y se actualiza al llegar.
- **Orden de aplicación:** filtros por columna → filtro rápido de texto → orden → grupos. El filtro rápido
  sigue siendo de la pestaña (no se guarda por carpeta). «(filtrado)» en la barra de estado solo por filtros
  de columna.
- **Claves de ubicaciones especiales:** carpeta de disco → ruta normalizada; «Este equipo» → `equipo`;
  ubicaciones del Shell (Papelera, etc.) → `shell:<nombre de análisis>`; resultados de búsqueda →
  `busqueda:<ruta de la carpeta buscada>` (los resultados de buscar en Descargas tienen su propia vista,
  distinta de la de Descargas).
- **Pruebas.** El núcleo se prueba con falsos (`FakeMetadata` en `sys::testing`). Las lecturas reales de
  Windows (`IPropertyStore`) y macOS (Spotlight) llevan tests con archivos de ejemplo cuando el sistema lo
  permite sin indexación (en macOS Spotlight puede no tener el archivo indexado en una carpeta temporal: el
  test se salta con aviso si no hay dato) y se verifican a mano: lista para Runick93 en el PR y versión de
  Mac instalada para el usuario.

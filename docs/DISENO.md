# Diseño: el archivo de FigCraft de xdir

Todo vive en `docs/design/xdir.figcraft` (un solo archivo; no se guardan imágenes: los renders se hacen fuera del repo).
Sigue la forma del de Cromos: **cada página corresponde a una parte del código** y cada componente lleva
el nombre de la función de Rust que lo dibuja.

## Qué hay

| Página de FigCraft | Módulo | Qué hay |
|---|---|---|
| **Tokens** | `crates/xdir-core/src/theme.rs`, `crates/xdir-core/src/xtree/colors.rs` | Colecciones de variables: **Tema** (modos Oscuro y Claro: background, surface, surface_alt, border, text, text_dim, text_secondary, accent, focus_fill, hover, selection, scrollbar, advertencia (ámbar #FAC74D en oscuro, #B7791F en claro), carpeta (amarillo fijo), ok/peligro y tres neutros del mapa), **Acentos** (los 12 `PRESETS`), **Mapa** (`WHEEL` y `CATS`) y **Medidas** (constantes de `crates/xdir-core/src/xtree/view.rs` y `crates/xdir-core/src/ui.rs`). Estilos de texto `Texto/ui`, `ui_bold`, `title`, `small`, `small_bold`: los `FontId::{Ui, UiBold, Title, Small, SmallBold}` de `crates/xdir-core/src/paint.rs`. |
| **Componentes** | `crates/xdir-core/src/ui.rs` y la vista de xtree | De xtree: `xtree::view::top_bar`, `table_row`, `folder_block`, `file_block`, `card`, `drive_menu`, `disk_bar` y `legend`. De xdir: `ui::draw_status` (14 variantes: xtree encendido o apagado y los 5 layouts, y con 2 paneles el indicador de xtree en progreso, giro, terminado y aviso), `ui::draw_group_bar` (píldora de grupo en 5 estados y botón «+»), `pane::Pane::draw_tabs`, `pane::Pane::draw_search`, `pane::Pane::draw_favs`, `list::FileList::draw` (fila en 6 estados y encabezado), `list::FileList::draw_cell` (cuadrícula), `tree::Tree::draw` (fijado, colapsado y fijado con la entrada «xtree»), `palette::Palette::draw` (comandos, carpetas, nombre de grupo), `typed::Suggest::draw`, `about::draw`, `ui::draw_picker` y `ui::draw_terminal_menu`. De la tanda B (sección «Tanda B: organizar la lista»): `list::FileList::draw_group_header` (encabezado de grupo expandido, plegado y con foco), `list::FileList::draw_checkbox` (casilla vacía, marcada y parcial), `list::FileList::draw_filter_menu` (desplegable del filtro de columna), `list::FileList::draw_content_row` (fila de la vista Contenido en 5 variantes) y `list::FileList::draw_list_cell` (celda de la vista Lista en 5 variantes). De la sección «Estado de xtree y cartel de ayuda»: `ui::draw_xtree_badge` (indicador de 14×14: progreso con anillo al 37 %, giro, terminado en `ok` y aviso en `warning`/advertencia) y `app::tooltip` (cartel de ayuda: `surface_alt`, borde `border`, radio 4, texto Small, 6 de margen, alto 26). De la tanda C (sección «Tanda C: vista previa»): `ui::draw_preview_toggle` (botón de la columna de vista previa: apagado, bajo el mouse y encendido; cartel «Vista previa (Alt+P)» / «Vista previa (⇧⌘P)»), tres variantes más de `ui::draw_status` con ese botón (`Vista previa=Encendida/Apagada`), `preview::draw` (la columna en 6 variantes: imagen, texto, carpeta, varios seleccionados, sin vista previa o solo en línea y cargando), `preview::draw_details` (el bloque de detalles en las mismas 6), `list::FileList::draw_strip_cell` (celda de la tira de la Galería: normal, bajo el mouse, seleccionada, seleccionada con foco y con casilla) y `quicklook::draw_title_bar` (barra de título de la vista rápida de Windows). Las variantes (estado, tipo, sistema) son propiedades del componente. |
| **Pantallas · xtree** | la vista de xtree dentro de xdir | 9 pantallas de 1400×820, cada una en tema oscuro (izquierda) y claro (derecha): T01 escaneando con mapa en vivo, T02 fase MFT con %, T03 completo (color por carpeta), T04 color por tipo con leyenda, T05 menú de discos abierto, T06 aviso de carpetas sin permiso (macOS), T07 aviso de administrador (Windows), T08 cronómetro y porcentaje («Escaneando… 37 % · 0:42» y barra de avance de 3 px en acento bajo la barra superior), T09 «Análisis completado en 1:23». |
| **Pantallas · xdir** | `crates/xdir-core/src/app/paint.rs` y lo que dibuja | 31 pantallas X01–X31 de la ventana principal que xdir ya tiene hoy, 6 pantallas X32–X37 de la tanda B (todavía sin programar), la X38 del estado de xtree y las X39–X44 de la tanda C (vista previa, Galería y vista rápida; sin programar), en oscuro y claro (ver los inventarios). |

Las pantallas usan **instancias** de los componentes (barra de estado, píldoras de grupo, filas y celdas de la lista, favoritos, árbol, paleta, desplegable de rutas, acerca de, panel de acento, menú de consola; en xtree, barra superior, filas, tarjeta, menú, barra de datos, leyenda). Los bloques del mapa se dibujan a medida con la misma receta que
`folder_block` y `file_block`; el marco del panel (pestañas, barra de dirección con migas, caja de búsqueda, pie) y las filas con columnas distintas de las de fábrica también se dibujan a medida con la receta de `pane::Pane::draw` y `list::FileList::draw`, porque dependen del ancho. El tema se elige con el modo de la variable en el marco de cada pantalla.

## Lo que muestran las pantallas

- La vista ocupa el área de los paneles (incluida la franja de grupos). La barra de estado de xdir queda
  abajo con el botón «xtree» encendido.
- Los colores del cromo (fondo, paneles, bordes, texto, botones, selección) son los del `Theme` de xdir;
  el mapa conserva su rueda y sus categorías. Los neutros del mapa por tipo y de las teselas de archivos
  chicos se derivan del tema (`mapa_carpeta_n`, `mapa_chico`).
- Botón 0 de la tarjeta: «Abrir en el panel» (antes «Mostrar en xdir»).
- En la barra de datos del aviso de macOS se achican las columnas para que entre el texto.

## Inventario de pantallas de xdir (X01–X31)

Leído de `crates/xdir-core/src/app/paint.rs` (orden de dibujo), `ui.rs`, `pane.rs`, `list.rs`, `tree.rs`, `palette.rs`, `typed.rs`,
`about.rs` y `app/*`. Cada una se dibuja en oscuro y claro. Rutas de Windows salvo la X31.

| N.º | Pantalla | Qué muestra | Qué se hace | Rust |
|---|---|---|---|---|
| X01 | Ventana principal, 2 paneles lado a lado | Panel activo con borde de acento, pestaña, botones de navegación, migas, estrella llena, consola, caja de búsqueda, barra de favoritos, lista en detalles con selección, foco y fila bajo el mouse, pie con botones; el otro panel con la selección apagada | Clic, doble clic, teclas, arrastrar | `app/paint.rs::paint`, `pane::Pane::draw`, `list::FileList::draw` |
| X02 | Layout de 1 panel | Sin botón de maximizar; dos pestañas | Cambiar de layout | `ui::draw_status`, `ui::pane_rects` |
| X03 | 2 paneles apilados | Dos paneles uno arriba del otro | Arrastrar el divisor | `ui::pane_rects` |
| X04 | 3 paneles, divisor resaltado | Uno a la izquierda y dos apilados; el divisor vertical resaltado en acento | Arrastrar el divisor | `ui::draw_splitter` |
| X05 | 4 paneles | Cuadrícula de 2×2; uno muestra «Este equipo» | — | `ui::pane_rects` |
| X06 | Panel maximizado | Un panel ocupa todo; botón de restaurar encendido | Maximizar y restaurar (Ctrl+M) | `pane::Pane::draw_maximize` |
| X07 | Pestañas y arrastre | Varias pestañas, hover con ×, pestaña arrastrada flotando con sombra, panel destino con borde y marca de inserción | Cerrar, reordenar, mover entre paneles | `pane::Pane::draw_tabs`, `draw_ghost`, `draw_tab_drop` |
| X08 | Barra de dirección en edición | Cuadro con acento, texto escrito, cruz para borrar y desplegable de rutas | Escribir, elegir, Enter | `pane::Pane::draw`, `typed::Suggest::draw` |
| X09 | Íconos medianos | Cuadrícula de celdas de 48 con el nombre debajo | Ctrl+rueda o botón del pie | `list::FileList::draw_cell` |
| X10 | Íconos grandes | Celdas de 96 | ídem | `list::FileList::draw_cell` |
| X11 | Miniaturas | Celdas de 160 con la imagen | ídem | `list::FileList::draw_cell` |
| X12 | Detalles con más columnas | Fecha de creación, extensión y atributos; orden por tamaño descendente con flecha; columna arrastrada resaltada | Clic en el encabezado, arrastrar y cambiar ancho | `list::FileList::draw` |
| X13 | Búsqueda en subcarpetas en curso | Caja con lupa de acento, indicador girando y %; columna «Carpeta»; estado «Buscando…» | Escribir, cruz para terminar | `pane::Pane::draw_search`, `tab::Tab::status` |
| X14 | Búsqueda terminada | Resultados, columna «Carpeta» y resumen con la duración | ídem | `tab::Tab::status` |
| X15 | Árbol lateral fijado | Panel de 240, chincheta de acento, Inicio, OneDrive, carpetas del usuario, Acceso rápido, Este equipo, Red y Linux | Expandir, abrir, anclar | `tree::Tree::draw` |
| X16 | Árbol colapsado | Franja de 48 con solo íconos | Pasar el mouse expande | `tree::Tree::draw` |
| X17 | Árbol suelto expandido | Árbol de 240 encima de los paneles, con sombra | — | `app/paint.rs::paint`, `tree::Tree::draw` |
| X18 | Barra de grupos | Píldoras (activa, bajo el mouse, sin guardar con ×) y botón «+» | Cambiar, cerrar, crear | `ui::draw_group_bar` |
| X19 | Rueda dentada | Panel de color de acento: 12 colores, color de Windows, personalizado, switches y «Acerca de xdir» | Elegir acento, tema y barras | `ui::draw_picker` |
| X20 | Menú de consola | Botón `>_` encendido y menú (PowerShell, CMD) | Abrir una consola en la carpeta | `ui::draw_terminal_menu` |
| X21 | Paleta de comandos | Campo, comandos con atajo, ayuda y «1–10 de N» | Ctrl+Shift+P | `palette::Palette::draw` |
| X22 | Ir a una carpeta | Favoritos, grupos, historial y «Este equipo» | Ctrl+P | `palette::Palette::draw` |
| X23 | Guardar paneles como grupo | «Guardar como…» y grupos para reemplazar | Nombre y Enter | `palette::Palette::draw` |
| X24 | Favoritos arrastrando | Indicador de inserción, carpeta de favoritos de destino, hover y desborde «»» | Arrastrar carpetas | `pane::Pane::draw_favs` |
| X25 | Este equipo | Carpetas del usuario y unidades con su capacidad; búsqueda y consola deshabilitadas | Abrir una unidad | `list::FileList::draw` |
| X26 | Acerca de | Velo, franja del logo, versión y «Reportar un problema» | Clic fuera o Esc cierra | `about::draw` |
| X27 | Error al abrir | Lista vacía y «No se pudo abrir: …» en la barra de estado | Reintentar | `tab::Tab::status` |
| X28 | Carpeta cargando | «N elementos · cargando…» | — | `tab::Tab::status` |
| X29 | Aviso (notice) | «Agregado a favoritos…» delante del estado | — | `app/paint.rs::paint` |
| X30 | Renombrar en línea | Control de edición sobre el nombre, sin la extensión seleccionada | F2, Enter, Esc | `app/edit.rs::begin_rename` |
| X31 | Variante macOS | Ventana principal con `/`, «Macintosh HD», fechas y tamaños de macOS | — | `fs::Location::crumbs`, `format.rs` |

## Inventario de pantallas de la tanda B (X32–X37)

Diseñadas antes de programar (spec `docs/superpowers/specs/2026-11-09-explorador-tanda-b-design.md`): fijan el layout de los
encabezados de grupo, el desplegable de filtros, las vistas Lista y Contenido y las casillas. Van debajo de X31, con la base
de X02 (un panel) y en oscuro y claro.

| N.º | Pantalla | Qué muestra | Qué se hace | Rust |
|---|---|---|---|---|
| X32 | Agrupar por fecha (Detalles) | Descargas agrupada por fecha de modificación: «Hoy (3)», «Ayer (2)», «La semana pasada (5)» expandidos y «Hace mucho (12)» plegado; chevron ▾/▸, nombre en `text_secondary` seminegrita y línea fina; fila seleccionada con foco y fila bajo el mouse | Clic en el encabezado o ←/→ para plegar | `list::FileList::draw_group_header`, `list::FileList::draw` |
| X33 | Filtro por columna | Mouse sobre «Tipo»: flecha a la derecha del encabezado y desplegable abierto con «Carpeta de archivos (4)», «Imagen JPEG (12)» ✓, «Imagen PNG (3)» ✓, «Documento PDF (2)», separador y «Quitar el filtro»; la lista muestra solo JPEG y PNG, embudo junto a «Tipo» y «(filtrado)» al final del estado | Tildar grupos, quitar el filtro | `list::FileList::draw_filter_menu`, `tab::Tab::status` |
| X34 | Vista Lista | Ícono de 16 y nombre en columnas de 240 que se llenan de arriba abajo; selección, foco, hover y barra de desplazamiento horizontal | Rueda desplaza en horizontal; Ctrl+Shift+5 | `list::FileList::draw_list_cell` |
| X35 | Vista Contenido | Filas de 56: ícono de 32 o miniatura, nombre y tipo (`text_dim`) a la izquierda; fecha de modificación y tamaño a la derecha; línea entre filas | Ctrl+Shift+6 | `list::FileList::draw_content_row` |
| X36 | Casillas de selección | Columna de casillas: la del encabezado parcial, 3 filas seleccionadas con la casilla marcada, la fila bajo el mouse con la casilla vacía y el resto sin casilla. **La app se aparta**: con el switch prendido todas las filas llevan casilla (vacía o marcada) y en oscuro el borde de la vacía es `text_secondary` (decisión del usuario) | Clic en la casilla suma o quita (como Ctrl+clic) | `list::FileList::draw_checkbox` |
| X37 | Más columnas, menú «Agrupar por» y casillas en la rueda dentada | Columnas Dimensiones («4032 × 3024»), Duración («0:03:21»), Autor y Etiquetas, lista agrupada por fecha; menú del fondo con «Vista ▸», «Ordenar por ▸» y «Agrupar por ▸» abierto (Nombre, Fecha de modificación ✓, Fecha de creación, Tipo, Tamaño, Dimensiones, Duración, «(Ninguno)»); rueda dentada con el switch nuevo «Casillas de selección» | Agrupar desde el menú; prender las casillas | `list::FileList::draw`, `ui::draw_picker` |

Notas de X32–X37:
- El menú contextual de X37 es nativo (ver abajo): se dibuja con aspecto aproximado, como los controles de X08; el submenú
  sube para entrar en la ventana como hace el del sistema.
- El panel de la rueda dentada de X37 es una copia desacoplada de `ui::draw_picker` con la fila nueva, para no cambiar el
  componente (y la X19, que muestra lo que hay hoy) hasta que se programe.
- Las filas de X36 y X37 se dibujan a medida (columnas distintas de las de fábrica), como en X12.

## Estado de xtree (X38, T08, T09)

Diseñadas antes de programar (spec `docs/superpowers/specs/2026-11-09-xtree-estado-design.md`).

| N.º | Pantalla | Qué muestra | Qué se hace | Rust |
|---|---|---|---|---|
| X38 | Estado de xtree en la barra y en la columna izquierda | Base X15 (árbol fijado y 2 paneles) con la vista de xtree cerrada y un volumen escaneándose: botón de la barra de estado con el anillo al 37 % en lugar del ícono del mapa, «37 %» y «xtree» (bajo el mouse), cartel «Escaneando / · 37 % · 0:42» arriba del botón; en el árbol, la entrada «xtree» debajo de «Inicio» (ícono del mapa, «xtree», «37 %» y el mismo anillo a la derecha) | Clic en el botón o en la entrada abre xtree sobre la carpeta del panel activo (el botón de la barra la cierra si está abierta; con la vista abierta el árbol queda tapado) | `ui::draw_xtree_badge`, `ui::draw_status`, `tree::Tree::draw`, `app::tooltip` |
| T08 | Cronómetro y porcentaje | Base T01 (volumen entero): estado «Escaneando… 37 % · 0:42» arriba a la derecha y barra de avance de 3 px en acento sobre `border` en el borde inferior de la barra superior; «706.371 archivos - 125,90 GB» analizados | — | `xtree::view` (`state_text`, barra de avance) |
| T09 | Análisis completado en 1:23 | Base T03: «Análisis completado en 1:23» con el tilde verde | — | `xtree::view` (`state_text`) |

Notas de X38, T08 y T09:
- Con el indicador en progreso el botón «xtree» se ensancha a la izquierda (de 74 a 105) para que entren el anillo, «37 %» y
  «xtree»; con giro, terminado o aviso el indicador ocupa el lugar del ícono del mapa y el botón conserva su ancho.
- El cartel sale arriba del botón porque debajo del mouse no entra (`tooltip_rect`: `y - alto - 4`).
- En T08 y T09 el punto y el texto del estado van en `r0 - STATE_W` (1172), como en el código; en T01–T07 siguen en 1240
  («Análisis completado en 1:23» no entraba ahí).
- T08 reusa el mapa y la tabla de T01: los tamaños de las carpetas no cuadran con los 125,90 GB analizados (son de muestra).
- En el árbol colapsado (franja de 48) la entrada «xtree» muestra solo el ícono del mapa, sin indicador.

## Tanda C: vista previa, Galería y vista rápida (X39–X44)

Diseñadas antes de programar (spec `docs/superpowers/specs/2026-11-09-explorador-tanda-c-design.md`). Van debajo de X38
(filas y = 34260…38760; marco de la página hasta 39800), en oscuro y claro.

| N.º | Pantalla | Qué muestra | Qué se hace | Rust |
|---|---|---|---|---|
| X39 | Columna de vista previa con una imagen | Base X16 (árbol colapsado) con 2 paneles de 501 y la columna de 320 a la derecha (x = 1074, mismo alto que el árbol). Panel activo «Viaje» con IMG_3101.jpg seleccionada con foco; la columna muestra la imagen (288×216) y los detalles: nombre, «Imagen JPEG», Tamaño 3,8 MB, Creado, Modificado, Último acceso, Dimensiones 4032 × 3024, Autor y Etiquetas en píldoras. Botón de la columna encendido en la barra de estado, bajo el mouse, con el cartel «Vista previa (Alt+P)» arriba | Botón, Alt+P / ⇧⌘P o el comando «Mostrar u ocultar la vista previa»; arrastrar el borde izquierdo de la columna cambia el ancho | `preview::draw`, `preview::draw_details`, `ui::draw_preview_toggle`, `ui::draw_status`, `ui::pane_rects` (área de paneles sin la columna), `app::tooltip` |
| X40 | Columna con un archivo de texto | El panel activo es el de la derecha (`src` de xdir-core, about.rs con foco): la columna sigue al panel activo y muestra las primeras líneas en letra de ancho fijo (Liberation Mono 11,5 en el diseño; Consolas/Menlo en la app) sobre `background` con borde, sin ajustar (las líneas largas se cortan), y los detalles («Archivo RS», 5 KB, fechas) | Cambiar de panel activo cambia la columna | `preview::draw` (texto), `preview::draw_details` |
| X41 | Variantes de la columna | Cuatro columnas lado a lado: carpeta (ícono de 96, «12 elementos · 1,24 GB» con el tamaño de xtree), «3 elementos seleccionados» (tres miniaturas en abanico con la cantidad en acento, «12,4 MB · Imagen JPEG»), solo en línea (ícono del tipo con la nube y «Disponible solo en línea», sin descargar) y cargando (ícono del tipo; la imagen lo reemplaza sin parpadeo) | — | `preview::draw`, `preview::draw_details` |
| X42 | Vista Galería | Base X02 (un panel), «Viaje» en la vista Galería con la columna cerrada: imagen grande (4:3, centrada, 16 de margen) y abajo la tira de 112 de alto con celdas de 88 (paso 96) y el enfocado (IMG_3101.jpg) centrado y resaltado; una celda bajo el mouse; carpeta y archivos sin vista previa con el ícono de 48. Botón de la columna apagado | ←/→ (y ↑/↓) pasan; clic enfoca; doble clic o Enter abre; Ctrl/⇧ en la tira; Ctrl+Shift+7 / ⌥⌘7 | `list::FileList::draw_gallery`, `list::FileList::draw_strip_cell` |
| X43 | Vista rápida en Windows con visor (PDF) | Ventana propia de xdir de 960×680 centrada sobre la principal (radio 8, sombra), barra de título con ícono y nombre, ‹ «2 de 12» ›, «Abrir» y cerrar; el resto es el visor del sistema (`IPreviewHandler` del PDF) con una página de muestra y su barra de desplazamiento | Espacio o Esc cierran; flechas pasan al anterior o siguiente (la selección de la lista acompaña); rueda y clic van al visor | `quicklook::draw_title_bar`, `quicklook` (Windows: `sys::win::quicklook`) |
| X44 | Vista rápida en Windows sin visor (video) | Misma ventana con VID_3102.mp4 («12 de 12»): la imagen grande del servicio de miniaturas (768×432, marca de video y duración), «No hay un visor para este tipo de archivo», «Abrir con…» y «Abrir» | «Abrir» lo abre con su programa; «Abrir con…» muestra el diálogo del sistema | `quicklook::draw_title_bar`, `quicklook::draw_fallback` |

Medidas que fija el diseño:
- Columna de vista previa: 320 de ancho al empezar, 240 de mínimo y 640 de máximo (y nunca deja un panel más angosto que su
  mínimo); 6 de separación con los paneles, como el árbol del otro lado. Margen interior 16; vista previa de 288×216
  (proporción 4:3 del ancho) o 288×300 para texto; detalles 20 más abajo, filas de 22 con el valor desde x = 104.
- Botón de la columna: 30×22 entre «Grupos» y los botones de layout (`ui::preview_rect`: 10 a la izquierda del primer
  layout); «Grupos» y «xtree» se corren 40 a la izquierda.
- Galería: tira de 112 de alto (línea `border` arriba), celdas de 88×88 con miniatura de 72 y paso de 96; la imagen grande
  usa el resto con 16 de margen.
- Vista rápida (Windows): 960×680 al empezar, centrada sobre la ventana de xdir, mínimo 480×360; barra de título de 40.

Notas de X39–X44:
- Los paneles de X39 y X40 (501 de ancho) se dibujan a medida con las columnas Nombre, Fecha de modificación y Tamaño, como
  en X36 y X37. Con el árbol fijado (240) y 2 paneles en una ventana de 1400 los paneles quedarían en 405.
- Las fotos y la página del PDF son contenido (colores literales, no del tema); el fondo del visor de X43 es el del visor
  del sistema (gris oscuro o claro según el modo de Windows).
- En macOS la vista rápida es el `QLPreviewPanel` del sistema: no se dibuja. Los elementos solo en la nube no se le pasan;
  xdir muestra su tarjeta «Disponible solo en línea» (la variante de X41).

### Lo nativo, que no se dibuja

- Vista rápida de macOS (`QLPreviewPanel`) y el contenido del visor de Windows (`IPreviewHandler`).
- Menús contextuales de la lista, del panel de favoritos, de las pestañas y del botón «Grupos» (menú del sistema).
- Menú de subcarpetas que abre la flecha entre dos tramos de la barra de dirección.
- Diálogos del sistema: elegir carpeta, credenciales de red, propiedades, confirmaciones (`NSAlert` en macOS).
- Los controles de edición que se montan encima de la interfaz (barra de dirección, caja de búsqueda, renombrar) son nativos; en X08, X13 y X30 se dibujan con su aspecto aproximado.
- Barra de título y marco de la ventana (Mica en Windows 11).
- Íconos de archivo del sistema y miniaturas reales: acá son formas simples y rectángulos de color.

### Qué no se pudo representar fiel

- Textos con puntos suspensivos: se cortan a mano; el ancho real depende de Segoe UI o San Francisco.
- Íconos de Segoe MDL2 / SF Symbols: vectores simples.
- Logo de Runick: marcador de texto sobre la franja oscura.
- Miniaturas: la misma imagen de muestra en todas las celdas.
- El marco del panel (pestañas con esquinas redondeadas unidas al cuerpo) se aproxima con un trapecio y un trazo continuo.

## Cómo se edita

```
figcraft-cli info docs/design/xdir.figcraft
figcraft-cli run <script.json> --in docs/design/xdir.figcraft --out docs/design/xdir.figcraft
figcraft-cli render docs/design/xdir.figcraft -o /tmp/xtree.png --page "Pantallas · xtree"
figcraft-cli export docs/design/xdir.figcraft -o /tmp/p.png --page "Pantallas · xtree" --node "T03 · Análisis completo, color por carpeta · Oscuro"
```

Los scripts de `run` son un arreglo de `{"command", "params"}` (ver `figcraft-cli commands`); `command.batch`
con `as` y `{"$ref"}` permite encadenar ids. Los renders van **fuera del repo**. Opcional, para editar con
Claude vía MCP: `claude mcp add -s local figcraft -- ~/bin/figcraft-cli-0.12.1/figcraft-cli mcp`.

## Diferencias a propósito

- Fuente **Inter** en FigCraft; la app dibuja con Segoe UI en Windows y San Francisco en macOS. Los anchos
  de texto pueden variar unos píxeles.
- Los íconos (Segoe MDL2 / SF Symbols) son formas simples dibujadas con vectores.
- Los datos (carpetas, tamaños, rutas) son de ejemplo; los porcentajes y totales son coherentes entre sí.
- El texto largo se corta a mano en el diseño (FigCraft no centra verticalmente el texto truncado); en la
  app se recorta con puntos suspensivos.
- No hay «Connect to code»: la relación componente-función está en el nombre de cada componente.

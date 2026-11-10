# Diseño: el archivo de FigCraft de xdir

Todo vive en `docs/design/xdir.figcraft` (un solo archivo; no se guardan imágenes: los renders se hacen fuera del repo).
Sigue la forma del de Cromos: **cada página corresponde a una parte del código** y cada componente lleva
el nombre de la función de Rust que lo dibuja.

## Qué hay

| Página de FigCraft | Módulo | Qué hay |
|---|---|---|
| **Tokens** | `crates/xdir-core/src/theme.rs`, `crates/xdir-core/src/xtree/colors.rs` | Colecciones de variables: **Tema** (modos Oscuro y Claro: background, surface, surface_alt, border, text, text_dim, text_secondary, accent, focus_fill, hover, selection, scrollbar, advertencia (ámbar #FAC74D en oscuro, #B7791F en claro), carpeta (amarillo fijo), ok/peligro y tres neutros del mapa), **Acentos** (los 12 `PRESETS`), **Mapa** (`WHEEL` y `CATS`) y **Medidas** (constantes de `crates/xdir-core/src/xtree/view.rs` y `crates/xdir-core/src/ui.rs`). Estilos de texto `Texto/ui`, `ui_bold`, `title`, `small`, `small_bold`: los `FontId::{Ui, UiBold, Title, Small, SmallBold}` de `crates/xdir-core/src/paint.rs`. |
| **Componentes** | `crates/xdir-core/src/ui.rs` y la vista de xtree | De xtree: `xtree::view::top_bar`, `table_row`, `folder_block`, `file_block`, `card`, `drive_menu`, `disk_bar` y `legend`. De xdir: `ui::draw_status` (14 variantes: xtree encendido o apagado y los 5 layouts, y con 2 paneles el indicador de xtree en progreso, giro, terminado y aviso), `ui::draw_group_bar` (píldora de grupo en 5 estados y botón «+»), `pane::Pane::draw_tabs`, `pane::Pane::draw_search`, `pane::Pane::draw_favs`, `list::FileList::draw` (fila en 6 estados y encabezado), `list::FileList::draw_cell` (cuadrícula), `tree::Tree::draw` (fijado, colapsado y fijado con la entrada «xtree»), `palette::Palette::draw` (comandos, carpetas, nombre de grupo), `typed::Suggest::draw`, `about::draw`, `ui::draw_picker` y `ui::draw_terminal_menu`. De la tanda B (sección «Tanda B: organizar la lista»): `list::FileList::draw_group_header` (encabezado de grupo expandido, plegado y con foco), `list::FileList::draw_checkbox` (casilla vacía, marcada y parcial), `list::FileList::draw_filter_menu` (desplegable del filtro de columna), `list::FileList::draw_content_row` (fila de la vista Contenido en 5 variantes) y `list::FileList::draw_list_cell` (celda de la vista Lista en 5 variantes). De la sección «Estado de xtree y cartel de ayuda»: `ui::draw_xtree_badge` (indicador de 14×14: progreso con anillo al 37 %, giro, terminado en `ok` y aviso en `warning`/advertencia) y `app::tooltip` (cartel de ayuda: `surface_alt`, borde `border`, radio 4, texto Small, 6 de margen, alto 26). De la tanda C (sección «Tanda C: vista previa»): `ui::draw_preview_toggle` (botón de la columna de vista previa: apagado, bajo el mouse y encendido; cartel «Vista previa (Alt+P)» / «Vista previa (⌥⌘P)»), tres variantes más de `ui::draw_status` con ese botón (`Vista previa=Encendida/Apagada`), `preview::draw` (la columna en 6 variantes: imagen, texto, carpeta, varios seleccionados, sin vista previa o solo en línea y cargando), `preview::draw_details` (el bloque de detalles en las mismas 6), `list::FileList::draw_strip_cell` (celda de la tira de la Galería: normal, bajo el mouse, seleccionada, seleccionada con foco y con casilla) y `quicklook::draw_title_bar` (barra de título de la vista rápida de Windows). De la tanda D (sección «Tanda D: operaciones con archivos»): `list::draw_eject` (botón ⏏: normal y bajo el mouse; lo usan la lista de Este equipo, `tree::Tree::draw` y, como botón «⏏ Expulsar», `info::draw`), `transfer::view::draw_graph` (gráfico de velocidad, en curso y en pausa), `transfer::view::draw` (contenido de la ventana de copia: la cola con cada trabajo en curso, en pausa, en cola, comprimiendo y extrayendo; más y menos detalles; el conflicto de archivo y de carpeta con sus dos tarjetas; los errores al terminar; los clics con `transfer::view::hit`), `info::draw::{layout, draw, hit}` (el panel de información: `layout` arma el encabezado con imagen, carpeta calculando o disco y las secciones abiertas o plegadas; `draw` lo pinta y `hit` resuelve los clics), `rename_many_draw::draw` (panel de renombrar varios en sus 3 modos; `rename_many_draw::{fields, hit}` para las cajas y los clics) y `list::FileList::draw_rename_name` en `list/rename_preview.rs` (vista previa en la columna Nombre: cambia, sin cambios y choque). De la tanda D+ (sección «Tanda D+: configuración y red»; nombres propuestos, todavía no existen): `settings::draw_nav_item` (sección de la lista de la izquierda: normal, bajo el mouse, elegida, con resultados y sin resultados), `settings::draw_switch` (encendido y apagado), `settings::draw_segmented` (3 opciones, elegida la 1, 2 o 3), `settings::draw_row` (fila de una opción con switch encendido o apagado, desplegable, deslizador, botón o solo texto), `settings::draw_color_row` (color de acento: un preset o «Del sistema»), `settings::draw_shortcut_row` (fila de Atajos: normal, dos atajos, sin atajo, cambiado, bajo el mouse, grabando, choque y rechazo), `settings::draw_net_row` (sección Red: favorito, favorito editándose, reciente y conectado), `netdlg::draw_server_row` (favorito, reciente, reciente bajo el mouse y elegida), `netdlg::draw` («Conectar a un servidor»: vacío, con favoritos y recientes, conectando y error), `netdlg::draw_map_drive` y `netdlg::draw_letter_menu` («Conectar unidad de red» de Windows y su desplegable de letras), `netdlg::draw_unmap_drive` («Desconectar unidad de red»), `list::FileList::draw_remote_wait` (carpeta remota: conectando y no responde) y `tree::Tree::draw_net_node` (nodos de «Red»: favorito, conectado, conectado bajo el mouse, conectado en la fila actual y máquina; cartel «Desconectar»). Las variantes (estado, tipo, sistema) son propiedades del componente. |
| **Pantallas · xtree** | la vista de xtree dentro de xdir | 9 pantallas de 1400×820, cada una en tema oscuro y claro, en dos grupos («Vista de xtree (T01–T07)» y «Estado de xtree: cronómetro y porcentaje (T08–T09)»; ver «Cómo están ordenadas las pantallas»): T01 escaneando con mapa en vivo, T02 fase MFT con %, T03 completo (color por carpeta), T04 color por tipo con leyenda, T05 menú de discos abierto, T06 aviso de carpetas sin permiso (macOS), T07 aviso de administrador (Windows), T08 cronómetro y porcentaje («Escaneando… 37 % · 0:42» y barra de avance de 3 px en acento bajo la barra superior), T09 «Análisis completado en 1:23». |
| **Pantallas · xdir** | `crates/xdir-core/src/app/paint.rs` y lo que dibuja | 31 pantallas X01–X31 de la ventana principal que xdir ya tiene hoy, 6 pantallas X32–X37 de la tanda B, la X38 del estado de xtree y las X39–X44 de la tanda C (vista previa, Galería y vista rápida), todas ya programadas, las X45–X53 de la tanda D (operaciones con archivos), también programadas, y las X54–X67 de la tanda D+ (configuración y red), diseñadas antes de programar, en oscuro y claro, en seis grupos: «Ventana principal (X01–X31)», «Tanda B: organizar la lista (X32–X37)», «Estado de xtree (X38)», «Tanda C: vista previa, Galería y vista rápida (X39–X44)», «Tanda D: operaciones con archivos (X45–X53)» y «Tanda D+: configuración y red (X54–X67)» (ver los inventarios y «Cómo están ordenadas las pantallas»). |

Las pantallas usan **instancias** de los componentes (barra de estado, píldoras de grupo, filas y celdas de la lista, favoritos, árbol, paleta, desplegable de rutas, acerca de, panel de acento, menú de consola; en xtree, barra superior, filas, tarjeta, menú, barra de datos, leyenda). Los bloques del mapa se dibujan a medida con la misma receta que
`folder_block` y `file_block`; el marco del panel (pestañas, barra de dirección con migas, caja de búsqueda, pie) y las filas con columnas distintas de las de fábrica también se dibujan a medida con la receta de `pane::Pane::draw` y `list::FileList::draw`, porque dependen del ancho. El tema se elige con el modo de la variable en el marco de cada pantalla.

### Cómo están ordenadas las pantallas

En las dos páginas de pantallas cada tema (una tanda o una parte de la app) es un **grupo** con su título arriba: Inter
SemiBold **120** en ámbar (la familia, el peso y el color de los títulos de sección de Componentes, `S:3`, pero sin el
estilo, que es de 16, para que se lea con la página entera a la vista); caja de 145 de alto. Dentro del grupo las
pantallas van en orden, lado a lado, en **filas de hasta 6**; debajo de cada fila oscura va la misma fila en claro, y después
la siguiente tanda de 6 (oscura, clara…). Medidas: margen 60 alrededor de todo; el título arriba del grupo y su primera fila 225 más abajo (145 del título + 80 de aire);
200 entre pantallas, a lo ancho y a lo alto (también entre la fila oscura y la clara); **400 entre grupos**. Con pantallas
de 1400×820 la fila llena mide 9400 y el marco de la página 9520 de ancho; el alto se ajusta al último grupo
(Pantallas · xdir: 30830; Pantallas · xtree: 6690). No hay marcos de grupo: títulos y pantallas son hijos directos del
marco de la página, y en la lista de capas van en ese orden (título, fila oscura, fila clara…).

| Página | Grupo | y del título | Filas (oscura / clara) |
|---|---|---|---|
| Pantallas · xdir | Ventana principal (X01–X31) | 60 | 6 pares (X01–X06 … X25–X30, X31 sola) |
| Pantallas · xdir | Tanda B: organizar la lista (X32–X37) | 12725 | 1 par |
| Pantallas · xdir | Estado de xtree (X38) | 15190 | 1 par |
| Pantallas · xdir | Tanda C: vista previa, Galería y vista rápida (X39–X44) | 17655 | 1 par |
| Pantallas · xdir | Tanda D: operaciones con archivos (X45–X53) | 20120 | 2 pares (X45–X50, X51–X53) |
| Pantallas · xdir | Tanda D+: configuración y red (X54–X67) | 24625 | 3 pares (X54–X59, X60–X65, X66–X67) |
| Pantallas · xtree | Vista de xtree (T01–T07) | 60 | 2 pares (T01–T06, T07 sola) |
| Pantallas · xtree | Estado de xtree: cronómetro y porcentaje (T08–T09) | 4565 | 1 par |

Pantallas nuevas (la tanda D en adelante): van como **un grupo nuevo al final** de su página, 400 debajo del último,
con la misma regla (título, filas de 6, la clara debajo de la oscura) y agrandando el alto del marco para que quede 60 de
margen abajo. No se intercalan pantallas en grupos viejos.

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

Diseñadas antes de programar y ya programadas (spec `docs/superpowers/specs/2026-11-09-explorador-tanda-b-design.md`): fijan el layout de los
encabezados de grupo, el desplegable de filtros, las vistas Lista y Contenido y las casillas. Son el grupo «Tanda B: organizar la lista (X32–X37)», con la base
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

Diseñadas antes de programar (spec `docs/superpowers/specs/2026-11-09-explorador-tanda-c-design.md`) y ya programadas. Son el grupo
«Tanda C: vista previa, Galería y vista rápida (X39–X44)», el último de la página (una fila oscura y su clara), en oscuro y claro.

| N.º | Pantalla | Qué muestra | Qué se hace | Rust |
|---|---|---|---|---|
| X39 | Columna de vista previa con una imagen | Base X16 (árbol colapsado) con 2 paneles de 501 y la columna de 320 a la derecha (x = 1074, mismo alto que el árbol). Panel activo «Viaje» con IMG_3101.jpg seleccionada con foco; la columna muestra la imagen (288×216) y los detalles: nombre, «Imagen JPEG», Tamaño 3,8 MB, Creado, Modificado, Último acceso, Dimensiones 4032 × 3024, Autor y Etiquetas en píldoras. Botón de la columna encendido en la barra de estado, bajo el mouse, con el cartel «Vista previa (Alt+P)» arriba | Botón, Alt+P / ⌥⌘P o el comando «Mostrar u ocultar la vista previa»; arrastrar el borde izquierdo de la columna cambia el ancho | `preview::draw`, `preview::draw_details`, `ui::draw_preview_toggle`, `ui::draw_status`, `ui::pane_rects` (área de paneles sin la columna), `app::tooltip` |
| X40 | Columna con un archivo de texto | El panel activo es el de la derecha (`src` de xdir-core, about.rs con foco): la columna sigue al panel activo y muestra las primeras líneas en letra de ancho fijo (Liberation Mono 11,5 en el diseño; Consolas/Menlo en la app) sobre `background` con borde, sin ajustar (las líneas largas se cortan), y los detalles («Archivo RS», 5 KB, fechas) | Cambiar de panel activo cambia la columna | `preview::draw` (texto), `preview::draw_details` |
| X41 | Variantes de la columna | Cuatro columnas lado a lado: carpeta (ícono de 96, «12 elementos · 1,24 GB» con el tamaño de xtree), «3 elementos seleccionados» (tres miniaturas en abanico con la cantidad en acento, «12,4 MB · Imagen JPEG»), solo en línea (ícono del tipo con la nube y «Disponible solo en línea», sin descargar) y cargando (ícono del tipo; la imagen lo reemplaza sin parpadeo) | — | `preview::draw`, `preview::draw_details` |
| X42 | Vista Galería | Base X02 (un panel), «Viaje» en la vista Galería con la columna cerrada: imagen grande (4:3, centrada, 16 de margen) y abajo la tira de 112 de alto con celdas de 88 (paso 96) y el enfocado (IMG_3101.jpg) centrado y resaltado; una celda bajo el mouse; carpeta y archivos sin vista previa con el ícono de 48. Botón de la columna apagado | ←/→ (y ↑/↓) pasan; clic enfoca; doble clic o Enter abre; Ctrl/⇧ en la tira; Ctrl+Shift+7 / ⌥⌘7 | `list::FileList::draw_gallery`, `list::FileList::draw_strip_cell` |
| X43 | Vista rápida en Windows con visor (PDF) | Ventana propia de xdir de 960×680 centrada sobre la principal (radio 8, sombra), barra de título con ícono y nombre, ‹ «2 de 12» ›, «Abrir» y cerrar; el resto es el visor del sistema (`IPreviewHandler` del PDF) con una página de muestra y su barra de desplazamiento | Espacio o Esc cierran; flechas pasan al anterior o siguiente (la selección de la lista acompaña); rueda y clic van al visor | `quicklook::draw_title_bar`; Windows: `crates/xdir-windows/src/quicklook` (`window.rs`, `handler.rs`) |
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
- Galería (X42) en la app, a propósito distinta del diseño:
  - Si todas las celdas entran en la tira, se centra el conjunto y no el enfocado. X42 tiene 12 celdas que entran y centra
    el enfocado.
  - La tira se centra en el enfocado con el teclado, la rueda y la búsqueda por tipeo. Un clic no la mueve, para que el
    doble clic caiga en la misma celda; solo la corre lo justo si la celda cliqueada no se ve entera.
  - `list::CellState` tiene un estado más que el componente: `Focus`, el foco sin selección que deja Ctrl+flecha. Se
    dibuja con el fondo `focus_fill` y el borde del acento, como en la lista.
  - En el panel inactivo, la selección de la tira es el gris de la lista (`text_dim` al 18 %), no `selection`.
  - La imagen grande y las miniaturas de la tira se dibujan con esquinas rectas, no con radio 6 y 4: `Painter` todavía no
    recorta imágenes con esquinas redondeadas.
  - X42 no escribe nombres; la app sí, para distinguir una carpeta de carpetas (todas con el mismo ícono). Debajo de la
    imagen grande, dentro de su área (`list::gallery::CAPTION_H`, 28), una línea centrada con el nombre del enfocado
    (`Ui`, `text`) y al lado, chico y tenue, el tipo y el tamaño («JPEG image · 115 KB»; las carpetas, solo el tipo).
    En la tira, el nombre debajo de cada miniatura en una línea (`Small`, `text_dim`; seleccionada o con foco, `text`)
    cortado con «…» al ancho de la celda. Por eso la tira mide 132 de alto (no 112) y las celdas 88×108: la miniatura
    de 72 con 8 de margen arriba y a los lados, y la línea del nombre de 18 debajo.
- Tarjeta de la vista rápida dibujada por xdir (sin visor del sistema, o un elemento solo en la nube), en la app:
  - La sombra del diseño (negro al 45 %, 12 abajo, desenfoque 36) se aproxima con 8 capas redondeadas que crecen de a 3,
    corridas 12 hacia abajo (negro al 5 % cada una en oscuro, al 2,2 % en claro): `Painter` no tiene desenfoque.
  - `quicklook::overlay_rect` deja 24 de margen a cada lado dentro de la ventana de xdir antes de bajar de 960×680 (nunca
    menos de 480×360, salvo que la ventana misma sea más chica).
  - Las áreas que responden al clic son más grandes que el dibujo: ‹ y › de 22×28, la cruz de 28×28 (el dibujo es de
    5×8 y 10×10); «Abrir» es el botón entero de 72×26.
- Ventana de la vista rápida en Windows (X43/X44), en la app:
  - Sin marco del sistema: la sombra, el borde y el radio 8 son los de DWM (esquinas redondeadas de Windows 11), no la
    sombra en capas de la tarjeta.
  - El visor queda 6 adentro de los costados y del borde de abajo (fondo `surface`): es una ventana de otro proceso y
    sobre ella el mouse no llega a la de xdir, así que ese borde es el que redimensiona. En X43 el visor llega al borde.
  - Mientras el visor arranca (hasta 3 s), el cuerpo muestra el ícono de 96 del tipo, centrado.
  - Los archivos solo en la nube se muestran en la misma ventana (ícono y «Disponible solo en línea»), no en la tarjeta
    sobre los paneles.

## Tanda D: operaciones con archivos (X45–X53)

Programadas (spec `docs/superpowers/specs/2026-11-10-explorador-tanda-d-design.md`, con las «Decisiones del usuario
(2026-11-10)», que mandan, y los «Desvíos en la implementación»: información y renombrar varios son un panel a la derecha; la copia es una ventana aparte
del sistema con gráfico de velocidad; el avance de comprimir y extraer va en esa ventana). Son el grupo «Tanda D: operaciones
con archivos (X45–X53)» (dos pares de filas: X45–X50 y X51–X53), en oscuro y claro. Rutas de Windows.

| N.º | Pantalla | Qué muestra | Qué se hace | Rust |
|---|---|---|---|---|
| X45 | Ventana de copia: cola y gráfico de velocidad | Base X02 (un panel, Descargas con los 9 elementos seleccionados). Encima, la ventana de copia como ventana propia del sistema (560 de ancho, barra de título de 32 con «58 % completado», minimizar y cerrar; radio 8 y sombra): «Copiando 1.284 elementos de Descargas a Respaldo», «58 % completado», Pausa y Cancelar, gráfico de velocidad (área de MB/s hasta el 58 % del ancho, línea de la velocidad actual «Velocidad: 48,2 MB/s» y «Promedio: 46,9 MB/s»), Nombre, Tiempo restante («Unos 2 minutos»), Elementos restantes («539 (5,1 GB)»), «Menos detalles»; la cola: «Moviendo 3 elementos de Escritorio a Proyectos» corriendo (31 %, 12,4 MB/s, quedan 20 s) y «Copiando "Proyecto 2026" a Respaldo» en cola | Pausa/Reanudar y Cancelar por trabajo; «Menos detalles» cambia el gráfico por una barra (variante de `transfer::view::draw`); minimizar deja el indicador en la barra de estado | `transfer::view::draw`, `transfer::view::draw_graph`; ventana propia del sistema (Windows: como la de la vista rápida; macOS: `NSWindow`) |
| X46 | Ventana de copia: conflicto | La misma ventana con el trabajo esperando: «Ya existe "informe.pdf" en Respaldo», las dos tarjetas (el que se copia, «Más nuevo», 2,1 MB, hoy 10:42; el que ya está, 1,9 MB, ayer 18:05), Reemplazar (principal), Conservar ambos, Omitir y la casilla «Hacer esto con todos (12 conflictos)»; abajo la cola sigue: «Comprimiendo 3 elementos en "Viaje.zip"» (64 %, el avance del zip) y uno en cola. La variante de carpeta (Combinar primero; «Hacer esto con todas las carpetas (2 conflictos)») también la dibuja `transfer::view::draw` | Elegir; la casilla aplica a los conflictos que quedan de ese trabajo | `transfer::view::draw` (conflicto) |
| X47 | Ventana de copia: errores al terminar | «Se copiaron 1.281 de 1.284 elementos de Descargas a Respaldo», «Terminó con 3 errores» en danger, la lista (Instalador-Rust.exe «En uso por otro programa», factura-0039.pdf «Sin permiso», setup-viejo.msi «Ya no existe», con su carpeta), Reintentar y Cerrar; debajo el zip sigue corriendo | Reintentar solo los que fallaron; Cerrar saca el trabajo de la cola | `transfer::view::draw` (errores) |
| X48 | Panel de información de una imagen | Base X39 (árbol colapsado, 2 paneles de 501) con el panel de información en el lugar de la columna (x = 1074, 320×787): «Información» y cerrar; encabezado con la vista previa de 72, IMG_3101.jpg, Imagen JPEG, 3,8 MB y modificado; Etiquetas (Rojo, Viaje y Bariloche con ×, y +), General (tipo, tamaño en bytes, en disco, ubicación como enlace, creado, modificado y las casillas Bloqueado / Oculto / Solo lectura), Más información (dimensiones, autor, abierto), Nombre y extensión y Comentarios plegadas, Abrir con («Fotos (predeterminada)», la nota y «Cambiar todos…») y Compartir y permisos cortada abajo: el panel se desplaza (barra de 4) | Alt+Enter / ⌘I, o «Propiedades» / «Obtener información», lo abren y lo cierran; ▾/▸ pliegan (se guarda en la sesión) | `info::draw::{layout, draw, hit}` |
| X49 | Panel de información de una carpeta | Imágenes con «Fotos 2026» seleccionada: el encabezado y General dicen «Calculando… 3,2 GB» con el indicador girando y «12.481 elementos»; Etiquetas vacía («Agregar etiqueta»); el resto plegado y «Más en el sistema…» al final | El tamaño se suma en otro hilo (al instante si xtree ya escaneó) | `info::draw::layout` (carpeta) |
| X50 | Panel de información de un disco | Este equipo con PENDRIVE (E:) seleccionado (⏏ en las filas de E: y F:); el panel: disco, «Disco extraíble (USB)», «21,4 GB disponibles de 58 GB», barra de capacidad con Usado y Disponible, tipo, formato (exFAT), capacidad, disponible, usado y el botón «⏏ Expulsar» | Expulsar desde el panel | `info::draw::layout` (disco, con «⏏ Expulsar») |
| X51 | Renombrar varios | Base X16 con un solo panel de 1008 y el panel de renombrar a la derecha (320): modo Reemplazar texto, Buscar «IMG_» (con foco), Reemplazar con «Bariloche-», «Distinguir mayúsculas», «8 de 10 cambian», «1 choque: "Bariloche-3090.jpg" ya existe», Cancelar y Renombrar apagado. En la lista, los 10 seleccionados: el nombre nuevo con «Bariloche-» sobre el acento y el viejo tachado al lado; IMG_3090.jpg en rojo (fondo danger) con el signo y el cartel «Ya existe "Bariloche-3090.jpg" en esta carpeta»; mapa-ruta-40.jpg y panoramica-cerro.jpg con «sin cambios» | F2 con varios seleccionados, «Renombrar N elementos…» o la paleta; la vista previa se actualiza al teclear; Esc o Cancelar vuelven a la lista normal | `rename_many_draw::draw`, `list::FileList::draw_rename_name` (`list/rename_preview.rs`), `app::tooltip` |
| X52 | Expulsar en Este equipo y en el árbol | Base X15 (árbol fijado): en el árbol, debajo de «Este equipo» (fila actual), PENDRIVE (E:) y Respaldo (F:) con ⏏ a la derecha (el de F: bajo el mouse, con el cartel «Expulsar»); en el panel, Este equipo con ⏏ al final de la columna Nombre de E: y F:, y el menú del clic derecho de PENDRIVE (E:) con «Expulsar» bajo el mouse | ⏏, el menú o la paleta expulsan; mientras tanto el botón gira; «Se puede quitar "PENDRIVE (E:)" con seguridad» en la barra de estado | `list::draw_eject` (desde `list::FileList::draw` y `tree::Tree::draw`) |
| X53 | Papelera con «Devolver» | Base X02: la Papelera (ícono y «Papelera» en la barra de dirección) con columnas Nombre, Ubicación original, Fecha de eliminación y Tamaño; dos seleccionados; el botón «Devolver» junto a la barra de dirección (que se achica 108) y el menú del clic derecho con «Devolver» bajo el mouse, «Eliminar definitivamente…», Cortar y Propiedades | Devuelve cada uno a su carpeta (la crea si no está; los choques van al conflicto de la ventana de copia); ⌘⌫ en macOS | `pane::Pane::draw` (botón de la Papelera), `list::FileList::draw` |

Medidas que fija el diseño:
- Ventana de copia: 560 de ancho y el alto del contenido (con «Más detalles» y dos trabajos en la cola, 32 + 410 = 442; con
  «Menos detalles», 276; con un conflicto, 468; con errores, 365). Margen 16; encabezado en Ui (`text_secondary`) y el
  porcentaje en Title; Pausa y Cancelar de 84×26. Se abre centrada sobre xdir; la cola crece hacia abajo y, si no entra,
  se desplaza.
- Gráfico de velocidad: 528×100 (el ancho de la ventana menos los márgenes), fondo `background`, borde y radio 4, guías al
  25/50/75 %, área en acento al 28 % con su línea de 1,5, escala hasta el máximo visto (80 MB/s en el ejemplo).
- Fila de la cola: 560×64 (56 en cola), barra de 4 de 440 desde x = 40, botones de 24.
- Conflicto: 528×222; tarjetas de 248×96 con ícono de 48; botones de 28 de alto.
- Panel de información y de renombrar: el lugar y las medidas de la columna de vista previa (320 al empezar, 240–640,
  mismo alto que los paneles; uno solo a la vez entre vista previa, información y renombrar). Barra superior de 40;
  margen 16; encabezado de 100 con vista previa de 72; secciones con cabecera de 32 y filas de 22 con el valor desde
  x = 112; «Más en el sistema…» (150×28) al final del contenido. Renombrar: modos en un control segmentado de 288×28,
  campos de 28, Cancelar y Renombrar en un pie fijo de 52.
- Vista previa de renombrar: la parte que cambia sobre el acento al 30 % (danger en un choque), el nombre viejo 12 a la
  derecha, tachado, en `text_dim`; el choque pinta la fila entera en danger al 18 % en lugar de la selección.
- ⏏: 20×20, radio 4; en la lista al final de la columna Nombre (28 antes del borde), en el árbol en x = 208 (donde va la
  chincheta en Acceso rápido).

Notas de X45–X53:
- La ventana de copia es del sistema: su barra de título (ícono, título, minimizar y cerrar) se dibuja aproximada, como
  los menús; el contenido es de xdir (`transfer::view::draw`). Se dibuja sobre la ventana de xdir porque así se ve al usarla.
- Las casillas de General son las de cada sistema: Bloqueado solo en macOS, Oculto en Windows y macOS (en Linux marcada
  y apagada si el nombre empieza con punto), Solo lectura en Windows y Linux. En el diseño están las tres.
- «Ocultar extensión» solo existe en macOS; en Windows y Linux no se muestra. «Abrir con» y «Compartir y permisos» cambian
  según el sistema (ver el spec); en Windows los permisos son solo para ver.
- En X49 la carpeta no tiene «Abrir con»; en X50 el disco no tiene etiquetas ni «Abrir con».
- Los paneles de X48–X50 (501) y el de X51 (1008) se dibujan a medida, como en X39; el árbol de X52 es una copia
  desacoplada de `tree::Tree::draw` con las dos unidades nuevas (como el panel de la rueda dentada de X37).
- `list::FileList::draw_rename_name` tiene el ancho fijo de lo que cambia («Bariloche-»): en la app se mide el texto.
- Los colores de las etiquetas (rojo, azul) son los del Finder; en Windows las palabras clave no tienen color.

## Tanda D+: configuración y red (X54–X67)

Diseñadas antes de programar (spec `docs/superpowers/specs/2026-11-10-explorador-tanda-dmas-design.md`). Son el grupo
«Tanda D+: configuración y red (X54–X67)», el último de la página (tres pares de filas: X54–X59, X60–X65 y X66–X67), en
oscuro y claro. Rutas y atajos de Windows (Ctrl+…; en macOS los chips dicen ⌘…).

| N.º | Pantalla | Qué muestra | Qué se hace | Rust (propuesto) |
|---|---|---|---|---|
| X54 | Configuración: Apariencia | Base X02: la pestaña «⚙ Configuración» (activa, después de «Descargas») en el panel; barra de dirección «⚙ Configuración › Apariencia» con Atrás/Adelante/Subir/Recargar apagados; la caja de búsqueda del panel dice «Buscar en la configuración»; a la izquierda la lista de secciones (Apariencia elegida, Atajos bajo el mouse); Tema (segmentado; en el oscuro está elegido Oscuro y en el claro, Claro), Color de acento (12 presets, «Del sistema», «Otro…») y Animaciones (Sistema / Sí / No) | Ctrl+, / ⌘, o «Más opciones…» de la rueda; cada cambio se guarda solo | `settings::draw_nav_item`, `settings::draw_row`, `settings::draw_segmented`, `settings::draw_color_row` |
| X55 | Configuración: búsqueda | «carpeta» escrito en la caja (borde de acento y ✕); la lista de secciones con la cantidad que coincide (las que no tienen, atenuadas con 0); a la derecha «Resultados para "carpeta"» y 7 opciones agrupadas por sección (Lista y vistas, Árbol, Atajos de teclado, xtree, Avanzado) | Sin mayúsculas ni tildes; borrar vuelve a la sección elegida | `settings::draw_nav_item` (Con/Sin resultados) |
| X56 | Configuración: Atajos de teclado | «Restablecer todos»; grupos Navegación, Archivos y Red; filas normales, con dos atajos y «Sin atajo»; «Copiar las rutas como texto» con choque («Ctrl+D ya es de "Duplicar"», Reasignar / Cancelar); «Duplicar» bajo el mouse (✕ en el chip); «Renombrar» grabando («Apretá la combinación…», «Esc cancela»); «Enviar a la papelera» con rechazo («Alt+F4 la usa el sistema»); «Conectar a un servidor…» cambiado (Ctrl+Shift+K, «(cambiado)», ↺); barra de desplazamiento | ✎ graba, + suma, ✕ quita, ↺ restablece; Reasignar escribe las dos líneas | `settings::draw_shortcut_row` |
| X57 | Configuración: Red | Favoritos (★, nombre, dirección, ✎ ↑ ↓ ✕; «Oficina» editándose en línea) con «Agregar…»; Recientes (cuándo, ☆, ✕) con «Borrar los recientes»; Conectados ahora (Z: \\srv01\compartido y \\nas.local\fotos sin letra, con ⏏); aviso «xdir no guarda contraseñas» con «Abrir el Administrador de credenciales» | Ordenar, editar, quitar, pasar a favoritos, desconectar | `settings::draw_net_row` |
| X58 | Configuración: Lista y vistas | Switches (extensiones, ocultos, casillas, barra de grupos, colores por tipo), las reglas de color activas solo para ver (la propia primero, con «propia») con «Editar en config.toml», y Vista por defecto (desplegable) | — | `settings::draw_row` |
| X59 | Hoja: Árbol, Vista previa, xtree y Avanzado | Solo el contenido de las cuatro secciones (sin el marco del panel; cada una ocupa la columna derecha al elegirla): Árbol (3 switches y «Volver a lo de config.toml»), Vista previa (switch y deslizador del ancho, 320 px), xtree (switch, desplegable y las teclas), Avanzado («Abrir config.toml», dónde están los archivos con la tabla y «Mostrar en el Explorador», modo portable) | — | `settings::draw_row` |
| X60 | Conectar a un servidor | Base X02 (Descargas). El panel centrado arriba: campo con foco («\\srv01\respaldo», puesto por el reciente elegido), ☆, el ejemplo, Favoritos (2), Recientes (3; uno elegido y otro bajo el mouse con ✕), Cancelar y Conectar | Ctrl+K / ⌘K; clic pone la dirección, doble clic conecta; Esc cierra | `netdlg::draw`, `netdlg::draw_server_row` |
| X61 | Conectar a un servidor: conectando | «Conectando con "srv01"…» con el indicador bajo el campo y el botón «Conectando…» apagado con indicador; Cancelar sigue | Cancelar abandona el intento | `netdlg::draw` (Conectando) |
| X62 | Conectar a un servidor: error | Campo con borde danger y «No se encuentra el servidor "srv02"» en danger; no se cierra | Corregir y reintentar | `netdlg::draw` (Error) |
| X63 | Conectar unidad de red (Windows) | Base Este equipo (sin unidades de red: Z: está libre). Panel con Unidad (Z:), el desplegable de letras abierto (Z: elegida, X: bajo el mouse), Carpeta con foco, «Examinar…», el ejemplo, «Conectar de nuevo al iniciar sesión» prendida y «Conectar con otras credenciales»; Cancelar y Finalizar | Finalizar conecta con la letra y abre la unidad en una pestaña nueva | `netdlg::draw_map_drive`, `netdlg::draw_letter_menu` |
| X64 | Desconectar unidad de red (Windows) | Este equipo con Z:; panel con Z: (marcada) y X: «desconectada» (recordada); Cancelar y Desconectar | Corre el flujo de ⏏ por cada una | `netdlg::draw_unmap_drive` |
| X65 | «Red» en el árbol y Z: en Este equipo | Base X15/X52: árbol con Acceso rápido y Este equipo plegados (Este equipo es la fila actual) y «Red» abierta: favoritos (★ NAS — fotos, ★ Oficina), conectados («fotos (nas.local)», «compartido (Z:)» bajo el mouse con ⏏ y el cartel «Desconectar»), separador y las máquinas de la red local; panel 1 con Este equipo y «compartido (\\srv01) (Z:)» seleccionada con ⏏ y «Unidad de red»; panel 2 con \\nas.local\fotos | Clic en un favorito conecta y abre; ⏏ desconecta | `tree::Tree::draw_net_node`, `ui::draw_eject`, `app::tooltip` |
| X66 | Carpetas remotas: «Conectando…» y «no responde» | Mismo árbol con «fotos (nas.local)» en la fila actual (⏏ siempre visible); panel 1 «Conectando con "nas.local"…» con el indicador, panel 2 «"srv01" no responde» con Reintentar y «Cerrar la pestaña»; estado «Conectando con "nas.local"…» | Se puede cambiar de pestaña, de panel o cerrar mientras tanto | `list::FileList::draw_remote_wait` |
| X67 | Rueda dentada con «Más opciones…» | Base X02; la rueda dentada (copia desacoplada de la de X37) con una fila más al pie, debajo de «Acerca de xdir»: ⚙ «Más opciones…» con «Ctrl+,», bajo el mouse | Cierra la rueda y abre la pestaña | `ui::draw_picker` |

Medidas que fija el diseño:
- Pestaña Configuración: lista de secciones de 220 (ítems de 204×32 cada 34, 12 arriba y 8 al costado; separador `border`);
  contenido desde x = 252 con el título en Title y filas de 820 (no se estiran con el panel); 4 entre filas.
- Fila de opción 820×56 (tarjeta `surface_alt`, borde, radio 6): título en Ui en y = 10, descripción en Small `text_dim`
  en y = 31; control a 16 del borde derecho: switch 40×20, segmentado 264×28 (88 por opción), desplegable 220×28,
  deslizador de 200 con el valor (56), botón de 180×28. Color de acento: 104 de alto, círculos de 22 cada 34.
- Fila de atajo 820×36 (choque 76, rechazo 64): chips de 104×24 desde x = 300, separados 8; botones de 24 cada 28.
- Fila de Red: 820×40. Encabezados de grupo en SmallBold `text_secondary`.
- «Conectar a un servidor»: 520 de ancho y alto del contenido (396 con 2 favoritos y 3 recientes; 216 vacío), en y = 96
  centrado; campo de 452×28 y ☆ de 28; filas de 488×32. Conectar unidad de red: 520×308; desconectar: 520×228;
  desplegable de letras 88 de ancho y filas de 26.
- Espera de carpeta remota: 360×104 centrado en la lista (y = 200).
- Nodos de «Red» en el árbol: filas de 32 en nivel 1 (flecha x = 32, ícono 50, nombre 76), ⏏ en x = 208.

Notas de X54–X67:
- La caja de búsqueda del panel pasa a ser la de la configuración (el spec dibuja un campo arriba del contenido): así no
  hay dos buscadores. La barra de favoritos del panel sigue. Decisión de diseño, corregible.
- En X56 hay tres filas grabando a la vez para mostrar los estados; en la app graba una sola.
- «Sistema» del Tema y de Animaciones solo se elige desde la pestaña; la rueda dentada sigue con su switch.
- Los conectados del árbol muestran ⏏ al pasar el mouse y en la fila actual (X65/X66), no siempre como los discos de X52.
- Las reglas de color de X58 tienen la muestra en su color y el patrón en `text_secondary` (para que se lean en claro).
- La rueda de X67 y el árbol de X65/X66 son copias desacopladas (como en X37 y X52).
- X59 es una hoja de contenido, no una ventana: ocupa el marco de 1400×820 con fondo `surface`.

### Lo nativo, que no se dibuja

- Vista rápida de macOS (`QLPreviewPanel`) y el contenido del visor de Windows (`IPreviewHandler`).
- Menús contextuales de la lista, del panel de favoritos, de las pestañas y del botón «Grupos» (menú del sistema).
- Menú de subcarpetas que abre la flecha entre dos tramos de la barra de dirección.
- Diálogos del sistema: elegir carpeta, credenciales de red (las ventanas de usuario y contraseña de Windows y macOS, también al conectar en X60–X63), propiedades, confirmaciones (`NSAlert` en macOS).
- Los controles de edición que se montan encima de la interfaz (barra de dirección, caja de búsqueda, renombrar) son nativos; en X08, X13 y X30 se dibujan con su aspecto aproximado.
- Barra de título y marco de la ventana (Mica en Windows 11), también los de la ventana de copia (X45–X47).
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

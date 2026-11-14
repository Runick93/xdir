# Ajustes 1 — diseño

Fecha: 2026-11-13. Aprobado por el usuario (seis cambios chicos pedidos después de usar la app).

## 1. Carpeta nueva y archivo nuevo en el menú contextual (Mac y Linux)

- Hoy: el menú del fondo de la carpeta no ofrece crear nada en Mac (en Windows lo trae el «Nuevo ▸» del Shell). Solo se crean con ⇧⌘N / ⌥⌘N o el botón del pie.
- Cambio: en el menú del fondo (`background_entries`, core/app/fileops.rs), «Carpeta nueva» (`Command::NewFolder`) y «Archivo de texto nuevo» (`Command::NewTextFile`), con su atajo a la derecha como el resto, en las plataformas cuyo menú no trae «Nuevo» del sistema (Mac y Linux; Windows no, para no duplicar). La condición sale de un dato del sistema (`SystemEnv`/servicio de shell), no de `cfg` en el núcleo.
- No aparecen donde no se puede crear: Papelera, dentro de un archivo comprimido, «Este equipo», Configuración, Novedades, carpetas virtuales.

## 2. Botón para volver de xtree a xdir

- Hoy: solo Esc o el botón «xtree» de la barra de estado.
- Cambio: botón «← xdir» a la izquierda de todo en la barra superior de xtree (antes del combo de discos; el resto se corre), con cartel de ayuda «Volver a xdir (Esc)». Hace lo mismo que cerrar xtree.

## 3. «Analizar al abrir xtree» (apagado de fábrica)

- Hoy: al abrir xtree por primera vez arranca el análisis en el acto.
- Cambio: opción `xtree_analizar_al_abrir` en `[opciones]` (bool, por defecto `false`), con switch «Analizar al abrir xtree» en Configuración ▸ xtree y en la búsqueda de la configuración. Apagada: al abrir xtree se muestra el último resultado si existe en esta ejecución; si no, la vista vacía con el cartel «Elegí un disco o una carpeta y tocá Analizar» (el combo ya muestra la raíz que corresponde según `xtree_abrir`). Encendida: como hoy.

## 4. Botones del pie del panel más grandes

- Hoy: carpeta nueva y vista miden 32×20 con el ícono a 10 pt (`GlyphsSmall`) en un pie de 26.
- Cambio: botones de 36×26 con el ícono en `Glyphs` (13–14 pt), pie de 30. Mismo orden y comportamiento; zonas de clic y carteles de ayuda acompañan. Revisar que nada que dependa de `FOOTER_H` (texto del pie, cálculos de alto de la lista, pruebas de pintado) quede corrido.

## 5. Tamaño total de lo elegido, carpetas incluidas

- Hoy: la barra de estado suma solo archivos; las carpetas cuentan en «N elementos» pero suman 0, y con solo carpetas no hay tamaño.
- Cambio: si la selección tiene carpetas, la barra muestra «N elementos seleccionados · calculando…» y después el total (archivos + contenido de las carpetas), calculado en segundo plano con `info::size::folder_size` (usando `xtree.size_of` cuando xtree ya analizó esa carpeta). Un cálculo por selección: se cancela (bandera compartida + generación) cuando cambia la selección, la carpeta o la pestaña; los resultados viejos se descartan. Se recuerda el tamaño de cada carpeta mientras no cambie (para no recalcular al sumar o quitar un elemento), y se invalida con el vigilante de la carpeta.
- Carpetas de red (`is_remote`): no se calcula solo; la barra muestra «· calcular tamaño» como enlace y al tocarlo calcula igual que en local (con el límite de tiempo por elemento de la D+). Lo mismo para carpetas solo en la nube: no se baja nada (mismo criterio `cloud_only` del panel de información).
- Si alguna carpeta no se pudo leer entera, el total lleva «al menos» delante. Nunca se lee el disco en el hilo de la ventana.

## 6. El árbol sigue al panel también con favoritos y Acceso rápido

- Hoy: `tree.reveal` se detiene si ya ve una fila con esa ruta, y la fila de Acceso rápido o del favorito cuenta; nunca despliega la ubicación real.
- Cambio: con «seguir al panel» encendido, al navegar (desde favoritos, Acceso rápido o lo que sea) el árbol despliega las carpetas hasta la ubicación real de la carpeta (bajo su disco o raíz) y la marca y desplaza a la vista. Las filas de Acceso rápido / favoritos no cuentan como «ya está». Si la carpeta no está bajo ninguna raíz del árbol, queda como hoy.

## Para todos

- Pantallas en FigCraft: barra de xtree con «← xdir» y el estado vacío (sin análisis), pie del panel con los botones nuevos, switch en Configuración ▸ xtree, menú del fondo en Mac con las dos entradas, barra de estado con «calculando…» y «calcular tamaño».
- CHANGELOG: una línea por punto en «Sin publicar».
- Wiki: «Funciones» y «Configuración» donde corresponda (lo hace el controlador al final).

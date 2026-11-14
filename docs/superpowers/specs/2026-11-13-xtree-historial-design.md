# Issue #58 · Historial de crecimiento de carpetas en xtree — diseño

Fecha: 2026-11-13. Aprobado por el usuario. Issue de Runick93.

## Por qué

xtree muestra cuánto pesa una carpeta ahora, pero no si creció de golpe. Caso real del issue: `target/` de este repo llegó a 21 GB sin que nadie lo notara.

## Qué guarda

- Al terminar un análisis **completo** de xtree (no cancelado ni con error de raíz), una **foto**: fecha y tamaño (el mismo que muestra xtree) de cada carpeta que esté en los **dos primeros niveles** bajo la raíz analizada **o** pese **100 MB o más**.
- Archivo por raíz analizada en la carpeta de configuración: `xtree-historial/<clave>.bin`, con la clave derivada de la raíz (disco o carpeta). Formato binario propio, versionado, compacto (rutas relativas a la raíz, deduplicadas entre fotos; tamaños en u64), escrito atómico. Si está roto o es de otra versión, se ignora (y se reemplaza al guardar la próxima foto), nunca rompe xtree.
- Consolidación: una foto por día los últimos 30 días, una por semana hasta un año, se descarta lo más viejo. Una foto nueva del mismo día reemplaza a la anterior.
- Al analizar una subcarpeta de una raíz con historial, se usan las fotos de esa raíz cuando cubren la carpeta (rutas relativas). Discos de red: no se guarda historial.
- Guardar no traba la ventana: se arma y se escribe en un hilo aparte al terminar el análisis.

## Qué muestra

En la tarjeta de la carpeta elegida en xtree (junto a lo que ya muestra):

- Selector **7 · 30 · 90 días** (recuerda la elección).
- **Cambio en el período**: «+14,2 GB en 7 días» (o «−1,1 GB», o «Sin cambios»), comparando con la foto más cercana al inicio del período.
- **Gráfico de línea** simple con un punto por foto del período (eje con las fechas extremas y el tamaño máximo), colores del tema.
- **Las 5 subcarpetas que más crecieron** en el período, con su cambio; clic entra en el mapa en esa carpeta.
- Sin datos: «Sin historial todavía: se arma cada vez que analizás»; con una sola foto: «Primera foto: 10/11».
- En la tabla del mapa, una flecha ▲ (color de advertencia) junto al tamaño de las carpetas que crecieron mucho en el período: al menos 100 MB **y** (al menos 1 GB **o** al menos 25 % de lo que pesaban al inicio del período), con el cambio en el cartel de ayuda.

## Configuración

- Configuración ▸ xtree: switch **«Guardar historial de tamaños»** (`xtree_historial`, encendido de fábrica) y botón **«Borrar historial»** (pregunta antes; borra la carpeta `xtree-historial`).

## Fuera de esto

- Análisis periódico en segundo plano (el historial se arma solo cuando el usuario analiza).
- Historial de discos de red.

## Para todos

- Pantallas en FigCraft (después de que se mergee #56/#57, que está editando el archivo): tarjeta con historial (con datos, una sola foto, sin historial), tabla con ▲, opción en Configuración.
- CHANGELOG en «Sin publicar». El PR menciona «Closes #58»; al mergear se cierra el issue con un comentario.

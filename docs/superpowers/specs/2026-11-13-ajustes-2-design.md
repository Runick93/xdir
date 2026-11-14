# Ajustes 2: dar señal cuando algo tarda — diseño

Fecha: 2026-11-13. Aprobado por el usuario.

## Por qué

La ventana nunca se traba (todo lo lento corre en segundo plano), pero en varios casos no se ve nada mientras tanto y parece que xdir no hizo caso: expulsar o desconectar, una carpeta local grande o en un disco lento, una conexión de ⌘K con el panel cerrado.

## Qué hace

1. **Expulsar y desconectar**: mientras dura, la barra de estado muestra «Expulsando «Pendrive»…» (o «Desconectando «srv»…») con el indicador girando; el ⏏ de ese disco o conexión (árbol, Este equipo, Configuración ▸ Red) gira y queda deshabilitado (un segundo clic no hace nada). Al terminar: «Se expulsó «Pendrive»» / «Se desconectó «srv»» como aviso, o el error de hoy. Incluye la espera por copias (`eject_after_jobs`): «Esperando a que terminen 2 operaciones para expulsar «Pendrive»…».
2. **Un solo «Cargando…» para cualquier carpeta**: si leer una carpeta (local, de red o comprimida) tarda más de 0,5 s, la lista muestra el mismo cuadro con el indicador y «Cargando…» (el de X70), con los elementos que ya llegaron debajo si la lectura es por partes. En red, si no responde, pasa a «no responde» como hoy (sin cambiar esa lógica).
3. **Indicador de actividad en la barra de estado**: un círculo girando chico a la derecha (antes de los botones de la barra) mientras haya trabajo en segundo plano de estos tipos: conexiones, expulsiones/desconexiones, tamaño de lo elegido, sumas y comparaciones, búsqueda en subcarpetas, lectura de carpetas que tarda, análisis de xtree (si xtree está cerrado), extracción a la temporal. Al pasar el mouse, cartel con la lista («Expulsando Pendrive · Calculando el tamaño de 3 carpetas»). Clic: si hay copias, abre la ventana de copia; si no, nada. No aparece por trabajos de menos de 0,3 s (sin parpadeo).
4. **El cursor no cambia**: nada de reloj de arena, la ventana no se traba.

## Arquitectura (decisión del controlador)

- Un registro de actividad en la `App` (lo que está en curso, con su texto y desde cuándo), alimentado por los lugares que ya lanzan trabajos (spawn_task, tamaños, hashes, conexiones, expulsar, carga de carpetas), sin consultar el disco para pintarlo. El dibujo del indicador reusa `spinner`.
- Redibujo con un temporizador solo mientras haya algo en curso (sin ticks con todo quieto).

## Fuera de esto

- Cambiar los tiempos de espera o la lógica de «no responde».

## Para todos

- Pantallas en FigCraft (cuando se mergee #56/#57, que está editando el archivo): barra de estado con el indicador y su cartel, ⏏ girando, «Expulsando…», lista con «Cargando…» para una carpeta local.
- CHANGELOG: líneas en «Sin publicar».

# Estado de xtree junto al botón y acceso desde la columna izquierda — diseño corto

## Intención

Que se pueda seguir trabajando en los paneles mientras xtree escanea y saber cómo va (o si terminó) sin
entrar a la vista de xtree; y abrir xtree también desde la columna izquierda. Propuesta aprobada por el
usuario tal cual («me parece perfecto, así lo quiero»).

## Qué hace

1. **Indicador junto al botón «xtree» de la barra de estado** (a la izquierda del texto «xtree»):
   - Escaneando un volumen entero: anillo de avance en el color de acento que se llena con el porcentaje,
     más el número («◔ 37 %  xtree»).
   - Escaneando una carpeta (sin porcentaje) o reescaneando con permisos: anillo con un arco que gira
     (avanza con los avisos del escaneo, cada 250 ms; sin repintado continuo extra).
   - Terminado: tilde verde (`ok` del tema) hasta que se abra la vista de xtree; al abrirla, vuelve al
     botón normal.
   - Terminado con carpetas sin permiso: ícono de aviso ámbar (`advertencia`) con la misma regla.
   - Sin escaneo (o después de abrir la vista): el botón como hoy.
   - Cartel (tooltip) del botón con el detalle: «Escaneando / · 37 % · 0:42», «Escaneando ~/Proyectos ·
     0:12», «Análisis completado en 1:23», «Análisis completado en 1:23 · 458 carpetas sin permiso».
2. **Entrada «xtree» en la columna izquierda (árbol, feature `arbol`)**, debajo de «Inicio» y antes del
   separador: ícono del mapa + «xtree» + el mismo indicador que la barra (anillo con %, giro, tilde o
   aviso). Clic: abre xtree sobre la carpeta del panel activo, igual que el botón de la barra; si la vista
   ya está abierta, la cierra (igual que el botón). Con el mismo cartel.
3. El botón de la barra se mantiene (sin el árbol no hay columna izquierda).

## Enfoques

Una sola forma razonable: el estado sale de `Xtree` (que ya sigue viva con su escaneo al cerrar la
vista): una función `Xtree::badge() -> XtreeBadge { Idle, Progress(u8), Spinning(u8 /* fase */),
Done, DoneWithIssues }` y un texto para el cartel; la barra de estado (`ui::draw_status`) y el árbol lo
dibujan con una sola función de dibujo compartida (`ui::draw_xtree_badge`). Repintado: el que ya provocan
los avisos del escaneo cada 250 ms.

## Diseño visual

Antes de programar se agregan a FigCraft (`docs/design/xdir.figcraft`): el componente
`ui::draw_xtree_badge` (variantes: progreso, giro, terminado, aviso) y la barra de estado con el
indicador; la columna izquierda con la entrada «xtree»; y las pantallas pendientes del cronómetro de xtree
(T01 escaneando con «Escaneando… 37 % · 0:42» y la barra de avance, T03 con «Análisis completado en
1:23») y el componente del cartel de ayuda (`app::tooltip`), que quedaron sin diseñar.

## Restricciones

- `xdir-core` sin dependencias ni `cfg` de sistema; tests con `install_fakes()`, `FakeHost`,
  `FakeDiskScan`, `RecordingPainter`; `cargo test` y `cargo check --all-targets` limpios en los 3
  sistemas; CI verde.
- Castellano; Conventional Commits sin atribución a Claude; PR propio contra `develop`; versión de Mac
  instalada al terminar.

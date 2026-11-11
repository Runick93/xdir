# Tooltips en los botones sin texto — diseño corto

## Intención

Que los botones que son solo un ícono (layouts y rueda dentada de la barra de estado, botones del pie del
panel, navegación, pestañas…) expliquen qué hacen al dejar el mouse encima, como en el Explorador. Pedido
del usuario con capturas de la barra de estado y del pie del panel.

## Qué hace

- Al quedarse el mouse quieto 500 ms sobre un botón sin texto aparece un cartelito con qué hace y, si tiene,
  su atajo entre paréntesis según la tabla de atajos del sistema (`combo_label`): «Atrás (Alt+Izquierda)»,
  en la Mac «Atrás (⌘[)». Se va al mover el mouse fuera, al hacer clic, al apretar una tecla o al abrir un
  menú. Pasar a otro botón con el cartel ya visible lo cambia enseguida (sin volver a esperar).
- Dibujado por xdir (no un tooltip del sistema), en las 3 plataformas igual: fondo `surface_alt`, borde
  `border`, radio 4, texto `FontId::Small` en `text`, 6 px de margen; debajo del cursor (o arriba si no
  entra) y siempre dentro de la ventana.
- Botones y textos (los de comandos usan el título del comando y su atajo):
  - Barra de estado: los 5 layouts («Layout: 1 panel», …, «Layout: 4 paneles», con Ctrl+1…5), la rueda
    dentada «Apariencia y opciones».
  - Pestañas: «Pestaña nueva», «Cerrar pestaña», «Maximizar o restaurar el panel»; y en cada pestaña, la
    ruta completa de su carpeta (útil cuando el título está recortado).
  - Barra del panel: «Atrás», «Adelante», «Subir a la carpeta superior», «Recargar», «Agregar o quitar de
    favoritos» (estrella), «Abrir una consola en la carpeta», «Borrar» (cruz de la barra de dirección),
    «Terminar la búsqueda» (cruz del buscador), «Más favoritos» («»»).
  - Pie del panel: «Carpeta nueva», «Cambiar la vista».
  - Barra de grupos: «Guardar los paneles como grupo» (botón «+»).
  - Árbol (feature `arbol`): el botón de fijar/soltar.
- Los botones que ya tienen texto («xtree», «Grupos») no llevan cartel.

## Enfoques

Una sola forma razonable: el núcleo ya sabe qué hay bajo el mouse (`Hit`); una función pura
`tooltip_for(hit, keymap, …) -> Option<String>` arma el texto, la `App` lleva el estado (qué, desde cuándo,
visible) y dibuja el cartel al final del cuadro; la espera de 500 ms usa el mismo patrón de hilo + aviso
que la pausa de la búsqueda. Se descartó el tooltip nativo de cada sistema (tres implementaciones, no
encaja con los botones dibujados por xdir).

## Restricciones

- `xdir-core` sin dependencias ni `cfg` de sistema; tests con `install_fakes()`, `FakeHost` y
  `RecordingPainter`; `cargo test` y `cargo check --all-targets` limpios en los 3 sistemas; CI verde.
- Sin repintado continuo: solo un aviso al cumplirse la espera.
- FigCraft: el componente del cartel se agrega después de mergear la tanda B (el archivo de diseño es
  binario y la tanda B lo está cambiando).
- Castellano; Conventional Commits sin atribución a Claude; PR propio contra `develop`.

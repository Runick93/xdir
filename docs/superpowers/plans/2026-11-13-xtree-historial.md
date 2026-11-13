# Historial de crecimiento en xtree (#58) — plan de implementación

**Spec:** `docs/superpowers/specs/2026-11-13-xtree-historial-design.md`.

Cada paso va con su prueba primero (falla), después el cambio, y un commit propio. Nada de `cfg` de
sistema en `xdir-core` ni dependencias nuevas; las rutas de las pruebas con `Path::join`; las pruebas
con tiempos esperan una condición. El día de hoy se inyecta (número de días desde 1970 en hora
local): las pruebas nunca dependen de la fecha en que corren. Las pantallas de FigCraft y
`docs/DISENO.md` van después (otra rama edita esos archivos).

## Pasos

### H1 — Foto de un análisis (`xtree/history.rs`)
- `History { root, paths, snaps }`: tabla de rutas relativas (tramos unidos con `/`, `""` es la
  raíz) y fotos `Snapshot { day, sizes: Vec<(u32, u64)> }` ordenadas por día.
- `select(arena) -> Vec<(String, u64)>`: la raíz, las carpetas de los dos primeros niveles y las de
  100 MB o más (nunca archivos ni el grupo de archivos chicos), con tope de 10.000 (las más pesadas).

### H2 — Formato binario
- `encode`/`decode`: `XDHT`, versión `u16`, raíz, tabla de rutas y fotos con enteros LEB128 (índice
  de ruta y tamaño). Lectura con límites (archivo ≤ 32 MB, rutas ≤ 4 KB, cantidades acotadas por lo
  que queda del archivo, índices dentro de la tabla, días en orden): ante cualquier cosa rara
  `None`. Pruebas con cortes en cada byte y basura pseudoaleatoria: nunca pánico.
- `save_in(dir)` atómico (temporal propio + renombrar), `load_in(dir, root)`.

### H3 — Consolidación
- `add(day, sizes)` reemplaza la del mismo día; `consolidate(today)`: una por día los últimos 30
  días, una por semana (la última de cada semana) hasta 365, se descarta lo más viejo; la tabla de
  rutas se poda al escribir.

### H4 — Clave por raíz y subcarpetas
- `key(root)`: nombre legible + FNV-1a de la ruta normalizada (sin separador final, minúsculas).
  `load_for(dir, root)`: el archivo de la raíz más los de las carpetas de arriba que la cubran,
  recortados a la subcarpeta y unidos por día (gana el propio).

### H5 — Guardar después del análisis
- `Xtree` arma la foto y la guarda en un hilo al tomar un árbol terminado (no cancelado), salvo
  discos de red o con el historial apagado; el hilo devuelve el historial unido para la tarjeta.
  Auditoría: leer y escribir el historial nunca pasa en el hilo de interfaz (prueba con el hilo
  marcado).

### H6 — Tarjeta
- La tarjeta crece con una franja de historial (si entra): selector 7 · 30 · 90 días, cambio en el
  período, gráfico de línea y las 5 subcarpetas que más crecieron (clic entra en el mapa). Estados
  sin historial, una sola foto, carpeta sin datos y archivo. El período se recuerda en la sesión
  (`xtree_dias`).

### H7 — ▲ en la tabla
- `grew_a_lot(base, now)`: al menos 100 MB y (al menos 1 GB o al menos 25 % de lo que pesaba).
  Flecha con el color de advertencia junto al tamaño y cartel con el cambio.

### H8 — Configuración y CHANGELOG
- `[opciones] xtree_historial` (de fábrica, sí) con su switch «Guardar historial de tamaños» y el
  botón «Borrar historial» (pregunta; borra `xtree-historial` en un hilo). Línea en «Sin publicar».

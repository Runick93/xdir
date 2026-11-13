# Issue #59 · Comparar archivos y suma de verificación — diseño

Fecha: 2026-11-13. Aprobado por el usuario. Issue de Runick93.

## Qué hace

### Comparar contenido

- Comando **«Comparar contenido»** (paleta, menú contextual de archivo, id `comparar_contenido`, sin atajo de fábrica, grupo Archivos). Disponible con exactamente dos archivos (no carpetas) elegidos entre los dos paneles visibles: uno elegido en el panel activo y uno en el otro panel, o dos en el mismo panel.
- Si los tamaños son distintos: responde «Distintos (tamaños distintos)» sin leer nada.
- Si no, compara **byte a byte** en segundo plano (bloques de 1 MB, lectura de los dos a la vez), con avance y Cancelar, cortando en el primer byte distinto. No calcula hash para esto.
- Resultado en un panel chico propio (como el de información, a la derecha, o una ventana del estilo de la de copia; elegir lo que encaje con DISENO): nombres y rutas de los dos, tamaño, «Iguales» (verde `ok`) o «Distintos» (con la posición del primer byte distinto), y botones «Calcular SHA-256» de cada uno.
- Nunca lee en el hilo de la ventana; un servidor de red colgado no traba (límite por falta de avance, como en la D+). Archivos dentro de un comprimido: se extraen a la temporal de la app (con sus límites) y se comparan desde ahí.

### Suma de verificación

- En el **panel de información** (⌘I / Alt+Enter) de un archivo, sección **«Suma de verificación»** plegada de fábrica: selector SHA-256 / MD5 y botón «Calcular». Muestra el valor en hexadecimal, en minúscula, con «Copiar».
- Caja «Comparar con…»: pegar una suma publicada (se ignoran espacios y mayúsculas; acepta formato `hash  nombre`) y dice «Coincide» o «No coincide»; detecta el algoritmo por el largo (64 = SHA-256, 32 = MD5).
- Se calcula en segundo plano con avance y cancelar; se cancela al cambiar de archivo o cerrar el panel. Archivos en red y comprimidos, igual que arriba.

## Arquitectura (decisión del controlador, aprobada)

- Crate nueva `xdir-hash` con las dependencias `sha2` y `md-5` (RustCrypto, sin features de más), como `xdir-archive`: solo la usan las crates de cada sistema, a través de un servicio `sys::Hashes` (trait en xdir-core con fake en `sys::testing`). xdir-core sigue sin dependencias.
- La comparación byte a byte va en el núcleo con las primitivas de lectura que ya hay (o en el servicio, si leer necesita cosas del sistema).
- Pantallas en FigCraft: panel de comparar (iguales, distintos, comparando), sección del panel de información con el hash y «Comparar con…».
- CHANGELOG: línea en «Sin publicar» (Nuevo). El PR menciona «Closes #59».

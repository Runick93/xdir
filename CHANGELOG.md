# Novedades de xdir

## Sin publicar

### Nuevo
- **xdir para macOS**: la app ahora también anda en la Mac (Apple Silicon e Intel), con los atajos de ⌘ y las costumbres del Finder.
- **xtree dentro de xdir**: el mapa de uso de disco se abre en la misma ventana, también en macOS.
  - Muestra el porcentaje y el tiempo del escaneo («Escaneando… 37 % · 0:42», «Análisis completado en 1:23»).
  - Mientras escanea podés seguir trabajando: el botón «xtree» de la barra de estado y el árbol muestran cómo va.
- **Organizar la lista**
  - Cada carpeta recuerda su vista, orden, columnas y filtros; «Usar esta vista en todas las carpetas» y «Restablecer la vista de esta carpeta» en el menú «Ver».
  - Agrupar por nombre, fecha, tipo, tamaño, dimensiones o duración, con grupos que se pliegan.
  - Filtros por columna desde el encabezado.
  - Vistas **Lista**, **Contenido** y **Galería**.
  - Casillas de selección (switch en la rueda dentada).
  - Columnas Dimensiones, Duración, Autor, Etiquetas, Artista y Álbum.
  - Seleccionar y deseleccionar por patrón (`Ctrl+Más` / `⌘+`), y la barra de estado suma lo que pesa la selección.
- **Ver sin abrir**
  - Columna de vista previa con la imagen o el texto y los detalles del archivo (`Alt+P` / `⌥⌘P`).
  - Vista rápida con la barra espaciadora: en Windows con los visores del Explorador, en la Mac con QuickLook.
  - Panel de información (`Alt+Enter` / `⌘I`): etiquetas, tamaño de carpetas, Abrir con, comentarios y permisos.
- **Operaciones con archivos**
  - Ventana de copia propia: cola, pausa, gráfico de velocidad, archivos que ya existen (Reemplazar, Conservar ambos, Omitir, Combinar) y errores con Reintentar.
  - **Duplicar** (`Ctrl+D` / `⌘D`) y **Carpeta nueva con la selección** (`Ctrl+Alt+Shift+N` / `⌃⌘N`).
  - **Renombrar varios** con vista previa en la lista (`F2` con varios seleccionados).
  - **Comprimir** en zip, **Extraer aquí** y **Extraer en una carpeta** (tar.gz también, en la Mac).
  - **Devolver** desde la Papelera, **Crear acceso directo**, **Mostrar el original** y **Expulsar** discos.
  - Archivo de texto nuevo (`Ctrl+Alt+N` / `⌥⌘N`) y «Copiar como ruta de acceso».
- **Archivos comprimidos**: un zip o tar.gz se abre como una carpeta de solo lectura; lo de adentro se abre, se previsualiza y se copia a otro panel.
- **Configuración**: pestaña propia (`Ctrl+,` / `⌘,` o «Más opciones…» de la rueda dentada) con buscador y editor de atajos. Lo que cambiás se guarda en `config.toml` sin tocar lo que escribiste a mano.
- **Red**
  - «Conectar a un servidor…» (`Ctrl+K` / `⌘K`) con favoritos y recientes.
  - «Red» en el árbol: favoritos, lo conectado y los equipos de la red local.
  - Desconectar con ⏏ y, en Windows, conectar y desconectar unidades de red.
- **Novedades**: esta pestaña, desde la rueda dentada, «Acerca de xdir» o la paleta. Después de actualizar, la barra de estado te avisa.
- Carteles de ayuda en los botones que son solo un ícono.
- Historial de Atrás y Adelante con clic derecho sobre el botón.
- Más consolas: Windows Terminal e iTerm, si están instalados.
- En la Mac, «Carpeta nueva» y «Archivo de texto nuevo» en el menú del fondo de la carpeta, con su atajo a la derecha.
- Botón «← xdir» en la barra de xtree para volver a los paneles (lo mismo que Esc).
- La barra de estado suma también lo que pesan las carpetas elegidas: «calculando…» y después el total, sin trabar la ventana. En carpetas de red se calcula al tocar «calcular tamaño».

### Cambios
- Favoritos pasa a `Ctrl+Shift+D` / `⇧⌘D`: `Ctrl+D` ahora duplica.
- En la Mac, las vistas van de `⌥⌘1` a `⌥⌘7` (`⇧⌘3`, `⇧⌘4` y `⇧⌘5` son las capturas del sistema).
- La rueda dentada suma «Mostrar extensiones», «Casillas de selección», «Mostrar archivos ocultos» y «El árbol sigue al panel»; abajo quedan Novedades, Más opciones… y Acerca de xdir.
- El árbol sigue a la carpeta del panel activo.
- Las carpetas de red lentas ya no traban la ventana: la lista muestra «Conectando…» o «no responde».
- El buscador también filtra en la Papelera y en otras carpetas virtuales.
- xtree ya no es un programa aparte.
- xtree ya no analiza apenas se abre: elegís el disco y tocás «Analizar». «Analizar al abrir xtree» (Configuración ▸ xtree) lo deja como antes.
- Los botones del pie del panel (carpeta nueva y vista) son más grandes.

### Arreglos
- Los atajos con Alt y Ctrl juntos (en la Mac, ⌥ y ⌘) no andaban.
- En la Mac, el tipo de archivo siempre empieza con mayúscula.
- Con «El árbol sigue al panel», ir a una carpeta desde los favoritos o el Acceso rápido también despliega su ubicación real en el árbol.
- **Seguridad**
  - Extraer un comprimido nunca escribe fuera de la carpeta elegida ni deja enlaces que apunten afuera, y lo extraído conserva la marca de «descargado de internet».
  - Devolver desde la Papelera nunca escribe fuera del disco ni sigue enlaces, y una Papelera con datos armados a propósito no traba xdir.
  - Copiar, mover y deshacer nunca pisan un archivo que ya existe, y un corte no deja archivos a medias que parezcan completos.
  - La carpeta temporal de los archivos comprimidos es privada de tu usuario y se borra al cerrar.
  - xdir no guarda contraseñas de red: las guarda el sistema.

## 0.1.23 — 2026-11-08
- xtree, el mapa de uso de disco (con lectura rápida de la MFT), viene con xdir y se abre desde la barra de estado.
- El grupo Acceso rápido del árbol se puede plegar y recuerda cómo quedó.

## 0.1.22 — 2026-11-04
- El árbol ya no muestra el encabezado de Acceso rápido.
- El título de la ventana muestra el usuario.

## 0.1.21 — 2026-11-03
- Barra lateral con el Acceso rápido del Explorador; suelta, queda una franja de íconos que se abre al pasar el mouse.

## 0.1.20 — 2026-11-02
- Los discos que se enchufan o se quitan aparecen y desaparecen solos.
- Se quitó la actualización automática.
- Vigilar una carpeta ya no impide borrarla.

## 0.1.19 — 2026-10-29
- Barra lateral como la del Explorador: Inicio, Galería, OneDrive, Acceso rápido y el nombre de cada unidad.

## 0.1.18 — 2026-10-29
- Botón de carpeta nueva en el pie del panel y lugar libre al final de la lista.
- Los zip se abren como carpetas del Explorador.

## 0.1.17 — 2026-10-29
- Al prender la barra de grupos aparece un grupo temporal con los paneles de ahora.
- Abrir una carpeta de red desde Favoritos pide las credenciales.

## 0.1.16 — 2026-10-29
- De fábrica: árbol fijo y a la vista, sin barra de grupos ni colores por tipo.

## 0.1.15 — 2026-10-29
- Barra de pestañas de grupos, chincheta para fijar el árbol y botón de vista en cada panel.
- Red en el árbol.

## 0.1.14 — 2026-10-28
- «Nuevo › Carpeta» del menú contextual deja el nombre listo para escribir.

## 0.1.13 — 2026-10-28
- Botón para actualizar en «Acerca de xdir», también en la versión portable.

## 0.1.12 — 2026-10-28
- Ícono propio para el Escritorio en el árbol.

## 0.1.11 — 2026-10-28
- Aviso de versión nueva y actualización automática.

## 0.1.10 — 2026-10-28
- Pestañas flotantes, árbol de carpetas nuevo y menú de subcarpetas en la barra de dirección.
- Pegar archivos copiados en un Escritorio remoto.

## 0.1.9 — 2026-10-28
- Barra de pestañas, barra de dirección por tramos, columnas e ícono nuevos.

## 0.1.8 — 2026-10-28
- La barra de dirección recuerda las rutas escritas.

## 0.1.7 — 2026-10-28
- «Acerca de xdir» en la rueda dentada, con la versión y para reportar un problema.
- El título de la ventana muestra el grupo cargado.

## 0.1.6 — 2026-10-27
- Menú de consolas: clic derecho para abrir como administrador o como otro usuario.
- Menú del fondo de la lista con Ver, Ordenar por, Actualizar, Pegar y Deshacer.

## 0.1.5 — 2026-10-26
- El rectángulo de selección arranca desde cualquier lugar de la fila, y un clic en un hueco limpia la selección.

## 0.1.4 — 2026-10-26
- Selección con un rectángulo del mouse.

## 0.1.3 — 2026-10-26
- Las rutas de red escritas en la barra piden credenciales y muestran el error en vez de solo pitar.

## 0.1.2 — 2026-10-26
- Árbol de carpetas incluido (oculto hasta prenderlo) y colores por tipo de fábrica.

## 0.1.1 — 2026-10-26
- Primera versión: hasta cuatro paneles con pestañas, favoritos, grupos, salto rápido y paleta de comandos, vistas de íconos y miniaturas, colores por tipo, atajos en `config.toml` e instalador.

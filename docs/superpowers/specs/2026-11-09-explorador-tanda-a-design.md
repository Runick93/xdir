# Funciones del Explorador, tanda A — diseño corto

## Intención

Que xdir tenga las funciones del Explorador de Windows que le faltan, en las 3 plataformas (Windows, macOS
y Linux), en tandas. La tanda A junta las chicas y sin decisiones de diseño, para que se noten enseguida.
Decisión del usuario: las funciones son generales para las 3 plataformas; lo que vive en el núcleo
(`xdir-core`) sirve para las tres, y lo que depende del sistema se implementa en cada crate de
plataforma (en Linux, sin interfaz todavía, quedan los servicios listos).

Lista completa y orden (acordado con el usuario): A cosas rápidas → B organizar la lista (agrupar,
filtros por columna, más columnas, vistas Lista y Contenido, casillas) → C paneles laterales (vista
previa, detalles) → D operaciones (zip, renombrado de varios, accesos directos, diálogo propio de copiar y
mover, expulsar volúmenes) → E árbol y red (favoritos en el árbol, red, Inicio con recientes, ventana
nueva). Cada tanda tiene su diseño, plan y PR.

## Qué entra en la tanda A (aprobado por el usuario)

| # | Función | Comportamiento |
|---|---|---|
| 16 | Tamaño de la selección | Con algo seleccionado, la barra de estado dice «N elementos seleccionados · 4,2 MB» (1 elemento: «1 elemento seleccionado · …»). Las carpetas no suman (como el Explorador); si solo hay carpetas, no se muestra tamaño. Sin selección, el texto de hoy. |
| 17 | Selección por patrón | Comando «Seleccionar por patrón…» (`Ctrl+Más`) y «Deseleccionar por patrón…» (`Ctrl+Menos`): la paleta se abre en un modo de texto; se escribe `*.jpg` o varios separados por `;` (`*.jpg;*.png`), sin distinguir mayúsculas; Enter suma (o quita) a la selección los elementos visibles cuyo nombre coincide. Usa el `wildcard` del núcleo. En macOS `⌘+` / `⌘-` según la tabla de atajos de macOS. |
| 18 | Copiar como ruta de acceso | Entrada propia en el menú contextual: copia las rutas de la selección entre comillas dobles, una por línea (como «Copiar como ruta de acceso» del Explorador). El comando actual «Copiar las rutas como texto» no cambia (sin comillas). |
| 19 | Mostrar extensiones | Switch en la rueda dentada, prendido por defecto (comportamiento de hoy). Apagado: la columna Nombre (y los nombres en íconos/miniaturas) muestran el nombre sin la extensión de los archivos (las carpetas, completas); el renombrado en línea edita el nombre completo. Se guarda en la sesión. |
| 20 | Más consolas | Windows Terminal en Windows (si `wt.exe` está en el PATH o en `%LOCALAPPDATA%\Microsoft\WindowsApps`), iTerm en macOS (si está instalada), y la terminal por defecto en Linux (`x-terminal-emulator`, si no `gnome-terminal`, `konsole`, `xterm`, la primera que exista). Aparecen en el menú de consolas junto a las de hoy. |
| 9 | Archivo de texto nuevo | Comando «Archivo de texto nuevo» (`Ctrl+Alt+N`): crea «Nuevo documento de texto.txt» (con « (2)», « (3)»… si existe) vacío en la carpeta del panel, lo enfoca y abre el renombrado en línea, como «Carpeta nueva»; se deshace con Ctrl+Z (a la papelera). Operación nueva del núcleo (`Op::NewFile`) implementada en las 3 plataformas. |
| 12 | Historial de Atrás y Adelante | Clic derecho sobre el botón Atrás o Adelante: menú con hasta 15 carpetas de esa pestaña (la más cercana primero); elegir una va directo ahí y deja el historial como si se hubiera apretado Atrás/Adelante varias veces. |
| 24 | El árbol sigue al panel | Cuando el panel activo cambia de carpeta (navegar, cambiar de pestaña o de panel), el árbol despliega sus carpetas padre y se desplaza para mostrarla. Prendido por defecto; `arbol_sigue_panel = false` en `config.toml` lo apaga. Solo con la feature `arbol`. |

## Enfoques

Una sola forma razonable por función: todo lo de interfaz y lógica en el núcleo (lista, paleta, barra de
estado, árbol, comandos, sesión, configuración), y en cada plataforma solo lo que el sistema decide
(crear el archivo en `FileOps`, la lista de consolas en `SystemEnv`). Se descartó implementar el
historial o el archivo nuevo por plataforma (duplicaría lógica) y agregar pantallas a FigCraft para esta
tanda (solo suma textos y menús a pantallas existentes).

## No objetivos

- Cambiar el comportamiento de «Copiar las rutas como texto», del menú contextual del sistema o de
  «Carpeta nueva».
- Interfaz de Linux.
- Que el árbol se mueva cuando está apagado o con la feature `arbol` desactivada.

## Restricciones

- Las de la migración: `xdir-core` sin dependencias ni `cfg` de sistema en producción; tests con
  `install_fakes()` y `FakeHost`; `cargo test` y `cargo check --all-targets` limpios en Windows, macOS y
  Linux; CI verde (incluida la prueba de humo de Windows).
- Castellano; Conventional Commits sin atribución a Claude; PR contra `develop`.

## A criterio de quien implementa

- Nombres internos, dónde se guarda el switch de extensiones en la sesión, el texto exacto de los
  comandos en la paleta (en la línea de los existentes).
- Cómo detectar iTerm (`/Applications/iTerm.app` o por bundle id) y Windows Terminal.

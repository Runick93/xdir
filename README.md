# xdir

Explorador de archivos multipanel para Windows, nativo y sin frameworks: Rust + Win32 + Direct2D.
Requerimientos y hitos en [docs/requerimientos.md](docs/requerimientos.md).

## Compilar

Requiere Rust (`rustup`) y Visual Studio Build Tools con el Windows SDK.

```bat
scripts\build.cmd                  :: cargo build --release
scripts\build.cmd run --release    :: compilar y ejecutar
```

El script carga el entorno de Build Tools: la instalación de Visual Studio Community de esta
máquina no trae las librerías de escritorio x64 (`libcmt.lib`).

El ejecutable queda en `target\release\xdir.exe`: un solo archivo, sin runtimes.

## Publicar una release

En GitHub, ejecuta **Actions → Release → Run workflow**. El runner compila, genera el ZIP
portable y el instalador, y los publica como adjuntos de una release en **Releases**. También
quedan disponibles como artifact de esa ejecución. El workflow pregunta qué número subir
(`patch` por defecto: 0.1.6 → 0.1.7; también `minor`, `major` o `ninguno` para republicar la
actual), actualiza `Cargo.toml` y `Cargo.lock`, deja el commit «Versión X» en la rama y publica
con el tag `v<versión>`. Solo se conservan los dos releases más recientes.

## Reportar errores y sugerencias

Prueba la última versión desde [Releases](https://github.com/Runick93/xdir/releases) y, si algo falla o te falta, abre un [issue](https://github.com/Runick93/xdir/issues/new/choose): hay un formulario para errores y otro para sugerencias.

## Uso

`xdir.exe [carpeta]` abre esa carpeta en una pestaña nueva. La primera vez, los cuatro paneles arrancan en "Este equipo"; después se restaura la sesión anterior.

| Atajo | Acción |
|---|---|
| Enter / doble clic | Abrir carpeta o archivo |
| Backspace, Alt+↑ | Subir |
| Alt+← / Alt+→, botones laterales del mouse | Atrás / adelante |
| Ctrl+P | Salto rápido: ir a una carpeta escribiendo parte de su ruta (historial por frecuencia y recencia, favoritos o una ruta completa). Ctrl+Enter la abre en pestaña nueva |
| Ctrl+Shift+P | Paleta de comandos: buscar y ejecutar cualquier acción; también busca favoritos y carpetas recientes |
| Ctrl+L, F4, Alt+D, clic en la barra | Escribir una ruta (con autocompletado; acepta `%VARIABLES%`) |
| F5 | Recargar |
| Ctrl+H | Mostrar u ocultar archivos ocultos |
| Ctrl+A / Ctrl+I | Seleccionar todo / invertir selección |
| Flechas, Re Pág, Av Pág, Inicio, Fin | Moverse (con Shift extiende la selección, con Ctrl solo mueve el foco) |
| Arrastrar sobre un hueco de la lista | Selección con rectángulo (con Ctrl suma a lo ya seleccionado; si te salís por arriba o abajo, la lista se desplaza) |
| Letras | Saltar al primer nombre que empieza así |
| Ctrl+F, embudo de la barra de dirección | Filtro rápido: texto (contiene), comodines `*` `?` (`*.pdf`), varios separados por `;`. Enter lo deja aplicado, Esc lo quita. Se limpia al cambiar de carpeta |
| Clic en un encabezado | Ordenar por esa columna (otro clic invierte); arrastrarlo lo reordena, arrastrar su borde cambia el ancho, clic derecho elige columnas |
| Ctrl+Shift+1..4, Ctrl+rueda | Vista: detalles, íconos medianos, íconos grandes, miniaturas |
| Tab / Shift+Tab, Alt+1..4 | Panel siguiente / anterior / panel N |
| Ctrl+1..5, botones abajo a la derecha | Layout: 1, 2 lado a lado, 2 apilados, 3, 4 paneles |
| Ctrl+M, botón a la derecha de las pestañas, doble clic en la tira de pestañas | Maximizar el panel activo o volver al layout (cambiar de panel también lo restaura) |
| Ctrl+T, botón + | Pestaña nueva en la misma carpeta |
| Ctrl+W, clic medio en la pestaña | Cerrar pestaña |
| Ctrl+Shift+T | Reabrir la última pestaña cerrada |
| Ctrl+Tab / Ctrl+Shift+Tab | Pestaña siguiente / anterior |
| Arrastrar una pestaña | Reordenarla en su panel, o soltarla en otro panel para llevarla ahí (con su historial) |
| Clic medio en una carpeta | Abrirla en una pestaña nueva, en segundo plano |
| Arrastrar un divisor (doble clic lo centra) | Cambiar el tamaño de los paneles |
| Ctrl+C / Ctrl+X / Ctrl+V | Copiar / cortar / pegar (compatible con el Explorador) |
| Ctrl+Shift+C | Copiar las rutas como texto |
| Supr / Shift+Supr | Enviar a la papelera / eliminar definitivamente |
| F2 | Renombrar en línea |
| Ctrl+Shift+N | Carpeta nueva (se abre para renombrar) |
| Ctrl+Z | Deshacer (renombrar, copiar, mover, papelera, carpeta nueva) |
| Alt+Enter | Propiedades |
| Clic derecho, tecla de menú, Shift+F10 | Menú contextual del Shell, con sus extensiones |
| Arrastrar archivos | Mover (misma unidad) o copiar (otra unidad); Ctrl copia, Shift mueve |
| Ctrl+D, estrella de la barra de dirección | Agregar o quitar la carpeta de favoritos |
| Ctrl+Shift+B | Mostrar u ocultar la barra de favoritos |
| Clic / clic medio en un favorito | Abrir en el panel / en pestaña nueva |
| Botón `>_` de la barra de dirección | Abrir PowerShell o CMD en la carpeta del panel |

**Barra de favoritos:** debajo de la navegación de cada panel, como en Chrome. Los favoritos
pueden ir sueltos o en carpetas de favoritos (anidables, se despliegan como menú). Se agregan
con la estrella, con Ctrl+D o arrastrando carpetas a la barra; se reordenan arrastrándolos, y
soltándolos en el centro de una carpeta de favoritos van adentro. Con clic derecho: renombrar,
eliminar, sacar de la carpeta, nueva carpeta. Se guardan en `favoritos.txt`, junto a la sesión,
en un formato editable a mano (`nombre = ruta`; las carpetas terminan en `:` y su contenido va
indentado con dos espacios).

Las operaciones usan `IFileOperation`, así que el progreso, los conflictos y las confirmaciones
son los del Explorador. Pegar y soltar funcionan con archivos de disco; los elementos virtuales
(contenido de un ZIP, de un teléfono) todavía no.

La carpeta se actualiza sola cuando cambia en disco. Al cerrar se guarda la sesión (layout,
divisores, pestañas de cada panel y posición de la ventana) en `%APPDATA%\xdir\sesion.toml`;
si junto al `.exe` hay un archivo `xdir.portable`, se guarda ahí (modo portable).

**Salto rápido:** cada carpeta que se abre (navegando, atrás, adelante o subiendo; recargar no
cuenta) se anota en `historial.txt`, junto a la sesión, con la cantidad de visitas y la fecha de
la última. Ctrl+P ordena por esas dos cosas: lo usado seguido y hace poco aparece primero. Las
carpetas que ya no existen salen del historial al intentar abrirlas.

## Animaciones y accesibilidad

- **Animaciones sutiles:** el scroll de la lista llega suave a su destino y los paneles se
  desplazan al cambiar de layout, maximizar o centrar un divisor. Nunca demoran nada: el estado
  cambia al instante (los clics y las teclas ya actúan sobre el destino) y solo se dibuja de a
  poco, unos 160 ms. Siguen la preferencia de Windows (Accesibilidad → Efectos visuales →
  Mostrar animaciones); `[opciones] animaciones = true` o `false` en `config.toml` la fuerza.
- **Lectores de pantalla (UI Automation):** Narrator, NVDA y JAWS ven la ventana como paneles
  con su ruta y su lista de archivos; cada elemento tiene nombre, tipo (carpeta o archivo),
  tamaño y fecha, se puede seleccionar y abrir, y al moverse con las flechas se anuncia el
  elemento con foco. La barra de estado también se lee. Todo se maneja con el teclado (NF-8).

## Vistas y columnas

- **Vistas:** detalles, íconos medianos, íconos grandes y miniaturas (Ctrl+Shift+1..4, Ctrl+rueda o
  la paleta). Cada panel recuerda la suya. Las miniaturas son las del Shell (imágenes, videos,
  PDF) y se piden solo para lo que está a la vista, en un hilo aparte.
- **Columnas:** clic derecho en el encabezado para mostrar u ocultar (fecha de creación,
  extensión, atributos…). Arrastrar un encabezado las reordena; arrastrar su borde cambia el
  ancho; un clic ordena. Valen para todas las pestañas y se guardan en la sesión. En las vistas
  de cuadrícula se ordena desde la paleta ("Ordenar por…").

### Árbol de carpetas (experimental)

Un panel lateral con la carpeta del usuario y las unidades, que navega el panel activo. Arranca
oculto: se muestra u oculta con el switch "Árbol de carpetas" de la rueda dentada, con
Ctrl+Shift+E o con "árbol" en la paleta, y la elección se guarda en la sesión. Junto a él, el
switch "Colores por tipo" apaga los colores por tipo de archivo sin tocar `[colores]`.

Es una función experimental y se puede quitar del programa al compilar (no queda nada del árbol,
ni el switch ni el atajo):

```bat
scriptsuild.cmd build --release --no-default-features
```

## Configuración

`config.toml`, junto a la sesión (`%APPDATA%\xdir\` o junto al exe en modo portable), se crea la
primera vez como plantilla comentada con todo lo que se puede cambiar. xdir no lo reescribe nunca:
lo vuelve a leer cuando cambia, al volver a la ventana, así que los cambios se aplican sin
reiniciar. Si algo está mal escrito, la barra de estado avisa en qué línea. Se abre desde la
paleta: "Abrir la configuración".

```toml
[atajos]
pestana_nueva = "Ctrl+K"            # reemplaza Ctrl+T
escribir_ruta = ["Ctrl+L", "F6"]    # varios atajos
este_equipo   = "Ctrl+E"            # acciones sin atajo de fábrica también
deshacer      = ""                  # sin atajo

[colores]
"*.pdf"                    = "#E5534B"
"*.jpg;*.jpeg;*.png"       = "#57AB5A"
"node_modules"             = "#768390"   # nombre exacto
"@oculto"                  = "#768390"   # por atributo: @carpeta, @oculto, @sistema,
                                         # @solo_lectura, @comprimido, @cifrado
```

Los colores pintan el nombre en la lista; gana la primera regla que coincide. Para rutas, las
comillas simples evitan escapar las barras: `'C:\carpeta'`.

**Grupos guardados:** el botón "Grupos" de la barra de estado guarda el layout, los divisores y
las pestañas de cada panel con un nombre, y los vuelve a abrir con un clic (también desde Ctrl+P
o la paleta, escribiendo el nombre). Al abrir un grupo, las pestañas que había quedan entre las
cerradas (Ctrl+Shift+T). Se guardan en `grupos.toml`, que también se puede editar a mano.

## Medir el arranque

```bat
target\release\xdir.exe C:\alguna\carpeta --probe arranque.txt
```

Abre la ventana, espera al primer frame con la carpeta ya leída, escribe los tiempos de cada fase
en `arranque.txt` y cierra.

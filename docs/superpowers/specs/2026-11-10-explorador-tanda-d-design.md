# Tanda D: operaciones con archivos — diseño

## Intención

Que copiar, mover y ordenar archivos en xdir sea igual o mejor que en el Finder y el Explorador: una sola
ventana propia que muestra todas las copias en curso y se puede pausar, conflictos que se resuelven de una
vez para todos, errores que no cortan el trabajo, y las operaciones que hoy faltan (duplicar, carpeta nueva
con la selección, comprimir y extraer, renombrar varios, accesos directos, expulsar discos, devolver de la
Papelera) más una ventana de información propia con todo lo que tiene «Obtener información» del Finder.
Todo para Windows, macOS y Linux (Linux sin interfaz por ahora: el núcleo y los servicios quedan listos).

## Objetivos

1. **Diálogo de copia propio** (reemplaza al del sistema): una ventana con la cola de operaciones, cada una
   con barra de progreso, velocidad, tiempo restante, «Pausa» y «Cancelar»; conflictos con «Reemplazar»,
   «Conservar ambos», «Omitir» y, en carpetas, «Combinar», más «Hacer esto con todos»; errores juntados al
   final con «Reintentar». Lo que dura menos de ~1 s no muestra la ventana. Lo usan copiar, mover, pegar,
   arrastrar y soltar, y el progreso de comprimir y extraer.
2. **Duplicar** con Ctrl+D / ⌘D («nombre copia», «nombre copia 2»). Favoritos pasa a Ctrl+Shift+D / ⇧⌘D.
3. **Carpeta nueva con la selección** (Ctrl+Alt+Shift+N / ⌃⌘N): mueve la selección a «Carpeta nueva con N
   elementos» y abre el renombrado en línea.
4. **Comprimir y extraer**: «Comprimir» arma un .zip de la selección; «Extraer aquí» y «Extraer en
   "nombre"» para .zip, y también .tar.gz en macOS y Linux.
5. **Renombrar varios** (ventana al estilo del Finder): «Reemplazar texto», «Agregar texto» y «Formato»,
   con vista previa antes/después, choques marcados antes de aplicar y un solo paso de deshacer.
6. **Accesos directos**: «Crear acceso directo» (.lnk en Windows, alias en macOS, enlace simbólico en
   Linux) y «Mostrar el original».
7. **Expulsar** discos extraíbles y externos: botón ⏏ en «Este equipo» y en el árbol, y en el menú del
   clic derecho; si está ocupado, decir qué programa lo usa cuando el sistema lo informa.
8. **Devolver** desde la Papelera a su lugar original en los tres sistemas.
9. **Ventana de información propia** con todo lo de «Obtener información» del Finder: encabezado, Etiquetas,
   General, Más información, Nombre y extensión, Comentarios, Abrir con, Compartir y permisos; selección
   múltiple con totales; Alt+Enter en Windows y Linux, ⌘I en macOS; «Más en el sistema…» abre el diálogo
   nativo para lo que no se rehace.

## No objetivos

- Entrar a un zip como si fuera una carpeta, o la vista previa de lo que tiene adentro (la tanda C lo
  anotó para esta tanda; queda para más adelante: ver Decisiones).
- Otros formatos para comprimir (7z, rar, tar.gz al comprimir) o zips con contraseña.
- Que mandar a la Papelera o borrar definitivamente pase por el diálogo de copia (ver Decisiones).
- Copiar desde o hacia carpetas virtuales sin ruta de disco (teléfonos MTP, la Red de Windows): siguen con
  el Shell como hoy.
- Arrastrar con Alt para crear un acceso directo (el efecto `LINK` de soltar).
- Editar las ACL detalladas de Windows dentro de xdir (van por «Más en el sistema…»).
- Interfaz de Linux (al final del plan).

## Anti-objetivos

- **Perder datos** en un conflicto o al cancelar: los archivos a medio copiar se borran; «Reemplazar»
  escribe primero en un archivo temporal y recién cuando terminó bien pisa al anterior; mover entre discos
  es copiar, verificar y recién ahí borrar el original.
- Que la interfaz se trabe esperando al disco: toda lectura y escritura corre en hilos de trabajo; el hilo
  de interfaz solo dibuja el estado que le llega.
- Pisar un archivo sin preguntar (ni siquiera el mismo archivo copiado sobre sí mismo).
- Bajar archivos que están solo en la nube (iCloud, OneDrive, Dropbox) cuando la operación no necesita el
  contenido: copiar sí los baja; mover dentro del mismo disco, renombrar, la ventana de información,
  crear accesos directos y expulsar, no.
- Dependencias nuevas en `xdir-core`.

## Enfoques considerados

- **A. Seguir con el sistema** (`IFileOperation` en Windows; en macOS y Linux, lo propio de hoy) y
  sumar solo las operaciones que faltan. Poco código, pero el diálogo y los conflictos son distintos en
  cada sistema, en macOS y Linux no hay progreso ni conflictos (hoy se elige solo un nombre libre) y no hay
  cola ni pausa en ninguno.
- **B. Motor propio en `xdir-core` con primitivas de cada sistema (elegido).** El núcleo arma el plan
  (recorrer, contar, decidir conflictos, nombres, cola, progreso, velocidad, errores, deshacer) con `std` y
  es probable con los falsos; cada sistema pone solo las primitivas que lo hacen fiel (copiar un archivo
  con avisos de progreso, clonar en APFS/ReFS, renombrar sin pisar, Papelera). El mismo diálogo y las
  mismas reglas en los tres sistemas.
- **C. Motor entero en cada plataforma.** Más control fino por sistema, pero la lógica (la parte difícil y
  la que se prueba) se escribiría tres veces.

## Diseño

### 0. Punto de partida (cómo está hoy)

- `sys::FileOps` tiene un solo método, `spawn(owner, tx, op)`: corre un `Op` (`Copy`, `Move`, `Delete`,
  `Rename`, `NewFolder`, `NewFile`, `Undo`) en un hilo y devuelve un `OpResult` (deshacer, elemento a
  enfocar, abrir renombrado, error en texto) con `WM_APP_OP_DONE`.
- **Windows** (`xdir-windows/src/ops/win32.rs`): todo con `IFileOperation` y el diálogo del Explorador
  (progreso, conflictos, confirmaciones); un `IFileOperationProgressSink` anota lo hecho para armar el
  deshacer. Los repetidos en la misma carpeta los nombra el Shell (`FOF_RENAMEONCOLLISION`).
- **macOS** (`xdir-mac/src/ops/macos.rs`): `copyfile` recursivo con clonado, `rename`, Papelera con
  `NSFileManager`. Sin progreso ni conflictos: si el nombre existe se usa «nombre copia», «nombre copia 2»
  (`free_name`). Mover entre discos copia y borra sin verificar. Borrar definitivamente pide confirmación
  con un `NSAlert`.
- **Linux** (`xdir-linux/src/ops.rs`, `trash.rs`): como macOS, con la Papelera de freedesktop (`recycle`
  y `restore` ya existen).
- `App` (`app/fileops.rs`): pegar (`Copy`/`Move` desde el portapapeles; lo cortado se pega una vez),
  borrar, arrastrar y soltar (`drop_effect`: Ctrl copia, Shift mueve, mismo disco mueve) y la pila de
  deshacer (`undo`, hasta 50, Ctrl+Z). En carpetas virtuales (`Location::Shell`, p. ej. la Papelera de
  Windows) todo lo resuelve el Shell (`invoke_verb`, su menú con «Restaurar»).
- `Command::Properties` (Alt+Enter; ⌘I en macOS con el nombre «Obtener información») llama a
  `Shell::properties`: en Windows `SHObjectProperties`/`SHMultiFileProperties`, en macOS la ventana de
  información del Finder por `osascript`, en Linux nada.
- Atajos: `ToggleFavorite` es Ctrl+D (⌘D en macOS); `InvertSelection` es Ctrl+I (⌥⌘I en macOS, donde ⌘I
  ya es «Obtener información», tabla `KEYS` de `xdir-mac/src/sys/macos.rs`). `NewFolder` es Ctrl+Shift+N y
  `NewTextFile` Ctrl+Alt+N.
- Las unidades de «Este equipo» salen de `list_computer` de cada sistema (grupo 1) y en el árbol son nodos
  `Kind::Drive` (`tree.rs`, `refresh_drives`). No hay nada para expulsar.
- La Papelera: en Windows es `Location::Shell` (`::{645FF040-…}`); en macOS es `~/.Trash` y en Linux
  `~/.local/share/Trash/files`, ambas como `Location::Path`. Solo se puede devolver lo borrado en esta
  sesión con Ctrl+Z (`Undo::Recycled`).
- Las ventanas propias de xdir son paneles dibujados dentro de la ventana principal («Acerca de», la
  tarjeta de la vista rápida); los campos de texto son los del host (`text_create`).

### 1. Diálogo de copia propio

```
┌─ Copiando ─────────────────────────────────────────── ─ ✕ ┐
│ Copiando 1.240 elementos de «Fotos» a «Backup (E:)»        │
│ ███████████████░░░░░░░░░░  58 %   48,2 MB/s  · quedan 2 min │
│ IMG_2041.jpg                                   [⏸] [✕]     │
├───────────────────────────────────────────────────────────┤
│ Moviendo «Proyecto» a «Documentos»     En cola      [✕]     │
├───────────────────────────────────────────────────────────┤
│ Comprimiendo 3 elementos               ████░░ 31 %  [⏸][✕] │
└───────────────────────────────────────────────────────────┘
```

**Qué pasa por el diálogo.** `Op::Copy` y `Op::Move` entre rutas de disco, vengan de pegar, de arrastrar y
soltar o de un comando; duplicar, carpeta nueva con la selección, comprimir, extraer, devolver de la
Papelera y deshacer un movimiento (`Undo::Moved`). Borrar, renombrar, carpeta nueva, archivo nuevo y el
resto de los deshacer siguen como hoy por `FileOps::spawn`. Copiar desde o hacia `Location::Shell`, o pegar
algo sin rutas (`paste_virtual`), sigue con el Shell.

**El motor (en `xdir-core`, nuevo módulo `transfer`).**
- Cada operación es un *trabajo* de la cola. Al empezar recorre el origen solo con metadatos («Preparando…
  N elementos»), sin leer contenido ni bajar nada de la nube, y sabe así total de elementos y de bytes.
- Antes de copiar compara con el espacio libre del destino (`DiskScan::volume_space`): si no alcanza, el
  trabajo no empieza y lo dice («Faltan 3,2 GB en "Backup (E:)"»). En FAT32 los archivos de más de 4 GB se
  anotan como error desde el principio.
- Copia archivo por archivo con la primitiva del sistema, en bloques con aviso de progreso. Con el progreso
  calcula el porcentaje (por bytes), la velocidad (promedio móvil de los últimos ~5 s) y el tiempo restante
  (aparece después de ~2 s de datos; antes, «Calculando…»).
- **Pausa**: el hilo del trabajo se detiene en el siguiente aviso de progreso (dentro del archivo, no al
  terminarlo) hasta que se reanuda. **Cancelar**: se corta en el siguiente aviso, se borra el archivo a
  medio copiar y lo que ya estaba completo queda (y entra en el deshacer).
- **Mover dentro del mismo disco** es renombrar (instantáneo, no baja nada de la nube). **Entre discos**:
  copia cada archivo, verifica que el tamaño copiado sea el del origen y recién entonces borra el original
  de ese archivo; las carpetas de origen se borran cuando quedan vacías. Si algo falla, en el origen queda
  solo lo que no se pudo mover.
- **Reemplazar** nunca escribe sobre el archivo existente: copia a un temporal oculto en la carpeta destino
  y, cuando terminó bien, lo pone en lugar del otro con un reemplazo atómico; lo reemplazado va a la
  Papelera si ese disco tiene (así Ctrl+Z lo devuelve), o se borra si no tiene.
- Archivos solo en la nube: copiar los baja (la primitiva del sistema lo hace al leer); el recorrido
  inicial no.
- Los enlaces simbólicos se copian como enlaces, no lo que apuntan. Los atributos, fechas y metadatos los
  conserva la primitiva de cada sistema.
- **Varias operaciones**: corren a la vez si no comparten disco; las que comparten un disco (de origen o de
  destino) con una que está corriendo esperan «En cola», para no hacer saltar el disco entre las dos.
- El estado de cada trabajo llega al hilo de interfaz por un canal, con un mensaje de la aplicación
  (`WM_APP_TRANSFER`), como `WM_APP_OP_DONE`. Al terminar, cada trabajo devuelve su `OpResult` (deshacer,
  enfocar lo copiado) como hoy.

**La ventana.**
- Un panel flotante de xdir (dibujado como «Acerca de», no una ventana del sistema aparte), abajo a la
  derecha, que no bloquea el resto: se sigue navegando y operando mientras copia. Muestra la cola: cada fila
  con su título («Copiando N elementos de "A" a "B"»), barra, porcentaje, velocidad, tiempo restante, el
  archivo actual y los botones «Pausa»/«Reanudar» y «Cancelar».
- **Aparece solo si un trabajo tarda más de ~1 s** (o antes, si hay un conflicto o un error que mostrar).
  Lo que termina antes no muestra nada.
- Se puede achicar (─): queda como un indicador de progreso en la barra de estado; un clic lo vuelve a
  abrir. Cuando la cola se vacía sin errores, se cierra sola.
- Cerrar xdir con trabajos en curso pregunta «Hay N operaciones en curso. ¿Cancelarlas y salir?»
  (`ask_yes_no_cancel`); cancelar limpia lo a medio copiar antes de salir.

**Conflictos.**
```
┌─ Ya existe «informe.pdf» en «Documentos» ──────────────┐
│ [ícono] El que se copia   2,1 MB · modificado hoy 10:42 │
│ [ícono] El que ya está    1,9 MB · modificado ayer      │
│                                                         │
│ [Reemplazar] [Conservar ambos] [Omitir]  (+[Combinar])  │
│ ☐ Hacer esto con todos (quedan 12 conflictos)           │
└─────────────────────────────────────────────────────────┘
```
- Se muestran dentro del panel de copia, sobre la fila del trabajo: ese trabajo espera; los demás siguen.
- «Reemplazar» (como se describe arriba), «Conservar ambos» (agrega « 2», « 3»… antes de la extensión:
  «informe 2.pdf»), «Omitir». En carpetas, además «Combinar»: entra y resuelve archivo por archivo (con
  los mismos conflictos); «Reemplazar» en una carpeta manda la carpeta existente entera a la Papelera.
- «Hacer esto con todos» aplica la elección a los conflictos que quedan de ese trabajo (archivos y carpetas
  por separado: elegir «Combinar» para una carpeta no decide los archivos).
- Copiar un elemento sobre sí mismo (pegar en la misma carpeta) no es un conflicto: se comporta como
  Duplicar («nombre copia»). Mover un elemento a su misma carpeta no hace nada (como hoy).
- Se compara sin distinguir mayúsculas donde el disco no las distingue (NTFS, APFS por defecto).

**Errores.** Un archivo bloqueado, sin permiso o que desapareció no corta el trabajo: se anota (archivo y
motivo en castellano: «Lo está usando otro programa», «No tenés permiso», «El disco está lleno») y se
sigue. Al terminar, la fila muestra «Terminó con N errores», con la lista y los botones «Reintentar» (vuelve
a intentar solo esos) y «Cerrar». «El disco está lleno» sí detiene el trabajo (seguir no tendría sentido) y
lo dice.

**Deshacer.** Un trabajo es un paso de deshacer: lo copiado se manda a la Papelera, lo movido vuelve a su
lugar (por el motor, con su progreso) y lo reemplazado vuelve desde la Papelera. Hace falta una variante
`Undo::Batch(Vec<Undo>)` (se deshace en orden inverso) para combinar «creado» y «reemplazado».

**Lo que pone cada sistema** (un servicio nuevo en `sys`, con su falso en `sys::testing`; orientativo,
el plan lo afina):
- copiar un archivo con aviso de progreso por bloque (el aviso puede pausar o cancelar), clonando cuando
  el disco puede, y conservando fechas, atributos y metadatos;
- crear una carpeta copiando los atributos y fechas de otra (las fechas se ponen al terminar su contenido);
- renombrar sin pisar (falla si el destino existe) y reemplazar atómicamente;
- mandar a la Papelera devolviendo cómo deshacer, y saber si un disco tiene Papelera;
- datos de un elemento sin leer contenido (tipo, tamaño, enlace, solo en la nube) y si dos rutas están en
  el mismo disco (hoy `Shell::volume_of`).

| | Copiar archivo | Renombrar sin pisar / reemplazar | Notas |
|---|---|---|---|
| Windows | `CopyFile2` con `COPYFILE2_CALLBACK` (copia flujos alternativos; `COPY_FILE_COPY_SYMLINK`) | `MoveFileExW` sin / con `MOVEFILE_REPLACE_EXISTING` | rutas largas con `\\?\`; ya no se usa `IFileOperation` para copiar y mover entre rutas de disco |
| macOS | `copyfile` por archivo con `COPYFILE_ALL \| COPYFILE_CLONE` y `COPYFILE_STATE_STATUS_CB` | `renamex_np` con `RENAME_EXCL` / `rename` | conserva etiquetas, atributos extendidos y ACL como hoy |
| Linux | `ioctl FICLONE` y si no `copy_file_range` en bloques; luego permisos, fechas (`utimensat`) y `user.*` | `renameat2` con `RENAME_NOREPLACE` / `rename` | si `renameat2` no está (sistemas de archivos viejos), comprobar y renombrar |

### 2. Duplicar

- **Ctrl+D** en Windows y Linux, **⌘D** en macOS; también en el menú del clic derecho de los elementos
  («Duplicar») y en la paleta.
- Crea al lado de cada elemento seleccionado «nombre copia» (antes de la extensión: «informe copia.pdf»);
  si ya existe, «nombre copia 2», «nombre copia 3»… (como el Finder, también en Windows). Las carpetas se
  copian enteras.
- Pasa por el motor de copia (si es grande, se ve en el diálogo). Al terminar quedan seleccionadas las
  copias. Un paso de deshacer.
- Hoy: no existe. Cambia: `ToggleFavorite` pasa de Ctrl+D a **Ctrl+Shift+D** (⇧⌘D en macOS), aprobado por
  el usuario; quien tenga su propio atajo en `[atajos]` de `config.toml` lo conserva.

### 3. Carpeta nueva con la selección

- **Ctrl+Alt+Shift+N** en Windows y Linux, **⌃⌘N** en macOS (el del Finder); menú del clic derecho con
  dos o más elementos seleccionados («Carpeta nueva con la selección») y paleta.
- Crea en la carpeta del panel «Carpeta nueva con N elementos» (« 2» si existe), mueve ahí la selección y
  abre el renombrado en línea sobre la carpeta (como `rename_after` de hoy).
- Mover dentro de la misma carpeta es renombrar: instantáneo. En resultados de búsqueda, la carpeta se crea
  en la carpeta buscada y los elementos de otros discos se mueven con el motor (diálogo si tarda).
- Un paso de deshacer: devuelve los elementos a su lugar y manda la carpeta (si quedó vacía) a la Papelera.
- ⌃⌘N necesita un modificador que hoy no existe: en macOS «Ctrl» de los atajos es ⌘ y la tecla Control
  física no se puede escribir. Se agrega `Meta` a `Combo` y a la sintaxis de `[atajos]` (ver Decisiones).

### 4. Comprimir y extraer

- **«Comprimir»** (menú del clic derecho y paleta): un elemento → «nombre.zip»; varios → «Archivo.zip»
  (como el Finder); « 2» si ya existe. Al terminar queda seleccionado el zip.
- **«Extraer aquí»**: lo de adentro va a la carpeta del zip, con los conflictos del diálogo de copia.
  **«Extraer en "nombre"»**: crea la carpeta «nombre» (« 2» si existe) y extrae ahí. El menú muestra el
  nombre real. Para `.zip` en los tres sistemas y para `.tar.gz`/`.tgz` en macOS y Linux.
- El progreso, la pausa, cancelar (borra el zip a medio escribir o lo extraído a medias de ese archivo),
  los conflictos y los errores son los del diálogo de copia (filas «Comprimiendo…» y «Extrayendo…»).
- Seguridad al extraer: se rechazan entradas con rutas absolutas o con `..` que saldrían de la carpeta
  destino; los enlaces simbólicos del archivo se crean solo en macOS y Linux y solo si apuntan adentro.
  Zips con contraseña o métodos no soportados: error «Este zip tiene contraseña» / «Formato no
  soportado», y se ofrece «Abrir con el sistema».
- Nombres: UTF-8 si el zip lo marca; si no, CP437 (como los zips viejos de Windows). Archivos de más de
  4 GB: zip64.
- Comprimir lee el contenido: los archivos solo en la nube se bajan (es contenido que se necesita).

**Dónde vive el código** (`xdir-core` no puede tener dependencias). Opciones:

| Opción | Windows | macOS | Linux | Ventajas | Contras |
|---|---|---|---|---|---|
| A. Herramientas del sistema | `tar.exe` (bsdtar, viene desde Windows 10 1803) o la carpeta comprimida del Shell (`IShellDispatch::CopyHere`) | `ditto -c -k --sequesterRsrc --keepParent` / `ditto -x -k`, `tar` | `zip`/`unzip`/`tar` si están instalados | sin dependencias; en macOS, zips idénticos a los del Finder (atributos en `__MACOSX`) | sin progreso por bytes ni pausa; los conflictos los decide la herramienta; la carpeta comprimida del Shell muestra su propio diálogo; en Linux `zip` puede no estar |
| B. Crates en cada plataforma | `zip` + `flate2` | idem + `tar` | idem + `tar` | progreso, pausa, cancelar y conflictos iguales al resto | el mismo código tres veces |
| **C. Crate compartida nueva `xdir-archive`** (`zip` con solo deflate, `flate2` con `miniz_oxide`, `tar`), usada por las tres plataformas detrás de un servicio de `sys` | ✓ | ✓ | ✓ | una sola implementación, probada una vez; todo el control del motor; `xdir-core` sigue sin dependencias | dependencias nuevas en el workspace; en macOS el zip no guarda atributos extendidos ni etiquetas (como el del Explorador) |

**Recomendación: C.** El motor del núcleo maneja la cola y los conflictos; `xdir-archive` solo lee y escribe
el formato con avisos de progreso por bloque. Si en macOS importa la fidelidad con el Finder, se puede
comprimir con `ditto` (opción A) solo en macOS a cambio de no tener pausa ni porcentaje exacto (ver
preguntas).

### 5. Renombrar varios

```
┌─ Renombrar 24 elementos ───────────────────────────────┐
│ [Reemplazar texto ▾]                                    │
│ Buscar: [IMG_     ]   Reemplazar con: [Viaje-  ]        │
│                                                         │
│ Antes                    Después                        │
│ IMG_2041.jpg             Viaje-2041.jpg                 │
│ IMG_2042.jpg             Viaje-2042.jpg                 │
│ IMG_2043.JPG             Viaje-2043.JPG   ⚠ ya existe   │
│                               [Cancelar] [Renombrar]    │
└─────────────────────────────────────────────────────────┘
```

- Se abre con **F2 cuando hay más de un elemento seleccionado** (con uno solo sigue el renombrado en
  línea), con «Renombrar N elementos…» del menú del clic derecho y con la paleta.
- Modos (como el Finder):
  - **Reemplazar texto**: «Buscar» y «Reemplazar con» (todas las apariciones, distinguiendo mayúsculas).
  - **Agregar texto**: el texto y «antes del nombre» / «después del nombre».
  - **Formato**: «Nombre y número», «Nombre y contador» (con ceros: 001, 002…) o «Nombre y fecha» (la de
    modificación, `AAAA-MM-DD`); el texto base («Personalizado»), «antes del nombre» / «después del
    nombre» y el número inicial. El orden de numeración es el de la lista (con su orden y agrupación).
- Siempre se trabaja sobre el nombre **sin la extensión**; la extensión queda igual.
- **Vista previa en vivo**: cada fila con el nombre de antes y el de después, actualizada al teclear.
- **Choques marcados antes de aplicar**: dos nuevos iguales entre sí, un nuevo igual a un archivo de la
  carpeta que no se renombra, nombres vacíos o con caracteres inválidos del sistema (`\/:*?"<>|` en
  Windows, `/` en macOS y Linux). Con choques, «Renombrar» está apagado y la fila dice el motivo. Las
  comparaciones sin distinguir mayúsculas donde el disco no las distingue.
- Aplicar es seguro aunque los nombres se crucen (a→b y b→a): primero todos a nombres temporales y después
  a los finales; si algo falla en el medio, se vuelve todo atrás.
- **Un solo paso de deshacer** (Ctrl+Z) para todos: `Undo::Renames(Vec<(PathBuf, String)>)` (o
  `Undo::Batch` de `Undo::Rename`), con el mismo cruce seguro.
- El cálculo de los nombres es lógica pura del núcleo (con tests); en el sistema solo se renombra.

### 6. Accesos directos

- **«Crear acceso directo»** (menú del clic derecho y paleta), en la misma carpeta:
  - Windows: `.lnk` con `IShellLinkW` + `IPersistFile`, «nombre - Acceso directo.lnk» (como el
    Explorador).
  - macOS: alias del Finder con `NSURL` (`bookmarkDataWithOptions` con
    `NSURLBookmarkCreationSuitableForBookmarkFile` y `writeBookmarkData`), «nombre alias».
  - Linux: enlace simbólico, «Enlace a nombre» (como Nautilus).
  - Queda seleccionado y con el renombrado en línea abierto. Un paso de deshacer (`Undo::Created`).
- **«Mostrar el original»** (menú del clic derecho de un acceso directo, alias o enlace, y paleta): resuelve
  a dónde apunta (`IShellLinkW::GetPath`; `URLByResolvingAliasFileAtURL`; `read_link`) y abre su carpeta
  en el panel activo con el original seleccionado. Si el original ya no existe: «No se encuentra el
  original de "nombre"».
- Resolver un alias o un .lnk no lee contenido de archivos de la nube (solo el archivo del acceso).

### 7. Expulsar

- Un botón **⏏** en las filas de los discos expulsables de «Este equipo» y en sus nodos del árbol, y
  «Expulsar» en el menú del clic derecho del disco (en Windows, como entrada propia de xdir en el menú del
  Shell) y en la paleta (sobre el disco enfocado o el de la carpeta del panel).
- Qué es expulsable:
  - Windows: unidades extraíbles (`DRIVE_REMOVABLE`), discos externos USB/SD/1394 (bus del dispositivo por
    `IOCTL_STORAGE_QUERY_PROPERTY`), lectoras de CD/DVD e imágenes ISO/VHD montadas.
  - macOS: volúmenes expulsables, extraíbles o no internos (`NSURLVolumeIsEjectableKey`,
    `…IsRemovableKey`, `…IsInternalKey`), imágenes de disco y volúmenes de red (como el Finder).
  - Linux: montajes de dispositivos extraíbles (`/sys/block/*/removable`, o bajo `/media`, `/run/media`).
- Cómo:
  - Windows: `CM_Request_Device_EjectW` sobre el dispositivo del volumen; si el sistema lo veta, el tipo de
    veto y, si se puede saber, el programa (el evento 225 de Kernel-PnP del registro de Sistema lo nombra:
    se lee el más reciente con `EvtQuery`, como mejor esfuerzo).
  - macOS: `NSWorkspace unmountAndEjectDeviceAtURL`; si falla porque está ocupado, `lsof -F pc <volumen>`
    da los programas.
  - Linux: `udisksctl unmount` + `power-off` (o `gio mount -e`, o `umount` para montajes del usuario); si
    está ocupado, se recorren `/proc/*/fd` y `/proc/*/cwd` buscando el punto de montaje y se nombra el
    programa (`/proc/<pid>/comm`).
- Antes de expulsar, **xdir suelta lo suyo** en ese disco: guardianes de carpetas de las pestañas, lecturas,
  miniaturas y escaneos de xtree (si no, el que lo bloquea sería xdir). Las pestañas que estaban en ese
  disco pasan a «Este equipo». Si hay trabajos del diálogo de copia sobre el disco, primero pregunta si se
  cancelan.
- Mensajes: «Se puede quitar "USB (E:)" con seguridad» (en la barra de estado); si está ocupado: «No se
  pudo expulsar "USB (E:)": lo está usando Word» o, sin nombre, «… : lo está usando otro programa», con
  «Reintentar».
- Corre fuera del hilo de interfaz (expulsar puede tardar varios segundos).

### 8. Devolver desde la Papelera

- **«Devolver»** en el menú del clic derecho de los elementos de la Papelera, en la paleta y como botón en
  la barra de la carpeta cuando el panel muestra la Papelera. En macOS, ⌘⌫ dentro de la Papelera devuelve
  (como el Finder).
- Cada elemento vuelve a su carpeta original; si esa carpeta ya no existe, se crea; si ya hay algo con ese
  nombre, conflicto del diálogo de copia. Volver al mismo disco es renombrar; a otro disco (Papelera de un
  disco externo en macOS), el motor.
- De dónde sale el lugar original:
  - Windows: la Papelera del Shell; se usa el verbo `undelete` como ya hace `restore_from_recycle_bin`.
    Hoy el menú del Shell ya trae «Restaurar»: se suma la entrada propia y el botón.
  - macOS: el Finder guarda el lugar en `~/.Trash/.DS_Store` (registros `ptbL` y `ptbN`): se lee ese
    archivo (solo lectura). Además xdir anota en su carpeta de configuración lo que manda él a la
    Papelera (nombre adentro → ruta original), por si el `.DS_Store` no lo tiene. Sin ninguno de los dos,
    «Devolver» queda apagado con el cartel «No se sabe de dónde vino».
  - Linux: el `.trashinfo` de freedesktop (`trash::restore`, que ya existe).
- Un paso de deshacer (vuelve a mandarlos a la Papelera).

### 9. Ventana de información

```
┌─ Información de «informe.pdf» ─────────────── ✕ ┐
│ [vista previa]  informe.pdf                      │
│                 Documento PDF · 2,1 MB           │
│                 Modificado: hoy 10:42            │
│ ▾ Etiquetas      (●Rojo ✕) (●Trabajo ✕) [+]      │
│ ▾ General                                        │
│   Tipo · Tamaño (2.104.331 bytes) · En disco     │
│   Ubicación · Creado · Modificado                │
│   ☐ Bloqueado  ☐ Oculto  ☐ Solo lectura          │
│ ▸ Más información                                │
│ ▸ Nombre y extensión                             │
│ ▸ Comentarios                                    │
│ ▸ Abrir con                                      │
│ ▸ Compartir y permisos                           │
│                          [Más en el sistema…]    │
└──────────────────────────────────────────────────┘
```

- Reemplaza a `Shell::properties` como acción de `Command::Properties`: **Alt+Enter** en Windows y Linux,
  **⌘I** en macOS (los dos ya están; en macOS invertir la selección sigue en ⌥⌘I y en Windows Ctrl+I sigue
  invirtiendo la selección). Nombre: «Propiedades» en Windows y Linux, «Obtener información» en macOS.
  También en el menú del clic derecho. Sin selección, la de la carpeta del panel; en «Este equipo», la del
  disco.
- Es un panel flotante de xdir (como el de copia), uno a la vez, para los elementos con que se abrió (no
  sigue la selección). Esc o ✕ lo cierran. Las secciones se pliegan (▾/▸) y cuáles están abiertas se guarda
  en la sesión.
- Todo se lee en otro hilo (patrón de generación de la tanda C) y **sin bajar archivos de la nube**; lo que
  necesitaría el contenido se muestra vacío o con «Disponible solo en línea». Los cambios se escriben en otro
  hilo y un error se muestra en la propia sección.
- **Encabezado**: la imagen de la vista previa de la tanda C (misma caché), nombre, tipo, tamaño y fecha de
  modificación.
- **Etiquetas**: ver, agregar y quitar. macOS: etiquetas del Finder (`NSURLTagNamesKey`), con sus colores.
  Windows: palabras clave (`System.Keywords` por `IPropertyStore`), solo en los formatos que las admiten
  (si no, la sección dice «Este tipo de archivo no admite etiquetas»). Linux: atributo extendido
  `user.xdg.tags` (el que usan KDE y Baloo).
- **General**: tipo; tamaño exacto en bytes y tamaño en disco (`GetCompressedFileSizeW` redondeado al
  clúster; `st_blocks × 512`); ubicación (con clic para ir); creado y modificado.
  - Carpetas: tamaño y cantidad de archivos y carpetas, calculados en otro hilo (con «Calculando… N
    elementos» mientras tanto), o al instante si xtree ya escaneó esa carpeta. No se cuentan dentro de
    carpetas solo en la nube.
  - Discos: capacidad, disponible, usado y formato (`GetVolumeInformationW`; `NSURLVolumeLocalizedFormat
    DescriptionKey`; `statvfs` y `/proc/self/mountinfo`).
  - Casillas, según el sistema (ver Decisiones): **Bloqueado** (macOS: `uchg` /
    `NSURLIsUserImmutableKey`), **Oculto** (Windows: atributo; macOS: `UF_HIDDEN`; en Linux se muestra
    marcada y apagada si el nombre empieza con punto), **Solo lectura** (Windows: atributo; Linux: quita
    o pone la escritura del dueño).
- **Más información**: los metadatos de la tanda B (dimensiones, duración, autores, artista, álbum) que
  tengan valor y la fecha de última apertura (macOS: `kMDItemLastUsedDate`; Windows y Linux: último
  acceso, con la aclaración de que el sistema puede no actualizarlo).
- **Nombre y extensión**: el nombre editable (renombra al confirmar, con deshacer) y «Ocultar la
  extensión» en macOS (`NSURLHasHiddenExtensionKey`; en Windows y Linux no existe por archivo y no se
  muestra).
- **Comentarios** (editable): macOS, comentarios del Finder (leer con `kMDItemFinderComment`, escribir por
  el Finder con `osascript`, la única forma que el Finder respeta); Windows, la propiedad «Comentarios»
  (`System.Comment`) en los formatos que la admiten; Linux, `user.xdg.comment`.
- **Abrir con**: la app actual y la lista de apps que pueden abrirlo.
  - macOS: elegir una app la deja para ese archivo (como el Finder); «Cambiar todos…» la pone por defecto
    para el tipo (`NSWorkspace setDefaultApplicationAtURL:toOpenContentType:`).
  - Linux: «Cambiar todos…» cambia el predeterminado del tipo MIME (`xdg-mime default`); por archivo no
    existe, así que elegir una app lo abre con ella esa vez.
  - Windows: elegir una app lo abre con ella esa vez (`SHAssocEnumHandlers`); «Cambiar todos…» abre el
    diálogo «Abrir con» del sistema ya preparado con «Usar siempre» (`SHOpenWithDialog` con
    `OAIF_FORCE_REGISTRATION`), porque desde Windows 8 el sistema no deja que otra app cambie el
    predeterminado.
- **Compartir y permisos**: dueño, grupo y todos, con lectura y escritura.
  - macOS y Linux: editable (`chmod`); «Aplicar a los elementos incluidos» en carpetas, en otro hilo y con
    los errores juntados como en el diálogo de copia. Dueño y grupo se muestran pero no se cambian (haría
    falta ser administrador).
  - Windows: el dueño y si el usuario actual puede leer y escribir, solo para ver; las ACL detalladas por
    «Más en el sistema…».
- **Selección múltiple**: «N elementos», tamaños totales, tipos si son iguales, y las casillas valen para
  todos (con estado mixto si difieren). Etiquetas comunes y agregar a todos.
- **«Más en el sistema…»** abre el diálogo nativo (lo de hoy: `SHObjectProperties` /
  `SHMultiFileProperties` en Windows; la ventana del Finder en macOS); en Linux no se muestra.
- Linux sin interfaz: quedan el servicio (leer y escribir todo lo de arriba) y la lógica del núcleo.

### 10. Atajos y comandos

| Comando (id) | Windows / Linux | macOS | Hoy |
|---|---|---|---|
| Duplicar (`duplicar`) | Ctrl+D | ⌘D | nuevo |
| Agregar o quitar de favoritos (`favorito`) | Ctrl+Shift+D | ⇧⌘D | era Ctrl+D / ⌘D |
| Carpeta nueva con la selección (`carpeta_con_seleccion`) | Ctrl+Alt+Shift+N | ⌃⌘N | nuevo |
| Propiedades / Obtener información (`propiedades`) | Alt+Enter | ⌘I | igual, ahora abre la ventana propia |
| Invertir selección (`invertir_seleccion`) | Ctrl+I | ⌥⌘I | igual |
| Renombrar (`renombrar`) | F2 (varios: Renombrar varios) | F2 | cambia con varios |
| Comprimir, Extraer aquí, Extraer en…, Crear acceso directo, Mostrar el original, Expulsar, Devolver, Renombrar varios, Mostrar operaciones | sin atajo | sin atajo (Devolver: ⌘⌫ en la Papelera) | nuevos, en paleta y menús |

En Windows, Ctrl+D en el Explorador manda a la Papelera; en xdir duplica (lo aprobó el usuario; una
duplicación de más se deshace con Ctrl+Z).

## Restricciones

- `xdir-core` sin dependencias ni `cfg` de sistema en producción; lo de cada sistema detrás de servicios en
  `xdir_core::sys` (con su versión falsa en `sys::testing` para los tests). Zip y tar en una crate aparte
  (ver §4), nunca en `xdir-core`.
- Toda E/S de archivos en hilos de trabajo; el hilo de interfaz recibe estado por canales y mensajes de la
  aplicación y nunca espera al disco (ni para saber si algo está en la nube).
- Respetar archivos solo en la nube (mismas comprobaciones que la tanda C: `SF_DATALESS` en macOS,
  atributos `OFFLINE`/`RECALL_*` en Windows): solo se baja lo que la operación necesita leer.
- Castellano en toda la interfaz y en los mensajes de error; atajos documentados en la wiki al terminar.
- Antes de programar, las pantallas en FigCraft (`docs/design/xdir.figcraft`), en oscuro y claro:
  1. Panel de copia con la cola (una operación; varias con una en pausa y otra «En cola»; achicado en la
     barra de estado).
  2. Conflicto de archivo y de carpeta (con «Combinar» y «Hacer esto con todos»).
  3. Fin con errores (lista y «Reintentar») y el aviso de espacio insuficiente.
  4. Renombrar varios en sus tres modos, con un choque marcado.
  5. Ventana de información: un archivo con todas las secciones abiertas, una carpeta (calculando), un
     disco, varios seleccionados (casillas mixtas); cada sección por separado: Etiquetas, General, Más
     información, Nombre y extensión, Comentarios, Abrir con, Compartir y permisos.
  6. Botón ⏏ en «Este equipo» (en las vistas Detalles e Íconos) y en el árbol, y el aviso de disco ocupado.
  7. Botón «Devolver» en la barra de la Papelera.
  8. Menú del clic derecho con las entradas nuevas (Duplicar, Comprimir, Extraer, Crear acceso directo,
     Mostrar el original, Carpeta nueva con la selección, Renombrar N elementos…, Expulsar).
- Tests con `install_fakes()`, `FakeHost`, `RecordingPainter`; el motor de copia, los nombres («copia», « 2»,
  «Carpeta nueva con N elementos»), Renombrar varios, los conflictos y el deshacer se prueban en el núcleo
  con un sistema de archivos falso; las primitivas de cada sistema con carpetas temporales (como los tests
  de `ops/macos.rs` y de `xdir-linux`). `cargo test` y `cargo check --all-targets` limpios en los 3
  sistemas; prueba de humo en Mac (copia grande con pausa y cancelar, conflicto, mover entre discos con un
  pendrive, expulsar).
- PRs propios contra `develop` a medida que se terminan las partes; versión de Mac instalada al terminar la
  tanda.

## Lo que queda para quien construya

- La forma exacta del servicio de primitivas de copia (nombre, métodos, errores) y si extiende `FileOps` o
  es uno nuevo; el formato de los mensajes del motor al hilo de interfaz.
- Medidas y ubicación de los paneles (copia, conflicto, renombrar varios, información): las define el
  diseño en FigCraft.
- Tamaño de bloque de la copia y frecuencia de los avisos de progreso (que la interfaz no se redibuje más
  de ~10 veces por segundo por trabajo).
- Los textos de error por código del sistema.
- El lector de `.DS_Store` (solo los registros `ptbL`/`ptbN`).
- Orden de las partes en el plan. Sugerido: motor de copia + diálogo (Windows y macOS) → Duplicar y
  favoritos → Carpeta nueva con la selección (+ modificador `Meta`) → Renombrar varios → Devolver →
  Accesos directos → Expulsar → Comprimir y extraer → Ventana de información (por secciones) → Linux.

## Decisiones tomadas al revisar el diseño (llamadas de Claude, corregibles)

1. **Paneles dibujados, no ventanas del sistema.** El diálogo de copia, los conflictos, Renombrar varios y la
   ventana de información son paneles flotantes dibujados por xdir dentro de su ventana, como «Acerca de» y
   la tarjeta de la vista rápida: el `WindowHost` no tiene ventanas secundarias y agregarlas en dos
   sistemas (más Linux después) es mucho trabajo. Consecuencia: una sola ventana de información a la vez y
   el panel de copia no se puede sacar fuera de la ventana de xdir.
2. **Borrar no pasa por el diálogo de copia.** Mandar a la Papelera es un renombrado (rápido, sin bytes que
   mostrar) y cada sistema tiene sus reglas (en Windows, el aviso de «demasiado grande para la Papelera» y
   las confirmaciones del Shell; en macOS, el `NSAlert` antes de borrar definitivamente). Se deja como hoy.
   Si una tanda de Papelera tarda más de ~1 s (miles de archivos, Papelera de otro disco en Linux), no se
   ve progreso: aceptable por ahora.
3. **Lo reemplazado va a la Papelera** (si el disco tiene), no se borra: así «Reemplazar» se puede deshacer y
   un error a mitad de camino no pierde nada. Es más seguro que el Finder y el Explorador, que lo borran.
4. **Modificador `Meta`** en `Combo` y en `[atajos]` («Meta+Ctrl+N»): en macOS es la tecla Control (⌃, se
   dibuja primero, en el orden de Apple ⌃⌥⇧⌘); en Windows y Linux es la tecla Windows/Super, sin atajos de
   fábrica. Hace falta para ⌃⌘N.
5. **Conservar ambos** usa « 2», « 3»… (como el Finder) y **Duplicar** «copia», «copia 2», también en
   Windows (el Explorador usa « - copia»): mismas reglas en los tres sistemas. Pegar en la misma carpeta se
   comporta como Duplicar.
6. **«Extraer en "nombre"» sin diálogo**: el borrador lo escribía con «…», pero no pide nada (el nombre se ve
   en el menú y si existe se agrega « 2»), así que va sin puntos suspensivos.
7. **Comprimir y extraer con una crate compartida `xdir-archive`** (opción C de §4): única implementación,
   con progreso y pausa como el resto. En macOS el zip no lleva atributos extendidos ni etiquetas.
8. **Casillas de la ventana de información por sistema**: Windows, Oculto y Solo lectura (Bloqueado no
   existe); macOS, Bloqueado y Oculto (la escritura se maneja en «Compartir y permisos», como en el
   Finder); Linux, Solo lectura y Oculto informativo (ocultar sería renombrar con un punto: no se hace
   desde una casilla).
9. **Concurrencia de la cola**: trabajos en paralelo salvo que compartan un disco; entonces esperan «En
   cola» (se puede cancelar uno que espera).
10. **Verificación al mover entre discos**: por tamaño de cada archivo, sin comparar contenido (un hash
    duplicaría el tiempo); la primitiva ya devuelve error si la copia falló.
11. **Accesos directos sin atajo de fábrica**: los del Finder (⌃⌘A / ⌘L para alias, ⌘R para mostrar el
    original) chocan con atajos de xdir en macOS (⌘L escribir ruta, ⌘R recargar).
12. **F2 con varios seleccionados abre Renombrar varios** (como el Finder con «Renombrar…»); con uno, el
    renombrado en línea de hoy.
13. **Al expulsar, las pestañas de ese disco pasan a «Este equipo»** antes de intentarlo (para soltar sus
    guardianes); si falla, quedan en «Este equipo».
14. **Navegar dentro de zips queda fuera** de esta tanda aunque la C lo anotó acá: no está en lo aprobado y es
    una ubicación nueva (`Location`) con su propia lectura; mejor como tanda propia.

## Decisiones del usuario (2026-11-10), mandan sobre las anteriores

1. **Ventana de información = panel**, como la columna de vista previa: se abre a la derecha de la ventana cuando se pide (Alt+Enter / ⌘I, menú «Propiedades / Obtener información») y muestra lo del panel activo; se cierra con su botón o con el mismo atajo. Una a la vez. Reemplaza a la Decisión 1 para la información.
2. **Diálogo de copia = ventana aparte del sistema** (como la de Windows), no dibujada dentro de xdir: muestra la cola, el avance de cada operación y, como Windows, **un gráfico de velocidad** de la copia en curso (velocidad a lo largo del tiempo, con el promedio y lo que falta). Implica una ventana secundaria en el host de Windows y de macOS (Windows ya tiene la base de la ventana de la vista rápida). Reemplaza a la Decisión 1 para la copia.
3. **Renombrar varios sin ventana:** un panel a la derecha (el mismo lugar que información) con el modo y los campos, y la vista previa en la lista misma: la columna Nombre muestra el nombre nuevo de cada seleccionado mientras se escribe (lo que cambia resaltado y el nombre viejo tachado al lado); los choques se marcan en rojo en la fila. «Renombrar» aplica; «Cancelar» o Esc vuelven a la lista normal. (Propuesta de Claude al usuario; confirmar en el diseño visual.)
4. **Zip con una librería común para las tres plataformas** (crate compartida, como en la Decisión 7), con avance del proceso en el diálogo de copia. En macOS no se conservan etiquetas ni atributos extendidos dentro del zip (aceptado).

## Desvíos en la implementación

Lo que quedó distinto de lo escrito arriba al programar la tanda (manda esto):

1. **Información y copia, según las Decisiones del usuario:** la información es un panel a la derecha (el lugar de la
   vista previa, uno solo a la vez con vista previa y Renombrar varios) y la copia es una ventana propia del sistema con
   la cola y el gráfico de velocidad. El título del panel es «Información» (no «Propiedades» ni «Obtener información»,
   que quedan como nombres del comando en cada sistema).
2. **`sys::Archives::extract_all` con `failed`:** extraer todo recibe `failed(nombre, error)` para cada entrada que no
   se puede extraer y sigue con las demás; solo corta si se cancela, se llena el disco, el zip tiene contraseña o no se
   puede leer. Las horas de un zip (DOS) se leen y se escriben en hora local (la diferencia con UTC la da cada
   plataforma).
3. **«Extraer en una carpeta»:** si el archivo trae una sola carpeta arriba de todo, se sube (no queda «Viaje/Viaje/»), y
   `__MACOSX` (los metadatos del Finder) no se extrae, ni ahí ni en «Extraer aquí».
4. **Expansión grande se pregunta antes de extraer:** más de 10 GiB, o más de 100 veces lo que ocupa el archivo cuando
   eso pasa de 1 GiB, o más de 100 000 entradas.
5. **Zip con contraseña:** no se pide la contraseña; al terminar se pregunta «¿Abrir con el sistema?» (Sí / No) y con Sí
   lo abre la aplicación del sistema.
6. **En la Mac no se renombra nada dentro de la Papelera:** lo que se reemplaza va a la Papelera ya apartado como
   «… (reemplazado)» y queda con ese nombre (el «Devolver» del Finder usa el nombre guardado en el `.DS_Store`), igual
   que en Windows; deshacer lo trae desde ahí al destino. En la Papelera de la Mac, ⌘⌫ y Supr devuelven.
7. **Deshacer un Renombrar varios** (`Undo::Renames`) lo hace el núcleo con el mismo cruce seguro de `rename_all`, que
   renombra siempre con `rename_noreplace` (nunca pisa), no `FileOps`.
8. **«Devolver» en Windows** usa el verbo `undelete` del Shell sobre la Papelera virtual; en macOS y Linux, el motor de
   copia (`JobSpec::Restore`, con conflictos en la ventana de copia).
9. **`Op::Copy`, `Op::Move` y `Op::Restore` se quitaron:** copiar, mover, duplicar, «Carpeta nueva con la selección» y
   devolver son todos del motor de copia; `FileOps` queda para borrar, renombrar, carpeta y archivo nuevos, accesos
   directos y deshacer.
10. **Windows, «Carpeta nueva con la selección» = Ctrl+Alt+Shift+N:** en los teclados español y latinoamericano Ctrl+Alt
    es AltGr, así que es AltGr+Shift+N; se puede cambiar en la configuración de atajos.

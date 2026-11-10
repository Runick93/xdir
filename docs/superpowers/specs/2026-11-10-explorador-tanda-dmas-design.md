# Tanda D+: configuración y red — diseño

## Intención

Que todo lo que hoy se ajusta a mano en `config.toml` o está repartido en la rueda dentada se pueda ver y
cambiar desde una **pestaña de Configuración** propia de xdir, como en VS Code o en los Ajustes del
sistema: con buscador, secciones y un editor de atajos donde se aprieta la combinación nueva. Y que las
carpetas de red sean de primera clase en los tres sistemas: **conectarse a un servidor** escribiendo su
dirección (con recientes y favoritos), **conectar y desconectar unidades de red** en Windows como en el
Explorador, una entrada **«Red»** en el árbol con los servidores conectados y lo que anuncia la red local,
y desconectar con ⏏. Las contraseñas las guarda siempre el sistema, nunca xdir, y una carpeta de red lenta
o colgada nunca traba la ventana. Todo para Windows, macOS y Linux (Linux sin interfaz por ahora: el
núcleo y los servicios quedan listos).

## Objetivos

1. **Pestaña Configuración** dentro del panel activo, como una pestaña de carpeta: se cierra, se arrastra a
   otro panel y hay una sola (abrirla de nuevo la enfoca). Se abre con **⌘,** (macOS) / **Ctrl+,**
   (Windows y Linux), con «Más opciones…» de la rueda dentada y desde la paleta.
2. **Distribución**: arriba un buscador («Buscar en la configuración»), a la izquierda la lista de
   secciones y a la derecha el contenido. Secciones: Apariencia, Lista y vistas, Árbol, Vista previa,
   Atajos de teclado, Red, xtree, Avanzado.
3. **Los cambios se aplican al instante** y se guardan donde viven hoy: lo de la sesión en `sesion.toml`
   (los switches de la rueda dentada siguen ahí y la rueda los sigue cambiando), la vista por defecto en
   `carpetas.toml`, y lo de `config.toml` en `config.toml` **sin perder lo que el usuario escribió a mano**
   (comentarios, orden, claves que xdir no conoce).
4. **Editor de atajos**: todos los comandos agrupados como en la wiki, con sus atajos de ahora; ✎ graba una
   combinación nueva apretándola; si choca con otro comando lo dice y ofrece «Reasignar»; se puede agregar
   un segundo atajo, quitar uno, restablecer uno o todos.
5. **Conectar a un servidor…** con **⌘K** (macOS) / **Ctrl+K** (Windows y Linux): un panel para escribir
   la dirección (`smb://servidor/recurso`, `\\servidor\recurso`; `afp://` en macOS; `sftp://` en Linux),
   con recientes y favoritos (estrella). Las credenciales las pide y guarda el sistema.
6. **Windows: «Conectar unidad de red…»** (letra, carpeta, «Conectar de nuevo al iniciar sesión», «Conectar
   con otras credenciales») y **«Desconectar unidad de red…»**, como el Explorador.
7. **«Red» en el árbol** en los tres sistemas: favoritos, servidores conectados (con ⏏) y las máquinas que
   anuncia la red local donde el sistema lo da (ver §B.4).
8. **Desconectar con ⏏**, con el mismo flujo de Expulsar de la tanda D (pregunta si hay copias en curso y
   espera a que limpien).
9. **Sección Red de la configuración**: favoritos y recientes, con editar, quitar y ordenar.
10. **Nada de red en el hilo de interfaz**: conectar, listar, desconectar y descubrir corren en hilos de
    trabajo con tiempos límite; una carpeta de red que tarda muestra «Conectando…».
11. **Wiki al día** al terminar: Atajos de teclado, Configuración y una página nueva de Red.

## No objetivos

- Editar las reglas de `[colores]` desde la pestaña (se ven con su muestra de color y hay un botón para
  abrir `config.toml`; editarlas en la interfaz queda para más adelante).
- Ver o borrar credenciales guardadas desde xdir (eso es el Llavero, el Administrador de credenciales o el
  llavero de GNOME/KDE).
- Protocolos que el sistema no monta: FTP, WebDAV, NFS a mano, `sftp://` en Windows y macOS (ver
  Decisiones).
- Compartir carpetas propias en la red, o cambiar permisos de recursos compartidos.
- Sincronizar la configuración entre equipos.
- Importar o exportar la configuración (más allá de copiar los archivos de la carpeta).
- Interfaz de Linux (al final del plan, como en la tanda D).

## Anti-objetivos

- **Guardar contraseñas**, ni en archivos, ni en la sesión, ni en memoria más allá de la llamada al sistema
  que las usa (en Linux, ver §B.2). Los favoritos y recientes guardan solo la dirección y, si el usuario
  la escribió, el nombre de usuario de la URL (`smb://ana@nas/fotos`), nunca una contraseña: una URL con
  `usuario:clave@` se guarda sin la clave.
- **Trabar la interfaz esperando a la red**: ninguna llamada que pueda tocar un servidor (montar, listar,
  `GetLongPathNameW`, `statfs`, `metadata`, la Red del Shell) corre en el hilo de interfaz.
- **Perder lo escrito a mano en `config.toml`**: xdir cambia solo las líneas de lo que se tocó en la
  pestaña; el resto del archivo queda byte por byte igual.
- Dependencias nuevas en `xdir-core`.
- Textos en otro idioma que no sea castellano.

## Enfoques considerados

### Dónde vive la pestaña

- **A. Una ventana o panel flotante de Configuración** (como «Acerca de»). Fácil de dibujar, pero tapa los
  paneles, no se puede tener al lado de una carpeta y no es lo aprobado.
- **B. Una vista que tapa los paneles, como xtree** (`xtree_open`). Reutiliza ese mecanismo, pero no es
  una pestaña: no se cierra con Ctrl+W, no se mueve entre paneles ni queda en la sesión.
- **C. Una ubicación nueva, `Location::Settings` (elegido).** La pestaña es una `Tab` como cualquier otra
  (tira de pestañas, arrastre entre paneles, Ctrl+W, sesión), pero en vez de una `FileList` dibuja la
  vista de configuración. Hay que enseñarle a cada `match` sobre `Location` qué hacer con ella (no se
  carga, no se vigila, no tiene padre ni migas), y eso es lo que cuesta.

### Cómo se escribe `config.toml`

- **A. Reescribir el archivo entero** desde el `Config` en memoria. Simple, pero borra comentarios, orden
  y claves desconocidas: rompe la promesa de la wiki («xdir nunca lo reescribe»).
- **B. Una sección administrada** (`# --- lo de abajo lo escribe xdir ---`) al final. No toca lo de
  arriba, pero en TOML una clave no puede aparecer dos veces en la misma tabla: o la sección administrada
  repite `[atajos]` (inválido) o hay que inventar tablas nuevas (`[atajos_xdir]`) que el usuario no
  entiende y que pisan en silencio lo que escribió arriba.
- **C. Un editor mínimo por líneas sobre el TOML de xdir (elegido).** `toml::parse` ya da la línea de cada
  clave y de cada sección; el editor reemplaza solo el valor de la línea de esa clave (conservando el
  comentario del final), la agrega si no está y la quita al restablecer. Es poco código, probable con
  textos, y deja el archivo como el usuario lo dejó. Ver §A.9.

### Red

- **A. Abrir lo del sistema** (el «Conectarse al servidor» del Finder, el asistente de unidades del
  Explorador). Poco código, pero cada sistema distinto, sin recientes ni favoritos propios y en macOS
  saca al usuario de xdir.
- **B. Panel propio de xdir y montaje con las APIs del sistema (elegido).** Mismo panel en los tres
  (dirección, recientes, favoritos); el montaje y las credenciales son del sistema (NetFS, WNet, `gio`),
  así que xdir nunca ve ni guarda una contraseña en macOS y Windows.
- **C. Cliente SMB propio.** Funcionaría igual en todos lados, pero es una dependencia enorme, habría que
  manejar contraseñas y no se integraría con el Llavero ni con las unidades del sistema. Descartado.

## Diseño

### 0. Punto de partida (cómo está hoy)

- **`config.toml`** (`xdir-core/src/config.rs`): se escribe la plantilla comentada la primera vez
  (`template()`) y después xdir solo lo lee; `reload_config` (`app/saved_groups.rs`) lo vuelve a leer al
  volver a la ventana si cambió la fecha (`changed_on_disk`). Secciones: `[atajos]` (id → string o lista,
  reemplaza los de fábrica; `""` deja sin atajo), `[colores]` (patrón o `@atributo` → `#RRGGBB`, antes de
  las de fábrica `DEFAULT_COLORS`) y `[opciones]` (`animaciones`, `arbol_sigue_panel`). El lector es
  `xdir-core/src/toml.rs` (sin dependencias; da `Entry { key, value, line }` y `Section { name, line }`,
  y `toml::quote`).
- **Sesión** (`session.rs`, `sesion.toml`): ventana, layout, pestañas (`location_str`/`location_from`:
  «Este equipo», ruta o `shell:<análisis>|<nombre>`), y los switches: `accent` (`Accent::System` o
  `Rgb`), `dark_mode: Option<bool>`, `file_colors`, `show_extensions`, `checkboxes`, `tree_follow:
  Option<bool>` (si es `None` manda `arbol_sigue_panel`), `show_hidden`, `show_tree`, `tree_pinned`,
  `group_bar`, `preview_on`, `preview_width` (240–640), `info_sections`. La vista por defecto de las
  listas está en `carpetas.toml` (`folder_views`). Modo portable: `xdir.portable` junto al ejecutable
  (`resolve_config_dir`).
- **Rueda dentada** (`ui.rs`): `draw_picker` con los 12 `theme::PRESETS`, «Color del sistema»,
  «Personalizado…» y los switches de `switch_kinds()` (`PickerSwitches`: modo oscuro, colores por tipo,
  extensiones, casillas, el árbol sigue al panel, ocultos, barra de grupos, árbol), más «Acerca de».
- **Comandos** (`commands.rs`): tabla `ALL` de `spec(Command, id, nombre, atajos)`; `Keymap::set`
  devuelve los comandos que perdieron una tecla; `parse_combo` entiende `Ctrl/Shift/Alt/Meta` y `Coma`.
  En macOS `KEYS` de `xdir-mac/src/sys/macos.rs` reemplaza atajos (`Ctrl` es ⌘, `Meta` es ⌃) y
  `command_title` cambia nombres. `OpenConfig` («Abrir la configuración (config.toml)») abre el archivo
  en el editor (`host.open_in_editor`); en macOS el menú xdir → «Configuración…» (⌘,) corre ese comando
  (`xdir-mac/src/app/mac.rs`). Ni `Ctrl+K` ni `Ctrl+,` están usados.
- **Pestañas** (`tab.rs`, `pane.rs`): `Tab { location, list: FileList, … }`; `Location` es `Computer`,
  `Path` o `Shell { parsing, name, parent }`. xtree **no** es una pestaña: es una vista que tapa los
  paneles (`App::xtree`, `xtree_open`). `Tab::new`/`navigate` llaman a `long_path` **en el hilo de
  interfaz** (en Windows `GetLongPathNameW`, que con una ruta UNC va al servidor).
- **Red hoy**: en Windows el árbol tiene «Red» (`Kind::Network`, la carpeta virtual del Shell
  `NETWORK_PLACE`, hijos por `shell_subfolders`) y «Linux» (WSL). En macOS y Linux `network_place()` es
  `""` y no hay «Red». `FileSystem::connect_share` existe solo en Windows (`WNetUseConnectionW` con
  `CONNECT_INTERACTIVE | CONNECT_PROMPT`) y la llama `Tab` una vez por navegación cuando una ruta UNC
  falla por credenciales (`auth_nav`); en macOS y Linux devuelve `false`.
- **Expulsar** (tanda D): `Shell::ejectable`/`eject` en hilos de trabajo; `App::eject` →
  `eject_after_jobs` (pregunta si hay copias sobre ese disco y espera la limpieza) → `release_volume`
  (suelta guardianes, xtree, vista previa) → `spawn_task` → `TaskDone::Ejected`. En Windows `ejectable`
  no incluye `DRIVE_REMOTE`; en macOS sí incluye volúmenes de red.

### A. Configuración

#### A.1 La pestaña

```
┌ Docs ┐┌ ⚙ Configuración ✕ ┐┌ + ┐
├──────────────────────────────────────────────────────────────┤
│  [🔍 Buscar en la configuración                            ] │
├────────────────┬─────────────────────────────────────────────┤
│ Apariencia     │ Apariencia                                  │
│ Lista y vistas │  Tema         ( Claro | Oscuro | Sistema )  │
│ Árbol          │  Color de acento  ● ● ● ● ● ● ● ● ● ● ● ●    │
│ Vista previa   │                   ○ Del sistema  ○ Otro…    │
│ Atajos         │  Animaciones  ( Sistema | Sí | No )         │
│ Red            │                                             │
│ xtree          │                                             │
│ Avanzado       │                                             │
└────────────────┴─────────────────────────────────────────────┘
```

- **Modelo**: `Location::Settings` (sin datos). En la `Tab`, la lista queda vacía y no se llama a
  `spawn_load` ni a `watch`; un estado propio de la vista (`SettingsView`: sección elegida, desplazamiento,
  texto buscado, grabación en curso) vive en la pestaña (`Tab::settings: Option<SettingsView>`), así se
  mueve con ella al arrastrarla a otro panel.
- **Una sola**: abrir la configuración busca una pestaña `Settings` en todos los paneles; si existe, activa
  ese panel y esa pestaña; si no, abre una pestaña nueva en el panel activo (no reemplaza la carpeta que se
  estaba viendo). Arrastrarla a otro panel la mueve (no la duplica). Ctrl+W / ⌘W la cierra y «Reabrir la
  última pestaña cerrada» la vuelve a abrir.
- **Título y migas**: la pestaña dice «Configuración» con el ícono de engranaje; la barra de dirección
  muestra «Configuración › Atajos de teclado» (las migas llevan a la sección) y no se puede escribir una
  ruta mientras está activa (Ctrl+L / ⌘L pasa al buscador). Atrás/Adelante/Subir quedan apagados.
- **Sesión**: se guarda como `xdir:configuracion` (y la sección como `xdir:configuracion#atajos`) en
  `location_str`; al volver a abrir xdir aparece donde estaba.
- **Comandos que no aplican** (copiar, pegar, vistas, ordenar, filtros, vista previa del elemento…) no
  hacen nada con la pestaña activa, como en «Este equipo» con los que necesitan una carpeta.
- **Cómo se abre**: comando nuevo `Settings` (id `configuracion`, «Configuración»), **Ctrl+Coma** de
  fábrica (en macOS ⌘, porque `Ctrl` es ⌘). El menú xdir → «Configuración…» de macOS pasa a correr
  `Settings` (no `OpenConfig`). En la rueda dentada, un botón **«Más opciones…»** abajo, al lado de
  «Acerca de» (`PickerHit::MoreOptions`), que cierra la rueda y abre la pestaña. En la paleta:
  «Configuración» y «Configuración: Atajos de teclado» (comando `SettingsKeys`, id
  `configuracion_atajos`, abre directo en esa sección).

#### A.2 Buscador

- Campo de texto del host (`text_create`) arriba. Cada opción tiene su título, su descripción corta y
  palabras clave (por ejemplo «Tema»: oscuro, claro, modo, noche).
- Al escribir, la columna derecha muestra **todas las opciones de todas las secciones** que coinciden,
  agrupadas con el nombre de la sección arriba; la lista de la izquierda marca cuántas hay en cada una y
  atenúa las que no tienen. En «Atajos» también se busca por nombre de comando y por atajo («⌘K»,
  «ctrl+k»). Sin texto, vuelve a la sección elegida.
- Comparación sin mayúsculas ni tildes (`atajo` encuentra «Atajos»).

#### A.3 Apariencia

| Opción | Control | Guarda en |
|---|---|---|
| Tema | Segmentado: Claro / Oscuro / Sistema | `sesion.toml` `dark_mode` (`Some(false)`, `Some(true)`, `None`) |
| Color de acento | Los 12 presets, «Del sistema» y «Otro…» (el selector de color del sistema, el mismo que «Personalizado…» de la rueda) | `sesion.toml` `accent` |
| Animaciones | Segmentado: Sistema / Sí / No | `config.toml` `[opciones] animaciones` (Sistema = se quita la línea) |

El switch de modo oscuro de la rueda dentada sigue siendo un switch: prenderlo o apagarlo fija el modo (como
hoy); «Sistema» solo se elige desde la pestaña.

#### A.4 Lista y vistas

| Opción | Control | Guarda en |
|---|---|---|
| Mostrar extensiones | Switch | sesión `show_extensions` |
| Mostrar archivos ocultos | Switch | sesión `show_hidden` (mismo estado que Ctrl+H / ⇧⌘.) |
| Casillas de selección | Switch | sesión `checkboxes` |
| Barra de grupos | Switch | sesión `group_bar` |
| Colores por tipo | Switch, y debajo las reglas activas (propias primero, después las de fábrica) con su muestra de color, solo para ver, y «Editar en config.toml» | sesión `file_colors` |
| Vista por defecto | Desplegable con las vistas de `list::VIEWS`, más «Usar la vista de esta carpeta en todas» y «Restablecer…» | `carpetas.toml` (vista por defecto) |

«Vista por defecto» cambia la vista de las carpetas que no tienen una propia (las que sí, la conservan),
como `ApplyViewToAll` pero sin borrar las demás.

#### A.5 Árbol

| Opción | Control | Guarda en |
|---|---|---|
| Mostrar el árbol de carpetas | Switch (solo con la feature `arbol`) | sesión `show_tree` |
| Fijo al lado de los paneles | Switch | sesión `tree_pinned` |
| El árbol sigue al panel | Switch | sesión `tree_follow` (como el switch de la rueda: una vez tocado, manda sobre `arbol_sigue_panel`) |

Al pie, «De fábrica: lo que diga `arbol_sigue_panel` en config.toml» con «Volver a lo de config.toml»
(pone `tree_follow = None`), para que la regla de hoy se vea.

#### A.6 Vista previa

| Opción | Control | Guarda en |
|---|---|---|
| Mostrar la vista previa | Switch | sesión `preview_on` |
| Ancho | Deslizador de 240 a 640 px, con el número | sesión `preview_width` |

#### A.7 Atajos de teclado

```
│ Atajos de teclado                [Restablecer todos]        │
│ Navegación                                                  │
│  Ir a una carpeta…           ⌘P                    ✎  +     │
│  Atrás                       ⌘[   ⌘←            ✎  +  ↺     │
│ Archivos                                                    │
│  Duplicar                    [ Apretá la combinación… ]  ✕  │
│    ⚠ ⌘K ya es de «Conectar a un servidor…»                  │
│                                   [Reasignar] [Cancelar]    │
│  Conectar a un servidor…     ⌘K   (cambiado)       ✎  +  ↺  │
```

- **Grupos y orden**: los de la página de la wiki (Navegación, Pestañas y paneles, Selección, Archivos,
  Vistas, Vista previa y vista rápida, Red, Otros), con los comandos sin atajo de fábrica dentro de su
  grupo (no en una lista aparte). La agrupación va en una tabla del núcleo (comando → grupo) junto a `ALL`,
  y la wiki se regenera con el mismo orden. Solo los comandos `available()` en ese sistema.
- **Cada fila**: nombre del comando (el del sistema: `title()`), sus atajos actuales como chips con el
  estilo del sistema (`combo_label`: ⌘⇧P en macOS, Ctrl+Shift+P en los otros), «(cambiado)» si difiere de
  fábrica, y botones: **✎** (grabar un atajo que reemplaza al chip elegido; sin chips, el primero), **+**
  (grabar uno más), **✕** en cada chip al pasar el mouse (quitarlo), **↺** (restablecer los de fábrica de
  ese comando; solo si está cambiado).
- **Grabar**: la fila muestra «Apretá la combinación…»; la siguiente tecla con sus modificadores es el
  atajo. **Esc cancela** (no se puede asignar Esc solo desde la pestaña). Mientras se graba, ninguna tecla
  llega al `Keymap` ni a la lista (tampoco ⌘W ni Tab): la pestaña las captura. Un clic afuera cancela.
  - Rechazos con mensaje en la fila, sin grabar: las teclas fijas de xdir (flechas, Enter, Retroceso,
    Inicio/Fin, RePág/AvPág sin modificadores, la escritura para saltar por nombre: letras, números y
    signos sin Ctrl/Alt/Meta) → «Esa tecla ya la usa la lista»; las del sistema (macOS: ⌘Q, ⌘H, ⌘⌥H,
    ⌘Tab, ⌘Espacio, ⌃⌘Q; Windows: Alt+F4, Alt+Tab, Ctrl+Alt+Supr, las de Win+…; Linux: Alt+F4, Alt+Tab)
    → «La usa el sistema»; Shift solo o un modificador solo → se sigue esperando.
- **Choque**: si la combinación ya es de otro comando, la fila dice «⌘K ya es de "Conectar a un
  servidor…"» con **«Reasignar»** (se la saca al otro y se la da a este; el otro conserva sus otros
  atajos o queda sin ninguno) y «Cancelar». Se usa lo que ya devuelve `Keymap::set` (los comandos que
  perdieron la tecla) para escribir también la línea del otro comando.
- **Restablecer todos**: pregunta «¿Volver todos los atajos a los de fábrica? Se borran N atajos
  propios.» (`ask_yes_no`) y quita todas las líneas activas de `[atajos]` (los comentarios quedan).
- **Cómo se guarda** en `config.toml` (§A.9): el atajo propio de un comando es la línea `id = [...]` con
  **la lista completa** de sus atajos (como funciona hoy: escribir un atajo reemplaza los de fábrica);
  quitar todos deja `id = ""`; restablecer quita la línea. Los nombres de tecla se escriben como en la
  plantilla (`"Ctrl+Shift+D"`, `"Meta+Ctrl+N"`, `"Ctrl+Coma"`), con el `Ctrl` lógico (⌘ en macOS).
- Un atajo de `config.toml` con error (línea mal escrita) se muestra arriba de la sección: «config.toml,
  línea 12: tecla desconocida "Hiper"» con «Abrir config.toml».

#### A.8 Red, xtree y Avanzado

- **Red**: ver §B.6.
- **xtree**:

| Opción | Control | Guarda en |
|---|---|---|
| Colorear el mapa por tipo de archivo al abrir | Switch (lo mismo que la tecla T, que hoy no se recuerda) | `config.toml` `[opciones] xtree_por_tipo` (nueva) |
| Disco que se analiza al abrir | Desplegable: «El del sistema» (`DiskScan::default_root`, como hoy) / «El de la carpeta del panel» / «El último analizado» | `config.toml` `[opciones] xtree_abrir` (`"sistema"`, `"panel"`, `"ultimo"`; nueva) |

  Y la lista de teclas de xtree (T, F5, ⌫, A, Esc) solo para ver, con enlace a la sección de atajos.
- **Avanzado**:
  - **«Abrir config.toml»** (el comando `OpenConfig`, que pasa a llamarse «Abrir config.toml»).
  - **Dónde están los archivos**: la carpeta de configuración (`session::config_dir`) con «Mostrar en
    el Finder / el Explorador / la carpeta», y la tabla de archivos de la wiki (`config.toml`,
    `sesion.toml`, `carpetas.toml`, `favoritos.txt`, `grupos.toml`, `historial.txt`, `red.toml`,
    `rutas.txt` en macOS) con qué guarda cada uno.
  - **Modo portable**: dice si está activo («Los archivos se guardan junto a xdir.exe»). En Windows y
    Linux, «Activar el modo portable» copia los archivos de configuración a la carpeta del ejecutable y
    crea `xdir.portable` (pregunta antes; si la carpeta no se puede escribir, lo dice), y «Desactivar»
    borra `xdir.portable` (los archivos quedan); los dos dicen «Se aplica al volver a abrir xdir». En
    macOS solo se informa (ver Decisiones).

#### A.9 Escribir `config.toml` sin perder lo escrito a mano

Módulo nuevo del núcleo, `toml_edit` (al lado de `toml.rs`, sin dependencias), con funciones puras sobre
el texto:

- `set(text, section, key, value) -> String`:
  1. Si hay una línea **activa** `key = …` en `[section]` (la última, si se repite), se cambia solo el
     valor; lo que hay antes de la clave (sangría, comillas de la clave) y el comentario del final de la
     línea quedan.
  2. Si no, y en esa sección hay una línea de **ejemplo comentada** `# key = …` (las de la plantilla), se
     escribe la línea activa justo debajo de ella (el ejemplo queda, como referencia de fábrica).
  3. Si no, se agrega después de la última línea activa de la sección (o después del encabezado).
  4. Si la sección no existe, se agrega al final `\n[section]\n` y la línea.
- `remove(text, section, key) -> String`: borra la línea activa (todas, si se repite); el ejemplo
  comentado queda.
- Lo escribe siempre en el formato de la plantilla (`toml::quote`, listas en una línea) y conserva el
  fin de línea del archivo (`\n` o `\r\n`) y la falta de salto final.
- Las líneas que xdir no entiende (errores de `parse`), las secciones desconocidas y las claves
  desconocidas no se tocan nunca.

Cómo se usa desde `App`:
- Antes de cada cambio se **vuelve a leer** el archivo (si `changed_on_disk`, se aplica primero como hoy
  `reload_config`), se edita el texto leído y se escribe con reemplazo atómico (temporal en la misma
  carpeta + renombrar). Después se actualiza `Config.modified` para que `reload_config` no lo tome como
  un cambio de afuera (y no muestre «Configuración aplicada» por algo que hizo la pestaña).
- Escribir corre en un hilo de trabajo (puede ser una carpeta de red en modo portable); el cambio ya se
  aplicó en memoria. Si falla la escritura, la pestaña muestra «No se pudo guardar config.toml: …» y lo
  que se cambió queda solo hasta cerrar xdir.
- Si el archivo no existe, se escribe primero la plantilla (como `Config::load`) y después el cambio.
- La pestaña **no** tiene botón «Guardar»: cada cambio se guarda solo.
- Tests con textos: comentarios al final, claves entre comillas, `\r\n`, sección repetida, clave al final
  sin salto, línea con error al lado, plantilla entera (cambiar un atajo y volver a leer da el mismo
  `Config` con solo ese cambio; un `remove` después del `set` devuelve el texto original salvo la línea).

#### A.10 La rueda dentada

Queda como panel rápido, igual que hoy, con «Más opciones…» al pie. Lo que se cambia en uno se ve en el
otro al instante (los dos leen los mismos campos de `App`).

### B. Red

#### B.1 Direcciones

Tipo puro del núcleo `net::Address` (módulo `net.rs`, con tests):

- Acepta `smb://servidor/recurso/carpeta`, `\\servidor\recurso\carpeta`, `//servidor/recurso`,
  `afp://servidor/volumen`, `sftp://[usuario@]servidor[:puerto]/ruta`, y un nombre suelto
  (`nas`, `192.168.1.10`) que se toma como `smb://nas`.
- Normaliza (esquema en minúsculas, sin barra final, sin contraseña: `usuario:clave@` → `usuario@`) y
  da la forma de cada sistema: en Windows `\\servidor\recurso\carpeta`; en macOS y Linux la URL.
- `servidor`, `recurso`, `resto`, `usuario` por separado (el recurso puede faltar: «conectarse al
  servidor» y elegir después).
- Qué esquemas acepta cada sistema lo dice el servicio (`supports(scheme)`): Windows `smb`; macOS
  `smb`, `afp`; Linux `smb`, `sftp`, `afp` (si `gvfs` lo tiene). Uno no soportado: «En este sistema no se
  puede conectar a direcciones sftp://».

#### B.2 Servicio del sistema

Un servicio nuevo `sys::Network` (con su falso en `sys::testing`). Todo es **bloqueante y corre en hilos
de trabajo** (`spawn_task`), con un `cancel: &AtomicBool` donde el sistema deja cancelar. Orientativo, el
plan lo afina:

- `connect(owner, &Address, ConnectOptions) -> Result<PathBuf, NetError>`: monta o conecta y devuelve la
  ruta para abrir. `ConnectOptions { drive_letter: Option<char>, reconnect: bool, other_credentials:
  bool }` (las dos primeras solo en Windows).
- `connections() -> Vec<NetConnection>`: lo conectado ahora (dirección, ruta local, letra, si se
  reconecta al iniciar sesión).
- `disconnect(&Path, force: bool) -> Result<(), NetError>`; `NetError::OpenFiles` si hay archivos
  abiertos (se ofrece «Desconectar igual»).
- `browse(tx, cancel)`: manda las máquinas que anuncia la red local a medida que aparecen y desaparecen.
- `shares(&host) -> Result<Vec<String>, NetError>`: recursos de una máquina (puede pedir credenciales).
- `supports(scheme)`, `free_drive_letters()` (Windows).
- `is_remote(&Path) -> bool` **sin E/S**: si una ruta está en una conexión de red, mirando una lista que
  el servicio mantiene (UNC y letras de red en Windows; en macOS y Linux los puntos de montaje de red,
  refrescados en otro hilo al cambiar los discos). Lo usa el núcleo para no tocar esas rutas en el hilo
  de interfaz (§B.7).

| | Conectar y credenciales | Lo conectado | Desconectar |
|---|---|---|---|
| Windows | `WNetAddConnection3W` con `CONNECT_INTERACTIVE` (y `CONNECT_PROMPT` si «otras credenciales» o si falla por acceso), `CONNECT_UPDATE_PROFILE` si «Conectar de nuevo», `lpLocalName` con la letra; el diálogo de credenciales es el de Windows y su «Recordar mis credenciales» las deja en el Administrador de credenciales. Sin letra, como hoy `connect_share` (que pasa a usar este servicio) | `WNetOpenEnumW(RESOURCE_CONNECTED)` y `RESOURCE_REMEMBERED` (unidades recordadas desconectadas) | `WNetCancelConnection2W` con `CONNECT_UPDATE_PROFILE`; `ERROR_OPEN_FILES` / `ERROR_DEVICE_IN_USE` → `OpenFiles`, y con `force` se repite con `fForce = TRUE` |
| macOS | `NetFSMountURLSync` (NetFS.framework) con `kNAUIOptionKey = kNAUIOptionAllowUI`: el sistema muestra su ventana de usuario y contraseña con «Recordar en mi llavero» y, si la URL no trae recurso, la de elegir volúmenes. Monta en `/Volumes/<recurso>` | volúmenes con `NSURLVolumeIsLocalKey = false` (de `mountedVolumeURLs`), con su URL (`NSURLVolumeURLForRemountingKey`) | el `eject` de la tanda D (`unmountAndEjectDeviceAtURL`, que ya acepta volúmenes de red); ocupado → `OpenFiles` con los programas de `lsof` como hoy |
| Linux | `gio mount <url>` en un proceso hijo con `stdin`/`stdout` conectados: si `gio` pregunta usuario, dominio o contraseña, el servicio devuelve `NeedsCredentials` y la interfaz (cuando exista) las pide y el servicio se las pasa por `stdin`; no se guardan. Si el llavero del escritorio ya las tiene, `gvfs` no pregunta. Monta bajo `$XDG_RUNTIME_DIR/gvfs/…` (la ruta local sale de `gio info` de la URL, atributo `standard::target-uri`/`local-path`) | `gio mount -l` (solo los de red: smb, sftp, afp, dav) | `gio mount -u <url>`; ocupado → recorrer `/proc/*/fd` como en la tanda D |

- Tiempo límite: 30 s para conectar (con «Cancelar» en el panel; en Windows y macOS la llamada no se
  puede interrumpir: se abandona el hilo y su resultado se descarta), 10 s para `shares` y
  `connections`, 15 s para desconectar.
- Errores en castellano por código: «No se encuentra el servidor "nas"», «Usuario o contraseña
  incorrectos», «El recurso "fotos" no existe en "nas"», «Se agotó el tiempo esperando a "nas"», «Ya
  hay una conexión con "nas" con otro usuario» (Windows `ERROR_SESSION_CREDENTIAL_CONFLICT`: se ofrece
  desconectar la otra y reintentar).

#### B.3 Conectar a un servidor…

```
┌─ Conectar a un servidor ─────────────────────────── ✕ ┐
│ [smb://nas.local/fotos                         ] [☆]  │
│ Por ejemplo: smb://servidor/recurso o \\servidor\rec. │
│                                                        │
│ Favoritos                                              │
│  ★ NAS — fotos         smb://nas.local/fotos           │
│  ★ Oficina             \\srv01\compartido               │
│ Recientes                                              │
│    sftp://ana@vps/var/www                hace 2 días ✕ │
│                                                        │
│                         [Cancelar]  [Conectar]         │
└────────────────────────────────────────────────────────┘
```

- **⌘K** en macOS / **Ctrl+K** en Windows y Linux (comando `ConnectServer`, id `conectar_servidor`,
  «Conectar a un servidor…»); también en la paleta, en el menú del clic derecho de «Red» en el árbol y del
  fondo de «Este equipo».
- Panel flotante dibujado por xdir (como el de Renombrar varios), centrado sobre los paneles; Esc lo
  cierra. El campo arranca con la dirección de la carpeta del panel si es de red, si no vacío; el foco
  está en el campo. Debajo, el ejemplo según el sistema (en macOS suma `afp://`, en Linux `sftp://`).
- Favoritos y recientes: un clic pone la dirección en el campo, doble clic conecta. ✕ quita un reciente.
  La estrella del campo agrega o quita la dirección de favoritos (pide un nombre opcional en línea).
- **Conectar** (o Enter): valida con `net::Address` (error en la línea de abajo, en rojo, sin cerrar),
  cambia el botón a «Conectando…» con un indicador y «Cancelar», y llama a `Network::connect` en un hilo.
  Las ventanas de credenciales que muestre el sistema quedan por encima de xdir.
- Al conectar: el panel se cierra, la ruta se abre **en una pestaña nueva del panel activo**, la dirección
  pasa a los recientes (al principio; hasta 10, sin repetir) y el árbol la muestra en «Red». Si falla, el
  error queda en el panel y se puede corregir y reintentar.
- Escribir `smb://…` o `\\…` en la barra de dirección hace lo mismo sin abrir el panel (como hoy en
  Windows con `\\`), pero sin sumar el «Conectando…» como panel: se ve en la lista (§B.7).

#### B.4 «Red» en el árbol

```
▾ Red
   ★ NAS — fotos
   🖥 nas.local / fotos                     ⏏
   🖥 Z: compartido (\\srv01)               ⏏
   ─
   🖧 MACBOOK-DE-ANA
   🖧 IMPRESORA-PISO-2
```

- Un nodo raíz «Red» en los tres sistemas, al final como hoy en Windows. Hijos, en tres grupos:
  1. **Favoritos** (★): un clic conecta si hace falta (con el flujo de §B.3, sin abrir el panel salvo que
     falle) y abre la carpeta.
  2. **Conectados** (los de `Network::connections`), con ⏏ al pasar el mouse y siempre en el seleccionado,
     como los discos de la tanda D. Se expanden como carpetas. En Windows las unidades de red también
     siguen en «Este equipo» (como en el Explorador); acá se repiten.
  3. **En la red local**, donde el sistema lo da:
     - **Windows**: los hijos de la Red del Shell (`shell_subfolders(NETWORK_PLACE)`, como hoy), leídos al
       expandir. Un clic en «Red» sigue abriendo la carpeta virtual del Shell en el panel. Depende de que
       la detección de redes esté prendida en Windows; si no hay nada, el grupo no aparece.
     - **macOS**: Bonjour con `NWBrowser` de Network.framework (servicios `_smb._tcp` y
       `_afpovertcp._tcp`), por `browse`. Desde macOS 15 la primera vez el sistema pide permiso de «red
       local» (hace falta `NSLocalNetworkUsageDescription` y `NSBonjourServices` en el `Info.plist`); si el
       usuario lo niega, el grupo no aparece y la sección Red de la configuración lo explica. Un clic en
       una máquina conecta a `smb://máquina` (el sistema ofrece elegir el recurso).
     - **Linux**: `gio list network:///` (lo que anuncian `gvfsd-network`, Avahi y wsdd si están). Sin
       `gvfs-backends`, el grupo no aparece.
     Las máquinas que dejan de anunciarse se quitan; nada de esto bloquea: se llena a medida que llega.
- En macOS y Linux «Red» no abre nada en el panel (solo se despliega); en Windows abre la Red del Shell
  como hoy.
- Nodos nuevos en `tree.rs`: `Kind::NetFavorite`, `Kind::NetConnected` (con ruta), `Kind::NetHost`;
  `Kind::Network` deja de exigir `network_place()` no vacío. Clic derecho: «Conectar a un servidor…»,
  «Desconectar», «Agregar a favoritos» / «Quitar de favoritos», «Copiar la dirección».

#### B.5 Desconectar (⏏) y unidades de red de Windows

- **⏏ y «Desconectar»** (árbol, «Este equipo», menú del clic derecho, comando `Eject` sobre una carpeta de
  red) usan el flujo de Expulsar: `eject_after_jobs` (si hay copias sobre esa conexión, pregunta si se
  cancelan y espera la limpieza) → `release_volume` (suelta guardianes, xtree, vista previa; las pestañas
  en esa conexión pasan a «Este equipo») → `Network::disconnect` en un hilo. `TaskDone` nuevo
  `Disconnected` con el mismo manejo de errores: «No se pudo desconectar "fotos": lo está usando Word» con
  «Reintentar» y, si el sistema lo permite, **«Desconectar igual»** (`force`). En la barra de estado:
  «Se desconectó "fotos"».
- En Windows `Shell::ejectable` sigue sin incluir `DRIVE_REMOTE`; para las conexiones de red el ⏏ sale de
  `Network::connections` (así un disco y una conexión no se confunden).
- **«Conectar unidad de red…»** (solo Windows: comando `MapNetworkDrive`, id `conectar_unidad_red`, sin
  atajo; paleta, clic derecho en «Este equipo» y en «Red»):

```
┌─ Conectar unidad de red ──────────────────────────── ✕ ┐
│ Unidad:   [ Z: ▾ ]                                       │
│ Carpeta:  [\\srv01\compartido               ] [Examinar…]│
│           Por ejemplo: \\servidor\recurso                │
│ ☑ Conectar de nuevo al iniciar sesión                    │
│ ☐ Conectar con otras credenciales                        │
│                              [Cancelar]  [Finalizar]     │
└──────────────────────────────────────────────────────────┘
```

  - Unidad: las letras libres (`free_drive_letters`), de Z: hacia abajo, con Z: (o la primera libre)
    elegida, como el Explorador. Carpeta: texto con autocompletar de favoritos y recientes; «Examinar…»
    abre `pick_folder` (el diálogo del sistema deja navegar la Red). Las dos casillas como el Explorador
    (la primera prendida). «Finalizar» llama a `connect` con la letra; al terminar abre la unidad en una
    pestaña nueva y la suma a los recientes. Los errores quedan en el panel.
- **«Desconectar unidad de red…»** (solo Windows: `DisconnectNetworkDrive`, id `desconectar_unidad_red`):
  panel con la lista de unidades de red (letra, carpeta, «desconectada» si es una recordada que hoy no
  está) con casillas; «Desconectar» corre el flujo de ⏏ para cada una. Sin unidades: «No hay unidades de
  red».
- En macOS y Linux esos dos comandos no existen (`command_available`).

#### B.6 Sección «Red» de la configuración

- **Favoritos**: lista con nombre (editable en línea), dirección (editable, validada con `net::Address`),
  ↑/↓ para ordenar (el orden es el del árbol y del panel) y ✕ para quitar; «Agregar…» abre una fila
  vacía.
- **Recientes**: lista con fecha, ★ para pasar a favoritos, ✕ para quitar, «Borrar los recientes».
- **Conectados ahora**: lo de `connections()` con ⏏ (leído en otro hilo al abrir la sección).
- Texto fijo: «xdir no guarda contraseñas: las guarda el sistema (Llavero / Administrador de
  credenciales / llavero del escritorio)», con el botón para abrir ese lugar donde se puede (macOS:
  Acceso a Llaveros; Windows: `control /name Microsoft.CredentialManager`).
- macOS: si se negó el permiso de red local, «Para ver las máquinas de la red local, permití "Red local"
  en Ajustes del Sistema → Privacidad» con el botón que abre ese ajuste.
- Se guarda en **`red.toml`** (nuevo, en la carpeta de configuración, lo escribe xdir):
  `[[favorito]] nombre = "NAS — fotos"  direccion = "smb://nas.local/fotos"` y `[[reciente]] direccion =
  "…"  fecha = 1731234567`. Se lee al arrancar y se relee al volver a la ventana si cambió (como
  `grupos.toml`). Las unidades de red de Windows **no** van ahí: las recuerda Windows.

#### B.7 Que la red nunca trabe la ventana

- Todo lo de §B.2 corre en hilos de trabajo; el hilo de interfaz solo recibe `TaskDone`.
- **Auditoría del hilo de interfaz** (parte del plan): las llamadas que hoy pueden tocar el servidor
  con una ruta de red y corren en el hilo de interfaz se saltean cuando `Network::is_remote(ruta)` (sin
  E/S) o pasan a un hilo. Conocidas: `long_path` en `Tab::new` y `Tab::navigate` (en Windows
  `GetLongPathNameW`; con una ruta remota no se llama: las rutas de red no tienen nombres 8.3 que valga la
  pena expandir), `true_case`, `drive_missing`, `cloud_only`, los `is_dir`/`exists`/`metadata` de las
  acciones y del árbol (`reveal`, `refresh_drives`), `volume_space` de la barra de estado, `drive_label`
  de una unidad de red. El plan las lista con `grep` y prueba cada una con un falso que se cuelga.
- **La lista muestra «Conectando…»**: si la carga de una carpeta remota no trajo nada en ~0,5 s, la lista
  muestra «Conectando con "nas"…» con un indicador (en vez de la lista vacía); a los 20 s sin respuesta,
  «"nas" no responde» con «Reintentar» y «Cerrar la pestaña». Mientras tanto se puede cambiar de pestaña,
  de panel, cerrar la pestaña o salir: la carga sigue en su hilo y su resultado se descarta (la
  generación de la carga ya lo hace).
- Los guardianes de carpetas remotas (`watch`) se crean en otro hilo; si el sistema no avisa cambios en
  esa red (SMB viejo, gvfs), la pestaña no se recarga sola y F5 recarga.
- Miniaturas, vista previa, metadatos e información de la tanda C/D sobre carpetas remotas ya corren en
  hilos; se suma un tiempo límite por elemento para que un archivo colgado no frene la cola.
- Al salir de xdir no se espera a ningún hilo de red.

### C. Comandos y atajos nuevos

| Comando (id) | Windows / Linux | macOS | Hoy |
|---|---|---|---|
| Configuración (`configuracion`) | Ctrl+, | ⌘, | nuevo (en macOS ⌘, abría config.toml) |
| Configuración: Atajos de teclado (`configuracion_atajos`) | sin atajo | sin atajo | nuevo |
| Abrir config.toml (`abrir_configuracion`) | sin atajo | sin atajo | igual, cambia el nombre |
| Conectar a un servidor… (`conectar_servidor`) | Ctrl+K | ⌘K | nuevo |
| Conectar unidad de red… (`conectar_unidad_red`) | sin atajo (solo Windows) | — | nuevo |
| Desconectar unidad de red… (`desconectar_unidad_red`) | sin atajo (solo Windows) | — | nuevo |
| Expulsar (`expulsar`) | sin atajo | sin atajo | también desconecta carpetas de red |

El id `abrir_configuracion` queda para `OpenConfig` (no se renombra: hay `config.toml` que ya lo usan). Si
alguien había puesto Ctrl+K o Ctrl+Coma en otra acción en `[atajos]`, lo conserva (como siempre, gana lo
de `config.toml`).

### D. Wiki

Al terminar, en `Runick93/xdir.wiki`:
- **Configuracion.md**: sección nueva «La pestaña Configuración» (cómo se abre, secciones, buscador, qué
  se guarda dónde); en «config.toml» cambiar «xdir nunca lo reescribe» por «xdir solo cambia las líneas de
  lo que tocás en la pestaña; comentarios y lo demás quedan»; opciones nuevas `xtree_por_tipo` y
  `xtree_abrir`; `red.toml` en «Otros archivos»; el modo portable desde Avanzado; Linux en «Dónde se
  guarda».
- **Atajos-de-teclado.md**: Ctrl+, / ⌘, y Ctrl+K / ⌘K, el grupo Red, y «Cambiar un atajo» desde la
  pestaña (grabar, reasignar, restablecer), con `config.toml` como alternativa. El orden de los grupos es
  el mismo de la tabla del núcleo.
- **Red.md** (página nueva, en `_Sidebar.md`): conectar a un servidor, direcciones por sistema,
  favoritos y recientes, unidades de red de Windows, «Red» en el árbol (qué se ve en cada sistema y el
  permiso de red local de macOS), desconectar, credenciales («xdir no guarda contraseñas»), carpetas que
  no responden.
- **Funciones.md**: una línea por función nueva con enlace.

## Restricciones

- `xdir-core` sin dependencias ni `cfg` de sistema en producción; lo de cada sistema detrás de servicios
  en `xdir_core::sys` (`Network` nuevo; con falso en `sys::testing`, que puede colgarse a pedido para
  probar §B.7). `toml_edit` y `net::Address` son lógica pura del núcleo con tests.
- macOS: NetFS y Network.framework se enlazan como frameworks del sistema desde `xdir-mac` (con
  `block2`/`dispatch2`, que ya están); si hiciera falta un crate de bindings nuevo, preguntar antes.
  `Info.plist` con `NSLocalNetworkUsageDescription` («xdir muestra las computadoras de tu red local en
  "Red"») y `NSBonjourServices` (`_smb._tcp`, `_afpovertcp._tcp`) en `package-mac.sh`.
- Windows: `Win32_NetworkManagement_WNet` ya está en las features de `windows`.
- Linux: `gio` (de `glib`/`gvfs`) en tiempo de ejecución, sin enlazar; si no está, los comandos de red
  dicen «Hace falta gvfs para conectarse a servidores».
- Toda E/S de red en hilos de trabajo; el hilo de interfaz nunca espera a la red (§B.7).
- Contraseñas: nunca en archivos ni en la sesión; en Linux pasan de la interfaz al `stdin` de `gio` y se
  sueltan (la memoria del campo se pisa al cerrar el panel).
- Castellano en toda la interfaz; atajos y opciones documentados en la wiki al terminar.
- Antes de programar, las pantallas en FigCraft (`docs/design/xdir.figcraft`), en oscuro y claro:
  1. Pestaña Configuración — **Apariencia** (con la pestaña en la tira, el buscador y la lista de
     secciones).
  2. Configuración — **buscador con resultados** de varias secciones.
  3. Configuración — **Atajos de teclado**: lista normal con un atajo cambiado; una fila **grabando**
     («Apretá la combinación…»); una fila con **choque** («⌘K ya es de…» con «Reasignar»); un rechazo
     («La usa el sistema»).
  4. Configuración — **Red** (favoritos, recientes, conectados, el aviso de contraseñas).
  5. Configuración — Lista y vistas, Árbol, Vista previa, xtree y Avanzado (pueden ir en una sola hoja).
  6. **Conectar a un servidor**: vacío, con favoritos y recientes, conectando, con error.
  7. **Conectar unidad de red** (Windows) y **Desconectar unidad de red**.
  8. **Árbol con «Red»**: favoritos, conectados con ⏏ (normal y al pasar el mouse) y máquinas de la red.
  9. La lista de una carpeta remota **«Conectando…»** y **«no responde»**.
  10. La rueda dentada con «Más opciones…».
- Tests con `install_fakes()`, `FakeHost`, `RecordingPainter`: `toml_edit` (§A.9), `net::Address`, el
  editor de atajos (grabar, choque, reasignar, restablecer, lo que se escribe), la pestaña única (abrir dos
  veces, mover entre paneles, sesión), la búsqueda, desconectar con copias en curso, y que ninguna acción
  sobre una ruta remota llame al servicio colgado en el hilo de interfaz. `cargo test` y `cargo check
  --all-targets` limpios en los 3 sistemas; prueba de humo en Mac (⌘, ; cambiar un atajo y ver
  `config.toml` con sus comentarios intactos; ⌘K a un SMB del NAS con contraseña del Llavero; ⏏; tirar la
  red con una carpeta remota abierta y comprobar que la ventana sigue respondiendo).
- PRs propios contra `develop` a medida que se terminan las partes; versión de Mac instalada al terminar
  la tanda.

## Lo que queda para quien construya

- La forma exacta de `SettingsView` y de los controles nuevos que hagan falta (segmentado, deslizador,
  desplegable, chip de atajo): si se generalizan para otros paneles o quedan en la vista de
  configuración.
- La tabla de opciones (título, descripción, palabras clave, sección, de dónde lee y a dónde escribe):
  conviene que sea datos en el núcleo, así el buscador y la wiki salen de lo mismo.
- La lista exacta de combinaciones del sistema que se rechazan al grabar.
- Los textos de error de red por código de cada sistema.
- Cómo se cuelga el falso de `Network` para los tests de §B.7 y la lista final de llamadas del hilo de
  interfaz a corregir.
- Orden sugerido de las partes: `toml_edit` → pestaña `Location::Settings` con Apariencia, Lista, Árbol,
  Vista previa y Avanzado → Atajos → `net::Address` + servicio `Network` (Windows y macOS) + Conectar a un
  servidor → auditoría de §B.7 y «Conectando…» → Desconectar con ⏏ → unidades de red de Windows → «Red» en
  el árbol (conectados y favoritos; después descubrimiento) → sección Red → xtree → Linux → wiki.

## Decisiones tomadas al escribir el diseño (llamadas de Claude, corregibles)

1. **La configuración es una `Location` nueva** (`Location::Settings`), no una vista encima de los paneles
   como xtree: así hereda gratis la tira de pestañas, el arrastre entre paneles, Ctrl+W, «Reabrir» y la
   sesión. Cuesta revisar cada `match` sobre `Location`.
2. **`config.toml` se edita por líneas** (§A.9), no se reescribe ni lleva una sección administrada: es lo
   único que no pierde lo escrito a mano y no duplica claves.
3. **Los switches de la sesión siguen en `sesion.toml`**; la pestaña solo los muestra y los cambia. No se
   mudan a `config.toml` (eso cambiaría dónde vive cada cosa y la migración no vale la pena ahora).
4. **Un atajo propio se guarda como la lista completa del comando**, igual que hoy en `[atajos]`. Reasignar
   escribe dos líneas (la del comando que gana y la del que pierde).
5. **Esc cancela la grabación** y no se puede asignar desde la pestaña (sí desde `config.toml`). Las teclas
   sin modificador que usa la lista (letras para saltar por nombre, flechas, Enter) y las del sistema se
   rechazan con mensaje.
6. **Animaciones es de tres estados** (Sistema / Sí / No) porque así es `[opciones] animaciones`; el Tema
   también (Sistema = `dark_mode = None`), algo que la rueda dentada hoy no deja volver a elegir.
7. **xtree suma dos opciones nuevas** en `[opciones]` (`xtree_por_tipo`, `xtree_abrir`): hoy xtree no
   tiene ninguna opción guardada y el alcance pedía «sus opciones».
8. **Modo portable solo se activa desde la pestaña en Windows y Linux**: en macOS el ejecutable está dentro
   del `.app` firmado (escribir ahí rompe el sellado y `/Applications` puede no ser escribible); en macOS
   solo se informa.
9. **Favoritos y recientes de red en `red.toml`**, no en `config.toml`: los recientes cambian con cada
   conexión y no tiene sentido tocar el archivo del usuario por eso.
10. **`sftp://` solo en Linux** (lo monta `gvfs`); en macOS el sistema no monta SFTP sin programas de
    terceros y en Windows tampoco: esa dirección da un error claro. `afp://` en macOS (y en Linux si gvfs
    lo trae).
11. **En Linux la contraseña la pide xdir y pasa directo a `gio`**, sin guardarse, porque `gio mount` desde
    un proceso sin terminal no tiene otra forma de preguntar; si el llavero del escritorio ya la tiene, no
    se pregunta. Como Linux no tiene interfaz en esta tanda, el servicio queda listo con
    `NeedsCredentials`.
12. **Descubrimiento por sistema**: Windows con la Red del Shell (lo de hoy); macOS con Bonjour por
    `NWBrowser` (con el permiso de red local de macOS 15); Linux con `gio list network:///`. Si el sistema
    no da nada, el grupo no aparece, sin error.
13. **Conectar abre una pestaña nueva** (no reemplaza la carpeta del panel), igual que abrir la
    configuración.
14. **Las unidades de red de Windows siguen en «Este equipo» y además en «Red»**, como el Explorador; el ⏏
    de las conexiones sale de `Network::connections`, no de `Shell::ejectable`.
15. **«Desconectar igual»** (forzar) se ofrece solo cuando el sistema lo permite (Windows `fForce`; macOS
    `unmount` forzado) y siempre después de un primer intento normal.
16. **Paneles dibujados** para Conectar a un servidor y las unidades de red (como Renombrar varios y
    «Acerca de»), no ventanas del sistema; las ventanas de credenciales sí son las del sistema.

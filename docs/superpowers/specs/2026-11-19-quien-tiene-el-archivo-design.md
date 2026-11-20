# Quién tiene tomado el archivo — diseño

Fecha: 2026-11-19. Tamaño: cambio mediano (núcleo + Windows, con una ventana nueva). Estado: pendiente de aprobación.

## Intención

Cuando una acción de xdir (borrar, mover, copiar, renombrar, mandar a la Papelera…) falla porque
otro proceso tiene tomado el archivo, hoy el aviso dice solo «Lo está usando otro programa» (en la
barra de estado o en la fila de la ventana de copia). El usuario no sabe cuál, y un usuario nuevo
puede ni ver el aviso. Queremos que xdir **lo diga solo y que se note**: una **ventana emergente**
con qué proceso es y qué usuario lo ejecutó.

Lo pidió Leandro, que trabaja en Windows y desconoce si Linux y macOS pueden dar esos datos.

## Objetivos

- Ante un `ErrKind::InUse` definitivo, abrir una **ventana emergente propia** que nombre el
  **proceso** (y su usuario) que tiene el archivo.
- La ventana es **independiente de la ventana de copia**: la misma ventana para borrar, renombrar,
  Papelera, copiar, mover, etc. No se acopla a `transfer::view`.
- Funciona en **Windows** desde el primer día.
- El núcleo queda **neutro**: cada sistema decide si lo implementa. Sin implementación, todo sigue
  como hoy (aviso genérico). Linux y macOS quedan sin implementar, a propósito.

## No objetivos

- No cerrar ni matar procesos (ni un botón para eso): el riesgo de perder datos sin guardar.
- No hay botón ni comando «¿Quién lo usa?» a pedido: solo automático ante el fallo.
- No detectar quién tiene una **carpeta** como carpeta actual (una consola abierta ahí) ni los
  bloqueos que el sistema hace por dentro. Si el sistema no informa quién es, no se abre la ventana
  y queda el aviso de hoy.
- No hay cartel al pasar el mouse para esto: la ventana lo reemplaza.
- Linux y macOS: no se implementan ahora (en Linux casi nunca un archivo abierto bloquea; en macOS
  hace falta `libproc` y los permisos limitan lo visible).

## Anti objetivos (sería un fracaso aunque «funcione»)

- Frenar o enlentecer algo en el uso normal: la consulta se hace **solo después del fallo definitivo**.
- Colgar la interfaz: la consulta corre en el hilo del trabajo, nunca en el de la ventana.
- Acusar a un proceso equivocado con seguridad. Si no se sabe, se calla o se dice «usuario desconocido».
- Abrir una ventana por cada archivo de una copia grande (50 bloqueados no son 50 ventanas).

## Enfoques considerados para averiguar quién

| | Qué es | Pro | Contra |
|---|---|---|---|
| **A. Restart Manager** (elegido) | API documentada de Windows: dado un archivo, lista los procesos que lo usan (nombre, PID, servicio). El usuario sale del token del proceso. | Estable, rápida, documentada, sin privilegios especiales | No ve carpetas usadas como carpeta actual ni bloqueos internos del sistema; el usuario de un proceso ajeno puede no poderse leer |
| B. Enumerar los handles del sistema (`NtQuerySystemInformation`) | Recorre todos los handles abiertos | Lo encuentra todo, incluso carpetas | API no documentada, tarda segundos, puede colgarse con ciertos handles, pide privilegios para ver procesos ajenos |
| C. A y, si no hay nada, B | Mixta | Lo más completo | El doble de código y de pruebas |

Eligió Leandro: **A**. Si más adelante molestan las carpetas, B se suma como segundo paso.

## Diseño

### 1. Averiguar quién (núcleo + Windows)

**Núcleo (`xdir-core`, `sys/copy.rs`)**

- Un tipo nuevo `Holder { name, exe: Option<String>, pid, user: Option<String> }` (el proceso que
  tiene el archivo: `name` es el nombre amigable que da el Restart Manager, «Microsoft Excel»;
  `exe`, el del ejecutable, «EXCEL.EXE»).
- `CopyError` gana un campo `holders: Vec<Holder>`, vacío por defecto (`CopyError::new` lo deja vacío,
  así los sitios que lo crean hoy no cambian). `ErrKind::InUse` **no cambia**.
- `CopyPrims` gana un método con valor por defecto: `fn holders(&self, path: &Path) -> Vec<Holder>`
  que devuelve vacío. Quien lo implementa opta por la función.
- Las operaciones llaman a `holders` **solo cuando el error `InUse` ya es definitivo**: el motor
  reintenta un rato los «ocupado por un momento» (`engine.rs`, `transient`) y no se consulta en cada
  reintento.

**Windows (`xdir-windows`, `ops/prims.rs`)**

- `WinCopyPrims::holders` usa el Restart Manager (`RmStartSession`, `RmRegisterResources`,
  `RmGetList`, `RmEndSession` siempre, también si falla).
- El usuario: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` → `OpenProcessToken` →
  `GetTokenInformation(TokenUser)` → `LookupAccountSidW`. Si algo falla, `user = None`.
- Hace falta sumar el feature `Win32_System_RestartManager` a la dependencia `windows` del crate
  (los de `Win32_System_Threading` y `Win32_Security` ya están).

### 2. La ventana emergente

Es una **ventana auxiliar** como la de copia: del sistema, dueña la principal, sin redimensionar
(el alto lo pone el contenido), centrada sobre xdir, y el núcleo dibuja su contenido. Hoy el motor
de ventanas auxiliares (`WindowHost::aux_*`, `auxwin.rs`) solo conoce la de copia: se generaliza
para admitir una segunda ventana, con su propio estado (`App` guarda el diálogo «en uso» aparte de
`transfers`) y su propia función de dibujo. **No toca `transfer::view`.**

Contenido (el mismo estilo que la ventana de copia, 560 de ancho):

```
┌ No se pudo eliminar «informe.xlsx» ────────┐
│  Lo está usando otro programa:             │
│   Microsoft Excel (EXCEL.EXE)              │
│   PID 4812 · usuario PC\Leandro            │
│  Cerralo y reintentá.                      │
│                     [Reintentar] [Cerrar]  │
└────────────────────────────────────────────┘
```

- **Una ventana por evento, no por archivo.** Una acción suelta (renombrar, borrar uno) abre la
  ventana al fallar. Un trabajo de varios archivos (borrar varios, copiar, mover) abre **una sola**
  ventana **al terminar o cortarse el trabajo**, con la lista de lo bloqueado: cada archivo con su
  proceso y usuario, con scroll o «y N más» si son muchos. Si el mismo proceso tiene varios
  archivos, se agrupan bajo él.
- **Reintentar** vuelve a hacer lo que falló, con el mecanismo de reintento que ya tiene cada
  origen (una acción suelta se repite; el trabajo de copia reintenta solo lo que falló, como el
  «Reintentar» que ya existe). **Cerrar** la oculta.
- **Sin holders** (no se pudo averiguar quién, o el sistema no lo implementa): no se abre la ventana
  y queda el aviso de hoy. La ventana existe para decir algo que el aviso no dice.
- **Usuario desconocido**: se muestra el proceso sin usuario y «usuario desconocido» donde iría.
- **Sistemas sin ventana auxiliar para esto** (Linux, macOS ahora): el motor de ventanas lo ignora
  y queda el aviso de hoy.
- La ventana de copia **no cambia**: sigue mostrando «En uso por otro programa» en la fila de sus
  errores.

## Pruebas

- Núcleo: con prims de prueba que devuelven holders, los textos de la ventana (uno, varios,
  agrupados, sin usuario) y que no se abre sin holders. El motor solo consulta tras agotar los
  reintentos. Una copia con muchos bloqueados abre una sola ventana, al final.
- Windows: una prueba de integración abre un archivo sin compartirlo desde el propio proceso de la
  prueba y comprueba que `holders` devuelve ese proceso (PID propio) con el usuario actual.
- Ventana: la de copia sigue funcionando igual (los tests de `transfer::view` y de `auxwin` no
  cambian de comportamiento).

## Decisiones mías (para que las revises)

- Una ventana **al terminar** el trabajo en copias y borrados de varios archivos, no al primer
  fallo, para no interrumpir a mitad.
- **Reintentar** incluido, apoyado en el reintento de cada origen.
- **No se abre sin holders**: queda el aviso de hoy.
- Se agrupan los archivos por proceso.

## Queda para quien lo construya

- Cómo generalizar `aux_*` para una segunda ventana (un identificador de ventana, o dos juegos de
  métodos), y el lugar exacto de las llamadas a `holders`.
- La redacción final de los textos y cuál nombre mostrar (el amigable del Restart Manager o el del
  `.exe`).
- El límite de cuántos archivos se listan antes de pasar a scroll o «y N más».

## Cambio de diseño (2026-11-19, después de probarlo)

Al probarlo con borrar, la ventana nueva se abría pegada a la de errores de la ventana de copia
(«Terminó con 1 error») y las dos se pisaban: molesto. Decisión de Leandro: **los trabajos del motor
(borrar, copiar, mover) muestran quién lo tiene dentro de la fila del error de la ventana de copia**,
sin ventana nueva. La ventana nueva queda **solo para las operaciones sueltas** (renombrar y demás,
las que no tienen ventana de errores).

- Fila de error con holders: a la derecha, en vez de «Lo está usando otro programa», dos líneas:
  «En uso por PowerShell 7» (y «y N más» si hay otros programas) y, debajo, en texto secundario,
  «PID 24012 · usuario DEV-WKS-01\Leandro-Dev» (o «usuario desconocido»), con puntos suspensivos si
  no entra. Las filas sin holders no cambian.
- El motor ya guarda los holders en `JobError.holders`; no cambia nada en Windows.
- Se saca el camino «trabajo → ventana nueva» (`InUseDialog::from_job`, `InUseRetry::Job`, el cierre al
  olvidar el trabajo, la apertura en `on_job_done`) y lo que solo servía a eso.
- «Reintentar» de la ventana de copia sigue siendo el reintento de los trabajos.

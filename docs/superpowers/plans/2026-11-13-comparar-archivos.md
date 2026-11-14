# Comparar archivos y suma de verificación (#59) — plan de implementación

**Spec:** `docs/superpowers/specs/2026-11-13-comparar-archivos-design.md`.

Cada paso va con su prueba primero (falla), después el cambio, y un commit propio. Nada de `cfg` de
sistema en `xdir-core`; las rutas de las pruebas con `Path::join`; las pruebas con tiempos esperan
una condición (nunca el orden de llegada). Las pantallas de FigCraft se dibujan después (no se toca
`docs/design/xdir.figcraft` en esta rama).

## Pasos

### P1 — Crate `xdir-hash`
- `crates/xdir-hash` con `sha2` y `md-5` (RustCrypto, `default-features = false`, versiones fijas en
  `Cargo.lock`). `hash_reader(lector, algoritmo, avance)`: lee por bloques de 1 MB, `avance(bytes)`
  devuelve `ControlFlow` (cortar = `Ok(None)`), hexadecimal en minúscula.
- Pruebas: vectores conocidos de SHA-256 y MD5 (vacío, «abc», el de 448 bits, un millón de «a»),
  lectura en pedazos irregulares, avance por bloque y corte.

### P2 — Servicio `sys::Hashes`
- `xdir-core/src/sys/hashes.rs`: `HashAlgo` (SHA-256 / MD5, rótulo y largo en hexadecimal) y el
  trait `Hashes` (`open` para leer por bloques, `hash` con avance y corte). `Services::hashes`.
- `xdir-hash` lo implementa (`SystemHashes`, con `std::fs`); Windows, macOS y Linux lo instalan.
- Falso `FakeHashes` en `sys::testing`: lee del disco falso de la copia o del real, anota lo remoto en
  el hilo de interfaz (`FakeNetwork::note_io`), duerme si la ruta está «colgada» y da una suma
  determinística del largo de cada algoritmo.
- Pruebas: el falso con la ruta colgada, la implementación real contra los vectores (xdir-hash).

### P3 — Comparación byte a byte (núcleo)
- `xdir-core/src/compare.rs`: `compare_readers` (bloques de 1 MB de los dos a la vez, primer byte
  distinto, corte, avance), `Source` (disco o elemento de un comprimido, que se extrae con
  `archive_nav::temps` y sus límites), `compare_files` (tamaño distinto sin leer; mismo archivo) y
  `run_watched` (hilo propio; sin avance en `net::ITEM_LIMIT` en la red: «"srv" no responde»).
- Pruebas: iguales, distintos en el primer/último byte y en el segundo bloque, tamaños distintos sin
  abrir, corte, avance, lectura que se cuelga en la red, elemento de un comprimido (extraído con su
  límite y su origen), solo en la nube.

### P4 — Comando `comparar_contenido`
- `Command::CompareContents` («Comparar contenido», sin atajo, grupo Archivos); paleta y menú del
  archivo (también dentro de un comprimido). Disponible con exactamente dos archivos: dos en el panel
  activo, o uno en el activo y uno en el otro panel cuando se ven dos. Si no, aviso en la barra de
  estado.
- Tablas de comandos de Windows y macOS, editor de atajos y tabla de la wiki.

### P5 — Panel de resultado
- `RightPanel::Compare` (en el lugar de la vista previa, como información y renombrar varios):
  `compare_draw.rs` (dibujo y clics puros) y `app/compare.rs` (estado, hilo, avance con
  `WM_APP_COMPARE`, Cancelar, Esc, «Calcular SHA-256» de cada uno con «Copiar»).
- Auditoría de la D+: `remote_navigation_no_ui_io` suma comparar dos archivos de red.

### P6 — «Suma de verificación» en el panel de información
- `Section::Checksum` (plegada de fábrica; bit nuevo al final para no mover los guardados),
  `Body::Checksum`: selector SHA-256 / MD5, «Calcular», avance y Cancelar, valor en hexadecimal con
  «Copiar», caja «Comparar con…» (`parse_expected`: espacios y mayúsculas no cuentan, `hash  nombre`,
  el largo elige el algoritmo) con «Coincide» / «No coincide». Se cancela al cambiar de archivo o al
  cerrar. Dentro de un comprimido, el panel muestra el elemento elegido con su suma.
- Auditoría de la D+: calcular la suma de un archivo de red.

### P7 — CHANGELOG
- Línea en «Sin publicar» › Nuevo.

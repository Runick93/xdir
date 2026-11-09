# xtree: cronómetro y porcentaje de avance — diseño corto

## Intención

Que al escanear con xtree se vea cuánto lleva el escaneo y, cuando se puede saber, cuánto falta, y que al
terminar quede el tiempo total. Pedido por el usuario; aprobado tal como se propuso.

## Qué hace

- **Cronómetro.** Mientras escanea, junto al estado: «Escaneando… 0:42» (m:ss; desde una hora,
  «1:02:15»). Al terminar: «Análisis completado en 1:23», en la barra de xtree (donde hoy dice «Escaneando
  el disco…») y en el texto de estado de xdir (`status_text`). Si el escaneo se corta (se cierra o se cambia
  de raíz), «Detenido a los 0:37». Cuenta desde el inicio del escaneo, incluida la lectura de la MFT y el
  ayudante elevado en Windows. El cuadro se repinta al menos una vez por segundo mientras escanea (para que
  el reloj avance aunque no lleguen datos).
- **Porcentaje.** Solo cuando la raíz es un volumen entero (`DiskScan::whole_volume(root)`): bytes leídos ÷
  espacio usado del volumen (`volume_space`: total − libre), mostrado como «Escaneando… 37 % · 0:42» y como
  una barra de avance fina (alto 3, color de acento, sobre el borde inferior de la barra superior de xtree).
  Tope en 99 % hasta que termina (APFS con clones/instantáneas, hardlinks o archivos que cambian pueden
  hacer que no coincida). En carpetas que no son un volumen entero no hay porcentaje: se ven archivos y
  tamaño leídos como hoy, más el cronómetro. La fase MFT de Windows conserva su porcentaje propio.
- Formato de los números en castellano (coma decimal no hace falta: el porcentaje es entero).

## Enfoques

Una sola forma razonable: guardar el `Instant` de inicio y el de fin en la vista (`Xtree`), con la duración
calculada por una función pura de formato; el total para el porcentaje sale de `volume_space`, que la vista
ya consulta. Se descartó estimar el porcentaje de carpetas con el tamaño de un escaneo anterior (no pedido;
se puede sumar después).

## No objetivos

- Tiempo restante estimado.
- Guardar historial de escaneos.
- Cambiar el escáner (`scan.rs`) o los servicios `DiskScan`.

## Restricciones

- `xdir-core` sin dependencias ni `cfg` de sistema; tests con `install_fakes()` y `FakeDiskScan`.
- `cargo test` y `cargo check --all-targets` limpios en Windows, macOS y Linux; CI verde.
- FigCraft: las pantallas T01/T03 con el cronómetro y el porcentaje se actualizan **después** de mergear la
  tanda B (que también modifica `docs/design/xdir.figcraft`; el archivo es binario y no se puede fusionar).
- Castellano; Conventional Commits sin atribución a Claude; PR propio contra `develop`; al mergear, versión
  de Mac instalada en `/Applications`.

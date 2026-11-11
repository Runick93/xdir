# Navegar dentro de zips — diseño

Fecha: 2026-11-10. Aprobado por el usuario (cambio chico, fuera de las tandas; la tanda D lo había dejado afuera en su punto 14).

## Por qué

Hoy el doble clic en un `.zip` lo abre con el programa del sistema. El usuario quiere ver el contenido y navegarlo dentro de xdir, como una carpeta más.

## Qué hace

- **Doble clic (o Enter) en un archivo comprimido** de los formatos que `xdir-archive` ya lee (`Archives::formats()`: zip, tar, tar.gz, …) lo abre como carpeta en el mismo panel. Barra de dirección: `Descargas › fotos.zip › 2024 › viaje`. Funcionan subir, Atrás/Adelante, abrir en pestaña nueva, la sesión (al reabrir xdir vuelve adentro del zip si sigue existiendo; si no, a la carpeta del zip) y la búsqueda del panel.
- **Columnas:** Nombre, Tamaño, Tamaño comprimido, Fecha de modificación (hora local, como al extraer). Orden y agrupar como en cualquier carpeta. Carpetas implícitas (entradas `a/b/c.txt` sin entrada `a/`) se muestran igual.
- **Abrir un archivo de adentro:** se extrae solo ese archivo a una carpeta temporal propia de xdir (por zip) y se abre con su programa. La vista previa, la Galería y la vista rápida usan esa misma extracción temporal, con el límite de tamaño que ya tiene la vista previa. Las temporales se borran al cerrar xdir (y al arrancar, las que hayan quedado).
- **Sacar cosas:** copiar (Ctrl+C / ⌘C → pegar en otro panel) o arrastrar desde el zip extrae con el motor de copia: ventana de copia, conflictos (Reemplazar / Conservar ambos / Omitir / Combinar), avance y cancelar. «Extraer aquí» sigue igual.
- **Solo lectura:** dentro del zip no se puede borrar, renombrar, cortar, pegar, crear carpeta ni archivo, comprimir ni renombrar varios. Esas entradas del menú aparecen deshabilitadas; si se intenta por atajo, un aviso dice «El contenido de un archivo comprimido es solo de lectura».
- **Zip con contraseña o dañado:** no se abre adentro; se ofrece «¿Abrir con el sistema?» (Sí / No), como hoy al extraer.
- **Zip dentro de zip:** se puede entrar (se extrae el zip interior a la temporal y se navega desde ahí, mostrando la ruta completa).
- **Doble clic con Alt (Windows/Linux) / ⌥ (Mac):** abre el archivo comprimido con el sistema, para quien lo prefiera.

## Seguridad

- Los nombres de las entradas se validan con las mismas reglas que al extraer (`resolve_rel`, sin `..`, sin rutas absolutas, sin `:` en Windows, nombres reservados de DOS). Una entrada con nombre inválido se muestra pero no se puede abrir ni copiar (aviso).
- Los enlaces simbólicos dentro del zip se muestran como enlaces y no se siguen.
- Listar no bloquea el hilo de la interfaz: se lee en un hilo aparte, con «Cargando…» mientras tanto y cancelación al salir.
- Límite de entradas listadas: el mismo que ya usa el motor para un zip con muchas entradas; un zip bomba no infla la memoria (no se descomprime para listar).

## Fuera de esto

- Escribir dentro del zip (meter, borrar, renombrar): el usuario lo pidió para una tanda futura (anotado en la nota del vault, pendientes).
- Formatos que `xdir-archive` no lee hoy (7z, rar).

## Arquitectura (decisión del controlador)

- `xdir-core`: una ubicación nueva para «dentro de un archivo comprimido» (archivo + ruta interior), su listado, sus migas y su lugar en la sesión, sin dependencias nuevas. El listado llega por el servicio `sys::Archives` existente (con un método para listar sin extraer si no alcanza el actual `list`) y la extracción de un elemento por el mismo servicio. Fakes en `sys::testing`.
- Las plataformas solo conectan el servicio (ya lo hacen) y las temporales.
- Pantallas en FigCraft (rama aparte): carpeta dentro de un zip (oscuro y claro), menú con las acciones deshabilitadas y el aviso de solo lectura, y «Cargando…».

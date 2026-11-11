# CHANGELOG y pestaña Novedades — plan de implementación

**Spec:** `docs/superpowers/specs/2026-11-10-novedades-design.md`.

**Architecture:** el texto de `CHANGELOG.md` entra al binario con `include_str!` desde `xdir-core`
(como el logo de `about.rs`). Un parser propio (`changelog.rs`, sin dependencias) arma bloques
(versión, subtítulo, viñeta con nivel, párrafo) con tramos normal / **negrita** / `código`; lo
desconocido queda como texto plano. La pestaña es una ubicación nueva, `Location::Changelog`
(`xdir:novedades` en la sesión), que como `Location::Settings` no se carga ni se vigila y dibuja
encima del lugar de la lista; `Tab::is_page()` agrupa las dos donde hoy se pregunta
`is_settings()` para decir «no es una carpeta». El estado de la vista (desplazamiento, versión a
mostrar) va en `Tab::changelog`.

## Tareas

### T1 — Diseño (FigCraft)
- X72 «Novedades» oscuro y claro en el grupo nuevo «Novedades (X72)» al final de «Pantallas · xdir»;
  X67 con el menú del engranaje en el orden Novedades / Más opciones… / Acerca de xdir; la variante
  del aviso de actualización en la barra de estado; componentes nuevos en Componentes. `docs/DISENO.md`.

### T2 — `CHANGELOG.md` y la regla para colaborar
- «Sin publicar» con lo de desde la 0.1.23, por tema; resumen corto por versión anterior.
- README → «Colaborar»: cada PR suma su línea en «Sin publicar».

### T3 — Publicar
- `scripts/changelog.sh` (`publicar <versión> <fecha> <archivo>`, `notas <versión> <archivo>`) en
  awk/sed POSIX; `scripts/changelog-test.sh` contra un CHANGELOG de ejemplo.
- `release.yml`: el paso de versión renombra la sección y la commitea en «Versión X»; el release usa
  `--notes-file` con la sección, o `--generate-notes` si está vacía.

### T4 — Parser (`xdir-core/src/changelog.rs`)
- `parse(&str) -> Vec<Block>`; BOM, CRLF, secciones faltantes, `Sin publicar` vacía fuera, «En
  desarrollo» con contenido; markup desconocido como texto.
- `wrap(spans, width, measure) -> Vec<Vec<Run>>` (partir líneas largas por palabras, y palabras
  más largas que el ancho por caracteres).
- Tests: `parse_*`, `wrap_*`, `bundled_changelog_parses`.

### T5 — Pestaña Novedades
- `Location::Changelog` (fs, sesión, folder_views, plataformas), `Tab::changelog`, `is_page()`.
- `App::open_changelog(version)`: una sola pestaña (como Configuración).
- Dibujo `changelog::draw` con los colores del tema; rueda y teclas (flechas, RePág/AvPág,
  Inicio/Fin). Migas «Novedades».
- Tests: abrir una sola vez, la sesión la guarda, la rueda y las teclas desplazan, abrir en una versión.

### T6 — Menú del engranaje, Acerca de y paleta
- `PickerHit::Changelog` arriba de «Más opciones…»; «Acerca de xdir» queda último.
- Acerca de: botón «Novedades» (`AboutHit::Changelog`).
- `Command::Changelog` (`novedades`, grupo Otros, sin atajo); tablas fijadas de mac/windows y la de la wiki.

### T7 — Aviso al actualizar
- `Session::seen_version` (`version_vista`). Al arrancar: sin sesión anterior → nada; sesión sin
  versión o con una menor → aviso «xdir se actualizó a X · Ver novedades»; igual o mayor → nada.
  Siempre queda la versión de ahora como vista.
- Barra de estado: el aviso adelante del texto, «Ver novedades» en el acento con su lugar para el
  clic (`Hit::UpdateNotice`), que abre Novedades en esa versión y quita el aviso.
- Tests: comparación de versiones, primera instalación, bajar de versión, clic.

### T8 — Verificación
- `cargo test -p xdir-core` (± `--features arbol`), `-p xdir-mac`, `-p xdir-archive`; check de Windows;
  clippy; Linux en Docker; prueba visual en macOS con un build portable `--features pruebas`.

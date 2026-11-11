# CHANGELOG y pestaña Novedades — diseño

Fecha: 2026-11-10. Aprobado por el usuario (cambio chico, fuera de las tandas).

## Por qué

No hay forma de saber qué cambió entre versiones: los releases usan las notas automáticas de GitHub (lista de PRs) y desde la 0.1.23 no salió ninguna versión. El usuario quiere un CHANGELOG y poder verlo desde la app.

## Qué hace

### `CHANGELOG.md` (raíz del repo)

- En castellano, escrito para quien usa xdir (no lista de commits). Formato:
  ```
  # Novedades de xdir

  ## Sin publicar
  ### Nuevo
  - …
  ### Cambios
  - …
  ### Arreglos
  - …

  ## 0.1.23 — 2026-11-08
  …
  ```
  Solo `## <versión> — <AAAA-MM-DD>` o `## Sin publicar`, `### Nuevo|Cambios|Arreglos`, viñetas `- ` (con sub-viñetas de dos espacios), `**negrita**` y `` `código` ``. Nada más (el visor de la app entiende solo eso).
- Primera versión: «Sin publicar» con todo lo hecho desde la 0.1.23, agrupado por tema (organizar la lista, vista previa, operaciones con archivos, configuración, red, archivos comprimidos), y para las versiones anteriores un resumen corto por versión sacado de los releases y del historial de git.
- Regla: cada PR suma su línea en «Sin publicar». Se anota en las instrucciones del repo (README o `AGENTS.md`/`CLAUDE.md` si existe; si no, en `docs/CONTRIBUIR.md`).

### Publicar (`.github/workflows/release.yml`)

- Al publicar, el paso de versión renombra `## Sin publicar` a `## <versión> — <fecha>` y deja un `## Sin publicar` vacío arriba, en el mismo commit «Versión X».
- Las notas del release son el texto de esa versión del CHANGELOG (en vez de `--generate-notes`). Si la sección está vacía, se usa `--generate-notes` como hoy.

### En la app: pestaña Novedades

- Una pestaña más del panel activo, como Configuración (`Location::Changelog` o la forma que encaje con `Location::Settings`), que muestra el CHANGELOG dibujado: título por versión, subtítulos, viñetas con sangría, negrita y código; desplazamiento con rueda y teclado; texto seleccionable no hace falta.
- El texto se incluye en el binario al compilar (`include_str!` desde `xdir-core`), sin internet. La sección «Sin publicar» se muestra como «En desarrollo» (y solo si tiene contenido).
- Se abre desde:
  - el **menú del engranaje**, con el orden pedido por el usuario: **Novedades**, **Más opciones…**, **Acerca de xdir** (de arriba hacia abajo);
  - la ventana **Acerca de xdir** (un enlace/botón «Novedades»);
  - la **paleta** (comando `novedades`, grupo Otros, sin atajo de fábrica).
- **Aviso al actualizar** (elección del usuario): la primera vez que se abre una versión nueva (comparando con la última versión vista, guardada en la sesión o en un archivo propio de la carpeta de configuración), la barra de estado muestra «xdir se actualizó a 0.1.24 · Ver novedades»; clic en «Ver novedades» abre la pestaña en esa versión. No se muestra en la primera instalación (sin versión vista anterior) ni al bajar de versión. Se marca como visto al mostrarse.

## Fuera de esto

- Buscar actualizaciones por internet.
- Traducción a otros idiomas.

## Diseño visual

- Pantalla nueva X72 «Novedades» (oscuro y claro) y X67 actualizada con el orden nuevo del menú del engranaje; el aviso en la barra de estado sigue el componente de avisos existente.

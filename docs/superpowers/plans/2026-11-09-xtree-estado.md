# Estado de xtree junto al botón y en la columna izquierda — plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** mostrar el avance o el resultado del escaneo de xtree junto al botón de la barra de estado y en una entrada «xtree» de la columna izquierda.

**Architecture:** el estado sale de `Xtree` (`badge()` y `badge_tip()`); una sola función de dibujo `ui::draw_xtree_badge` la usan la barra de estado y el árbol; el cartel sale de `app::tooltip`.

**Tech Stack:** Rust, `xdir-core` (sin dependencias), Painter/RecordingPainter.

**Spec:** `docs/superpowers/specs/2026-11-09-xtree-estado-design.md`. Diseño visual: `docs/design/xdir.figcraft` (componente `ui::draw_xtree_badge`, variantes de `ui::draw_status`, `tree::Tree::draw` «Fijado con xtree», pantalla X38).

## Global Constraints

- `xdir-core` sin dependencias ni `cfg` de sistema en producción; tests con `install_fakes()`, `FakeHost`, `FakeDiskScan`, `RecordingPainter`.
- `cargo test -p xdir-core` (con y sin `--features arbol`), `cargo clippy -p xdir-core --all-targets` sin warnings nuevos, `cargo check -p xdir-mac`; Linux por Docker.
- Textos exactos del cartel: «Escaneando / · 37 % · 0:42», «Escaneando ~/Proyectos · 0:12», «Análisis completado en 1:23», «Análisis completado en 1:23 · 458 carpetas sin permiso» (ruta como la muestra hoy la vista de xtree; tiempo con `fmt_elapsed`; porcentaje con `scan_percent`).
- Medidas y colores del diseño: indicador 14×14; progreso = anillo en acento lleno según el %; giro = arco que avanza con cada aviso del escaneo (250 ms), sin repintado continuo extra; terminado = tilde en `ok`; aviso = ícono en `advertencia`. El botón de la barra se ensancha de 74 a 99 mientras muestra el indicador y el anillo reemplaza al ícono del mapa.
- Terminado/aviso se ven hasta que se abre la vista de xtree; al abrirla, vuelve a `Idle`.
- Castellano; Conventional Commits sin atribución a Claude.

## Review Focus

- Escaneo cancelado (cerrar xtree con Esc durante el escaneo, o escaneo nuevo encima): el indicador no queda pegado en progreso ni marca «terminado» de un escaneo cancelado.
- Escaneo de una carpeta (sin porcentaje) y reescaneo con permisos elevados: giro, no porcentaje.
- Barra angosta: el botón más ancho no tapa el texto de estado ni los otros botones (mismo recorte que hoy).
- Sin feature `arbol`: no hay entrada en el árbol y compila sin código muerto.
- Clic en la entrada del árbol con la vista abierta: la cierra (como el botón), y no navega el panel.

---

### Task 1: Indicador en la barra de estado y cartel

**Files:**
- Modify: `crates/xdir-core/src/xtree/{mod,view}.rs` (`XtreeBadge`, `badge()`, `badge_tip()`, marca de «visto»), `crates/xdir-core/src/ui.rs` (`draw_xtree_badge`, `draw_status` con el indicador y el ancho del botón), `crates/xdir-core/src/app/{tooltip,mod,…}.rs` (cartel del botón con `badge_tip()`, repintado de la barra con los avisos del escaneo aunque la vista esté cerrada).

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum XtreeBadge { Idle, Progress(u8), Spinning(u8), Done, DoneWithIssues }` (en `Spinning` el `u8` es la fase del arco, que avanza con cada aviso).
  - `impl Xtree { pub fn badge(&self) -> XtreeBadge; pub fn badge_tip(&self) -> Option<String>; pub fn mark_seen(&mut self) }` (`mark_seen` lo llama la app al abrir la vista).
  - `pub fn draw_xtree_badge(p: &dyn Painter, r: Rect, badge: XtreeBadge, theme…)` en `ui.rs` (seguir la firma de los otros dibujos de `ui.rs`).
  - `draw_status` recibe el `XtreeBadge` (reemplazar o complementar `xtree_on`; si la lista de parámetros crece, agrupar en un struct como se hizo con `PickerSwitches`).

- [ ] **Step 1: Tests que fallan**: `badge_progress_while_scanning_volume` (FakeDiskScan con volumen de 1000 usados y 370 leídos → `Progress(37)`), `badge_spins_for_folder_scan` (carpeta sin volumen → `Spinning(_)` y la fase cambia entre avisos), `badge_done_until_view_opened` (terminado → `Done`; tras `mark_seen` → `Idle`), `badge_done_with_issues_when_denied` (carpetas sin permiso > 0 → `DoneWithIssues` y cartel «Análisis completado en 1:23 · 458 carpetas sin permiso» con el tiempo de FakeEnv), `badge_idle_after_cancel`, `badge_tip_texts` (los cuatro textos de Global Constraints), `status_bar_draws_ring_and_widens_button` (RecordingPainter: con `Progress(37)` el botón mide 99 y se dibuja «37 %»; con `Idle`, 74 y el ícono del mapa), `status_tooltip_uses_badge_tip`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS + Global Constraints.
- [ ] **Step 5: Commit** `feat(core): indicador del escaneo de xtree junto al botón de la barra de estado`.

### Task 2: Entrada «xtree» en la columna izquierda

**Files:**
- Modify: `crates/xdir-core/src/tree.rs` (nuevo `Kind::Xtree` debajo de «Inicio», antes del separador; dibujo con ícono del mapa + «xtree» + `draw_xtree_badge` a la derecha; sin subcarpetas, no navega), `crates/xdir-core/src/app/…` (clic → mismo comando que el botón de la barra: abre xtree sobre la carpeta del panel activo o cierra la vista; cartel con `badge_tip()` o «xtree» si no hay escaneo), todo bajo la feature `arbol`.

**Interfaces:**
- Consumes: `XtreeBadge`, `Xtree::badge()`, `badge_tip()`, `ui::draw_xtree_badge` (Task 1).
- Produces: `Kind::Xtree`.

- [ ] **Step 1: Tests que fallan** (con `arbol`): `tree_has_xtree_entry_below_home` (orden de raíces: Inicio, xtree, …, separador), `tree_xtree_click_opens_and_closes_view` (primer clic abre xtree sobre la carpeta del panel activo; segundo clic la cierra; el panel no navega), `tree_xtree_entry_draws_badge` (con `Progress(37)` se dibuja «37 %» en la fila), `tree_xtree_entry_not_selectable_by_follow` (que «el árbol sigue al panel» nunca seleccione esa entrada), `tree_xtree_tooltip`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar.
- [ ] **Step 4:** PASS + Global Constraints.
- [ ] **Step 5: Commit** `feat(core): acceso a xtree con su estado en la columna izquierda`.

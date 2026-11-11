# xtree: cronómetro y porcentaje — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cronómetro del escaneo (en curso y total) y porcentaje de avance para volúmenes enteros en la vista de xtree.

**Architecture:** Todo en `crates/xdir-core/src/xtree/view.rs` (estado de tiempos, textos, barra de avance) más una función pura de formato; repintado periódico mientras escanea con el temporizador/animación que ya usa la vista o la `App`.

**Tech Stack:** Rust 2024, `xdir-core`.

**Spec:** `docs/superpowers/specs/2026-11-09-xtree-tiempo-design.md`

## Global Constraints

- Textos exactos: «Escaneando… 0:42», «Escaneando… 37 % · 0:42», «Análisis completado en 1:23», «Detenido a los 0:37»; formato m:ss y h:mm:ss desde una hora.
- Porcentaje solo con `disk_scan().whole_volume(root)`; tope 99 % mientras escanea; entero.
- Repintado al menos una vez por segundo mientras escanea; sin repintado continuo cuando no escanea.
- `xdir-core` sin dependencias ni `cfg` de sistema. `cargo test` (Mac 216 en `develop`) y Docker sin bajar; `cargo check --all-targets` limpio para la Mac, Windows y Linux.
- No `cargo fmt` del repo. Castellano; Conventional Commits sin atribución a Claude. Rama `feature/xtree-tiempo`, base `develop`, PR propio.

## Review Focus

- **Volumen casi vacío o con espacio usado 0**: no divide por cero; sin porcentaje.
- **Bytes leídos mayores que el usado** (APFS, hardlinks): el porcentaje no pasa de 99 % ni retrocede.
- **Escaneo cancelado o reiniciado**: el cronómetro de un escaneo nuevo arranca de 0; «Detenido a los…» solo si se cortó.

---

### Task 1: Cronómetro y porcentaje

**Files:**
- Modify: `crates/xdir-core/src/xtree/view.rs` (estado `scan_started: Option<Instant>`, `scan_elapsed: Option<Duration>`, `scan_stopped: bool`; textos en la barra y en `status_text`; barra de avance), y donde la `App` decide si pide otro cuadro mientras xtree escanea (`app/paint.rs` o la propia vista: devolver que sigue animando mientras `scanning`, con un tope de un cuadro por segundo si hay un mecanismo de temporizador; si solo existe el «otro cuadro ya», usar el que haya y documentarlo).

**Interfaces:**
- Produces: `pub fn fmt_elapsed(d: Duration) -> String` (pura: `42 s` → «0:42», `83 s` → «1:23», `3735 s` → «1:02:15»); `fn scan_percent(read: u64, used: u64) -> Option<u8>` (pura: `None` si `used == 0`; `min(99, read * 100 / used)`).

- [ ] **Step 1: Tests que fallan**: `fmt_elapsed_formats` (0 → «0:00», 42 → «0:42», 83 → «1:23», 3735 → «1:02:15»), `scan_percent_caps_and_handles_zero` ((50, 100) → 50; (150, 100) → 99; (1, 0) → None), `status_shows_elapsed_while_scanning` (con `FakeDiskScan` y un escaneo en curso: `status_text()` contiene «Escaneando…» y un tiempo `m:ss`), `status_shows_total_when_done` (al terminar: «Análisis completado en »), `percent_only_for_whole_volume` (raíz que `FakeDiskScan` reporta como volumen entero con usado conocido → el texto tiene « % · »; carpeta → no), `restart_resets_timer` (iniciar, cancelar, iniciar otra raíz → el tiempo vuelve a 0 y no dice «Detenido»), `cancel_says_stopped`.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implementar (barra de avance de alto 3 en el borde inferior de la barra superior, color `theme.accent`).
- [ ] **Step 4:** PASS; Global Constraints; humo en la Mac: escanear `/` con la copia aislada y capturar a los pocos segundos (porcentaje y cronómetro) y una carpeta (solo cronómetro); al terminar una carpeta chica, captura con «Análisis completado en …».
- [ ] **Step 5: Commit** `feat(xtree): cronómetro del escaneo y porcentaje de avance en volúmenes enteros`.

### Task 2: PR

- [ ] PR `feature/xtree-tiempo` → `develop` con lista para Runick93 (escanear `C:\` con y sin el ayudante elevado: porcentaje, cronómetro, MFT; una carpeta; cancelar). Al mergear, versión de Mac en `/Applications`.

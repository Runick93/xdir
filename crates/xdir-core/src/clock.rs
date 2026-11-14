//! El reloj de lo que se muestra con umbrales (indicador de actividad, «Cargando…», expulsar):
//! `Instant::now()`, salvo que una prueba lo fije para su hilo (`set`). Así la `App`, las pestañas
//! y el temporizador miden con el mismo reloj.

use std::time::Instant;

#[cfg(test)]
thread_local! {
    static FIXED: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Ahora (o el momento fijado por la prueba de este hilo).
pub fn now() -> Instant {
    #[cfg(test)]
    if let Some(t) = FIXED.with(|c| c.get()) {
        return t;
    }
    Instant::now()
}

/// Pruebas: fija el reloj de este hilo (`None`: el de verdad). `App::new` lo suelta.
#[cfg(test)]
pub fn set(t: Option<Instant>) {
    FIXED.with(|c| c.set(t));
}

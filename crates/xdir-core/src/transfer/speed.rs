//! Velocidad y tiempo restante de un trabajo: promedio móvil de los últimos 5 s, tiempo
//! restante después de 2 s de datos («Calculando…» antes) y el gráfico de velocidad (una muestra
//! por cada 1 % de avance, como el diálogo de copia de Windows).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Ventana del promedio móvil.
const WINDOW: Duration = Duration::from_secs(5);
/// Datos que hacen falta para estimar el tiempo restante.
const ETA_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct Speed {
    start: Instant,
    total: u64,
    /// (cuándo, bytes hechos), de los últimos `WINDOW` (más uno anterior como borde).
    samples: VecDeque<(Instant, u64)>,
    graph: Vec<f32>,
    /// En pausa (o esperando la respuesta a un conflicto) desde entonces.
    paused_at: Option<Instant>,
    /// Tiempo en pausa ya cerrado (no cuenta para el promedio).
    excluded: Duration,
}

impl Speed {
    pub fn new(start: Instant, total: u64) -> Speed {
        Speed { start, total, samples: VecDeque::from([(start, 0)]), graph: Vec::new(), paused_at: None, excluded: Duration::ZERO }
    }

    /// Empieza una pausa (o la espera de una respuesta): ese tiempo no cuenta para el promedio.
    pub fn pause(&mut self, now: Instant) {
        self.paused_at.get_or_insert(now);
    }

    /// Termina la pausa.
    pub fn resume(&mut self, now: Instant) {
        if let Some(t) = self.paused_at.take() {
            self.excluded += now.saturating_duration_since(t);
        }
    }

    /// Bytes hechos hasta ahora (`done`, acumulado).
    pub fn push(&mut self, now: Instant, done: u64) {
        self.samples.push_back((now, done));
        // Se descartan las muestras viejas, dejando una en el borde de la ventana o antes.
        while self.samples.len() > 2 && now.saturating_duration_since(self.samples[1].0) >= WINDOW {
            self.samples.pop_front();
        }
        if self.total == 0 {
            return;
        }
        let percent = ((done.min(self.total) as u128 * 100) / self.total as u128) as usize;
        if percent > self.graph.len() {
            let rate = self.rate().unwrap_or(0.0) as f32;
            self.graph.resize(percent.min(100), rate);
        }
    }

    /// Bytes por segundo, promedio de los últimos 5 s.
    pub fn rate(&self) -> Option<f64> {
        let (&(t0, d0), &(t1, d1)) = (self.samples.front()?, self.samples.back()?);
        let secs = t1.saturating_duration_since(t0).as_secs_f64();
        (secs > 0.0).then(|| d1.saturating_sub(d0) as f64 / secs)
    }

    /// Tiempo restante; `None` hasta tener 2 s de datos o si no avanza.
    pub fn eta(&self, now: Instant) -> Option<Duration> {
        let &(last, done) = self.samples.back()?;
        if last.saturating_duration_since(self.start) < ETA_AFTER {
            return None;
        }
        let rate = self.rate().filter(|r| *r > 0.0)?;
        let left = self.total.saturating_sub(done) as f64 / rate;
        // Lo que pasó desde la última muestra ya corre.
        let since = now.saturating_duration_since(last).as_secs_f64();
        Some(Duration::from_secs_f64((left - since).max(0.0)))
    }

    /// Bytes por segundo desde el principio, sin contar pausas ni esperas por conflictos.
    pub fn average(&self) -> Option<f64> {
        let &(last, done) = self.samples.back()?;
        let open = self.paused_at.map_or(Duration::ZERO, |t| last.saturating_duration_since(t));
        let secs = last.saturating_duration_since(self.start).saturating_sub(self.excluded + open).as_secs_f64();
        (secs > 0.0).then(|| done as f64 / secs)
    }

    /// Velocidad en cada 1 % de avance (hasta 100 muestras): el eje x es el avance.
    pub fn graph(&self) -> &[f32] {
        &self.graph
    }
}

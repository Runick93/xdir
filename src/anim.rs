//! Animaciones sutiles (F-8.4): scroll suave de la lista y transición al cambiar de layout.
//! Nunca retrasan la interacción: el estado lógico (a dónde va el scroll, qué layout hay) cambia
//! al instante y solo lo que se dibuja se acerca de a poco; los clics y las teclas ya actúan
//! sobre el destino. Mientras algo se anima, la app pide otro frame (el vsync marca el ritmo);
//! cuando termina, deja de pedirlos.
//!
//! Se apagan con `animaciones = false` en `config.toml` o con "Mostrar animaciones en Windows"
//! desactivado (Configuración → Accesibilidad → Efectos visuales).

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};

use crate::render::Rect;

/// Duración de la transición de layout.
pub const LAYOUT_MS: f32 = 160.0;
/// Rapidez del scroll suave (1/s): cerca de 0.95 del camino a los ~170 ms.
const SCROLL_RATE: f32 = 18.0;
/// Menos de esto (en DIPs) de distancia: se da por llegado.
const SNAP: f32 = 0.4;

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// ¿Windows permite animaciones? (por defecto sí si la consulta falla).
pub fn system_allows() -> bool {
    let mut on = windows::core::BOOL(1);
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as *mut _), Default::default()) };
    ok.is_err() || on.as_bool()
}

/// Acerca `current` a `target` en `dt` segundos, sin pasarse. Devuelve el valor nuevo.
pub fn smooth(current: f32, target: f32, dt: f32) -> f32 {
    let d = target - current;
    if d.abs() < SNAP {
        return target;
    }
    current + d * (1.0 - (-dt.max(0.0) * SCROLL_RATE).exp())
}

/// Frenado suave al final: 0 → 0, 1 → 1.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect { left: lerp(a.left, b.left, t), top: lerp(a.top, b.top, t), right: lerp(a.right, b.right, t), bottom: lerp(a.bottom, b.bottom, t) }
}

/// Rectángulo desde el que "aparece" un panel que no estaba: el suyo, un poco más chico.
pub fn grow_from(r: Rect) -> Rect {
    let (dx, dy) = ((r.right - r.left) * 0.04, (r.bottom - r.top) * 0.04);
    Rect { left: r.left + dx, top: r.top + dy, right: r.right - dx, bottom: r.bottom - dy }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_converges_without_overshoot() {
        let mut x = 0.0;
        let mut frames = 0;
        while x != 500.0 {
            let next = smooth(x, 500.0, 1.0 / 60.0);
            assert!(next > x && next <= 500.0, "avanza sin pasarse: {x} -> {next}");
            x = next;
            frames += 1;
            assert!(frames < 120, "llega en menos de 2 s a 60 fps");
        }
        // Hacia atrás también, y con un cuadro largo no se pasa.
        assert_eq!(smooth(500.0, 0.0, 10.0), 0.0f32.max(smooth(500.0, 0.0, 10.0)));
        assert!(smooth(500.0, 0.0, 10.0) >= 0.0);
        // Ya cerca: se da por llegado.
        assert_eq!(smooth(99.8, 100.0, 1.0 / 60.0), 100.0);
    }

    #[test]
    fn easing_bounds() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "arranca rápido y frena");
        assert_eq!(ease_out(2.0), 1.0);
        let r = lerp_rect(Rect { left: 0.0, top: 0.0, right: 100.0, bottom: 50.0 }, Rect { left: 100.0, top: 10.0, right: 200.0, bottom: 150.0 }, 0.5);
        assert_eq!((r.left, r.top, r.right, r.bottom), (50.0, 5.0, 150.0, 100.0));
        let g = grow_from(Rect { left: 0.0, top: 0.0, right: 100.0, bottom: 100.0 });
        assert_eq!((g.left, g.right), (4.0, 96.0));
    }
}

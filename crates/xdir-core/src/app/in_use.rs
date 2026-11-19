//! La ventana de «archivo en uso» (`AuxWindow::InUse`): una ventana secundaria del sistema, aparte
//! de la de copia, cuyo contenido dibuja `in_use_draw`. Se abre sola cuando algo falla porque otro
//! programa tiene el archivo y se sabe quién: una operación suelta, al fallar (`on_op_done`); un
//! trabajo del motor, una sola vez al terminar (`on_job_done`). Otro fallo con la ventana abierta
//! reemplaza su contenido. «Reintentar» (o Enter) vuelve a hacer lo que falló con el reintento de
//! cada origen; «Cerrar» (o Esc) la oculta. Donde el host no tiene esta ventana queda solo el aviso
//! de la barra de estado.

use super::*;
use crate::geom::rect;
use crate::host::{AuxMouse, AuxWindow};
use crate::in_use::{InUseDialog, InUseRetry};
use crate::in_use_draw::{self, InUseHit};
use crate::paint::Painter;
use crate::transfer::JobId;

/// La ventana abierta: lo que muestra, el botón bajo el mouse y el apretado.
pub(super) struct InUseState {
    dialog: InUseDialog,
    hover: Option<InUseHit>,
    pressed: Option<InUseHit>,
}

/// El área de la ventana: el alto que pide el contenido, sin recortar (los botones van al pie).
fn size_of(d: &InUseDialog) -> (f32, f32) {
    (in_use_draw::WIDTH, in_use_draw::height(d))
}

impl App {
    /// Abre la ventana con `d`. Si ya estaba abierta, reemplaza su contenido (título, tamaño y
    /// texto) y la vuelve a mostrar, así una minimizada vuelve sin quitarle el foco a nadie. Si el
    /// host no tiene esta ventana, no hace nada (queda el aviso de la barra de estado).
    pub fn open_in_use(&mut self, d: InUseDialog) {
        let host = self.host.clone();
        let size = size_of(&d);
        if !host.aux_show(AuxWindow::InUse, &d.title, size) {
            self.in_use = None;
            return;
        }
        self.in_use = Some(InUseState { dialog: d, hover: None, pressed: None });
        host.aux_invalidate(AuxWindow::InUse);
    }

    /// Lo que muestra la ventana abierta (pruebas).
    #[cfg(test)]
    pub(super) fn in_use_dialog(&self) -> Option<&InUseDialog> {
        self.in_use.as_ref().map(|s| &s.dialog)
    }

    /// La ventana se pinta (desde `paint_aux`).
    pub(super) fn paint_in_use(&self, p: &dyn Painter, width: f32, height: f32) {
        if let Some(s) = &self.in_use {
            in_use_draw::draw(p, rect(0.0, 0.0, width, height), &s.dialog, s.hover, s.pressed);
        }
    }

    /// El mouse en la ventana (desde `aux_mouse`): resalta el botón de abajo, lo aprieta, y al
    /// soltarlo sobre el mismo botón lo usa.
    pub(super) fn in_use_mouse(&mut self, x: f32, y: f32, ev: AuxMouse) {
        let host = self.host.clone();
        let Some(s) = &mut self.in_use else { return };
        let (width, height) = size_of(&s.dialog);
        let at = in_use_draw::hit(rect(0.0, 0.0, width, height), &s.dialog, x, y);
        let before = (s.hover, s.pressed);
        match ev {
            AuxMouse::Move => s.hover = at,
            AuxMouse::Leave => s.hover = None,
            AuxMouse::Down => s.pressed = at,
            AuxMouse::Cancel => s.pressed = None,
            AuxMouse::Up => {
                let pressed = s.pressed.take();
                if let Some(h) = at.filter(|h| pressed == Some(*h)) {
                    return self.on_in_use_hit(h);
                }
            }
            AuxMouse::Wheel { .. } => {}
        }
        if before != (s.hover, s.pressed) {
            host.aux_invalidate(AuxWindow::InUse);
        }
    }

    /// Una tecla en la ventana (desde `aux_key`): Esc la cierra y Enter reintenta. `true` si se usó.
    pub(super) fn in_use_key(&mut self, vk: Key) -> bool {
        match vk {
            VK_ESCAPE => self.on_in_use_hit(InUseHit::Close),
            VK_RETURN => self.on_in_use_hit(InUseHit::Retry),
            _ => return false,
        }
        true
    }

    /// El usuario cerró la ventana (desde `aux_closed`): se olvida lo que mostraba.
    pub(super) fn in_use_closed(&mut self) {
        self.in_use = None;
    }

    /// Se olvidó el trabajo `id` (se descartó o se reintentó desde la ventana de copia): si la
    /// ventana abierta es la de ese trabajo, ya no tiene a qué volver, así que se cierra.
    pub(super) fn in_use_forget_job(&mut self, id: JobId) {
        if self.in_use.as_ref().is_some_and(|s| s.dialog.retry == InUseRetry::Job(id)) {
            self.in_use = None;
            self.host.clone().aux_hide(AuxWindow::InUse);
        }
    }

    /// «Reintentar»: lo que falló se vuelve a hacer, con el reintento de su origen (la operación
    /// suelta se repite; el trabajo reintenta lo que falló), y la ventana se oculta. «Cerrar»: solo
    /// se oculta.
    fn on_in_use_hit(&mut self, h: InUseHit) {
        let Some(s) = self.in_use.take() else { return };
        if h == InUseHit::Retry {
            match s.dialog.retry {
                InUseRetry::Op(op) => sys::file_ops().spawn(self.hwnd, self.ops_tx.clone(), op),
                InUseRetry::Job(id) => {
                    self.retry_job(id);
                }
            }
        }
        // Si el reintento ya la volvió a abrir (falló en el acto), queda abierta con lo nuevo.
        if self.in_use.is_none() {
            self.host.clone().aux_hide(AuxWindow::InUse);
        }
        self.redraw();
    }
}

//! «Suma de verificación» del panel de información (issue #59): el algoritmo elegido (SHA-256 o
//! MD5), la suma de un archivo calculada en otro hilo (`compare::hash_source`, vigilado como la
//! comparación: avance, Cancelar y límite sin avance en la red) y «Comparar con…» con la suma
//! publicada (`compare::parse_expected`: el largo elige el algoritmo y, si falta, se calcula).
//!
//! Se calcula solo al pedirlo y se corta al cambiar de archivo o al cerrar el panel. Un archivo de
//! adentro de un comprimido se extrae antes a la temporal de la app (con sus límites).

use super::compare::{Job, JobReply};
use super::*;
use crate::compare::{self, Failure, Source};
use crate::fs::{ATTR_ARCHIVE_BAD, ATTR_LINK};
use crate::info::draw::{ChecksumView, SumCheck, SumState};
use crate::sys::HashAlgo;

/// La suma de verificación del panel: de qué archivo, el algoritmo, el trabajo en curso, lo
/// calculado y lo pegado en «Comparar con…».
pub(super) struct Checksum {
    pub target: Option<Source>,
    pub algo: HashAlgo,
    pub job: Option<(HashAlgo, Job)>,
    pub results: Vec<(HashAlgo, Result<String, String>)>,
    pub expected: String,
}

impl Default for Checksum {
    fn default() -> Self {
        Checksum { target: None, algo: HashAlgo::Sha256, job: None, results: Vec::new(), expected: String::new() }
    }
}

impl Checksum {
    /// Corta la suma en curso.
    pub(super) fn stop(&mut self) {
        if let Some((_, j)) = self.job.take() {
            j.stop();
        }
    }

    /// Expulsar el disco de `root`: si la suma lee de ahí, se corta y se olvida; devuelve su hilo
    /// para esperar a que suelte el archivo.
    pub(super) fn release_under(&mut self, root: &std::path::Path) -> Option<std::thread::JoinHandle<()>> {
        if !self.target.as_ref().is_some_and(|t| t.disk_path().starts_with(root)) {
            return None;
        }
        let handle = self.job.take().and_then(|(_, j)| j.release());
        self.reset(None);
        handle
    }

    /// Otro archivo (o ninguno): se corta lo que estaba y se olvida todo, salvo el algoritmo.
    pub(super) fn reset(&mut self, target: Option<Source>) {
        self.stop();
        self.target = target;
        self.results.clear();
        self.expected.clear();
    }

    fn result(&self, algo: HashAlgo) -> Option<&Result<String, String>> {
        self.results.iter().rev().find(|(a, _)| *a == algo).map(|(_, r)| r)
    }

    /// Lo que dice «Comparar con…» (`None`: nada pegado o falta calcular).
    fn check(&self) -> Option<SumCheck> {
        if self.expected.trim().is_empty() {
            return None;
        }
        let Some((algo, want)) = compare::parse_expected(&self.expected) else { return Some(SumCheck::Invalid) };
        match self.result(algo)? {
            Ok(got) if *got == want => Some(SumCheck::Matches),
            Ok(_) => Some(SumCheck::Differs),
            Err(_) => None,
        }
    }
}

impl App {
    /// El archivo del que se calcularía la suma: el único elemento que muestra el panel, si es un
    /// archivo (del disco o de adentro de un comprimido).
    pub(super) fn checksum_target(&self) -> Option<Source> {
        let tab = self.panes[self.active].tab();
        if tab.location.is_archive() {
            let mut sel = tab.list.selected_indices();
            let (Some(i), None) = (sel.next(), sel.next()) else { return None };
            let e = &tab.list.entries[i];
            if e.is_dir || e.attrs & (ATTR_ARCHIVE_BAD | ATTR_LINK) != 0 {
                return None;
            }
            let Location::Archive { file, nested, .. } = &tab.location else { return None };
            return Some(Source::Archive { file: file.clone(), nested: nested.clone(), rel: tab.archive_rel(i)? });
        }
        let items = self.info_items()?;
        match items.as_slice() {
            [it] if !it.is_dir && it.volume.is_none() && matches!(tab.location, Location::Path(_)) => Some(Source::Disk(it.path.clone())),
            _ => None,
        }
    }

    /// Antes de cada cuadro (con el panel abierto): si cambió el archivo, se corta y se olvida.
    pub(super) fn sync_checksum(&mut self) {
        let target = self.checksum_target();
        if target != self.info.sum.target {
            if self.info.edit.is_some_and(|(_, k)| k == super::info::EditKind::Expected) {
                self.end_info_edit(false);
            }
            self.info.sum.reset(target);
        }
    }

    /// La sección para el archivo que se muestra (`cloud`: solo en la nube, no se ofrece).
    pub(super) fn checksum_view(&self, cloud: bool) -> ChecksumView {
        let s = &self.info.sum;
        let state = if cloud {
            SumState::CloudOnly
        } else {
            match (&s.job, s.result(s.algo)) {
                (Some((a, j)), _) if *a == s.algo => SumState::Running(j.progress.fraction()),
                (_, Some(Ok(h))) => SumState::Done(h.clone()),
                (_, Some(Err(m))) => SumState::Failed(m.clone()),
                _ => SumState::Idle,
            }
        };
        ChecksumView { algo: s.algo, state, expected: s.expected.clone(), check: s.check() }
    }

    /// Calcula la suma `algo` del archivo del panel (corta la que estuviera en curso).
    pub(super) fn start_checksum(&mut self, algo: HashAlgo) {
        self.info.sum.stop();
        let Some(src) = self.info.sum.target.clone() else { return };
        self.info.sum.results.retain(|(a, _)| *a != algo);
        let remote = src.remote_host();
        let job = self.spawn_job(remote, move |stop, p| compare::hash_source(&src, algo, stop, p), |id, result| JobReply::Hashed { id, result });
        self.info.sum.job = Some((algo, job));
        self.redraw();
    }

    /// Llegó una suma: si es la del panel de información, queda para su algoritmo. Devuelve si era.
    pub(super) fn on_checksum(&mut self, id: u64, result: &Result<String, Failure>) -> bool {
        let s = &mut self.info.sum;
        let Some(algo) = s.job.as_ref().filter(|(_, j)| j.id == id).map(|(a, _)| *a) else { return false };
        s.job = None;
        match result {
            Ok(h) => s.results.push((algo, Ok(h.clone()))),
            Err(Failure::Failed(m)) => s.results.push((algo, Err(m.clone()))),
            Err(Failure::Cancelled) => {}
        }
        true
    }

    /// Un clic en la sección.
    pub(super) fn checksum_click(&mut self, h: crate::info::draw::InfoHit) {
        use crate::info::draw::InfoHit;
        match h {
            InfoHit::HashAlgo(a) => {
                if self.info.sum.algo != a {
                    self.info.sum.stop();
                    self.info.sum.algo = a;
                }
            }
            InfoHit::HashRun => {
                if self.info.sum.job.is_some() {
                    self.info.sum.stop();
                } else {
                    self.start_checksum(self.info.sum.algo);
                }
            }
            InfoHit::HashCopy => {
                let algo = self.info.sum.algo;
                if let Some(Ok(hex)) = self.info.sum.result(algo).cloned() {
                    let _ = sys::shell().copy_text(self.hwnd, &hex);
                    self.notice = Some(format!("Se copió la suma {}", algo.label()));
                }
            }
            _ => {}
        }
        self.redraw();
    }

    /// Lo pegado en «Comparar con…»: si es una suma, se elige su algoritmo y, si todavía no está
    /// calculada, se calcula.
    pub(super) fn set_expected(&mut self, text: &str) {
        self.info.sum.expected = text.to_string();
        if let Some((algo, _)) = compare::parse_expected(text) {
            if self.info.sum.algo != algo {
                self.info.sum.stop();
                self.info.sum.algo = algo;
            }
            let running = self.info.sum.job.as_ref().is_some_and(|(a, _)| *a == algo);
            if !running && self.info.sum.result(algo).is_none() && self.info.sum.target.is_some() {
                self.start_checksum(algo);
            }
        }
        self.redraw();
    }

    /// Pruebas (`suma <qué> [texto]`): `sha` / `md5` (el selector), `calcular`, `copiar`,
    /// `comparar <texto>`.
    #[cfg(feature = "pruebas")]
    pub(super) fn checksum_test(&mut self, what: &str, text: &str) {
        use crate::info::draw::InfoHit;
        match what {
            "sha" => self.checksum_click(InfoHit::HashAlgo(HashAlgo::Sha256)),
            "md5" => self.checksum_click(InfoHit::HashAlgo(HashAlgo::Md5)),
            "calcular" => self.checksum_click(InfoHit::HashRun),
            "copiar" => self.checksum_click(InfoHit::HashCopy),
            "comparar" => self.set_expected(text),
            _ => {}
        }
    }
}

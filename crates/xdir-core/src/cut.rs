//! Lo cortado (Ctrl+X / ⌘X): las rutas que el portapapeles del sistema tiene marcadas para mover
//! al pegar. Mientras están ahí, sus íconos se dibujan atenuados (`CUT_ALPHA`), como en el
//! Explorador. La fuente de verdad es el portapapeles (`Shell::clipboard_files`); la `App` guarda
//! una copia normalizada para comparar al dibujar sin tocar el disco.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Opacidad del ícono (y la miniatura) de lo cortado.
pub const CUT_ALPHA: f32 = 0.5;

/// Rutas cortadas, normalizadas con `cut_key`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CutSet(HashSet<Vec<String>>);

/// Clave de comparación de una ruta: sus tramos en minúsculas (Windows y macOS no distinguen
/// mayúsculas; por tramos, `C:/a` y `C:\a` o una barra final no cambian nada).
pub fn cut_key(p: &Path) -> Vec<String> {
    crate::sys::folded(p)
}

impl CutSet {
    /// Lo que dice el portapapeles (`rutas`, `cortado`): las rutas si se cortaron; si se copiaron
    /// o no hay archivos, nada.
    pub fn from_clipboard(clip: Option<(Vec<PathBuf>, bool)>) -> Self {
        match clip {
            Some((paths, true)) => Self(paths.iter().map(|p| cut_key(p)).collect()),
            _ => Self::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// ¿`path` está cortado? Sin E/S: se compara contra las claves guardadas.
    pub fn contains(&self, path: &Path) -> bool {
        !self.0.is_empty() && self.0.contains(&cut_key(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sin distinguir mayúsculas (Windows y macOS): `C:\Dir\A.TXT` y `c:\dir\a.txt` son lo mismo.
    #[test]
    fn cut_set_ignores_case() {
        let cut = CutSet::from_clipboard(Some((vec![Path::new("C:").join("Dir").join("A.TXT")], true)));
        assert!(cut.contains(&Path::new("c:").join("dir").join("a.txt")));
        assert!(cut.contains(&Path::new("C:").join("DIR").join("a.Txt")));
        assert!(!cut.contains(&Path::new("c:").join("dir").join("b.txt")));
        assert!(!cut.contains(&Path::new("c:").join("dir")));
    }

    /// Copiado (no cortado) o sin archivos: nada atenuado.
    #[test]
    fn copied_is_not_cut() {
        let a = Path::new("x").join("a.txt");
        assert!(CutSet::from_clipboard(Some((vec![a.clone()], false))).is_empty());
        assert!(CutSet::from_clipboard(None).is_empty());
        assert!(!CutSet::from_clipboard(None).contains(&a));
    }
}

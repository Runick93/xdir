//! Consola y color de acento / modo oscuro.

use super::*;

impl App {
    // ---------- consola ----------

    /// Abre la consola en la carpeta del panel.
    pub(super) fn open_console(&mut self, pane: usize, console: Console) {
        if let Location::Path(dir) = self.panes[pane].tab().location.clone() {
            if !console.open(self.hwnd, &dir) {
                self.notice = Some(format!("No se pudo abrir {}", console.label()));
            }
        }
        self.redraw();
    }

    // ---------- color de acento ----------

    pub(super) fn set_accent(&mut self, accent: Accent) {
        self.accent = accent;
        self.theme = Theme::from_system(accent, self.dark_mode);
        self.update_edit_resources();
        self.redraw();
    }

    /// Switch del panel de la rueda dentada: fija oscuro o claro (deja de seguir a Windows).
    pub(super) fn set_dark_mode(&mut self, dark: bool) {
        self.dark_mode = Some(dark);
        self.theme = Theme::from_system(self.accent, self.dark_mode);
        // Barra de título y menús nativos al mismo modo.
        let mica = self.theme.apply_to_window(self.hwnd);
        self.renderer.set_transparent(mica);
        self.transparent = mica;
        shell::set_menu_mode(self.dark_mode);
        self.update_edit_resources();
        self.redraw();
    }

    /// "Personalizado…": el selector de color de Windows (espectro, RGB y HSL).
    pub(super) fn choose_custom_color(&mut self) {
        let current = match self.accent {
            Accent::Rgb(c) => c,
            Accent::System => theme::PRESETS[0].1,
        };
        // COLORREF es 0x00BBGGRR.
        let bgr = |c: u32| COLORREF((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16));
        let mut cc = CHOOSECOLORW {
            lStructSize: size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: self.hwnd,
            rgbResult: bgr(current),
            lpCustColors: self.custom_colors.as_mut_ptr(),
            Flags: CC_FULLOPEN | CC_RGBINIT | CC_ANYCOLOR,
            ..Default::default()
        };
        if unsafe { ChooseColorW(&mut cc) }.as_bool() {
            let c = cc.rgbResult.0;
            self.set_accent(Accent::Rgb((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF));
        }
    }
}

//! Tema claro/oscuro y color de acento, leídos de la configuración de Windows.

use std::ffi::c_void;

use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{BOOL, PCWSTR, w};

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub background: D2D1_COLOR_F,
    pub surface: D2D1_COLOR_F,
    pub surface_alt: D2D1_COLOR_F,
    pub border: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub text_dim: D2D1_COLOR_F,
    /// Datos secundarios de la lista (fecha, tipo, tamaño): entre `text` y `text_dim`.
    pub text_secondary: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    /// Fondo tenue de la fila con foco (sin seleccionar).
    pub focus_fill: D2D1_COLOR_F,
    /// Fila bajo el mouse.
    pub hover: D2D1_COLOR_F,
    /// Filas seleccionadas.
    pub selection: D2D1_COLOR_F,
    pub scrollbar: D2D1_COLOR_F,
}

/// Colores de acento predefinidos (rueda dentada de la barra de estado): (nombre, 0xRRGGBB).
pub const PRESETS: [(&str, u32); 12] = [
    ("Azul", 0x3B9EFF),
    ("Celeste", 0x4CC2FF),
    ("Turquesa", 0x2EC4B6),
    ("Verde", 0x3FB950),
    ("Lima", 0xA3D65C),
    ("Amarillo", 0xE3B341),
    ("Naranja", 0xF0883E),
    ("Rojo", 0xF85149),
    ("Rosa", 0xF778BA),
    ("Violeta", 0xA371F7),
    ("Índigo", 0x6E7BFF),
    ("Gris", 0x8B98A9),
];

/// Acento por defecto: el azul del diseño.
pub const DEFAULT_ACCENT: Accent = Accent::Rgb(PRESETS[0].1);

/// De dónde sale el color de acento.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    /// El acento de Windows (Configuración > Personalización > Colores).
    System,
    /// Un color elegido: 0xRRGGBB.
    Rgb(u32),
}

impl Theme {
    /// Arma la paleta con el acento elegido. `dark`: modo fijado por el usuario; sin él, el
    /// modo de apps de Windows.
    pub fn from_system(accent: Accent, dark: Option<bool>) -> Self {
        let dark = dark.unwrap_or_else(system_dark);

        let accent = match accent {
            Accent::Rgb(c) => rgb(c >> 16, (c >> 8) & 0xFF, c & 0xFF),
            // AccentColor se guarda como 0xAABBGGRR.
            Accent::System => read_dword(w!("Software\\Microsoft\\Windows\\DWM"), w!("AccentColor"))
                .map(|abgr| rgb(abgr & 0xFF, (abgr >> 8) & 0xFF, (abgr >> 16) & 0xFF))
                .unwrap_or(rgb(0x00, 0x78, 0xD4)),
        };

        // Oscuro con tinte azulado y superficies apenas más claras que el fondo; los paneles
        // se separan por su borde, no por contraste de relleno.
        if dark {
            Self {
                dark,
                background: rgb(0x0B, 0x0F, 0x14),
                surface: rgb(0x10, 0x15, 0x1C),
                surface_alt: rgb(0x15, 0x1B, 0x23),
                border: rgb(0x26, 0x30, 0x3C),
                text: rgb(0xE6, 0xED, 0xF3),
                text_dim: rgb(0x8B, 0x98, 0xA9),
                text_secondary: rgb(0xB4, 0xBF, 0xCC),
                accent,
                focus_fill: alpha(accent, 0.10),
                hover: rgba(0xFF, 0xFF, 0xFF, 0.05),
                selection: alpha(accent, 0.26),
                scrollbar: rgba(0xFF, 0xFF, 0xFF, 0.22),
            }
        } else {
            Self {
                dark,
                background: rgb(0xEE, 0xF1, 0xF5),
                surface: rgb(0xFF, 0xFF, 0xFF),
                surface_alt: rgb(0xF6, 0xF8, 0xFA),
                border: rgb(0xD5, 0xDC, 0xE4),
                text: rgb(0x1F, 0x23, 0x28),
                text_dim: rgb(0x59, 0x63, 0x6E),
                text_secondary: rgb(0x3D, 0x46, 0x50),
                accent,
                focus_fill: alpha(accent, 0.08),
                hover: rgba(0x00, 0x00, 0x00, 0.045),
                selection: alpha(accent, 0.20),
                scrollbar: rgba(0x00, 0x00, 0x00, 0.28),
            }
        }
    }

    /// Ajusta la ventana al tema: barra de título oscura y fondo Mica (Windows 11).
    /// Devuelve si Mica quedó activo; en ese caso el marco se extiende a toda la ventana para
    /// que el fondo se vea donde la aplicación no pinta.
    pub fn apply_to_window(&self, hwnd: HWND) -> bool {
        let dark = BOOL::from(self.dark);
        unsafe {
            // Los errores se ignoran: en versiones viejas de Windows el atributo no existe.
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as *const c_void,
                size_of::<BOOL>() as u32,
            );
            let backdrop = DWMSBT_MAINWINDOW;
            let mica = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop as *const _ as *const c_void,
                size_of_val(&backdrop) as u32,
            )
            .is_ok();
            if mica {
                let all = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
                return DwmExtendFrameIntoClientArea(hwnd, &all).is_ok();
            }
            false
        }
    }
}

/// ¿Windows está en modo oscuro para las apps?
pub fn system_dark() -> bool {
    read_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(false)
}

fn read_dword(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut _ as *mut c_void),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn rgb(r: u32, g: u32, b: u32) -> D2D1_COLOR_F {
    rgba(r, g, b, 1.0)
}

fn rgba(r: u32, g: u32, b: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

pub fn alpha(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a, ..c }
}

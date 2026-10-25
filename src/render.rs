//! Dibujo con Direct2D + DirectWrite sobre un swapchain DXGI (flip model).
//! Todas las coordenadas están en DIPs (1 DIP = 1 px a 96 DPI); Direct2D escala según el DPI.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_MULTI_THREADED, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext, ID2D1Factory1,
    D2D1_ELLIPSE, ID2D1PathGeometry1, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::D2D1_QUADRATIC_BEZIER_SEGMENT;
use windows_numerics::Vector2;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT, DXGI_SCALING_NONE,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, Result, w};

use crate::icons;
use crate::theme::Theme;

pub type Rect = D2D_RECT_F;

pub struct Renderer {
    d2d: ID2D1Factory1,
    dwrite: IDWriteFactory,
    fonts: Fonts,
    /// Dispositivo de GPU que se está creando en otro hilo (lo más lento del arranque).
    pending: Option<JoinHandle<Result<Gpu>>>,
    target: Option<Target>,
    icons: RefCell<Icons>,
    /// Fondo translúcido (Mica): se presenta con DirectComposition y lo no pintado deja ver
    /// el fondo del sistema.
    transparent: bool,
}

pub struct Fonts {
    /// Texto normal, alineado a la izquierda, con "…" si no entra.
    pub ui: IDWriteTextFormat,
    pub ui_right: IDWriteTextFormat,
    pub ui_center: IDWriteTextFormat,
    pub ui_bold: IDWriteTextFormat,
    /// Íconos de interfaz (Segoe Fluent Icons en Windows 11, MDL2 Assets en Windows 10).
    pub glyphs: IDWriteTextFormat,
    pub glyphs_small: IDWriteTextFormat,
}

struct Gpu {
    d3d: ID3D11Device,
    ctx: ID2D1DeviceContext,
}

/// Recursos que dependen del dispositivo y de la ventana: se recrean si la GPU se pierde.
struct Target {
    gpu: Gpu,
    swapchain: IDXGISwapChain1,
    brush: ID2D1SolidColorBrush,
    /// Árbol de DirectComposition cuando el fondo es translúcido (se mantiene vivo acá).
    composition: Option<(IDCompositionDevice, IDCompositionTarget, IDCompositionVisual)>,
}

impl Renderer {
    /// Arranca la creación del dispositivo de GPU en segundo plano y devuelve enseguida.
    pub fn new() -> Result<Self> {
        unsafe {
            // Multi-thread porque el dispositivo se crea en otro hilo.
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)?;
            let factory = d2d.clone();
            let pending = std::thread::spawn(move || create_gpu(&factory));

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let fonts = Fonts::new(&dwrite)?;
            Ok(Self {
                d2d,
                dwrite,
                fonts,
                pending: Some(pending),
                target: None,
                icons: RefCell::new(Icons::new(96)),
                transparent: false,
            })
        }
    }

    pub fn resize(&mut self, width: u32, height: u32, dpi: u32) {
        // Minimizada: no hay nada que redimensionar.
        if width == 0 || height == 0 {
            return;
        }
        if let Some(t) = &self.target {
            let ok = unsafe {
                // El swapchain no se puede redimensionar mientras D2D tenga referencias al buffer.
                t.gpu.ctx.SetTarget(None);
                t.swapchain
                    .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
                    .and_then(|_| attach_backbuffer(t, dpi))
            };
            if ok.is_err() {
                self.drop_target();
            }
        }
    }

    pub fn set_transparent(&mut self, on: bool) {
        if on != self.transparent {
            self.transparent = on;
            self.drop_target();
        }
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        if let Some(t) = &self.target {
            unsafe { t.gpu.ctx.SetDpi(dpi as f32, dpi as f32) };
        }
        // Los íconos se rasterizan al tamaño en píxeles del monitor.
        let hwnd = self.icons.borrow().hwnd;
        let mut icons = Icons::new(dpi);
        icons.hwnd = hwnd;
        *self.icons.borrow_mut() = icons;
    }

    /// Crea el target si no existe (también lo hace `draw`; separado para medir el arranque).
    pub fn ensure_target(&mut self, hwnd: HWND, dpi: u32) -> Result<()> {
        if self.target.is_none() {
            let gpu = match self.pending.take() {
                Some(handle) => handle.join().expect("hilo de creación de GPU")?,
                None => create_gpu(&self.d2d)?,
            };
            self.target = Some(create_target(gpu, hwnd, dpi, self.transparent)?);
            self.icons.borrow_mut().hwnd = hwnd;
        }
        Ok(())
    }

    /// Dibuja un frame: `paint` recibe el pintor y el tamaño del área en DIPs.
    pub fn draw(&mut self, hwnd: HWND, dpi: u32, theme: &Theme, paint: impl FnOnce(&Painter, f32, f32)) -> Result<()> {
        self.ensure_target(hwnd, dpi)?;
        let t = self.target.as_ref().unwrap();
        let ctx = &t.gpu.ctx;
        let painter = Painter {
            ctx,
            factory: &self.d2d,
            brush: &t.brush,
            theme,
            fonts: &self.fonts,
            dwrite: &self.dwrite,
            icons: &self.icons,
        };

        // Con fondo translúcido se limpia a transparente: ahí se ve Mica.
        // Con Mica el fondo se pinta casi opaco: el diseño manda, Mica apenas tiñe los huecos.
        let clear = if t.composition.is_some() { theme_alpha(theme.background, 0.88) } else { theme.background };
        unsafe {
            ctx.BeginDraw();
            ctx.Clear(Some(&clear));
        }
        let size = unsafe { ctx.GetSize() };
        paint(&painter, size.width, size.height);

        let drawn = unsafe { ctx.EndDraw(None, None) };
        if let Err(e) = &drawn {
            if e.code() == D2DERR_RECREATE_TARGET {
                self.drop_target();
                return Ok(());
            }
        }
        drawn?;

        // Sincronizado con el refresco del monitor.
        let hr = unsafe { t.swapchain.Present(1, DXGI_PRESENT(0)) };
        if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET {
            self.drop_target();
            return Ok(());
        }
        hr.ok()
    }

    fn drop_target(&mut self) {
        self.target = None;
        // Los bitmaps de íconos pertenecen al dispositivo perdido.
        self.icons.borrow_mut().bitmaps.clear();
    }
}

pub struct Painter<'a> {
    ctx: &'a ID2D1DeviceContext,
    factory: &'a ID2D1Factory1,
    brush: &'a ID2D1SolidColorBrush,
    pub theme: &'a Theme,
    pub fonts: &'a Fonts,
    dwrite: &'a IDWriteFactory,
    icons: &'a RefCell<Icons>,
}

impl Painter<'_> {
    fn color(&self, c: D2D1_COLOR_F) {
        unsafe { self.brush.SetColor(&c) };
    }

    pub fn fill(&self, r: Rect, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.ctx.FillRectangle(&r, self.brush) };
    }

    pub fn round_fill(&self, r: Rect, radius: f32, c: D2D1_COLOR_F) {
        self.color(c);
        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
        unsafe { self.ctx.FillRoundedRectangle(&rr, self.brush) };
    }

    /// Polígono con esquinas redondeadas: cada punto es (x, y, radio de la esquina). Abierto, los
    /// extremos no se redondean. Sirve para el contorno de panel + pestaña activa como una sola
    /// línea continua, y para las pestañas.
    pub fn rounded_path(&self, pts: &[(f32, f32, f32)], closed: bool, fill: Option<D2D1_COLOR_F>, stroke: Option<(D2D1_COLOR_F, f32)>) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        // Puntos donde arranca y termina la curva de cada esquina.
        let corner = |i: usize| -> (Vector2, Vector2, Vector2) {
            let (x, y, r) = pts[i];
            let p = v2(x, y);
            let prev = pts[(i + n - 1) % n];
            let next = pts[(i + 1) % n];
            let toward = |(qx, qy, _): (f32, f32, f32), d: f32| {
                let (dx, dy) = (qx - x, qy - y);
                let len = (dx * dx + dy * dy).sqrt().max(0.001);
                v2(x + dx / len * d, y + dy / len * d)
            };
            let dist = |(qx, qy, _): (f32, f32, f32)| ((qx - x).powi(2) + (qy - y).powi(2)).sqrt();
            let r = r.min(dist(prev) / 2.0).min(dist(next) / 2.0);
            (toward(prev, r), p, toward(next, r))
        };
        let shape = |closed: bool| -> Option<ID2D1PathGeometry1> {
            unsafe {
                let geo = self.factory.CreatePathGeometry().ok()?;
                let sink = geo.Open().ok()?;
                let begin = if closed { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW };
                if closed {
                    let (_, _, b0) = corner(0);
                    sink.BeginFigure(b0, begin);
                    for i in 1..=n {
                        let (a, c, b) = corner(i % n);
                        sink.AddLine(a);
                        sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: c, point2: b });
                    }
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                } else {
                    sink.BeginFigure(v2(pts[0].0, pts[0].1), begin);
                    for i in 1..n - 1 {
                        let (a, c, b) = corner(i);
                        sink.AddLine(a);
                        sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: c, point2: b });
                    }
                    sink.AddLine(v2(pts[n - 1].0, pts[n - 1].1));
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
                sink.Close().ok()?;
                Some(geo)
            }
        };
        unsafe {
            if let (Some(c), Some(geo)) = (fill, shape(true)) {
                self.color(c);
                self.ctx.FillGeometry(&geo, self.brush, None);
            }
            if let (Some((c, w)), Some(geo)) = (stroke, shape(closed)) {
                self.color(c);
                self.ctx.DrawGeometry(&geo, self.brush, w, None);
            }
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, fill: D2D1_COLOR_F) {
        self.color(fill);
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: radius, radiusY: radius };
        unsafe { self.ctx.FillEllipse(&e, self.brush) };
    }

    pub fn circle_stroke(&self, cx: f32, cy: f32, radius: f32, c: D2D1_COLOR_F, width: f32) {
        self.color(c);
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: radius, radiusY: radius };
        unsafe { self.ctx.DrawEllipse(&e, self.brush, width, None) };
    }

    /// Contorno redondeado de grosor arbitrario.
    pub fn round_stroke_w(&self, r: Rect, radius: f32, c: D2D1_COLOR_F, width: f32) {
        self.color(c);
        let h = width / 2.0;
        let r = rect(r.left + h, r.top + h, r.right - h, r.bottom - h);
        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
        unsafe { self.ctx.DrawRoundedRectangle(&rr, self.brush, width, None) };
    }

    pub fn round_stroke(&self, r: Rect, radius: f32, c: D2D1_COLOR_F) {
        self.color(c);
        // Medio DIP hacia adentro para que el trazo de 1 DIP quede nítido.
        let r = rect(r.left + 0.5, r.top + 0.5, r.right - 0.5, r.bottom - 0.5);
        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius };
        unsafe { self.ctx.DrawRoundedRectangle(&rr, self.brush, 1.0, None) };
    }

    pub fn text(&self, format: &IDWriteTextFormat, s: &str, r: Rect, c: D2D1_COLOR_F) {
        if r.right <= r.left {
            return;
        }
        self.color(c);
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            self.ctx.DrawText(
                &wide,
                format,
                &r,
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    pub fn measure(&self, format: &IDWriteTextFormat, s: &str) -> f32 {
        measure(self.dwrite, format, s)
    }

    /// Dibuja un ícono de la lista de imágenes del sistema en un cuadrado de `size` DIPs.
    pub fn shell_icon(&self, index: i32, overlay: u8, x: f32, y: f32, size: f32) {
        let Some(bitmap) = self.icons.borrow_mut().get(self.ctx, index, overlay) else { return };
        let dest = rect(x, y, x + size, y + size);
        unsafe {
            self.ctx.DrawBitmap(
                &bitmap,
                Some(&dest),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            )
        };
    }

    pub fn clip<R>(&self, r: Rect, f: impl FnOnce() -> R) -> R {
        unsafe {
            self.ctx.PushAxisAlignedClip(&r, windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let out = f();
        unsafe { self.ctx.PopAxisAlignedClip() };
        out
    }
}

impl Fonts {
    fn new(dwrite: &IDWriteFactory) -> Result<Self> {
        let family = if has_font(dwrite, "Segoe UI Variable Text") { w!("Segoe UI Variable Text") } else { w!("Segoe UI") };
        let glyph_family = if has_font(dwrite, "Segoe Fluent Icons") { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
        let make = |family: PCWSTR, weight, size, align| text_format(dwrite, family, weight, size, align);
        Ok(Self {
            ui: make(family, DWRITE_FONT_WEIGHT_NORMAL, 13.0, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            ui_right: make(family, DWRITE_FONT_WEIGHT_NORMAL, 13.0, DWRITE_TEXT_ALIGNMENT_TRAILING)?,
            ui_center: make(family, DWRITE_FONT_WEIGHT_NORMAL, 13.0, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            ui_bold: make(family, DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0, DWRITE_TEXT_ALIGNMENT_LEADING)?,
            glyphs: make(glyph_family, DWRITE_FONT_WEIGHT_NORMAL, 14.0, DWRITE_TEXT_ALIGNMENT_CENTER)?,
            glyphs_small: make(glyph_family, DWRITE_FONT_WEIGHT_NORMAL, 10.0, DWRITE_TEXT_ALIGNMENT_CENTER)?,
        })
    }
}

fn has_font(dwrite: &IDWriteFactory, name: &str) -> bool {
    unsafe {
        let mut collection: Option<IDWriteFontCollection> = None;
        if dwrite.GetSystemFontCollection(&mut collection, false).is_err() {
            return false;
        }
        let Some(collection) = collection else { return false };
        let (mut index, mut exists) = (0u32, BOOL(0));
        collection.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists).is_ok() && exists.as_bool()
    }
}

fn text_format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    align: DWRITE_TEXT_ALIGNMENT,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let f = dwrite.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("es-es"),
        )?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetTextAlignment(align)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        let ellipsis = dwrite.CreateEllipsisTrimmingSign(&f)?;
        f.SetTrimming(&trimming, &ellipsis)?;
        Ok(f)
    }
}

fn measure(dwrite: &IDWriteFactory, format: &IDWriteTextFormat, s: &str) -> f32 {
    let wide: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let Ok(layout) = dwrite.CreateTextLayout(&wide, format, 100_000.0, 100.0) else { return 0.0 };
        let mut m = DWRITE_TEXT_METRICS::default();
        if layout.GetMetrics(&mut m).is_err() {
            return 0.0;
        }
        m.widthIncludingTrailingWhitespace
    }
}

/// Bitmaps de Direct2D hechos con los píxeles que prepara `crate::icons`, cacheados por índice.
struct Icons {
    kind: icons::ListKind,
    hwnd: HWND,
    bitmaps: HashMap<(i32, u8), Option<ID2D1Bitmap1>>,
}

impl Icons {
    fn new(dpi: u32) -> Self {
        let kind = icons::list_for_dpi(dpi);
        icons::prewarm(kind);
        Self { kind, hwnd: HWND::default(), bitmaps: HashMap::new() }
    }

    fn get(&mut self, ctx: &ID2D1DeviceContext, index: i32, overlay: u8) -> Option<ID2D1Bitmap1> {
        if let Some(hit) = self.bitmaps.get(&(index, overlay)) {
            return hit.clone();
        }
        // Todavía no está: se dibuja sin ícono y se reintenta cuando avise el hilo.
        let pixels = icons::get(self.hwnd, self.kind, index, overlay)?;
        let bitmap = pixels.and_then(|px| unsafe {
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };
            let size = D2D_SIZE_U { width: px.width, height: px.height };
            ctx.CreateBitmap(size, Some(px.data.as_ptr() as *const c_void), px.width * 4, &props).ok()
        });
        self.bitmaps.insert((index, overlay), bitmap.clone());
        bitmap
    }
}

fn create_gpu(factory: &ID2D1Factory1) -> Result<Gpu> {
    // Si no hay GPU (escritorio remoto, VM) se usa WARP, el rasterizador por software de Windows.
    let d3d = create_d3d(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create_d3d(D3D_DRIVER_TYPE_WARP))?;
    unsafe {
        let device = factory.CreateDevice(&d3d.cast::<IDXGIDevice>()?)?;
        let ctx = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        Ok(Gpu { d3d, ctx })
    }
}

fn create_d3d(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    Ok(device.unwrap())
}

fn create_target(gpu: Gpu, hwnd: HWND, dpi: u32, transparent: bool) -> Result<Target> {
    let mut rc = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rc)?;
        let dxgi: IDXGIDevice = gpu.d3d.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
        let mut desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: (rc.right - rc.left).max(1) as u32,
            Height: (rc.bottom - rc.top).max(1) as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
            ..Default::default()
        };

        // Translúcido: swapchain de composición con alfa, presentado con DirectComposition
        // debajo de la superficie GDI de la ventana (así los controles de edición siguen arriba).
        // Si algo falla, queda el modo opaco de siempre.
        let mut composition = None;
        let mut swapchain = None;
        if transparent {
            let comp_desc = DXGI_SWAP_CHAIN_DESC1 {
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                ..desc
            };
            let made = (|| -> Result<_> {
                let sc = factory.CreateSwapChainForComposition(&gpu.d3d, &comp_desc, None)?;
                let device: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
                let target = device.CreateTargetForHwnd(hwnd, false)?;
                let visual = device.CreateVisual()?;
                visual.SetContent(&sc)?;
                target.SetRoot(&visual)?;
                device.Commit()?;
                Ok((sc, (device, target, visual)))
            })();
            if let Ok((sc, tree)) = made {
                swapchain = Some(sc);
                composition = Some(tree);
                desc = comp_desc;
            }
        }
        let swapchain = match swapchain {
            Some(sc) => sc,
            None => factory.CreateSwapChainForHwnd(&gpu.d3d, hwnd, &desc, None, None)?,
        };
        let brush = gpu.ctx.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
        // Escala de grises como WinUI: ClearType deja bordes de color en glifos finos.
        gpu.ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let target = Target { gpu, swapchain, brush, composition };
        attach_backbuffer(&target, dpi)?;
        Ok(target)
    }
}

/// Apunta el contexto de D2D al buffer trasero actual del swapchain.
fn attach_backbuffer(t: &Target, dpi: u32) -> Result<()> {
    unsafe {
        let surface: IDXGISurface = t.swapchain.GetBuffer(0)?;
        let alpha = if t.composition.is_some() { D2D1_ALPHA_MODE_PREMULTIPLIED } else { D2D1_ALPHA_MODE_IGNORE };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: alpha },
            dpiX: dpi as f32,
            dpiY: dpi as f32,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        let bitmap = t.gpu.ctx.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
        t.gpu.ctx.SetTarget(&bitmap);
        t.gpu.ctx.SetDpi(dpi as f32, dpi as f32);
    }
    Ok(())
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn theme_alpha(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a, ..c }
}

pub fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    D2D_RECT_F { left, top, right, bottom }
}

pub fn contains(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

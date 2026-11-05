//! macOS: dibujo con Core Graphics sobre el contexto de la vista (que es "flipped": y crece hacia
//! abajo, como en Windows). El texto se dibuja con AppKit y los íconos de interfaz son SF Symbols
//! (equivalentes a los glifos de Segoe Fluent Icons que usa Windows).

use std::cell::RefCell;
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSColor, NSCompositingOperation, NSFont, NSFontAttributeName, NSFontWeightRegular, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSImageSymbolConfiguration, NSLineBreakMode,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStringDrawing, NSStringDrawingOptions,
    NSStringNSExtendedStringDrawing, NSTextAlignment,
};
use objc2_core_foundation::{CFData, CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::{NSArray, NSDictionary, NSString};

use super::{Color, Rect, rect};
use crate::config::ColorRule;
use crate::icons::{self, Pixels};
use crate::platform::Handle;
use crate::theme::Theme;
use crate::thumbs;

/// Formato de texto: fuente, alineación y si corta en varias líneas. Los formatos de glifos
/// dibujan SF Symbols.
pub struct TextFormat {
    font: Retained<NSFont>,
    para: Retained<NSMutableParagraphStyle>,
    /// Varias líneas, arriba (nombre debajo de una miniatura).
    wrap: bool,
    /// Íconos de interfaz: el texto es un glifo de Segoe Fluent Icons.
    glyph: bool,
    size: f32,
}

pub struct Fonts {
    /// Texto normal, alineado a la izquierda, con "…" si no entra.
    pub ui: TextFormat,
    pub ui_right: TextFormat,
    pub ui_center: TextFormat,
    pub ui_bold: TextFormat,
    /// Íconos de interfaz (SF Symbols).
    pub glyphs: TextFormat,
    pub glyphs_small: TextFormat,
    /// Nombre debajo de una miniatura: centrado, arriba, partido en varias líneas y con "…" si no entra.
    pub label: TextFormat,
}

pub struct Renderer {
    fonts: Fonts,
    /// Tamaño del área en puntos (lo fija la vista antes de dibujar).
    size: (f32, f32),
    /// Píxeles por punto de la pantalla (2 en Retina).
    scale: f32,
    images: RefCell<Images>,
}

/// Imágenes ya convertidas para dibujar.
#[derive(Default)]
struct Images {
    thumbs: HashMap<thumbs::Key, Option<CFRetained<CGImage>>>,
    logo: Option<CFRetained<CGImage>>,
    /// SF Symbols por (glifo, tamaño, color).
    symbols: HashMap<(char, u32, [u8; 4]), Option<Retained<NSImage>>>,
    handle: Handle,
}

/// Más miniaturas que esto en memoria y se descartan todas (se vuelven a pedir las que se vean).
const MAX_THUMBS: usize = 1500;

impl Renderer {
    pub fn new() -> Result<Self, String> {
        Ok(Self { fonts: Fonts::new(), size: (0.0, 0.0), scale: 2.0, images: RefCell::new(Images::default()) })
    }

    /// Ancho de un texto con la fuente de la interfaz (o su negrita, la de los encabezados).
    pub fn text_width(&self, s: &str, bold: bool) -> f32 {
        measure(if bold { &self.fonts.ui_bold } else { &self.fonts.ui }, s)
    }

    /// Tamaño del área en puntos; `dpi` = 96 × píxeles por punto.
    #[allow(dead_code)]
    pub fn resize(&mut self, width: u32, height: u32, dpi: u32) {
        self.size = (width as f32, height as f32);
        self.set_dpi(dpi);
    }

    pub fn set_size(&mut self, width: f32, height: f32, scale: f32) {
        self.size = (width, height);
        self.scale = scale.max(1.0);
    }

    pub fn set_transparent(&mut self, _on: bool) {}

    #[allow(dead_code)]
    pub fn set_dpi(&mut self, dpi: u32) {
        self.scale = (dpi as f32 / 96.0).max(1.0);
    }

    pub fn ensure_target(&mut self, owner: Handle, _dpi: u32) -> Result<(), String> {
        self.images.borrow_mut().handle = owner;
        Ok(())
    }

    /// Dibuja un frame en el contexto actual de AppKit: `paint` recibe el pintor y el tamaño.
    pub fn draw(
        &mut self,
        owner: Handle,
        _dpi: u32,
        theme: &Theme,
        file_colors: &[ColorRule],
        paint: impl FnOnce(&Painter, f32, f32),
    ) -> Result<(), String> {
        self.images.borrow_mut().handle = owner;
        let Some(ctx) = NSGraphicsContext::currentContext() else { return Ok(()) };
        let cg = ctx.CGContext();
        let painter = Painter { cg: &cg, theme, fonts: &self.fonts, file_colors, images: &self.images, scale: self.scale };
        let (w, h) = self.size;
        painter.fill(rect(0.0, 0.0, w, h), theme.background);
        paint(&painter, w, h);
        Ok(())
    }
}

pub struct Painter<'a> {
    cg: &'a CGContext,
    pub theme: &'a Theme,
    pub fonts: &'a Fonts,
    pub file_colors: &'a [ColorRule],
    images: &'a RefCell<Images>,
    scale: f32,
}

fn cgr(r: Rect) -> CGRect {
    CGRect::new(CGPoint::new(r.left as CGFloat, r.top as CGFloat), CGSize::new((r.right - r.left) as CGFloat, (r.bottom - r.top) as CGFloat))
}

fn ns_color(c: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r as CGFloat, c.g as CGFloat, c.b as CGFloat, c.a as CGFloat)
}

impl Painter<'_> {
    fn ctx(&self) -> Option<&CGContext> {
        Some(self.cg)
    }

    fn set_fill(&self, c: Color) {
        CGContext::set_rgb_fill_color(self.ctx(), c.r as CGFloat, c.g as CGFloat, c.b as CGFloat, c.a as CGFloat);
    }

    fn set_stroke(&self, c: Color, width: f32) {
        CGContext::set_rgb_stroke_color(self.ctx(), c.r as CGFloat, c.g as CGFloat, c.b as CGFloat, c.a as CGFloat);
        CGContext::set_line_width(self.ctx(), width as CGFloat);
    }

    /// Agrega al trazo actual un rectángulo con esquinas redondeadas.
    fn add_round_rect(&self, r: Rect, radius: f32) {
        let radius = radius.min((r.right - r.left) / 2.0).min((r.bottom - r.top) / 2.0).max(0.0) as CGFloat;
        let (l, t, rr, b) = (r.left as CGFloat, r.top as CGFloat, r.right as CGFloat, r.bottom as CGFloat);
        let c = self.ctx();
        CGContext::move_to_point(c, l + radius, t);
        CGContext::add_arc_to_point(c, rr, t, rr, b, radius);
        CGContext::add_arc_to_point(c, rr, b, l, b, radius);
        CGContext::add_arc_to_point(c, l, b, l, t, radius);
        CGContext::add_arc_to_point(c, l, t, rr, t, radius);
        CGContext::close_path(c);
    }

    pub fn fill(&self, r: Rect, c: Color) {
        self.set_fill(c);
        CGContext::fill_rect(self.ctx(), cgr(r));
    }

    pub fn round_fill(&self, r: Rect, radius: f32, c: Color) {
        self.set_fill(c);
        CGContext::begin_path(self.ctx());
        self.add_round_rect(r, radius);
        CGContext::fill_path(self.ctx());
    }

    /// Polígono con esquinas redondeadas: cada punto es (x, y, radio de la esquina). Abierto, los
    /// extremos no se redondean.
    pub fn rounded_path(&self, pts: &[(f32, f32, f32)], closed: bool, fill: Option<Color>, stroke: Option<(Color, f32)>) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        let corner = |i: usize| -> ((f32, f32), (f32, f32), (f32, f32)) {
            let (x, y, r) = pts[i];
            let prev = pts[(i + n - 1) % n];
            let next = pts[(i + 1) % n];
            let toward = |(qx, qy, _): (f32, f32, f32), d: f32| {
                let (dx, dy) = (qx - x, qy - y);
                let len = (dx * dx + dy * dy).sqrt().max(0.001);
                (x + dx / len * d, y + dy / len * d)
            };
            let dist = |(qx, qy, _): (f32, f32, f32)| ((qx - x).powi(2) + (qy - y).powi(2)).sqrt();
            let r = r.min(dist(prev) / 2.0).min(dist(next) / 2.0);
            (toward(prev, r), (x, y), toward(next, r))
        };
        let c = self.ctx();
        let shape = |closed: bool| {
            CGContext::begin_path(c);
            if closed {
                let (_, _, b0) = corner(0);
                CGContext::move_to_point(c, b0.0 as CGFloat, b0.1 as CGFloat);
                for i in 1..=n {
                    let (a, p, b) = corner(i % n);
                    CGContext::add_line_to_point(c, a.0 as CGFloat, a.1 as CGFloat);
                    CGContext::add_quad_curve_to_point(c, p.0 as CGFloat, p.1 as CGFloat, b.0 as CGFloat, b.1 as CGFloat);
                }
                CGContext::close_path(c);
            } else {
                CGContext::move_to_point(c, pts[0].0 as CGFloat, pts[0].1 as CGFloat);
                for i in 1..n - 1 {
                    let (a, p, b) = corner(i);
                    CGContext::add_line_to_point(c, a.0 as CGFloat, a.1 as CGFloat);
                    CGContext::add_quad_curve_to_point(c, p.0 as CGFloat, p.1 as CGFloat, b.0 as CGFloat, b.1 as CGFloat);
                }
                CGContext::add_line_to_point(c, pts[n - 1].0 as CGFloat, pts[n - 1].1 as CGFloat);
            }
        };
        if let Some(col) = fill {
            self.set_fill(col);
            shape(true);
            CGContext::fill_path(c);
        }
        if let Some((col, w)) = stroke {
            self.set_stroke(col, w);
            shape(closed);
            CGContext::stroke_path(c);
        }
    }

    pub fn circle(&self, cx: f32, cy: f32, radius: f32, fill: Color) {
        self.set_fill(fill);
        CGContext::fill_ellipse_in_rect(self.ctx(), cgr(rect(cx - radius, cy - radius, cx + radius, cy + radius)));
    }

    pub fn circle_stroke(&self, cx: f32, cy: f32, radius: f32, c: Color, width: f32) {
        self.set_stroke(c, width);
        CGContext::stroke_ellipse_in_rect(self.ctx(), cgr(rect(cx - radius, cy - radius, cx + radius, cy + radius)));
    }

    /// Contorno redondeado de grosor arbitrario.
    pub fn round_stroke_w(&self, r: Rect, radius: f32, c: Color, width: f32) {
        self.set_stroke(c, width);
        let h = width / 2.0;
        CGContext::begin_path(self.ctx());
        self.add_round_rect(rect(r.left + h, r.top + h, r.right - h, r.bottom - h), radius);
        CGContext::stroke_path(self.ctx());
    }

    pub fn round_stroke(&self, r: Rect, radius: f32, c: Color) {
        self.round_stroke_w(r, radius, c, 1.0);
    }

    pub fn text(&self, format: &TextFormat, s: &str, r: Rect, c: Color) {
        if r.right <= r.left || s.is_empty() {
            return;
        }
        if format.glyph {
            if let Some(ch) = s.chars().next() {
                self.symbol(ch, format.size, r, c);
            }
            return;
        }
        let attrs = attributes(format, Some(c));
        let text = NSString::from_str(s);
        let options = NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::TruncatesLastVisibleLine;
        let w = (r.right - r.left) as CGFloat;
        let target = if format.wrap {
            CGRect::new(CGPoint::new(r.left as CGFloat, r.top as CGFloat), CGSize::new(w, (r.bottom - r.top) as CGFloat))
        } else {
            // Una línea centrada a lo alto (como DirectWrite con párrafo centrado).
            let line = line_height(&format.font);
            let top = r.top as CGFloat + ((r.bottom - r.top) as CGFloat - line) / 2.0;
            CGRect::new(CGPoint::new(r.left as CGFloat, top), CGSize::new(w, line))
        };
        CGContext::save_g_state(self.ctx());
        CGContext::clip_to_rect(self.ctx(), cgr(r));
        unsafe { text.drawWithRect_options_attributes_context(target, options, Some(&attrs), None) };
        CGContext::restore_g_state(self.ctx());
    }

    pub fn measure(&self, format: &TextFormat, s: &str) -> f32 {
        measure(format, s)
    }

    /// Dibuja el ícono `index` (ver `icons`) en un cuadrado de `size` DIPs.
    pub fn shell_icon(&self, index: i32, _overlay: u8, x: f32, y: f32, size: f32) {
        let Some(image) = icons::image(index) else { return };
        draw_ns_image(&image, rect(x, y, x + size, y + size));
    }

    /// Miniatura centrada en `r`, sin deformar y sin agrandarla más que su tamaño en píxeles.
    /// Devuelve false si todavía no está (se dibuja otra cosa).
    pub fn thumbnail(&self, key: &thumbs::Key, r: Rect) -> bool {
        let image = {
            let mut images = self.images.borrow_mut();
            for (k, px) in thumbs::take_done() {
                let img = px.and_then(|px| cgimage_from(&px));
                images.thumbs.insert(k, img);
            }
            match images.thumbs.get(key) {
                Some(hit) => hit.clone(),
                None => {
                    if images.thumbs.len() > MAX_THUMBS {
                        images.thumbs.clear();
                    }
                    thumbs::request(images.handle, key.clone());
                    None
                }
            }
        };
        let Some(image) = image else { return false };
        let (bw, bh) = (CGImage::width(Some(&image)) as f32, CGImage::height(Some(&image)) as f32);
        if bw <= 0.0 || bh <= 0.0 {
            return false;
        }
        let k = ((r.right - r.left) / bw).min((r.bottom - r.top) / bh).min(1.0 / self.scale.max(0.01));
        let (w, h) = (bw * k, bh * k);
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        self.draw_cg_image(&image, rect(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0));
        true
    }

    /// Logo de Runick centrado en `r` sin deformarlo.
    pub fn logo(&self, r: Rect) {
        let image = {
            let mut images = self.images.borrow_mut();
            if images.logo.is_none() {
                images.logo = crate::about::logo_pixels().and_then(|px| cgimage_from(&px));
            }
            images.logo.clone()
        };
        let Some(image) = image else { return };
        let (bw, bh) = (CGImage::width(Some(&image)) as f32, CGImage::height(Some(&image)) as f32);
        let k = ((r.right - r.left) / bw).min((r.bottom - r.top) / bh);
        let (w, h) = (bw * k, bh * k);
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        self.draw_cg_image(&image, rect(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0));
    }

    /// Lado en píxeles para pedir una miniatura de `dips` DIPs.
    pub fn thumb_px(&self, dips: f32) -> u32 {
        (dips * self.scale).round() as u32
    }

    pub fn clip<R>(&self, r: Rect, f: impl FnOnce() -> R) -> R {
        CGContext::save_g_state(self.ctx());
        CGContext::clip_to_rect(self.ctx(), cgr(r));
        let out = f();
        CGContext::restore_g_state(self.ctx());
        out
    }

    /// Una imagen de Core Graphics en `r`: la vista está invertida, así que se dibuja dándola vuelta.
    fn draw_cg_image(&self, image: &CGImage, r: Rect) {
        let c = self.ctx();
        CGContext::save_g_state(c);
        CGContext::translate_ctm(c, r.left as CGFloat, r.bottom as CGFloat);
        CGContext::scale_ctm(c, 1.0, -1.0);
        CGContext::set_interpolation_quality(c, objc2_core_graphics::CGInterpolationQuality::High);
        let size = CGSize::new((r.right - r.left) as CGFloat, (r.bottom - r.top) as CGFloat);
        CGContext::draw_image(c, CGRect::new(CGPoint::new(0.0, 0.0), size), Some(image));
        CGContext::restore_g_state(c);
    }

    /// Un glifo de Segoe Fluent Icons dibujado con su equivalente de SF Symbols, centrado en `r`.
    fn symbol(&self, ch: char, size: f32, r: Rect, c: Color) {
        let key = (ch, size.round() as u32, [(c.r * 255.0) as u8, (c.g * 255.0) as u8, (c.b * 255.0) as u8, (c.a * 255.0) as u8]);
        let image = {
            let mut images = self.images.borrow_mut();
            images.symbols.entry(key).or_insert_with(|| symbol_image(ch, size, c)).clone()
        };
        let Some(image) = image else { return };
        let s = image.size();
        let (w, h) = (s.width as f32, s.height as f32);
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        draw_ns_image(&image, rect(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0));
    }
}

/// Dibuja una imagen de AppKit respetando que la vista está invertida.
fn draw_ns_image(image: &NSImage, r: Rect) {
    let zero = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
    unsafe {
        image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(cgr(r), zero, NSCompositingOperation::SourceOver, 1.0, true, None);
    }
}

/// Píxeles BGRA premultiplicados → imagen de Core Graphics.
fn cgimage_from(px: &Pixels) -> Option<CFRetained<CGImage>> {
    if px.width == 0 || px.height == 0 || px.data.len() < (px.width * px.height * 4) as usize {
        return None;
    }
    let data = CFData::from_bytes(&px.data);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let space = CGColorSpace::new_device_rgb()?;
    let info = CGBitmapInfo(objc2_core_graphics::CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0);
    unsafe {
        CGImage::new(
            px.width as usize,
            px.height as usize,
            8,
            32,
            px.width as usize * 4,
            Some(&space),
            info,
            Some(&provider),
            std::ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

fn line_height(font: &NSFont) -> CGFloat {
    (font.ascender() - font.descender() + font.leading()).ceil()
}

fn attributes(format: &TextFormat, color: Option<Color>) -> Retained<NSDictionary<NSString, AnyObject>> {
    let font: &AnyObject = &format.font;
    let para: &AnyObject = &format.para;
    unsafe {
        match color {
            Some(c) => {
                let color = ns_color(c);
                let color: &AnyObject = &color;
                NSDictionary::from_slices(
                    &[NSFontAttributeName, NSParagraphStyleAttributeName, NSForegroundColorAttributeName],
                    &[font, para, color],
                )
            }
            None => NSDictionary::from_slices(&[NSFontAttributeName, NSParagraphStyleAttributeName], &[font, para]),
        }
    }
}

fn measure(format: &TextFormat, s: &str) -> f32 {
    if format.glyph {
        return format.size;
    }
    let attrs = attributes(format, None);
    let size = unsafe { NSString::from_str(s).sizeWithAttributes(Some(&attrs)) };
    size.width as f32
}

impl Fonts {
    fn new() -> Self {
        let make = |size: f32, bold: bool, align: NSTextAlignment, wrap: bool, glyph: bool| {
            let weight = unsafe { if bold { NSFontWeightSemibold } else { NSFontWeightRegular } };
            let font = NSFont::systemFontOfSize_weight(size as CGFloat, weight);
            let para = NSMutableParagraphStyle::new();
            para.setAlignment(align);
            para.setLineBreakMode(if wrap { NSLineBreakMode::ByWordWrapping } else { NSLineBreakMode::ByTruncatingTail });
            TextFormat { font, para, wrap, glyph, size }
        };
        Fonts {
            ui: make(13.0, false, NSTextAlignment::Left, false, false),
            ui_right: make(13.0, false, NSTextAlignment::Right, false, false),
            ui_center: make(13.0, false, NSTextAlignment::Center, false, false),
            ui_bold: make(13.0, true, NSTextAlignment::Left, false, false),
            glyphs: make(13.0, false, NSTextAlignment::Center, false, true),
            glyphs_small: make(10.0, false, NSTextAlignment::Center, false, true),
            label: make(12.0, false, NSTextAlignment::Center, true, false),
        }
    }
}

/// SF Symbol para un glifo de Segoe Fluent Icons / MDL2 Assets.
fn symbol_name(ch: char) -> Option<&'static str> {
    Some(match ch as u32 {
        0xE708 => "moon",
        0xE70A => "menubar.dock.rectangle",
        0xE70D => "chevron.down",
        0xE70E => "chevron.up",
        0xE710 => "plus",
        0xE711 => "xmark",
        0xE712 => "ellipsis",
        0xE713 => "gearshape",
        0xE714 => "film",
        0xE718 => "pin",
        0xE71C => "line.3.horizontal.decrease",
        0xE721 => "magnifyingglass",
        0xE72A => "arrow.right",
        0xE72B => "arrow.left",
        0xE72C => "arrow.clockwise",
        0xE734 => "star",
        0xE735 => "star.fill",
        0xE73E => "checkmark",
        0xE73F => "arrow.down.right.and.arrow.up.left",
        0xE740 => "arrow.up.left.and.arrow.down.right",
        0xE74A => "arrow.up",
        0xE74E => "square.and.arrow.down",
        0xE753 => "cloud",
        0xE756 => "terminal",
        0xE76C => "chevron.right",
        0xE76B => "chevron.left",
        0xE783 => "exclamationmark.circle",
        0xE790 => "paintpalette",
        0xE7F4 => "desktopcomputer",
        0xE7F8 => "laptopcomputer",
        0xE80F => "house",
        0xE838 => "folder",
        0xE839 => "network",
        0xE840 => "pin.fill",
        0xE896 => "arrow.down.circle",
        0xE8A5 => "doc",
        0xE8A7 => "macwindow.on.rectangle",
        0xE8A9 => "square.grid.2x2",
        0xE8AD => "arrow.left.arrow.right",
        0xE8B7 => "folder",
        0xE8C8 => "doc.on.doc",
        0xE8D5 => "folder.fill",
        0xE8D6 => "music.note",
        0xE8EC => "tag",
        0xE8F4 => "folder.badge.plus",
        0xE91B => "photo",
        0xE946 => "info.circle",
        0xE9D9 => "chart.pie",
        0xEDA2 => "internaldrive",
        _ => return None,
    })
}

fn symbol_image(ch: char, size: f32, c: Color) -> Option<Retained<NSImage>> {
    let name = symbol_name(ch)?;
    let base = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let sized = NSImageSymbolConfiguration::configurationWithPointSize_weight(size as CGFloat, unsafe { NSFontWeightRegular });
    let colors = NSArray::from_retained_slice(&[ns_color(c)]);
    let palette = NSImageSymbolConfiguration::configurationWithPaletteColors(&colors);
    let config = sized.configurationByApplyingConfiguration(&palette);
    base.imageWithSymbolConfiguration(&config)
}


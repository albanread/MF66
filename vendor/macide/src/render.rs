//! Core Graphics / Core Text rasteriser for the `locus-ide-protocol` `DrawCmd`
//! IR — the macOS analogue of the Windows IDE's `igui` Direct2D batch executor.
//!
//! It walks a `&[DrawCmd]` and issues the equivalent Core Graphics calls against
//! a `CGBitmapContext`, so it renders **headlessly** (no window, no display).
//! That makes the whole drawing path unit-testable: render a batch, read back
//! pixels, assert. The AppKit window shell (`window.rs`, `mac-gui`-gated) reuses
//! the very same canvas and just blits its `cg_image()` into an `NSImageView`.
//!
//! Coordinate system: the protocol (like Direct2D) is top-left origin, y-down.
//! Core Graphics is bottom-left, y-up. We install a one-time flip
//! (`translate(0,h); scale(1,-1)`) so protocol coordinates map directly and
//! memory row `y` corresponds to protocol `y` — `pixel(x, y)` reads `(y*w+x)*4`.
//!
//! Pixels are RGBA8888, premultiplied-last (`kCGImageAlphaPremultipliedLast`).

use core_foundation::attributed_string::CFMutableAttributedString;
use core_foundation::base::{CFRange, TCFType};
use core_foundation::string::CFString;
use core_graphics::base::kCGImageAlphaPremultipliedLast;
use core_graphics::color::CGColor;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGAffineTransform, CGPoint, CGRect, CGSize};
use core_text::line::CTLine;
use core_text::string_attributes::{kCTFontAttributeName, kCTForegroundColorAttributeName};

use locus_ide_protocol::draw::{Color, DrawBatch, DrawCmd, Point, Rect};

/// The default monospace family for `DrawText` (a code IDE wants a fixed pitch);
/// falls back to Helvetica if unavailable.
pub const DEFAULT_TEXT_FAMILY: &str = "Menlo";

/// Measured text geometry — the macOS analogue of a DirectWrite `GetMetrics`.
#[derive(Debug, Clone, Copy)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
}

#[inline]
fn rect_cg(r: &Rect) -> CGRect {
    let x = r.left.min(r.right) as f64;
    let y = r.top.min(r.bottom) as f64;
    let w = (r.right - r.left).abs() as f64;
    let h = (r.bottom - r.top).abs() as f64;
    CGRect::new(&CGPoint::new(x, y), &CGSize::new(w, h))
}

#[inline]
fn circle_rect(c: Point, radius: f32) -> CGRect {
    let r = radius as f64;
    CGRect::new(
        &CGPoint::new(c.x as f64 - r, c.y as f64 - r),
        &CGSize::new(2.0 * r, 2.0 * r),
    )
}

/// Build a `CTLine` (and its font ascent) for `text` at `size`/`color` in
/// `family`. Falls back to Helvetica if the family is unavailable.
fn build_line(text: &str, size: f32, color: Color, family: &str) -> Option<(CTLine, f32)> {
    let font = core_text::font::new_from_name(family, size as f64)
        .or_else(|_| core_text::font::new_from_name("Helvetica", size as f64))
        .ok()?;
    let ascent = font.ascent() as f32;

    let mut attr = CFMutableAttributedString::new();
    let cfstr = CFString::new(text);
    attr.replace_str(&cfstr, CFRange { location: 0, length: 0 });
    let len = attr.char_len();
    let whole = CFRange { location: 0, length: len };
    let cg = CGColor::rgb(color.r as f64, color.g as f64, color.b as f64, color.a as f64);
    unsafe {
        attr.set_attribute(whole, kCTFontAttributeName, &font);
        attr.set_attribute(whole, kCTForegroundColorAttributeName, &cg);
    }
    let line = CTLine::new_with_attributed_string(attr.as_concrete_TypeRef());
    Some((line, ascent))
}

/// A headless Core Graphics drawing surface. Drawing is in **points**
/// (`pw`×`ph`); the backing bitmap is `scale`× larger per axis, so on a Retina
/// display the rasterised output is full-resolution while `DrawCmd` coordinates
/// stay in points.
pub struct CgCanvas {
    ctx: CGContext,
    pw: usize,
    ph: usize,
    scale: f64,
    text_family: String,
}

impl CgCanvas {
    /// A `width`×`height` point canvas at 1× (memory row y == protocol y).
    pub fn new(width: usize, height: usize) -> Self {
        Self::new_scaled(width, height, 1.0)
    }

    /// A `pw`×`ph` **point** canvas whose backing bitmap is `scale`× larger per
    /// axis (HiDPI). Drawing is in points; pixels are crisp.
    pub fn new_scaled(pw: usize, ph: usize, scale: f64) -> Self {
        let scale = if scale >= 1.0 { scale } else { 1.0 };
        let pix_w = ((pw as f64) * scale).round() as usize;
        let pix_h = ((ph as f64) * scale).round() as usize;
        let cs = CGColorSpace::create_device_rgb();
        let ctx = CGContext::create_bitmap_context(
            None,
            pix_w,
            pix_h,
            8,
            pix_w * 4,
            &cs,
            kCGImageAlphaPremultipliedLast,
        );
        // Flip to top-left, y-down, then scale points→pixels.
        ctx.translate(0.0, pix_h as f64);
        ctx.scale(1.0, -1.0);
        ctx.scale(scale, scale);
        Self {
            ctx,
            pw,
            ph,
            scale,
            text_family: DEFAULT_TEXT_FAMILY.to_string(),
        }
    }

    /// Override the monospace family used for `DrawText` (e.g. an IDE theme font).
    pub fn set_text_family(&mut self, family: impl Into<String>) {
        self.text_family = family.into();
    }

    fn pix_w(&self) -> usize {
        ((self.pw as f64) * self.scale).round() as usize
    }
    fn pix_h(&self) -> usize {
        ((self.ph as f64) * self.scale).round() as usize
    }

    pub fn width(&self) -> usize {
        self.pw
    }
    pub fn height(&self) -> usize {
        self.ph
    }

    #[inline]
    fn set_fill(&self, c: Color) {
        self.ctx
            .set_rgb_fill_color(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
    }

    #[inline]
    fn set_stroke(&self, c: Color, thickness: f32) {
        self.ctx
            .set_rgb_stroke_color(c.r as f64, c.g as f64, c.b as f64, c.a as f64);
        self.ctx.set_line_width(thickness.max(0.0) as f64);
    }

    /// Rasterise a whole validated `DrawBatch`.
    pub fn execute_batch(&mut self, batch: &DrawBatch) {
        self.execute(&batch.commands);
    }

    /// Rasterise every command in `cmds`.
    pub fn execute(&mut self, cmds: &[DrawCmd]) {
        for cmd in cmds {
            self.exec_one(cmd);
        }
        self.ctx.flush();
    }

    fn exec_one(&mut self, cmd: &DrawCmd) {
        let ctx = &self.ctx;
        match cmd {
            DrawCmd::Clear(color) => {
                self.set_fill(*color);
                ctx.fill_rect(CGRect::new(
                    &CGPoint::new(0.0, 0.0),
                    &CGSize::new(self.pw as f64, self.ph as f64),
                ));
            }
            DrawCmd::FillRect { rect, color } => {
                self.set_fill(*color);
                ctx.fill_rect(rect_cg(rect));
            }
            DrawCmd::StrokeRect {
                rect,
                color,
                thickness,
            } => {
                self.set_stroke(*color, *thickness);
                ctx.stroke_rect_with_width(rect_cg(rect), thickness.max(0.0) as f64);
            }
            DrawCmd::DrawLine {
                from,
                to,
                color,
                thickness,
            } => {
                self.set_stroke(*color, *thickness);
                ctx.stroke_line_segments(&[
                    CGPoint::new(from.x as f64, from.y as f64),
                    CGPoint::new(to.x as f64, to.y as f64),
                ]);
            }
            DrawCmd::FillOval { rect, color } => {
                self.set_fill(*color);
                ctx.fill_ellipse_in_rect(rect_cg(rect));
            }
            DrawCmd::StrokeOval {
                rect,
                color,
                thickness,
            } => {
                self.set_stroke(*color, *thickness);
                ctx.stroke_ellipse_in_rect(rect_cg(rect));
            }
            DrawCmd::FillCircle {
                center,
                radius,
                color,
            } => {
                self.set_fill(*color);
                ctx.fill_ellipse_in_rect(circle_rect(*center, *radius));
            }
            DrawCmd::StrokeCircle {
                center,
                radius,
                color,
                thickness,
            } => {
                self.set_stroke(*color, *thickness);
                ctx.stroke_ellipse_in_rect(circle_rect(*center, *radius));
            }
            DrawCmd::DrawArc {
                center,
                radius,
                rotation_rad,
                half_aperture_rad,
                color,
                thickness,
            } => {
                self.set_stroke(*color, *thickness);
                self.arc_path(*center, *radius, *rotation_rad, *half_aperture_rad);
                ctx.stroke_path();
            }
            DrawCmd::DrawText {
                text,
                x,
                y,
                size,
                color,
            } => {
                self.draw_text(text, *x, *y, *size, *color);
            }
        }
    }

    /// Draw a line of text. `(x, y)` is the top-left of the text box (Direct2D
    /// semantics); Core Text draws from the baseline, so we offset by the font
    /// ascent and counter the global y-flip with a y-flipped text matrix.
    fn draw_text(&self, text: &str, x: f32, y: f32, size: f32, color: Color) {
        let Some((line, ascent)) = build_line(text, size, color, &self.text_family) else {
            return;
        };
        let ctx = &self.ctx;
        ctx.save();
        ctx.set_text_matrix(&CGAffineTransform::new(1.0, 0.0, 0.0, -1.0, 0.0, 0.0));
        ctx.set_text_position(x as f64, y as f64 + ascent as f64);
        line.draw(ctx);
        ctx.restore();
    }

    /// Measure a single line of text in the canvas's text family.
    pub fn measure_text(&self, text: &str, size: f32) -> Option<TextMetrics> {
        let (line, _) = build_line(text, size, WHITE, &self.text_family)?;
        let tb = line.get_typographic_bounds();
        Some(TextMetrics {
            width: tb.width as f32,
            height: (tb.ascent + tb.descent + tb.leading) as f32,
            ascent: tb.ascent as f32,
        })
    }

    /// Append a stroked circular-arc path approximated by line segments.
    fn arc_path(&self, center: Point, radius: f32, rotation_rad: f32, half_aperture_rad: f32) {
        let ctx = &self.ctx;
        const SEGMENTS: usize = 48;
        let start = rotation_rad - half_aperture_rad;
        let total = 2.0 * half_aperture_rad;
        ctx.begin_path();
        for i in 0..=SEGMENTS {
            let t = start + total * (i as f32 / SEGMENTS as f32);
            let px = (center.x + radius * t.cos()) as f64;
            let py = (center.y + radius * t.sin()) as f64;
            if i == 0 {
                ctx.move_to_point(px, py);
            } else {
                ctx.add_line_to_point(px, py);
            }
        }
    }

    /// Snapshot the bitmap as a Core Graphics image for blitting into an
    /// `NSImageView` (see `window.rs`). Kept free of AppKit/objc2 types so this
    /// file stays always-compiled.
    pub fn cg_image(&self) -> Option<core_graphics::image::CGImage> {
        self.ctx.create_image()
    }

    /// Read back the RGBA bytes of pixel `(x, y)`. Panics out of bounds.
    pub fn pixel(&mut self, x: usize, y: usize) -> [u8; 4] {
        let stride = self.ctx.bytes_per_row();
        let data = self.ctx.data();
        let o = y * stride + x * 4;
        [data[o], data[o + 1], data[o + 2], data[o + 3]]
    }

    /// Dump the canvas as a binary PPM (P6, RGB) for eyeballing. No image-crate
    /// dependency; alpha is dropped.
    pub fn to_ppm(&mut self) -> Vec<u8> {
        let (w, h) = (self.pix_w(), self.pix_h());
        let stride = self.ctx.bytes_per_row();
        let data = self.ctx.data();
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        out.reserve(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let o = y * stride + x * 4;
                out.push(data[o]);
                out.push(data[o + 1]);
                out.push(data[o + 2]);
            }
        }
        out
    }
}

const WHITE: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    const BLUE: Color = Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn clear_fills_whole_canvas() {
        let mut c = CgCanvas::new(32, 32);
        c.execute(&[DrawCmd::Clear(BLUE)]);
        for &(x, y) in &[(0, 0), (31, 0), (0, 31), (31, 31), (16, 16)] {
            assert_eq!(c.pixel(x, y), [0, 0, 255, 255], "pixel ({x},{y}) not blue");
        }
    }

    #[test]
    fn fill_rect_lands_in_the_right_place() {
        let mut c = CgCanvas::new(64, 64);
        c.execute(&[
            DrawCmd::Clear(BLUE),
            DrawCmd::FillRect { rect: rect(16.0, 16.0, 48.0, 48.0), color: RED },
        ]);
        assert_eq!(c.pixel(32, 32), [255, 0, 0, 255], "center should be red");
        assert_eq!(c.pixel(20, 20), [255, 0, 0, 255], "inside top-left → red");
        assert_eq!(c.pixel(4, 4), [0, 0, 255, 255], "corner should stay blue");
    }

    #[test]
    fn top_left_origin_is_respected() {
        let mut c = CgCanvas::new(32, 32);
        c.execute(&[
            DrawCmd::Clear(BLUE),
            DrawCmd::FillRect { rect: rect(0.0, 0.0, 32.0, 8.0), color: RED },
        ]);
        assert_eq!(c.pixel(16, 1), [255, 0, 0, 255], "row 1 (near top) red");
        assert_eq!(c.pixel(16, 30), [0, 0, 255, 255], "row 30 (near bottom) blue");
    }

    #[test]
    fn fill_circle_hits_center_misses_corner() {
        let mut c = CgCanvas::new(64, 64);
        c.execute(&[
            DrawCmd::Clear(BLUE),
            DrawCmd::FillCircle { center: Point { x: 32.0, y: 32.0 }, radius: 16.0, color: RED },
        ]);
        assert_eq!(c.pixel(32, 32), [255, 0, 0, 255], "circle center red");
        assert_eq!(c.pixel(2, 2), [0, 0, 255, 255], "circle leaves corner blue");
    }

    #[test]
    fn hidpi_renders_at_2x() {
        let mut c = CgCanvas::new_scaled(16, 16, 2.0);
        assert_eq!((c.width(), c.height()), (16, 16));
        c.execute(&[
            DrawCmd::Clear(BLUE),
            DrawCmd::FillRect { rect: rect(0.0, 0.0, 8.0, 8.0), color: RED },
        ]);
        assert_eq!(c.pixel(8, 8), [255, 0, 0, 255]);
        assert_eq!(c.pixel(24, 24), [0, 0, 255, 255]);
    }

    #[test]
    fn text_measures_positive() {
        let c = CgCanvas::new(160, 48);
        let m = c.measure_text("Hello", 24.0).expect("metrics");
        assert!(m.width > 10.0, "width too small: {}", m.width);
        assert!(m.ascent > 0.0, "ascent should be positive");
        assert!(m.height > m.ascent, "height should exceed ascent");
    }

    #[test]
    fn text_draws_ink_in_the_box_only() {
        let mut c = CgCanvas::new(160, 48);
        c.execute(&[
            DrawCmd::Clear(WHITE),
            DrawCmd::DrawText {
                text: "Locus".into(),
                x: 8.0,
                y: 8.0,
                size: 24.0,
                color: Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
            },
        ]);
        let mut ink = 0;
        for y in 4..44 {
            for x in 4..130 {
                let p = c.pixel(x, y);
                if (p[0] as u16 + p[1] as u16 + p[2] as u16) < 600 {
                    ink += 1;
                }
            }
        }
        assert!(ink > 30, "expected glyph ink, only {ink} dark px");
        assert_eq!(c.pixel(158, 46), [255, 255, 255, 255], "corner stays white");
    }

    #[test]
    fn executes_a_validated_batch() {
        let batch = DrawBatch {
            frame_id: 1,
            commands: vec![
                DrawCmd::Clear(BLUE),
                DrawCmd::FillRect { rect: rect(8.0, 8.0, 24.0, 24.0), color: RED },
            ],
        };
        batch.validate().expect("batch should be valid");
        let mut c = CgCanvas::new(32, 32);
        c.execute_batch(&batch);
        assert_eq!(c.pixel(16, 16), [255, 0, 0, 255]);
    }
}

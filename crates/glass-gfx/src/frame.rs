use crate::color::{mul255, Color};
use crate::{GLASS_HEIGHT, GLASS_WIDTH};
use tiny_skia::{
    FillRule, FilterQuality, Paint, PathBuilder, Pattern, Pixmap, PixmapPaint, SpreadMode,
    Transform,
};

/// A rectangle in pixels (floating point, so shapes can be anti-aliased).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    fn to_skia(self) -> Option<tiny_skia::Rect> {
        tiny_skia::Rect::from_xywh(self.x, self.y, self.w, self.h)
    }
}

/// An image file could not be read or written.
#[derive(Debug)]
pub struct ImageError(pub String);

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "image error: {}", self.0)
    }
}

impl std::error::Error for ImageError {}

/// An RGBA8888 pixel buffer.
///
/// Pixels are stored row by row, 4 bytes each, with **premultiplied** alpha (the format
/// `tiny-skia` draws in). For opaque pixels, which is all of a finished display frame,
/// premultiplied and straight alpha are the same bytes. [`Frame::pixel`] returns straight
/// alpha.
#[derive(Clone)]
pub struct Frame {
    pixmap: Pixmap,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{})", self.width(), self.height())
    }
}

impl Frame {
    /// A transparent frame of any size. Panics if a side is zero or the size overflows.
    pub fn new(width: u32, height: u32) -> Frame {
        Frame {
            pixmap: Pixmap::new(width, height).expect("frame size must be non-zero"),
        }
    }

    /// An opaque black frame of the Glass display size (640×360).
    pub fn glass() -> Frame {
        let mut f = Frame::new(GLASS_WIDTH, GLASS_HEIGHT);
        f.fill(Color::BLACK);
        f
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// The raw premultiplied RGBA bytes.
    pub fn data(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// The raw premultiplied RGBA bytes, mutable.
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.pixmap.data_mut()
    }

    /// The colour at `(x, y)`, straight alpha. Panics outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Color {
        assert!(x < self.width() && y < self.height(), "pixel out of bounds");
        let i = ((y * self.width() + x) * 4) as usize;
        let d = &self.data()[i..i + 4];
        Color::from_premultiplied([d[0], d[1], d[2], d[3]])
    }

    /// Sets every pixel to `color`.
    pub fn fill(&mut self, color: Color) {
        let p = color.premultiplied();
        for px in self.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&p);
        }
    }

    fn paint(color: Color) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color(color.to_skia());
        paint.anti_alias = true;
        paint
    }

    /// Fills a rectangle (source-over).
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(r) = rect.to_skia() {
            self.pixmap
                .fill_rect(r, &Self::paint(color), Transform::identity(), None);
        }
    }

    /// Fills a rectangle with rounded corners of `radius` pixels, anti-aliased.
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        let r = radius.max(0.0).min(rect.w / 2.0).min(rect.h / 2.0);
        if r <= 0.0 {
            return self.fill_rect(rect, color);
        }
        // Cubic Bézier approximation of a quarter circle.
        const K: f32 = 0.552_284_8;
        let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
        let k = r * K;
        let mut pb = PathBuilder::new();
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        pb.close();
        if let Some(path) = pb.finish() {
            self.pixmap.fill_path(
                &path,
                &Self::paint(color),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    /// Composites `src` at integer offset `(x, y)` with `opacity` in `0.0..=1.0`
    /// (source-over, honouring the source's own alpha). Parts outside this frame are clipped.
    pub fn blit(&mut self, src: &Frame, x: i32, y: i32, opacity: f32) {
        let o = (opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
        if o == 0 {
            return;
        }
        let Some(span) = Span::clip(self, src, x, y) else {
            return;
        };
        let (dw, sw) = (self.width() as usize, src.width() as usize);
        let sdata = src.data();
        let ddata = self.data_mut();
        for row in 0..span.h {
            let s0 = ((span.sy + row) * sw + span.sx) * 4;
            let d0 = ((span.dy + row) * dw + span.dx) * 4;
            let s = &sdata[s0..s0 + span.w * 4];
            let d = &mut ddata[d0..d0 + span.w * 4];
            for (sp, dp) in s.chunks_exact(4).zip(d.chunks_exact_mut(4)) {
                let sa = if o == 255 {
                    sp[3] as u32
                } else {
                    mul255(sp[3] as u32, o)
                };
                if sa == 0 {
                    continue;
                }
                if sa == 255 {
                    dp.copy_from_slice(sp);
                    continue;
                }
                let inv = 255 - sa;
                for c in 0..3 {
                    let sc = if o == 255 {
                        sp[c] as u32
                    } else {
                        mul255(sp[c] as u32, o)
                    };
                    dp[c] = (sc + mul255(dp[c] as u32, inv)).min(255) as u8;
                }
                dp[3] = (sa + mul255(dp[3] as u32, inv)).min(255) as u8;
            }
        }
    }

    /// Copies `src` at `(x, y)` replacing the destination pixels, alpha included.
    /// The fast path for opaque layers (a row `memcpy`); parts outside are clipped.
    pub fn copy_from(&mut self, src: &Frame, x: i32, y: i32) {
        let Some(span) = Span::clip(self, src, x, y) else {
            return;
        };
        let (dw, sw) = (self.width() as usize, src.width() as usize);
        let sdata = src.data();
        let ddata = self.data_mut();
        for row in 0..span.h {
            let s0 = ((span.sy + row) * sw + span.sx) * 4;
            let d0 = ((span.dy + row) * dw + span.dx) * 4;
            ddata[d0..d0 + span.w * 4].copy_from_slice(&sdata[s0..s0 + span.w * 4]);
        }
    }

    /// Draws `image` scaled to fill `dest` (bilinear filtering, source-over).
    pub fn draw_image(&mut self, image: &Frame, dest: Rect) {
        let Some(r) = dest.to_skia() else {
            return;
        };
        let sx = dest.w / image.width() as f32;
        let sy = dest.h / image.height() as f32;
        let transform = Transform::from_row(sx, 0.0, 0.0, sy, dest.x, dest.y);
        let paint = Paint {
            shader: Pattern::new(
                image.pixmap.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Bilinear,
                1.0,
                transform,
            ),
            anti_alias: true,
            ..Paint::default()
        };
        self.pixmap
            .fill_rect(r, &paint, Transform::identity(), None);
    }

    /// Draws `image` at integer `(x, y)` without scaling.
    pub fn draw_image_at(&mut self, image: &Frame, x: i32, y: i32) {
        self.pixmap.draw_pixmap(
            x,
            y,
            image.pixmap.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    /// Decodes a PNG file (any colour type `png` supports, converted to RGBA).
    pub fn from_png(bytes: &[u8]) -> Result<Frame, ImageError> {
        Pixmap::decode_png(bytes)
            .map(|pixmap| Frame { pixmap })
            .map_err(|e| ImageError(e.to_string()))
    }

    /// Encodes the frame as an RGBA PNG (straight alpha).
    pub fn to_png(&self) -> Result<Vec<u8>, ImageError> {
        self.pixmap
            .encode_png()
            .map_err(|e| ImageError(e.to_string()))
    }

    /// Writes the frame as a PNG file.
    pub fn write_png(&self, path: impl AsRef<std::path::Path>) -> Result<(), ImageError> {
        let bytes = self.to_png()?;
        std::fs::write(path, bytes).map_err(|e| ImageError(e.to_string()))
    }

    /// Draws the glyphs of `run` in `color` (rasterized with fontdue, source-over).
    pub fn draw_glyph_run(&mut self, run: &crate::text::GlyphRun<'_>, color: Color) {
        for g in &run.glyphs {
            let (m, bitmap) = run.font.rasterize(g.ch, run.size);
            if m.width == 0 || m.height == 0 {
                continue;
            }
            let gx = g.x + m.xmin;
            let gy = g.baseline - (m.ymin + m.height as i32);
            self.blend_coverage(&bitmap, m.width, m.height, gx, gy, color);
        }
    }

    /// Blends coverage values (`0..=255`, one per pixel, `w` per row) of `color` at
    /// `(x, y)`; used by the text renderer. Parts outside the frame are clipped.
    pub fn blend_coverage(
        &mut self,
        coverage: &[u8],
        w: usize,
        h: usize,
        x: i32,
        y: i32,
        color: Color,
    ) {
        let fw = self.width() as i64;
        let fh = self.height() as i64;
        let pc = color.premultiplied();
        let width = self.width() as usize;
        let data = self.data_mut();
        for row in 0..h {
            let py = y as i64 + row as i64;
            if py < 0 || py >= fh {
                continue;
            }
            for col in 0..w {
                let px = x as i64 + col as i64;
                if px < 0 || px >= fw {
                    continue;
                }
                let cov = coverage[row * w + col] as u32;
                if cov == 0 {
                    continue;
                }
                let i = (py as usize * width + px as usize) * 4;
                let d = &mut data[i..i + 4];
                let sa = mul255(pc[3] as u32, cov);
                let inv = 255 - sa;
                for c in 0..3 {
                    let sc = mul255(pc[c] as u32, cov);
                    d[c] = (sc + mul255(d[c] as u32, inv)).min(255) as u8;
                }
                d[3] = (sa + mul255(d[3] as u32, inv)).min(255) as u8;
            }
        }
    }
}

/// The overlap of a source placed at an offset in a destination.
struct Span {
    sx: usize,
    sy: usize,
    dx: usize,
    dy: usize,
    w: usize,
    h: usize,
}

impl Span {
    fn clip(dst: &Frame, src: &Frame, x: i32, y: i32) -> Option<Span> {
        let (x, y) = (x as i64, y as i64);
        let dx0 = x.max(0);
        let dy0 = y.max(0);
        let dx1 = (x + src.width() as i64).min(dst.width() as i64);
        let dy1 = (y + src.height() as i64).min(dst.height() as i64);
        if dx1 <= dx0 || dy1 <= dy0 {
            return None;
        }
        Some(Span {
            sx: (dx0 - x) as usize,
            sy: (dy0 - y) as usize,
            dx: dx0 as usize,
            dy: dy0 as usize,
            w: (dx1 - dx0) as usize,
            h: (dy1 - dy0) as usize,
        })
    }
}

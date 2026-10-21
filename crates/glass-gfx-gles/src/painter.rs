//! The GLES2 painter and the resources it shares with the compositor: programs, the vertex
//! buffer and the glyph atlas.

use crate::downscale::{downscale, texture_size};
use crate::sgx::Gl;
use crate::shaders;
use glass_gfx::text::GlyphRun;
use glass_gfx::{Color, Frame, Painter, Rect};
use glow::HasContext;
use std::collections::HashMap;

/// Side of the glyph atlas (at most; less if `GL_MAX_TEXTURE_SIZE` is smaller).
pub const ATLAS_SIZE: u32 = 1024;

const A_POS: u32 = 0;
const A_AUX: u32 = 1;

/// A linked program and its uniforms.
pub(crate) struct Program {
    pub program: glow::NativeProgram,
    scale: Option<glow::NativeUniformLocation>,
    bias: Option<glow::NativeUniformLocation>,
    color: Option<glow::NativeUniformLocation>,
    radius: Option<glow::NativeUniformLocation>,
    opacity: Option<glow::NativeUniformLocation>,
    tex: Option<glow::NativeUniformLocation>,
}

impl Program {
    fn new(gl: &Gl, fragment: &str) -> Result<Program, String> {
        let vs = gl.compile_shader(glow::VERTEX_SHADER, shaders::VERTEX)?;
        let fs = gl.compile_shader(glow::FRAGMENT_SHADER, fragment)?;
        let raw = gl.raw();
        unsafe {
            let program = raw.create_program()?;
            raw.attach_shader(program, vs);
            raw.attach_shader(program, fs);
            raw.bind_attrib_location(program, A_POS, "a_pos");
            raw.bind_attrib_location(program, A_AUX, "a_aux");
            raw.link_program(program);
            raw.delete_shader(vs);
            raw.delete_shader(fs);
            if !raw.get_program_link_status(program) {
                let log = raw.get_program_info_log(program);
                raw.delete_program(program);
                return Err(format!("program link failed: {log}"));
            }
            let u = |n: &str| raw.get_uniform_location(program, n);
            Ok(Program {
                program,
                scale: u("u_scale"),
                bias: u("u_bias"),
                color: u("u_color"),
                radius: u("u_radius"),
                opacity: u("u_opacity"),
                tex: u("u_tex"),
            })
        }
    }
}

/// Shelf packing with 1 px of padding between entries.
#[derive(Debug)]
pub struct ShelfPacker {
    size: u32,
    x: u32,
    y: u32,
    shelf_h: u32,
}

impl ShelfPacker {
    pub fn new(size: u32) -> ShelfPacker {
        ShelfPacker {
            size,
            x: 0,
            y: 0,
            shelf_h: 0,
        }
    }

    /// The top-left corner for a `w`×`h` entry, or `None` when full.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            return None;
        }
        if self.x + w > self.size {
            self.y += self.shelf_h + 1;
            self.x = 0;
            self.shelf_h = 0;
        }
        if self.y + h > self.size {
            return None;
        }
        let at = (self.x, self.y);
        self.x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        Some(at)
    }

    pub fn clear(&mut self) {
        *self = ShelfPacker::new(self.size);
    }
}

#[derive(Clone, Copy, Debug)]
struct AtlasGlyph {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    xmin: i32,
    ymin: i32,
}

type GlyphKey = (usize, char, u32);

/// The glyph atlas: an ALPHA texture filled with fontdue bitmaps on first use, cleared and
/// refilled when full.
pub(crate) struct Atlas {
    tex: glow::NativeTexture,
    size: u32,
    packer: ShelfPacker,
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
    clears: u64,
}

/// Programs, buffers and the atlas; one set per context.
pub(crate) struct Resources {
    pub gl: Gl,
    pub shape: Program,
    pub texture: Program,
    pub glyph: Program,
    vbo: glow::NativeBuffer,
    pub atlas: Atlas,
    pub max_texture_size: u32,
    verts: Vec<f32>,
}

/// Sets NEAREST or LINEAR filtering and CLAMP_TO_EDGE on the bound texture.
pub(crate) fn texture_params(gl: &Gl, filter: u32) {
    gl.tex_parameter(glow::TEXTURE_MIN_FILTER, filter);
    gl.tex_parameter(glow::TEXTURE_MAG_FILTER, filter);
    gl.tex_parameter(glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE);
    gl.tex_parameter(glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE);
}

fn as_bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and any byte pattern of it is valid to read as u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

fn premul(c: Color) -> [f32; 4] {
    let p = c.premultiplied();
    p.map(|v| v as f32 / 255.0)
}

impl Resources {
    pub fn new(gl: Gl, max_texture_size: u32) -> Result<Resources, String> {
        let shape = Program::new(&gl, shaders::SHAPE_FRAGMENT)?;
        let texture = Program::new(&gl, shaders::TEXTURE_FRAGMENT)?;
        let glyph = Program::new(&gl, shaders::GLYPH_FRAGMENT)?;
        let vbo = unsafe { gl.raw().create_buffer()? };
        let size = ATLAS_SIZE.min(max_texture_size);
        let tex = gl.create_texture()?;
        gl.active_texture(0);
        gl.bind_texture(Some(tex));
        texture_params(&gl, glow::NEAREST);
        let zeros = vec![0u8; (size * size) as usize];
        unsafe { gl.raw().pixel_store_i32(glow::UNPACK_ALIGNMENT, 1) };
        gl.tex_image_2d(
            glow::ALPHA,
            size,
            size,
            glow::ALPHA,
            glow::UNSIGNED_BYTE,
            Some(&zeros),
        );
        gl.bind_texture(None);
        Ok(Resources {
            gl,
            shape,
            texture,
            glyph,
            vbo,
            atlas: Atlas {
                tex,
                size,
                packer: ShelfPacker::new(size),
                glyphs: HashMap::new(),
                clears: 0,
            },
            max_texture_size,
            verts: Vec::new(),
        })
    }

    /// Frees the GL objects (the context must be current).
    pub fn delete(&mut self) {
        let raw = self.gl.raw();
        unsafe {
            raw.delete_program(self.shape.program);
            raw.delete_program(self.texture.program);
            raw.delete_program(self.glyph.program);
            raw.delete_buffer(self.vbo);
        }
        self.gl.delete_texture(self.atlas.tex);
    }

    /// How many times the atlas was full and was cleared.
    pub fn atlas_clears(&self) -> u64 {
        self.atlas.clears
    }

    /// Uses `p` for a target of `w`×`h` pixels (pixel (0,0) is the first row in memory,
    /// which holds for FBOs; a window surface, M3, needs the y axis flipped).
    pub(crate) fn use_program(&self, which: Which, w: u32, h: u32) {
        let p = self.program(which);
        let raw = self.gl.raw();
        unsafe {
            raw.use_program(Some(p.program));
            raw.uniform_2_f32(p.scale.as_ref(), 2.0 / w as f32, 2.0 / h as f32);
            raw.uniform_2_f32(p.bias.as_ref(), -1.0, -1.0);
            if let Some(t) = &p.tex {
                raw.uniform_1_i32(Some(t), 0);
            }
        }
    }

    fn program(&self, which: Which) -> &Program {
        match which {
            Which::Shape => &self.shape,
            Which::Texture => &self.texture,
            Which::Glyph => &self.glyph,
        }
    }

    pub(crate) fn set_color(&self, which: Which, c: [f32; 4]) {
        unsafe {
            self.gl
                .raw()
                .uniform_4_f32(self.program(which).color.as_ref(), c[0], c[1], c[2], c[3])
        };
    }

    pub(crate) fn set_opacity(&self, opacity: f32) {
        unsafe {
            self.gl
                .raw()
                .uniform_1_f32(self.texture.opacity.as_ref(), opacity)
        };
    }

    fn set_radius(&self, r: f32) {
        unsafe { self.gl.raw().uniform_1_f32(self.shape.radius.as_ref(), r) };
    }

    /// Appends a quad: pixel rectangle `(x0, y0)-(x1, y1)` and aux coordinates at its
    /// corners.
    fn push_quad(&mut self, p: [f32; 4], a: [f32; 4]) {
        push_quad(&mut self.verts, p, a);
    }

    /// Draws the queued quads with the current program and textures.
    pub(crate) fn flush(&mut self) {
        if self.verts.is_empty() {
            return;
        }
        let raw = self.gl.raw();
        unsafe {
            raw.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            raw.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&self.verts), glow::STREAM_DRAW);
            raw.enable_vertex_attrib_array(A_POS);
            raw.enable_vertex_attrib_array(A_AUX);
            raw.vertex_attrib_pointer_f32(A_POS, 2, glow::FLOAT, false, 16, 0);
            raw.vertex_attrib_pointer_f32(A_AUX, 2, glow::FLOAT, false, 16, 8);
        }
        self.gl.draw_triangles((self.verts.len() / 4) as i32);
        self.verts.clear();
    }

    /// Draws `tex` at pixel rectangle `dest` (texture coordinates 0..1).
    pub(crate) fn textured_quad(&mut self, dest: [f32; 4]) {
        self.push_quad(dest, [0.0, 0.0, 1.0, 1.0]);
        self.flush();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Which {
    Shape,
    Texture,
    Glyph,
}

/// Two triangles: positions `p = [x0, y0, x1, y1]`, aux `a = [u0, v0, u1, v1]`.
fn push_quad(v: &mut Vec<f32>, p: [f32; 4], a: [f32; 4]) {
    let [x0, y0, x1, y1] = p;
    let [u0, v0, u1, v1] = a;
    v.extend_from_slice(&[
        x0, y0, u0, v0, x1, y0, u1, v0, x0, y1, u0, v1, //
        x1, y0, u1, v0, x1, y1, u1, v1, x0, y1, u0, v1,
    ]);
}

/// The four quadrant quads of a rounded rectangle, 1 px larger than the rectangle on every
/// side (the anti-aliased fringe). Aux coordinates are the outward offset from each
/// quadrant's corner circle centre. Returns the clamped radius too.
pub fn shape_quads(rect: Rect, radius: f32) -> (Vec<f32>, f32) {
    let r = radius.max(0.0).min(rect.w / 2.0).min(rect.h / 2.0);
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let mut v = Vec::with_capacity(4 * 24);
    // (quad x range, quad y range, corner centre, outward direction)
    let quads = [
        (
            [x0 - 1.0, mx],
            [y0 - 1.0, my],
            (x0 + r, y0 + r),
            (-1.0, -1.0),
        ),
        (
            [mx, x1 + 1.0],
            [y0 - 1.0, my],
            (x1 - r, y0 + r),
            (1.0, -1.0),
        ),
        (
            [x0 - 1.0, mx],
            [my, y1 + 1.0],
            (x0 + r, y1 - r),
            (-1.0, 1.0),
        ),
        ([mx, x1 + 1.0], [my, y1 + 1.0], (x1 - r, y1 - r), (1.0, 1.0)),
    ];
    for (xs, ys, (cx, cy), (dx, dy)) in quads {
        let local = |x: f32, y: f32| ((x - cx) * dx, (y - cy) * dy);
        let (a0, b0) = local(xs[0], ys[0]);
        let (a1, b1) = local(xs[1], ys[1]);
        push_quad(&mut v, [xs[0], ys[0], xs[1], ys[1]], [a0, b0, a1, b1]);
    }
    (v, r)
}

/// Draws into the bound framebuffer (a layer) of `width`×`height` pixels.
pub struct GlesPainter<'a> {
    pub(crate) res: &'a mut Resources,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl GlesPainter<'_> {
    fn glyph(&mut self, run: &GlyphRun<'_>, ch: char) -> Result<Option<AtlasGlyph>, ()> {
        let key = (run.font.file_hash(), ch, run.size.to_bits());
        if let Some(g) = self.res.atlas.glyphs.get(&key) {
            return Ok(*g);
        }
        let (m, bitmap) = run.font.rasterize(ch, run.size);
        if m.width == 0 || m.height == 0 {
            self.res.atlas.glyphs.insert(key, None);
            return Ok(None);
        }
        let (w, h) = (m.width as u32, m.height as u32);
        if w > self.res.atlas.size || h > self.res.atlas.size {
            // Larger than the whole atlas: not drawn, and no reason to clear the atlas.
            self.res.atlas.glyphs.insert(key, None);
            return Ok(None);
        }
        let (x, y) = self.res.atlas.packer.alloc(w, h).ok_or(())?;
        let gl = &self.res.gl;
        gl.active_texture(0);
        gl.bind_texture(Some(self.res.atlas.tex));
        unsafe {
            let raw = gl.raw();
            raw.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            raw.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                x as i32,
                y as i32,
                w as i32,
                h as i32,
                glow::ALPHA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&bitmap)),
            );
        }
        let g = AtlasGlyph {
            x,
            y,
            w,
            h,
            xmin: m.xmin,
            ymin: m.ymin,
        };
        self.res.atlas.glyphs.insert(key, Some(g));
        Ok(Some(g))
    }

    fn begin_glyphs(&mut self, color: Color) {
        self.res.use_program(Which::Glyph, self.width, self.height);
        self.res.set_color(Which::Glyph, premul(color));
        self.res.gl.active_texture(0);
        self.res.gl.bind_texture(Some(self.res.atlas.tex));
    }
}

impl Painter for GlesPainter<'_> {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn clear(&mut self, color: Color) {
        let c = premul(color);
        unsafe {
            let raw = self.res.gl.raw();
            raw.clear_color(c[0], c[1], c[2], c[3]);
            raw.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.fill_rounded_rect(rect, 0.0, color);
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        if rect.w <= 0.0 || rect.h <= 0.0 || color.a == 0 {
            return;
        }
        let (v, r) = shape_quads(rect, radius);
        self.res.use_program(Which::Shape, self.width, self.height);
        self.res.set_color(Which::Shape, premul(color));
        self.res.set_radius(r);
        self.res.verts.extend_from_slice(&v);
        self.res.flush();
    }

    fn draw_image(&mut self, image: &Frame, dest: Rect) {
        if dest.w <= 0.0 || dest.h <= 0.0 {
            return;
        }
        let (tw, th) = texture_size(
            image.width(),
            image.height(),
            dest.w,
            dest.h,
            self.res.max_texture_size,
        );
        let scaled;
        let src = if (tw, th) == (image.width(), image.height()) {
            image
        } else {
            scaled = downscale(image, tw, th);
            &scaled
        };
        let gl = &self.res.gl;
        let Ok(tex) = gl.create_texture() else {
            return;
        };
        gl.active_texture(0);
        gl.bind_texture(Some(tex));
        texture_params(gl, glow::LINEAR);
        unsafe { gl.raw().pixel_store_i32(glow::UNPACK_ALIGNMENT, 1) };
        gl.tex_image_2d(
            glow::RGBA,
            tw,
            th,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            Some(src.data()),
        );
        self.res
            .use_program(Which::Texture, self.width, self.height);
        self.res.set_opacity(1.0);
        self.res
            .textured_quad([dest.x, dest.y, dest.right(), dest.bottom()]);
        self.res.gl.bind_texture(None);
        self.res.gl.delete_texture(tex);
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>, color: Color) {
        if color.a == 0 || run.glyphs.is_empty() {
            return;
        }
        self.begin_glyphs(color);
        let inv = 1.0 / self.res.atlas.size as f32;
        for pg in &run.glyphs {
            let g = match self.glyph(run, pg.ch) {
                Ok(g) => g,
                Err(()) => {
                    // Full: draw what is queued, clear, refill for the current cards.
                    self.res.flush();
                    self.res.atlas.packer.clear();
                    self.res.atlas.glyphs.clear();
                    self.res.atlas.clears += 1;
                    self.begin_glyphs(color);
                    // Fits in an empty atlas (larger glyphs were turned away above).
                    self.glyph(run, pg.ch).unwrap_or(None)
                }
            };
            let Some(g) = g else {
                continue;
            };
            // `glyph` may have rebound the atlas; it is the only texture used here.
            let x = (pg.x + g.xmin) as f32;
            let y = (pg.baseline - (g.ymin + g.h as i32)) as f32;
            push_quad(
                &mut self.res.verts,
                [x, y, x + g.w as f32, y + g.h as f32],
                [
                    g.x as f32 * inv,
                    g.y as f32 * inv,
                    (g.x + g.w) as f32 * inv,
                    (g.y + g.h) as f32 * inv,
                ],
            );
        }
        self.res.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelf_packer_pads_and_wraps_and_fills() {
        let mut p = ShelfPacker::new(10);
        assert_eq!(p.alloc(4, 3), Some((0, 0)));
        assert_eq!(p.alloc(4, 2), Some((5, 0)));
        // 10 - 10 = no room on this shelf: next shelf below the tallest + 1.
        assert_eq!(p.alloc(2, 2), Some((0, 4)));
        assert_eq!(p.alloc(11, 1), None, "wider than the atlas");
        assert_eq!(p.alloc(9, 9), None, "full");
        p.clear();
        assert_eq!(p.alloc(9, 9), Some((0, 0)));
    }

    /// The shape shader's coverage, evaluated on the CPU at a pixel centre.
    fn coverage(v: &[f32], r: f32, px: f32, py: f32) -> f32 {
        // Find the quadrant quad containing the point and interpolate its aux coordinates.
        for q in v.chunks_exact(24) {
            let (x0, y0, u0, v0) = (q[0], q[1], q[2], q[3]);
            let (x1, y1, u1, v1) = (q[16], q[17], q[18], q[19]);
            if px >= x0 && px < x1 && py >= y0 && py < y1 {
                let a = u0 + (u1 - u0) * (px - x0) / (x1 - x0);
                let b = v0 + (v1 - v0) * (py - y0) / (y1 - y0);
                let d = (a.max(0.0).hypot(b.max(0.0))) + a.max(b).min(0.0) - r;
                return (0.5 - d).clamp(0.0, 1.0);
            }
        }
        0.0
    }

    #[test]
    fn shape_quads_cover_whole_pixel_rects_exactly() {
        let (v, r) = shape_quads(Rect::new(2.0, 3.0, 5.0, 4.0), 0.0);
        for y in 0..10 {
            for x in 0..10 {
                let inside = (2..7).contains(&x) && (3..7).contains(&y);
                let c = coverage(&v, r, x as f32 + 0.5, y as f32 + 0.5);
                assert_eq!(c, if inside { 1.0 } else { 0.0 }, "{x},{y}");
            }
        }
    }

    #[test]
    fn rounded_corners_are_cut_and_edges_are_solid() {
        let (v, r) = shape_quads(Rect::new(0.0, 0.0, 100.0, 60.0), 10.0);
        assert_eq!(r, 10.0);
        assert_eq!(
            coverage(&v, r, 0.5, 0.5),
            0.0,
            "corner pixel outside the arc"
        );
        assert_eq!(coverage(&v, r, 50.5, 0.5), 1.0, "top edge");
        assert_eq!(coverage(&v, r, 99.5, 30.5), 1.0, "right edge");
        assert_eq!(coverage(&v, r, 50.5, 30.5), 1.0, "centre");
        let diag = coverage(&v, r, 2.5, 3.5);
        assert!(diag > 0.0 && diag < 1.0, "anti-aliased arc: {diag}");
        // Radius larger than half the height is clamped like the software path.
        assert_eq!(shape_quads(Rect::new(0.0, 0.0, 100.0, 60.0), 50.0).1, 30.0);
    }
}

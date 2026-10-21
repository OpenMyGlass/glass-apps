//! The GLES2 compositor: card layers are textures attached to FBOs; a frame is a full clear
//! then one textured quad per visible layer, drawn into an off-screen target (host tests
//! and development) and read back for the [`glass_gfx::Display`]. The window-surface
//! target (an `ANativeWindow`, `eglSwapBuffers`, no read-back) comes with the Android
//! wrapper (M3).

use crate::egl::{EglContext, EglInfo};
use crate::painter::{texture_params, GlesPainter, Resources, Which};
use crate::sgx::Gl;
use glass_gfx::{Color, Compositor, Display, Frame, GfxError, LayerId, Painter, Placement};
use glow::HasContext;
use std::collections::HashMap;

/// Renderer options.
#[derive(Clone, Copy, Debug)]
pub struct GlesOptions {
    /// The SGX540 assertion layer (see [`crate::sgx`]). On in debug builds and tests.
    pub sgx540_checks: bool,
    /// Store opaque layers as RGB565 as if RGBA FBOs were incomplete (tests the fallback).
    pub force_rgb565_layers: bool,
}

impl Default for GlesOptions {
    fn default() -> GlesOptions {
        GlesOptions {
            sgx540_checks: cfg!(debug_assertions),
            force_rgb565_layers: false,
        }
    }
}

/// The colour format of a layer texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerFormat {
    Rgba8888,
    /// The fallback for opaque layers when an RGBA FBO is incomplete.
    Rgb565,
}

/// What the GL context is, for logs.
#[derive(Clone, Debug)]
pub struct GlInfo {
    pub vendor: String,
    pub renderer: String,
    pub version: String,
    pub shading_language: String,
    pub max_texture_size: u32,
    /// Whether fragment shaders have `highp` floats.
    pub fragment_highp: bool,
    pub egl: EglInfo,
}

impl std::fmt::Display for GlInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GL_RENDERER={} GL_VERSION={} GL_VENDOR={} GLSL={} GL_MAX_TEXTURE_SIZE={} \
             fragment highp={} EGL {}.{} ({}, {})",
            self.renderer,
            self.version,
            self.vendor,
            self.shading_language,
            self.max_texture_size,
            self.fragment_highp,
            self.egl.version.0,
            self.egl.version.1,
            self.egl.vendor,
            self.egl.setup
        )
    }
}

struct Target {
    tex: glow::NativeTexture,
    fbo: glow::NativeFramebuffer,
    width: u32,
    height: u32,
    opaque: bool,
    format: LayerFormat,
}

/// The GLES2 compositor. Not `Send`: one context, one thread.
pub struct GlesCompositor {
    res: Resources,
    screen: Target,
    layers: HashMap<LayerId, Target>,
    next_id: u64,
    options: GlesOptions,
    info: GlInfo,
    // Last, so that it is dropped after the GL objects are deleted in `drop`.
    egl: EglContext,
}

fn gl_err(e: String) -> GfxError {
    GfxError::Renderer(e)
}

impl GlesCompositor {
    /// A compositor with an off-screen target of `width`×`height`, its frames read back
    /// on [`Compositor::present`].
    pub fn offscreen(
        width: u32,
        height: u32,
        options: GlesOptions,
    ) -> Result<GlesCompositor, GfxError> {
        let egl = EglContext::offscreen().map_err(gl_err)?;
        let raw = egl.load_gl();
        let (info, max) = unsafe {
            let max = raw.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(64) as u32;
            let highp = raw
                .get_shader_precision_format(glow::FRAGMENT_SHADER, glow::HIGH_FLOAT)
                .is_some_and(|p| p.precision > 0);
            (
                GlInfo {
                    vendor: raw.get_parameter_string(glow::VENDOR),
                    renderer: raw.get_parameter_string(glow::RENDERER),
                    version: raw.get_parameter_string(glow::VERSION),
                    shading_language: raw.get_parameter_string(glow::SHADING_LANGUAGE_VERSION),
                    max_texture_size: max,
                    fragment_highp: highp,
                    egl: egl.info().clone(),
                },
                max,
            )
        };
        if !info.version.starts_with("OpenGL ES") {
            return Err(GfxError::Renderer(format!(
                "not a GLES context: GL_VERSION={}",
                info.version
            )));
        }
        let gl = Gl::new(raw, options.sgx540_checks, max);
        let res = Resources::new(gl, max).map_err(gl_err)?;
        let screen = create_target(&res.gl, width, height, true, false).map_err(gl_err)?;
        let c = GlesCompositor {
            res,
            screen,
            layers: HashMap::new(),
            next_id: 1,
            options,
            info,
            egl,
        };
        unsafe {
            let raw = c.res.gl.raw();
            raw.disable(glow::DEPTH_TEST);
            raw.disable(glow::STENCIL_TEST);
            raw.disable(glow::SCISSOR_TEST);
            raw.disable(glow::DITHER);
            raw.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        }
        c.check_error("setup")?;
        Ok(c)
    }

    pub fn info(&self) -> &GlInfo {
        &self.info
    }

    /// The format a layer was created with.
    pub fn layer_format(&self, id: LayerId) -> Option<LayerFormat> {
        self.layers.get(&id).map(|l| l.format)
    }

    /// Whether the SGX540 assertion layer is on.
    pub fn checks_enabled(&self) -> bool {
        self.res.gl.checks_enabled()
    }

    /// How many times the glyph atlas was full and was cleared.
    pub fn atlas_clears(&self) -> u64 {
        self.res.atlas_clears()
    }

    /// Runs `f` with the raw GL wrapper (tests of the assertion layer).
    pub fn with_gl<R>(&mut self, f: impl FnOnce(&Gl) -> R) -> R {
        self.current().expect("make current");
        f(&self.res.gl)
    }

    fn current(&self) -> Result<(), GfxError> {
        if !self.egl.is_current() {
            self.egl.make_current().map_err(gl_err)?;
        }
        Ok(())
    }

    fn check_error(&self, what: &str) -> Result<(), GfxError> {
        let e = unsafe { self.res.gl.raw().get_error() };
        if e == glow::NO_ERROR {
            Ok(())
        } else {
            Err(GfxError::Renderer(format!("GL error 0x{e:04x} in {what}")))
        }
    }

    fn bind_target(&self, t: &Target) {
        unsafe {
            let raw = self.res.gl.raw();
            raw.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
            raw.viewport(0, 0, t.width as i32, t.height as i32);
        }
    }
}

/// A texture of `w`×`h` attached to a new FBO, cleared to transparent. RGBA first; for an
/// opaque target RGB565 if the RGBA FBO is incomplete (or `force_565`).
fn create_target(gl: &Gl, w: u32, h: u32, opaque: bool, force_565: bool) -> Result<Target, String> {
    let mut formats = vec![];
    if !(opaque && force_565) {
        formats.push(LayerFormat::Rgba8888);
    }
    if opaque {
        formats.push(LayerFormat::Rgb565);
    }
    let mut last = String::new();
    for format in formats {
        let tex = gl.create_texture()?;
        gl.active_texture(0);
        gl.bind_texture(Some(tex));
        texture_params(gl, glow::NEAREST);
        match format {
            LayerFormat::Rgba8888 => {
                gl.tex_image_2d(glow::RGBA, w, h, glow::RGBA, glow::UNSIGNED_BYTE, None)
            }
            LayerFormat::Rgb565 => {
                gl.tex_image_2d(glow::RGB, w, h, glow::RGB, glow::UNSIGNED_SHORT_5_6_5, None)
            }
        }
        gl.bind_texture(None);
        let raw = gl.raw();
        let fbo = unsafe { raw.create_framebuffer() }?;
        let status = unsafe {
            raw.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            raw.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            raw.check_framebuffer_status(glow::FRAMEBUFFER)
        };
        if status == glow::FRAMEBUFFER_COMPLETE {
            unsafe {
                raw.viewport(0, 0, w as i32, h as i32);
                raw.clear_color(0.0, 0.0, 0.0, 0.0);
                raw.clear(glow::COLOR_BUFFER_BIT);
            }
            return Ok(Target {
                tex,
                fbo,
                width: w,
                height: h,
                opaque,
                format,
            });
        }
        last = format!("{format:?} FBO {w}x{h} incomplete (status 0x{status:04x})");
        unsafe { raw.delete_framebuffer(fbo) };
        gl.delete_texture(tex);
    }
    Err(last)
}

fn delete_target(gl: &Gl, t: &Target) {
    unsafe { gl.raw().delete_framebuffer(t.fbo) };
    gl.delete_texture(t.tex);
}

impl Compositor for GlesCompositor {
    fn name(&self) -> &'static str {
        "gles"
    }

    fn size(&self) -> (u32, u32) {
        (self.screen.width, self.screen.height)
    }

    fn create_layer(&mut self, width: u32, height: u32, opaque: bool) -> Result<LayerId, GfxError> {
        self.current()?;
        if width == 0 || height == 0 {
            return Err(GfxError::Renderer(format!(
                "layer size {width}x{height} is empty"
            )));
        }
        let t = create_target(
            &self.res.gl,
            width,
            height,
            opaque,
            self.options.force_rgb565_layers,
        )
        .map_err(gl_err)?;
        if let Err(e) = self.check_error("create_layer") {
            delete_target(&self.res.gl, &t);
            return Err(e);
        }
        let id = LayerId(self.next_id);
        self.next_id += 1;
        self.layers.insert(id, t);
        Ok(id)
    }

    fn paint_layer(
        &mut self,
        id: LayerId,
        paint: &mut dyn FnMut(&mut dyn Painter),
    ) -> Result<(), GfxError> {
        self.current()?;
        let t = self.layers.get(&id).ok_or(GfxError::UnknownLayer(id))?;
        let (w, h) = (t.width, t.height);
        self.bind_target(t);
        unsafe { self.res.gl.raw().enable(glow::BLEND) };
        let mut painter = GlesPainter {
            res: &mut self.res,
            width: w,
            height: h,
        };
        paint(&mut painter);
        self.check_error("paint_layer")
    }

    fn drop_layer(&mut self, id: LayerId) {
        if let Some(t) = self.layers.remove(&id) {
            if self.current().is_ok() {
                delete_target(&self.res.gl, &t);
            }
        }
    }

    fn layer_count(&self) -> usize {
        self.layers.len()
    }

    fn compose(&mut self, background: Color, layers: &[Placement]) -> Result<(), GfxError> {
        self.current()?;
        for p in layers {
            if !self.layers.contains_key(&p.layer) {
                return Err(GfxError::UnknownLayer(p.layer));
            }
        }
        self.bind_target(&self.screen);
        let bg = background.premultiplied().map(|v| v as f32 / 255.0);
        let (sw, sh) = (self.screen.width, self.screen.height);
        unsafe {
            let raw = self.res.gl.raw();
            raw.clear_color(bg[0], bg[1], bg[2], bg[3]);
            raw.clear(glow::COLOR_BUFFER_BIT);
        }
        self.res.use_program(Which::Texture, sw, sh);
        for p in layers {
            let t = &self.layers[&p.layer];
            let opacity = p.opacity.clamp(0.0, 1.0);
            if opacity <= 0.0 {
                continue;
            }
            unsafe {
                let raw = self.res.gl.raw();
                // Opaque layers are copied, as the software path does.
                if t.opaque && opacity >= 1.0 {
                    raw.disable(glow::BLEND);
                } else {
                    raw.enable(glow::BLEND);
                }
            }
            self.res.set_opacity(opacity);
            self.res.gl.active_texture(0);
            self.res.gl.bind_texture(Some(t.tex));
            let (x, y) = (p.x as f32, p.y as f32);
            let dest = [x, y, x + t.width as f32, y + t.height as f32];
            self.res.textured_quad(dest);
        }
        self.res.gl.bind_texture(None);
        unsafe { self.res.gl.raw().enable(glow::BLEND) };
        self.check_error("compose")
    }

    fn present(&mut self, display: &mut dyn Display) -> Result<(), GfxError> {
        let frame = self.read_frame()?;
        display.present(&frame)?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Frame, GfxError> {
        self.current()?;
        let (w, h) = (self.screen.width, self.screen.height);
        let mut frame = Frame::new(w, h);
        self.bind_target(&self.screen);
        unsafe {
            let raw = self.res.gl.raw();
            raw.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
            raw.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(frame.data_mut())),
            );
        }
        self.check_error("read_frame")?;
        Ok(frame)
    }
}

impl Drop for GlesCompositor {
    fn drop(&mut self) {
        if self.current().is_err() {
            return;
        }
        for (_, t) in self.layers.drain() {
            delete_target(&self.res.gl, &t);
        }
        delete_target(&self.res.gl, &self.screen);
        self.res.delete();
    }
}

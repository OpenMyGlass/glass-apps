//! The SGX540 assertion layer.
//!
//! Mesa llvmpipe (CI) supports non-power-of-two textures fully and computes `mediump` at
//! 32-bit precision, so code that breaks the SGX540's limits would pass in CI. Every texture
//! and shader call of this crate goes through [`Gl`], which, when checks are on (debug builds
//! and tests), panics on:
//!
//! - a non-power-of-two (NPOT) texture sampled with a wrap mode other than `CLAMP_TO_EDGE` or
//!   with a mipmap filter (core GLES2 makes such a texture incomplete);
//! - `glGenerateMipmap`;
//! - a texture larger than `GL_MAX_TEXTURE_SIZE`;
//! - a shader that does not start with `#version 100`.
//!
//! Precision is not checked here (only on the device). The rules themselves are plain
//! functions on [`TextureTracker`] and [`check_shader_source`], tested without a GPU.

use glow::HasContext;
use std::cell::RefCell;
use std::collections::HashMap;

/// A broken SGX540 rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    /// A shader does not start with `#version 100`.
    ShaderVersion,
    /// `glGenerateMipmap` was called.
    MipmapGeneration,
    /// A texture is larger than `GL_MAX_TEXTURE_SIZE`.
    TooLarge { width: u32, height: u32, max: u32 },
    /// An NPOT texture is drawn with a repeating wrap mode.
    NpotWrap { width: u32, height: u32 },
    /// An NPOT texture is drawn with a mipmap minification filter.
    NpotMipmapFilter { width: u32, height: u32 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::ShaderVersion => write!(f, "shader does not start with #version 100"),
            Violation::MipmapGeneration => write!(f, "glGenerateMipmap is not used"),
            Violation::TooLarge { width, height, max } => {
                write!(
                    f,
                    "texture {width}x{height} exceeds GL_MAX_TEXTURE_SIZE {max}"
                )
            }
            Violation::NpotWrap { width, height } => {
                write!(
                    f,
                    "NPOT texture {width}x{height} drawn without CLAMP_TO_EDGE"
                )
            }
            Violation::NpotMipmapFilter { width, height } => {
                write!(
                    f,
                    "NPOT texture {width}x{height} drawn with a mipmap filter"
                )
            }
        }
    }
}

/// Checks that `src` starts with `#version 100` on its own line.
pub fn check_shader_source(src: &str) -> Result<(), Violation> {
    match src.strip_prefix("#version 100") {
        Some(rest) if rest.starts_with('\n') || rest.starts_with("\r\n") => Ok(()),
        _ => Err(Violation::ShaderVersion),
    }
}

#[derive(Clone, Copy, Debug)]
struct TexState {
    size: Option<(u32, u32)>,
    wrap_s: u32,
    wrap_t: u32,
    min_filter: u32,
}

impl Default for TexState {
    /// The GL defaults: REPEAT and NEAREST_MIPMAP_LINEAR.
    fn default() -> TexState {
        TexState {
            size: None,
            wrap_s: glow::REPEAT,
            wrap_t: glow::REPEAT,
            min_filter: glow::NEAREST_MIPMAP_LINEAR,
        }
    }
}

/// Texture state as the SGX540 rules need it: sizes, wrap modes, filters, and which texture
/// is bound to which unit. Texture names are plain numbers here.
#[derive(Debug)]
pub struct TextureTracker {
    max_size: u32,
    textures: HashMap<u32, TexState>,
    units: [u32; 8],
    active: usize,
}

impl TextureTracker {
    pub fn new(max_size: u32) -> TextureTracker {
        TextureTracker {
            max_size,
            textures: HashMap::new(),
            units: [0; 8],
            active: 0,
        }
    }

    pub fn created(&mut self, tex: u32) {
        self.textures.insert(tex, TexState::default());
    }

    pub fn deleted(&mut self, tex: u32) {
        self.textures.remove(&tex);
        for u in &mut self.units {
            if *u == tex {
                *u = 0;
            }
        }
    }

    /// `glActiveTexture(GL_TEXTURE0 + unit)`.
    pub fn active_unit(&mut self, unit: usize) {
        self.active = unit.min(self.units.len() - 1);
    }

    /// `glBindTexture(GL_TEXTURE_2D, tex)`; 0 unbinds.
    pub fn bound(&mut self, tex: u32) {
        self.units[self.active] = tex;
    }

    fn current(&mut self) -> Option<&mut TexState> {
        let t = self.units[self.active];
        self.textures.get_mut(&t)
    }

    /// `glTexParameteri` on the bound texture.
    pub fn parameter(&mut self, pname: u32, value: u32) {
        if let Some(s) = self.current() {
            match pname {
                glow::TEXTURE_WRAP_S => s.wrap_s = value,
                glow::TEXTURE_WRAP_T => s.wrap_t = value,
                glow::TEXTURE_MIN_FILTER => s.min_filter = value,
                _ => {}
            }
        }
    }

    /// `glTexImage2D` (level 0) on the bound texture.
    pub fn image(&mut self, width: u32, height: u32) -> Result<(), Violation> {
        let max = self.max_size;
        if width > max || height > max {
            return Err(Violation::TooLarge { width, height, max });
        }
        if let Some(s) = self.current() {
            s.size = Some((width, height));
        }
        Ok(())
    }

    /// A draw call: every bound texture must be usable on the SGX540.
    pub fn draw(&self) -> Result<(), Violation> {
        for &t in &self.units {
            let Some(s) = self.textures.get(&t) else {
                continue;
            };
            let Some((width, height)) = s.size else {
                continue;
            };
            if width.is_power_of_two() && height.is_power_of_two() {
                continue;
            }
            if s.wrap_s != glow::CLAMP_TO_EDGE || s.wrap_t != glow::CLAMP_TO_EDGE {
                return Err(Violation::NpotWrap { width, height });
            }
            if s.min_filter != glow::NEAREST && s.min_filter != glow::LINEAR {
                return Err(Violation::NpotMipmapFilter { width, height });
            }
        }
        Ok(())
    }
}

/// The GL context with the texture and shader calls routed through the SGX540 checks.
/// Other calls go to the [`glow::Context`] directly ([`Gl::raw`]).
pub struct Gl {
    gl: glow::Context,
    tracker: Option<RefCell<TextureTracker>>,
}

fn fail(v: Violation) -> ! {
    panic!("SGX540 rule broken: {v}")
}

impl Gl {
    /// Wraps `gl`; `checks` turns the assertion layer on.
    pub fn new(gl: glow::Context, checks: bool, max_texture_size: u32) -> Gl {
        Gl {
            gl,
            tracker: checks.then(|| RefCell::new(TextureTracker::new(max_texture_size))),
        }
    }

    pub fn raw(&self) -> &glow::Context {
        &self.gl
    }

    pub fn checks_enabled(&self) -> bool {
        self.tracker.is_some()
    }

    fn track(&self, f: impl FnOnce(&mut TextureTracker) -> Result<(), Violation>) {
        if let Some(t) = &self.tracker {
            if let Err(v) = f(&mut t.borrow_mut()) {
                fail(v);
            }
        }
    }

    pub fn create_texture(&self) -> Result<glow::NativeTexture, String> {
        let tex = unsafe { self.gl.create_texture() }?;
        self.track(|t| {
            t.created(tex.0.get());
            Ok(())
        });
        Ok(tex)
    }

    pub fn delete_texture(&self, tex: glow::NativeTexture) {
        unsafe { self.gl.delete_texture(tex) };
        self.track(|t| {
            t.deleted(tex.0.get());
            Ok(())
        });
    }

    pub fn active_texture(&self, unit: u32) {
        unsafe { self.gl.active_texture(glow::TEXTURE0 + unit) };
        self.track(|t| {
            t.active_unit(unit as usize);
            Ok(())
        });
    }

    pub fn bind_texture(&self, tex: Option<glow::NativeTexture>) {
        unsafe { self.gl.bind_texture(glow::TEXTURE_2D, tex) };
        self.track(|t| {
            t.bound(tex.map_or(0, |x| x.0.get()));
            Ok(())
        });
    }

    pub fn tex_parameter(&self, pname: u32, value: u32) {
        unsafe {
            self.gl
                .tex_parameter_i32(glow::TEXTURE_2D, pname, value as i32)
        };
        self.track(|t| {
            t.parameter(pname, value);
            Ok(())
        });
    }

    /// `glTexImage2D` level 0 on the bound texture; `pixels` may be `None` (undefined
    /// contents).
    pub fn tex_image_2d(
        &self,
        internal_format: u32,
        width: u32,
        height: u32,
        format: u32,
        ty: u32,
        pixels: Option<&[u8]>,
    ) {
        self.track(|t| t.image(width, height));
        unsafe {
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal_format as i32,
                width as i32,
                height as i32,
                0,
                format,
                ty,
                glow::PixelUnpackData::Slice(pixels),
            )
        };
    }

    /// Never allowed by the SGX540 rules (NPOT textures have no mipmaps); here so the
    /// assertion layer can be tested.
    pub fn generate_mipmap(&self) {
        self.track(|_| Err(Violation::MipmapGeneration));
        unsafe { self.gl.generate_mipmap(glow::TEXTURE_2D) };
    }

    /// Compiles a shader, checking `#version 100` first.
    pub fn compile_shader(&self, kind: u32, src: &str) -> Result<glow::NativeShader, String> {
        if self.tracker.is_some() {
            if let Err(v) = check_shader_source(src) {
                fail(v);
            }
        }
        unsafe {
            let sh = self.gl.create_shader(kind)?;
            self.gl.shader_source(sh, src);
            self.gl.compile_shader(sh);
            if !self.gl.get_shader_compile_status(sh) {
                let log = self.gl.get_shader_info_log(sh);
                self.gl.delete_shader(sh);
                return Err(format!("shader compile failed: {log}"));
            }
            Ok(sh)
        }
    }

    /// `glDrawArrays(GL_TRIANGLES, 0, count)`, checking the bound textures.
    pub fn draw_triangles(&self, count: i32) {
        if let Some(t) = &self.tracker {
            if let Err(v) = t.borrow().draw() {
                fail(v);
            }
        }
        unsafe { self.gl.draw_arrays(glow::TRIANGLES, 0, count) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_sources_must_start_with_version_100() {
        assert!(check_shader_source("#version 100\nvoid main() {}").is_ok());
        assert!(check_shader_source("void main() {}").is_err());
        assert!(check_shader_source("#version 300 es\nvoid main() {}").is_err());
        assert!(check_shader_source("#version 1000\n").is_err());
        assert!(check_shader_source("\n#version 100\n").is_err());
    }

    fn npot(t: &mut TextureTracker) {
        t.created(1);
        t.bound(1);
        t.image(640, 360).unwrap();
    }

    #[test]
    fn npot_textures_with_gl_defaults_are_rejected_at_draw() {
        let mut t = TextureTracker::new(2048);
        npot(&mut t);
        assert_eq!(
            t.draw(),
            Err(Violation::NpotWrap {
                width: 640,
                height: 360
            })
        );
        t.parameter(glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE);
        t.parameter(glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE);
        assert!(matches!(t.draw(), Err(Violation::NpotMipmapFilter { .. })));
        t.parameter(glow::TEXTURE_MIN_FILTER, glow::NEAREST);
        assert_eq!(t.draw(), Ok(()));
    }

    #[test]
    fn parameters_set_before_the_upload_count() {
        let mut t = TextureTracker::new(2048);
        t.created(1);
        t.bound(1);
        t.parameter(glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE);
        t.parameter(glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE);
        t.parameter(glow::TEXTURE_MIN_FILTER, glow::LINEAR);
        t.image(640, 360).unwrap();
        assert_eq!(t.draw(), Ok(()));
    }

    #[test]
    fn power_of_two_textures_may_repeat() {
        let mut t = TextureTracker::new(2048);
        t.created(1);
        t.bound(1);
        t.image(1024, 1024).unwrap();
        assert_eq!(t.draw(), Ok(()));
    }

    #[test]
    fn textures_above_the_maximum_are_rejected() {
        let mut t = TextureTracker::new(2048);
        t.created(1);
        t.bound(1);
        assert!(matches!(
            t.image(4000, 3000),
            Err(Violation::TooLarge { max: 2048, .. })
        ));
        assert!(t.image(2048, 16).is_ok());
    }

    #[test]
    fn only_bound_textures_are_checked() {
        let mut t = TextureTracker::new(2048);
        npot(&mut t);
        t.bound(0);
        assert_eq!(t.draw(), Ok(()));
        t.active_unit(1);
        t.bound(1);
        assert!(t.draw().is_err(), "bound on another unit");
        t.deleted(1);
        assert_eq!(t.draw(), Ok(()));
    }
}

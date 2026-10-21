//! GLES2 renderer for the Glass apps: the primary rendering path (the software renderer in
//! `glass-gfx` is a debugging aid and the test reference).
//!
//! - [`egl`]: EGL and GL loaded at run time (`libEGL`, `libGLESv2`), off-screen contexts.
//! - [`GlesPainter`]: the [`glass_gfx::Painter`] for GLES2: rounded rectangles by a distance
//!   function, images as textures downscaled on the CPU first, text from an ALPHA glyph
//!   atlas.
//! - [`GlesCompositor`]: the [`glass_gfx::Compositor`]: layers are FBO textures, a frame is
//!   one quad per visible layer.
//! - [`sgx`]: the SGX540 assertion layer (on in debug builds and tests).
//!
//! GLES 2.0 only, no extensions required; one context, one thread.

pub mod compositor;
pub mod downscale;
pub mod egl;
pub mod painter;
pub mod sgx;
pub mod shaders;

pub use compositor::{GlInfo, GlesCompositor, GlesOptions, LayerFormat};
pub use painter::{GlesPainter, ATLAS_SIZE};

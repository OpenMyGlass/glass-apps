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

/// Loads a framebuffer window provider (Model B's `libglass_fbwin`) and asks it for a window:
/// `glass_gfx_native_window_format() -> i32`, then
/// `glass_gfx_create_native_window(u32 width, u32 height, i32 format) -> ANativeWindow*`.
/// Returns the window and its fixed buffer format. The library is never unloaded, because the
/// window lives in it.
pub fn load_framebuffer_window(
    lib: &std::path::Path,
    width: u32,
    height: u32,
) -> Result<(*mut std::ffi::c_void, i32), String> {
    type FormatFn = unsafe extern "C" fn() -> i32;
    type CreateFn = unsafe extern "C" fn(u32, u32, i32) -> *mut std::ffi::c_void;
    let name = lib.display();
    // SAFETY: the provider is a library of this project, following the ABI above.
    unsafe {
        let library =
            libloading::Library::new(lib).map_err(|e| format!("cannot load {name}: {e}"))?;
        let format: libloading::Symbol<FormatFn> = library
            .get(b"glass_gfx_native_window_format\0")
            .map_err(|e| format!("{name}: {e}"))?;
        let create: libloading::Symbol<CreateFn> = library
            .get(b"glass_gfx_create_native_window\0")
            .map_err(|e| format!("{name}: {e}"))?;
        let f = format();
        if f < 0 {
            return Err(format!("{name}: no framebuffer window"));
        }
        let window = create(width, height, f);
        if window.is_null() {
            return Err(format!("{name}: no framebuffer window"));
        }
        std::mem::forget(library);
        Ok((window, f))
    }
}

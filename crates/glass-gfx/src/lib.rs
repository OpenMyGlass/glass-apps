//! Thin software graphics layer for the Glass apps.
//!
//! A [`Frame`] is an RGBA8888 pixel buffer (640×360 for the Glass display, any size for
//! card layers and images). Drawing is done in software with `tiny-skia` (shapes, images)
//! and `fontdue` (text), so the output is the same on every platform and tests can compare
//! pixels. A [`Display`] shows finished frames: [`HeadlessDisplay`] keeps them in memory or
//! writes PNG files; with the `window` feature, [`window::WindowDisplay`] opens a host window.
//!
//! UI code draws through the [`Painter`] and [`Compositor`] traits, so the same card and
//! timeline code runs on this software renderer ([`SkiaPainter`], [`SoftwareCompositor`])
//! and on the GLES2 renderer of `glass-gfx-gles`.

mod color;
mod display;
pub mod fonts;
mod frame;
mod render;
pub mod text;
#[cfg(feature = "window")]
pub mod window;

pub use color::Color;
pub use display::{Display, DisplayError, HeadlessDisplay};
pub use fontdue::Font;
pub use fonts::Weight;
pub use frame::{Frame, ImageError, Rect};
pub use render::{
    Compositor, GfxError, LayerId, Painter, Placement, SkiaPainter, SoftwareCompositor,
};

/// Width of the Glass display in pixels.
pub const GLASS_WIDTH: u32 = 640;
/// Height of the Glass display in pixels.
pub const GLASS_HEIGHT: u32 = 360;

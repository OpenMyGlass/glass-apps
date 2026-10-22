//! The two seams between the UI and a renderer.
//!
//! - [`Painter`]: draw commands into one surface (a card layer). Card code is written
//!   against it only. [`SkiaPainter`] draws into a [`Frame`] in software; the GLES2 painter
//!   lives in `glass-gfx-gles`.
//! - [`Compositor`]: card layers and finished frames. [`SoftwareCompositor`] keeps layers as
//!   [`Frame`]s and hands each composed frame to a [`Display`].

use crate::text::GlyphRun;
use crate::{Color, Display, DisplayError, Frame, Rect};
use std::collections::HashMap;

/// Draw commands into one surface. Colours have straight alpha; all drawing is source-over
/// except [`Painter::clear`], which replaces every pixel.
pub trait Painter {
    /// The surface size in pixels.
    fn size(&self) -> (u32, u32);
    /// Sets every pixel to `color`.
    fn clear(&mut self, color: Color);
    /// Fills a rectangle, anti-aliased at fractional edges.
    fn fill_rect(&mut self, rect: Rect, color: Color);
    /// Fills a rectangle with rounded corners of `radius` pixels, anti-aliased.
    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color);
    /// Draws `image` (premultiplied) scaled to fill `dest`, filtered.
    fn draw_image(&mut self, image: &Frame, dest: Rect);
    /// Draws a laid-out glyph run in `color`.
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>, color: Color);
}

/// The software painter: tiny-skia and fontdue into a [`Frame`] (the M1 renderer).
pub struct SkiaPainter<'a> {
    frame: &'a mut Frame,
}

impl<'a> SkiaPainter<'a> {
    pub fn new(frame: &'a mut Frame) -> SkiaPainter<'a> {
        SkiaPainter { frame }
    }
}

impl Painter for SkiaPainter<'_> {
    fn size(&self) -> (u32, u32) {
        (self.frame.width(), self.frame.height())
    }

    fn clear(&mut self, color: Color) {
        self.frame.fill(color);
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.frame.fill_rect(rect, color);
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        self.frame.fill_rounded_rect(rect, radius, color);
    }

    fn draw_image(&mut self, image: &Frame, dest: Rect) {
        self.frame.draw_image(image, dest);
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>, color: Color) {
        self.frame.draw_glyph_run(run, color);
    }
}

/// A layer owned by a [`Compositor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub u64);

/// Where a layer goes in a composed frame: its top-left corner at whole pixels, and its
/// opacity in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub layer: LayerId,
    pub x: i32,
    pub y: i32,
    pub opacity: f32,
}

impl Placement {
    /// A fully opaque placement.
    pub fn at(layer: LayerId, x: i32, y: i32) -> Placement {
        Placement {
            layer,
            x,
            y,
            opacity: 1.0,
        }
    }
}

/// Rendering failed.
#[derive(Debug)]
pub enum GfxError {
    /// The display refused the frame.
    Display(DisplayError),
    /// The renderer failed (EGL or GL errors, incomplete framebuffers, out of memory).
    Renderer(String),
    /// No layer with this id exists.
    UnknownLayer(LayerId),
}

impl std::fmt::Display for GfxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GfxError::Display(e) => e.fmt(f),
            GfxError::Renderer(e) => write!(f, "renderer error: {e}"),
            GfxError::UnknownLayer(id) => write!(f, "unknown layer {}", id.0),
        }
    }
}

impl std::error::Error for GfxError {}

impl From<DisplayError> for GfxError {
    fn from(e: DisplayError) -> GfxError {
        GfxError::Display(e)
    }
}

/// Layers and frames.
///
/// A frame is composed with [`Compositor::compose`] (every pixel set to the background,
/// then each placement in order) and shown with [`Compositor::present`]. Layers keep their
/// pixels until they are painted again or dropped, so moving a layer costs no painting.
pub trait Compositor {
    /// Short name for logs ("software", "gles").
    fn name(&self) -> &'static str;
    /// The size of composed frames.
    fn size(&self) -> (u32, u32);
    /// A new layer, transparent until painted. `opaque` promises that painting covers every
    /// pixel with opaque colour; the compositor may then store it without alpha and copy it
    /// without blending.
    fn create_layer(&mut self, width: u32, height: u32, opaque: bool) -> Result<LayerId, GfxError>;
    /// Paints into layer `id`.
    fn paint_layer(
        &mut self,
        id: LayerId,
        paint: &mut dyn FnMut(&mut dyn Painter),
    ) -> Result<(), GfxError>;
    /// Frees layer `id` (no effect if it does not exist).
    fn drop_layer(&mut self, id: LayerId);
    /// How many layers exist.
    fn layer_count(&self) -> usize;
    /// Composes a frame: `background` everywhere, then `layers` in order.
    fn compose(&mut self, background: Color, layers: &[Placement]) -> Result<(), GfxError>;
    /// Shows the composed frame. Off-screen compositors hand it to `display`; a compositor
    /// with its own window surface swaps buffers instead.
    fn present(&mut self, display: &mut dyn Display) -> Result<(), GfxError>;
    /// The composed frame as pixels (tests, host development). Slow on a GPU.
    fn read_frame(&mut self) -> Result<Frame, GfxError>;
    /// The native-window side of a compositor that presents into a window surface
    /// (`None` for off-screen compositors).
    fn window_target(&mut self) -> Option<&mut dyn WindowTarget> {
        None
    }
}

/// A compositor that draws into a native window (Android's `ANativeWindow`), which comes
/// and goes with the app's surface while the compositor and its layers stay.
pub trait WindowTarget {
    /// The buffer format the window must have (EGL's native visual id).
    fn native_visual_id(&self) -> Result<i32, GfxError>;
    /// Attaches `window`; frames are presented into it from now on.
    ///
    /// # Safety
    /// `window` must be a valid native window that stays valid until
    /// [`WindowTarget::detach_window`] returns or the compositor is dropped.
    unsafe fn attach_window(&mut self, window: *mut std::ffi::c_void) -> Result<(), GfxError>;
    /// Stops using the window (it is about to be destroyed).
    fn detach_window(&mut self);
    fn has_window(&self) -> bool;
}

struct SoftLayer {
    frame: Frame,
    opaque: bool,
}

/// The software compositor: layers are [`Frame`]s, an opaque layer at full opacity is a row
/// copy, anything else is blended (source-over).
pub struct SoftwareCompositor {
    frame: Frame,
    layers: HashMap<LayerId, SoftLayer>,
    next_id: u64,
}

impl SoftwareCompositor {
    pub fn new(width: u32, height: u32) -> SoftwareCompositor {
        let mut frame = Frame::new(width, height);
        frame.fill(Color::BLACK);
        SoftwareCompositor {
            frame,
            layers: HashMap::new(),
            next_id: 1,
        }
    }

    /// A compositor of the Glass display size.
    pub fn glass() -> SoftwareCompositor {
        SoftwareCompositor::new(crate::GLASS_WIDTH, crate::GLASS_HEIGHT)
    }

    /// The last composed frame.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// The pixels of layer `id`.
    pub fn layer(&self, id: LayerId) -> Option<&Frame> {
        self.layers.get(&id).map(|l| &l.frame)
    }
}

impl Compositor for SoftwareCompositor {
    fn name(&self) -> &'static str {
        "software"
    }

    fn size(&self) -> (u32, u32) {
        (self.frame.width(), self.frame.height())
    }

    fn create_layer(&mut self, width: u32, height: u32, opaque: bool) -> Result<LayerId, GfxError> {
        if width == 0 || height == 0 {
            return Err(GfxError::Renderer(format!(
                "layer size {width}x{height} is empty"
            )));
        }
        let id = LayerId(self.next_id);
        self.next_id += 1;
        self.layers.insert(
            id,
            SoftLayer {
                frame: Frame::new(width, height),
                opaque,
            },
        );
        Ok(id)
    }

    fn paint_layer(
        &mut self,
        id: LayerId,
        paint: &mut dyn FnMut(&mut dyn Painter),
    ) -> Result<(), GfxError> {
        let layer = self.layers.get_mut(&id).ok_or(GfxError::UnknownLayer(id))?;
        paint(&mut SkiaPainter::new(&mut layer.frame));
        Ok(())
    }

    fn drop_layer(&mut self, id: LayerId) {
        self.layers.remove(&id);
    }

    fn layer_count(&self) -> usize {
        self.layers.len()
    }

    fn compose(&mut self, background: Color, layers: &[Placement]) -> Result<(), GfxError> {
        self.frame.fill(background);
        for p in layers {
            let layer = self
                .layers
                .get(&p.layer)
                .ok_or(GfxError::UnknownLayer(p.layer))?;
            if layer.opaque && p.opacity >= 1.0 {
                self.frame.copy_from(&layer.frame, p.x, p.y);
            } else {
                self.frame.blit(&layer.frame, p.x, p.y, p.opacity);
            }
        }
        Ok(())
    }

    fn present(&mut self, display: &mut dyn Display) -> Result<(), GfxError> {
        display.present(&self.frame)?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Frame, GfxError> {
        Ok(self.frame.clone())
    }
}

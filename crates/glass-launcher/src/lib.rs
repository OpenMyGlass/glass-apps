//! The Glass launcher: shows the card timeline and turns input into timeline motion and
//! actions.
//!
//! [`App`] is independent of where input comes from and where frames go. Today the input is
//! a mock script ([`script`]) or the host keyboard; a later change connects it to `glassd`
//! through `glass-api`, converting each `Gesture`/`Scroll` message into a
//! [`glass_ui::InputEvent`] field by field and sending `ScreenRequest{ACTION_SLEEP}` for
//! [`glass_ui::Action::RequestSleep`].

pub mod clock;
pub mod demo;
pub mod script;

pub mod renderer;

use glass_gfx::{Color, Compositor, Display, Frame, GfxError, SoftwareCompositor};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext, Timeline};

/// The launcher state: the timeline, the compositor it renders through, and a stand-in for
/// the screen state that `glassd` owns.
pub struct App {
    timeline: Timeline,
    compositor: Box<dyn Compositor>,
    screen_on: bool,
    shown: bool,
}

impl App {
    /// An app rendering in software (tests, debugging).
    pub fn new(timeline: Timeline, ctx: RenderContext) -> App {
        App::with_compositor(timeline, ctx, Box::new(SoftwareCompositor::glass()))
    }

    /// An app rendering through `compositor` (of the Glass display size).
    pub fn with_compositor(
        mut timeline: Timeline,
        ctx: RenderContext,
        compositor: Box<dyn Compositor>,
    ) -> App {
        timeline.set_context(ctx);
        App {
            timeline,
            compositor,
            screen_on: true,
            shown: false,
        }
    }

    /// The renderer's name ("software", "gles").
    pub fn renderer(&self) -> &'static str {
        self.compositor.name()
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Whether the (simulated) screen is on.
    pub fn screen_on(&self) -> bool {
        self.screen_on
    }

    /// Handles one input event.
    ///
    /// While the (simulated) screen is off, this stands in for `glassd`'s screen policy: a
    /// tap or a forward or backward swipe wakes the screen and is consumed; `Scroll` frames
    /// and other gestures are ignored. With `glassd`, none of these events reach the
    /// launcher while the screen is off, and the dim step (where the restoring touch is
    /// consumed too) is handled there; the launcher only follows `ScreenState`.
    pub fn input(&mut self, event: &InputEvent) -> Option<Action> {
        if !self.screen_on {
            if let InputEvent::Gesture {
                kind: GestureKind::Tap | GestureKind::SwipeForward | GestureKind::SwipeBackward,
                ..
            } = event
            {
                self.screen_on = true;
                self.shown = false;
            }
            return None;
        }
        let action = self.timeline.handle(event);
        if action == Some(Action::RequestSleep) {
            // With glassd: send ScreenRequest{ACTION_SLEEP} and follow ScreenState.
            self.screen_on = false;
            self.shown = false;
        }
        action
    }

    /// Advances time by `dt` seconds with the wall clock `ctx`.
    pub fn step(&mut self, dt: f32, ctx: RenderContext) {
        self.timeline.set_context(ctx);
        self.timeline.tick(dt);
    }

    /// Composes a new frame if anything changed since the last one; true if it did.
    pub fn render_if_changed(&mut self) -> Result<bool, GfxError> {
        if !self.screen_on {
            if self.shown {
                return Ok(false);
            }
            self.compositor.compose(Color::BLACK, &[])?;
            self.shown = true;
            return Ok(true);
        }
        if self.shown && !self.timeline.needs_redraw() {
            return Ok(false);
        }
        self.timeline.render(self.compositor.as_mut())?;
        self.shown = true;
        Ok(true)
    }

    /// Composes and presents a new frame if anything changed; true if one was presented.
    pub fn present_if_changed(&mut self, display: &mut dyn Display) -> Result<bool, GfxError> {
        if !self.render_if_changed()? {
            return Ok(false);
        }
        self.compositor.present(display)?;
        Ok(true)
    }

    /// The last composed frame, read back (tests).
    pub fn read_frame(&mut self) -> Result<Frame, GfxError> {
        self.compositor.read_frame()
    }
}

//! The Glass launcher: shows the card timeline and turns input into timeline motion and
//! actions.
//!
//! [`App`] is independent of where input comes from and where frames go. The input is
//! `glassd` ([`glassd`]: `Gesture`/`Scroll` converted field by field into
//! [`glass_ui::InputEvent`], `ScreenRequest{ACTION_SLEEP}` for
//! [`glass_ui::Action::RequestSleep`], the screen following `ScreenState`), a mock script
//! ([`script`]) or the host keyboard.

pub mod clock;
pub mod demo;
pub mod glassd;
pub mod script;

pub mod renderer;

use glass_gfx::{Color, Compositor, Display, Frame, GfxError, SoftwareCompositor};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext, Timeline};

/// Who decides whether the screen is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenOwner {
    /// A local stand-in for `glassd`'s screen policy (mock scripts, the host window).
    Local,
    /// `glassd`: the app follows `ScreenState` and only asks for sleep.
    Glassd,
}

/// The launcher state: the timeline, the compositor it renders through, and the screen
/// state (owned by `glassd`, or simulated locally without it).
pub struct App {
    timeline: Timeline,
    compositor: Box<dyn Compositor>,
    owner: ScreenOwner,
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
            owner: ScreenOwner::Local,
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

    /// Whether the screen is on (or dimmed).
    pub fn screen_on(&self) -> bool {
        self.screen_on
    }

    /// Lets `glassd` own the screen: the local wake stand-in is off, and the screen changes
    /// only through [`App::screen_off`] and [`App::screen_wake`].
    pub fn follow_glassd(&mut self) {
        self.owner = ScreenOwner::Glassd;
    }

    pub fn screen_owner(&self) -> ScreenOwner {
        self.owner
    }

    /// The screen went off: motion stops where it was heading, and nothing is drawn.
    pub fn screen_off(&mut self) {
        self.timeline.stop_motion();
        self.screen_on = false;
        self.shown = false;
    }

    /// The screen is on again; `return_home` shows the home card at once.
    pub fn screen_wake(&mut self, return_home: bool) {
        if return_home {
            self.timeline.show_home();
        }
        self.screen_on = true;
        self.shown = false;
    }

    /// Ends a touch whose end will not arrive (focus lost, glassd gone); see
    /// [`glass_ui::Timeline::cancel_touch`].
    pub fn cancel_touch(&mut self) {
        self.timeline.cancel_touch();
    }

    /// Whether the timeline is moving (frames are due at the frame rate).
    pub fn is_animating(&self) -> bool {
        !self.timeline.is_settled()
    }

    /// Handles one input event.
    ///
    /// While the (simulated) screen is off, this stands in for `glassd`'s screen policy: a
    /// tap or a forward or backward swipe wakes the screen and is consumed; `Scroll` frames
    /// and other gestures are ignored. With `glassd`, none of these events reach the
    /// launcher while the screen is off, and the dim step (where the restoring touch is
    /// consumed too) is handled there; the launcher only follows `ScreenState`.
    pub fn input(&mut self, event: &InputEvent) -> Option<Action> {
        if self.owner == ScreenOwner::Glassd {
            // glassd delivers no touchpad events while the screen is off, and changes the
            // screen itself (the caller sends ScreenRequest for RequestSleep).
            if !self.screen_on {
                return None;
            }
            return self.timeline.handle(event);
        }
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
            // With glassd the panel is off: nothing to draw until the wake.
            if self.shown || self.owner == ScreenOwner::Glassd {
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

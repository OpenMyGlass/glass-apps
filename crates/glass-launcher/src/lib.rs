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

use glass_gfx::{Color, Frame};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext, Timeline};

/// The launcher state: the timeline, the frame it is drawn into, and a stand-in for the
/// screen state that `glassd` owns.
pub struct App {
    timeline: Timeline,
    frame: Frame,
    screen_on: bool,
    shown: bool,
}

impl App {
    pub fn new(mut timeline: Timeline, ctx: RenderContext) -> App {
        timeline.set_context(ctx);
        App {
            timeline,
            frame: Frame::glass(),
            screen_on: true,
            shown: false,
        }
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

    /// The new frame to present, or `None` when nothing changed since the last one.
    pub fn frame_if_changed(&mut self) -> Option<&Frame> {
        if !self.screen_on {
            if self.shown {
                return None;
            }
            self.frame.fill(Color::BLACK);
            self.shown = true;
            return Some(&self.frame);
        }
        if self.shown && !self.timeline.needs_redraw() {
            return None;
        }
        self.timeline.render(&mut self.frame);
        self.shown = true;
        Some(&self.frame)
    }
}

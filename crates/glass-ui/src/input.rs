//! Input events, the local mirror of the glass-services touchpad messages.
//!
//! Each variant and field maps 1:1 onto the `glass.services.v1` messages (see the
//! glass-services `docs/api.md`), so the launcher converts `glass-api` types field by field:
//!
//! | here | glass-services |
//! |---|---|
//! | `InputEvent::Gesture { kind, fingers, velocity, timestamp_us }` | `Gesture` |
//! | `InputEvent::Scroll { displacement, delta, velocity, fingers, ended, cancelled, timestamp_us }` | `Scroll` |
//! | `GestureKind::Tap`, `LongPress`, `SwipeForward`, `SwipeBackward`, `SwipeDown`, `SwipeUp` | `GESTURE_KIND_TAP`, `GESTURE_KIND_LONG_PRESS`, `GESTURE_KIND_SWIPE_FORWARD`, `GESTURE_KIND_SWIPE_BACKWARD`, `GESTURE_KIND_SWIPE_DOWN`, `GESTURE_KIND_SWIPE_UP` |
//!
//! Units are those of the interface: displacement in touchpad widths (1.0 = the full
//! horizontal travel), velocity in widths per second, positive forward (toward the eye).

/// A touchpad gesture kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GestureKind {
    Tap,
    LongPress,
    SwipeForward,
    SwipeBackward,
    SwipeDown,
    SwipeUp,
}

/// A touchpad event delivered to the focus holder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    /// A completed gesture (sent when the last finger lifts; long press when the hold time passes).
    Gesture {
        kind: GestureKind,
        /// Largest number of fingers seen during the gesture (1, 2 or 3).
        fingers: u32,
        /// Swipes: widths (horizontal) or heights (vertical) per second, positive forward or down.
        velocity: f32,
        timestamp_us: u64,
    },
    /// Continuous horizontal movement while fingers are down.
    Scroll {
        /// Since touch down, in pad widths, positive forward.
        displacement: f32,
        /// Since the previous `Scroll`.
        delta: f32,
        /// Pad widths per second, positive forward.
        velocity: f32,
        fingers: u32,
        /// The last `Scroll` of this touch.
        ended: bool,
        /// With `ended`: the touch was taken away (focus change, long press).
        cancelled: bool,
        timestamp_us: u64,
    },
}

impl InputEvent {
    /// A one-finger gesture with no velocity, for tests and scripts.
    pub fn gesture(kind: GestureKind) -> InputEvent {
        InputEvent::Gesture {
            kind,
            fingers: 1,
            velocity: 0.0,
            timestamp_us: 0,
        }
    }
}

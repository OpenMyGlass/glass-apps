//! UI audio feedback: the short sounds the launcher plays on navigation.
//!
//! On a real Glass every navigation plays a short "click" on the bone-conduction
//! transducer. The launcher knows its own navigation events (a card centres, a card or
//! menu item is selected, a setting toggles), so it plays the feedback locally rather
//! than routing it through `glassd` (no new capability or protocol message is needed;
//! see `docs/plans`).
//!
//! The player is a trait object the caller installs on the [`crate::App`]. The default is
//! [`Silent`], so desktop, headless and CI builds (and any device build without the
//! `sound` feature) make no sound and pull in no audio code. The on-device player that
//! drives ALSA directly lives in [`crate::audio`] behind the `sound` feature.

/// A short UI sound, chosen for a navigation event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// The centred card changed on the timeline: a soft tick.
    CardChange,
    /// A card was tapped or a menu item chosen: a confirm blip.
    Select,
    /// The highlighted item in an open menu changed: a soft, higher tick.
    MenuMove,
    /// A setting (Wi-Fi, Bluetooth) turned on: a rising two-tone tick.
    ToggleOn,
    /// A setting turned off: a falling two-tone tick.
    ToggleOff,
}

/// Plays UI feedback cues. [`App`](crate::App) calls [`Feedback::play`] on the input
/// thread, so an implementation must not block it for long (the ALSA player hands the
/// cue to an audio thread and returns at once).
pub trait Feedback {
    /// Play `cue` (best effort: feedback is never essential, so errors are swallowed).
    fn play(&mut self, cue: Cue);

    /// Sets the output volume in earpiece gain steps (0 to 15). No sound here.
    fn set_volume(&mut self, _steps: u32) {}
}

/// The default player: no sound. Used everywhere the `sound` feature is off, and on the
/// host (window, headless, tests).
#[derive(Default)]
pub struct Silent;

impl Feedback for Silent {
    fn play(&mut self, _cue: Cue) {}
}

//! The first-boot setup flow (R5 plan, milestone 3): pair the Glass with the phone and
//! connect the OpenMyGlass companion, as a pure state machine. The app shows
//! [`Setup::card`] instead of the timeline until the flow is done, and carries out the
//! [`SetupEffect`]s.
//!
//! Steps:
//! 1. **Welcome**: `TAP` starts.
//! 2. **Pair**: Bluetooth on and discoverable; the card names this Glass. A pairing request
//!    from a phone shows its code: `TAP` accepts, `SWIPE_DOWN` refuses. `SWIPE_FORWARD` goes
//!    on (a phone paired before).
//! 3. **Phone**: waits for the companion (`CompanionState{phone_connected}` from glassd);
//!    once connected, `TAP` finishes. `SWIPE_BACKWARD` returns to pairing.
//! 4. Done: the platform records that the system is set up.
//!
//! `SWIPE_DOWN` on the pairing and phone steps asks whether to skip (a user without the
//! phone app can finish): `TAP` skips, `SWIPE_DOWN` goes back.

use crate::platform::{PlatformEvent, PlatformRequest};
use glass_ui::{CardContent, GestureKind, InputEvent};

/// How long the Glass stays visible to phones when the pairing step starts.
pub const DISCOVERABLE_S: u32 = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Pair,
    /// A phone asks to pair.
    Pairing {
        device: String,
        passkey: String,
    },
    Phone,
    /// "Skip setup?", returning to the step it came from.
    Skip(Box<Step>),
}

/// What the app must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupEffect {
    Platform(PlatformRequest),
    /// The flow ended (`skipped`: without a phone); the timeline is shown.
    Done {
        skipped: bool,
    },
}

/// The phone link as glassd reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Companion {
    pub provider: bool,
    pub connected: bool,
    pub phone_name: String,
}

#[derive(Clone, Debug)]
pub struct Setup {
    step: Step,
    bt_name: String,
    companion: Companion,
    done: bool,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup::new()
    }
}

impl Setup {
    pub fn new() -> Setup {
        Setup {
            step: Step::Welcome,
            bt_name: String::new(),
            companion: Companion::default(),
            done: false,
        }
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn set_bluetooth_name(&mut self, name: String) {
        self.bt_name = name;
    }

    pub fn set_companion(&mut self, c: Companion) {
        self.companion = c;
    }

    fn enter_pair(&mut self, fx: &mut Vec<SetupEffect>) {
        self.step = Step::Pair;
        fx.push(SetupEffect::Platform(PlatformRequest::Bluetooth(true)));
        fx.push(SetupEffect::Platform(PlatformRequest::Discoverable(
            DISCOVERABLE_S,
        )));
    }

    fn finish(&mut self, skipped: bool, fx: &mut Vec<SetupEffect>) {
        self.done = true;
        fx.push(SetupEffect::Platform(PlatformRequest::SetProvisioned));
        fx.push(SetupEffect::Done { skipped });
    }

    /// Handles a gesture (other events are ignored).
    pub fn input(&mut self, event: &InputEvent) -> Vec<SetupEffect> {
        let mut fx = Vec::new();
        if self.done {
            return fx;
        }
        let InputEvent::Gesture { kind, fingers, .. } = *event else {
            return fx;
        };
        if fingers > 1 {
            return fx;
        }
        use GestureKind as G;
        match (self.step.clone(), kind) {
            (Step::Welcome, G::Tap) => self.enter_pair(&mut fx),
            (Step::Pair, G::SwipeForward) => self.step = Step::Phone,
            (Step::Pairing { .. }, G::Tap) => {
                fx.push(SetupEffect::Platform(PlatformRequest::AnswerPairing(true)));
                self.step = Step::Phone;
            }
            (Step::Pairing { .. }, G::SwipeDown) => {
                fx.push(SetupEffect::Platform(PlatformRequest::AnswerPairing(false)));
                self.step = Step::Pair;
            }
            (Step::Phone, G::Tap) if self.companion.connected => self.finish(false, &mut fx),
            (Step::Phone, G::SwipeBackward) => self.enter_pair(&mut fx),
            (s @ (Step::Pair | Step::Phone), G::SwipeDown) => self.step = Step::Skip(Box::new(s)),
            (Step::Skip(_), G::Tap) => self.finish(true, &mut fx),
            (Step::Skip(back), G::SwipeDown) => self.step = *back,
            _ => {}
        }
        fx
    }

    /// Handles a platform event: a pairing request, a finished pairing, the device already
    /// being set up.
    pub fn platform_event(&mut self, e: &PlatformEvent) -> Vec<SetupEffect> {
        let mut fx = Vec::new();
        if self.done {
            return fx;
        }
        match e {
            PlatformEvent::BluetoothName(n) => self.bt_name = n.clone(),
            PlatformEvent::PairingRequest { device, passkey } => {
                if matches!(
                    self.step,
                    Step::Pair | Step::Phone | Step::Pairing { .. } | Step::Skip(_)
                ) {
                    self.step = Step::Pairing {
                        device: device.clone(),
                        passkey: passkey.clone(),
                    };
                }
            }
            // Paired without our confirmation (a consent answered elsewhere, or a phone that
            // bonded on its own): go on to the phone step.
            PlatformEvent::Paired { ok: true, .. } => {
                if matches!(self.step, Step::Pair | Step::Pairing { .. }) {
                    self.step = Step::Phone;
                }
            }
            PlatformEvent::Paired { ok: false, .. } => {
                if matches!(self.step, Step::Pairing { .. } | Step::Phone) {
                    self.step = Step::Pair;
                }
            }
            PlatformEvent::Provisioned(true) => {
                // Set up already (an earlier run, or another setup tool): nothing to do.
                self.done = true;
                fx.push(SetupEffect::Done { skipped: true });
            }
            _ => {}
        }
        fx
    }

    /// The card for the current step.
    pub fn card(&self) -> CardContent {
        let info = |title: &str, lines: &[&str]| CardContent::Info {
            title: title.into(),
            lines: lines.iter().map(|s| s.to_string()).collect(),
        };
        let name = if self.bt_name.is_empty() {
            "this Glass".to_string()
        } else {
            self.bt_name.clone()
        };
        match &self.step {
            Step::Welcome => info("Welcome", &["Let's connect your phone.", "Tap to start"]),
            Step::Pair => info(
                "Pair your phone",
                &[
                    "In your phone's Bluetooth",
                    &format!("settings, pair with {name}"),
                    "Swipe down to skip",
                ],
            ),
            Step::Pairing { device, passkey } => {
                let code = if passkey.is_empty() {
                    "Pair with this phone?".to_string()
                } else {
                    format!("Code {passkey}")
                };
                info(
                    "Pairing",
                    &[device, &code, "Tap to pair, swipe down to refuse"],
                )
            }
            Step::Phone if self.companion.connected => {
                let phone = if self.companion.phone_name.is_empty() {
                    "your phone"
                } else {
                    &self.companion.phone_name
                };
                info("Connected", &[&format!("to {phone}"), "Tap to finish"])
            }
            Step::Phone if !self.companion.provider => info(
                "Open OpenMyGlass",
                &[
                    "on your phone and connect.",
                    "Waiting for the Glass app",
                    "Swipe down to skip",
                ],
            ),
            Step::Phone => info(
                "Open OpenMyGlass",
                &["on your phone and connect.", "Swipe down to skip"],
            ),
            Step::Skip(_) => info(
                "Skip setup?",
                &[
                    "You can pair later in Bluetooth.",
                    "Tap to skip, swipe down to go back",
                ],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(kind: GestureKind) -> InputEvent {
        InputEvent::gesture(kind)
    }

    fn platform(fx: &[SetupEffect]) -> Vec<PlatformRequest> {
        fx.iter()
            .filter_map(|e| match e {
                SetupEffect::Platform(r) => Some(r.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn full_flow_with_pairing_and_companion() {
        let mut s = Setup::new();
        assert_eq!(s.step(), &Step::Welcome);
        let fx = s.input(&g(GestureKind::Tap));
        assert_eq!(
            platform(&fx),
            vec![
                PlatformRequest::Bluetooth(true),
                PlatformRequest::Discoverable(DISCOVERABLE_S)
            ]
        );
        assert_eq!(s.step(), &Step::Pair);
        s.platform_event(&PlatformEvent::PairingRequest {
            device: "Pixel".into(),
            passkey: "123456".into(),
        });
        match s.card() {
            CardContent::Info { lines, .. } => assert!(lines.contains(&"Code 123456".to_string())),
            c => panic!("{c:?}"),
        }
        let fx = s.input(&g(GestureKind::Tap));
        assert_eq!(platform(&fx), vec![PlatformRequest::AnswerPairing(true)]);
        assert_eq!(s.step(), &Step::Phone);
        // Not connected yet: a tap does nothing.
        assert!(s.input(&g(GestureKind::Tap)).is_empty());
        s.set_companion(Companion {
            provider: true,
            connected: true,
            phone_name: "Pixel".into(),
        });
        let fx = s.input(&g(GestureKind::Tap));
        assert_eq!(
            fx,
            vec![
                SetupEffect::Platform(PlatformRequest::SetProvisioned),
                SetupEffect::Done { skipped: false }
            ]
        );
        assert!(s.is_done());
        assert!(s.input(&g(GestureKind::Tap)).is_empty());
    }

    #[test]
    fn refusing_a_pairing_and_skipping() {
        let mut s = Setup::new();
        s.input(&g(GestureKind::Tap));
        s.platform_event(&PlatformEvent::PairingRequest {
            device: "X".into(),
            passkey: String::new(),
        });
        let fx = s.input(&g(GestureKind::SwipeDown));
        assert_eq!(platform(&fx), vec![PlatformRequest::AnswerPairing(false)]);
        assert_eq!(s.step(), &Step::Pair);
        s.input(&g(GestureKind::SwipeDown));
        assert_eq!(s.step(), &Step::Skip(Box::new(Step::Pair)));
        s.input(&g(GestureKind::SwipeDown));
        assert_eq!(s.step(), &Step::Pair);
        s.input(&g(GestureKind::SwipeDown));
        let fx = s.input(&g(GestureKind::Tap));
        assert_eq!(fx.last(), Some(&SetupEffect::Done { skipped: true }));
    }

    #[test]
    fn already_provisioned_ends_at_once_and_failed_pairing_returns() {
        let mut s = Setup::new();
        s.input(&g(GestureKind::Tap));
        s.input(&g(GestureKind::SwipeForward));
        assert_eq!(s.step(), &Step::Phone);
        s.platform_event(&PlatformEvent::Paired {
            device: "X".into(),
            ok: false,
        });
        assert_eq!(s.step(), &Step::Pair);
        s.platform_event(&PlatformEvent::Paired {
            device: "X".into(),
            ok: true,
        });
        assert_eq!(s.step(), &Step::Phone);
        s.platform_event(&PlatformEvent::Paired {
            device: "X".into(),
            ok: false,
        });
        assert_eq!(s.step(), &Step::Pair);
        // Two-finger gestures are not for setup.
        let two = InputEvent::Gesture {
            kind: GestureKind::SwipeForward,
            fingers: 2,
            velocity: 1.0,
            timestamp_us: 0,
        };
        assert!(s.input(&two).is_empty());
        assert_eq!(s.step(), &Step::Pair);
        let fx = s.platform_event(&PlatformEvent::Provisioned(true));
        assert_eq!(fx, vec![SetupEffect::Done { skipped: true }]);
        assert!(s.is_done());
        // Provisioned(false) at start changes nothing.
        let mut s = Setup::new();
        assert!(s
            .platform_event(&PlatformEvent::Provisioned(false))
            .is_empty());
        assert!(!s.is_done());
    }
}

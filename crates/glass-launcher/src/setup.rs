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
use glass_ui::i18n::{fmt1, Locale, Msg};
use glass_ui::{CardContent, GestureKind, InputEvent};

/// How long the Glass stays visible to phones when the pairing step starts.
pub const DISCOVERABLE_S: u32 = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Scroll-to-pick the UI language (the first step when no locale is set): swipe moves
    /// through the languages, tap chooses. `index` is into [`Locale::ALL`].
    Language {
        index: usize,
    },
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
    /// Apply (and persist through glassd) a language picked in the setup flow.
    SetLocale(Locale),
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
    /// A phone is bonded (glassd holds the link, Model B), whether connected or not.
    pub paired: bool,
    /// The bonded phone's name (empty if none or unknown).
    pub paired_name: String,
    /// The pairing mode is open until this `CLOCK_MONOTONIC` time, in µs; 0 when closed.
    pub pairing_until_us: u64,
}

#[derive(Clone, Debug)]
pub struct Setup {
    step: Step,
    bt_name: String,
    companion: Companion,
    /// The language the cards render in (and the picker's live preview).
    locale: Locale,
    done: bool,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup::new()
    }
}

impl Setup {
    /// A flow that starts at the welcome step (a locale is already set).
    pub fn new() -> Setup {
        Setup {
            step: Step::Welcome,
            bt_name: String::new(),
            companion: Companion::default(),
            locale: Locale::DEFAULT,
            done: false,
        }
    }

    /// A flow that opens with the language picker, highlighting `current` — the first-boot
    /// path when no locale is set yet.
    pub fn with_language_picker(current: Locale) -> Setup {
        let index = Locale::ALL.iter().position(|&l| l == current).unwrap_or(0);
        Setup {
            step: Step::Language { index },
            locale: current,
            ..Setup::new()
        }
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn set_bluetooth_name(&mut self, name: String) {
        self.bt_name = name;
    }

    /// Keeps the cards' language in step with the app (e.g. the companion pushed a locale
    /// mid-setup).
    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
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
            (Step::Language { index }, G::SwipeForward | G::SwipeBackward) => {
                let next = if kind == G::SwipeForward {
                    (index + 1).min(Locale::ALL.len() - 1)
                } else {
                    index.saturating_sub(1)
                };
                if next != index {
                    self.step = Step::Language { index: next };
                    // Preview the highlighted language at once (live re-render).
                    self.locale = Locale::ALL[next];
                    fx.push(SetupEffect::SetLocale(self.locale));
                }
            }
            (Step::Language { index }, G::Tap) => {
                self.locale = Locale::ALL[index];
                fx.push(SetupEffect::SetLocale(self.locale));
                self.step = Step::Welcome;
            }
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

    /// The card for the current step, in the flow's current language.
    pub fn card(&self) -> CardContent {
        let loc = self.locale;
        let t = |m: Msg| m.get(loc).to_string();
        let info = |title: Msg, lines: Vec<String>| CardContent::Info {
            title: t(title),
            lines,
        };
        let name = if self.bt_name.is_empty() {
            t(Msg::SetupThisGlass)
        } else {
            self.bt_name.clone()
        };
        match &self.step {
            Step::Language { index } => {
                let picked = Locale::ALL[*index];
                CardContent::MenuItem {
                    // The endonym is the same in any language; the hint previews the pick.
                    label: picked.endonym().to_string(),
                    detail: Some(Msg::ChooseLanguage.get(picked).to_string()),
                    index: *index,
                    count: Locale::ALL.len(),
                }
            }
            Step::Welcome => info(
                Msg::WelcomeTitle,
                vec![t(Msg::WelcomeConnectPhone), t(Msg::TapToStart)],
            ),
            Step::Pair => info(
                Msg::PairTitle,
                vec![
                    t(Msg::PairLine1),
                    fmt1(Msg::PairLine2.get(loc), &name),
                    t(Msg::SwipeDownToSkip),
                ],
            ),
            Step::Pairing { device, passkey } => {
                let code = if passkey.is_empty() {
                    t(Msg::PairWithThisPhone)
                } else {
                    fmt1(Msg::Code.get(loc), passkey)
                };
                info(
                    Msg::PairingTitle,
                    vec![device.clone(), code, t(Msg::TapPairSwipeRefuse)],
                )
            }
            Step::Phone if self.companion.connected => {
                let phone = if self.companion.phone_name.is_empty() {
                    t(Msg::YourPhone)
                } else {
                    self.companion.phone_name.clone()
                };
                info(
                    Msg::ConnectedTitle,
                    vec![fmt1(Msg::ToTarget.get(loc), &phone), t(Msg::TapToFinish)],
                )
            }
            Step::Phone if !self.companion.provider => info(
                Msg::OpenCompanionTitle,
                vec![
                    t(Msg::OpenCompanionLine),
                    t(Msg::WaitingGlassApp),
                    t(Msg::SwipeDownToSkip),
                ],
            ),
            Step::Phone => info(
                Msg::OpenCompanionTitle,
                vec![t(Msg::OpenCompanionLine), t(Msg::SwipeDownToSkip)],
            ),
            Step::Skip(_) => info(
                Msg::SkipSetupTitle,
                vec![t(Msg::SkipSetupLine), t(Msg::TapSkipSwipeBack)],
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
    fn language_picker_scrolls_previews_and_chooses() {
        let mut s = Setup::with_language_picker(Locale::En);
        assert_eq!(s.step(), &Step::Language { index: 1 });
        // The card shows the highlighted language's own name.
        match s.card() {
            CardContent::MenuItem { label, count, .. } => {
                assert_eq!(label, "English");
                assert_eq!(count, 4);
            }
            c => panic!("{c:?}"),
        }
        // Swiping backward highlights Italian and previews it at once.
        let fx = s.input(&g(GestureKind::SwipeBackward));
        assert_eq!(fx, vec![SetupEffect::SetLocale(Locale::It)]);
        assert_eq!(s.locale(), Locale::It);
        match s.card() {
            CardContent::MenuItem { label, .. } => assert_eq!(label, "Italiano"),
            c => panic!("{c:?}"),
        }
        // Already at the first language: another backward swipe does nothing.
        assert!(s.input(&g(GestureKind::SwipeBackward)).is_empty());
        // Tap chooses the highlighted language and moves to Welcome.
        let fx = s.input(&g(GestureKind::Tap));
        assert_eq!(fx, vec![SetupEffect::SetLocale(Locale::It)]);
        assert_eq!(s.step(), &Step::Welcome);
        match s.card() {
            CardContent::Info { title, .. } => assert_eq!(title, "Benvenuto"),
            c => panic!("{c:?}"),
        }
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
            ..Default::default()
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

//! "Pair phone": the screen shown while glassd's pairing mode is open (glass-services
//! `docs/api.md`, "Pairing mode"), as a pure state machine behind a full-screen card.
//!
//! Over BLE glassd bonds a first phone only while the owner has the mode open on the Glass.
//! The launcher sends `CompanionPairingWindow{open: true}` and shows this screen: how to
//! connect from the phone, and the time left, read from `CompanionState.pairing_until_us`
//! (on `CLOCK_MONOTONIC`, like every glassd `timestamp_us`). The first phone that bonds wins
//! and no code is compared, so whenever `paired` turns true (here or in any other way) the app
//! shows `paired_name` and asks whether this is the owner's phone (`App::companion_changed`).
//! `SWIPE_DOWN` backs out and closes the mode. When the time runs out the screen says so;
//! `TAP` opens the mode again. The same screen tells why a request was refused, also a
//! `CompanionForgetBond` ([`Pairing::failed`]).

use crate::settings::timeout_text;
use crate::setup::Companion;
use glass_api::proto::ErrorCode;
use glass_ui::i18n::{fmt1, Locale, Msg};
use glass_ui::{CardContent, GestureKind, InputEvent};
use std::time::Duration;

/// How long the mode is opened for. glassd allows up to 300 s; its default (0) is 120 s.
pub const WINDOW_MS: u32 = 120_000;

/// How long the screen is held on for one opening: the window and a margin for the answer.
pub const KEEP_ON_MS: u32 = WINDOW_MS + 10_000;

/// How long the screen is held on for the question after a bond. Unanswered, it times out and
/// the question comes back at the next wake.
pub const QUESTION_KEEP_ON_MS: u32 = 60_000;

/// glassd's message for an opening refused because its phone link is down (api.md contract).
pub const LINK_DISABLED: &str = "the companion link is disabled";

/// A pairing request to glassd, to tell its error answers apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairRequest {
    /// `CompanionPairingWindow{open: true}`.
    Open,
    /// `CompanionPairingWindow{open: false}`.
    Close,
    /// `CompanionForgetBond`.
    Forget,
}

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `ERROR_CODE_UNSUPPORTED`: this glassd does not hold the phone link.
    Unsupported,
    /// `ERROR_CODE_DENIED`: the launcher is not a privileged client of glassd.
    Denied,
    /// `ERROR_CODE_UNAVAILABLE` to an opening: a phone is bonded already.
    AlreadyPaired,
    /// `ERROR_CODE_UNAVAILABLE`, "the companion link is disabled": glassd could not seed its
    /// random numbers, so it holds no link at all.
    LinkDisabled,
    /// `ERROR_CODE_UNAVAILABLE` to a forget: the bond is still on the disk.
    NotErased,
    Other,
}

impl Refusal {
    /// The refusal of `request` with `code` and glassd's `message`. The message is for logs,
    /// except one: glass-services docs/api.md ("Pairing mode") makes "the companion link is
    /// disabled" the contract that tells that `ERROR_CODE_UNAVAILABLE` of an opening from the
    /// "already bonded" one, and glassd pins it with a test (`LINK_DISABLED`).
    pub fn of(request: PairRequest, code: ErrorCode, message: &str) -> Refusal {
        match code {
            ErrorCode::Unsupported => Refusal::Unsupported,
            ErrorCode::Denied => Refusal::Denied,
            ErrorCode::Unavailable if message.contains(LINK_DISABLED) => Refusal::LinkDisabled,
            ErrorCode::Unavailable if request == PairRequest::Forget => Refusal::NotErased,
            ErrorCode::Unavailable if request == PairRequest::Open => Refusal::AlreadyPaired,
            _ => Refusal::Other,
        }
    }

    fn text(self, request: PairRequest, locale: Locale) -> &'static str {
        match self {
            Refusal::Unsupported => Msg::PairingNotAvailable,
            Refusal::Denied => Msg::PairingNotAllowed,
            Refusal::AlreadyPaired => Msg::AlreadyPaired,
            Refusal::LinkDisabled => Msg::PhoneLinkDisabled,
            Refusal::NotErased => Msg::BondNotErased,
            Refusal::Other if request == PairRequest::Forget => Msg::BondNotErased,
            Refusal::Other => Msg::PairingCouldNotOpen,
        }
        .get(locale)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Asked for, glassd has not shown it open yet.
    Opening,
    /// Open until this `CLOCK_MONOTONIC` time (µs).
    Open {
        until_us: u64,
    },
    /// It closed by itself (its time ran out, or glassd restarted) with no phone bonded.
    Ended,
    Refused(PairRequest, Refusal),
    /// A phone bonded (the mode is closed by glassd); the app asks the owner about it.
    Paired,
}

/// What the app must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingOut {
    /// Close the screen; glassd's mode is closed already or is closed by the app on close.
    Close,
    /// Open the mode again (the time starts again).
    Reopen,
    /// The mode is no longer open: the screen need not be held on.
    ReleaseScreen,
}

#[derive(Clone, Debug)]
pub struct Pairing {
    phase: Phase,
}

impl Default for Pairing {
    fn default() -> Pairing {
        Pairing::new()
    }
}

impl Pairing {
    pub fn new() -> Pairing {
        Pairing {
            phase: Phase::Opening,
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Whether the mode is (or is about to be) open at glassd, so closing the screen must close it.
    pub fn is_open(&self) -> bool {
        matches!(self.phase, Phase::Opening | Phase::Open { .. })
    }

    /// The screen that tells why `request` was refused.
    pub fn failed(request: PairRequest, r: Refusal) -> Pairing {
        Pairing {
            phase: Phase::Refused(request, r),
        }
    }

    /// glassd refused the `CompanionPairingWindow{open: true}`.
    pub fn refused(&mut self, r: Refusal) -> Option<PairingOut> {
        if !self.is_open() {
            return None;
        }
        self.phase = Phase::Refused(PairRequest::Open, r);
        Some(PairingOut::ReleaseScreen)
    }

    /// glassd's `CompanionState`.
    pub fn on_companion(&mut self, c: &Companion) -> Option<PairingOut> {
        if self.phase == Phase::Paired {
            return None;
        }
        if c.paired {
            // The app asks the owner about the phone, however it bonded, and replaces this
            // screen with the question; glassd closed the mode with the bond.
            self.phase = Phase::Paired;
            return None;
        }
        match (&self.phase, c.pairing_until_us) {
            (Phase::Opening | Phase::Open { .. }, until_us) if until_us > 0 => {
                self.phase = Phase::Open { until_us };
                None
            }
            (Phase::Open { .. }, 0) => {
                self.phase = Phase::Ended;
                Some(PairingOut::ReleaseScreen)
            }
            _ => None,
        }
    }

    /// The local clock passed the end of the window before glassd said it closed: show it
    /// ended (glassd's own close then changes nothing).
    pub fn tick(&mut self, now_us: u64) -> Option<PairingOut> {
        match self.phase {
            Phase::Open { until_us } if now_us >= until_us => {
                self.phase = Phase::Ended;
                Some(PairingOut::ReleaseScreen)
            }
            _ => None,
        }
    }

    /// A gesture while the screen is shown.
    pub fn input(&mut self, event: &InputEvent) -> Option<PairingOut> {
        let InputEvent::Gesture { kind, .. } = *event else {
            return None;
        };
        match (kind, &self.phase) {
            (GestureKind::SwipeDown, _) => Some(PairingOut::Close),
            (GestureKind::Tap, Phase::Ended) => {
                self.phase = Phase::Opening;
                Some(PairingOut::Reopen)
            }
            (GestureKind::Tap, Phase::Refused(..)) => Some(PairingOut::Close),
            _ => None,
        }
    }

    /// How long until the shown time left changes (the loop wakes then); `None` when no
    /// time is shown.
    pub fn next_change(&self, now_us: u64) -> Option<Duration> {
        let Phase::Open { until_us } = self.phase else {
            return None;
        };
        let left = until_us.saturating_sub(now_us);
        // The time shown is rounded up, so it changes when `left` crosses a whole second.
        let to_next = match left % 1_000_000 {
            0 => 1_000_000,
            r => r,
        };
        Some(Duration::from_micros(to_next.min(left.max(1))))
    }

    /// The screen, built in `locale` at `now_us` (`CLOCK_MONOTONIC`).
    pub fn card(&self, locale: Locale, now_us: u64) -> CardContent {
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
        match &self.phase {
            Phase::Opening => CardContent::Info {
                title: Msg::PairingModeTitle.get(locale).into(),
                lines: lines(&[
                    Msg::PairingOpening.get(locale),
                    Msg::SwipeDownToStop.get(locale),
                ]),
            },
            Phase::Open { until_us } => CardContent::Info {
                title: Msg::PairingModeTitle.get(locale).into(),
                lines: vec![
                    Msg::PairingHowTo.get(locale).into(),
                    fmt1(
                        Msg::TimeLeft.get(locale),
                        &time_left(until_us.saturating_sub(now_us)),
                    ),
                    Msg::SwipeDownToStop.get(locale).into(),
                ],
            },
            Phase::Ended => CardContent::Info {
                title: Msg::PairingEnded.get(locale).into(),
                lines: lines(&[
                    Msg::NoPhonePaired.get(locale),
                    Msg::TapRetrySwipeClose.get(locale),
                ]),
            },
            Phase::Refused(request, r) => CardContent::Info {
                title: match request {
                    PairRequest::Forget => Msg::PhoneNotForgotten,
                    _ => Msg::PairingNotStarted,
                }
                .get(locale)
                .into(),
                lines: lines(&[r.text(*request, locale), Msg::TapToClose.get(locale)]),
            },
            // Not shown: the app replaces the screen with the question at once.
            Phase::Paired => CardContent::Info {
                title: Msg::Paired.get(locale).into(),
                lines: Vec::new(),
            },
        }
    }
}

/// "1:52", rounded up to the second (it reads 0:00 only once closed). Language-neutral.
pub fn time_left(left_us: u64) -> String {
    let s = left_us.div_ceil(1_000_000);
    format!("{}:{:02}", s / 60, s % 60)
}

/// "Paired with Pixel 8", or "Paired" when glassd does not know the name.
pub fn paired_text(locale: Locale, name: &str) -> String {
    if name.is_empty() {
        Msg::Paired.get(locale).into()
    } else {
        fmt1(Msg::PairedWith.get(locale), name)
    }
}

/// "2 minutes": how long one opening lasts.
pub fn window_text(locale: Locale) -> String {
    timeout_text(locale, WINDOW_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn companion(paired: bool, name: &str, until: u64) -> Companion {
        Companion {
            paired,
            paired_name: name.into(),
            pairing_until_us: until,
            ..Default::default()
        }
    }

    fn lines(c: CardContent) -> Vec<String> {
        match c {
            CardContent::Info { lines, .. } => lines,
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn opening_counts_down_and_ends() {
        let mut p = Pairing::new();
        assert!(p.is_open());
        // A state from before the request (still closed) changes nothing.
        assert_eq!(p.on_companion(&companion(false, "", 0)), None);
        assert_eq!(p.phase(), &Phase::Opening);
        let now = 5_000_000;
        assert_eq!(
            p.on_companion(&companion(false, "", now + 112_300_000)),
            None
        );
        assert_eq!(lines(p.card(Locale::En, now))[1], "1:53 left");
        assert_eq!(lines(p.card(Locale::It, now))[1], "Ancora 1:53");
        // The next change is when 112.3 s left becomes 112 s.
        assert_eq!(p.next_change(now), Some(Duration::from_millis(300)));
        assert_eq!(p.next_change(now + 300_000), Some(Duration::from_secs(1)));
        // A repeat restarts the time.
        p.on_companion(&companion(false, "", now + 120_000_000));
        assert_eq!(lines(p.card(Locale::En, now))[1], "2:00 left");
        assert_eq!(
            p.on_companion(&companion(false, "", 0)),
            Some(PairingOut::ReleaseScreen)
        );
        assert_eq!(p.phase(), &Phase::Ended);
        assert!(!p.is_open());
        assert_eq!(p.next_change(now), None);
        assert_eq!(
            p.input(&InputEvent::gesture(GestureKind::Tap)),
            Some(PairingOut::Reopen)
        );
        assert_eq!(p.phase(), &Phase::Opening);
    }

    #[test]
    fn the_local_clock_ends_it_too() {
        let mut p = Pairing::new();
        p.on_companion(&companion(false, "", 10_000_000));
        assert_eq!(p.tick(9_999_999), None);
        assert_eq!(p.tick(10_000_000), Some(PairingOut::ReleaseScreen));
        assert_eq!(p.phase(), &Phase::Ended);
        // glassd's close afterwards changes nothing.
        assert_eq!(p.on_companion(&companion(false, "", 0)), None);
    }

    #[test]
    fn a_bond_ends_the_screen() {
        let mut p = Pairing::new();
        p.on_companion(&companion(false, "", 10_000_000));
        // The app asks the owner; this screen only stops being open.
        assert_eq!(p.on_companion(&companion(true, "Pixel 8", 0)), None);
        assert_eq!(p.phase(), &Phase::Paired);
        assert!(!p.is_open());
        assert_eq!(p.on_companion(&companion(false, "", 0)), None);
        assert_eq!(paired_text(Locale::En, "Pixel 8"), "Paired with Pixel 8");
        assert_eq!(paired_text(Locale::Fr, ""), "Associé");
    }

    #[test]
    fn refusals_and_back_out() {
        let mut p = Pairing::new();
        assert_eq!(
            p.input(&InputEvent::gesture(GestureKind::SwipeDown)),
            Some(PairingOut::Close)
        );
        let r = Refusal::of(PairRequest::Open, ErrorCode::Unsupported, "");
        assert_eq!(p.refused(r), Some(PairingOut::ReleaseScreen));
        assert_eq!(
            p.phase(),
            &Phase::Refused(PairRequest::Open, Refusal::Unsupported)
        );
        assert!(!p.is_open());
        assert_eq!(
            lines(p.card(Locale::En, 0))[0],
            "Not available on this Glass"
        );
        assert_eq!(
            p.input(&InputEvent::gesture(GestureKind::Tap)),
            Some(PairingOut::Close)
        );
    }

    #[test]
    fn each_refusal_says_what_happened() {
        use PairRequest::{Forget, Open};
        let busy = ErrorCode::Unavailable;
        let cases = [
            (
                Open,
                busy,
                "already bonded: forget the bond first",
                Refusal::AlreadyPaired,
            ),
            (
                Open,
                busy,
                "the companion link is disabled",
                Refusal::LinkDisabled,
            ),
            (
                Forget,
                busy,
                "the bond is still on disk: EIO",
                Refusal::NotErased,
            ),
            (Open, ErrorCode::Denied, "", Refusal::Denied),
            (Forget, ErrorCode::Denied, "", Refusal::Denied),
            (Forget, ErrorCode::Unsupported, "", Refusal::Unsupported),
            (Open, ErrorCode::InvalidArgument, "", Refusal::Other),
        ];
        for (req, code, msg, want) in cases {
            assert_eq!(Refusal::of(req, code, msg), want, "{req:?} {code:?} {msg}");
        }
        let card = |req, r| match Pairing::failed(req, r).card(Locale::En, 0) {
            CardContent::Info { title, lines } => (title, lines[0].clone()),
            c => panic!("{c:?}"),
        };
        assert_eq!(
            card(Forget, Refusal::NotErased),
            (
                "Phone not forgotten".to_string(),
                "The pairing could not be erased".to_string()
            )
        );
        assert_eq!(
            card(Open, Refusal::AlreadyPaired).1,
            "A phone is already paired"
        );
        assert_eq!(
            card(Open, Refusal::LinkDisabled).1,
            "The phone link is disabled"
        );
        assert_eq!(
            card(Open, Refusal::Denied).1,
            "The launcher is not allowed to do this"
        );
        // All different, in every language.
        for l in Locale::ALL {
            let mut texts: Vec<&str> = [
                Refusal::Unsupported,
                Refusal::Denied,
                Refusal::AlreadyPaired,
                Refusal::LinkDisabled,
                Refusal::NotErased,
            ]
            .iter()
            .map(|r| r.text(Open, l))
            .collect();
            texts.sort();
            texts.dedup();
            assert_eq!(texts.len(), 5, "{l:?}");
        }
    }

    #[test]
    fn time_left_rounds_up() {
        assert_eq!(time_left(120_000_000), "2:00");
        assert_eq!(time_left(119_000_001), "2:00");
        assert_eq!(time_left(59_000_000), "0:59");
        assert_eq!(time_left(1), "0:01");
        assert_eq!(time_left(0), "0:00");
        assert_eq!(window_text(Locale::En), "2 minutes");
    }
}

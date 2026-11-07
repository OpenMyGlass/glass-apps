//! "Pair phone": the screen shown while glassd's pairing mode is open (glass-services
//! `docs/api.md`, "Pairing mode"), as a pure state machine behind a full-screen card.
//!
//! Over BLE glassd bonds a first phone only while the owner has the mode open on the Glass.
//! The launcher sends `CompanionPairingWindow{open: true}` and shows this screen: how to
//! connect from the phone, and the time left, read from `CompanionState.pairing_until_us`
//! (on `CLOCK_MONOTONIC`, like every glassd `timestamp_us`). The first phone that bonds wins
//! and no code is compared, so when `paired` turns true the app shows `paired_name` and asks
//! whether this is the owner's phone ([`PairingOut::Paired`]). `SWIPE_DOWN` backs out and
//! closes the mode. When the time runs out the screen says so; `TAP` opens the mode again.

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

/// Why the mode could not be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `ERROR_CODE_UNSUPPORTED`: this glassd has no pairing mode.
    Unsupported,
    /// `ERROR_CODE_UNAVAILABLE`: a phone is bonded already.
    AlreadyPaired,
    Other,
}

impl Refusal {
    pub fn from_code(code: ErrorCode) -> Refusal {
        match code {
            ErrorCode::Unsupported => Refusal::Unsupported,
            ErrorCode::Unavailable => Refusal::AlreadyPaired,
            _ => Refusal::Other,
        }
    }

    fn text(self, locale: Locale) -> &'static str {
        match self {
            Refusal::Unsupported => Msg::PairingNotAvailable,
            Refusal::AlreadyPaired => Msg::AlreadyPaired,
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
    Refused(Refusal),
    /// A phone bonded (the mode is closed by glassd).
    Paired(String),
}

/// What the app must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingOut {
    /// Close the screen; glassd's mode is closed already or is closed by the app on close.
    Close,
    /// Open the mode again (the time starts again).
    Reopen,
    /// A phone bonded: show its name and ask if it is the owner's.
    Paired(String),
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

    /// The mode was asked for again.
    pub fn reopening(&mut self) {
        self.phase = Phase::Opening;
    }

    /// glassd refused the `CompanionPairingWindow`.
    pub fn refused(&mut self, r: Refusal) -> Option<PairingOut> {
        if !self.is_open() {
            return None;
        }
        self.phase = Phase::Refused(r);
        Some(PairingOut::ReleaseScreen)
    }

    /// glassd's `CompanionState`.
    pub fn on_companion(&mut self, c: &Companion) -> Option<PairingOut> {
        if matches!(self.phase, Phase::Paired(_)) {
            return None;
        }
        if c.paired {
            // Also outside the window (a bond over TCP): either way, the owner checks it.
            self.phase = Phase::Paired(c.paired_name.clone());
            return Some(PairingOut::Paired(c.paired_name.clone()));
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
            (GestureKind::Tap, Phase::Refused(_)) => Some(PairingOut::Close),
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
            Phase::Refused(r) => CardContent::Info {
                title: Msg::PairingNotStarted.get(locale).into(),
                lines: lines(&[r.text(locale), Msg::TapToClose.get(locale)]),
            },
            // Not shown: the app replaces the screen with the question at once.
            Phase::Paired(name) => CardContent::Info {
                title: paired_text(locale, name),
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
    fn a_bond_asks_the_owner() {
        let mut p = Pairing::new();
        p.on_companion(&companion(false, "", 10_000_000));
        assert_eq!(
            p.on_companion(&companion(true, "Pixel 8", 0)),
            Some(PairingOut::Paired("Pixel 8".into()))
        );
        assert!(!p.is_open());
        assert_eq!(p.on_companion(&companion(true, "Pixel 8", 0)), None);
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
        p.refused(Refusal::from_code(ErrorCode::Unsupported));
        assert_eq!(p.phase(), &Phase::Refused(Refusal::Unsupported));
        assert!(!p.is_open());
        assert_eq!(
            lines(p.card(Locale::En, 0))[0],
            "Not available on this Glass"
        );
        assert_eq!(
            p.input(&InputEvent::gesture(GestureKind::Tap)),
            Some(PairingOut::Close)
        );
        assert_eq!(
            Refusal::from_code(ErrorCode::Unavailable),
            Refusal::AlreadyPaired
        );
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

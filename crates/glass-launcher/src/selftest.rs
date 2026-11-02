//! Guided on-device self-test (`glass-launcher --self-test`): an ordered, locale-aware
//! walkthrough that exercises each home feature and logs one `OMGB-SELFTEST:` line per
//! step so the full report is readable from `/proc/last_kmsg`.
//!
//! This module is the pure state machine: it is fed [`Ev`] events (gestures, head, wink,
//! wear, screen) and ticks, and it produces the instruction text, the audio [`Sound`]s to
//! play, the `OMGB-SELFTEST:` log lines, and the ✓/✗ marks. The device runner
//! (`main::run_self_test`) wires it to glassd frames, the framebuffer and the ALSA player;
//! the headless test drives it with a scripted event sequence. Every step is skippable
//! (a swipe-down, or a ~20 s timeout) so a stuck sensor never blocks the run.

use crate::feedback::Cue;
use glass_ui::i18n::Locale;
use glass_ui::GestureKind;

/// Per-step timeout before it is auto-skipped.
const STEP_TIMEOUT_S: f32 = 20.0;
/// Seconds each language is shown in the `language` step.
const LANG_SHOW_S: f32 = 2.0;

/// The ordered steps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    NavSwipeFwd,
    NavSwipeBack,
    NavTap,
    NavSwipeDown,
    TouchPrecision,
    AudioTone,
    HeadWake,
    Wink,
    OnHead,
    WifiToggle,
    BtToggle,
    Airplane,
    Language,
    ScreenDim,
}

impl Step {
    pub const ALL: [Step; 14] = [
        Step::NavSwipeFwd,
        Step::NavSwipeBack,
        Step::NavTap,
        Step::NavSwipeDown,
        Step::TouchPrecision,
        Step::AudioTone,
        Step::HeadWake,
        Step::Wink,
        Step::OnHead,
        Step::WifiToggle,
        Step::BtToggle,
        Step::Airplane,
        Step::Language,
        Step::ScreenDim,
    ];

    /// The stable step name used in the log (`step=<name>`).
    pub fn name(self) -> &'static str {
        match self {
            Step::NavSwipeFwd => "nav_swipe_fwd",
            Step::NavSwipeBack => "nav_swipe_back",
            Step::NavTap => "nav_tap",
            Step::NavSwipeDown => "nav_swipe_down",
            Step::TouchPrecision => "touch_precision",
            Step::AudioTone => "audio_tone",
            Step::HeadWake => "head_wake",
            Step::Wink => "wink",
            Step::OnHead => "on_head",
            Step::WifiToggle => "wifi_toggle",
            Step::BtToggle => "bt_toggle",
            Step::Airplane => "airplane",
            Step::Language => "language",
            Step::ScreenDim => "screen_dim",
        }
    }

    /// What the step waits for, for the log (`expected=<x>`).
    fn expected(self) -> &'static str {
        match self {
            Step::NavSwipeFwd => "swipe_forward",
            Step::NavSwipeBack => "swipe_backward",
            Step::NavTap => "tap",
            Step::NavSwipeDown => "swipe_down",
            Step::TouchPrecision => "3taps+3swipes",
            Step::AudioTone => "tap=yes|down=no",
            Step::HeadWake => "head_up",
            Step::Wink => "wink",
            Step::OnHead => "doff_then_don",
            Step::WifiToggle => "wifi_toggled",
            Step::BtToggle => "bt_toggled",
            Step::Airplane => "airplane_on",
            Step::Language => "4_locales_shown",
            Step::ScreenDim => "dim_then_wake",
        }
    }
}

/// An input to the self-test, mapped from a glassd frame (or synthesised in tests).
#[derive(Clone, Copy, Debug)]
pub enum Ev {
    Gesture(GestureKind),
    /// A head gesture (head-up) was recognised.
    Head,
    /// A wink was recognised.
    Wink,
    /// A wear-state change; `on_head` is the new state.
    Wear {
        on_head: bool,
    },
    /// A screen-state change; `lit` false means dim or off.
    Screen {
        lit: bool,
    },
    /// A `Settings` snapshot/update from glassd (radio state for the toggle steps).
    Settings {
        wifi: Option<bool>,
        bt: Option<bool>,
        airplane: Option<bool>,
    },
}

/// A settings request the runner should send to glassd (`SetSettings`), to drive the
/// radio/airplane steps and verify the state actually changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Req {
    Wifi(bool),
    Bluetooth(bool),
    Airplane(bool),
}

/// A sound the runner should play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Cue(Cue),
    /// The startup chime (audibility check).
    Chime,
}

/// The outcome of one step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Res {
    Pass,
    Fail,
    Skip,
}

impl Res {
    fn as_str(self) -> &'static str {
        match self {
            Res::Pass => "PASS",
            Res::Fail => "FAIL",
            Res::Skip => "SKIP",
        }
    }
}

/// The guided self-test state machine.
pub struct SelfTest {
    locale: Locale,
    idx: usize,
    elapsed: f32,
    done: bool,
    pass: u32,
    fail: u32,
    skip: u32,
    log: Vec<String>,
    sounds: Vec<Sound>,
    reqs: Vec<Req>,
    /// `touch_precision`: (taps_ok, taps_seen, swipes_ok, swipes_seen, in_swipe_phase).
    tp: (u32, u32, u32, u32, bool),
    /// `on_head`: whether a doff has been seen (waiting for the following don).
    doffed: bool,
    /// `language`: index of the locale currently shown (0..4) and whether the chime/sample
    /// of the current one was emitted.
    lang_shown: u32,
    /// `screen_dim`: whether a dim/off was observed (waiting for the wake).
    dimmed: bool,
    /// Whether the current step's instruction/sounds have been emitted yet.
    entered: bool,
}

impl SelfTest {
    pub fn new(locale: Locale) -> SelfTest {
        let mut s = SelfTest {
            locale,
            idx: 0,
            elapsed: 0.0,
            done: false,
            pass: 0,
            fail: 0,
            skip: 0,
            log: Vec::new(),
            sounds: Vec::new(),
            reqs: Vec::new(),
            tp: (0, 0, 0, 0, false),
            doffed: false,
            lang_shown: 0,
            dimmed: false,
            entered: false,
        };
        s.log.push(format!(
            "BEGIN total={} locale={}",
            Step::ALL.len(),
            s.locale.tag()
        ));
        s.enter();
        s
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// The current step (None once finished).
    pub fn step(&self) -> Option<Step> {
        Step::ALL.get(self.idx).copied()
    }

    /// `OMGB-SELFTEST:` lines produced since the last call.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// Sounds to play since the last call.
    pub fn take_sounds(&mut self) -> Vec<Sound> {
        std::mem::take(&mut self.sounds)
    }

    /// Settings requests (`SetSettings`) the runner should send since the last call.
    pub fn take_reqs(&mut self) -> Vec<Req> {
        std::mem::take(&mut self.reqs)
    }

    /// The instruction card for the current step: (title, body).
    pub fn card(&self) -> (String, String) {
        let Some(step) = self.step() else {
            return (
                self.t(T::SummaryTitle),
                format!("{} {}/{}", self.t(T::SummaryOk), self.pass, Step::ALL.len()),
            );
        };
        let n = self.idx + 1;
        let title = format!("{}/{}  {}", n, Step::ALL.len(), step.name());
        let body = match step {
            Step::Language => {
                // Show one localized sample per language as it cycles.
                let loc = [Locale::It, Locale::En, Locale::Fr, Locale::Es]
                    .get(self.lang_shown.min(3) as usize)
                    .copied()
                    .unwrap_or(Locale::En);
                format!("{}: {}", loc.tag(), sample(loc))
            }
            _ => instruction(step, self.locale).to_string(),
        };
        (title, body)
    }

    /// Called on entering a step: emit its cue/chime (sounds that play immediately).
    fn enter(&mut self) {
        if self.entered || self.done {
            return;
        }
        self.entered = true;
        self.elapsed = 0.0;
        match self.step() {
            Some(Step::AudioTone) => {
                // Play the startup chime + a tick for the audibility check.
                self.sounds.push(Sound::Chime);
                self.sounds.push(Sound::Cue(Cue::Select));
            }
            // Radio steps drive themselves: ask glassd to turn the radio on, then verify
            // the state actually changes (via the Settings echo) and restore it.
            Some(Step::WifiToggle) => self.reqs.push(Req::Wifi(true)),
            Some(Step::BtToggle) => self.reqs.push(Req::Bluetooth(true)),
            Some(Step::Airplane) => self.reqs.push(Req::Airplane(true)),
            _ => {}
        }
    }

    fn finish_step(&mut self, result: Res, got: &str) {
        let Some(step) = self.step() else { return };
        self.log.push(format!(
            "step={} expected={} got={} result={}",
            step.name(),
            step.expected(),
            got,
            result.as_str()
        ));
        match result {
            Res::Pass => {
                self.pass += 1;
                self.sounds.push(Sound::Cue(Cue::Select));
            }
            Res::Fail => {
                self.fail += 1;
                self.sounds.push(Sound::Cue(Cue::ToggleOff));
            }
            Res::Skip => {
                self.skip += 1;
                self.sounds.push(Sound::Cue(Cue::MenuMove));
            }
        }
        // Reset per-step scratch and advance.
        self.tp = (0, 0, 0, 0, false);
        self.doffed = false;
        self.lang_shown = 0;
        self.dimmed = false;
        self.idx += 1;
        self.entered = false;
        if self.idx >= Step::ALL.len() {
            self.done = true;
            self.log.push(format!(
                "SUMMARY pass={} skip={} fail={} total={}",
                self.pass,
                self.skip,
                self.fail,
                Step::ALL.len()
            ));
        } else {
            self.enter();
        }
    }

    /// Advances time by `dt` seconds (drives timeouts and the language cycle).
    pub fn tick(&mut self, dt: f32) {
        if self.done {
            return;
        }
        self.elapsed += dt;
        match self.step() {
            Some(Step::Language) => {
                // Auto-cycle IT→EN→FR→ES, LANG_SHOW_S each, then PASS.
                let want = ((self.elapsed / LANG_SHOW_S) as u32).min(4);
                if want > self.lang_shown {
                    self.lang_shown = want;
                }
                if self.lang_shown >= 4 {
                    self.finish_step(Res::Pass, "locales_shown=4");
                }
            }
            Some(_) if self.elapsed >= STEP_TIMEOUT_S => {
                let got = self.timeout_got();
                self.finish_step(Res::Skip, &got);
            }
            _ => {}
        }
    }

    /// The `got=` value to log when a step times out (captures partial progress).
    fn timeout_got(&self) -> String {
        match self.step() {
            Some(Step::TouchPrecision) => {
                format!("taps_ok={}/3 swipes_ok={}/3 timeout", self.tp.0, self.tp.2)
            }
            Some(Step::OnHead) if self.doffed => "doff_only timeout".into(),
            _ => "timeout".into(),
        }
    }

    /// Feeds one event to the current step.
    pub fn on_event(&mut self, ev: Ev) {
        if self.done {
            return;
        }
        let Some(step) = self.step() else { return };

        // A swipe-down is the universal skip, except where the step uses it itself
        // (nav_swipe_down expects it; audio_tone reads it as "no").
        if let Ev::Gesture(GestureKind::SwipeDown) = ev {
            if !matches!(step, Step::NavSwipeDown | Step::AudioTone) {
                self.finish_step(Res::Skip, "swipe_down");
                return;
            }
        }

        match step {
            Step::NavSwipeFwd => self.expect_gesture(ev, GestureKind::SwipeForward),
            Step::NavSwipeBack => self.expect_gesture(ev, GestureKind::SwipeBackward),
            Step::NavTap => self.expect_gesture(ev, GestureKind::Tap),
            Step::NavSwipeDown => self.expect_gesture(ev, GestureKind::SwipeDown),
            Step::TouchPrecision => self.touch_precision(ev),
            Step::AudioTone => match ev {
                Ev::Gesture(GestureKind::Tap) => self.finish_step(Res::Pass, "tap=heard"),
                Ev::Gesture(GestureKind::SwipeDown) => {
                    self.finish_step(Res::Fail, "down=not_heard")
                }
                _ => {}
            },
            Step::HeadWake => {
                if let Ev::Head = ev {
                    self.finish_step(Res::Pass, "head_up");
                }
            }
            Step::Wink => {
                if let Ev::Wink = ev {
                    self.finish_step(Res::Pass, "wink");
                }
            }
            Step::OnHead => match ev {
                Ev::Wear { on_head: false } => self.doffed = true,
                Ev::Wear { on_head: true } if self.doffed => {
                    self.finish_step(Res::Pass, "doff_then_don")
                }
                _ => {}
            },
            // Radio/airplane: real state verification. On entering the step the machine
            // asked glassd (SetSettings) to turn the radio on; here we confirm glassd
            // echoed the new state, then restore it off.
            Step::WifiToggle => {
                if let Ev::Settings {
                    wifi: Some(true), ..
                } = ev
                {
                    self.reqs.push(Req::Wifi(false));
                    self.finish_step(Res::Pass, "wifi=on_verified");
                }
            }
            Step::BtToggle => {
                if let Ev::Settings { bt: Some(true), .. } = ev {
                    self.reqs.push(Req::Bluetooth(false));
                    self.finish_step(Res::Pass, "bt=on_verified");
                }
            }
            Step::Airplane => {
                if let Ev::Settings {
                    airplane: Some(true),
                    ..
                } = ev
                {
                    self.reqs.push(Req::Airplane(false));
                    self.finish_step(Res::Pass, "airplane=on_verified");
                }
            }
            Step::Language => {} // time-driven only
            Step::ScreenDim => match ev {
                Ev::Screen { lit: false } => self.dimmed = true,
                Ev::Screen { lit: true } if self.dimmed => {
                    self.finish_step(Res::Pass, "dim_then_wake")
                }
                Ev::Gesture(GestureKind::Tap) if self.dimmed => {
                    self.finish_step(Res::Pass, "dim_then_tap")
                }
                _ => {}
            },
        }
    }

    fn expect_gesture(&mut self, ev: Ev, want: GestureKind) {
        if let Ev::Gesture(k) = ev {
            if k == want {
                self.finish_step(Res::Pass, gesture_name(k));
            } else {
                // A different gesture: report what arrived, but keep waiting (do not fail
                // on a stray classification; the timeout will skip if it never comes).
                self.log
                    .push(format!("  (got {} while waiting)", gesture_name(k)));
            }
        }
    }

    fn touch_precision(&mut self, ev: Ev) {
        let Ev::Gesture(k) = ev else { return };
        let is_swipe = matches!(k, GestureKind::SwipeForward | GestureKind::SwipeBackward);
        let is_tap = matches!(k, GestureKind::Tap);
        if !self.tp.4 {
            // Tap phase: expect 3 taps.
            if is_tap {
                self.tp.0 += 1;
            }
            self.tp.1 += 1;
            if self.tp.1 >= 3 {
                self.tp.4 = true; // move to swipe phase
            }
        } else {
            // Swipe phase: expect 3 swipes.
            if is_swipe {
                self.tp.2 += 1;
            }
            self.tp.3 += 1;
            if self.tp.3 >= 3 {
                let got = format!("taps_ok={}/3 swipes_ok={}/3", self.tp.0, self.tp.2);
                let res = if self.tp.0 >= 2 && self.tp.2 >= 2 {
                    Res::Pass
                } else {
                    Res::Fail
                };
                self.finish_step(res, &got);
            }
        }
    }

    fn t(&self, key: T) -> String {
        ui_text(key, self.locale).to_string()
    }
}

fn gesture_name(k: GestureKind) -> &'static str {
    match k {
        GestureKind::Tap => "tap",
        GestureKind::LongPress => "long_press",
        GestureKind::SwipeForward => "swipe_forward",
        GestureKind::SwipeBackward => "swipe_backward",
        GestureKind::SwipeDown => "swipe_down",
        GestureKind::SwipeUp => "swipe_up",
    }
}

// ---- localized text ----------------------------------------------------------------

enum T {
    SummaryTitle,
    SummaryOk,
}

fn ui_text(key: T, locale: Locale) -> &'static str {
    match (key, locale) {
        (T::SummaryTitle, Locale::It) => "Self-test finito",
        (T::SummaryTitle, _) => "Self-test done",
        (T::SummaryOk, Locale::It) => "OK:",
        (T::SummaryOk, _) => "OK:",
    }
}

/// A short sample sentence per language, to verify i18n rendering in the `language` step.
fn sample(locale: Locale) -> &'static str {
    match locale {
        Locale::It => "Ciao, questo è il test della lingua.",
        Locale::En => "Hello, this is the language test.",
        Locale::Fr => "Bonjour, ceci est le test de langue.",
        Locale::Es => "Hola, esta es la prueba de idioma.",
    }
}

/// The per-step instruction, in Italian and English (other locales fall back to English).
fn instruction(step: Step, locale: Locale) -> &'static str {
    let it = locale == Locale::It;
    match step {
        Step::NavSwipeFwd => {
            if it {
                "Scorri avanti"
            } else {
                "Swipe forward"
            }
        }
        Step::NavSwipeBack => {
            if it {
                "Scorri indietro"
            } else {
                "Swipe backward"
            }
        }
        Step::NavTap => {
            if it {
                "Fai tap"
            } else {
                "Tap"
            }
        }
        Step::NavSwipeDown => {
            if it {
                "Scorri giù"
            } else {
                "Swipe down"
            }
        }
        Step::TouchPrecision => {
            if it {
                "Fai 3 TAP puliti, poi 3 SCORRIMENTI"
            } else {
                "3 clean TAPS, then 3 SWIPES"
            }
        }
        Step::AudioTone => {
            if it {
                "Hai sentito il tono? Tap=sì, giù=no"
            } else {
                "Heard the tone? Tap=yes, down=no"
            }
        }
        Step::HeadWake => {
            if it {
                "Alza la testa"
            } else {
                "Raise your head"
            }
        }
        Step::Wink => {
            if it {
                "Strizza l'occhio"
            } else {
                "Wink"
            }
        }
        Step::OnHead => {
            if it {
                "Togli e rimetti gli occhiali"
            } else {
                "Take the glasses off, then on"
            }
        }
        Step::WifiToggle => {
            if it {
                "Verifico il WiFi (automatico)..."
            } else {
                "Verifying Wi-Fi (automatic)..."
            }
        }
        Step::BtToggle => {
            if it {
                "Verifico il Bluetooth (automatico)..."
            } else {
                "Verifying Bluetooth (automatic)..."
            }
        }
        Step::Airplane => {
            if it {
                "Verifico modalità aereo (automatico)..."
            } else {
                "Verifying airplane mode (automatic)..."
            }
        }
        Step::Language => "", // body built in card() (cycles locales)
        Step::ScreenDim => {
            if it {
                "Aspetta che lo schermo si abbassi, poi fai tap"
            } else {
                "Wait for the screen to dim, then tap"
            }
        }
    }
}

//! "Take a note": free dictation through glassd's relay to the phone (glass-services
//! `docs/api.md`, "Dictation"), as a pure state machine behind a full-screen card.
//!
//! `DictationStart{purpose: "free"}` → `STATE_LISTENING` ("Speak now", then the partial
//! transcript) → the final `Transcript` and `STATE_DONE` → the note becomes a text card in
//! the timeline. `SWIPE_DOWN` cancels while listening (`DictationCancel`). Errors (the
//! `ERROR_CODE_NO_COMPANION` or `ERROR_CODE_UNSUPPORTED` answer, or `STATE_FAILED`) show a
//! short card that `TAP` or `SWIPE_DOWN` closes.

use glass_api::proto::{dictation_state, DictationState, ErrorCode, Transcript};
use glass_ui::{CardContent, GestureKind, InputEvent};

/// The card that opens dictation.
pub const NOTE_CARD: glass_ui::CardId = 120;

/// The source shown in a note card's footer.
pub const NOTE_SOURCE: &str = "Note";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Sent, nothing heard back yet.
    Starting,
    Listening,
    Processing,
    Failed(String),
}

/// What the app must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictationOut {
    /// Send `DictationCancel` for this request and close.
    Cancel(u32),
    /// Close the dictation screen.
    Close,
    /// The final text: add a note card and close.
    Note(String),
}

#[derive(Clone, Debug)]
pub struct Dictation {
    request_id: u32,
    phase: Phase,
    text: String,
}

/// A short message for a failure text from glassd or the phone.
pub fn failure_text(error: &str) -> String {
    match error {
        "no-speech" => "Nothing was heard".into(),
        "link-lost" => "The phone link was lost".into(),
        "expired" => "That took too long".into(),
        "replaced" => "Another note was started".into(),
        "" => "Dictation failed".into(),
        e => e.to_string(),
    }
}

impl Dictation {
    pub fn new(request_id: u32) -> Dictation {
        Dictation {
            request_id,
            phase: Phase::Starting,
            text: String::new(),
        }
    }

    pub fn request_id(&self) -> u32 {
        self.request_id
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Whether the request is still open at glassd (a close must cancel it).
    pub fn is_open(&self) -> bool {
        !matches!(self.phase, Phase::Failed(_))
    }

    /// glassd refused the `DictationStart`.
    pub fn refused(&mut self, code: ErrorCode) {
        self.phase = Phase::Failed(match code {
            ErrorCode::NoCompanion => "The phone is not connected".into(),
            ErrorCode::Unsupported => "The phone app cannot take notes".into(),
            _ => "Dictation is not available".into(),
        });
    }

    /// A `DictationState` for this request.
    pub fn on_state(&mut self, s: &DictationState) -> Option<DictationOut> {
        if s.request_id != self.request_id || !self.is_open() {
            return None;
        }
        use dictation_state::State as St;
        match St::try_from(s.state).unwrap_or(St::Unspecified) {
            St::Listening => self.phase = Phase::Listening,
            St::Processing => self.phase = Phase::Processing,
            St::Failed => self.phase = Phase::Failed(failure_text(&s.error)),
            St::Done => {
                let text = self.text.trim().to_string();
                if text.is_empty() {
                    self.phase = Phase::Failed(failure_text("no-speech"));
                } else {
                    // The request ended at glassd: nothing to cancel.
                    self.phase = Phase::Failed(String::new());
                    return Some(DictationOut::Note(text));
                }
            }
            St::Sent | St::Unspecified => {}
        }
        None
    }

    /// A `Transcript` for this request (partial or final).
    pub fn on_transcript(&mut self, t: &Transcript) {
        if t.request_id == self.request_id && self.is_open() {
            self.text = t.text.clone();
        }
    }

    /// A gesture while the dictation screen is shown.
    pub fn input(&mut self, event: &InputEvent) -> Option<DictationOut> {
        let InputEvent::Gesture { kind, .. } = *event else {
            return None;
        };
        match (kind, self.is_open()) {
            (GestureKind::SwipeDown, true) => Some(DictationOut::Cancel(self.request_id)),
            (GestureKind::SwipeDown | GestureKind::Tap, false) => Some(DictationOut::Close),
            _ => None,
        }
    }

    /// The card shown.
    pub fn card(&self, now_s: i64) -> CardContent {
        match &self.phase {
            Phase::Failed(msg) => CardContent::Info {
                title: "Note not taken".into(),
                lines: vec![msg.clone(), "Tap to close".into()],
            },
            p => {
                let body = if !self.text.is_empty() {
                    self.text.clone()
                } else if *p == Phase::Processing {
                    "…".into()
                } else {
                    "Speak now".into()
                };
                CardContent::Text {
                    body,
                    source: match p {
                        Phase::Starting => "Connecting",
                        Phase::Listening => "Listening",
                        _ => "Processing",
                    }
                    .into(),
                    timestamp_s: now_s,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: u32, s: dictation_state::State, e: &str) -> DictationState {
        DictationState {
            request_id: id,
            state: s as i32,
            error: e.into(),
        }
    }

    fn tr(id: u32, text: &str, fin: bool) -> Transcript {
        Transcript {
            request_id: id,
            text: text.into(),
            r#final: fin,
        }
    }

    #[test]
    fn a_note_from_listening_to_done() {
        use dictation_state::State as S;
        let mut d = Dictation::new(7);
        assert!(d.on_state(&st(8, S::Listening, "")).is_none());
        assert_eq!(d.phase(), &Phase::Starting);
        d.on_state(&st(7, S::Listening, ""));
        assert_eq!(d.phase(), &Phase::Listening);
        d.on_transcript(&tr(7, "buy", false));
        match d.card(0) {
            CardContent::Text { body, source, .. } => {
                assert_eq!(body, "buy");
                assert_eq!(source, "Listening");
            }
            c => panic!("{c:?}"),
        }
        d.on_transcript(&tr(7, "buy milk", true));
        assert_eq!(
            d.on_state(&st(7, S::Done, "")),
            Some(DictationOut::Note("buy milk".into()))
        );
        assert!(!d.is_open());
    }

    #[test]
    fn failures_and_cancel() {
        use dictation_state::State as S;
        let mut d = Dictation::new(1);
        assert_eq!(
            d.input(&InputEvent::gesture(GestureKind::SwipeDown)),
            Some(DictationOut::Cancel(1))
        );
        d.on_state(&st(1, S::Failed, "no-speech"));
        assert_eq!(d.phase(), &Phase::Failed("Nothing was heard".into()));
        assert_eq!(
            d.input(&InputEvent::gesture(GestureKind::Tap)),
            Some(DictationOut::Close)
        );
        let mut d = Dictation::new(2);
        d.refused(ErrorCode::NoCompanion);
        assert!(!d.is_open());
        assert_eq!(
            d.input(&InputEvent::gesture(GestureKind::SwipeDown)),
            Some(DictationOut::Close)
        );
        // An empty final transcript is not a note.
        let mut d = Dictation::new(3);
        d.on_transcript(&tr(3, "  ", true));
        assert!(d.on_state(&st(3, S::Done, "")).is_none());
        assert!(matches!(d.phase(), Phase::Failed(_)));
    }
}

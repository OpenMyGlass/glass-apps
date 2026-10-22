//! "Take a note" against glass-api's in-process mock of glassd, with a test client
//! registered as the companion link provider (glass-services api.md, "Companion link
//! provider" and "Dictation").

use glass_api::proto::{
    client_frame::Body as C, dictation_state, service_frame::Body, CompanionRegister,
    CompanionStatus, DictationState, Gesture, GestureKind as PKind, Transcript,
};
use glass_api::{Client, MockService};
use glass_launcher::dictation::NOTE_CARD;
use glass_launcher::glassd::Session;
use glass_launcher::{demo, App, OverlayKind};
use glass_ui::{CardContent, RenderContext};
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(5);

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
    }
}

fn gesture(kind: PKind) -> Body {
    Body::Gesture(Gesture {
        kind: kind as i32,
        fingers: 1,
        velocity: 0.0,
        timestamp_us: 1,
    })
}

fn pump_until(s: &mut Session, app: &mut App, what: &str, pred: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + T;
    while !pred(app) {
        assert!(Instant::now() < deadline, "not reached: {what}");
        s.pump(app, Some(Duration::from_millis(20))).unwrap();
    }
}

fn provider(m: &MockService, caps: &[&str]) -> Client {
    let mut p = Client::connect_as(m.path(), "test-provider").unwrap();
    p.send(C::CompanionRegister(CompanionRegister {
        name: "test".into(),
    }))
    .unwrap();
    p.send(C::CompanionStatus(CompanionStatus {
        phone_connected: true,
        phone_capabilities: caps.iter().map(|s| s.to_string()).collect(),
        phone_name: "Pixel".into(),
    }))
    .unwrap();
    p
}

fn launcher(m: &MockService) -> (Session, App) {
    let mut a = App::new(demo::timeline(), ctx());
    a.follow_glassd();
    let mut s = Session::connect(m.path()).unwrap();
    a.set_service(Some(s.service_info()));
    pump_until(&mut s, &mut a, "focus", |_| true);
    (s, a)
}

/// Swipes backward until the note card is shown, then taps it.
fn open_note(m: &MockService, s: &mut Session, a: &mut App) {
    for _ in 0..4 {
        if a.timeline().current_card().id == NOTE_CARD {
            break;
        }
        m.publish_body(gesture(PKind::SwipeBackward));
        let before = a.timeline().current_index();
        pump_until(s, a, "swipe", |a| a.timeline().current_index() != before);
        for _ in 0..60 {
            a.step(1.0 / 30.0, ctx());
        }
    }
    assert_eq!(a.timeline().current_card().id, NOTE_CARD);
    m.publish_body(gesture(PKind::Tap));
    pump_until(s, a, "dictation open", |a| {
        a.overlay() == Some(OverlayKind::Dictation)
    });
}

fn next_start(p: &mut Client) -> u32 {
    let deadline = Instant::now() + T;
    loop {
        assert!(Instant::now() < deadline, "no provider_dictation_start");
        if let Some(f) = p.recv(Some(Duration::from_millis(50))).unwrap() {
            if let Some(Body::ProviderDictationStart(d)) = f.body {
                assert_eq!(d.purpose, "free");
                return d.request_id;
            }
        }
    }
}

#[test]
fn a_note_is_dictated_through_the_provider() {
    let m = MockService::start().unwrap();
    let mut p = provider(&m, &["dictation-free"]);
    let (mut s, mut a) = launcher(&m);
    pump_until(&mut s, &mut a, "companion", |a| a.companion().connected);
    open_note(&m, &mut s, &mut a);
    // The pump sends the queued DictationStart.
    s.pump(&mut a, Some(Duration::from_millis(20))).unwrap();
    let link = next_start(&mut p);
    p.send(C::ProviderDictationState(DictationState {
        request_id: link,
        state: dictation_state::State::Listening as i32,
        error: String::new(),
    }))
    .unwrap();
    p.send(C::ProviderTranscript(Transcript {
        request_id: link,
        text: "buy milk".into(),
        r#final: true,
    }))
    .unwrap();
    pump_until(&mut s, &mut a, "note card", |a| {
        a.timeline().cards().iter().any(|c| {
            matches!(&c.content, CardContent::Text { body, source, .. }
                if body == "buy milk" && source == "Note")
        })
    });
    assert_eq!(a.overlay(), None);
}

#[test]
fn without_free_dictation_the_note_fails_and_swipe_down_closes() {
    let m = MockService::start().unwrap();
    let _p = provider(&m, &[]);
    let (mut s, mut a) = launcher(&m);
    pump_until(&mut s, &mut a, "companion", |a| a.companion().connected);
    open_note(&m, &mut s, &mut a);
    // glassd answers ERROR_CODE_UNSUPPORTED to the start.
    let deadline = Instant::now() + T;
    loop {
        s.pump(&mut a, Some(Duration::from_millis(20))).unwrap();
        let mut probe = s.take_log();
        probe.retain(|l| l.contains("ERROR_CODE_UNSUPPORTED"));
        if !probe.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "no UNSUPPORTED answer");
    }
    assert_eq!(a.overlay(), Some(OverlayKind::Dictation));
    m.publish_body(gesture(PKind::SwipeDown));
    pump_until(&mut s, &mut a, "closed", |a| a.overlay().is_none());
}

#[test]
fn cancel_while_listening_reaches_the_provider() {
    let m = MockService::start().unwrap();
    let mut p = provider(&m, &["dictation-free"]);
    let (mut s, mut a) = launcher(&m);
    pump_until(&mut s, &mut a, "companion", |a| a.companion().connected);
    open_note(&m, &mut s, &mut a);
    s.pump(&mut a, Some(Duration::from_millis(20))).unwrap();
    let link = next_start(&mut p);
    m.publish_body(gesture(PKind::SwipeDown));
    pump_until(&mut s, &mut a, "closed", |a| a.overlay().is_none());
    s.pump(&mut a, Some(Duration::from_millis(20))).unwrap();
    let deadline = Instant::now() + T;
    loop {
        assert!(Instant::now() < deadline, "no provider_dictation_cancel");
        if let Some(f) = p.recv(Some(Duration::from_millis(50))).unwrap() {
            if let Some(Body::ProviderDictationCancel(c)) = f.body {
                assert_eq!(c.request_id, link);
                break;
            }
        }
    }
}

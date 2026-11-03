//! UI audio feedback: the cues the launcher plays on navigation. Uses a recording
//! player (the `sound` feature and real ALSA are not needed for this), so it runs in the
//! ordinary host/CI test pass.

use glass_launcher::feedback::{Cue, Feedback};
use glass_launcher::platform::{MockPlatform, RadioStatus};
use glass_launcher::settings::ids;
use glass_launcher::{demo, App};
use glass_ui::{CardId, GestureKind, InputEvent, RenderContext};
use std::sync::{Arc, Mutex};

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
        locale: glass_ui::Locale::En,
        time_24h: true,
        battery_percent: None,
    }
}

/// A [`Feedback`] that records every cue, shared so the test can read it back.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Cue>>>);

impl Recorder {
    fn cues(&self) -> Vec<Cue> {
        self.0.lock().unwrap().clone()
    }
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl Feedback for Recorder {
    fn play(&mut self, cue: Cue) {
        self.0.lock().unwrap().push(cue);
    }
}

fn app_with(rec: &Recorder) -> App {
    let mut a = App::new(demo::timeline(), ctx());
    a.set_feedback(Box::new(rec.clone()));
    a.render_if_changed().unwrap();
    a
}

fn g(app: &mut App, kind: GestureKind) {
    app.input(&InputEvent::gesture(kind));
    app.step(1.0, ctx());
    app.render_if_changed().unwrap();
}

fn goto(app: &mut App, id: CardId) {
    if glass_launcher::settings::ORDER.contains(&id) && !app.settings_open() {
        goto(app, ids::ROOT);
        g(app, GestureKind::Tap);
    }
    for _ in 0..30 {
        let t = app.timeline();
        let want = t.cards().iter().position(|c| c.id == id).unwrap();
        let at = t.current_index();
        if at == want {
            return;
        }
        let kind = if want > at {
            GestureKind::SwipeForward
        } else {
            GestureKind::SwipeBackward
        };
        g(app, kind);
    }
    panic!("card {id} not reached");
}

#[test]
fn a_swipe_that_moves_a_card_ticks() {
    let rec = Recorder::default();
    let mut a = app_with(&rec);
    rec.clear();
    g(&mut a, GestureKind::SwipeForward);
    assert!(
        rec.cues().contains(&Cue::CardChange),
        "expected a CardChange tick, got {:?}",
        rec.cues()
    );
}

#[test]
fn tapping_a_card_blips() {
    let rec = Recorder::default();
    let mut a = app_with(&rec);
    goto(&mut a, ids::WIFI);
    rec.clear();
    g(&mut a, GestureKind::Tap);
    assert_eq!(rec.cues(), vec![Cue::Select]);
}

#[test]
fn toggling_a_radio_plays_a_distinct_tick() {
    let rec = Recorder::default();
    let mut a = App::new(demo::timeline(), ctx()).with_platform(Box::new(MockPlatform::new(
        RadioStatus::on(&["HomeNet"]),
        RadioStatus::on(&["Headset"]),
    )));
    a.set_feedback(Box::new(rec.clone()));
    a.render_if_changed().unwrap();

    goto(&mut a, ids::WIFI);
    g(&mut a, GestureKind::Tap); // open the Wi-Fi menu ("Turn off")
    rec.clear();
    g(&mut a, GestureKind::Tap); // choose "Turn off"
    assert_eq!(rec.cues(), vec![Cue::ToggleOff]);

    g(&mut a, GestureKind::Tap); // open the menu again ("Turn on")
    rec.clear();
    g(&mut a, GestureKind::Tap); // choose "Turn on"
    assert_eq!(rec.cues(), vec![Cue::ToggleOn]);
}

#[test]
fn a_resting_or_dragging_finger_is_silent() {
    // Regression for the "continuous buzz on touch": feedback must be edge-triggered on a
    // committed card change, never on the live touch/drag position.
    let rec = Recorder::default();
    let mut a = app_with(&rec);
    rec.clear();

    // A finger resting on the pad: many Scroll frames, no movement, never ended.
    for i in 0..12u64 {
        a.input(&InputEvent::Scroll {
            displacement: 0.1,
            delta: 0.0,
            velocity: 0.0,
            fingers: 1,
            ended: false,
            cancelled: false,
            timestamp_us: i * 16_000,
        });
        a.step(0.016, ctx());
    }
    assert!(
        rec.cues().is_empty(),
        "a resting finger must be silent, got {:?}",
        rec.cues()
    );

    // A finger dragging across several cards: still silent until the touch ends.
    for i in 0..12u64 {
        a.input(&InputEvent::Scroll {
            displacement: 0.25 * (i as f32 + 1.0),
            delta: 0.25,
            velocity: 2.0,
            fingers: 1,
            ended: false,
            cancelled: false,
            timestamp_us: 200_000 + i * 16_000,
        });
        a.step(0.016, ctx());
    }
    assert!(
        rec.cues().is_empty(),
        "an in-progress drag must be silent (no per-delta ticks), got {:?}",
        rec.cues()
    );
}

#[test]
fn the_default_player_is_silent() {
    // An app with no installed player makes no sound and does not panic on navigation.
    let mut a = App::new(demo::timeline(), ctx());
    a.render_if_changed().unwrap();
    g(&mut a, GestureKind::SwipeForward);
    g(&mut a, GestureKind::Tap);
}

#[test]
fn changing_the_volume_plays_a_blip_at_the_new_level() {
    let rec = Recorder::default();
    let mut a = app_with(&rec);
    goto(&mut a, ids::VOLUME);
    g(&mut a, GestureKind::Tap); // the volume screen
    rec.clear();
    g(&mut a, GestureKind::SwipeBackward);
    assert_eq!(rec.cues(), vec![Cue::Select]);
    assert!(a.overlay().is_some(), "the screen stays open");
}

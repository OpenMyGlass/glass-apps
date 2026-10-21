use glass_launcher::{demo, App};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext};

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
    }
}

fn asleep() -> App {
    let mut app = App::new(demo::timeline(), ctx());
    assert_eq!(
        app.input(&InputEvent::gesture(GestureKind::SwipeDown)),
        Some(Action::RequestSleep)
    );
    assert!(!app.screen_on());
    app
}

fn scroll(d: f32, ended: bool) -> InputEvent {
    InputEvent::Scroll {
        displacement: d,
        delta: 0.0,
        velocity: 1.0,
        fingers: 1,
        ended,
        cancelled: false,
        timestamp_us: 0,
    }
}

#[test]
fn tap_and_horizontal_swipes_wake_and_are_consumed() {
    for kind in [
        GestureKind::Tap,
        GestureKind::SwipeForward,
        GestureKind::SwipeBackward,
    ] {
        let mut app = asleep();
        assert_eq!(app.input(&InputEvent::gesture(kind)), None);
        assert!(app.screen_on(), "{kind:?} wakes");
        let t = app.timeline();
        assert_eq!(t.current_index(), t.home_index(), "{kind:?} did not move");
    }
}

#[test]
fn other_touches_do_not_wake() {
    for kind in [
        GestureKind::SwipeDown,
        GestureKind::SwipeUp,
        GestureKind::LongPress,
    ] {
        let mut app = asleep();
        assert_eq!(app.input(&InputEvent::gesture(kind)), None);
        assert!(!app.screen_on(), "{kind:?} keeps the screen off");
    }
}

#[test]
fn a_waking_swipe_with_scroll_frames_moves_nothing() {
    let mut app = asleep();
    app.input(&scroll(0.2, false));
    app.input(&scroll(0.4, true));
    assert!(!app.screen_on(), "the scroll alone does not wake");
    app.input(&InputEvent::gesture(GestureKind::SwipeForward));
    assert!(app.screen_on());
    for _ in 0..60 {
        app.step(1.0 / 60.0, ctx());
    }
    let t = app.timeline();
    assert_eq!(t.current_index(), t.home_index());
    // The next touch is delivered.
    app.input(&InputEvent::gesture(GestureKind::SwipeForward));
    for _ in 0..60 {
        app.step(1.0 / 60.0, ctx());
    }
    let t = app.timeline();
    assert_eq!(t.current_index(), t.home_index() + 1);
}

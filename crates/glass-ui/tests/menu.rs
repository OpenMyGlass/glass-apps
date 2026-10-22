use glass_gfx::{Compositor, SoftwareCompositor};
use glass_ui::{Adjuster, GestureKind, InputEvent, Menu, MenuItem, Outcome};

fn g(kind: GestureKind) -> InputEvent {
    InputEvent::gesture(kind)
}

fn menu() -> Menu<&'static str> {
    Menu::new(vec![
        MenuItem::new("on", "Turn on"),
        MenuItem::new("angle", "Wake angle").with_detail("30°"),
        MenuItem::new("off", "Turn off"),
    ])
    .unwrap()
}

fn settle<T: Clone>(m: &mut Menu<T>) {
    for _ in 0..60 {
        m.tick(1.0 / 60.0);
    }
    assert!(m.is_settled());
}

#[test]
fn a_menu_without_items_is_none() {
    assert!(Menu::<u8>::new(vec![]).is_none());
}

#[test]
fn swipes_move_through_items_and_tap_selects() {
    let mut m = menu();
    assert_eq!(m.index(), 0);
    assert_eq!(m.handle(&g(GestureKind::SwipeForward)), None);
    settle(&mut m);
    assert_eq!(m.index(), 1);
    m.handle(&g(GestureKind::SwipeForward));
    m.handle(&g(GestureKind::SwipeForward)); // past the end: stays on the last
    settle(&mut m);
    assert_eq!(m.index(), 2);
    m.handle(&g(GestureKind::SwipeBackward));
    settle(&mut m);
    assert_eq!(
        m.handle(&g(GestureKind::Tap)),
        Some(Outcome::Selected("angle"))
    );
}

#[test]
fn swipe_down_closes_and_other_gestures_do_nothing() {
    let mut m = menu();
    assert_eq!(m.handle(&g(GestureKind::LongPress)), None);
    assert_eq!(m.handle(&g(GestureKind::SwipeUp)), None);
    let two = InputEvent::Gesture {
        kind: GestureKind::Tap,
        fingers: 2,
        velocity: 0.0,
        timestamp_us: 0,
    };
    assert_eq!(m.handle(&two), None, "only one-finger gestures act");
    assert_eq!(m.handle(&g(GestureKind::SwipeDown)), Some(Outcome::Closed));
}

#[test]
fn scroll_follows_the_finger_through_items() {
    let mut m = menu();
    let scroll = |d: f32, ended: bool| InputEvent::Scroll {
        displacement: d,
        delta: 0.0,
        velocity: 0.0,
        fingers: 1,
        ended,
        cancelled: false,
        timestamp_us: 0,
    };
    m.handle(&scroll(0.4, false));
    assert!(m.strip().position() > 0.5);
    m.handle(&scroll(0.4, true));
    settle(&mut m);
    assert_eq!(m.index(), 1);
}

#[test]
fn select_index_shows_an_item_at_once() {
    let mut m = menu();
    m.select_index(2);
    assert!(m.is_settled());
    assert_eq!(m.index(), 2);
    m.select_index(9);
    assert_eq!(m.index(), 2, "clamped");
}

#[test]
fn rendering_uses_the_compositor_and_release_frees_layers() {
    let mut c = SoftwareCompositor::glass();
    let mut m = menu();
    m.render(&mut c).unwrap();
    assert!(c.layer_count() >= 1);
    m.release(&mut c);
    assert_eq!(c.layer_count(), 0);
    assert!(m.needs_redraw(), "rendered again after a release");
}

fn degrees(v: f32) -> String {
    format!("{v:.0}°")
}

#[test]
fn adjuster_steps_within_its_range() {
    let mut a = Adjuster::new("Wake angle", 10.0, 40.0, 5.0, 30.0, degrees);
    a.handle(&g(GestureKind::SwipeForward));
    assert_eq!(a.value(), 35.0);
    for _ in 0..5 {
        a.handle(&g(GestureKind::SwipeForward));
    }
    assert_eq!(a.value(), 40.0, "clamped at the top");
    for _ in 0..10 {
        a.handle(&g(GestureKind::SwipeBackward));
    }
    assert_eq!(a.value(), 10.0, "clamped at the bottom");
    assert_eq!(
        a.handle(&g(GestureKind::Tap)),
        Some(Outcome::Selected(10.0))
    );
    assert_eq!(a.handle(&g(GestureKind::SwipeDown)), Some(Outcome::Closed));
    let b = Adjuster::new("x", 10.0, 40.0, 5.0, 99.0, degrees);
    assert_eq!(b.value(), 40.0, "the start value is clamped");
}

#[test]
fn adjuster_redraws_only_when_the_value_changes() {
    let mut c = SoftwareCompositor::glass();
    let mut a = Adjuster::new("Wake angle", 10.0, 40.0, 5.0, 40.0, degrees);
    a.render(&mut c).unwrap();
    assert!(!a.needs_redraw());
    a.handle(&g(GestureKind::SwipeForward)); // already at the top
    assert!(!a.needs_redraw());
    a.handle(&g(GestureKind::SwipeBackward));
    assert!(a.needs_redraw());
    a.release(&mut c);
    assert_eq!(c.layer_count(), 0);
}

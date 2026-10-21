use glass_gfx::Frame;
use glass_ui::{
    Action, Card, CardContent, GestureKind, InputEvent, RenderContext, Timeline,
    CARDS_PER_PAD_WIDTH,
};

fn text(id: u64, body: &str) -> Card {
    Card::new(
        id,
        CardContent::Text {
            body: body.into(),
            source: "Test".into(),
            timestamp_s: 1_000,
        },
    )
}

/// Layout: [settings(1), now(2), HOME(10), h1(11), h2(12), h3(13)]
fn timeline() -> Timeline {
    Timeline::new(
        vec![
            text(2, "now"),
            Card::new(1, CardContent::settings("Settings")),
        ],
        Card::new(10, CardContent::Home),
        vec![text(11, "newest"), text(12, "older"), text(13, "oldest")],
    )
}

fn ctx() -> RenderContext {
    RenderContext {
        now_s: 1_060,
        hour: 10,
        minute: 42,
    }
}

fn gesture(kind: GestureKind) -> InputEvent {
    InputEvent::gesture(kind)
}

fn scroll(displacement: f32, velocity: f32, ended: bool) -> InputEvent {
    InputEvent::Scroll {
        displacement,
        delta: 0.0,
        velocity,
        fingers: 1,
        ended,
        cancelled: false,
        timestamp_us: 0,
    }
}

fn settle(t: &mut Timeline) {
    for _ in 0..200 {
        t.tick(1.0 / 60.0);
    }
    assert!(t.is_settled());
}

fn current_id(t: &Timeline) -> u64 {
    t.current_card().id
}

#[test]
fn starts_on_home_in_the_middle() {
    let t = timeline();
    assert_eq!(t.len(), 6);
    assert_eq!(t.home_index(), 2);
    assert_eq!(t.current_index(), 2);
    assert_eq!(current_id(&t), 10);
    let ids: Vec<u64> = t.cards().iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![1, 2, 10, 11, 12, 13]);
    assert!(t.is_settled());
}

#[test]
fn swipe_forward_goes_to_older_cards_and_backward_to_settings() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 11);
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 12);
    for _ in 0..3 {
        t.handle(&gesture(GestureKind::SwipeBackward));
        settle(&mut t);
    }
    assert_eq!(current_id(&t), 2);
    t.handle(&gesture(GestureKind::SwipeBackward));
    settle(&mut t);
    assert_eq!(current_id(&t), 1);
}

#[test]
fn swipes_stop_at_both_ends() {
    let mut t = timeline();
    for _ in 0..10 {
        t.handle(&gesture(GestureKind::SwipeForward));
        settle(&mut t);
    }
    assert_eq!(current_id(&t), 13);
    for _ in 0..10 {
        t.handle(&gesture(GestureKind::SwipeBackward));
        settle(&mut t);
    }
    assert_eq!(current_id(&t), 1);
    assert_eq!(t.position(), 0.0);
}

#[test]
fn swipes_during_an_animation_accumulate() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    t.tick(0.02);
    t.handle(&gesture(GestureKind::SwipeForward));
    assert_eq!(t.current_index(), 4, "the target counts both swipes");
    settle(&mut t);
    assert_eq!(current_id(&t), 12);
}

#[test]
fn animation_moves_monotonically_and_lands_exactly() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    let mut last = t.position();
    let mut steps = 0;
    while !t.is_settled() {
        t.tick(1.0 / 60.0);
        let p = t.position();
        assert!(p >= last, "position goes forward only");
        last = p;
        steps += 1;
        assert!(steps < 100, "settles in under 100 frames");
    }
    assert_eq!(t.position(), 3.0);
    assert!(steps > 3, "animated over several frames, not a jump");
}

#[test]
fn tick_is_deterministic_and_frame_rate_independent_at_the_end() {
    let run = |dt: f32| {
        let mut t = timeline();
        t.handle(&gesture(GestureKind::SwipeForward));
        let mut trace = Vec::new();
        for _ in 0..(1.0 / dt) as usize {
            t.tick(dt);
            trace.push(t.position());
        }
        trace
    };
    assert_eq!(run(1.0 / 60.0), run(1.0 / 60.0));
    assert_eq!(*run(1.0 / 30.0).last().unwrap(), 3.0);
    assert_eq!(*run(1.0 / 60.0).last().unwrap(), 3.0);
}

#[test]
fn scroll_follows_the_finger_live() {
    let mut t = timeline();
    t.handle(&scroll(0.1, 0.0, false));
    let expected = 2.0 + 0.1 * CARDS_PER_PAD_WIDTH;
    assert!((t.position() - expected).abs() < 1e-5);
    t.handle(&scroll(-0.1, 0.0, false));
    assert!((t.position() - (2.0 - 0.1 * CARDS_PER_PAD_WIDTH)).abs() < 1e-5);
    assert!(!t.is_settled());
}

#[test]
fn slow_release_settles_on_the_nearest_card() {
    let mut t = timeline();
    let d = 0.4 / CARDS_PER_PAD_WIDTH; // 0.4 card
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 0.0, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 10, "back to home");

    let d = 0.7 / CARDS_PER_PAD_WIDTH; // 0.7 card
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 0.0, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 11);
}

#[test]
fn fling_moves_at_least_one_card_in_its_direction() {
    let mut t = timeline();
    let d = 0.1 / CARDS_PER_PAD_WIDTH; // a tenth of a card
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 1.5, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 11);
}

#[test]
fn fast_fling_can_cross_several_cards_but_not_the_end() {
    let mut t = timeline();
    t.handle(&scroll(0.3, 0.0, false));
    t.handle(&scroll(0.3, 40.0, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 13, "clamped at the oldest card");
}

#[test]
fn swipe_right_after_a_scroll_is_not_counted_twice() {
    // A real swipe: Scroll frames, a final Scroll{ended}, then SWIPE_FORWARD.
    let mut t = timeline();
    let d = 0.6 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 0.8, true));
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 11, "one card, not two");
}

#[test]
fn swipe_after_a_short_scroll_still_moves_one_card() {
    let mut t = timeline();
    let d = 0.2 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 0.0, true));
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 11);
}

#[test]
fn cancelled_scroll_returns_to_where_it_started() {
    let mut t = timeline();
    let d = 0.9 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.handle(&InputEvent::Scroll {
        displacement: d,
        delta: 0.0,
        velocity: 2.0,
        fingers: 1,
        ended: true,
        cancelled: true,
        timestamp_us: 0,
    });
    settle(&mut t);
    assert_eq!(current_id(&t), 10);
}

#[test]
fn drag_past_the_end_resists_and_springs_back() {
    let mut t = timeline();
    for _ in 0..3 {
        t.handle(&gesture(GestureKind::SwipeForward));
    }
    settle(&mut t);
    assert_eq!(current_id(&t), 13);
    t.handle(&scroll(1.0, 0.0, false));
    let over = t.position() - 5.0;
    assert!(over > 0.0 && over < 0.5, "rubber band: {over}");
    t.handle(&scroll(1.0, 0.0, true));
    settle(&mut t);
    assert_eq!(t.position(), 5.0);
}

#[test]
fn grabbing_during_an_animation_continues_from_the_current_position() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    t.tick(0.05);
    let p = t.position();
    assert!(p > 2.0 && p < 3.0);
    t.handle(&scroll(0.0001, 0.0, false));
    assert!((t.position() - p).abs() < 0.01, "no jump when grabbed");
}

#[test]
fn tap_opens_the_menu_of_the_current_card() {
    let mut t = timeline();
    assert_eq!(
        t.handle(&gesture(GestureKind::Tap)),
        Some(Action::OpenMenu(10))
    );
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(
        t.handle(&gesture(GestureKind::Tap)),
        Some(Action::OpenMenu(11))
    );
}

#[test]
fn swipe_down_on_home_requests_sleep() {
    let mut t = timeline();
    assert_eq!(
        t.handle(&gesture(GestureKind::SwipeDown)),
        Some(Action::RequestSleep)
    );
}

#[test]
fn swipe_down_elsewhere_goes_back_home() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(
        t.handle(&gesture(GestureKind::SwipeDown)),
        Some(Action::Back)
    );
    settle(&mut t);
    assert_eq!(current_id(&t), 10);
}

#[test]
fn other_gestures_do_nothing() {
    let mut t = timeline();
    assert_eq!(t.handle(&gesture(GestureKind::LongPress)), None);
    assert_eq!(t.handle(&gesture(GestureKind::SwipeUp)), None);
    assert_eq!(current_id(&t), 10);
}

#[test]
fn new_history_card_keeps_the_current_card_in_view() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 12);
    t.insert_history(text(14, "brand new"));
    assert_eq!(current_id(&t), 12);
    assert_eq!(t.cards()[t.home_index() + 1].id, 14);
    t.handle(&gesture(GestureKind::SwipeBackward));
    settle(&mut t);
    assert_eq!(current_id(&t), 11);
}

#[test]
fn removing_a_card_keeps_a_valid_position() {
    let mut t = timeline();
    for _ in 0..3 {
        t.handle(&gesture(GestureKind::SwipeForward));
    }
    settle(&mut t);
    assert!(t.remove_card(13));
    assert!(t.is_settled());
    assert_eq!(current_id(&t), 12);
    assert!(!t.remove_card(10), "home cannot be removed");
    assert!(!t.remove_card(99));
}

// ---- rendering and the layer cache ----

#[test]
fn render_draws_and_counts_card_renders() {
    let mut t = timeline();
    t.set_context(ctx());
    let mut f = Frame::glass();
    assert!(t.needs_redraw());
    t.render(&mut f);
    assert!(!t.needs_redraw());
    // Visible card plus its two neighbours are rendered ahead.
    assert_eq!(t.render_count(), 3);
    t.render(&mut f);
    assert_eq!(t.render_count(), 3, "nothing changed, nothing re-rendered");
}

#[test]
fn scrolling_between_prepared_cards_does_not_rerender() {
    let mut t = timeline();
    t.set_context(ctx());
    let mut f = Frame::glass();
    t.render(&mut f);
    let before = t.render_count();
    for i in 1..=20 {
        let d = 0.04 * i as f32 / 20.0;
        t.handle(&scroll(d, 0.0, false));
        assert!(t.needs_redraw());
        t.render(&mut f);
    }
    assert_eq!(
        t.render_count(),
        before,
        "the scroll path only composites layers"
    );
}

#[test]
fn layers_are_bounded_to_the_neighbourhood() {
    let mut t = timeline();
    t.set_context(ctx());
    let mut f = Frame::glass();
    for _ in 0..3 {
        t.handle(&gesture(GestureKind::SwipeForward));
        while !t.is_settled() {
            t.tick(1.0 / 60.0);
            t.render(&mut f);
        }
        t.render(&mut f);
        assert!(t.cached_layers() <= 3, "{} layers", t.cached_layers());
    }
}

#[test]
fn changing_a_card_rerenders_only_that_card() {
    let mut t = timeline();
    t.set_context(ctx());
    let mut f = Frame::glass();
    t.render(&mut f);
    let before = t.render_count();
    assert!(t.update_card(
        11,
        CardContent::Text {
            body: "changed".into(),
            source: "Test".into(),
            timestamp_s: 1_000
        }
    ));
    assert!(t.needs_redraw());
    t.render(&mut f);
    assert_eq!(t.render_count(), before + 1);
}

#[test]
fn clock_change_rerenders_only_home() {
    let mut t = timeline();
    t.set_context(ctx());
    let mut f = Frame::glass();
    t.render(&mut f);
    let before = t.render_count();
    // Same minute, later second: relative times of the text cards still read the same.
    t.set_context(RenderContext {
        now_s: 1_061,
        ..ctx()
    });
    t.render(&mut f);
    assert_eq!(t.render_count(), before);
    t.set_context(RenderContext {
        minute: 43,
        now_s: 1_062,
        ..ctx()
    });
    assert!(t.needs_redraw());
    t.render(&mut f);
    assert_eq!(t.render_count(), before + 1);
}

#[test]
fn mid_scroll_frame_shows_both_cards_side_by_side() {
    let mut t = timeline();
    t.set_context(ctx());
    let mut a = Frame::glass();
    t.render(&mut a);
    let home = a.clone();
    t.handle(&scroll(0.5 / CARDS_PER_PAD_WIDTH, 0.0, false));
    t.render(&mut a);
    // The right half of home is now on the left half of the screen.
    for y in (0..360).step_by(7) {
        for x in (0..320).step_by(7) {
            assert_eq!(a.pixel(x, y), home.pixel(x + 320, y), "at {x},{y}");
        }
    }
}

// ---- review fixes ----

#[test]
fn insert_during_an_animation_keeps_target_and_position_together() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 11);
    t.handle(&gesture(GestureKind::SwipeBackward));
    t.tick(0.03); // position between home (2) and 11 (3)
    t.insert_history(text(14, "brand new"));
    assert_eq!(current_id(&t), 10, "still heading home");
    settle(&mut t);
    assert_eq!(current_id(&t), 10);
    assert_eq!(t.position(), t.home_index() as f32);
}

#[test]
fn insert_during_a_drag_keeps_the_dragged_cards() {
    let mut t = timeline();
    let d = 0.3 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false)); // home dragged 0.3 toward 11
    t.insert_history(text(14, "brand new"));
    // The cards under the finger stay home and the card next to it is now 14.
    let home = t.home_index() as f32;
    assert!(
        (t.position() - (home + 0.3)).abs() < 1e-4,
        "{}",
        t.position()
    );
    t.handle(&scroll(d, 0.0, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 10, "a short drag springs back to home");
}

#[test]
fn insert_on_the_backward_side_during_a_drag_after_a_swipe() {
    let mut t = timeline();
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    let d = 0.7 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.insert_backward(text(3, "now 2"));
    t.handle(&scroll(d, 0.0, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 12);
}

#[test]
fn swipe_after_a_scroll_is_one_touch_even_after_a_slow_frame() {
    let mut t = timeline();
    let d = 0.6 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 0.8, true));
    t.tick(0.12);
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 11, "one card, not two");
}

#[test]
fn swipe_long_after_a_scroll_by_timestamp_is_a_new_touch() {
    let mut t = timeline();
    let d = 0.6 / CARDS_PER_PAD_WIDTH;
    t.handle(&InputEvent::Scroll {
        displacement: d,
        delta: d,
        velocity: 0.0,
        fingers: 1,
        ended: true,
        cancelled: false,
        timestamp_us: 1_000_000,
    });
    t.handle(&InputEvent::Gesture {
        kind: GestureKind::SwipeForward,
        fingers: 1,
        velocity: 1.0,
        timestamp_us: 3_000_000,
    });
    settle(&mut t);
    assert_eq!(current_id(&t), 12, "the drag's card, then one more");
}

#[test]
fn a_tap_ends_the_scroll_touch() {
    let mut t = timeline();
    let d = 0.7 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, 0.0, true));
    settle(&mut t);
    t.handle(&gesture(GestureKind::Tap));
    t.handle(&gesture(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 12);
}

#[test]
fn only_one_finger_gestures_act() {
    let mut t = timeline();
    let two = |kind| InputEvent::Gesture {
        kind,
        fingers: 2,
        velocity: 0.0,
        timestamp_us: 0,
    };
    assert_eq!(t.handle(&two(GestureKind::Tap)), None);
    assert_eq!(t.handle(&two(GestureKind::SwipeDown)), None);
    t.handle(&two(GestureKind::SwipeForward));
    settle(&mut t);
    assert_eq!(current_id(&t), 10);
}

#[test]
fn flick_against_the_drag_returns_to_the_start() {
    let mut t = timeline();
    let d = 0.7 / CARDS_PER_PAD_WIDTH;
    t.handle(&scroll(d, 0.0, false));
    t.handle(&scroll(d, -1.0, true));
    settle(&mut t);
    assert_eq!(current_id(&t), 10);
}

#[test]
#[should_panic(expected = "frame size")]
fn render_rejects_a_frame_of_the_wrong_size() {
    let mut t = timeline();
    t.render(&mut Frame::new(10, 10));
}

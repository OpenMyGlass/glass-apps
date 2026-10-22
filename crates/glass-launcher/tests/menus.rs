//! Menus and settings in the app, without glassd (settings applied locally).

use glass_gfx::Color;
use glass_launcher::platform::{MockPlatform, RadioStatus};
use glass_launcher::settings::ids;
use glass_launcher::{demo, App, OverlayKind};
use glass_ui::timeline::MAX_RESIDENT_LAYERS;
use glass_ui::{CardContent, CardId, GestureKind, InputEvent, RenderContext};

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
    }
}

fn app() -> App {
    let mut a = App::new(demo::timeline(), ctx());
    a.render_if_changed().unwrap();
    a
}

fn g(app: &mut App, kind: GestureKind) {
    app.input(&InputEvent::gesture(kind));
    app.step(1.0, ctx()); // any animation ends
    app.render_if_changed().unwrap();
}

/// Swipes until card `id` is shown.
fn goto(app: &mut App, id: CardId) {
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

fn subtitle(app: &App, id: CardId) -> String {
    let card = app.timeline().cards().iter().find(|c| c.id == id).unwrap();
    match &card.content {
        CardContent::Settings {
            subtitle: Some(s), ..
        } => s.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn settings_cards_sit_on_the_backward_side() {
    let a = app();
    let t = a.timeline();
    let home = t.home_index();
    let ids: Vec<CardId> = t.cards()[..home].iter().rev().map(|c| c.id).collect();
    assert_eq!(ids[0], 2, "the now card is next to home");
    assert_eq!(&ids[1..], &glass_launcher::settings::ORDER);
    assert_eq!(subtitle(&a, ids::SCREEN_TIMEOUT), "15 seconds");
    assert_eq!(subtitle(&a, ids::HEAD_WAKE), "On, at 30°");
}

#[test]
fn head_wake_menu_and_the_wake_angle_screen() {
    let mut a = app();
    goto(&mut a, ids::HEAD_WAKE);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), Some(OverlayKind::Menu { index: 0 }));
    assert_eq!(a.menu_labels(), vec!["Turn off", "Wake angle"]);
    g(&mut a, GestureKind::SwipeForward);
    assert_eq!(a.overlay(), Some(OverlayKind::Menu { index: 1 }));
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), Some(OverlayKind::Adjust));
    g(&mut a, GestureKind::SwipeForward);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), None);
    assert_eq!(a.settings().unwrap().head_wake_angle_deg, Some(35.0));
    assert_eq!(subtitle(&a, ids::HEAD_WAKE), "On, at 35°");
    // Turn it off from the menu.
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    assert_eq!(subtitle(&a, ids::HEAD_WAKE), "Off");
    assert!(a.take_log().iter().any(|l| l.starts_with("settings:")));
}

#[test]
fn swipe_down_leaves_a_menu_or_the_angle_screen_unchanged() {
    let mut a = app();
    goto(&mut a, ids::HEAD_WAKE);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::SwipeDown);
    assert_eq!(a.overlay(), None);
    assert_eq!(subtitle(&a, ids::HEAD_WAKE), "On, at 30°");
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::SwipeForward);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::SwipeBackward);
    g(&mut a, GestureKind::SwipeDown);
    assert_eq!(a.overlay(), None);
    assert_eq!(a.settings().unwrap().head_wake_angle_deg, Some(30.0));
    assert_eq!(
        a.timeline().current_card().id,
        ids::HEAD_WAKE,
        "the timeline is where it was"
    );
}

#[test]
fn screen_timeout_menu_starts_at_the_current_choice() {
    let mut a = app();
    goto(&mut a, ids::SCREEN_TIMEOUT);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), Some(OverlayKind::Menu { index: 2 }));
    g(&mut a, GestureKind::SwipeForward);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.settings().unwrap().screen_timeout_ms, Some(30_000));
    assert_eq!(subtitle(&a, ids::SCREEN_TIMEOUT), "30 seconds");
}

#[test]
fn toggles_for_wink_on_head_and_nudge() {
    let mut a = app();
    for id in [ids::WINK, ids::ON_HEAD, ids::NUDGE] {
        goto(&mut a, id);
        assert_eq!(subtitle(&a, id), "Off");
        g(&mut a, GestureKind::Tap);
        assert_eq!(a.menu_labels(), vec!["Turn on"]);
        g(&mut a, GestureKind::Tap);
        assert_eq!(subtitle(&a, id), "On");
    }
    let s = a.settings().unwrap();
    assert_eq!(s.wink_enabled, Some(true));
    assert_eq!(s.on_head_detection_enabled, Some(true));
    assert_eq!(s.nudge_sleep_enabled, Some(true));
}

#[test]
fn radios_go_through_the_platform() {
    let mut a = App::new(demo::timeline(), ctx()).with_platform(Box::new(MockPlatform::new(
        RadioStatus::on(&["HomeNet"]),
        RadioStatus::on(&["Headset"]),
    )));
    assert_eq!(subtitle(&a, ids::BLUETOOTH), "Connected to Headset");
    goto(&mut a, ids::WIFI);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.menu_labels(), vec!["Turn off"]);
    g(&mut a, GestureKind::Tap);
    assert_eq!(subtitle(&a, ids::WIFI), "Off");
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    assert_eq!(subtitle(&a, ids::WIFI), "Connected to HomeNet");
}

#[test]
fn a_failing_radio_is_logged_and_the_card_stays() {
    let mut p = MockPlatform::default();
    p.fail = true;
    let mut a = App::new(demo::timeline(), ctx()).with_platform(Box::new(p));
    goto(&mut a, ids::BLUETOOTH);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    assert_eq!(subtitle(&a, ids::BLUETOOTH), "On, not connected");
    assert!(a
        .take_log()
        .iter()
        .any(|l| l.contains("bluetooth: mock failure")));
}

#[test]
fn device_card_has_no_menu_and_shows_versions() {
    let mut a = app();
    goto(&mut a, ids::DEVICE);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), None);
    let card = a.timeline().current_card();
    let CardContent::Info { lines, .. } = &card.content else {
        panic!("{:?}", card.content)
    };
    assert!(lines[0].starts_with("glass-launcher "));
    assert!(lines.iter().any(|l| l == "glassd not connected"));
}

#[test]
fn a_menu_covers_the_screen_and_keeps_layers_bounded() {
    let mut a = app();
    goto(&mut a, ids::SCREEN_TIMEOUT);
    let before = a.read_frame().unwrap();
    g(&mut a, GestureKind::Tap);
    let menu = a.read_frame().unwrap();
    assert_ne!(before.data(), menu.data(), "the menu is drawn");
    for _ in 0..7 {
        a.input(&InputEvent::gesture(GestureKind::SwipeForward));
        for _ in 0..10 {
            a.step(1.0 / 60.0, ctx());
            a.render_if_changed().unwrap();
            assert!(
                a.layer_count() <= MAX_RESIDENT_LAYERS,
                "{}",
                a.layer_count()
            );
        }
    }
    g(&mut a, GestureKind::SwipeDown);
    assert_eq!(
        a.read_frame().unwrap().data(),
        before.data(),
        "back to the card"
    );
    assert!(a.layer_count() <= MAX_RESIDENT_LAYERS);
}

#[test]
fn the_screen_turning_off_closes_a_menu() {
    let mut a = app();
    goto(&mut a, ids::WINK);
    g(&mut a, GestureKind::Tap);
    assert!(a.overlay().is_some());
    a.screen_off();
    assert_eq!(a.overlay(), None);
    a.render_if_changed().unwrap();
    assert_eq!(a.read_frame().unwrap().pixel(320, 180), Color::BLACK);
}

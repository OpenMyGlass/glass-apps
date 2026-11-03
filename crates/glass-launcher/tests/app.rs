use glass_launcher::{demo, App};
use glass_ui::{Action, GestureKind, InputEvent, RenderContext};

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

fn settings_subtitle(app: &App, id: glass_ui::CardId) -> String {
    use glass_ui::CardContent;
    match &glass_launcher::settings::card_content(id, &app.settings_view()).unwrap() {
        CardContent::Settings {
            subtitle: Some(s), ..
        } => s.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn changing_the_locale_rerenders_the_cards() {
    use glass_launcher::settings::ids;
    use glass_ui::Locale;

    let mut app = App::new(demo::timeline(), ctx());
    // Default language before any choice is English.
    assert_eq!(app.locale(), Locale::En);
    assert!(!app.locale_chosen());
    assert_eq!(settings_subtitle(&app, ids::SCREEN_TIMEOUT), "2 minutes");

    // Switching to Italian re-renders every settings card in Italian.
    assert!(app.set_locale(Locale::It));
    assert!(app.locale_chosen());
    assert_eq!(settings_subtitle(&app, ids::SCREEN_TIMEOUT), "2 minuti");
    assert_eq!(settings_subtitle(&app, ids::LANGUAGE), "Italiano");

    // Then to French.
    assert!(app.set_locale(Locale::Fr));
    assert_eq!(settings_subtitle(&app, ids::SCREEN_TIMEOUT), "2 minutes");

    // Re-selecting the same language reports no visible change.
    assert!(!app.set_locale(Locale::Fr));
}

//! The launcher against glass-api's in-process mock of glassd.

use glass_api::proto::{
    client_frame, screen_request, service_frame::Body, FocusState, Gesture, GestureKind as PKind,
    Screen, ScreenState, Scroll, Topic,
};
use glass_api::{Client, MockService};
use glass_gfx::HeadlessDisplay;
use glass_launcher::glassd::{
    returns_home, run, to_input, wait_time, Backoff, LoopOptions, Session, CLIENT_NAME,
};
use glass_launcher::{demo, App, ScreenOwner};
use glass_ui::{GestureKind, InputEvent, RenderContext};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(5);

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
    }
}

fn app() -> App {
    let mut a = App::new(demo::timeline(), ctx());
    a.follow_glassd();
    a
}

fn gesture(kind: PKind) -> Body {
    Body::Gesture(Gesture {
        kind: kind as i32,
        fingers: 1,
        velocity: 0.0,
        timestamp_us: 1,
    })
}

fn screen(s: Screen, reason: &str, last_sleep: &str, off_ms: u32) -> ScreenState {
    ScreenState {
        screen: s as i32,
        reason: reason.into(),
        timestamp_us: 1,
        last_sleep_reason: last_sleep.into(),
        off_ms,
    }
}

/// Pumps until `pred` holds.
fn pump_until(s: &mut Session, app: &mut App, pred: impl Fn(&Session, &App) -> bool) {
    let deadline = Instant::now() + T;
    while !pred(s, app) {
        assert!(Instant::now() < deadline, "condition not reached");
        s.pump(app, Some(Duration::from_millis(50))).unwrap();
    }
}

/// Pumps whatever arrives within `d`.
fn pump_for(s: &mut Session, app: &mut App, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        s.pump(app, Some(Duration::from_millis(20))).unwrap();
    }
}

fn connected() -> (MockService, Session, App) {
    let m = MockService::start().unwrap();
    let mut a = app();
    let mut s = Session::connect(m.path()).unwrap();
    pump_until(&mut s, &mut a, |s, _| s.focused());
    (m, s, a)
}

// ---- pure mapping and rules ----

#[test]
fn gestures_and_scrolls_map_field_by_field() {
    let g = Body::Gesture(Gesture {
        kind: PKind::SwipeBackward as i32,
        fingers: 2,
        velocity: -1.5,
        timestamp_us: 77,
    });
    assert_eq!(
        to_input(&g),
        Some(InputEvent::Gesture {
            kind: GestureKind::SwipeBackward,
            fingers: 2,
            velocity: -1.5,
            timestamp_us: 77,
        })
    );
    let s = Body::Scroll(Scroll {
        displacement: 0.3,
        delta: 0.1,
        velocity: 0.9,
        fingers: 3,
        ended: true,
        cancelled: true,
        timestamp_us: 99,
    });
    assert_eq!(
        to_input(&s),
        Some(InputEvent::Scroll {
            displacement: 0.3,
            delta: 0.1,
            velocity: 0.9,
            fingers: 3,
            ended: true,
            cancelled: true,
            timestamp_us: 99,
        })
    );
    for (p, k) in [
        (PKind::Tap, GestureKind::Tap),
        (PKind::LongPress, GestureKind::LongPress),
        (PKind::SwipeForward, GestureKind::SwipeForward),
        (PKind::SwipeDown, GestureKind::SwipeDown),
        (PKind::SwipeUp, GestureKind::SwipeUp),
    ] {
        assert!(
            matches!(to_input(&gesture(p)), Some(InputEvent::Gesture { kind, .. }) if kind == k)
        );
    }
    assert_eq!(to_input(&gesture(PKind::Unspecified)), None);
    let unknown = Gesture {
        kind: 99,
        ..Default::default()
    };
    assert_eq!(to_input(&Body::Gesture(unknown)), None);
    assert_eq!(
        to_input(&Body::FocusState(FocusState { focused: true })),
        None
    );
}

#[test]
fn return_home_rule() {
    let wake = |reason, last, off| screen(Screen::On, reason, last, off);
    // Short sleeps by timeout or gesture keep the card.
    assert!(!returns_home(&wake("tap", "timeout", 3_000)));
    assert!(!returns_home(&wake("look-up", "nudge", 5_000)));
    assert!(!returns_home(&wake("tap", "two-finger-swipe-down", 100)));
    // Longer, or another sleep reason: home.
    assert!(returns_home(&wake("tap", "timeout", 5_001)));
    assert!(returns_home(&wake("tap", "request", 1_000)));
    assert!(returns_home(&wake("don", "off-head", 1_000)));
    // A swipe wake always returns home.
    assert!(returns_home(&wake("swipe-forward", "timeout", 1_000)));
    assert!(returns_home(&wake("swipe-backward", "nudge", 1_000)));
}

#[test]
fn backoff_doubles_up_to_the_limit_and_resets() {
    let mut b = Backoff::new(Duration::from_millis(100), Duration::from_millis(500));
    let d: Vec<u128> = (0..5).map(|_| b.next_delay().as_millis()).collect();
    assert_eq!(d, vec![100, 200, 400, 500, 500]);
    b.reset();
    assert_eq!(b.next_delay(), Duration::from_millis(100));
}

#[test]
fn the_loop_blocks_without_a_timeout_when_nothing_moves() {
    let mut a = app();
    // At rest: until the next minute (the home clock), 18 s after 10:42:42.
    let w = wait_time(&a, 42, 60, None).unwrap();
    assert!(
        w >= Duration::from_secs(18) && w < Duration::from_secs(19),
        "{w:?}"
    );
    // Moving: one frame.
    a.input(&InputEvent::gesture(GestureKind::SwipeForward));
    assert_eq!(
        wait_time(&a, 42, 60, None),
        Some(Duration::from_micros(16_666))
    );
    // Screen off: no timeout at all.
    a.screen_off();
    assert_eq!(wait_time(&a, 42, 60, None), None);
    // A host window caps every wait.
    let cap = Some(Duration::from_millis(16));
    assert_eq!(wait_time(&a, 42, 60, cap), cap);
}

// ---- the session against the mock ----

#[test]
fn session_says_hello_subscribes_and_takes_the_focus() {
    let (m, s, a) = connected();
    assert!(s.capabilities().iter().any(|c| c == "mock"));
    assert_eq!(a.screen_owner(), ScreenOwner::Glassd);
    let holder = m.focus_holder().expect("a focus holder");
    assert_eq!(
        m.with_hub(|h| h.client_name(holder).map(String::from))
            .as_deref(),
        Some(CLIENT_NAME)
    );
    let mut frames = vec![];
    while let Some((_, f)) = m.next_client_frame(Duration::from_millis(200)) {
        frames.push(f.body.unwrap());
    }
    assert!(matches!(&frames[0], client_frame::Body::Hello(h) if h.client_name == CLIENT_NAME));
    let client_frame::Body::Subscribe(sub) = &frames[1] else {
        panic!("{:?}", frames[1]);
    };
    let mut topics = sub.topics.clone();
    topics.sort();
    let mut want: Vec<i32> = [Topic::Gesture, Topic::Screen, Topic::Settings, Topic::Wear]
        .iter()
        .map(|&t| t as i32)
        .collect();
    want.sort();
    assert_eq!(topics, want);
    assert!(matches!(frames[2], client_frame::Body::AcquireFocus(_)));
}

#[test]
fn settings_snapshot_is_kept() {
    let (_m, mut s, mut a) = connected();
    pump_until(&mut s, &mut a, |s, _| s.settings().is_some());
    assert_eq!(s.settings().unwrap().screen_timeout_ms, Some(15_000));
}

#[test]
fn gestures_and_scrolls_move_the_timeline() {
    let (m, mut s, mut a) = connected();
    let home = a.timeline().home_index();
    m.publish_body(gesture(PKind::SwipeForward));
    pump_until(&mut s, &mut a, |_, a| {
        a.timeline().current_index() == home + 1
    });
    a.step(1.0, ctx()); // the animation ends
    m.publish_body(Body::Scroll(Scroll {
        displacement: -0.3,
        delta: -0.3,
        velocity: 0.0,
        fingers: 1,
        ended: false,
        cancelled: false,
        timestamp_us: 2,
    }));
    pump_until(&mut s, &mut a, |_, a| {
        (a.timeline().position() - (home as f32 + 1.0 - 0.6)).abs() < 1e-3
    });
    m.publish_body(Body::Scroll(Scroll {
        displacement: -0.3,
        ended: true,
        cancelled: true,
        fingers: 1,
        timestamp_us: 3,
        ..Default::default()
    }));
    // A cancelled touch returns to where the drag started.
    pump_for(&mut s, &mut a, Duration::from_millis(200));
    assert_eq!(a.timeline().current_index(), home + 1);
}

#[test]
fn swipe_down_on_home_asks_glassd_for_sleep_and_follows_the_screen() {
    let (m, mut s, mut a) = connected();
    while m.next_client_frame(Duration::from_millis(100)).is_some() {}
    m.publish_body(gesture(PKind::SwipeDown));
    let deadline = Instant::now() + T;
    let req = loop {
        s.pump(&mut a, Some(Duration::from_millis(20))).unwrap();
        if let Some((_, f)) = m.next_client_frame(Duration::from_millis(20)) {
            if let Some(client_frame::Body::Screen(r)) = f.body {
                break r;
            }
        }
        assert!(Instant::now() < deadline, "no ScreenRequest");
    };
    assert_eq!(req.action, screen_request::Action::Sleep as i32);
    // The mock applies it and broadcasts SCREEN_OFF: nothing is drawn any more.
    pump_until(&mut s, &mut a, |_, a| !a.screen_on());
    assert!(!a.render_if_changed().unwrap());
    assert!(s.take_log().iter().any(|l| l.contains("ACTION_SLEEP")));
    // The local stand-in does not wake the screen: a tap from glassd while off does nothing.
    a.input(&InputEvent::gesture(GestureKind::Tap));
    assert!(!a.screen_on());
}

#[test]
fn wake_keeps_the_card_after_a_short_timeout_and_else_returns_home() {
    let (m, mut s, mut a) = connected();
    let home = a.timeline().home_index();
    let away = |m: &MockService, s: &mut Session, a: &mut App| {
        m.publish_body(gesture(PKind::SwipeForward));
        pump_until(s, a, |_, a| a.timeline().current_index() == home + 1);
        m.publish_body(Body::ScreenState(screen(Screen::Off, "timeout", "", 0)));
        pump_until(s, a, |_, a| !a.screen_on());
    };
    away(&m, &mut s, &mut a);
    m.publish_body(Body::ScreenState(screen(
        Screen::On,
        "tap",
        "timeout",
        4_000,
    )));
    pump_until(&mut s, &mut a, |_, a| a.screen_on());
    assert_eq!(
        a.timeline().current_index(),
        home + 1,
        "short timeout: same card"
    );
    assert!(a.render_if_changed().unwrap(), "the wake redraws");

    m.publish_body(gesture(PKind::SwipeBackward));
    pump_until(&mut s, &mut a, |_, a| a.timeline().current_index() == home);
    away(&m, &mut s, &mut a);
    m.publish_body(Body::ScreenState(screen(
        Screen::On,
        "tap",
        "timeout",
        60_000,
    )));
    pump_until(&mut s, &mut a, |_, a| a.screen_on());
    assert_eq!(a.timeline().current_index(), home, "long sleep: home");

    m.publish_body(gesture(PKind::SwipeForward));
    pump_until(&mut s, &mut a, |_, a| {
        a.timeline().current_index() == home + 1
    });
    m.publish_body(Body::ScreenState(screen(Screen::Off, "nudge", "", 0)));
    pump_until(&mut s, &mut a, |_, a| !a.screen_on());
    m.publish_body(Body::ScreenState(screen(
        Screen::On,
        "swipe-forward",
        "nudge",
        1_000,
    )));
    pump_until(&mut s, &mut a, |_, a| a.screen_on());
    assert_eq!(a.timeline().current_index(), home, "swipe wake: home");
    assert!(a.timeline().is_settled(), "home is shown at once");
}

#[test]
fn dim_keeps_drawing() {
    let (m, mut s, mut a) = connected();
    m.publish_body(Body::ScreenState(screen(Screen::Dim, "dim", "", 0)));
    pump_for(&mut s, &mut a, Duration::from_millis(200));
    assert!(a.screen_on());
}

#[test]
fn input_pauses_while_another_client_holds_the_focus() {
    let (m, mut s, mut a) = connected();
    let launcher = m.focus_holder().unwrap();
    let home = a.timeline().home_index();
    let mut other = Client::connect_as(m.path(), "an-app").unwrap();
    other.subscribe(&[Topic::Gesture]).unwrap();
    other.acquire_focus().unwrap();
    pump_until(&mut s, &mut a, |s, _| !s.focused());
    // Even a frame sent to the launcher directly is ignored while unfocused.
    m.send_to(
        launcher,
        glass_api::proto::ServiceFrame {
            body: Some(gesture(PKind::SwipeForward)),
        },
    );
    pump_for(&mut s, &mut a, Duration::from_millis(200));
    assert_eq!(a.timeline().current_index(), home);
    // The app goes away: the focus comes back, and so does input.
    drop(other);
    pump_until(&mut s, &mut a, |s, _| s.focused());
    m.publish_body(gesture(PKind::SwipeForward));
    pump_until(&mut s, &mut a, |_, a| {
        a.timeline().current_index() == home + 1
    });
}

#[test]
fn focus_loss_mid_drag_returns_the_drag_to_its_start() {
    let (m, mut s, mut a) = connected();
    let home = a.timeline().home_index();
    m.publish_body(Body::Scroll(Scroll {
        displacement: 0.35,
        delta: 0.35,
        fingers: 1,
        timestamp_us: 2,
        ..Default::default()
    }));
    pump_until(&mut s, &mut a, |_, a| {
        a.timeline().position() > home as f32 + 0.5
    });
    // An app takes the focus mid-touch: glassd sends FocusState{false}, then the final
    // Scroll{ended, cancelled}, which the launcher no longer accepts as input.
    let mut other = Client::connect_as(m.path(), "an-app").unwrap();
    other.subscribe(&[Topic::Gesture]).unwrap();
    other.acquire_focus().unwrap();
    pump_until(&mut s, &mut a, |s, _| !s.focused());
    pump_for(&mut s, &mut a, Duration::from_millis(200));
    a.step(1.0, ctx());
    assert!(
        !a.is_animating(),
        "settled: the loop does not keep waking at 60 Hz"
    );
    assert_eq!(a.timeline().current_index(), home);
    assert_eq!(a.timeline().position(), home as f32);
}

// ---- the loop: reconnection ----

/// Points `link` at `target` (a symlink, so the launcher's socket path stays the same while
/// the mock behind it is replaced).
fn point(link: &Path, target: &Path) {
    let _ = std::fs::remove_file(link);
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn the_loop_reconnects_when_glassd_restarts() {
    let dir = std::env::temp_dir().join(format!("glass-launcher-reconnect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("glassd.sock");
    let first = MockService::start().unwrap();
    point(&link, first.path());

    let done = Arc::new(AtomicBool::new(false));
    let ok = Arc::new(AtomicBool::new(false));
    let (d2, ok2, link2) = (done.clone(), ok.clone(), link.clone());
    let driver = std::thread::spawn(move || {
        let named = |m: &MockService| {
            m.wait_until(T, |h| {
                h.focus_holder()
                    .is_some_and(|id| h.client_name(id) == Some(CLIENT_NAME))
            })
        };
        let connected_first = named(&first);
        // A touch is in progress when glassd goes away: its end never arrives.
        first.publish_body(Body::Scroll(Scroll {
            displacement: 0.2,
            delta: 0.2,
            fingers: 1,
            timestamp_us: 2,
            ..Default::default()
        }));
        std::thread::sleep(Duration::from_millis(200));
        // glassd goes away; a new one comes up at the same path.
        drop(first);
        std::thread::sleep(Duration::from_millis(300));
        let second = MockService::start().unwrap();
        point(&link2, second.path());
        let reconnected = named(&second);
        // The new session works: a swipe arrives.
        second.publish_body(gesture(PKind::SwipeForward));
        std::thread::sleep(Duration::from_millis(300));
        ok2.store(connected_first && reconnected, Ordering::SeqCst);
        d2.store(true, Ordering::SeqCst);
        drop(second);
    });

    let mut a = App::new(demo::timeline(), ctx());
    let home = a.timeline().home_index();
    let mut display = HeadlessDisplay::new(640, 360);
    let mut lines = vec![];
    run(
        &link,
        &mut a,
        &mut display,
        ctx,
        LoopOptions {
            fps: 60,
            // Bounded waits so the test notices `done` promptly.
            max_wait: Some(Duration::from_millis(50)),
            backoff: Backoff::new(Duration::from_millis(20), Duration::from_millis(100)),
        },
        |_: &mut HeadlessDisplay| !done.load(Ordering::SeqCst),
        |l| lines.push(l.to_string()),
    )
    .unwrap();
    driver.join().unwrap();
    assert!(ok.load(Ordering::SeqCst), "{lines:#?}");
    assert!(
        lines.iter().filter(|l| l.starts_with("connected")).count() >= 2,
        "{lines:#?}"
    );
    assert!(lines
        .iter()
        .any(|l| l.contains("reconnecting") || l.contains("unavailable")));
    assert_eq!(
        a.timeline().current_index(),
        home + 1,
        "input after the reconnect"
    );
    assert!(display.presented() >= 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Without glassd, a window keeps being polled every `max_wait` while the loop waits for
/// the next attempt, and each real attempt is logged once with its true delay.
#[test]
fn waiting_to_reconnect_keeps_polling_and_logs_each_attempt_once() {
    use std::cell::{Cell, RefCell};
    let missing = std::env::temp_dir().join(format!("glass-no-glassd-{}.sock", std::process::id()));
    let mut a = App::new(demo::timeline(), ctx());
    let mut display = HeadlessDisplay::new(640, 360);
    // The first frame paints three cards: seconds in a debug build under QEMU. Paint it
    // before, so the wait below measures polling only.
    a.present_if_changed(&mut display).unwrap();
    let start = Instant::now();
    let polls = Cell::new(0u32);
    let polls_before_second = Cell::new(0u32);
    let lines = RefCell::new(Vec::<String>::new());
    let attempts = || {
        lines
            .borrow()
            .iter()
            .filter(|l| l.contains("unavailable"))
            .count()
    };
    run(
        &missing,
        &mut a,
        &mut display,
        ctx,
        LoopOptions {
            fps: 60,
            max_wait: Some(Duration::from_millis(10)),
            // Long enough for several polls even when the first frame is slow to render
            // (a debug build under QEMU).
            backoff: Backoff::new(Duration::from_millis(1000), Duration::from_millis(2000)),
        },
        |_: &mut HeadlessDisplay| {
            polls.set(polls.get() + 1);
            // Stop at the second attempt (timing-independent; slow under QEMU).
            attempts() < 2 && start.elapsed() < Duration::from_secs(20)
        },
        |l| {
            if l.contains("unavailable") && attempts() == 1 {
                polls_before_second.set(polls.get());
            }
            lines.borrow_mut().push(l.to_string());
        },
    )
    .unwrap();
    let lines = lines.into_inner();
    let tried: Vec<&String> = lines.iter().filter(|l| l.contains("unavailable")).collect();
    assert_eq!(tried.len(), 2, "{lines:#?}");
    assert!(tried[0].contains("retrying in 1000 ms"), "{lines:#?}");
    assert!(tried[1].contains("retrying in 2000 ms"), "{lines:#?}");
    assert!(
        start.elapsed() >= Duration::from_millis(1000),
        "the second attempt waited for the first delay"
    );
    assert!(
        polls_before_second.get() >= 10,
        "the window was polled while waiting ({} polls)",
        polls_before_second.get()
    );
}

// ---- settings through glassd ----

fn settings_subtitle(a: &App, id: u64) -> String {
    let card = a.timeline().cards().iter().find(|c| c.id == id).unwrap();
    match &card.content {
        glass_ui::CardContent::Settings {
            subtitle: Some(s), ..
        } => s.clone(),
        other => panic!("{other:?}"),
    }
}

/// Shows card `id` (navigation through the app directly).
fn show(a: &mut App, id: u64) {
    for _ in 0..30 {
        let t = a.timeline();
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
        a.input(&InputEvent::gesture(kind));
        a.step(1.0, ctx());
    }
    panic!("card {id} not reached");
}

/// The next SetSettings the mock received, pumping the launcher meanwhile.
fn next_set_settings(m: &MockService, s: &mut Session, a: &mut App) -> glass_api::proto::Settings {
    let deadline = Instant::now() + T;
    loop {
        s.pump(a, Some(Duration::from_millis(20))).unwrap();
        if let Some((_, f)) = m.next_client_frame(Duration::from_millis(20)) {
            if let Some(client_frame::Body::SetSettings(s)) = f.body {
                return s.settings.expect("settings in SetSettings");
            }
        }
        assert!(Instant::now() < deadline, "no SetSettings");
    }
}

#[test]
fn settings_cards_wait_for_glassd_then_show_its_snapshot() {
    let m = MockService::start().unwrap();
    let mut a = app();
    use glass_launcher::settings::ids;
    assert_eq!(settings_subtitle(&a, ids::WINK), "Waiting for glassd");
    let mut s = Session::connect(m.path()).unwrap();
    pump_until(&mut s, &mut a, |_, a| a.settings().is_some());
    assert_eq!(settings_subtitle(&a, ids::WINK), "Off");
    assert_eq!(settings_subtitle(&a, ids::SCREEN_TIMEOUT), "15 seconds");
}

#[test]
fn a_menu_choice_sends_set_settings_and_the_reply_updates_the_card() {
    use glass_launcher::settings::ids;
    let (m, mut s, mut a) = connected();
    pump_until(&mut s, &mut a, |_, a| a.settings().is_some());
    while m.next_client_frame(Duration::from_millis(100)).is_some() {}
    show(&mut a, ids::WINK);
    m.publish_body(gesture(PKind::Tap)); // opens the menu
    pump_until(&mut s, &mut a, |_, a| a.overlay().is_some());
    m.publish_body(gesture(PKind::Tap)); // "Turn on"
    let sent = next_set_settings(&m, &mut s, &mut a);
    assert_eq!(
        sent,
        glass_api::proto::Settings {
            wink_enabled: Some(true),
            ..Default::default()
        },
        "only the changed field"
    );
    pump_until(&mut s, &mut a, |_, a| {
        settings_subtitle(a, ids::WINK) == "On"
    });
    assert_eq!(
        m.with_hub(|h| h.settings().and_then(|s| s.wink_enabled)),
        Some(true)
    );
}

#[test]
fn the_wake_angle_screen_sends_the_angle() {
    use glass_launcher::settings::ids;
    let (m, mut s, mut a) = connected();
    pump_until(&mut s, &mut a, |_, a| a.settings().is_some());
    while m.next_client_frame(Duration::from_millis(100)).is_some() {}
    show(&mut a, ids::HEAD_WAKE);
    for kind in [PKind::Tap, PKind::SwipeForward] {
        m.publish_body(gesture(kind));
        pump_for(&mut s, &mut a, Duration::from_millis(100));
        a.step(1.0, ctx());
    }
    m.publish_body(gesture(PKind::Tap)); // "Wake angle"
    pump_until(&mut s, &mut a, |_, a| {
        a.overlay() == Some(glass_launcher::OverlayKind::Adjust)
    });
    for kind in [PKind::SwipeBackward, PKind::SwipeBackward, PKind::Tap] {
        m.publish_body(gesture(kind));
    }
    assert_eq!(
        next_set_settings(&m, &mut s, &mut a).head_wake_angle_deg,
        Some(20.0)
    );
    pump_until(&mut s, &mut a, |_, a| {
        settings_subtitle(a, ids::HEAD_WAKE) == "On, at 20°"
    });
}

#[test]
fn a_clamped_value_is_shown_as_glassd_applied_it() {
    use glass_launcher::settings::ids;
    let (m, mut s, mut a) = connected();
    pump_until(&mut s, &mut a, |_, a| a.settings().is_some());
    a.change_settings(glass_api::proto::Settings {
        head_wake_angle_deg: Some(55.0),
        screen_timeout_ms: Some(1_000),
        ..Default::default()
    });
    pump_until(&mut s, &mut a, |_, a| {
        a.settings().and_then(|s| s.screen_timeout_ms) == Some(3_000)
    });
    assert_eq!(settings_subtitle(&a, ids::HEAD_WAKE), "On, at 40°");
    assert_eq!(settings_subtitle(&a, ids::SCREEN_TIMEOUT), "3 seconds");
    drop(m);
}

#[test]
fn a_settings_change_by_another_client_is_shown() {
    use glass_launcher::settings::ids;
    let (m, mut s, mut a) = connected();
    pump_until(&mut s, &mut a, |_, a| a.settings().is_some());
    let mut other = Client::connect_as(m.path(), "settings-app").unwrap();
    other
        .send(client_frame::Body::SetSettings(
            glass_api::proto::SetSettings {
                settings: Some(glass_api::proto::Settings {
                    nudge_sleep_enabled: Some(true),
                    ..Default::default()
                }),
            },
        ))
        .unwrap();
    pump_until(&mut s, &mut a, |_, a| {
        settings_subtitle(a, ids::NUDGE) == "On"
    });
}

#[test]
fn the_device_card_lists_glassd_and_its_capabilities() {
    use glass_launcher::settings::ids;
    let (_m, s, mut a) = connected();
    a.set_service(Some(s.service_info()));
    let card = a
        .timeline()
        .cards()
        .iter()
        .find(|c| c.id == ids::DEVICE)
        .unwrap();
    let glass_ui::CardContent::Info { lines, .. } = &card.content else {
        panic!()
    };
    assert!(
        lines.iter().any(|l| l.starts_with("glassd mock-")),
        "{lines:?}"
    );
    assert!(lines
        .iter()
        .any(|l| l.contains("touchpad") && l.contains("mock")));
}

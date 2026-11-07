//! The "Pair phone" card: the radios asked about first, glassd's pairing mode opened with the
//! screen held on, the countdown, the bonded phone's name checked by the owner, forgetting
//! after a confirmation, and a glassd without the pairing mode. The app alone first (its
//! requests), then the wire frames against glass-api's in-process mock of glassd.

use glass_api::proto::{
    client_frame, screen_request, service_frame::Body, CompanionState, ErrorCode, Gesture,
    GestureKind as PKind, ServiceError, ServiceFrame, Settings,
};
use glass_api::MockService;
use glass_launcher::glassd::{wait_time, Session};
use glass_launcher::settings::{self, ids, ServiceInfo};
use glass_launcher::setup::Companion;
use glass_launcher::{demo, pairing, App, OverlayKind, Request};
use glass_ui::{CardContent, CardId, GestureKind, InputEvent, RenderContext};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(5);
/// The fake `CLOCK_MONOTONIC` starts here (µs).
const START_US: u64 = 1_000_000_000;

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
        locale: glass_ui::Locale::En,
        time_24h: true,
        battery_percent: None,
        on_power: false,
    }
}

fn service(capabilities: &[&str]) -> ServiceInfo {
    ServiceInfo {
        version: "test".into(),
        capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
    }
}

/// An app following glassd, which holds the phone link, with `settings` applied; and its clock.
fn app_with(settings: Settings) -> (App, Rc<Cell<u64>>) {
    let mut a = App::new(demo::timeline(), ctx());
    a.follow_glassd();
    let clock = Rc::new(Cell::new(START_US));
    let c = clock.clone();
    a.set_monotonic_clock(Box::new(move || c.get()));
    a.set_service(Some(service(&["screen", "radio", "companion-link"])));
    a.settings_changed(settings);
    a.render_if_changed().unwrap();
    (a, clock)
}

fn radios(bluetooth: bool, airplane: bool) -> Settings {
    let mut s = settings::settings_defaults();
    s.bluetooth_enabled = Some(bluetooth);
    s.airplane_mode = Some(airplane);
    s
}

fn g(a: &mut App, kind: GestureKind) {
    a.input(&InputEvent::gesture(kind));
    a.step(1.0, ctx());
    a.render_if_changed().unwrap();
}

fn goto(a: &mut App, id: CardId) {
    if settings::ORDER.contains(&id) && !a.settings_open() {
        goto(a, ids::ROOT);
        g(a, GestureKind::Tap);
    }
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
        g(a, kind);
    }
    panic!("card {id} not reached");
}

fn card_subtitle(a: &App, id: CardId) -> String {
    match a.timeline().cards().iter().find(|c| c.id == id) {
        Some(c) => match &c.content {
            CardContent::Settings {
                subtitle: Some(s), ..
            } => s.clone(),
            other => panic!("{other:?}"),
        },
        None => panic!("no card {id}"),
    }
}

fn screen_lines(a: &App) -> Vec<String> {
    match a.pairing_screen() {
        Some(CardContent::Info { lines, .. }) => lines.clone(),
        other => panic!("{other:?}"),
    }
}

fn state(paired: bool, name: &str, until_us: u64) -> Companion {
    Companion {
        paired,
        paired_name: name.into(),
        pairing_until_us: until_us,
        ..Default::default()
    }
}

const OPEN: Request = Request::PairingWindow {
    open: true,
    duration_ms: pairing::WINDOW_MS,
};
const CLOSE: Request = Request::PairingWindow {
    open: false,
    duration_ms: 0,
};

#[test]
fn bluetooth_is_turned_on_after_asking_and_the_mode_counts_down() {
    let (mut a, clock) = app_with(radios(false, false));
    goto(&mut a, ids::PHONE);
    assert_eq!(card_subtitle(&a, ids::PHONE), "Tap to pair");
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.menu_labels(), vec!["Turn on Bluetooth and pair"]);
    assert!(
        a.take_requests().is_empty(),
        "nothing changes before the owner says so"
    );
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), Some(OverlayKind::Pairing));
    assert_eq!(
        a.take_requests(),
        vec![
            Request::SetSettings(Settings {
                bluetooth_enabled: Some(true),
                ..Default::default()
            }),
            OPEN,
            Request::KeepScreenOnFor(pairing::KEEP_ON_MS),
        ]
    );
    assert_eq!(screen_lines(&a)[0], "Opening…");
    assert_eq!(a.next_change(), None);

    // glassd shows it open.
    a.companion_changed(state(false, "", START_US + 120_000_000));
    a.render_if_changed().unwrap();
    assert_eq!(screen_lines(&a)[1], "2:00 left");
    // The loop wakes for the next second, not at the next minute.
    let w = wait_time(&a, ctx().now_s, 60, None).unwrap();
    assert!(w <= Duration::from_millis(1_010), "{w:?}");
    clock.set(START_US + 7_500_000);
    a.step(0.1, ctx());
    assert_eq!(screen_lines(&a)[1], "1:53 left");
    assert!(a.render_if_changed().unwrap(), "the countdown is redrawn");

    // The time runs out: the screen says so and lets the screen time out again.
    clock.set(START_US + 120_000_000);
    a.step(0.1, ctx());
    assert_eq!(
        a.pairing_screen().map(|c| match c {
            CardContent::Info { title, .. } => title.clone(),
            _ => String::new(),
        }),
        Some("Pairing mode ended".into())
    );
    assert_eq!(a.take_requests(), vec![Request::KeepScreenOn(false)]);
    a.companion_changed(state(false, "", 0));
    assert!(a.take_requests().is_empty());
    // A tap opens it again.
    g(&mut a, GestureKind::Tap);
    assert_eq!(
        a.take_requests(),
        vec![OPEN, Request::KeepScreenOnFor(pairing::KEEP_ON_MS)]
    );
    assert_eq!(screen_lines(&a)[0], "Opening…");
    // Backing out closes the mode.
    g(&mut a, GestureKind::SwipeDown);
    assert_eq!(a.overlay(), None);
    assert_eq!(a.take_requests(), vec![CLOSE, Request::KeepScreenOn(false)]);
}

#[test]
fn airplane_mode_is_turned_off_after_asking() {
    let (mut a, _) = app_with(radios(true, true));
    goto(&mut a, ids::PHONE);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.menu_labels(), vec!["Turn off airplane mode and pair"]);
    g(&mut a, GestureKind::Tap);
    assert_eq!(
        a.take_requests()[0],
        Request::SetSettings(Settings {
            airplane_mode: Some(false),
            ..Default::default()
        })
    );
}

#[test]
fn with_the_radios_ready_it_opens_at_once_and_the_screen_going_off_closes_it() {
    let (mut a, _) = app_with(radios(true, false));
    goto(&mut a, ids::PHONE);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.menu_labels(), vec!["Start pairing"]);
    g(&mut a, GestureKind::Tap);
    assert_eq!(
        a.take_requests(),
        vec![OPEN, Request::KeepScreenOnFor(pairing::KEEP_ON_MS)]
    );
    a.companion_changed(state(false, "", START_US + 60_000_000));
    a.screen_off();
    assert_eq!(a.overlay(), None);
    assert_eq!(a.take_requests(), vec![CLOSE, Request::KeepScreenOn(false)]);
}

#[test]
fn a_bonded_phone_is_shown_and_can_be_refused_at_once() {
    let (mut a, _) = app_with(radios(true, false));
    goto(&mut a, ids::PHONE);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    a.take_requests();
    a.companion_changed(state(false, "", START_US + 60_000_000));
    // The first phone in range bonds; glassd closes the mode itself.
    a.companion_changed(state(true, "Pixel 8", 0));
    assert_eq!(a.overlay(), Some(OverlayKind::Menu { index: 0 }));
    assert_eq!(a.menu_labels(), vec!["Paired with Pixel 8", "Not my phone"]);
    assert_eq!(
        a.take_requests(),
        vec![Request::KeepScreenOn(false)],
        "the mode is not closed again"
    );
    assert_eq!(card_subtitle(&a, ids::PHONE), "Paired with Pixel 8");
    g(&mut a, GestureKind::SwipeForward);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.take_requests(), vec![Request::ForgetBond]);
    assert_eq!(a.overlay(), None);
    a.companion_changed(state(false, "", 0));
    assert_eq!(card_subtitle(&a, ids::PHONE), "Tap to pair");
}

#[test]
fn keeping_the_bonded_phone_sends_nothing() {
    let (mut a, _) = app_with(radios(true, false));
    goto(&mut a, ids::PHONE);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    a.companion_changed(state(true, "Pixel 8", 0));
    a.take_requests();
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), None);
    assert!(a.take_requests().is_empty());
}

#[test]
fn forgetting_asks_first_and_cancel_is_the_default() {
    let (mut a, _) = app_with(radios(true, false));
    a.companion_changed(state(true, "Pixel 8", 0));
    goto(&mut a, ids::PHONE);
    assert_eq!(card_subtitle(&a, ids::PHONE), "Paired with Pixel 8");
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.menu_labels(), vec!["Forget the phone"]);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.menu_labels(), vec!["Cancel", "Yes, forget it"]);
    assert_eq!(a.overlay(), Some(OverlayKind::Menu { index: 0 }));
    // A second tap by mistake forgets nothing.
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), None);
    assert!(a.take_requests().is_empty());
    // A settings change while the question is open does not replace it.
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    a.settings_changed(radios(true, false));
    assert_eq!(a.menu_labels(), vec!["Cancel", "Yes, forget it"]);
    g(&mut a, GestureKind::SwipeForward);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.take_requests(), vec![Request::ForgetBond]);
}

#[test]
fn a_glassd_without_the_pairing_mode_disables_the_card() {
    let (mut a, _) = app_with(radios(true, false));
    goto(&mut a, ids::PHONE);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    a.take_requests();
    a.pairing_refused(ErrorCode::Unsupported);
    assert_eq!(screen_lines(&a)[0], "Not available on this Glass");
    assert_eq!(a.take_requests(), vec![Request::KeepScreenOn(false)]);
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), None);
    assert!(a.take_requests().is_empty(), "the mode never opened");
    assert_eq!(card_subtitle(&a, ids::PHONE), "Not available on this Glass");
    g(&mut a, GestureKind::Tap);
    assert_eq!(a.overlay(), None, "no menu");
    // A new glassd may have it.
    a.glassd_disconnected();
    a.set_service(Some(service(&["companion-link"])));
    a.settings_changed(radios(true, false));
    assert_eq!(card_subtitle(&a, ids::PHONE), "Tap to pair");
}

#[test]
fn no_card_without_the_companion_link() {
    let mut a = App::new(demo::timeline(), ctx());
    a.follow_glassd();
    a.set_service(Some(service(&["mock", "screen"])));
    goto(&mut a, ids::ROOT);
    g(&mut a, GestureKind::Tap);
    assert!(a.timeline().cards().iter().all(|c| c.id != ids::PHONE));
    // Nor while settings are open when glassd turns out not to have it.
    let (mut a, _) = app_with(radios(true, false));
    goto(&mut a, ids::PHONE);
    a.set_service(Some(service(&["mock"])));
    assert!(a.timeline().cards().iter().all(|c| c.id != ids::PHONE));
}

#[test]
fn losing_glassd_closes_the_pairing_screen() {
    let (mut a, _) = app_with(radios(true, false));
    goto(&mut a, ids::PHONE);
    g(&mut a, GestureKind::Tap);
    g(&mut a, GestureKind::Tap);
    a.glassd_disconnected();
    assert_eq!(a.overlay(), None);
    assert!(
        a.take_requests().is_empty(),
        "nothing is sent to a new glassd"
    );
}

#[test]
fn the_texts_fit_in_every_language() {
    use glass_launcher::pairing::{Pairing, Refusal};
    use glass_ui::card::text_is_cut;
    use glass_ui::i18n::Locale;
    for locale in Locale::ALL {
        let mut screens = Vec::new();
        let mut p = Pairing::new();
        screens.push(p.card(locale, 0));
        p.on_companion(&state(false, "", 100_000_000));
        screens.push(p.card(locale, 0));
        p.tick(100_000_000);
        screens.push(p.card(locale, 0));
        let mut p = Pairing::new();
        p.refused(Refusal::AlreadyPaired);
        screens.push(p.card(locale, 0));
        let items = settings::forget_question(locale)
            .into_iter()
            .chain(settings::paired_question(locale, "Galaxy S24 Ultra"));
        for it in items {
            screens.push(CardContent::MenuItem {
                label: it.label,
                detail: it.detail,
                index: 0,
                count: 2,
            });
        }
        for c in screens {
            assert!(!text_is_cut(&c, 640, 360), "{locale:?}: {c:?}");
        }
    }
}

// ---- the wire, against the mock glassd ----

fn tap() -> Body {
    Body::Gesture(Gesture {
        kind: PKind::Tap as i32,
        fingers: 1,
        velocity: 0.0,
        timestamp_us: 1,
    })
}

fn pump_until(s: &mut Session, a: &mut App, pred: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + T;
    while !pred(a) {
        assert!(Instant::now() < deadline, "condition not reached");
        s.pump(a, Some(Duration::from_millis(20))).unwrap();
    }
}

/// The next client frame matching `f`, pumping the launcher meanwhile.
fn next_frame<R>(
    m: &MockService,
    s: &mut Session,
    a: &mut App,
    f: impl Fn(&client_frame::Body) -> Option<R>,
) -> (u64, u32, R) {
    let deadline = Instant::now() + T;
    loop {
        s.pump(a, Some(Duration::from_millis(20))).unwrap();
        if let Some((client, frame)) = m.next_client_frame(Duration::from_millis(20)) {
            if let Some(r) = frame.body.as_ref().and_then(&f) {
                return (client, frame.seq, r);
            }
        }
        assert!(Instant::now() < deadline, "frame not sent");
    }
}

#[test]
fn the_requests_go_to_glassd_and_an_unsupported_answer_disables_the_card() {
    let m = MockService::start().unwrap();
    let (mut a, _) = app_with(radios(true, false));
    let mut s = Session::connect(m.path()).unwrap();
    pump_until(&mut s, &mut a, |a| a.settings().is_some());
    // The mock has no companion-link; stand in for a Model B glassd.
    a.set_service(Some(service(&["companion-link"])));
    let mut on = settings::settings_defaults();
    on.bluetooth_enabled = Some(true);
    a.change_settings(on);
    pump_until(&mut s, &mut a, |a| {
        a.settings().and_then(|s| s.bluetooth_enabled) == Some(true)
    });
    goto(&mut a, ids::PHONE);
    while m.next_client_frame(Duration::from_millis(50)).is_some() {}
    m.publish_body(tap()); // the menu
    pump_until(&mut s, &mut a, |a| a.overlay().is_some());
    m.publish_body(tap()); // "Start pairing"
    let (client, seq, w) = next_frame(&m, &mut s, &mut a, |b| match b {
        client_frame::Body::CompanionPairingWindow(w) => Some(*w),
        _ => None,
    });
    assert!(w.open);
    assert_eq!(w.duration_ms, 120_000);
    let (_, _, keep) = next_frame(&m, &mut s, &mut a, |b| match b {
        client_frame::Body::Screen(r) => Some(*r),
        _ => None,
    });
    assert_eq!(keep.action, screen_request::Action::KeepOn as i32);
    assert_eq!(keep.duration_ms, pairing::KEEP_ON_MS);

    // glassd shows it open on TOPIC_COMPANION.
    m.publish_body(Body::CompanionState(CompanionState {
        pairing_until_us: START_US + 90_000_000,
        ..Default::default()
    }));
    pump_until(&mut s, &mut a, |a| {
        a.pairing_screen().is_some_and(|c| {
            matches!(c, CardContent::Info { lines, .. } if lines.len() == 3 && lines[1] == "1:30 left")
        })
    });

    // A glassd without the feature answers the request (matched by seq).
    m.send_to(
        client,
        ServiceFrame {
            body: Some(Body::Error(ServiceError {
                code: ErrorCode::Unsupported as i32,
                message: "unknown request".into(),
                seq,
            })),
        },
    );
    pump_until(&mut s, &mut a, |a| {
        a.pairing_screen().is_some_and(|c| {
            matches!(c, CardContent::Info { lines, .. } if lines[0] == "Not available on this Glass")
        })
    });
    assert!(s
        .take_log()
        .iter()
        .any(|l| l.contains("ERROR_CODE_UNSUPPORTED")));

    // Paired over another transport: the name comes with the state, and forget is sent.
    m.publish_body(tap()); // closes the refusal
    pump_until(&mut s, &mut a, |a| a.overlay().is_none());
    a.glassd_disconnected();
    a.set_service(Some(service(&["companion-link"])));
    a.settings_changed(radios(true, false));
    m.publish_body(Body::CompanionState(CompanionState {
        paired: true,
        paired_name: "Pixel 8".into(),
        ..Default::default()
    }));
    pump_until(&mut s, &mut a, |a| a.companion().paired);
    assert_eq!(a.companion().paired_name, "Pixel 8");
    a.input(&InputEvent::gesture(GestureKind::Tap)); // "Forget the phone"
    a.input(&InputEvent::gesture(GestureKind::Tap)); // the question, on "Cancel"
    a.input(&InputEvent::gesture(GestureKind::SwipeForward));
    a.step(1.0, ctx());
    a.input(&InputEvent::gesture(GestureKind::Tap)); // "Yes, forget it"
    next_frame(&m, &mut s, &mut a, |b| match b {
        client_frame::Body::CompanionForgetBond(_) => Some(()),
        _ => None,
    });
}

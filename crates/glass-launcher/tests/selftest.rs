//! Drives the guided self-test state machine with a scripted sequence of synthetic inputs
//! and asserts the `OMGB-SELFTEST:` PASS/SKIP/FAIL logging. No glassd or audio needed.

use glass_launcher::selftest::{Ev, Req, SelfTest};
use glass_ui::i18n::Locale;
use glass_ui::GestureKind as G;

/// Collects every log line the machine emits (and discards the queued sounds).
fn drain(st: &mut SelfTest, log: &mut Vec<String>) {
    log.extend(st.take_log());
    let _ = st.take_sounds();
}

fn g(st: &mut SelfTest, kind: G, log: &mut Vec<String>) {
    st.on_event(Ev::Gesture(kind));
    drain(st, log);
}

#[test]
fn a_full_scripted_run_logs_each_step_and_a_summary() {
    let mut st = SelfTest::new(Locale::It);
    let mut log = Vec::new();
    drain(&mut st, &mut log);

    // 1 nav_swipe_fwd, 2 nav_swipe_back, 3 nav_tap, 4 nav_swipe_down
    g(&mut st, G::SwipeForward, &mut log);
    g(&mut st, G::SwipeBackward, &mut log);
    g(&mut st, G::Tap, &mut log);
    g(&mut st, G::SwipeDown, &mut log);
    // 5 touch_precision: 3 taps then 3 swipes (all correct)
    for _ in 0..3 {
        g(&mut st, G::Tap, &mut log);
    }
    for _ in 0..3 {
        g(&mut st, G::SwipeForward, &mut log);
    }
    // 6 audio_tone: tap = heard
    g(&mut st, G::Tap, &mut log);
    // 7 head_wake
    st.on_event(Ev::Head);
    drain(&mut st, &mut log);
    // 8 wink
    st.on_event(Ev::Wink);
    drain(&mut st, &mut log);
    // 9 on_head: doff then don
    st.on_event(Ev::Wear { on_head: false });
    st.on_event(Ev::Wear { on_head: true });
    drain(&mut st, &mut log);
    // 10/11/12 radios + airplane: glassd echoes the new state (real verification).
    st.on_event(Ev::Settings {
        wifi: Some(true),
        bt: None,
        airplane: None,
    });
    drain(&mut st, &mut log);
    st.on_event(Ev::Settings {
        wifi: None,
        bt: Some(true),
        airplane: None,
    });
    drain(&mut st, &mut log);
    st.on_event(Ev::Settings {
        wifi: None,
        bt: None,
        airplane: Some(true),
    });
    drain(&mut st, &mut log);
    // 13 language: time-driven, 2 s per locale × 4
    for _ in 0..5 {
        st.tick(2.0);
        drain(&mut st, &mut log);
    }
    // 14 screen_dim: dim then wake
    st.on_event(Ev::Screen { lit: false });
    st.on_event(Ev::Screen { lit: true });
    drain(&mut st, &mut log);

    assert!(
        st.is_done(),
        "the run should finish; log:\n{}",
        log.join("\n")
    );

    // The machine must have driven the sensor enables (gated off by default), requested
    // relative look-up for head, and released/re-asserted keep-awake for the dim step.
    let reqs = st.take_reqs();
    assert!(reqs.contains(&Req::HeadRelative), "{reqs:?}");
    assert!(
        reqs.contains(&Req::Wink(true)) && reqs.contains(&Req::Wink(false)),
        "{reqs:?}"
    );
    assert!(
        reqs.contains(&Req::OnHead(true)) && reqs.contains(&Req::OnHead(false)),
        "{reqs:?}"
    );
    assert!(
        reqs.contains(&Req::KeepAwake(false)) && reqs.contains(&Req::KeepAwake(true)),
        "{reqs:?}"
    );

    let has = |needle: &str| log.iter().any(|l| l.contains(needle));
    assert!(has(
        "step=nav_swipe_fwd expected=swipe_forward got=swipe_forward result=PASS"
    ));
    assert!(has("step=nav_tap expected=tap got=tap result=PASS"));
    assert!(has(
        "step=nav_swipe_down expected=swipe_down got=swipe_down result=PASS"
    ));
    assert!(has("step=touch_precision") && has("taps_ok=3/3 swipes_ok=3/3 result=PASS"));
    assert!(has(
        "step=audio_tone expected=tap=yes|down=no got=tap=heard result=PASS"
    ));
    assert!(has(
        "step=head_wake expected=head_up got=head_up result=PASS"
    ));
    assert!(has("step=wink expected=wink got=wink result=PASS"));
    assert!(has(
        "step=on_head expected=doff_then_don got=doff_then_don result=PASS"
    ));
    assert!(has(
        "step=wifi_toggle expected=wifi_toggled got=wifi=on_verified result=PASS"
    ));
    assert!(has(
        "step=bt_toggle expected=bt_toggled got=bt=on_verified result=PASS"
    ));
    assert!(has(
        "step=airplane expected=airplane_on got=airplane=on_verified result=PASS"
    ));
    assert!(has(
        "step=language expected=4_locales_shown got=locales_shown=4 result=PASS"
    ));
    assert!(has(
        "step=screen_dim expected=dim_then_wake got=dim_then_wake result=PASS"
    ));
    assert!(
        has("SUMMARY pass=14 skip=0 fail=0 total=14"),
        "log:\n{}",
        log.join("\n")
    );
}

#[test]
fn a_swipe_down_skips_a_step_and_timeouts_skip_sensors() {
    let mut st = SelfTest::new(Locale::It);
    let mut log = Vec::new();
    drain(&mut st, &mut log);

    // Step 1 expects swipe-forward; a swipe-down skips it.
    g(&mut st, G::SwipeDown, &mut log);
    assert!(log
        .iter()
        .any(|l| l.contains("step=nav_swipe_fwd") && l.contains("result=SKIP")));

    // Now on step 2 (nav_swipe_back). Let every remaining step time out (20 s each).
    for _ in 0..glass_launcher::selftest::Step::ALL.len() {
        st.tick(21.0);
        drain(&mut st, &mut log);
    }
    assert!(st.is_done());
    // The audio step fails closed? No: on timeout it SKIPs like the rest.
    assert!(log
        .iter()
        .any(|l| l.contains("step=head_wake") && l.contains("result=SKIP")));
    assert!(log
        .iter()
        .any(|l| l.contains("step=wink") && l.contains("result=SKIP")));
    // The summary counts the lone skip from step 1 plus the timeouts.
    assert!(log
        .iter()
        .any(|l| l.starts_with("SUMMARY ") && l.contains("total=14")));
}

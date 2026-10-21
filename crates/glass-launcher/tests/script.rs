use glass_gfx::{Color, HeadlessDisplay};
use glass_launcher::script::{parse, Command, Runner};
use glass_launcher::{demo, App};
use glass_ui::{GestureKind, RenderContext};

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
    }
}

#[test]
fn parses_the_mock_syntax() {
    let script = "\
# a comment
tap
tap 2
swipe forward
swipe backward 2
swipe down
swipe up
longpress
scroll 0.3
scroll -0.25
wait 200ms
wait 150
wait 1s

lookup
wear off
";
    let cmds = parse(script).unwrap();
    assert_eq!(
        cmds,
        vec![
            Command::Gesture(GestureKind::Tap, 1),
            Command::Gesture(GestureKind::Tap, 2),
            Command::Gesture(GestureKind::SwipeForward, 1),
            Command::Gesture(GestureKind::SwipeBackward, 2),
            Command::Gesture(GestureKind::SwipeDown, 1),
            Command::Gesture(GestureKind::SwipeUp, 1),
            Command::Gesture(GestureKind::LongPress, 1),
            Command::Scroll(0.3),
            Command::Scroll(-0.25),
            Command::Wait(200),
            Command::Wait(150),
            Command::Wait(1000),
            Command::Ignored("lookup".into()),
            Command::Ignored("wear off".into()),
        ]
    );
}

#[test]
fn reports_bad_lines_with_their_number() {
    let err = parse("tap\nswipe sideways\n").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
    assert!(parse("scroll x").is_err());
    assert!(parse("wait soon").is_err());
    assert!(parse("tap 9").is_err());
    assert!(parse("dance").is_err());
}

fn run(script: &str) -> (App, HeadlessDisplay, Runner) {
    let mut app = App::new(demo::timeline(), ctx());
    let mut display = HeadlessDisplay::new(640, 360);
    let mut runner = Runner::new(30);
    runner
        .run(&parse(script).unwrap(), &mut app, &mut display, |_| ctx())
        .unwrap();
    (app, display, runner)
}

#[test]
fn swipes_move_through_the_demo_timeline() {
    let (app, display, _) = run("swipe forward\nwait 500ms\nswipe forward\n");
    let t = app.timeline();
    assert_eq!(t.current_index(), t.home_index() + 2);
    assert!(t.is_settled());
    assert!(display.presented() > 5, "animation frames were presented");
}

#[test]
fn idle_waits_present_no_frames() {
    let (_, display, _) = run("wait 2s\n");
    assert_eq!(display.presented(), 1, "only the first frame");
}

#[test]
fn scroll_drags_and_settles() {
    let (app, _, _) = run("scroll 0.3\nwait 500ms\n");
    let t = app.timeline();
    assert_eq!(t.current_index(), t.home_index() + 1);
    let (app, _, _) = run("scroll 0.1\nwait 500ms\n");
    let t = app.timeline();
    assert_eq!(
        t.current_index(),
        t.home_index(),
        "a short slow drag springs back"
    );
}

#[test]
fn swipe_down_on_home_sleeps_and_the_waking_touch_is_consumed() {
    let (app, display, runner) = run("swipe down\nwait 100ms\n");
    assert!(!app.screen_on());
    assert_eq!(display.last_frame().unwrap().pixel(320, 180), Color::BLACK);
    assert!(runner.log().iter().any(|l| l.contains("sleep")));

    let (app, _, _) = run("swipe down\nswipe forward\nwait 500ms\n");
    assert!(app.screen_on());
    let t = app.timeline();
    assert_eq!(
        t.current_index(),
        t.home_index(),
        "the waking swipe did not move"
    );
}

#[test]
fn tap_logs_the_menu_placeholder() {
    let (_, _, runner) = run("tap\n");
    assert!(
        runner.log().iter().any(|l| l.contains("menu")),
        "{:?}",
        runner.log()
    );
}

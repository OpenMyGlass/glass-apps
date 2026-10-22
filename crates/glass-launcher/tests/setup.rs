//! The first-boot setup flow through the app (mock platform, no glassd).

use glass_launcher::platform::PlatformEvent;
use glass_launcher::setup::{Companion, Step};
use glass_launcher::{demo, App};
use glass_ui::{GestureKind, InputEvent, RenderContext};

fn ctx() -> RenderContext {
    RenderContext {
        now_s: demo::DEMO_NOW_S,
        hour: 10,
        minute: 42,
    }
}

fn tap(a: &mut App) {
    a.input(&InputEvent::gesture(GestureKind::Tap));
}

fn marker(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("glass-setup-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn setup_runs_once_and_shows_the_timeline_after() {
    let m = marker("once");
    let mut a = App::new(demo::timeline(), ctx());
    assert!(a.start_setup(Some(m.clone())));
    assert!(a.render_if_changed().unwrap());
    assert!(!a.render_if_changed().unwrap(), "nothing changed");
    tap(&mut a);
    assert_eq!(a.setup().unwrap().step(), &Step::Pair);
    a.platform_event(&PlatformEvent::PairingRequest {
        device: "Pixel".into(),
        passkey: "000123".into(),
    });
    assert!(matches!(a.setup().unwrap().step(), Step::Pairing { .. }));
    assert!(a.render_if_changed().unwrap(), "the code card is drawn");
    tap(&mut a);
    assert_eq!(a.setup().unwrap().step(), &Step::Phone);
    a.companion_changed(Companion {
        provider: true,
        connected: true,
        phone_name: "Pixel".into(),
    });
    tap(&mut a);
    assert!(a.setup().is_none());
    assert!(m.exists());
    assert_eq!(a.timeline().current_index(), a.timeline().home_index());
    assert!(a.render_if_changed().unwrap());
    // Next start: the marker says it is done.
    let mut b = App::new(demo::timeline(), ctx());
    assert!(!b.start_setup(Some(m.clone())));
    assert!(b.setup().is_none());
    let _ = std::fs::remove_file(&m);
}

#[test]
fn an_already_provisioned_system_skips_setup() {
    let m = marker("provisioned");
    let mut a = App::new(demo::timeline(), ctx());
    assert!(a.start_setup(Some(m.clone())));
    a.platform_event(&PlatformEvent::Provisioned(true));
    assert!(a.setup().is_none());
    assert!(m.exists());
    let _ = std::fs::remove_file(&m);
}

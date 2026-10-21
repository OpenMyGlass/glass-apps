//! Golden-image tests: rendered frames are compared with the PNGs in `tests/golden/` at the
//! repository root, allowing a small difference per channel. Run with
//! `GLASS_UPDATE_GOLDEN=1 cargo test -p glass-ui --test golden` to regenerate them.
//! On a mismatch, the actual image is written to `target/golden-actual/`.

use glass_gfx::{Frame, SoftwareCompositor};
use glass_ui::card::render_card;
use glass_ui::{Card, CardContent, RenderContext, Timeline, CARDS_PER_PAD_WIDTH};
use std::path::PathBuf;

/// Largest allowed difference in any channel of any pixel.
const TOLERANCE: u8 = 3;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn check(name: &str, frame: &Frame) {
    let path = root().join("tests/golden").join(format!("{name}.png"));
    if std::env::var_os("GLASS_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        frame.write_png(&path).unwrap();
        return;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run with GLASS_UPDATE_GOLDEN=1 to create it",
            path.display()
        )
    });
    let golden = Frame::from_png(&bytes).unwrap();
    let mismatch = |why: String| {
        let out = root().join("target/golden-actual");
        std::fs::create_dir_all(&out).unwrap();
        frame.write_png(out.join(format!("{name}.png"))).unwrap();
        panic!("{name}: {why}; actual image in {}", out.display());
    };
    if (golden.width(), golden.height()) != (frame.width(), frame.height()) {
        mismatch(format!(
            "size {}x{} vs golden {}x{}",
            frame.width(),
            frame.height(),
            golden.width(),
            golden.height()
        ));
    }
    let mut worst = 0u8;
    let mut over = 0usize;
    for (a, b) in frame.data().iter().zip(golden.data()) {
        let d = a.abs_diff(*b);
        worst = worst.max(d);
        if d > TOLERANCE {
            over += 1;
        }
    }
    if over > 0 {
        mismatch(format!(
            "{over} channel values differ by more than {TOLERANCE} (worst {worst})"
        ));
    }
}

fn ctx() -> RenderContext {
    RenderContext {
        // 2026-10-21 10:42 in the test's clock; the timestamps below are relative to it.
        now_s: 1_792_579_320,
        hour: 10,
        minute: 42,
    }
}

fn message() -> Card {
    Card::new(
        11,
        CardContent::Text {
            body: "Running ten minutes late, save me a seat by the window.".into(),
            source: "Messages".into(),
            timestamp_s: ctx().now_s - 5 * 60,
        },
    )
}

#[test]
fn golden_home() {
    check(
        "home",
        &render_card(&Card::new(10, CardContent::Home), &ctx(), 640, 360),
    );
}

#[test]
fn golden_text() {
    check("text", &render_card(&message(), &ctx(), 640, 360));
}

#[test]
fn golden_text_long() {
    let card = Card::new(
        12,
        CardContent::Text {
            body: "The open stack keeps working without any cloud service: cards, \
                   settings, the camera and voice all run on the device, and the phone \
                   companion only relays notifications and dictation when it is near. \
                   This sentence is long on purpose so that the text has to wrap, shrink \
                   and finally end with an ellipsis."
                .into(),
            source: "OpenMyGlass".into(),
            timestamp_s: ctx().now_s - 3 * 3600,
        },
    );
    check("text-long", &render_card(&card, &ctx(), 640, 360));
}

#[test]
fn golden_settings() {
    check(
        "settings",
        &render_card(
            &Card::new(1, CardContent::settings("Settings")),
            &ctx(),
            640,
            360,
        ),
    );
}

#[test]
fn golden_image_placeholder() {
    let card = Card::new(
        13,
        CardContent::Image {
            image: None,
            caption: "Photo".into(),
            timestamp_s: ctx().now_s - 26 * 3600,
        },
    );
    check("image-placeholder", &render_card(&card, &ctx(), 640, 360));
}

#[test]
fn golden_timeline_mid_scroll() {
    let mut t = Timeline::new(
        vec![Card::new(1, CardContent::settings("Settings"))],
        Card::new(10, CardContent::Home),
        vec![message()],
    );
    t.set_context(ctx());
    let mut f = SoftwareCompositor::glass();
    t.render(&mut f).unwrap();
    t.handle(&glass_ui::InputEvent::Scroll {
        displacement: 0.35 / CARDS_PER_PAD_WIDTH,
        delta: 0.0,
        velocity: 0.0,
        fingers: 1,
        ended: false,
        cancelled: false,
        timestamp_us: 0,
    });
    t.render(&mut f).unwrap();
    check("timeline-mid-scroll", f.frame());
}

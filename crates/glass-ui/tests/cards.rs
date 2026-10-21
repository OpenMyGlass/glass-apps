use glass_gfx::Color;
use glass_ui::card::{clock_text, relative_time, render_card, render_key};
use glass_ui::{Card, CardContent, RenderContext};

fn ctx(now_s: i64) -> RenderContext {
    RenderContext {
        now_s,
        hour: 22,
        minute: 7,
    }
}

#[test]
fn clock_is_twelve_hour_without_leading_zero() {
    assert_eq!(clock_text(10, 42), "10:42");
    assert_eq!(clock_text(22, 7), "10:07");
    assert_eq!(clock_text(0, 0), "12:00");
    assert_eq!(clock_text(12, 30), "12:30");
    assert_eq!(clock_text(9, 5), "9:05");
}

#[test]
fn relative_times() {
    let now = 1_000_000;
    assert_eq!(relative_time(now, now), "just now");
    assert_eq!(relative_time(now, now + 30), "just now", "future clamps");
    assert_eq!(relative_time(now, now - 59), "just now");
    assert_eq!(relative_time(now, now - 60), "1 minute ago");
    assert_eq!(relative_time(now, now - 600), "10 minutes ago");
    assert_eq!(relative_time(now, now - 3_600), "1 hour ago");
    assert_eq!(relative_time(now, now - 5 * 3_600), "5 hours ago");
    assert_eq!(relative_time(now, now - 30 * 3_600), "yesterday");
    assert_eq!(relative_time(now, now - 3 * 86_400), "3 days ago");
}

#[test]
fn every_card_kind_renders_on_black_at_glass_size() {
    let cards = [
        CardContent::Home,
        CardContent::Text {
            body: "Hello".into(),
            source: "Messages".into(),
            timestamp_s: 0,
        },
        CardContent::settings("Settings"),
        CardContent::Image {
            image: None,
            caption: "Photo".into(),
            timestamp_s: 0,
        },
    ];
    for content in cards {
        let f = render_card(&Card::new(1, content.clone()), &ctx(100), 640, 360);
        assert_eq!((f.width(), f.height()), (640, 360));
        assert_eq!(f.pixel(0, 0).a, 255, "opaque layer");
        let lit = f.data().chunks_exact(4).filter(|p| p[0] > 128).count();
        assert!(lit > 50, "{content:?} draws something");
        if !matches!(content, CardContent::Image { .. }) {
            assert_eq!(f.pixel(0, 0), Color::BLACK, "{content:?} background");
        }
    }
}

#[test]
fn text_card_footer_is_grey_and_at_the_bottom() {
    let card = Card::new(
        1,
        CardContent::Text {
            body: String::new(),
            source: "Source".into(),
            timestamp_s: 0,
        },
    );
    let f = render_card(&card, &ctx(7_200), 640, 360);
    let mut top = 360;
    let mut brightest = 0;
    for y in 0..360 {
        for x in 0..640 {
            let p = f.pixel(x, y);
            if p.r > 0 {
                top = top.min(y);
                brightest = brightest.max(p.r);
            }
        }
    }
    assert!(top > 280, "footer text sits at the bottom, top at {top}");
    assert!(brightest < 200, "footer text is grey, max {brightest}");
}

#[test]
fn render_key_changes_only_with_what_is_shown() {
    let text = Card::new(
        1,
        CardContent::Text {
            body: "b".into(),
            source: "s".into(),
            timestamp_s: 0,
        },
    );
    assert_eq!(render_key(&text, &ctx(120)), render_key(&text, &ctx(150)));
    assert_ne!(render_key(&text, &ctx(120)), render_key(&text, &ctx(200)));
    let home = Card::new(2, CardContent::Home);
    assert_eq!(render_key(&home, &ctx(1)), render_key(&home, &ctx(99_999)));
    let later = RenderContext {
        minute: 8,
        ..ctx(1)
    };
    assert_ne!(render_key(&home, &ctx(1)), render_key(&home, &later));
}

#[test]
fn long_body_text_shrinks_and_ellipsizes() {
    let long = "word ".repeat(200);
    let card = Card::new(
        1,
        CardContent::Text {
            body: long,
            source: "s".into(),
            timestamp_s: 0,
        },
    );
    // Must not panic or draw outside the margins.
    let f = render_card(&card, &ctx(0), 640, 360);
    for y in 0..360 {
        for x in 0..30 {
            assert_eq!(f.pixel(x, y).r, 0, "left margin at {x},{y}");
        }
    }
}

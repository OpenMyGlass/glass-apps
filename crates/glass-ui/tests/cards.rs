use glass_gfx::Color;
use glass_ui::card::{clock_text, relative_time, render_card, render_key};
use glass_ui::{Card, CardContent, RenderContext};

fn ctx(now_s: i64) -> RenderContext {
    RenderContext {
        now_s,
        hour: 22,
        minute: 7,
        locale: glass_ui::Locale::En,
        time_24h: true,
        battery_percent: None,
    }
}

#[test]
fn clock_is_twelve_hour_without_leading_zero() {
    assert_eq!(clock_text(10, 42), "10:42");
    assert_eq!(clock_text(22, 7), "10:07");
    assert_eq!(clock_text(0, 0), "12:00");
    assert_eq!(clock_text(12, 30), "12:30");
    assert_eq!(clock_text(9, 5), "9:05");
    let ctx = |hour, time_24h| RenderContext {
        hour,
        minute: 5,
        time_24h,
        ..RenderContext::default()
    };
    assert_eq!(glass_ui::card::clock_text_in(&ctx(21, true)), "21:05");
    assert_eq!(glass_ui::card::clock_text_in(&ctx(9, true)), "09:05");
    assert_eq!(glass_ui::card::clock_text_in(&ctx(21, false)), "9:05");
}

#[test]
fn relative_times() {
    use glass_ui::Locale::{En, It};
    let now = 1_000_000;
    assert_eq!(relative_time(En, now, now), "just now");
    assert_eq!(
        relative_time(En, now, now + 30),
        "just now",
        "future clamps"
    );
    assert_eq!(relative_time(En, now, now - 59), "just now");
    assert_eq!(relative_time(En, now, now - 60), "1 minute ago");
    assert_eq!(relative_time(En, now, now - 600), "10 minutes ago");
    assert_eq!(relative_time(En, now, now - 3_600), "1 hour ago");
    assert_eq!(relative_time(En, now, now - 5 * 3_600), "5 hours ago");
    assert_eq!(relative_time(En, now, now - 30 * 3_600), "yesterday");
    assert_eq!(relative_time(En, now, now - 3 * 86_400), "3 days ago");
    // The same ages in Italian.
    assert_eq!(relative_time(It, now, now), "ora");
    assert_eq!(relative_time(It, now, now - 60), "1 minuto fa");
    assert_eq!(relative_time(It, now, now - 600), "10 minuti fa");
    assert_eq!(relative_time(It, now, now - 30 * 3_600), "ieri");
    assert_eq!(relative_time(It, now, now - 3 * 86_400), "3 giorni fa");
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

#[test]
fn image_render_key_follows_the_image_id_not_its_address() {
    use glass_gfx::Frame;
    use glass_ui::card::CardImage;
    use std::sync::Arc;
    let card = |id: u64| {
        let mut f = Frame::new(4, 4);
        f.fill(Color::WHITE);
        Card::new(
            1,
            CardContent::Image {
                image: Some(CardImage {
                    id,
                    frame: Arc::new(f),
                }),
                caption: "c".into(),
                timestamp_s: 0,
            },
        )
    };
    assert_eq!(render_key(&card(7), &ctx(0)), render_key(&card(7), &ctx(0)));
    assert_ne!(render_key(&card(7), &ctx(0)), render_key(&card(8), &ctx(0)));
}

#[test]
fn relative_time_survives_extreme_timestamps() {
    use glass_ui::Locale::En;
    assert_eq!(relative_time(En, i64::MIN, i64::MAX), "just now");
    assert!(relative_time(En, i64::MAX, i64::MIN).ends_with("days ago"));
}

/// The rows and columns with lit pixels of a rendered card.
fn lit_extent(card: &Card) -> (u32, u32, u32, u32) {
    let f = render_card(card, &RenderContext::default(), 640, 360);
    let (mut x0, mut x1, mut y0, mut y1) = (640, 0, 360, 0);
    for y in 0..360 {
        for x in 0..640 {
            if f.pixel(x, y) != Color::BLACK {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    (x0, x1, y0, y1)
}

#[test]
fn long_translations_wrap_and_shrink_inside_the_card() {
    let long = "Un colpetto di testa in su spegne lo schermo quando vuoi riposare gli occhi";
    for content in [
        CardContent::Settings {
            title: "Détection sur la tête de l'utilisateur".into(),
            subtitle: Some(long.into()),
        },
        CardContent::MenuItem {
            label: "Désactiver la détection sur la tête".into(),
            detail: Some(long.into()),
            index: 0,
            count: 1,
        },
        CardContent::settings(long),
    ] {
        let (x0, x1, y0, y1) = lit_extent(&Card::new(1, content));
        assert!(x0 >= 30 && x1 <= 610, "inside the side margins: {x0}..{x1}");
        assert!(y0 >= 20 && y1 <= 340, "inside the card: {y0}..{y1}");
    }
}

#[test]
fn a_very_long_text_is_cut_but_stays_inside_the_card() {
    let huge = "parola ".repeat(200);
    let (_, _, y0, y1) = lit_extent(&Card::new(1, CardContent::settings(huge)));
    assert!(y0 >= 20 && y1 <= 340, "inside the card: {y0}..{y1}");
}

#[test]
fn the_battery_is_shown_small_at_the_top_left() {
    let lit = |battery_percent, xs: std::ops::Range<u32>| {
        let ctx = RenderContext {
            battery_percent,
            ..RenderContext::default()
        };
        let f = render_card(&Card::new(10, CardContent::Home), &ctx, 640, 360);
        xs.clone()
            .any(|x| (0..70).any(|y| f.pixel(x, y) != Color::BLACK))
    };
    assert!(lit(Some(100), 40..160), "at the left margin");
    assert!(!lit(Some(100), 0..40), "not past the margin");
    assert!(
        !lit(Some(100), 160..640),
        "nothing in the middle or on the right"
    );
    assert!(!lit(None, 0..640), "nothing without a reading");
}

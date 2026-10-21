//! Host benchmarks, reported and not enforced. Run in release mode:
//! `cargo test --release -p glass-ui --test bench -- --ignored --nocapture`

use glass_gfx::Frame;
use glass_ui::card::render_card;
use glass_ui::{Card, CardContent, InputEvent, RenderContext, Timeline};
use std::time::Instant;

fn ctx() -> RenderContext {
    RenderContext {
        now_s: 10_000,
        hour: 10,
        minute: 42,
    }
}

fn text_card(id: u64) -> Card {
    Card::new(
        id,
        CardContent::Text {
            body: "Running ten minutes late, save me a seat by the window.".into(),
            source: "Messages".into(),
            timestamp_s: 9_000,
        },
    )
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture, preferably --release"]
fn bench_card_render_and_scroll_composite() {
    const N: u32 = 200;

    for (name, card) in [
        ("home", Card::new(1, CardContent::Home)),
        ("text", text_card(2)),
        ("settings", Card::new(3, CardContent::settings("Settings"))),
    ] {
        let start = Instant::now();
        for _ in 0..N / 4 {
            std::hint::black_box(render_card(&card, &ctx(), 640, 360));
        }
        let per = start.elapsed() / (N / 4);
        println!("render one {name} card: {per:?}");
    }

    let mut t = Timeline::new(
        vec![],
        Card::new(1, CardContent::Home),
        vec![text_card(2), text_card(3)],
    );
    t.set_context(ctx());
    let mut f = Frame::glass();
    t.render(&mut f);
    let renders = t.render_count();
    let start = Instant::now();
    for i in 0..N {
        let d = 0.45 * (i as f32 / N as f32);
        t.handle(&InputEvent::Scroll {
            displacement: d,
            delta: 0.0,
            velocity: 0.0,
            fingers: 1,
            ended: false,
            cancelled: false,
            timestamp_us: 0,
        });
        t.render(&mut f);
    }
    let per = start.elapsed() / N;
    println!("composite one scrolling frame (two layers): {per:?}");
    assert_eq!(
        t.render_count(),
        renders,
        "no card render on the scroll path"
    );
}

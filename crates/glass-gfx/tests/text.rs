use glass_gfx::text::{self, Align};
use glass_gfx::{fonts, Color, Frame, Rect, Weight};

fn lit_pixels(f: &Frame) -> usize {
    let mut n = 0;
    for y in 0..f.height() {
        for x in 0..f.width() {
            if f.pixel(x, y).r > 0 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn bundled_fonts_load() {
    let font = fonts::font(Weight::Light);
    assert!(text::measure(font, 32.0, "Hello") > 0.0);
}

#[test]
fn measure_grows_with_text_and_size() {
    let f = fonts::font(Weight::Light);
    let a = text::measure(f, 32.0, "Hi");
    let b = text::measure(f, 32.0, "Hi there");
    let c = text::measure(f, 64.0, "Hi");
    assert!(b > a);
    assert!((c / a - 2.0).abs() < 0.1, "width scales with size: {a} {c}");
    assert_eq!(text::measure(f, 32.0, ""), 0.0);
}

#[test]
fn draw_line_draws_inside_its_box_only() {
    let font = fonts::font(Weight::Light);
    let mut f = Frame::new(300, 100);
    f.fill(Color::BLACK);
    let w = text::draw_line(&mut f, font, 40.0, 10.0, 60.0, "Glass", Color::WHITE);
    assert!(w > 50.0);
    assert!(lit_pixels(&f) > 100);
    // Nothing left of the start x or below the descender area.
    for y in 0..100 {
        for x in 0..8 {
            assert_eq!(f.pixel(x, y).r, 0, "pixel {x},{y} left of text");
        }
    }
    for x in 0..300 {
        assert_eq!(f.pixel(x, 99).r, 0);
    }
}

#[test]
fn draw_line_clips_at_frame_edges() {
    let font = fonts::font(Weight::Light);
    let mut f = Frame::new(50, 30);
    f.fill(Color::BLACK);
    // Partly off every edge: must not panic.
    text::draw_line(
        &mut f,
        font,
        60.0,
        -20.0,
        10.0,
        "Wide text here",
        Color::WHITE,
    );
    text::draw_line(
        &mut f,
        font,
        60.0,
        30.0,
        80.0,
        "Wide text here",
        Color::WHITE,
    );
}

#[test]
fn ellipsize_keeps_short_text_and_shortens_long_text() {
    let f = fonts::font(Weight::Light);
    assert_eq!(text::ellipsize(f, 32.0, "Short", 500.0), "Short");
    let long = "This is a much longer line that cannot fit in the space";
    let e = text::ellipsize(f, 32.0, long, 200.0);
    assert!(e.ends_with('…'), "{e}");
    assert!(text::measure(f, 32.0, &e) <= 200.0);
    assert!(e.len() < long.len());
    // Nothing fits but the ellipsis itself.
    assert_eq!(text::ellipsize(f, 32.0, long, 1.0), "");
}

#[test]
fn wrap_breaks_on_spaces_within_width() {
    let f = fonts::font(Weight::Light);
    let lines = text::wrap(f, 32.0, "one two three four five six seven eight", 200.0);
    assert!(lines.len() > 1);
    for l in &lines {
        assert!(text::measure(f, 32.0, l) <= 200.0, "{l}");
        assert!(!l.starts_with(' ') && !l.ends_with(' '));
    }
    assert_eq!(lines.join(" "), "one two three four five six seven eight");
}

#[test]
fn wrap_breaks_a_word_longer_than_the_width() {
    let f = fonts::font(Weight::Light);
    let lines = text::wrap(f, 32.0, "Supercalifragilisticexpialidocious", 150.0);
    assert!(lines.len() > 1);
    assert_eq!(lines.concat(), "Supercalifragilisticexpialidocious");
    for l in &lines {
        assert!(text::measure(f, 32.0, l) <= 150.0);
    }
}

#[test]
fn wrap_keeps_explicit_newlines() {
    let f = fonts::font(Weight::Light);
    let lines = text::wrap(f, 32.0, "a\nb", 500.0);
    assert_eq!(lines, vec!["a", "b"]);
}

#[test]
fn layout_box_limits_lines_and_ellipsizes_the_last() {
    let f = fonts::font(Weight::Light);
    let body = "word ".repeat(80);
    let lines = text::layout_box(f, 32.0, body.trim(), 300.0, 3);
    assert_eq!(lines.len(), 3);
    assert!(lines[2].ends_with('…'));
    let short = text::layout_box(f, 32.0, "two words", 300.0, 3);
    assert_eq!(short, vec!["two words"]);
}

#[test]
fn draw_box_respects_alignment() {
    let font = fonts::font(Weight::Light);
    let mut left = Frame::new(400, 100);
    left.fill(Color::BLACK);
    text::draw_box(
        &mut left,
        font,
        32.0,
        1.2,
        Rect::new(0.0, 0.0, 400.0, 100.0),
        "Hi",
        Color::WHITE,
        Align::Left,
        2,
    );
    let mut right = Frame::new(400, 100);
    right.fill(Color::BLACK);
    text::draw_box(
        &mut right,
        font,
        32.0,
        1.2,
        Rect::new(0.0, 0.0, 400.0, 100.0),
        "Hi",
        Color::WHITE,
        Align::Right,
        2,
    );
    let first_lit_x = |f: &Frame| {
        (0..f.width())
            .find(|&x| (0..f.height()).any(|y| f.pixel(x, y).r > 0))
            .unwrap()
    };
    assert!(first_lit_x(&left) < 10);
    assert!(first_lit_x(&right) > 300);
}

#[test]
fn text_rendering_is_deterministic() {
    let font = fonts::font(Weight::Light);
    let render = || {
        let mut f = Frame::new(200, 60);
        f.fill(Color::BLACK);
        text::draw_line(&mut f, font, 36.0, 5.0, 45.0, "10:42 ok", Color::WHITE);
        f.data().to_vec()
    };
    assert_eq!(render(), render());
}

#[test]
fn draw_box_with_zero_line_height_draws_one_line() {
    let font = fonts::font(Weight::Light);
    let mut f = Frame::new(200, 100);
    let n = text::draw_box(
        &mut f,
        font,
        32.0,
        0.0,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        "one two three four five six",
        Color::WHITE,
        Align::Left,
        usize::MAX,
    );
    assert_eq!(n, 1);
}

#[test]
fn layout_line_places_glyphs_on_whole_pixels_and_skips_spaces() {
    let font = fonts::font(Weight::Light);
    let (run, w) = text::layout_line(font, 32.0, 10.4, 50.6, "a b");
    assert_eq!(w, text::measure(font, 32.0, "a b"));
    assert_eq!(run.glyphs.len(), 2, "the space has no glyph");
    assert_eq!(run.glyphs[0].x, 10);
    assert!(run.glyphs.iter().all(|g| g.baseline == 51));
    assert!(run.glyphs[1].x > run.glyphs[0].x);
}

#[test]
fn drawing_a_laid_out_run_matches_draw_line() {
    let font = fonts::font(Weight::Light);
    let mut a = Frame::new(200, 60);
    a.fill(Color::BLACK);
    text::draw_line(&mut a, font, 36.0, 5.3, 45.0, "10:42 ok", Color::WHITE);
    let mut b = Frame::new(200, 60);
    b.fill(Color::BLACK);
    let (run, _) = text::layout_line(font, 36.0, 5.3, 45.0, "10:42 ok");
    b.draw_glyph_run(&run, Color::WHITE);
    assert_eq!(a.data(), b.data());
}

#[test]
fn layout_box_runs_has_one_run_per_line() {
    let font = fonts::font(Weight::Light);
    let rect = Rect::new(0.0, 0.0, 200.0, 200.0);
    let body = "one two three four five six seven eight";
    let runs = text::layout_box_runs(font, 32.0, 1.2, rect, body, Align::Left, usize::MAX);
    let mut f = Frame::new(200, 200);
    let n = text::draw_box(
        &mut f,
        font,
        32.0,
        1.2,
        rect,
        body,
        Color::WHITE,
        Align::Left,
        usize::MAX,
    );
    assert_eq!(runs.len(), n);
    assert!(n > 1);
    assert!(runs
        .windows(2)
        .all(|r| r[1].glyphs[0].baseline > r[0].glyphs[0].baseline));
}

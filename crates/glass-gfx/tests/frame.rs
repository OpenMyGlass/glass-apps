use glass_gfx::{Color, Frame, Rect};

#[test]
fn default_frame_is_glass_sized_and_opaque_black() {
    let f = Frame::glass();
    assert_eq!((f.width(), f.height()), (640, 360));
    assert_eq!(f.data().len(), 640 * 360 * 4);
    assert_eq!(f.pixel(0, 0), Color::BLACK);
    assert_eq!(f.pixel(639, 359), Color::BLACK);
}

#[test]
fn any_size_frame_starts_transparent() {
    let f = Frame::new(3, 2);
    assert_eq!((f.width(), f.height()), (3, 2));
    assert_eq!(f.pixel(2, 1), Color::TRANSPARENT);
}

#[test]
fn fill_sets_every_pixel() {
    let mut f = Frame::new(4, 4);
    f.fill(Color::rgb(10, 20, 30));
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(f.pixel(x, y), Color::rgb(10, 20, 30));
        }
    }
}

#[test]
fn fill_rect_covers_only_the_rect() {
    let mut f = Frame::new(10, 10);
    f.fill(Color::BLACK);
    f.fill_rect(Rect::new(2.0, 3.0, 4.0, 2.0), Color::WHITE);
    assert_eq!(f.pixel(2, 3), Color::WHITE);
    assert_eq!(f.pixel(5, 4), Color::WHITE);
    assert_eq!(f.pixel(1, 3), Color::BLACK);
    assert_eq!(f.pixel(6, 3), Color::BLACK);
    assert_eq!(f.pixel(2, 5), Color::BLACK);
}

#[test]
fn rounded_rect_leaves_corners_and_fills_center() {
    let mut f = Frame::new(40, 40);
    f.fill(Color::BLACK);
    f.fill_rounded_rect(Rect::new(0.0, 0.0, 40.0, 40.0), 12.0, Color::WHITE);
    assert_eq!(f.pixel(0, 0), Color::BLACK, "corner outside the radius");
    assert_eq!(f.pixel(20, 20), Color::WHITE);
    assert_eq!(f.pixel(20, 0), Color::WHITE, "edge middle is inside");
    // Anti-aliased: some pixel near the corner arc is between black and white.
    let partial = (0..12).any(|i| {
        let p = f.pixel(i, i);
        p.r > 0 && p.r < 255
    });
    assert!(partial, "corner arc is anti-aliased");
}

#[test]
fn blit_copies_with_offset_and_clips() {
    let mut src = Frame::new(4, 4);
    src.fill(Color::rgb(200, 0, 0));
    let mut dst = Frame::new(10, 10);
    dst.fill(Color::BLACK);
    dst.blit(&src, 8, -2, 1.0);
    assert_eq!(dst.pixel(8, 0), Color::rgb(200, 0, 0));
    assert_eq!(dst.pixel(9, 1), Color::rgb(200, 0, 0));
    assert_eq!(dst.pixel(9, 2), Color::BLACK);
    assert_eq!(dst.pixel(7, 0), Color::BLACK);
    // Fully outside: no panic, no change.
    dst.blit(&src, 100, 100, 1.0);
    dst.blit(&src, -100, 0, 1.0);
}

#[test]
fn blit_with_alpha_blends() {
    let mut src = Frame::new(2, 2);
    src.fill(Color::WHITE);
    let mut dst = Frame::new(2, 2);
    dst.fill(Color::BLACK);
    dst.blit(&src, 0, 0, 0.5);
    let p = dst.pixel(0, 0);
    assert!(
        (126..=129).contains(&p.r),
        "half white over black, got {p:?}"
    );
    assert_eq!(p.a, 255);
}

#[test]
fn blit_respects_source_transparency() {
    let mut src = Frame::new(2, 1);
    src.fill(Color::TRANSPARENT);
    src.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::WHITE);
    let mut dst = Frame::new(2, 1);
    dst.fill(Color::rgb(0, 0, 255));
    dst.blit(&src, 0, 0, 1.0);
    assert_eq!(dst.pixel(0, 0), Color::WHITE);
    assert_eq!(dst.pixel(1, 0), Color::rgb(0, 0, 255));
}

#[test]
fn png_round_trip() {
    let mut f = Frame::new(5, 3);
    f.fill(Color::rgb(1, 2, 3));
    f.fill_rect(Rect::new(1.0, 1.0, 1.0, 1.0), Color::rgb(250, 128, 7));
    let bytes = f.to_png().unwrap();
    let g = Frame::from_png(&bytes).unwrap();
    assert_eq!((g.width(), g.height()), (5, 3));
    assert_eq!(g.pixel(1, 1), Color::rgb(250, 128, 7));
    assert_eq!(g.pixel(0, 0), Color::rgb(1, 2, 3));
}

#[test]
fn draw_image_scaled_into_rect() {
    let mut img = Frame::new(2, 2);
    img.fill(Color::rgb(0, 255, 0));
    let mut f = Frame::new(20, 20);
    f.fill(Color::BLACK);
    f.draw_image(&img, Rect::new(5.0, 5.0, 10.0, 10.0));
    assert_eq!(f.pixel(10, 10), Color::rgb(0, 255, 0));
    assert_eq!(f.pixel(2, 2), Color::BLACK);
    assert_eq!(f.pixel(16, 16), Color::BLACK);
}

#[test]
fn bad_png_is_an_error() {
    assert!(Frame::from_png(b"not a png").is_err());
}

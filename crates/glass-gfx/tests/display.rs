use glass_gfx::{Color, Display, Frame, HeadlessDisplay};

#[test]
fn headless_keeps_the_last_frame_and_counts() {
    let mut d = HeadlessDisplay::new(640, 360);
    assert_eq!(d.size(), (640, 360));
    assert!(d.last_frame().is_none());
    let mut f = Frame::glass();
    f.fill(Color::rgb(9, 9, 9));
    d.present(&f).unwrap();
    d.present(&f).unwrap();
    assert_eq!(d.presented(), 2);
    assert_eq!(d.last_frame().unwrap().pixel(1, 1), Color::rgb(9, 9, 9));
}

#[test]
fn headless_rejects_a_frame_of_the_wrong_size() {
    let mut d = HeadlessDisplay::new(640, 360);
    assert!(d.present(&Frame::new(10, 10)).is_err());
}

#[test]
fn headless_writes_numbered_pngs() {
    let dir = std::env::temp_dir().join(format!("glass-gfx-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut d = HeadlessDisplay::new(4, 4).with_output_dir(&dir).unwrap();
    let mut f = Frame::new(4, 4);
    f.fill(Color::WHITE);
    d.present(&f).unwrap();
    d.present(&f).unwrap();
    let a = dir.join("frame-00000.png");
    let b = dir.join("frame-00001.png");
    assert!(a.exists() && b.exists());
    let g = Frame::from_png(&std::fs::read(&a).unwrap()).unwrap();
    assert_eq!(g.pixel(3, 3), Color::WHITE);
    std::fs::remove_dir_all(&dir).unwrap();
}

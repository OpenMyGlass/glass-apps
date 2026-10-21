use glass_gfx::{
    Color, Compositor, Display, HeadlessDisplay, LayerId, Painter, Placement, Rect,
    SoftwareCompositor,
};

fn solid(c: &mut SoftwareCompositor, w: u32, h: u32, color: Color, opaque: bool) -> LayerId {
    let id = c.create_layer(w, h, opaque).unwrap();
    c.paint_layer(id, &mut |p: &mut dyn Painter| p.clear(color))
        .unwrap();
    id
}

#[test]
fn compose_clears_then_places_layers_in_order() {
    let mut c = SoftwareCompositor::new(40, 20);
    let a = solid(&mut c, 20, 20, Color::rgb(200, 0, 0), true);
    let b = solid(&mut c, 20, 20, Color::rgb(0, 200, 0), true);
    c.compose(
        Color::rgb(0, 0, 9),
        &[Placement::at(a, -5, 0), Placement::at(b, 10, 0)],
    )
    .unwrap();
    let f = c.frame();
    assert_eq!(f.pixel(0, 0), Color::rgb(200, 0, 0));
    assert_eq!(f.pixel(12, 5), Color::rgb(0, 200, 0), "later layers on top");
    assert_eq!(
        f.pixel(35, 5),
        Color::rgb(0, 0, 9),
        "background where nothing is"
    );
}

#[test]
fn translucent_layers_and_opacity_blend() {
    let mut c = SoftwareCompositor::new(10, 10);
    let l = solid(&mut c, 10, 10, Color::WHITE, true);
    c.compose(
        Color::BLACK,
        &[Placement {
            layer: l,
            x: 0,
            y: 0,
            opacity: 0.5,
        }],
    )
    .unwrap();
    let v = c.frame().pixel(3, 3).r;
    assert!((126..=129).contains(&v), "{v}");
}

#[test]
fn layers_keep_their_pixels_until_dropped() {
    let mut c = SoftwareCompositor::new(10, 10);
    let l = c.create_layer(10, 10, false).unwrap();
    c.paint_layer(l, &mut |p: &mut dyn Painter| {
        assert_eq!(p.size(), (10, 10));
        p.fill_rect(Rect::new(0.0, 0.0, 5.0, 10.0), Color::WHITE);
    })
    .unwrap();
    assert_eq!(c.layer_count(), 1);
    c.compose(Color::BLACK, &[Placement::at(l, 0, 0)]).unwrap();
    assert_eq!(c.frame().pixel(2, 2), Color::WHITE);
    assert_eq!(c.frame().pixel(7, 2), Color::BLACK);
    c.drop_layer(l);
    assert_eq!(c.layer_count(), 0);
    assert!(c.compose(Color::BLACK, &[Placement::at(l, 0, 0)]).is_err());
    assert!(c.paint_layer(l, &mut |_| {}).is_err());
}

#[test]
fn present_hands_the_frame_to_the_display() {
    let mut c = SoftwareCompositor::new(8, 8);
    let mut d = HeadlessDisplay::new(8, 8);
    c.compose(Color::rgb(1, 2, 3), &[]).unwrap();
    c.present(&mut d).unwrap();
    assert_eq!(d.presented(), 1);
    assert_eq!(d.last_frame().unwrap().pixel(4, 4), Color::rgb(1, 2, 3));
    assert_eq!(
        c.read_frame().unwrap().data(),
        d.last_frame().unwrap().data()
    );
    assert_eq!(d.size(), c.size());
}

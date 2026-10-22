//! GLES2 tests on a real GL implementation (Mesa llvmpipe in CI). Skipped unless
//! `GLASS_GLES_TESTS=1`. The scenes are those of `glass-ui/tests/golden.rs`, compared with
//! the same goldens (`tests/golden/`) per scene:
//!
//! - solid fills, layers and text: at most ±2 per channel on every pixel;
//! - anti-aliased shape edges (rounded rects): ±2 except on at most 1 % of the pixels, whose
//!   difference is at most 64.
//!
//! On a mismatch the GLES image goes to `target/golden-actual-gles/`. The SGX540 assertion
//! layer is on.

use glass_gfx::text::{self, Align};
use glass_gfx::{
    fonts, Color, Compositor, Frame, HeadlessDisplay, Painter, Placement, Rect, SoftwareCompositor,
    Weight,
};
use glass_gfx_gles::{GlesCompositor, GlesOptions, LayerFormat};
use glass_ui::card::{paint_card, render_card};
use glass_ui::timeline::MAX_RESIDENT_LAYERS;
use glass_ui::CARDS_PER_PAD_WIDTH;
use glass_ui::{Card, CardContent, CardImage, InputEvent, RenderContext, Timeline};
use std::path::PathBuf;
use std::sync::Arc;

fn enabled() -> bool {
    if std::env::var("GLASS_GLES_TESTS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped: set GLASS_GLES_TESTS=1 to run the GLES tests");
    false
}

fn compositor_with(options: GlesOptions) -> GlesCompositor {
    let c = GlesCompositor::offscreen(640, 360, options).expect("GLES context");
    assert!(c.checks_enabled() || !options.sgx540_checks);
    c
}

fn compositor() -> GlesCompositor {
    compositor_with(GlesOptions {
        sgx540_checks: true,
        ..GlesOptions::default()
    })
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Clone, Copy, Debug)]
enum Tolerance {
    /// Every channel of every pixel within ±2.
    Strict,
    /// ±2 except on at most 1 % of the pixels, which are within 64.
    Edges,
}

#[derive(Debug)]
struct Diff {
    worst: u8,
    pixels_over_2: usize,
    pixels: usize,
}

fn diff(a: &Frame, b: &Frame) -> Diff {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()), "sizes");
    let mut worst = 0;
    let mut over = 0;
    for (pa, pb) in a.data().chunks_exact(4).zip(b.data().chunks_exact(4)) {
        let d = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap();
        worst = worst.max(d);
        if d > 2 {
            over += 1;
        }
    }
    Diff {
        worst,
        pixels_over_2: over,
        pixels: (a.width() * a.height()) as usize,
    }
}

fn within(d: &Diff, tol: Tolerance) -> bool {
    match tol {
        Tolerance::Strict => d.worst <= 2,
        Tolerance::Edges => d.pixels_over_2 * 100 <= d.pixels && d.worst <= 64,
    }
}

fn compare(name: &str, actual: &Frame, expected: &Frame, tol: Tolerance) {
    let d = diff(actual, expected);
    println!(
        "scene {name}: worst {} per channel, {} pixels over ±2 ({:.3} %), tolerance {tol:?}",
        d.worst,
        d.pixels_over_2,
        100.0 * d.pixels_over_2 as f64 / d.pixels as f64
    );
    if !within(&d, tol) {
        let out = root().join("target/golden-actual-gles");
        std::fs::create_dir_all(&out).unwrap();
        actual.write_png(out.join(format!("{name}.png"))).unwrap();
        panic!(
            "{name}: {d:?} exceeds {tol:?}; GLES image in {}",
            out.display()
        );
    }
}

fn golden(name: &str) -> Frame {
    let path = root().join("tests/golden").join(format!("{name}.png"));
    Frame::from_png(&std::fs::read(&path).unwrap()).unwrap()
}

fn check_golden(name: &str, actual: &Frame, tol: Tolerance) {
    compare(name, actual, &golden(name), tol);
}

/// One card, painted into a layer and composed at (0, 0), read back.
fn render_card_gles(c: &mut GlesCompositor, card: &Card) -> Frame {
    let id = c.create_layer(640, 360, true).unwrap();
    c.paint_layer(id, &mut |p: &mut dyn Painter| paint_card(card, &ctx(), p))
        .unwrap();
    c.compose(Color::BLACK, &[Placement::at(id, 0, 0)]).unwrap();
    let f = c.read_frame().unwrap();
    c.drop_layer(id);
    f
}

// ---- the scenes of glass-ui/tests/golden.rs ----

fn ctx() -> RenderContext {
    RenderContext {
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

fn scene(name: &str) -> Card {
    match name {
        "home" => Card::new(10, CardContent::Home),
        "text" => message(),
        "text-long" => Card::new(
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
        ),
        "settings" => Card::new(1, CardContent::settings("Settings")),
        "image-placeholder" => Card::new(
            13,
            CardContent::Image {
                image: None,
                caption: "Photo".into(),
                timestamp_s: ctx().now_s - 26 * 3600,
            },
        ),
        "menu-item" => Card::new(
            20,
            CardContent::MenuItem {
                label: "Turn off".into(),
                detail: Some("Head wake is on".into()),
                index: 1,
                count: 3,
            },
        ),
        "adjust" => Card::new(
            21,
            CardContent::Adjust {
                title: "Wake angle".into(),
                value: "30°".into(),
                fraction: 2.0 / 3.0,
            },
        ),
        "info" => Card::new(
            22,
            CardContent::Info {
                title: "Device".into(),
                lines: vec![
                    "glass-launcher 0.1.0, renderer gles".into(),
                    "glassd mock-0.1.0".into(),
                    "touchpad, screen, head, wear, wink, button-camera, button-power, hinge".into(),
                    "Platform: mock".into(),
                ],
            },
        ),
        _ => unreachable!(),
    }
}

#[test]
fn gl_info() {
    if !enabled() {
        return;
    }
    let c = compositor();
    println!("{}", c.info());
    assert!(c.info().max_texture_size >= 1024);
}

#[test]
fn golden_cards() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    for (name, tol) in [
        ("home", Tolerance::Strict),
        ("text", Tolerance::Strict),
        ("text-long", Tolerance::Strict),
        ("settings", Tolerance::Strict),
        ("image-placeholder", Tolerance::Edges),
        // Menu position dots and the adjustment bar are rounded rects.
        ("menu-item", Tolerance::Edges),
        ("adjust", Tolerance::Edges),
        ("info", Tolerance::Strict),
    ] {
        let f = render_card_gles(&mut c, &scene(name));
        check_golden(name, &f, tol);
    }
}

#[test]
fn golden_timeline_mid_scroll() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    let mut t = Timeline::new(
        vec![Card::new(1, CardContent::settings("Settings"))],
        Card::new(10, CardContent::Home),
        vec![message()],
    );
    t.set_context(ctx());
    t.render(&mut c).unwrap();
    t.handle(&InputEvent::Scroll {
        displacement: 0.35 / CARDS_PER_PAD_WIDTH,
        delta: 0.0,
        velocity: 0.0,
        fingers: 1,
        ended: false,
        cancelled: false,
        timestamp_us: 0,
    });
    t.render(&mut c).unwrap();
    check_golden(
        "timeline-mid-scroll",
        &c.read_frame().unwrap(),
        Tolerance::Strict,
    );
    // Present hands the read-back frame to the display.
    let mut d = HeadlessDisplay::new(640, 360);
    c.present(&mut d).unwrap();
    assert_eq!(d.presented(), 1);
}

#[test]
fn a_fling_keeps_at_most_the_resident_layers_on_the_gpu() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    let mut t = Timeline::new(
        vec![],
        Card::new(10, CardContent::Home),
        (0..10).map(|i| scene_text(100 + i)).collect(),
    );
    t.set_context(ctx());
    t.render(&mut c).unwrap();
    for i in 1..=40 {
        t.handle(&InputEvent::Scroll {
            displacement: 4.0 * i as f32 / 40.0 / CARDS_PER_PAD_WIDTH,
            delta: 0.0,
            velocity: 0.0,
            fingers: 1,
            ended: false,
            cancelled: false,
            timestamp_us: 0,
        });
        t.render(&mut c).unwrap();
        assert!(c.layer_count() <= MAX_RESIDENT_LAYERS);
    }
}

fn scene_text(id: u64) -> Card {
    Card::new(
        id,
        CardContent::Text {
            body: format!("History card {id}"),
            source: "Test".into(),
            timestamp_s: ctx().now_s - 60,
        },
    )
}

/// Translucent layers and opacity blend like the software compositor.
#[test]
fn translucent_composition_matches_software() {
    if !enabled() {
        return;
    }
    let mut g = compositor();
    let mut s = SoftwareCompositor::new(640, 360);
    for c in [&mut g as &mut dyn Compositor, &mut s] {
        let a = c.create_layer(640, 360, true).unwrap();
        c.paint_layer(a, &mut |p: &mut dyn Painter| {
            paint_card(&message(), &ctx(), p)
        })
        .unwrap();
        let b = c.create_layer(200, 100, false).unwrap();
        c.paint_layer(b, &mut |p: &mut dyn Painter| {
            p.clear(Color::rgba(0, 0, 0, 0));
            p.fill_rect(
                Rect::new(0.0, 0.0, 200.0, 100.0),
                Color::rgba(40, 120, 220, 160),
            );
        })
        .unwrap();
        c.compose(
            Color::BLACK,
            &[
                Placement {
                    layer: a,
                    x: 0,
                    y: 0,
                    opacity: 0.6,
                },
                Placement::at(b, 220, 130),
            ],
        )
        .unwrap();
    }
    compare(
        "translucent-composition",
        &g.read_frame().unwrap(),
        s.frame(),
        Tolerance::Strict,
    );
}

/// Text after the atlas filled up and was cleared still matches.
#[test]
fn text_after_the_atlas_was_refilled() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    let id = c.create_layer(640, 360, true).unwrap();
    let font = fonts::font(Weight::Light);
    let line = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let paint_at = |size: f32| {
        move |p: &mut dyn Painter| {
            p.clear(Color::BLACK);
            let (run, _) = text::layout_line(font, size, 0.0, 200.0, line);
            p.draw_glyph_run(&run, Color::WHITE);
        }
    };
    let mut size = 60.0;
    loop {
        c.paint_layer(id, &mut paint_at(size)).unwrap();
        if c.atlas_clears() > 0 {
            break;
        }
        size += 3.0;
        assert!(size < 2000.0, "atlas never filled");
    }
    // The run whose glyphs filled the atlas was drawn partly before and partly after the
    // refill: it must still match the software path.
    c.compose(Color::BLACK, &[Placement::at(id, 0, 0)]).unwrap();
    let mut s = SoftwareCompositor::new(640, 360);
    let sid = s.create_layer(640, 360, true).unwrap();
    s.paint_layer(sid, &mut paint_at(size)).unwrap();
    s.compose(Color::BLACK, &[Placement::at(sid, 0, 0)])
        .unwrap();
    compare(
        &format!("atlas-refill-run-{size}px"),
        &c.read_frame().unwrap(),
        s.frame(),
        Tolerance::Strict,
    );
    c.drop_layer(id);
    check_golden(
        "text",
        &render_card_gles(&mut c, &scene("text")),
        Tolerance::Strict,
    );
    check_golden(
        "home",
        &render_card_gles(&mut c, &scene("home")),
        Tolerance::Strict,
    );
}

fn gradient(w: u32, h: u32) -> Frame {
    let mut f = Frame::new(w, h);
    let d = f.data_mut();
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            d[i] = (x * 255 / w) as u8;
            d[i + 1] = (y * 255 / h) as u8;
            d[i + 2] = 128;
            d[i + 3] = 255;
        }
    }
    f
}

fn image_card(frame: Frame) -> Card {
    Card::new(
        20,
        CardContent::Image {
            image: Some(CardImage {
                id: 1,
                frame: Arc::new(frame),
            }),
            caption: "Photo".into(),
            timestamp_s: ctx().now_s - 60,
        },
    )
}

/// An image drawn 1:1 matches the software path.
#[test]
fn image_at_its_own_size_matches_software() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    let card = image_card(gradient(640, 360));
    compare(
        "image-1to1",
        &render_card_gles(&mut c, &card),
        &render_card(&card, &ctx(), 640, 360),
        Tolerance::Strict,
    );
}

/// A photo larger than the texture limit (2048, the SGX540's, with the checks on) is
/// downscaled on the CPU before upload (the
/// assertion layer would reject the upload otherwise); a smooth image stays close to the
/// software path, which filters the full-size image.
#[test]
fn large_photo_is_downscaled_before_upload() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    let max = c.max_texture_size();
    assert_eq!(
        max,
        2048.min(c.info().max_texture_size),
        "SGX540 limit under checks"
    );
    let card = image_card(gradient(max + 952, (max + 952) * 2 / 3));
    let g = render_card_gles(&mut c, &card);
    let s = render_card(&card, &ctx(), 640, 360);
    let d = diff(&g, &s);
    println!("scene image-downscaled: {d:?}");
    assert!(d.worst <= 8, "{d:?}");
}

/// The RGB565 fallback for opaque layers works (forced here; used when an RGBA FBO is
/// incomplete). Its output is not held to the goldens' ±2 (5 and 6 bits per channel).
#[test]
fn rgb565_fallback_layers_render() {
    if !enabled() {
        return;
    }
    let mut c = compositor_with(GlesOptions {
        sgx540_checks: true,
        force_rgb565_layers: true,
        ..GlesOptions::default()
    });
    let id = c.create_layer(640, 360, true).unwrap();
    assert_eq!(c.layer_format(id), Some(LayerFormat::Rgb565));
    let translucent = c.create_layer(64, 64, false).unwrap();
    assert_eq!(c.layer_format(translucent), Some(LayerFormat::Rgba8888));
    c.paint_layer(id, &mut |p: &mut dyn Painter| {
        p.clear(Color::rgb(200, 100, 50));
    })
    .unwrap();
    c.compose(Color::BLACK, &[Placement::at(id, 0, 0)]).unwrap();
    let px = c.read_frame().unwrap().pixel(320, 180);
    for (a, b) in [(px.r, 200), (px.g, 100), (px.b, 50)] {
        assert!(a.abs_diff(b) <= 8, "{px:?}");
    }
}

/// The assertion layer is active: a mipmap generation and a too-large texture panic.
#[test]
fn sgx540_assertion_layer_rejects_violations() {
    if !enabled() {
        return;
    }
    let mut c = compositor();
    let max = c.max_texture_size();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        c.with_gl(|gl| gl.generate_mipmap())
    }));
    assert!(r.is_err(), "glGenerateMipmap must be rejected");
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        c.with_gl(|gl| {
            let t = gl.create_texture().unwrap();
            gl.bind_texture(Some(t));
            gl.tex_image_2d(
                glow::RGBA,
                max + 1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                None,
            );
        })
    }));
    assert!(
        r.is_err(),
        "a texture above GL_MAX_TEXTURE_SIZE must be rejected"
    );
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        c.with_gl(|gl| gl.compile_shader(glow::FRAGMENT_SHADER, "void main() {}"))
    }));
    assert!(r.is_err(), "a shader without #version 100 must be rejected");
}

#[test]
fn box_text_matches_software() {
    if !enabled() {
        return;
    }
    let mut g = compositor();
    let mut s = SoftwareCompositor::new(640, 360);
    for c in [&mut g as &mut dyn Compositor, &mut s] {
        let id = c.create_layer(640, 360, true).unwrap();
        c.paint_layer(id, &mut |p: &mut dyn Painter| {
            p.clear(Color::grey(0x20));
            let font = fonts::font(Weight::Light);
            for run in text::layout_box_runs(
                font,
                30.0,
                1.2,
                Rect::new(20.0, 20.0, 600.0, 320.0),
                "Grey on dark grey, centred: text blends over a non-black background too.",
                Align::Center,
                usize::MAX,
            ) {
                p.draw_glyph_run(&run, Color::rgba(250, 200, 120, 200));
            }
        })
        .unwrap();
        c.compose(Color::BLACK, &[Placement::at(id, 0, 0)]).unwrap();
    }
    compare(
        "box-text",
        &g.read_frame().unwrap(),
        s.frame(),
        Tolerance::Strict,
    );
}

/// The EGL 1.4 set-up (default display, pbuffer surface), as on Android and libhybris.
/// Mesa needs `EGL_PLATFORM=surfaceless` for a default display without a window system.
#[test]
fn pbuffer_context_renders_the_goldens_too() {
    if !enabled() {
        return;
    }
    if std::env::var_os("EGL_PLATFORM").is_none() {
        eprintln!("skipped: set EGL_PLATFORM (surfaceless on Mesa) for the pbuffer test");
        return;
    }
    let mut c = compositor_with(GlesOptions {
        sgx540_checks: true,
        force_pbuffer: true,
        ..GlesOptions::default()
    });
    println!("{}", c.info());
    assert!(
        c.info().egl.setup.contains("pbuffer"),
        "{}",
        c.info().egl.setup
    );
    check_golden(
        "text",
        &render_card_gles(&mut c, &scene("text")),
        Tolerance::Strict,
    );
}
